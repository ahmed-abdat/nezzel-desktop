//! Usage counts (owner decision 2026-11-05, spec docs/specs/usage-counts.md): daily anonymous totals to our own
//! inbox, like Nezzel Android. Endpoint from `NEZZEL_USAGE_URL` at build time; unset = nothing counted or sent.
//! Switch: Settings → Advanced → Help improve Nezzel (`privacy.send_usage`). No ID, link, title, file name, path,
//! cookie or account name can form a key (`keys::allowed`). Counts and bodies are never logged.

pub mod keys;
mod sender;
mod snapshot;
pub mod store;

use crate::models::parsed::{FetchEngine, MediaKind};
use crate::models::TrackType;
use crate::scheduling::download_pipeline::DownloadEntry;
use crate::SharedConfig;
use keys::Kind;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use store::{End, Origin, State};
use tauri::{AppHandle, Listener, Manager};

const FILE: &str = "usage.json";

pub struct Usage {
  path: Option<PathBuf>,
  endpoint: Option<String>,
  version: String,
  state: Mutex<State>,
  /// "Help improve Nezzel" and an endpoint. Read and written under the state lock, so no count lands after off.
  on: AtomicBool,
  /// Bumped by `clear`: a send begun before the switch went off never writes back after it.
  generation: AtomicU64,
  /// Fired by `clear`: an in-flight POST is dropped (never sent, or abandoned mid-way).
  cancel: tokio::sync::Notify,
  /// One send at a time (the 6-hour tick and the quit path): a second caller waits, then finds the day sent.
  sending: tokio::sync::Mutex<()>,
  /// Per running job: speed samples (sum, n) and when its link was received. Memory only.
  speeds: Mutex<HashMap<String, (f64, u32)>>,
  links: Mutex<HashMap<String, (Instant, String)>>,
  /// Active jobs only: fixed app error codes, never diagnostic text.
  failures: Mutex<HashMap<String, &'static str>>,
}

fn now_ms() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}

fn job_id(group_id: &str, id: &str) -> String {
  format!("{group_id}/{id}")
}

/// `NEZZEL_USAGE_URL` as built in: https only, else off.
pub fn endpoint(raw: Option<&str>) -> Option<String> {
  let raw = raw?.trim();
  (raw.starts_with("https://") && raw.len() > "https://".len()).then(|| raw.to_string())
}

impl Usage {
  pub fn new(path: Option<PathBuf>, endpoint: Option<String>, version: String) -> Self {
    let state = path
      .as_ref()
      .and_then(|path| std::fs::read(path).ok())
      .and_then(|bytes| serde_json::from_slice(&bytes).ok())
      .unwrap_or_default();
    Self {
      path,
      endpoint,
      version,
      state: Mutex::new(state),
      on: AtomicBool::new(false),
      generation: AtomicU64::new(0),
      cancel: tokio::sync::Notify::new(),
      sending: tokio::sync::Mutex::new(()),
      speeds: Mutex::default(),
      links: Mutex::default(),
      failures: Mutex::default(),
    }
  }

  pub fn configured(&self) -> bool {
    self.endpoint.is_some()
  }

  fn persist(&self, state: &State) {
    let Some(path) = &self.path else { return };
    let Ok(json) = serde_json::to_vec(state) else {
      return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
      let _ = std::fs::rename(&tmp, path);
    }
  }

