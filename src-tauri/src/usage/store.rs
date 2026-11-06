//! Local counters since the last good send, kept in `usage.json` in the app data folder. Pure: every rule here is
//! tested without a clock, a file or the network. Counts are never logged.

use super::keys;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DAY_MS: u64 = 86_400_000;
pub const MAX_AGE_MS: u64 = 7 * DAY_MS;
/// The server's limits.
pub const MAX_KEYS: usize = 200;
pub const MAX_N: u32 = 10_000;
pub const MAX_BODY: usize = 16_000;
/// Android's MAX_SITES, per open prefix between sends.
pub const MAX_SITES: usize = 50;
/// Desktop-only client limit: one key counts at most this many times per UTC day (Android's 11,358-cancel loop).
pub const MAX_PER_DAY: u32 = 500;
/// Download jobs remembered so a retried, resumed or replayed job never counts a second end.
pub const MAX_JOBS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
  /// One Download on a single item: its end counts under `funnel.*`.
  Tap,
  /// A playlist or several links: its ends count under `dl.*` only.
  Batch,
  /// A paused or stopped job started again: its end counts under `resume.*`.
  Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
  Finished,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Job {
  pub id: String,
  pub origin: Option<Origin>,
  #[serde(default)]
  pub started: bool,
  #[serde(default)]
  pub cancelled: bool,
  #[serde(default)]
  pub ended: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
  /// When the oldest unsent count was made (ms); counters older than 7 days are dropped.
  pub since: Option<u64>,
  /// The UTC day of the last good send.
  pub usage_day: Option<u64>,
  pub counts: BTreeMap<String, u32>,
  /// Per-key counts on `today_day`, for the daily cap.
  pub today_day: u64,
  pub today: BTreeMap<String, u32>,
  pub installed_at: Option<u64>,
  pub last_version: Option<String>,
  pub offered_version: Option<String>,
  /// Last good yt-dlp update (ms).
  pub engine_ok_at: Option<u64>,
  /// Oldest first.
  pub jobs: Vec<Job>,
}

impl State {
  /// Counts `key` once. Unknown keys are dropped (a hook must never break a download). Returns true when counted.
  pub fn count(&mut self, key: &str, now: u64) -> bool {
    if !keys::allowed(key) {
      return false;
    }
    let key = match keys::open_prefix(key) {
      Some(prefix) if !self.counts.contains_key(key) && self.open_keys(prefix) >= MAX_SITES => {
        if prefix == keys::SITE {
          keys::SITE_OTHER
        } else {
          return false;
        }
      }
      _ => key,
    };
    let day = now / DAY_MS;
    if self.today_day != day {
      self.today_day = day;
      self.today.clear();
    }
    let today = self.today.get(key).copied().unwrap_or(0);
    if today >= MAX_PER_DAY {
      if key != keys::USAGE_CAPPED && !self.today.contains_key(keys::USAGE_CAPPED) {
        self.add(keys::USAGE_CAPPED, now);
      }
      return false;
    }
    self.add(key, now);
    true
  }

  fn add(&mut self, key: &str, now: u64) {
    self.since.get_or_insert(now);
    let n = self.counts.entry(key.to_string()).or_insert(0);
    *n = (*n + 1).min(MAX_N);
    *self.today.entry(key.to_string()).or_insert(0) += 1;
  }

  fn open_keys(&self, prefix: &str) -> usize {
    self
      .counts
      .keys()
      .filter(|key| key.starts_with(prefix))
      .count()
  }

  /// Counters unsent for 7 days (or dated in the future by a clock jump) are dropped.
  pub fn expire(&mut self, now: u64) {
    // Keys a newer build no longer allows would sit unsent forever.
    self.counts.retain(|key, n| *n > 0 && keys::allowed(key));
    if self.counts.is_empty() {
      self.since = None;
    }
    if let Some(since) = self.since {
      if now.saturating_sub(since) > MAX_AGE_MS || since > now + MAX_AGE_MS {
        self.counts.clear();
        self.since = None;
      }
    }
  }

