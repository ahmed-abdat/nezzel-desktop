//! Download-only retries. Login/source retries retain their separate budgets.
use crate::{
  models::{MediaProgress, MediaProgressStage, ProgressCategory, ProgressStage},
  scheduling::download_pipeline::DownloadEntry,
};
use std::{
  collections::HashMap,
  sync::{LazyLock, Mutex},
  time::Duration,
};
use tokio::{sync::watch, time::Instant};

/// YouTube page and CDN aliases share one budget, across fetch and download workers.
static YOUTUBE_CHILDREN: LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
  LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));

pub(crate) async fn playlist_slot(
  url: &str,
  child: bool,
  cancel: &mut watch::Receiver<bool>,
) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, ()> {
  if !*cancel.borrow() {
    return Err(());
  }
  if !child || !super::retry_policy::youtube_url(url) {
    return Ok(None);
  }
  let acquire = YOUTUBE_CHILDREN.clone().acquire_owned();
  tokio::pin!(acquire);
  let mut watching = true;
  loop {
    // This watch stores “running”: only false cancels a queued child.
    if !*cancel.borrow_and_update() {
      return Err(());
    }
    tokio::select! {
      biased;
      changed = cancel.changed(), if watching => {
        // A closed sender cannot produce another notification. Keep our FIFO place.
        if changed.is_err() { watching = false; }
      }
      permit = &mut acquire => return permit.map(Some).map_err(|_| ()),
    }
  }
}

pub(crate) fn playlist_args(child: bool) -> Vec<&'static str> {
  if child {
    vec!["--sleep-requests", "1", "--sleep-interval", "2"]
  } else {
    Vec::new()
  }
}

pub(crate) fn refetch_delay(url: &str) -> Duration {
  Duration::from_secs(if super::retry_policy::youtube_url(url) {
    rand::random_range(5..=15)
  } else {
    2
  })
}

#[derive(Default)]
pub(crate) struct Recovery {
  network_started: Option<Duration>,
  network_attempt: u32,
  busy_attempt: u8,
  restarted_stall: bool,
}
impl Recovery {
  pub fn progressed(&mut self) {
    self.network_started = None;
    self.network_attempt = 0;
  }
  pub fn network_delay(&mut self, now: Duration) -> Option<Duration> {
    let started = *self.network_started.get_or_insert(now);
    let delay =
      Duration::from_secs((5u64.saturating_mul(1u64 << self.network_attempt.min(4))).min(60));
    if now.saturating_sub(started) + delay > Duration::from_secs(600) {
      return None;
    }
    self.network_attempt += 1;
    Some(delay)
  }
  pub fn busy_delay(&mut self) -> Option<Duration> {
    let seconds = match self.busy_attempt {
      0 => 30,
      1 => 120,
      _ => return None,
    };
    self.busy_attempt += 1;
    Some(Duration::from_secs(seconds))
  }
  pub fn restart_stall(&mut self) -> bool {
    !std::mem::replace(&mut self.restarted_stall, true)
  }
}

pub(crate) fn transient(line: &str) -> bool {
  error_lines(line).any(|line| {
    let line = line.to_ascii_lowercase();
    [
      "timed out",
      "timeout",
      "connection reset",
      "connection aborted",
      "connection refused",
      "temporary failure in name resolution",
      "name or service not known",
      "getaddrinfo failed",
      "nodename nor servname",
      "network is unreachable",
      "network unreachable",
      "network is down",
      "no route to host",
      "remote end closed",
      "unable to resolve",
      "failed to resolve",
      "offline",
    ]
    .iter()
    .any(|needle| line.contains(needle))
  })
}
pub(crate) fn rate_limited(line: &str) -> bool {
  error_lines(line).any(|line| {
    let line = line.to_ascii_lowercase();
    line.contains("http error 429")
      || line.contains("too many requests")
      || line.contains("http status 429")
  })
}

fn error_lines(text: &str) -> impl Iterator<Item = &str> {
  text.lines().filter_map(|line| line.strip_prefix("ERROR:"))
}