  /// Runs `f` on the state and saves it.
  fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    let out = f(&mut state);
    self.persist(&state);
    out
  }

  /// Like `with`, only while on: the check and the write share the lock that `set_on(false)` takes.
  fn with_on<T>(&self, f: impl FnOnce(&mut State) -> T) -> Option<T> {
    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    if !self.on.load(Ordering::SeqCst) {
      return None;
    }
    let out = f(&mut state);
    self.persist(&state);
    Some(out)
  }

  /// Runs `f` while on, holding the state lock but without saving: memory-only notes (links, speeds) then can't
  /// outlive a switch-off, which clears them under the same lock.
  fn locked_on<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
    let _state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    self.on.load(Ordering::SeqCst).then(f)
  }

  /// A link received: its counts, a fresh job, and when it came (for time to options), all under one lock.
  fn note_link(&self, job: &str, url: &str, now: u64) {
    let mut found = vec![keys::FUNNEL_LINK.to_string()];
    found.extend(keys::site(keys::SITE_IN, url));
    self.with_on(|state| {
      // Retry reads the link again under the same ID: a fresh job.
      state.forget_job(job);
      for key in &found {
        state.count(key, now);
      }
      let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
      if links.len() >= store::MAX_JOBS {
        links.clear();
      }
      links.insert(job.to_string(), (Instant::now(), url.to_string()));
    });
  }

  fn note_speed(&self, job: &str, bps: f64) {
    self.locked_on(|| {
      let mut speeds = self.speeds.lock().unwrap_or_else(|e| e.into_inner());
      if speeds.len() >= store::MAX_JOBS {
        speeds.clear();
      }
      let sample = speeds.entry(job.to_string()).or_insert((0.0, 0));
      sample.0 += bps;
      sample.1 += 1;
    });
  }

  fn start_job(&self, job: &str, kind: Kind, login: bool) {
    let now = now_ms();
    self.with_on(|state| {
      let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
      if failures.len() >= store::MAX_JOBS {
        failures.clear();
      }
      failures.insert(job.to_string(), "unknown");
      if state.started(job) {
        state.count(&keys::download("started", kind), now);
        if login {
          state.count(keys::LOGIN_PROFILE, now);
        }
      }
    });
  }

  fn note_diagnostic(&self, payload: &str) {
    // Read only the classified code from the backend row event. Other fields are ignored.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Diagnostic {
      group_id: String,
      id: String,
      level: crate::models::error::DiagnosticLevel,
      code: String,
    }
    let Ok(event) = serde_json::from_str::<Diagnostic>(payload) else {
      return;
    };
    if !matches!(event.level, crate::models::error::DiagnosticLevel::Error) {
      return;
    }
    self.locked_on(|| {
      let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
      if let Some(code) = failures.get_mut(&job_id(&event.group_id, &event.id)) {
        *code = keys::failure_code(Some(&event.code));
      }
    });
  }

  fn finish_job(&self, job: &str, end: End, kind: Kind, url: &str) {
    let now = now_ms();
    self.with_on(|state| {
      let speed = self
        .speeds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(job);
      let code = self
        .failures
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(job);
      for key in ended_keys(state, job, end, kind, url, speed, code) {
        state.count(&key, now);
      }
    });
  }

  pub fn is_on(&self) -> bool {
    self.on.load(Ordering::SeqCst)
  }

  /// Off clears every count not yet sent and drops a POST in flight.
  pub fn set_on(&self, on: bool) {
    let on = on && self.configured();
    if on {
      self.on.store(true, Ordering::SeqCst);
    } else {
      self.clear();
    }
  }

  pub fn count_keys<K: AsRef<str>>(&self, keys: &[K]) {
    if keys.is_empty() {
      return;
    }
    let now = now_ms();
    self.with_on(|state| {
      for key in keys {
        state.count(key.as_ref(), now);
      }
    });
  }

  fn clear(&self) {
    {
      let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
      self.on.store(false, Ordering::SeqCst);
      self.generation.fetch_add(1, Ordering::SeqCst);
      state.clear();
      self.persist(&state);
      // Under the state lock: a note taken while on is either cleared here or refused after.
      self
        .speeds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
      self.links.lock().unwrap_or_else(|e| e.into_inner()).clear();
      self
        .failures
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    }
    self.cancel.notify_waiters();
  }

  pub fn snapshot_state(&self) -> State {
    self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
  }
}

/// A cheap early out; every write checks `on` again under the state lock.
fn usage(app: &AppHandle) -> Option<tauri::State<'_, Usage>> {
  let usage = app.try_state::<Usage>()?;
  usage.is_on().then_some(usage)
}

pub fn setup(app: &AppHandle, app_dir: &std::path::Path) {
  let version = app.package_info().version.to_string();
  let usage = Usage::new(
    Some(app_dir.join(FILE)),
    endpoint(option_env!("NEZZEL_USAGE_URL")),
    version.clone(),
  );
  let now = now_ms();
  let first = usage.with(|state| state.on_start(&version, now));
  // Off: whatever is left from before the switch went off is never sent.
  usage.set_on(app.state::<SharedConfig>().load().privacy.send_usage);
  usage.count_keys(&first);
  usage.count_keys(&[keys::OPEN]);
  app.manage(usage);
  listen_for_failures(app);
  crate::state::config::listen(app, |app, config| on_config(app, config.privacy.send_usage));
  sender::start(app);
}