  /// Switch off: drop every count, keep only the dates that say what this install already did.
  pub fn clear(&mut self) {
    *self = State {
      usage_day: self.usage_day,
      installed_at: self.installed_at,
      last_version: self.last_version.take(),
      offered_version: self.offered_version.take(),
      engine_ok_at: self.engine_ok_at,
      ..State::default()
    };
  }

  /// After the server's 204: subtract what was sent (counts added meanwhile stay).
  /// `once` = the ping's once-keys; a counted key that clashes with one is never sent, so it goes too.
  pub fn sent(&mut self, sent: &BTreeMap<String, u32>, once: &[String], now: u64) {
    self.counts.retain(|key, _| !once.contains(key));
    for (key, n) in sent {
      if let Some(left) = self.counts.get_mut(key) {
        *left = left.saturating_sub(*n);
        if *left == 0 {
          self.counts.remove(key);
        }
      }
    }
    // Leftovers (past the 200-key / 16 KB cut) keep their age, so they still expire after 7 days.
    if self.counts.is_empty() {
      self.since = None;
    }
    self.usage_day = Some(now / DAY_MS);
  }

  pub fn due(&self, now: u64) -> bool {
    self.usage_day != Some(now / DAY_MS)
  }

  /// First start of this install and first start of a new version. Returns the keys to count.
  pub fn on_start(&mut self, version: &str, now: u64) -> Vec<&'static str> {
    let mut out = Vec::new();
    if self.installed_at.is_none() {
      self.installed_at = Some(now);
      out.push(keys::INSTALL_NEW);
    } else if self.last_version.as_deref() != Some(version) {
      out.push(keys::UPDATE_INSTALLED);
    }
    self.last_version = Some(version.to_string());
    out
  }

  /// An update offered: true once per version.
  pub fn offered(&mut self, version: &str) -> bool {
    if self.offered_version.as_deref() == Some(version) {
      return false;
    }
    self.offered_version = Some(version.to_string());
    true
  }

  fn job(&mut self, id: &str) -> &mut Job {
    if let Some(at) = self.jobs.iter().position(|job| job.id == id) {
      return &mut self.jobs[at];
    }
    if self.jobs.len() >= MAX_JOBS {
      self.jobs.remove(0);
    }
    self.jobs.push(Job {
      id: id.to_string(),
      ..Job::default()
    });
    self.jobs.last_mut().expect("just pushed")
  }

  /// The item left the list, or its link is read again (Retry reuses the ID): its next end counts again.
  pub fn forget_job(&mut self, id: &str) {
    self.jobs.retain(|job| job.id != id);
  }

  pub fn forget_group(&mut self, group_id: &str) {
    let prefix = format!("{group_id}/");
    self.jobs.retain(|job| !job.id.starts_with(&prefix));
  }

  /// A job handed to the download queue. A resume re-labels it; otherwise the first origin stays.
  pub fn queued(&mut self, id: &str, origin: Origin) {
    let job = self.job(id);
    if origin == Origin::Resume || job.origin.is_none() {
      job.origin = Some(origin);
    }
  }

  /// True the first time a job starts.
  pub fn started(&mut self, id: &str) -> bool {
    let job = self.job(id);
    !std::mem::replace(&mut job.started, true)
  }

  /// The keys a job's end counts: at most one stop and one final end per job, ever.
  pub fn ended(&mut self, id: &str, end: End, kind: keys::Kind) -> Vec<String> {
    let job = self.job(id);
    if job.ended {
      return Vec::new();
    }
    let event = match end {
      End::Cancelled => {
        if std::mem::replace(&mut job.cancelled, true) {
          return Vec::new();
        }
        return vec![keys::download("cancelled", kind)];
      }
      End::Finished => "finished",
      End::Failed => "failed",
    };
    job.ended = true;
    let mut out = vec![keys::download(event, kind)];
    match (job.origin, end) {
      (Some(Origin::Tap), End::Finished) => out.push(keys::FUNNEL_FINISHED.into()),
      (Some(Origin::Tap), End::Failed) => out.push(keys::FUNNEL_FAILED.into()),
      (Some(Origin::Resume), End::Finished) => out.push(keys::RESUME_FINISHED.into()),
      (Some(Origin::Resume), End::Failed) => out.push(keys::RESUME_FAILED.into()),
      _ => {}
    }
    out
  }
}