pub(crate) struct StallWatch {
  last_progress: Duration,
  bytes: u64,
  file_bytes: Option<u64>,
  active: bool,
  extracting_since: Option<Duration>,
  retry_since: Option<Duration>,
}
impl StallWatch {
  pub fn new(now: Duration) -> Self {
    Self {
      last_progress: now,
      bytes: 0,
      file_bytes: None,
      active: false,
      extracting_since: Some(now),
      retry_since: None,
    }
  }
  /// True only when new bytes arrived (a destination/log line is not progress).
  pub fn observe(&mut self, line: &str, now: Duration) -> bool {
    if line.starts_with("[download] Destination:") {
      self.bytes = 0;
      self.last_progress = now;
      self.active = true;
      self.extracting_since = None;
    }
    // Let yt-dlp exhaust its own bounded retry policy before interpreting its final ERROR.
    if line.starts_with("[download] Got error:") && line.contains(". Retrying (") {
      self.retry_since.get_or_insert(now);
    }
    if let Some(raw) = line.strip_prefix("RAW|") {
      if let Some(bytes) = raw.split('|').nth(4).and_then(|s| s.parse::<u64>().ok()) {
        let moved = bytes > self.bytes;
        self.bytes = bytes;
        if moved {
          self.progress(now);
        }
        return moved;
      }
    }
    false
  }
  pub fn observe_file_bytes(&mut self, bytes: u64, now: Duration) -> bool {
    let moved = (self.active || self.extracting_since.is_some())
      && self.file_bytes.is_some_and(|previous| bytes > previous);
    self.file_bytes = Some(bytes);
    if moved {
      self.progress(now);
    }
    moved
  }
  fn progress(&mut self, now: Duration) {
    self.last_progress = now;
    self.active = true;
    self.extracting_since = None;
    self.retry_since = None;
  }
  pub fn extraction_expired(&self, now: Duration) -> bool {
    self
      .extracting_since
      .is_some_and(|start| now.saturating_sub(start) >= Duration::from_secs(300))
  }
  pub fn stage(&mut self, stage: &ProgressStage) {
    if matches!(
      stage,
      ProgressStage::Merging
        | ProgressStage::Remuxing
        | ProgressStage::Reencoding
        | ProgressStage::Finalizing
    ) {
      self.active = false;
      self.extracting_since = None;
      self.retry_since = None;
    }
  }
  pub fn stalled(&self, now: Duration) -> bool {
    self.active
      && match self.retry_since {
        Some(start) => now.saturating_sub(start) >= Duration::from_secs(300),
        None => now.saturating_sub(self.last_progress) >= Duration::from_secs(60),
      }
  }
}

/// Size only, never read file contents or follow symlinks. Ignore ownership/receipt metadata.
pub(crate) fn partial_bytes(path: &std::path::Path) -> Option<u64> {
  fn walk(path: &std::path::Path, depth: usize) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
      let entry = entry?;
      let kind = entry.file_type()?;
      if kind.is_dir() && depth < 8 {
        total = total.saturating_add(walk(&entry.path(), depth + 1)?);
      } else if kind.is_file() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with('.')
          && !name.ends_with(".json")
          && !name.ends_with(".ytdl")
          && name != "images.txt"
        {
          total = total.saturating_add(entry.metadata()?.len());
        }
      }
    }
    Ok(total)
  }
  walk(path, 0).ok()
}

static HOSTS: LazyLock<Mutex<HashMap<String, Instant>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
fn host(url: &str) -> String {
  reqwest::Url::parse(url)
    .ok()
    .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
    .unwrap_or_default()
}
pub(crate) fn cool_host(url: &str, delay: Duration) {
  let now = Instant::now();
  let mut hosts = HOSTS.lock().unwrap_or_else(|e| e.into_inner());
  hosts.retain(|_, until| *until > now);
  let until = hosts.entry(host(url)).or_insert(now);
  *until = (*until).max(now + delay);
}
pub(crate) fn host_delay(url: &str) -> Duration {
  HOSTS
    .lock()
    .unwrap_or_else(|e| e.into_inner())
    .get(&host(url))
    .map(|until| until.saturating_duration_since(Instant::now()))
    .unwrap_or_default()
}

/// Returns false on pause/cancel; no retry can escape the group cancellation signal.
pub(crate) async fn wait<R: tauri::Runtime>(
  app: &tauri::AppHandle<R>,
  entry: &DownloadEntry,
  cancel: &mut watch::Receiver<bool>,
  delay: Duration,
  busy: bool,
) -> bool {
  let until = Instant::now() + delay;
  while *cancel.borrow() {
    let left = until.saturating_duration_since(Instant::now());
    if left.is_zero() {
      return true;
    }
    crate::events::emit(
      app,
      crate::events::MEDIA_PROGRESS_STAGE,
      MediaProgressStage {
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        stage: if busy {
          ProgressStage::SiteBusy
        } else {
          ProgressStage::WaitingNetwork
        },
      },
    );
    crate::events::emit(
      app,
      crate::events::MEDIA_PROGRESS,
      MediaProgress {
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        category: ProgressCategory::Other,
        percentage: None,
        speed_bps: Some(0.0),
        eta_secs: Some(left.as_secs().max(1)),
      },
    );
    tokio::select! {
      biased;
      changed = cancel.changed() => { if changed.is_err() || !*cancel.borrow() { return false; } }
      _ = tokio::time::sleep(left.min(Duration::from_secs(1))) => {}
    }
  }
  false
}