fn listen_for_failures<R: tauri::Runtime>(app: &AppHandle<R>) {
  let handle = app.clone();
  app.listen(crate::events::MEDIA_DIAGNOSTIC, move |event| {
    if let Some(usage) = handle.try_state::<Usage>() {
      usage.note_diagnostic(event.payload());
    }
  });
}

/// The Settings switch. Our own flag flips here at once, before the config snapshot is swapped, so nothing
/// depends on when other readers see the new config.
fn on_config(app: &AppHandle, send_usage: bool) {
  if let Some(usage) = app.try_state::<Usage>() {
    if usage.is_on() != (send_usage && usage.configured()) {
      usage.set_on(send_usage);
    }
  }
}

/// Quitting: best-effort send of today's ping (≤ 2 s).
pub fn on_exit(app: &AppHandle) {
  sender::on_exit(app);
}

/// The item left the list (Remove, Delete): a reused ID counts its own end later.
pub fn forget_group(app: &AppHandle, group_id: &str) {
  if let Some(usage) = app.try_state::<Usage>() {
    usage.with(|state| state.forget_group(group_id));
  }
}

pub fn count(app: &AppHandle, key: &str) {
  if let Some(usage) = usage(app) {
    usage.count_keys(&[key]);
  }
}

/// The window came back from the tray or background.
pub fn opened(app: &AppHandle) {
  count(app, keys::OPEN);
}

/// UI-only events; the web view can't name any other key.
#[tauri::command]
pub fn usage_count(app: AppHandle, key: String) {
  if keys::UI_KEYS.contains(&key.as_str()) {
    count(&app, &key);
  }
}

pub fn link_received(app: &AppHandle, group_id: &str, id: &str, url: &str) {
  let Some(usage) = usage(app) else { return };
  usage.note_link(&job_id(group_id, id), url, now_ms());
}

/// Options shown for a received link; `None` engine = the read failed.
pub fn options_shown(
  app: &AppHandle,
  group_id: &str,
  id: &str,
  engine: Option<Option<FetchEngine>>,
) {
  let Some(usage) = app.try_state::<Usage>() else {
    return;
  };
  let link = usage
    .links
    .lock()
    .unwrap_or_else(|e| e.into_inner())
    .remove(&job_id(group_id, id));
  let (Some((at, url)), Some(engine)) = (link, engine) else {
    return;
  };
  if let Some(usage) = self::usage(app) {
    usage.count_keys(&options_keys(engine, at.elapsed().as_millis(), &url));
  }
}

fn options_keys(engine: Option<FetchEngine>, millis: u128, url: &str) -> Vec<String> {
  let path = match engine {
    Some(FetchEngine::Gallery) => "gallery",
    Some(FetchEngine::Files | FetchEngine::Reader) => "own",
    Some(FetchEngine::Ytdlp) | None => "ytdlp",
  };
  let mut out = vec![
    keys::FUNNEL_OPTIONS.to_string(),
    keys::options(path, millis),
  ];
  out.extend(keys::options_site(url, millis));
  out
}