/// The ping body: once-keys (value 1) first, then fixed keys, then open keys by count, at most 200 keys and
/// 16 KB. Returns the counted keys it carries (to subtract after a 204) and the JSON.
pub fn payload(
  counts: &BTreeMap<String, u32>,
  once: &[String],
  version: &str,
) -> (BTreeMap<String, u32>, String) {
  let mut once: Vec<&String> = once.iter().filter(|key| keys::allowed(key)).collect();
  once.dedup();
  let (mut open, fixed): (Vec<_>, Vec<_>) = counts
    .iter()
    .filter(|(key, n)| **n > 0 && keys::allowed(key) && !once.contains(key))
    .partition(|(key, _)| keys::open_prefix(key).is_some());
  open.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
  let room = MAX_KEYS.saturating_sub(once.len());
  let mut sent: Vec<(&String, &u32)> = fixed.into_iter().chain(open).take(room).collect();
  loop {
    let mut map = serde_json::Map::new();
    for key in &once {
      map.insert((*key).clone(), 1.into());
    }
    for (key, n) in &sent {
      map.insert((*key).clone(), (**n).clamp(1, MAX_N).into());
    }
    let body = serde_json::json!({ "platform": "desktop", "app_version": version, "counts": map })
      .to_string();
    if body.len() <= MAX_BODY || sent.is_empty() {
      let sent = sent.into_iter().map(|(k, n)| (k.clone(), *n)).collect();
      return (sent, body);
    }
    sent.pop();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::usage::keys::Kind;

  const T: u64 = 20_000 * DAY_MS + 3_600_000;
  const VIDEO: Kind = Kind::Video { height: Some(1080) };

  #[test]
  fn unknown_keys_are_dropped_and_known_ones_counted() {
    let mut state = State::default();
    assert!(!state.count("https://youtube.com/watch?v=1", T));
    assert!(!state.count("title:My video", T));
    assert!(state.count(keys::OPEN, T));
    assert!(state.count(keys::OPEN, T));
    assert_eq!(state.counts.get(keys::OPEN), Some(&2));
    assert_eq!(state.counts.len(), 1);
    assert_eq!(state.since, Some(T));
  }

  #[test]
  fn a_key_counts_at_most_500_a_day_then_usage_capped_once() {
    let mut state = State::default();
    for _ in 0..11_358 {
      state.count("dl.cancelled.video.fhd", T);
    }
    assert_eq!(state.counts["dl.cancelled.video.fhd"], MAX_PER_DAY);
    assert_eq!(state.counts[keys::USAGE_CAPPED], 1);
    // The next UTC day counts again.
    assert!(state.count("dl.cancelled.video.fhd", T + DAY_MS));
    assert_eq!(state.counts["dl.cancelled.video.fhd"], MAX_PER_DAY + 1);
  }

  #[test]
  fn sites_past_50_go_to_site_other_and_other_open_keys_are_dropped() {
    let mut state = State::default();
    for i in 0..60 {
      state.count(&format!("site:s{i}.com"), T);
      state.count(&format!("sitein:s{i}.com"), T);
    }
    assert_eq!(state.open_keys(keys::SITE), MAX_SITES);
    assert_eq!(state.open_keys(keys::SITE_IN), MAX_SITES);
    assert_eq!(state.counts[keys::SITE_OTHER], 10);
    // A site already counted still counts up.
    assert!(state.count("site:s0.com", T));
    assert_eq!(state.counts["site:s0.com"], 2);
  }

  #[test]
  fn counts_older_than_7_days_are_dropped_before_a_send() {
    let mut state = State::default();
    state.count(keys::OPEN, T);
    state.expire(T + MAX_AGE_MS);
    assert_eq!(state.counts.len(), 1);
    state.expire(T + MAX_AGE_MS + 1);
    assert!(state.counts.is_empty());
    assert_eq!(state.since, None);
  }

  #[test]
  fn a_sent_ping_subtracts_only_what_it_carried_and_marks_the_day() {
    let mut state = State::default();
    state.count(keys::OPEN, T);
    state.count(keys::OPEN, T);
    let (sent, _) = payload(&state.counts, &[], "0.1.0");
    state.count(keys::OPEN, T + 5);
    state.sent(&sent, &[], T + 10);
    assert_eq!(state.counts.get(keys::OPEN), Some(&1));
    assert!(!state.due(T + 10));
    assert!(state.due(T + DAY_MS));
  }

  #[test]
  fn switch_off_drops_counts_and_jobs_but_keeps_install_dates() {
    let mut state = State::default();
    state.on_start("0.1.0", T);
    state.count(keys::OPEN, T);
    state.queued("g/1", Origin::Tap);
    state.usage_day = Some(5);
    state.clear();
    assert!(state.counts.is_empty() && state.jobs.is_empty() && state.since.is_none());
    assert_eq!(state.installed_at, Some(T));
    assert_eq!(state.last_version.as_deref(), Some("0.1.0"));
    assert_eq!(state.usage_day, Some(5));
  }

  #[test]
  fn install_new_once_then_update_installed_once_per_version() {
    let mut state = State::default();
    assert_eq!(state.on_start("0.1.0", T), vec![keys::INSTALL_NEW]);
    assert!(state.on_start("0.1.0", T + 1).is_empty());
    assert_eq!(state.on_start("0.2.0", T + 2), vec![keys::UPDATE_INSTALLED]);
    assert!(state.on_start("0.2.0", T + 3).is_empty());
    assert!(state.offered("0.3.0"));
    assert!(!state.offered("0.3.0"));
  }

  #[test]
  fn a_job_ends_once_and_its_funnel_follows_the_tap_that_started_it() {
    let mut state = State::default();
    state.queued("g/1", Origin::Tap);
    assert!(state.started("g/1"));
    assert!(!state.started("g/1"));
    assert_eq!(
      state.ended("g/1", End::Finished, VIDEO),
      vec!["dl.finished.video.fhd", keys::FUNNEL_FINISHED]
    );
    // A replayed end (retry, login resume, duplicate event) counts nothing.
    assert!(state.ended("g/1", End::Finished, VIDEO).is_empty());
    assert!(state.ended("g/1", End::Failed, VIDEO).is_empty());

    state.queued("g/2", Origin::Batch);
    assert_eq!(
      state.ended("g/2", End::Failed, VIDEO),
      vec!["dl.failed.video.fhd"]
    );
  }

  #[test]
  fn a_paused_then_resumed_job_counts_one_stop_and_its_end_under_resume() {
    let mut state = State::default();
    state.queued("g/1", Origin::Tap);
    assert_eq!(
      state.ended("g/1", End::Cancelled, VIDEO),
      vec!["dl.cancelled.video.fhd"]
    );
    state.queued("g/1", Origin::Resume);
    assert!(state.ended("g/1", End::Cancelled, VIDEO).is_empty());
    assert_eq!(
      state.ended("g/1", End::Failed, VIDEO),
      vec!["dl.failed.video.fhd", keys::RESUME_FAILED]
    );
  }

  #[test]
  fn leftovers_past_the_cut_keep_their_age_and_still_expire() {
    // Older counters (e.g. from a build with other caps) can exceed one ping.
    let mut state = State {
      since: Some(T),
      ..State::default()
    };
    for i in 0..300 {
      state.counts.insert(format!("site:s{i}.com"), 1);
    }
    let (sent, _) = payload(&state.counts, &[], "0.1.0");
    state.sent(&sent, &[], T + DAY_MS);
    assert!(!state.counts.is_empty());
    assert_eq!(state.since, Some(T));
    state.expire(T + MAX_AGE_MS + 1);
    assert!(state.counts.is_empty());
  }

  #[test]
  fn keys_that_can_never_be_sent_are_dropped() {
    let mut state = State::default();
    state.count(keys::OPEN, T);
    state.counts.insert("removed.in.a.newer.build".into(), 4);
    state.counts.insert("os.macos".into(), 2);
    state.expire(T + 1);
    assert!(!state.counts.contains_key("removed.in.a.newer.build"));
    let once = vec!["os.macos".to_string()];
    let (sent, _) = payload(&state.counts, &once, "0.1.0");
    state.sent(&sent, &once, T + 2);
    assert!(state.counts.is_empty());
  }

  #[test]
  fn a_forgotten_job_counts_its_next_end() {
    let mut state = State::default();
    state.queued("g/1", Origin::Tap);
    assert!(!state.ended("g/1", End::Finished, VIDEO).is_empty());
    state.forget_job("g/1");
    state.queued("g/1", Origin::Tap);
    assert!(!state.ended("g/1", End::Failed, VIDEO).is_empty());
    state.queued("g/2", Origin::Tap);
    state.queued("h/1", Origin::Tap);
    state.forget_group("g");
    assert_eq!(
      state.jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
      vec!["h/1"]
    );
  }

  #[test]
  fn jobs_are_capped() {
    let mut state = State::default();
    for i in 0..(MAX_JOBS + 20) {
      state.queued(&format!("g/{i}"), Origin::Batch);
    }
    assert_eq!(state.jobs.len(), MAX_JOBS);
    assert_eq!(state.jobs[0].id, "g/20");
  }

  #[test]
  fn payload_matches_androids_shape_plus_platform() {
    let mut state = State::default();
    state.count(keys::OPEN, T);
    state.count("site:youtube.com", T);
    state.count("dl.finished.video.fhd", T);
    let once = vec![keys::ACTIVE.to_string(), "os.macos".into(), "nope".into()];
    let (sent, body) = payload(&state.counts, &once, "0.1.0-beta.1");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["platform"], "desktop");
    assert_eq!(json["app_version"], "0.1.0-beta.1");
    assert_eq!(json["counts"]["active"], 1);
    assert_eq!(json["counts"]["os.macos"], 1);
    assert_eq!(json["counts"]["open"], 1);
    assert_eq!(json["counts"]["site:youtube.com"], 1);
    assert!(json["counts"].get("nope").is_none());
    assert_eq!(json.as_object().unwrap().len(), 3);
    assert_eq!(sent.len(), 3);
  }

  #[test]
  fn payload_keeps_fixed_keys_first_and_cuts_at_200_keys_and_16_kb() {
    let mut counts = BTreeMap::new();
    counts.insert(keys::OPEN.to_string(), 3);
    for i in 0..300 {
      counts.insert(format!("opt:t10up:{}{i}.com", "a".repeat(55)), 1 + i);
    }
    let once = vec![keys::ACTIVE.to_string()];
    let (sent, body) = payload(&counts, &once, "0.1.0");
    assert!(body.len() <= MAX_BODY);
    assert!(sent.len() < MAX_KEYS);
    assert!(sent.contains_key(keys::OPEN));
    // The most counted open keys win.
    assert!(sent.contains_key(&format!("opt:t10up:{}299.com", "a".repeat(55))));
    assert!(!sent.contains_key(&format!("opt:t10up:{}0.com", "a".repeat(55))));

    let small: BTreeMap<String, u32> = (0..300).map(|i| (format!("site:s{i}.com"), 1)).collect();
    let (sent, _) = payload(&small, &once, "0.1.0");
    assert_eq!(sent.len(), MAX_KEYS - 1);
  }

  #[test]
  fn state_survives_a_round_trip_and_old_files_load() {
    let mut state = State::default();
    state.count(keys::OPEN, T);
    state.queued("g/1", Origin::Resume);
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(serde_json::from_str::<State>(&json).unwrap(), state);
    assert_eq!(
      serde_json::from_str::<State>("{}").unwrap(),
      State::default()
    );
  }
}
