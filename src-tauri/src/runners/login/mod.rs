//! Site login through a browser profile: shared vocabulary and discovery cache.
//! `session` drives one fetch/download, `registry` holds waiting requests and the
//! Tauri commands, `consent` remembers which browser a host or row may use.
mod consent;
pub(crate) mod registry;
mod session;

use super::browser_cookies::site_key;
use super::browser_logins::{discover, BrowserCandidate};
use super::browsers::BrowserKind;
use serde::Serialize;
use std::{
  collections::HashMap,
  sync::{LazyLock, Mutex},
  time::{Duration, Instant},
};

pub(crate) use consent::{forget_row_browser, row_browser};
pub use registry::{forget_group, LoginRequest};
#[cfg(test)]
pub(crate) use session::test_consented_session;
pub use session::{CookieRetry, LoginSession};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginCandidate {
  /// Engine argument internally; the wire representation is a process-local opaque token.
  #[serde(serialize_with = "serialize_candidate_id")]
  pub id: String,
  pub browser: BrowserKind,
  pub profile: String,
  pub signed_in: bool,
  pub app_bound: bool,
}
#[derive(Default)]
struct CandidateIds {
  by_engine: HashMap<String, String>,
  by_id: HashMap<String, String>,
}
static CANDIDATE_IDS: LazyLock<Mutex<CandidateIds>> =
  LazyLock::new(|| Mutex::new(CandidateIds::default()));
fn opaque_candidate_id(engine: &str) -> String {
  let mut ids = CANDIDATE_IDS.lock().unwrap();
  if let Some(id) = ids.by_engine.get(engine) {
    return id.clone();
  }
  let id = uuid::Uuid::new_v4().to_string();
  ids.by_engine.insert(engine.into(), id.clone());
  ids.by_id.insert(id.clone(), engine.into());
  id
}
fn serialize_candidate_id<S: serde::Serializer>(
  engine: &str,
  serializer: S,
) -> Result<S::Ok, S::Error> {
  serializer.serialize_str(&opaque_candidate_id(engine))
}
fn resolve_candidate_id(id: &str) -> Result<String, String> {
  CANDIDATE_IDS
    .lock()
    .unwrap()
    .by_id
    .get(id)
    .cloned()
    .ok_or_else(|| "Unknown browser selection".into())
}

impl From<BrowserCandidate> for LoginCandidate {
  fn from(c: BrowserCandidate) -> Self {
    Self {
      id: c.id,
      browser: c.browser,
      profile: c.profile,
      signed_in: c.signed_in,
      app_bound: c.app_bound,
    }
  }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LoginStatus {
  Needed,
  Exhausted,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginNeeded {
  pub request_id: String,
  pub id: String,
  pub group_id: String,
  pub host: String,
  pub reason: String,
  /// Browser whose site session is app-bound, when discovery identified one.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub blocked_browser: Option<BrowserKind>,
  pub status: LoginStatus,
  pub candidates: Vec<LoginCandidate>,
  pub change_only: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginUsing {
  pub id: String,
  pub group_id: String,
  pub host: String,
  pub candidate: LoginCandidate,
  pub selected_only: bool,
}
#[derive(Clone)]
struct CachedDiscovery {
  candidates: Vec<LoginCandidate>,
  created: Instant,
}
static DISCOVERY: LazyLock<Mutex<HashMap<String, CachedDiscovery>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
fn cached_discovery(
  cache: &mut HashMap<String, CachedDiscovery>,
  host: &str,
  now: Instant,
  find: impl FnOnce() -> Vec<LoginCandidate>,
) -> Vec<LoginCandidate> {
  let host = site_key(host).unwrap_or_else(|| host.into());
  cache.retain(|_, entry| now.saturating_duration_since(entry.created) < LOGIN_TTL);
  if let Some(entry) = cache.get(&host) {
    return entry.candidates.clone();
  }
  if cache.len() >= 128 {
    if let Some(oldest) = cache
      .iter()
      .min_by_key(|(_, entry)| entry.created)
      .map(|(host, _)| host.clone())
    {
      cache.remove(&oldest);
    }
  }
  let candidates = find();
  cache.insert(
    host,
    CachedDiscovery {
      candidates: candidates.clone(),
      created: now,
    },
  );
  candidates
}
fn discovered_candidates(host: &str, remembered: Option<&str>) -> Vec<LoginCandidate> {
  let mut candidates =
    cached_discovery(&mut DISCOVERY.lock().unwrap(), host, Instant::now(), || {
      discover(None, host).into_iter().map(Into::into).collect()
    });
  // Stable sorting preserves the cached Chromium/Firefox recency order. Remembered
  // choices affect only picker ordering; cached metadata never grants consent.
  candidates.sort_by_key(|candidate| {
    if candidate.app_bound {
      4
    } else if candidate.signed_in {
      0
    } else if candidate.browser == BrowserKind::Safari {
      1
    } else if remembered == Some(candidate.id.as_str()) {
      2
    } else {
      3
    }
  });
  candidates
}
/// How long a login request waits for the user; the webview forgets it after the same time (`crate::contract`).
pub(crate) const LOGIN_TTL: Duration = Duration::from_secs(900);
/// Internal engine override, carried through gallery/video handoffs; never a profile choice.
pub(super) const ANONYMOUS_BROWSER: &str = "none";
/// Login reason: every signed-in profile is behind Windows app-bound encryption (`crate::contract`).
pub(crate) const APP_BOUND_REASON: &str = "browserAppBound";

/// Login reason: the user asked to change the profile of a row that already used one.
const CHANGE_PROFILE_REASON: &str = "changeProfile";
/// Login reason: no profile worked and the site gave no more specific code.
const LOGIN_FAILED_REASON: &str = "browserLoginFailed";
/// The reasons the engine itself gives; any other reason is the site's diagnostic code (`crate::contract`).
#[cfg(test)]
pub(crate) const LOGIN_REASONS: [&str; 3] =
  [APP_BOUND_REASON, CHANGE_PROFILE_REASON, LOGIN_FAILED_REASON];

#[cfg(test)]
mod cookie_tests;
#[cfg(test)]
mod tests;