/// `media_download`: one tap (or batch, or resume) for these items.
pub fn queued(
  app: &AppHandle,
  group_id: &str,
  ids: &[String],
  (origin, tap): (Origin, Option<&'static str>),
) {
  let Some(usage) = usage(app) else { return };
  let now = now_ms();
  usage.with_on(|state| {
    if let Some(tap) = tap {
      state.count(tap, now);
    }
    for id in ids {
      state.queued(&job_id(group_id, id), origin);
    }
  });
}

/// `origin` from the web view, and the tap key this call counts. "batch" = one part of a multi-link action whose
/// `funnel.batch` the web view counted once; a tap on several items (a playlist) is one batch action counted here.
pub fn origin(raw: Option<&str>, items: usize) -> (Origin, Option<&'static str>) {
  match raw {
    Some("resume") => (Origin::Resume, Some(keys::RESUME)),
    Some("batch") => (Origin::Batch, None),
    _ if items > 1 => (Origin::Batch, Some(keys::FUNNEL_BATCH)),
    _ => (Origin::Tap, Some(keys::FUNNEL_TAPPED)),
  }
}

pub fn kind(
  media_kind: Option<MediaKind>,
  track: &TrackType,
  height: Option<u32>,
  audio: Option<&str>,
) -> Kind {
  match (media_kind, track) {
    (Some(MediaKind::Image | MediaKind::Carousel), _) => Kind::Photo,
    (_, TrackType::Audio) => Kind::Audio {
      mp3: audio.is_some_and(|codec| codec.eq_ignore_ascii_case("mp3")),
    },
    _ => Kind::Video { height },
  }
}

fn entry_kind(entry: &DownloadEntry) -> Kind {
  kind(
    entry.media_kind,
    &entry.format.track_type,
    entry.format.height,
    entry.format.audio_encoding.as_deref(),
  )
}

pub fn download_started(app: &AppHandle, entry: &DownloadEntry) {
  let Some(usage) = usage(app) else { return };
  usage.start_job(
    &job_id(&entry.group_id, &entry.id),
    entry_kind(entry),
    entry.login_browser.is_some(),
  );
}

pub fn speed_sample(app: &AppHandle, group_id: &str, id: &str, bytes_per_second: Option<f64>) {
  let Some(bps) = bytes_per_second.filter(|bps| bps.is_finite() && *bps > 0.0) else {
    return;
  };
  let Some(usage) = usage(app) else { return };
  usage.note_speed(&job_id(group_id, id), bps);
}

pub fn download_ended(app: &AppHandle, entry: &DownloadEntry, end: End) {
  let Some(usage) = usage(app) else { return };
  usage.finish_job(
    &job_id(&entry.group_id, &entry.id),
    end,
    entry_kind(entry),
    &entry.url,
  );
}

fn ended_keys(
  state: &mut State,
  job: &str,
  end: End,
  kind: Kind,
  url: &str,
  speed: Option<(f64, u32)>,
  code: Option<&str>,
) -> Vec<String> {
  let mut out = state.ended(job, end, kind);
  if out.is_empty() || end == End::Cancelled {
    return out;
  }
  out.extend(keys::site(keys::SITE, url));
  if end == End::Failed {
    out.extend(keys::site(keys::SITE_FAIL, url));
    out.push(format!("{}{}", keys::FAIL_CODE, keys::failure_code(code)));
  }
  if let (End::Finished, Some((sum, n))) = (end, speed) {
    if n > 0 {
      out.push(keys::speed(sum / n as f64));
    }
  }
  out
}

pub fn engine_updated(app: &AppHandle, ok: bool) {
  let Some(usage) = usage(app) else { return };
  let now = now_ms();
  usage.with_on(|state| {
    if ok {
      state.engine_ok_at = Some(now);
      state.count(keys::ENGINE_UPDATE_OK, now);
    } else {
      state.count(keys::ENGINE_UPDATE_FAILED, now);
    }
  });
}

/// A failed update download: a bad signature is `verify`, anything else `network`.
pub fn update_failed_key(error: &tauri_plugin_updater::Error) -> &'static str {
  use tauri_plugin_updater::Error;
  match error {
    Error::Minisign(_) | Error::Base64(_) | Error::SignatureUtf8(_) => keys::UPDATE_FAILED_VERIFY,
    _ => keys::UPDATE_FAILED_NETWORK,
  }
}