pub(crate) fn fatal(app: &tauri::AppHandle, entry: &DownloadEntry, code: &str) {
  let event = crate::models::error::DiagnosticEvent {
    id: entry.id.clone(),
    group_id: entry.group_id.clone(),
    level: crate::models::error::DiagnosticLevel::Error,
    code: code.into(),
    component: Some("download".into()),
    message: code.into(),
    raw: code.into(),
    timestamp: 0,
  };
  crate::extract::terminal::emit_terminal(
    app,
    crate::models::MediaDiagnosticPayload::from_diagnostic_event(event),
    crate::models::MediaFatalPayload::with_exit(
      entry.group_id.clone(),
      entry.id.clone(),
      1,
      code.into(),
    ),
  );
}

#[cfg(test)]
mod tests {
  use super::*;
  #[tokio::test(start_paused = true)]
  async fn playlist_slots_share_youtube_aliases_and_cancel_queued_children() {
    let mut results = Vec::new();
    for notification in ["dropped", "running", "cancelled"] {
      let (tx, mut rx) = watch::channel(true);
      let one = playlist_slot("https://youtube.com/watch?v=1", true, &mut rx)
        .await
        .unwrap();
      let two = playlist_slot("https://rr1.googlevideo.com/v", true, &mut rx)
        .await
        .unwrap();
      assert!(
        playlist_slot("https://youtube.com/watch?v=single", false, &mut rx)
          .await
          .unwrap()
          .is_none()
      );
      let pending = playlist_slot("https://youtu.be/3", true, &mut rx);
      tokio::pin!(pending);
      tokio::select! {
        biased;
        _ = &mut pending => panic!("third child started"),
        _ = tokio::task::yield_now() => {}
      }
      match notification {
        "dropped" => drop(tx),
        "running" => {
          tx.send(true).unwrap();
        }
        _ => {
          tx.send(false).unwrap();
        }
      }
      drop(one);
      results.push((notification, pending.await.is_ok()));
      drop(two);
    }
    assert_eq!(
      results,
      [("dropped", true), ("running", true), ("cancelled", false)]
    );
  }

  #[test]
  fn pacing_only_applies_to_children_and_refetch_jitter_is_bounded() {
    assert!(playlist_args(false).is_empty());
    assert_eq!(
      playlist_args(true),
      ["--sleep-requests", "1", "--sleep-interval", "2"]
    );
    for _ in 0..100 {
      assert!((5..=15).contains(&refetch_delay("https://youtube.com/watch?v=1").as_secs()));
    }
    assert_eq!(
      refetch_delay("https://bilibili.com/video/1"),
      Duration::from_secs(2)
    );
  }

  #[test]
  fn only_terminal_error_lines_classify_transport_failures() {
    for line in [
      "[download] Destination: /Downloads/Offline Tour.mp4",
      "[download] Destination: /Downloads/timeout - Too Many Requests.mp4",
      "[info] Offline timeout Too Many Requests",
      "[download] Got error: connection timed out. Retrying (1/10)",
      "WARNING: HTTP Error 429: Too Many Requests. Retrying (1/10)",
    ] {
      assert!(!transient(line), "{line}");
      assert!(!rate_limited(line), "{line}");
    }
    assert!(transient("ERROR: Unable to download: connection timed out"));
    assert!(rate_limited("ERROR: HTTP Error 429: Too Many Requests"));
    assert!(!transient("[info] timeout\nERROR: HTTP Error 403"));
    assert!(transient("[info] title\nERROR: connection reset"));
  }

  #[test]
  fn extraction_is_not_a_transfer_stall() {
    let mut watch = StallWatch::new(Duration::ZERO);
    watch.observe("[info] fetching playlist", Duration::from_secs(90));
    watch.observe_file_bytes(0, Duration::from_secs(90));
    assert!(!watch.stalled(Duration::from_secs(120)));
    assert!(!watch.extraction_expired(Duration::from_secs(299)));
    assert!(watch.extraction_expired(Duration::from_secs(300)));
    watch.observe("[download] Destination: clip.mp4", Duration::from_secs(150));
    assert!(!watch.stalled(Duration::from_secs(209)));
    assert!(watch.stalled(Duration::from_secs(210)));
    assert!(!watch.extraction_expired(Duration::from_secs(450)));
    let mut files = StallWatch::new(Duration::ZERO);
    assert!(!files.observe_file_bytes(200, Duration::ZERO)); // Existing .part is baseline.
    assert!(!files.stalled(Duration::from_secs(90)));
    assert!(files.observe_file_bytes(201, Duration::from_secs(100)));
    assert!(files.stalled(Duration::from_secs(160)));
  }