pub fn update_offered(app: &AppHandle, version: &str) {
  let Some(usage) = usage(app) else { return };
  let now = now_ms();
  usage.with_on(|state| {
    if state.offered(version) {
      state.count(keys::UPDATE_OFFERED, now);
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::BTreeSet;

  /// Lesson from Android (clip.accepted never arrived): every allowed key has a producer. Keys counted straight
  /// at a call site are listed with the call site; everything else is produced by running the hook logic.
  #[test]
  fn every_allowed_fixed_key_has_a_producer() {
    let call_sites = [
      keys::OPEN,                  // setup, background::show_main
      keys::TRAY_HIDDEN,           // background::close_to_tray
      keys::SHORTCUT_USED,         // commands::shortcuts::on_quick_bar
      keys::UPDATE_TAPPED,         // updater_download
      keys::UPDATE_READY,          // updater_download
      keys::UPDATE_FAILED_INSTALL, // updater_install
      keys::FUNNEL_LINK,           // link_received
      keys::LOGIN_PROFILE,         // download_started
      keys::ACTIVE,                // snapshot
      keys::CLIP_SHOWN,            // capture clipboard watcher, card shown
      keys::CLIP_ACCEPTED,         // capture::usage_keys::action_keys, card use
      keys::QUICK_DOWNLOAD,        // capture::usage_keys::action_keys
      keys::QUICK_AUDIO,           // capture::usage_keys::action_keys
      keys::QUICK_SMALLER,         // capture::usage_keys::action_keys
      keys::QUICK_OPTIONS,         // capture::usage_keys::action_keys
      keys::CARD_FORMAT_VIDEO,     // capture::usage_keys::action_keys
      keys::CARD_FORMAT_AUDIO,     // capture::usage_keys::action_keys
    ];
    let mut made: BTreeSet<String> = call_sites.iter().map(|k| k.to_string()).collect();
    made.extend(keys::UI_KEYS.iter().map(|k| k.to_string()));
    made
      .insert(update_failed_key(&tauri_plugin_updater::Error::SignatureUtf8(String::new())).into());
    made.insert(update_failed_key(&tauri_plugin_updater::Error::ReleaseNotFound).into());

    let mut state = State::default();
    made.extend(state.on_start("1", 0).iter().map(|k| k.to_string()));
    made.extend(state.on_start("2", 0).iter().map(|k| k.to_string()));
    if state.offered("3") {
      made.insert(keys::UPDATE_OFFERED.into());
    }
    made.extend([keys::ENGINE_UPDATE_OK, keys::ENGINE_UPDATE_FAILED].map(String::from)); // engine_updated
    for origin in [Origin::Tap, Origin::Batch, Origin::Resume] {
      made.insert(
        match origin {
          Origin::Tap => keys::FUNNEL_TAPPED,
          Origin::Batch => keys::FUNNEL_BATCH,
          Origin::Resume => keys::RESUME,
        }
        .into(),
      );
    }
    let kinds = [
      [
        None,
        Some(300),
        Some(500),
        Some(800),
        Some(1080),
        Some(1440),
        Some(2160),
      ]
      .map(|height| Kind::Video { height })
      .to_vec(),
      vec![
        Kind::Audio { mp3: true },
        Kind::Audio { mp3: false },
        Kind::Photo,
      ],
    ]
    .concat();
    let mut job = 0;
    for kind in &kinds {
      made.insert(keys::download("started", *kind));
      for origin in [Origin::Tap, Origin::Batch, Origin::Resume] {
        for end in [End::Finished, End::Failed, End::Cancelled] {
          job += 1;
          let id = job.to_string();
          state.queued(&id, origin);
          for speed in [5e4, 5e5, 2e6, 9e6] {
            made.extend(ended_keys(
              &mut state,
              &id,
              end,
              *kind,
              "https://a.com/x",
              Some((speed, 1)),
              None,
            ));
            state.jobs.retain(|j| j.id != id);
            state.queued(&id, origin);
          }
        }
      }
    }
    for engine in [
      None,
      Some(FetchEngine::Gallery),
      Some(FetchEngine::Reader),
      Some(FetchEngine::Ytdlp),
    ] {
      for millis in [0, 3_000, 7_000, 20_000] {
        made.extend(options_keys(engine, millis, "https://a.com/x"));
      }
    }
    for days in [None, Some(0), Some(10), Some(40)] {
      made.insert(keys::engine_fresh(days));
      made.insert(keys::age(days.unwrap_or(1)));
    }
    let mut config = crate::state::config_models::Config::default();
    for theme in ["system", "light", "dark"] {
      for (i, language) in std::iter::once("system").chain(keys::LOCALES).enumerate() {
        config.appearance.theme = theme.into();
        config.appearance.language = language.into();
        config.performance.max_concurrency = i % 5;
        let on = i % 2 == 0;
        config.system.tray_enabled = on;
        config.input.watch_clipboard = on;
        config.system.auto_start_enabled = on;
        config.privacy.send_reports = on;
        for (os, arch) in [
          ("macos", "aarch64"),
          ("windows", "x86_64"),
          ("linux", "x86_64"),
        ] {
          made.extend(snapshot::snapshot(
            &config,
            &snapshot::Facts {
              os,
              arch,
              locale: language,
              installed_at: None,
              engine_ok_at: None,
              now: 0,
            },
          ));
        }
      }
    }
    made.insert(keys::SITE_OTHER.into()); // State::count past MAX_SITES (store tests)
    made.insert(keys::USAGE_CAPPED.into()); // State::count past MAX_PER_DAY (store tests)
    made.retain(|key| keys::open_prefix(key).is_none() && !key.starts_with(keys::FAIL_CODE));

    let missing: Vec<_> = keys::FIXED.difference(&made).collect();
    assert!(
      missing.is_empty(),
      "allowed but never produced: {missing:?}"
    );
    let unknown: Vec<_> = made.difference(&keys::FIXED).collect();
    assert!(unknown.is_empty(), "produced but not allowed: {unknown:?}");
  }

  #[test]
  fn endpoint_needs_https() {
    assert_eq!(endpoint(None), None);
    assert_eq!(endpoint(Some("")), None);
    assert_eq!(endpoint(Some("http://nezzel.example/api/usage")), None);
    assert_eq!(
      endpoint(Some(" https://nezzel.example/api/usage ")).as_deref(),
      Some("https://nezzel.example/api/usage")
    );
  }

  #[test]
  fn origin_follows_the_web_view_and_item_count() {
    assert_eq!(origin(None, 1), (Origin::Tap, Some(keys::FUNNEL_TAPPED)));
    assert_eq!(
      origin(Some("tap"), 3),
      (Origin::Batch, Some(keys::FUNNEL_BATCH))
    );
    // The web view counted funnel.batch once for the whole action.
    assert_eq!(origin(Some("batch"), 1), (Origin::Batch, None));
    assert_eq!(
      origin(Some("resume"), 4),
      (Origin::Resume, Some(keys::RESUME))
    );
  }

  #[test]
  fn kinds_follow_media_and_track() {
    assert_eq!(
      kind(
        Some(MediaKind::Carousel),
        &TrackType::Both,
        Some(1080),
        None
      ),
      Kind::Photo
    );
    assert_eq!(
      kind(None, &TrackType::Audio, None, Some("MP3")),
      Kind::Audio { mp3: true }
    );
    assert_eq!(
      kind(Some(MediaKind::Video), &TrackType::Both, Some(720), None),
      Kind::Video { height: Some(720) }
    );
  }

  #[test]
  fn options_keys_name_the_engine_and_the_site_only() {
    assert_eq!(
      options_keys(
        Some(FetchEngine::Gallery),
        6_000,
        "https://www.instagram.com/p/AbC/?igsh=x"
      ),
      vec![
        "funnel.options",
        "options.gallery.t5-10",
        "opt:t5-10:instagram.com"
      ]
    );
    assert_eq!(
      options_keys(Some(FetchEngine::Reader), 100, "https://example.org/a.mp4"),
      vec!["funnel.options", "options.own.t0-2", "opt:t0-2:example.org"]
    );
    assert_eq!(
      options_keys(None, 100, "https://10.0.0.1/a"),
      vec!["funnel.options", "options.ytdlp.t0-2"]
    );
  }

  #[test]
  fn a_finished_tap_counts_site_and_speed_and_a_failure_counts_sitefail() {
    let mut state = State::default();
    state.queued("g/1", Origin::Tap);
    let video = Kind::Video { height: Some(2160) };
    assert_eq!(
      ended_keys(
        &mut state,
        "g/1",
        End::Finished,
        video,
        "https://youtu.be/x",
        Some((6e6, 2)),
        Some("signInRequired")
      ),
      vec![
        "dl.finished.video.uhd",
        "funnel.finished",
        "site:youtu.be",
        "speed.s1m-5m"
      ]
    );
    state.queued("g/2", Origin::Resume);
    assert_eq!(
      ended_keys(
        &mut state,
        "g/2",
        End::Failed,
        Kind::Photo,
        "https://x.com/a/status/1",
        None,
        None
      ),
      vec![
        "dl.failed.photo.any",
        "resume.failed",
        "site:x.com",
        "sitefail:x.com",
        "failcode:unknown"
      ]
    );
    state.queued("g/3", Origin::Tap);
    assert_eq!(
      ended_keys(
        &mut state,
        "g/3",
        End::Cancelled,
        Kind::Photo,
        "https://x.com/a",
        None,
        Some("signInRequired")
      ),
      vec!["dl.cancelled.photo.any"]
    );
    state.queued("g/4", Origin::Tap);
    assert_eq!(
      ended_keys(
        &mut state,
        "g/4",
        End::Failed,
        Kind::Photo,
        "file:///private/photo.jpg",
        None,
        Some("notClassified")
      ),
      vec!["dl.failed.photo.any", "funnel.failed", "failcode:unknown"]
    );
  }

  #[test]
  fn download_failures_count_only_fixed_codes_once() {
    for (code, expected) in [
      (Some("signInRequired"), "failcode:signInRequired"),
      (Some("accessForbidden403"), "failcode:accessForbidden403"),
      (Some("urlNotSupported"), "failcode:urlNotSupported"),
      (Some("unknown"), "failcode:unknown"),
      (Some("unclassifiedEngineText"), "failcode:unknown"),
      (
        Some("https://secret.example/watch?v=private"),
        "failcode:unknown",
      ),
      (Some("signInRequired trailing text"), "failcode:unknown"),
      (None, "failcode:unknown"),
    ] {
      let usage = Usage::new(None, Some("https://x.test/u".into()), "0.1.0".into());
      usage.set_on(true);
      let app = tauri::test::mock_app();
      app.manage(usage);
      listen_for_failures(app.handle());
      let usage = app.state::<Usage>();
      usage.start_job("g/1", Kind::Photo, false);
      if let Some(code) = code {
        tauri::Emitter::emit(
          app.handle(),
          crate::events::MEDIA_DIAGNOSTIC,
          serde_json::json!({
            "groupId": "g", "id": "1", "level": "error", "code": code,
            "message": "private engine text", "raw": "/Users/alice/private.mp4"
          }),
        )
        .unwrap();
      }
      // A warning must not replace the failure type.
      usage.note_diagnostic(r#"{"groupId":"g","id":"1","level":"warning","code":"noAudioTrack"}"#);
      usage.finish_job("g/1", End::Failed, Kind::Photo, "https://x.com/a/status/1");
      usage.finish_job("g/1", End::Failed, Kind::Photo, "https://x.com/a/status/1");
      let state = usage.snapshot_state();
      assert_eq!(
        state.counts,
        [
          ("dl.started.photo.any".into(), 1),
          ("dl.failed.photo.any".into(), 1),
          ("site:x.com".into(), 1),
          ("sitefail:x.com".into(), 1),
          (expected.into(), 1),
        ]
        .into()
      );
      assert!(usage.failures.lock().unwrap().is_empty());
    }
  }

  #[test]
  fn links_and_speeds_are_never_kept_while_off() {
    let usage = Usage::new(None, Some("https://x.test/u".into()), "0.1.0".into());
    usage.set_on(true);
    usage.note_link("g/1", "https://youtube.com/watch?v=1", 1);
    usage.note_speed("g/1", 1e6);
    usage.start_job("g/1", Kind::Photo, false);
    assert_eq!(usage.links.lock().unwrap().len(), 1);
    usage.set_on(false);
    assert!(usage.links.lock().unwrap().is_empty() && usage.speeds.lock().unwrap().is_empty());
    // A link arriving after off (it passed the early check before the switch flipped) is refused.
    usage.note_link("g/2", "https://youtube.com/watch?v=2", 2);
    usage.note_speed("g/2", 1e6);
    assert!(usage.links.lock().unwrap().is_empty() && usage.speeds.lock().unwrap().is_empty());
    assert!(usage.snapshot_state().counts.is_empty());
    usage.note_diagnostic(r#"{"groupId":"g","id":"1","level":"error","code":"signInRequired"}"#);
    assert!(usage.failures.lock().unwrap().is_empty());
    usage.set_on(true);
    assert!(usage.links.lock().unwrap().is_empty());
  }

  #[test]
  fn usage_file_is_written_and_cleared() {
    let dir = std::env::temp_dir().join(format!("nezzel-usage-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(FILE);
    let usage = Usage::new(
      Some(path.clone()),
      Some("https://x.test/u".into()),
      "0.1.0".into(),
    );
    usage.set_on(true);
    usage.count_keys(&[keys::OPEN, "https://secret.example/watch?v=1"]);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("\"open\":1"));
    assert!(!saved.contains("secret"));
    let reloaded = Usage::new(Some(path.clone()), None, "0.1.0".into());
    assert_eq!(reloaded.snapshot_state().counts.get(keys::OPEN), Some(&1));
    usage.clear();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("\"open\""));
    std::fs::remove_dir_all(dir).unwrap();
  }
}