  #[test]
  fn native_retry_sleep_is_not_a_stall() {
    let mut watch = StallWatch::new(Duration::ZERO);
    watch.observe("[download] Destination: clip.mp4", Duration::ZERO);
    watch.observe(
      "[download] Got error: connection timed out. Retrying (1/10)",
      Duration::from_secs(20),
    );
    assert!(!watch.stalled(Duration::from_secs(90)));
  }

  #[test]
  fn network_backoff_caps_delay_and_elapsed_budget() {
    let mut policy = Recovery::default();
    let mut now = Duration::ZERO;
    let mut delays = vec![];
    while let Some(delay) = policy.network_delay(now) {
      delays.push(delay.as_secs());
      now += delay;
    }
    assert_eq!(&delays[..5], &[5, 10, 20, 40, 60]);
    assert!(now <= Duration::from_secs(600));
    assert!(now > Duration::from_secs(540));
    assert!(policy.network_delay(Duration::from_secs(601)).is_none());
  }
  #[test]
  fn a_later_outage_gets_a_fresh_network_budget_after_progress() {
    let mut policy = Recovery::default();
    assert_eq!(
      policy.network_delay(Duration::ZERO),
      Some(Duration::from_secs(5))
    );
    policy.progressed();
    assert_eq!(
      policy.network_delay(Duration::from_secs(7200)),
      Some(Duration::from_secs(5))
    );
  }
  #[test]
  fn busy_and_stall_have_independent_bounded_budgets() {
    let mut policy = Recovery::default();
    assert_eq!(policy.busy_delay(), Some(Duration::from_secs(30)));
    assert_eq!(policy.busy_delay(), Some(Duration::from_secs(120)));
    assert_eq!(policy.busy_delay(), None);
    assert!(policy.restart_stall());
    assert!(!policy.restart_stall());
  }
  #[test]
  fn watchdog_needs_byte_movement_not_repeated_logs_and_ignores_processing() {
    let mut watch = StallWatch::new(Duration::ZERO);
    watch.observe("RAW|10|10|10|10|100|1000", Duration::from_secs(20));
    watch.observe("RAW|10|10|10|10|100|1000", Duration::from_secs(70));
    assert!(!watch.stalled(Duration::from_secs(79)));
    assert!(watch.stalled(Duration::from_secs(80)));
    watch.observe(
      "[download] Destination: second audio",
      Duration::from_secs(90),
    );
    assert!(!watch.stalled(Duration::from_secs(100)));
    watch.stage(&ProgressStage::Merging);
    assert!(!watch.stalled(Duration::from_secs(1000)));
    watch.observe_file_bytes(10, Duration::from_secs(1000));
    watch.observe_file_bytes(100, Duration::from_secs(1001));
    assert!(!watch.stalled(Duration::from_secs(1100)));
  }
  #[tokio::test(start_paused = true)]
  async fn host_cooldown_is_shared_only_with_that_host() {
    cool_host("https://busy.example/a", Duration::from_secs(30));
    assert_eq!(
      host_delay("https://busy.example/b"),
      Duration::from_secs(30)
    );
    assert!(host_delay("https://other.example/b").is_zero());
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(host_delay("https://busy.example/b").is_zero());
  }
  #[tokio::test(start_paused = true)]
  async fn cancellation_and_closed_group_interrupt_the_wait_without_advancing_time() {
    let app = tauri::test::mock_app();
    let item = serde_json::from_value::<crate::models::DownloadItem>(serde_json::json!({
      "id": "waiting", "url": "https://example.com/video", "format": {"trackType": "both"},
      "templateContext": {"values": {}}
    }))
    .unwrap();
    let entry = DownloadEntry::from((item, "waiting-group".into()));
    let (tx, mut rx) = watch::channel(true);
    let waiting = wait(
      app.handle(),
      &entry,
      &mut rx,
      Duration::from_secs(60),
      false,
    );
    tokio::pin!(waiting);
    tokio::select! {
      biased;
      _ = &mut waiting => panic!("wait returned before cancellation"),
      _ = tokio::task::yield_now() => {}
    }
    tx.send(false).unwrap();
    assert!(!waiting.await);
    let (tx, mut rx) = watch::channel(true);
    drop(tx);
    assert!(!wait(app.handle(), &entry, &mut rx, Duration::from_secs(60), true).await);
  }

  #[test]
  fn network_classification_does_not_retry_permanent_or_auth_errors() {
    for line in [
      "DNS: getaddrinfo failed",
      "Connection reset by peer",
      "request timeout",
      "Network is unreachable",
    ] {
      assert!(transient(&format!("ERROR: {line}")));
    }
    for line in [
      "HTTP Error 404",
      "HTTP Error 403",
      "Sign in required",
      "No space left on device",
    ] {
      assert!(!transient(line));
    }
  }
}
