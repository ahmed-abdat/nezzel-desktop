use super::browser_cookies::{known_login_host, migrate_site_browsers, site_key};
use super::browser_logins::{describe, discover, BrowserCandidate};
use super::override_resolver::resolve_with_patch;
use super::retry_policy::{RetryAction, RetryFailure, RetryState};
use crate::extract::{outcome::Attempt, policy::Phase};
use crate::models::download::DownloadOverrides;
use crate::models::error::{DiagnosticEvent, DiagnosticLevel};
use crate::models::{MediaDiagnosticPayload, MediaFatalPayload};
use crate::parsers::ytdlp_error::CookieFetchFailure;
use crate::scheduling::{
  dispatcher::DispatchRequest,
  download_pipeline::{DownloadEntry, DownloadRequest, DownloadSender},
  fetch_pipeline::{FetchEntry, FetchRequest, FetchSender},
  group_state::{is_group_running, subscribe_group},
};
use crate::state::config_models::AuthSettings;
use crate::SharedConfig;
use serde::Serialize;
use std::{
  collections::{HashMap, HashSet},
  sync::{LazyLock, Mutex},
  time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginCandidate {
  /// Engine argument internally; the wire representation is a process-local opaque token.
  #[serde(serialize_with = "serialize_candidate_id")]
  pub id: String,
  pub browser: String,
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
struct ProfileChoice {
  used: LoginUsing,
  candidates: Vec<LoginCandidate>,
  created: Instant,
}
static USED: LazyLock<Mutex<HashMap<String, (LoginUsing, Instant)>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
static CHOICES: LazyLock<Mutex<HashMap<String, ProfileChoice>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
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
    } else if candidate.browser == "safari" {
      1
    } else if remembered == Some(candidate.id.as_str()) {
      2
    } else {
      3
    }
  });
  candidates
}

fn announce_using<R: tauri::Runtime>(app: &AppHandle<R>, used: LoginUsing) {
  {
    let mut map = USED.lock().unwrap();
    map.retain(|_, (_, created)| created.elapsed() < LOGIN_TTL);
    if map.len() >= 64 {
      if let Some(oldest) = map
        .iter()
        .min_by_key(|(_, (_, created))| *created)
        .map(|(key, _)| key.clone())
      {
        map.remove(&oldest);
      }
    }
    map.insert(used.group_id.clone(), (used.clone(), Instant::now()));
  }
  let _ = app.emit("media_login_using", used);
}

/// Explicit user action on a row that has already used site login. Never a background scan.
#[tauri::command]
pub async fn media_login_choices(group_id: String) -> Result<LoginNeeded, String> {
  tauri::async_runtime::spawn_blocking(move || choices_for_group(group_id))
    .await
    .map_err(|_| "Browser discovery failed".to_owned())?
}
fn choices_for_group(group_id: String) -> Result<LoginNeeded, String> {
  let used = USED
    .lock()
    .unwrap()
    .get(&group_id)
    .filter(|(_, created)| created.elapsed() < LOGIN_TTL)
    .map(|(used, _)| used.clone())
    .ok_or("Login selection expired")?;
  // An explicit Change action refreshes metadata after the user signs in elsewhere.
  DISCOVERY.lock().unwrap().remove(&used.host);
  let candidates = discovered_candidates(&used.host, Some(&used.candidate.id));
  let blocked = candidates.iter().any(|c| c.app_bound);
  let candidates: Vec<_> = candidates.into_iter().filter(|c| !c.app_bound).collect();
  let request_id = uuid::Uuid::new_v4().to_string();
  let payload = LoginNeeded {
    request_id: request_id.clone(),
    id: used.id.clone(),
    group_id: used.group_id.clone(),
    host: used.host.clone(),
    reason: if blocked {
      "browserAppBound".into()
    } else {
      "changeProfile".into()
    },
    status: LoginStatus::Needed,
    candidates: candidates.clone(),
    change_only: true,
  };
  let mut map = CHOICES.lock().unwrap();
  map.retain(|_, choice| choice.created.elapsed() < LOGIN_TTL);
  if map.len() >= 64 {
    return Err("Too many browser choices".into());
  }
  map.insert(
    request_id,
    ProfileChoice {
      used,
      candidates,
      created: Instant::now(),
    },
  );
  Ok(payload)
}
fn claim_choice(
  map: &mut HashMap<String, ProfileChoice>,
  request_id: &str,
  browser_id: &str,
) -> Result<ProfileChoice, String> {
  let choice = map.get(request_id).ok_or("Login selection expired")?;
  if choice.created.elapsed() >= LOGIN_TTL
    || !choice
      .candidates
      .iter()
      .any(|c| c.id == browser_id && !c.app_bound)
  {
    return Err("Browser was not offered for this selection".into());
  }
  Ok(map.remove(request_id).unwrap())
}

#[derive(Clone)]
pub enum LoginRequest {
  Fetch(FetchEntry),
  Download(DownloadEntry),
}
impl LoginRequest {
  fn ids(&self) -> (&str, &str) {
    match self {
      Self::Fetch(e) => (&e.id, &e.group_id),
      Self::Download(e) => (&e.id, &e.group_id),
    }
  }
  fn fatal_payload(&self, message: &str) -> MediaFatalPayload {
    let (id, group) = self.ids();
    MediaFatalPayload::internal(group.into(), id.into(), message.into(), None)
  }
  fn timeout_payloads(&self) -> (MediaDiagnosticPayload, MediaFatalPayload) {
    let (id, group) = self.ids();
    let mut fatal = MediaFatalPayload::with_exit(
      group.into(),
      id.into(),
      1,
      "Browser login request timed out".into(),
    );
    fatal.details = Some("loginConsentTimeout".into());
    let diagnostic = MediaDiagnosticPayload {
      id: id.into(),
      group_id: group.into(),
      level: DiagnosticLevel::Error,
      code: "loginConsentTimeout".into(),
      component: Some("login".into()),
      message: fatal.message.clone(),
      raw: "loginConsentTimeout".into(),
      timestamp: fatal.timestamp,
    };
    (diagnostic, fatal)
  }
  fn fail<R: tauri::Runtime>(&self, app: &AppHandle<R>, message: &str) {
    if message.contains("timed out") || message.contains("expired") {
      let (diagnostic, fatal) = self.timeout_payloads();
      let _ = app.emit("media_diagnostic", diagnostic);
      let _ = app.emit("media_fatal", fatal);
    } else {
      let _ = app.emit("media_fatal", self.fatal_payload(message));
    }
    self.release(app);
  }
  fn host(&self) -> Option<String> {
    let url = match self {
      Self::Fetch(e) => &e.url,
      Self::Download(e) => &e.url,
    };
    reqwest::Url::parse(url).ok().and_then(|u| {
      u.host_str()
        .map(|host| site_key(host).unwrap_or_else(|| host.to_owned()))
    })
  }
  fn release<R: tauri::Runtime>(&self, app: &AppHandle<R>) {
    let (_, group) = self.ids();
    match self {
      Self::Fetch(_) => {
        crate::scheduling::fetch_pipeline::finish_fetch(&app.state::<FetchSender>().0, group)
      }
      Self::Download(entry) => {
        if let Some(library) = app.try_state::<crate::library::Library>() {
          library.forget_item(group, &entry.id);
        }
        crate::scheduling::download_pipeline::finish_download(
          &app.state::<DownloadSender>().0,
          group,
          &entry.id,
          false,
        );
      }
    }
  }

  fn consent_to_browser(&mut self, browser: &str) {
    let (id, group) = self.ids();
    if let Some(host) = self.host() {
      remember_row_browser(group, id, &host, browser);
    }
    match self {
      Self::Fetch(entry) => {
        entry.login_browser = Some(browser.into());
        entry.login_was_accepted = true;
      }
      Self::Download(entry) => {
        entry.login_browser = Some(browser.into());
        entry.login_was_accepted = true;
      }
    }
  }
  fn enqueue<R: tauri::Runtime>(
    mut self,
    app: &AppHandle<R>,
    browser: String,
  ) -> Result<(), String> {
    let (_, group) = self.ids();
    if !is_group_running(group) {
      return Err("Login request cancelled".into());
    }
    self.consent_to_browser(&browser);
    match &mut self {
      Self::Fetch(e) => app
        .state::<FetchSender>()
        .0
        .send(DispatchRequest::Pipeline(FetchRequest::Resume(Box::new(
          e.clone(),
        ))))
        .map_err(|e| e.to_string()),
      Self::Download(e) => app
        .state::<DownloadSender>()
        .0
        .send(DispatchRequest::Pipeline(DownloadRequest::Resume(
          Box::new(e.clone()),
        )))
        .map_err(|e| e.to_string()),
    }
  }
}
struct PendingLogin {
  followers: Vec<PendingLogin>,
  announcement: Option<(AppHandle, LoginNeeded)>,
  request: LoginRequest,
  candidates: Vec<LoginCandidate>,
  cancel: watch::Receiver<bool>,
  created: Instant,
  cleanup: CleanupChannels,
}
impl PendingLogin {
  fn into_rows(mut self) -> Vec<Self> {
    let mut rows = std::mem::take(&mut self.followers);
    rows.insert(0, self);
    rows
  }
}

/// Same-host waiting rows share one offered profile list and one consent token.
fn queue_pending(
  map: &mut HashMap<String, PendingLogin>,
  token: String,
  pending: PendingLogin,
) -> bool {
  let host = pending.request.host();
  if let Some(existing) = map.values_mut().find(|existing| {
    existing.created.elapsed() < LOGIN_TTL && host.is_some() && existing.request.host() == host
  }) {
    existing.followers.push(pending);
    false
  } else {
    map.insert(token, pending);
    true
  }
}

struct CleanupChannels {
  fetch: FetchSender,
  download: DownloadSender,
}
impl CleanupChannels {
  fn send(&self, group_id: &str) {
    let _ = self.fetch.0.send(DispatchRequest::Cleanup {
      group_id: group_id.into(),
    });
    let _ = self.download.0.send(DispatchRequest::Cleanup {
      group_id: group_id.into(),
    });
  }
}
const LOGIN_TTL: Duration = Duration::from_secs(900);
// Legacy unknown-host and Safari entries may have come from old automatic selection.
// A current picker choice authorizes reusing those entries for fetch -> download.
static EXPLICIT_SITES: LazyLock<Mutex<HashSet<String>>> =
  LazyLock::new(|| Mutex::new(HashSet::new()));
fn may_reuse_remembered(auth: &AuthSettings, host: &str) -> bool {
  (known_login_host(host) && auth.site_browsers.get(host).map(String::as_str) != Some("safari"))
    || auth.always_use_browser_logins
    || EXPLICIT_SITES.lock().unwrap().contains(host)
}

struct RowBrowser {
  browser: String,
  host: String,
}
static ROW_BROWSERS: LazyLock<Mutex<HashMap<(String, String), RowBrowser>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
fn remember_row_browser(group: &str, id: &str, host: &str, browser: &str) {
  let mut rows = ROW_BROWSERS.lock().unwrap();
  rows.insert(
    (group.into(), id.into()),
    RowBrowser {
      browser: browser.into(),
      host: site_key(host).unwrap_or_else(|| host.into()),
    },
  );
}
pub(crate) fn row_browser(group: &str, id: &str, url: &str) -> Option<String> {
  let url = reqwest::Url::parse(url).ok()?;
  let host = site_key(url.host_str()?).unwrap_or_else(|| url.host_str().unwrap_or_default().into());
  ROW_BROWSERS
    .lock()
    .unwrap()
    .get(&(group.into(), id.into()))
    .filter(|choice| choice.host == host)
    .map(|choice| choice.browser.clone())
}

/// Download termination releases row consent; fetch Cleanup must preserve the handoff.
pub(crate) fn forget_row_browser(group: &str, id: &str) {
  ROW_BROWSERS
    .lock()
    .unwrap()
    .remove(&(group.into(), id.into()));
}

static CONSENT_WRITES: Mutex<()> = Mutex::new(());

/// The deadline is injectable so timeout effects are tested without wall-clock sleeps.
async fn expire_pending(
  request_id: &str,
  deadline: impl std::future::Future<Output = ()>,
  mut emit: impl FnMut(MediaDiagnosticPayload, MediaFatalPayload),
  mut release: impl FnMut(LoginRequest),
) {
  deadline.await;
  let pending = PENDING.lock().unwrap().remove(request_id);
  if let Some(pending) = pending {
    for row in pending.into_rows() {
      if *row.cancel.borrow() {
        let (diagnostic, fatal) = row.request.timeout_payloads();
        emit(diagnostic, fatal);
      }
      release(row.request);
    }
  }
}
fn schedule_expiry(app: AppHandle, request_id: String) {
  tauri::async_runtime::spawn(async move {
    expire_pending(
      &request_id,
      tokio::time::sleep(LOGIN_TTL),
      |diagnostic, fatal| {
        let _ = app.emit("media_diagnostic", diagnostic);
        let _ = app.emit("media_fatal", fatal);
      },
      |request| request.release(&app),
    )
    .await;
  });
}

static PENDING: LazyLock<Mutex<HashMap<String, PendingLogin>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

/// Internal engine override, carried through gallery/video handoffs; never a profile choice.
pub(super) const ANONYMOUS_BROWSER: &str = "none";

#[derive(Clone)]
pub struct LoginSession {
  fetch_managed: bool,
  fetch_browser: Option<String>,
  fetch_gate_used: bool,
  fetch_attempts: Vec<Attempt>,
  fetch_resolved_url: Option<String>,
  pub state: RetryState,
  pub browser: Option<String>,
  auth: AuthSettings,
  host: String,
  cookie_fetch_retried: bool,
  just_accepted: bool,
  attempted: HashSet<String>,
  starting_browser: Option<String>,
  selected: Option<LoginCandidate>,
  app_bound_detected: bool,
  network_down: bool,
  best_failure: Option<DiagnosticEvent>,
}
impl LoginSession {
  pub fn new(
    app: &AppHandle,
    url: &str,
    overrides: Option<&DownloadOverrides>,
    accepted: Option<String>,
    just_accepted: bool,
  ) -> Self {
    let config = app.state::<SharedConfig>();
    let mut cfg = config.load();
    if cfg
      .auth
      .site_browsers
      .keys()
      .any(|host| site_key(host).is_some_and(|key| key != *host))
    {
      let _consent = CONSENT_WRITES.lock().unwrap();
      let _ = config.modify(app, |cfg| {
        migrate_site_browsers(&mut cfg.auth.site_browsers)
      });
      cfg = config.load();
    }
    let auth = resolve_with_patch(&cfg.auth, overrides.and_then(|o| o.auth.as_ref()));
    let mut session = Self::from_auth(auth, url, accepted);
    session.just_accepted &= just_accepted;
    session
  }
  fn from_auth(mut auth: AuthSettings, url: &str, accepted: Option<String>) -> Self {
    migrate_site_browsers(&mut auth.site_browsers);
    let host = reqwest::Url::parse(url)
      .ok()
      .and_then(|u| {
        u.host_str()
          .map(|host| site_key(host).unwrap_or_else(|| host.to_ascii_lowercase()))
      })
      .unwrap_or_default();
    let anonymous = accepted.as_deref() == Some(ANONYMOUS_BROWSER);
    let just_accepted = accepted.is_some() && !anonymous;
    let starting_browser = auth.site_browsers.get(&host).cloned();
    // A remembered host is already consented, independently of the global opt-in.
    // All fetch/download engines share this boundary, including gallery fallbacks.
    let accepted = accepted.or_else(|| {
      (auth.cookie_browser == "automatic"
        && auth.cookie_file.as_ref().is_none_or(|s| s.is_empty())
        && may_reuse_remembered(&auth, &host))
      .then(|| {
        auth
          .site_browsers
          .get(&host)
          .filter(|s| !s.is_empty())
          .cloned()
      })
      .flatten()
    });
    Self {
      fetch_managed: false,
      fetch_browser: None,
      fetch_gate_used: false,
      fetch_attempts: Vec::new(),
      fetch_resolved_url: None,
      state: RetryState {
        browser_attempted: accepted.is_some() && !anonymous,
        anonymous_attempted: anonymous,
        ..Default::default()
      },
      attempted: accepted
        .iter()
        .filter(|browser| browser.as_str() != ANONYMOUS_BROWSER)
        .cloned()
        .collect(),
      browser: accepted,
      auth,
      host,
      cookie_fetch_retried: false,
      just_accepted,
      starting_browser,
      selected: None,
      app_bound_detected: false,
      network_down: false,
      best_failure: None,
    }
  }

  pub(crate) fn known_host(host: &str) -> bool {
    known_login_host(host)
  }

  pub(crate) fn begin_fetch(&mut self, phase: Phase) {
    self.fetch_managed = true;
    self.fetch_browser = self.browser.clone();
    self.fetch_gate_used = phase == Phase::Login;
    if phase == Phase::Anonymous {
      self.browser = Some(ANONYMOUS_BROWSER.into());
    }
  }
  pub(crate) fn degrade_fetch(&mut self) {
    self.browser = Some(ANONYMOUS_BROWSER.into());
    self.state.anonymous_attempted = true;
  }
  pub(crate) fn fetch_managed(&self) -> bool {
    self.fetch_managed
  }
  pub(crate) fn set_fetch_history(&mut self, attempts: Vec<Attempt>, resolved: Option<String>) {
    self.fetch_attempts = attempts;
    self.fetch_resolved_url = resolved;
  }
  pub(crate) fn request_fetch_login(
    &mut self,
    app: &AppHandle,
    entry: &FetchEntry,
    reason: &str,
  ) -> RetryAction {
    if self.fetch_gate_used {
      return RetryAction::Stop;
    }
    self.fetch_gate_used = true;
    self.browser = row_browser(&entry.group_id, &entry.id, &entry.url)
      // Cookie-free download carry does not decline login for a later fresh fetch.
      .filter(|browser| browser != ANONYMOUS_BROWSER)
      .or_else(|| self.fetch_browser.clone());
    if self.browser.as_deref() == Some(ANONYMOUS_BROWSER) {
      return RetryAction::Stop;
    }
    if self.uses_cookies() {
      self.announce(app, &entry.id, &entry.group_id);
      return RetryAction::Browser;
    }
    if reason == "drmProtected" {
      let automatic = self.auth.cookie_browser == "automatic"
        && self.auth.cookie_file.as_ref().is_none_or(|s| s.is_empty());
      if !automatic
        || !discovered_candidates(&self.host, self.starting_browser.as_deref())
          .iter()
          .any(|candidate| !candidate.app_bound)
      {
        return RetryAction::Stop;
      }
    }
    let mut request = entry.clone();
    request.prior_attempts = self.fetch_attempts.clone();
    if reason == "drmProtected" {
      for attempt in &mut request.prior_attempts {
        if attempt.stage == crate::extract::outcome::StageId::Ytdlp
          && attempt.outcome == crate::extract::outcome::Outcome::Drm
        {
          let diagnostic = crate::extract::outcome::classify(
            "signInRequired",
            "",
            crate::extract::outcome::NetworkScope::StageLocal,
          );
          attempt.outcome = diagnostic.outcome;
          attempt.code = Some(diagnostic.code);
        }
      }
    }
    request.prior_resolved_url = self.fetch_resolved_url.clone();
    self.failed(
      app,
      LoginRequest::Fetch(request),
      Some(if reason == "drmProtected" {
        "signInRequired"
      } else {
        reason
      }),
      false,
      !is_group_running(&entry.group_id),
    )
  }
  fn carry_fetch_browser(&self, id: &str, group: &str) {
    if self.uses_cookies() {
      if let Some(browser) = self.browser.as_deref() {
        remember_row_browser(group, id, &self.host, browser);
      }
    } else {
      remember_row_browser(group, id, &self.host, ANONYMOUS_BROWSER);
    }
  }
  pub(crate) fn fetch_succeeded(&mut self, app: &AppHandle, id: &str, group: &str) {
    self.carry_fetch_browser(id, group);
    self.fetch_managed = false;
    self.succeeded(app, group);
  }
  pub(crate) fn fetch_exhausted(&self, app: &AppHandle, entry: &FetchEntry) {
    let _ = app.emit(
      "media_login_needed",
      LoginNeeded {
        request_id: uuid::Uuid::new_v4().to_string(),
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        host: self.host.clone(),
        reason: "browserLoginFailed".into(),
        status: LoginStatus::Exhausted,
        candidates: Vec::new(),
        change_only: false,
      },
    );
  }

  pub fn announce(&self, app: &AppHandle, id: &str, group_id: &str) {
    if self.browser.as_deref() == Some(ANONYMOUS_BROWSER) || !is_group_running(group_id) {
      return;
    }
    let cached = USED
      .lock()
      .unwrap()
      .get(group_id)
      .filter(|(used, _)| self.browser.as_deref() == Some(used.candidate.id.as_str()))
      .map(|(used, _)| used.candidate.clone());
    let Some(candidate) = self
      .selected
      .clone()
      .or(cached)
      .or_else(|| self.browser.as_deref().and_then(describe).map(Into::into))
    else {
      return;
    };
    announce_using(
      app,
      LoginUsing {
        id: id.into(),
        group_id: group_id.into(),
        host: self.host.clone(),
        candidate,
        selected_only: false,
      },
    );
  }

  pub fn uses_cookies(&self) -> bool {
    if self.browser.as_deref() == Some(ANONYMOUS_BROWSER) {
      return false;
    }
    self.browser.is_some()
      || self
        .auth
        .cookie_file
        .as_ref()
        .is_some_and(|s| !s.is_empty())
      || !matches!(self.auth.cookie_browser.as_str(), "automatic" | "none" | "")
  }

  /// Retain only redacted diagnostics, never cookie contents or source URLs.
  pub fn record_failure(&mut self, diagnostics: &[DiagnosticEvent], network_down: bool) {
    self.network_down = network_down
      || diagnostics.iter().any(|event| {
        matches!(event.level, DiagnosticLevel::Error)
          && crate::parsers::ytdlp_error::network_down(&event.code, &event.message)
      });
    let current = diagnostics
      .iter()
      .filter(|event| matches!(event.level, DiagnosticLevel::Error))
      .max_by_key(|event| diagnostic_priority(&event.code));
    if !self.state.anonymous_attempted {
      self.best_failure = current.cloned();
    } else if let Some(current) = current {
      if self.best_failure.as_ref().is_none_or(|previous| {
        diagnostic_priority(&current.code) >= diagnostic_priority(&previous.code)
      }) {
        self.best_failure = Some(current.clone());
      }
    }
  }

  pub fn has_better_failure(&self, code: &str) -> bool {
    self.state.anonymous_attempted
      && self
        .best_failure
        .as_ref()
        .is_some_and(|previous| diagnostic_priority(&previous.code) > diagnostic_priority(code))
  }

  pub fn failure_diagnostics(&self, mut diagnostics: Vec<DiagnosticEvent>) -> Vec<DiagnosticEvent> {
    // The row displays the final diagnostic. A generic retry must not hide a private/removed post.
    if self.state.anonymous_attempted && !self.network_down {
      if let Some(best) = &self.best_failure {
        diagnostics.retain(|event| !matches!(event.level, DiagnosticLevel::Error));
        diagnostics.push(best.clone());
      }
    }
    diagnostics
  }

  /// The caller races this backoff against group cancellation; tests inject a ready future.
  pub async fn retry_cookie_fetch<F: std::future::Future<Output = ()>>(
    &mut self,
    failure: CookieFetchFailure,
    delay: impl FnOnce(Duration) -> F,
  ) -> bool {
    if failure != CookieFetchFailure::Transient || !self.uses_cookies() || self.cookie_fetch_retried
    {
      return false;
    }
    if !claim_host_retry(
      &mut TRANSIENT_RETRIES.lock().unwrap(),
      &self.host,
      Instant::now(),
    ) {
      return false;
    }
    self.cookie_fetch_retried = true;
    delay(Duration::from_secs(2)).await;
    true
  }

  fn failure_status(
    &self,
    action: &RetryAction,
    reason: Option<&str>,
    cancelled: bool,
    has_candidates: bool,
  ) -> Option<LoginStatus> {
    if cancelled
      || !(action == &RetryAction::AskLogin
        || (self.state.browser_attempted && reason.is_some() && action == &RetryAction::Stop))
    {
      return None;
    }
    Some(if self.state.browser_attempted || !has_candidates {
      LoginStatus::Exhausted
    } else {
      LoginStatus::Needed
    })
  }

  pub fn failed(
    &mut self,
    app: &AppHandle,
    request: LoginRequest,
    reason: Option<&str>,
    aria_denied: bool,
    cancelled: bool,
  ) -> RetryAction {
    let host = self.host.clone();
    let (action, candidates) = self.decide(reason, aria_denied, cancelled, |remembered| {
      discovered_candidates(&host, remembered)
    });
    if action == RetryAction::Browser {
      let (id, group) = request.ids();
      self.announce(app, id, group);
    }
    if let Some(status) = self.failure_status(&action, reason, cancelled, !candidates.is_empty()) {
      let (id, group_id) = request.ids();
      let exhausted = matches!(status, LoginStatus::Exhausted);
      let request_id = uuid::Uuid::new_v4().to_string();
      let payload = LoginNeeded {
        request_id: request_id.clone(),
        id: id.into(),
        group_id: group_id.into(),
        host: self.host.clone(),
        reason: if self.app_bound_detected && !candidates.iter().any(|c| c.signed_in) {
          "browserAppBound".into()
        } else {
          reason.unwrap_or("browserLoginFailed").into()
        },
        status,
        candidates: candidates.clone(),
        change_only: false,
      };
      if !exhausted {
        let mut pending = PENDING.lock().unwrap();
        pending.retain(|_, p| {
          let keep = p.created.elapsed() < LOGIN_TTL
            && (*p.cancel.borrow() || p.followers.iter().any(|row| *row.cancel.borrow()));
          if !keep {
            for row in std::iter::once(&*p).chain(p.followers.iter()) {
              if *row.cancel.borrow() {
                row.request.fail(app, "Browser login request timed out");
              } else {
                row.request.release(app);
              }
            }
          }
          keep
        });
        if pending.len() >= 64
          && !pending
            .values()
            .any(|p| p.request.host().as_deref() == Some(self.host.as_str()))
        {
          if let Some(oldest) = pending
            .iter()
            .min_by_key(|(_, p)| p.created)
            .map(|(k, _)| k.clone())
          {
            if let Some(old) = pending.remove(&oldest) {
              for row in old.into_rows() {
                row.request.fail(app, "Browser login request expired");
              }
            }
          }
        }
        let announce = queue_pending(
          &mut pending,
          request_id.clone(),
          PendingLogin {
            followers: Vec::new(),
            announcement: Some((app.clone(), payload.clone())),
            cancel: subscribe_group(group_id),
            request,
            candidates,
            created: Instant::now(),
            cleanup: CleanupChannels {
              fetch: app.state::<FetchSender>().inner().clone(),
              download: app.state::<DownloadSender>().inner().clone(),
            },
          },
        );
        if !announce {
          return RetryAction::AskLogin;
        }
        schedule_expiry(app.clone(), request_id);
      }
      let _ = app.emit("media_login_needed", payload);
      if exhausted {
        return RetryAction::Stop;
      }
    }
    action
  }
  fn decide(
    &mut self,
    reason: Option<&str>,
    aria_denied: bool,
    cancelled: bool,
    find: impl FnOnce(Option<&str>) -> Vec<LoginCandidate>,
  ) -> (RetryAction, Vec<LoginCandidate>) {
    let automatic = self.auth.cookie_browser == "automatic"
      && self.auth.cookie_file.as_ref().is_none_or(|s| s.is_empty());
    if cancelled
      || self.network_down
      || aria_denied
      || reason.is_none()
      || !automatic
      || self.just_accepted
    {
      let action = self.state.next(
        automatic,
        false,
        false,
        RetryFailure {
          cancelled,
          login: reason.is_some(),
          aria_denied,
          used_cookies: self.uses_cookies(),
          network_down: self.network_down,
        },
      );
      if action == RetryAction::Anonymous {
        self.browser = Some(ANONYMOUS_BROWSER.into());
        self.selected = None;
      }
      return (action, vec![]);
    }
    // A remembered/auto-selected profile can expire. Only this attempt ignores it; a
    // working replacement is persisted on success using a compare-and-swap below.
    self.browser = None;
    self.selected = None;
    self.state.browser_attempted = false;
    let candidates = find(self.starting_browser.as_deref());
    self.app_bound_detected |= candidates.iter().any(|c| c.app_bound);
    let candidates: Vec<_> = candidates.into_iter().filter(|c| !c.app_bound).collect();
    let available = |c: &&LoginCandidate| !self.attempted.contains(&c.id);
    let selected = candidates
      .iter()
      .filter(available)
      .find(|c| known_login_host(&self.host) && c.signed_in)
      .or_else(|| {
        may_reuse_remembered(&self.auth, &self.host)
          .then(|| {
            candidates
              .iter()
              .filter(available)
              .find(|c| self.starting_browser.as_deref() == Some(c.id.as_str()))
          })
          .flatten()
      })
      .or_else(|| {
        self
          .auth
          .always_use_browser_logins
          .then(|| candidates.iter().find(available))
          .flatten()
      });
    if let Some(selected) = selected {
      self.browser = Some(selected.id.clone());
      self.attempted.insert(selected.id.clone());
      self.selected = Some(selected.clone());
      self.state.browser_attempted = true;
      return (RetryAction::Browser, candidates);
    }
    (RetryAction::AskLogin, candidates)
  }
  fn success_patch(&self) -> Option<serde_json::Value> {
    self
      .browser
      .as_ref()
      .filter(|browser| !self.host.is_empty() && browser.as_str() != ANONYMOUS_BROWSER)
      .map(|browser| serde_json::json!({"auth":{"siteBrowsers":{&self.host:browser}}}))
  }
  pub fn log_anonymous_success(&self, app: &AppHandle, group_id: &str) {
    if self.state.anonymous_attempted && !self.uses_cookies() {
      tracing::debug!("anonymous retry succeeded");
      if let Some(logs) = app.try_state::<crate::logging::LogStoreState>() {
        logs
          .write()
          .append_line(app, group_id, "anonymous retry succeeded");
      }
    }
  }

  pub fn succeeded(&self, app: &AppHandle, group_id: &str) {
    if self.fetch_managed {
      return;
    }
    self.log_anonymous_success(app, group_id);
    if self.success_patch().is_some() {
      if self.just_accepted {
        EXPLICIT_SITES.lock().unwrap().insert(self.host.clone());
      }
      let Some(browser) = self.browser.as_ref() else {
        return;
      };
      let _consent = CONSENT_WRITES.lock().unwrap();
      let config = app.state::<SharedConfig>();
      if config.load().auth.site_browsers.get(&self.host) == Some(browser) {
        return;
      }
      let _ = config.modify(app, |cfg| {
        remember_success(
          &mut cfg.auth,
          &self.host,
          self.starting_browser.as_deref(),
          browser,
        );
      });
    }
  }
}

fn diagnostic_priority(code: &str) -> u8 {
  match code {
    "unknown"
    | "unableToExtractIdOrInfo"
    | "unableToExtractInitialData"
    | "imageSourceUnavailable" => 0,
    "urlNotSupported" | "urlUnsupported" | "unknownUrlType" | "photosOnly" => 1,
    "signInRequired"
    | "signInRequiredForBotDetection"
    | "membersOnly"
    | "videoNotFoundOrPrivate"
    | "playlistPrivateOrMissing"
    | "notFound404"
    | "channelUnavailable" => 3,
    _ => 2,
  }
}

fn claim_pending(
  map: &mut HashMap<String, PendingLogin>,
  request_id: &str,
  browser_id: &str,
) -> Result<PendingLogin, String> {
  let pending = map.get(request_id).ok_or("Login request expired")?;
  if !pending.candidates.iter().any(|c| c.id == browser_id) {
    return Err("Browser was not offered for this request".into());
  }
  Ok(map.remove(request_id).unwrap())
}

#[cfg(test)]
fn record_host_browser(auth: &mut AuthSettings, host: &str, browser: Option<String>) {
  let host = site_key(host).unwrap_or_else(|| host.into());
  match browser {
    Some(browser) => {
      auth.site_browsers.insert(host.clone(), browser);
    }
    None => {
      auth.site_browsers.remove(&host);
    }
  }
}

fn remember_success(auth: &mut AuthSettings, host: &str, previous: Option<&str>, browser: &str) {
  // A fallback may replace the starting browser, never another row's newer choice.
  migrate_site_browsers(&mut auth.site_browsers);
  let host = site_key(host).unwrap_or_else(|| host.into());
  if auth.site_browsers.get(&host).map(String::as_str) == previous {
    auth.site_browsers.insert(host, browser.into());
  }
}

const HOST_RETRY_WINDOW: Duration = Duration::from_secs(30);
static TRANSIENT_RETRIES: LazyLock<Mutex<HashMap<String, Instant>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
fn claim_host_retry(budget: &mut HashMap<String, Instant>, host: &str, now: Instant) -> bool {
  budget.retain(|_, last| now.saturating_duration_since(*last) < HOST_RETRY_WINDOW);
  let Some(host) = site_key(host) else {
    return false;
  };
  if budget.contains_key(&host) {
    return false;
  }
  budget.insert(host, now);
  true
}

#[tauri::command]
pub fn media_login_accept(
  app: AppHandle,
  request_id: String,
  browser_id: String,
) -> Result<(), String> {
  accept_login(app, request_id, browser_id)
}

fn accept_login<R: tauri::Runtime>(
  app: AppHandle<R>,
  request_id: String,
  browser_id: String,
) -> Result<(), String> {
  let browser_id = resolve_candidate_id(&browser_id)?;
  if CHOICES.lock().unwrap().contains_key(&request_id) {
    let choice = claim_choice(&mut CHOICES.lock().unwrap(), &request_id, &browser_id)?;
    let candidate = choice
      .candidates
      .iter()
      .find(|c| c.id == browser_id)
      .cloned()
      .ok_or("Browser was not offered")?;
    remember_row_browser(
      &choice.used.group_id,
      &choice.used.id,
      &choice.used.host,
      &browser_id,
    );
    announce_using(
      &app,
      LoginUsing {
        candidate,
        selected_only: true,
        ..choice.used
      },
    );
    return Ok(());
  }
  let mut map = PENDING.lock().unwrap();
  let pending = map.get(&request_id).ok_or("Login request expired")?;
  if pending.created.elapsed() >= LOGIN_TTL {
    if let Some(pending) = map.remove(&request_id) {
      for row in pending.into_rows() {
        if *row.cancel.borrow() {
          row.request.fail(&app, "Browser login request timed out");
        } else {
          row.request.release(&app);
        }
      }
    }
    return Err("Login request expired or cancelled".into());
  }
  let pending = claim_pending(&mut map, &request_id, &browser_id)?;
  drop(map);
  let mut result = Ok(());
  for row in pending.into_rows() {
    if !*row.cancel.borrow() {
      row.request.release(&app);
      continue;
    }
    if let Err(error) = row.request.clone().enqueue(&app, browser_id.clone()) {
      row
        .request
        .fail(&app, "Browser login retry could not be queued");
      result = Err(error);
    }
  }
  result
}
#[tauri::command]
pub fn media_login_dismiss(app: AppHandle, request_id: String) {
  if CHOICES.lock().unwrap().remove(&request_id).is_some() {
    return;
  }
  if let Some(pending) = PENDING.lock().unwrap().remove(&request_id) {
    for row in pending.into_rows() {
      if *row.cancel.borrow() {
        let (mut diagnostic, mut fatal) = row.request.timeout_payloads();
        diagnostic.code = "loginConsentDeclined".into();
        diagnostic.raw = "loginConsentDeclined".into();
        diagnostic.message = "Browser login was not used".into();
        fatal.message = diagnostic.message.clone();
        fatal.details = Some(diagnostic.code.clone());
        let _ = app.emit("media_diagnostic", diagnostic);
        let _ = app.emit("media_fatal", fatal);
      }
      row.request.release(&app);
    }
  }
}
fn remove_pending_group(map: &mut HashMap<String, PendingLogin>, group_id: &str) {
  let tokens: Vec<_> = map
    .iter()
    .filter(|(_, pending)| {
      pending.request.ids().1 == group_id
        || pending
          .followers
          .iter()
          .any(|row| row.request.ids().1 == group_id)
    })
    .map(|(token, _)| token.clone())
    .collect();
  for token in tokens {
    let Some(mut pending) = map.remove(&token) else {
      continue;
    };
    let announcement = pending.announcement.take();
    let offered = pending.candidates.clone();
    let created = pending.created;
    let promote = pending.request.ids().1 == group_id;
    let mut remaining = Vec::new();
    for row in pending.into_rows() {
      if row.request.ids().1 == group_id {
        row.cleanup.send(group_id);
      } else {
        remaining.push(row);
      }
    }
    if remaining.is_empty() {
      continue;
    }
    let mut next = remaining.remove(0);
    next.followers = remaining;
    next.candidates = offered;
    next.created = created;
    next.announcement = announcement;
    if promote {
      let (id, group) = next.request.ids();
      if let Some((app, payload)) = &mut next.announcement {
        payload.id = id.into();
        payload.group_id = group.into();
        let _ = app.emit("media_login_needed", payload.clone());
      }
    }
    map.insert(token, next);
  }
}

pub fn forget_group(group_id: &str) {
  ROW_BROWSERS
    .lock()
    .unwrap()
    .retain(|(group, _), _| group != group_id);
  CHOICES
    .lock()
    .unwrap()
    .retain(|_, choice| choice.used.group_id != group_id);
  let mut map = PENDING.lock().unwrap();
  remove_pending_group(&mut map, group_id);
  drop(map);
  crate::scheduling::download_pipeline::forget_group_counter(group_id);
  crate::scheduling::fetch_pipeline::forget_group_counter(group_id);
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineAvailability {
  aria2_available: bool,
}
#[tauri::command]
pub fn engine_availability(app: AppHandle) -> EngineAvailability {
  EngineAvailability {
    aria2_available: super::transport::resolve_aria2(&app).is_some(),
  }
}

#[cfg(test)]
pub(crate) fn test_consented_session() -> LoginSession {
  LoginSession::from_auth(
    AuthSettings::default(),
    "https://www.instagram.com/p/123/",
    Some("firefox".into()),
  )
}

#[cfg(test)]
#[path = "login_cookie_tests.rs"]
mod cookie_tests;

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn unknown_hosts_ignore_legacy_remembered_and_generic_cookie_evidence() {
    let host = format!("unsafe-{}.example", uuid::Uuid::new_v4());
    let mut auth = AuthSettings::default();
    record_host_browser(&mut auth, &host, Some("chrome:/legacy".into()));
    let mut session = LoginSession::from_auth(auth.clone(), &format!("https://{host}/login"), None);
    assert!(session.browser.is_none());
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, |_| vec![
          LoginCandidate {
            id: "chrome:/legacy".into(),
            browser: "chrome".into(),
            profile: "Legacy".into(),
            signed_in: true,
            app_bound: false,
          }
        ])
        .0,
      RetryAction::AskLogin
    );
    EXPLICIT_SITES
      .lock()
      .unwrap()
      .insert(site_key(&host).unwrap_or_else(|| host.clone()));
    let consented = LoginSession::from_auth(auth, &format!("https://{host}/login"), None);
    assert_eq!(consented.browser.as_deref(), Some("chrome:/legacy"));
    EXPLICIT_SITES
      .lock()
      .unwrap()
      .remove(&site_key(&host).unwrap_or_else(|| host.clone()));
  }

  #[test]
  fn reddit_aliases_reuse_a_legacy_remembered_profile_and_write_one_canonical_key() {
    for legacy in [
      "www.reddit.com",
      "old.reddit.com",
      "v.redd.it",
      "redd.it",
      "reddit.com",
    ] {
      for host in ["redd.it", "v.redd.it", "old.reddit.com", "www.reddit.com"] {
        let mut auth = AuthSettings::default();
        auth
          .site_browsers
          .insert(legacy.into(), "chrome:/remembered".into());
        let session = LoginSession::from_auth(auth, &format!("https://{host}/item"), None);
        assert_eq!(
          session.browser.as_deref(),
          Some("chrome:/remembered"),
          "{legacy} → {host}"
        );
        assert_eq!(session.auth.site_browsers.len(), 1);
        assert_eq!(
          session
            .auth
            .site_browsers
            .get("reddit.com")
            .map(String::as_str),
          Some("chrome:/remembered")
        );
        assert_eq!(
          session.success_patch().unwrap(),
          serde_json::json!({"auth":{"siteBrowsers":{"reddit.com":"chrome:/remembered"}}})
        );
      }
    }
  }

  #[test]
  fn legacy_site_key_migration_preserves_values_and_is_idempotent() {
    for (legacy, canonical) in [
      ("www.reddit.com", "reddit.com"),
      ("redd.it", "reddit.com"),
      ("www.youtube.com", "youtube.com"),
      ("youtu.be", "youtube.com"),
      ("threads.com", "threads.net"),
      ("twitter.com", "x.com"),
      ("www.instagram.com", "instagram.com"),
    ] {
      let mut browsers =
        HashMap::from([(legacy.to_owned(), "chrome:/private/Profile 1".to_owned())]);
      migrate_site_browsers(&mut browsers);
      assert_eq!(
        browsers,
        HashMap::from([(canonical.to_owned(), "chrome:/private/Profile 1".to_owned())])
      );
      let once = browsers.clone();
      migrate_site_browsers(&mut browsers);
      assert_eq!(browsers, once);
      browsers.insert(legacy.into(), "firefox:/stale".into());
      migrate_site_browsers(&mut browsers);
      assert_eq!(
        browsers, once,
        "canonical selection must survive a conflicting legacy key"
      );
    }
  }

  #[test]
  fn candidate_payload_ids_do_not_expose_engine_profile_paths() {
    let candidate = LoginCandidate {
      id: "chrome:/Users/private-user/Chrome/Default".into(),
      browser: "chrome".into(),
      profile: "Ahmed".into(),
      signed_in: true,
      app_bound: false,
    };
    let wire = serde_json::to_value(&candidate).unwrap();
    assert!(uuid::Uuid::parse_str(wire["id"].as_str().unwrap()).is_ok());
    assert!(!wire.to_string().contains("private-user"));
    let id = wire["id"].as_str().unwrap();
    assert_eq!(resolve_candidate_id(id).unwrap(), candidate.id);
    assert!(resolve_candidate_id(&candidate.id).is_err());
    assert_eq!(serde_json::to_value(&candidate).unwrap()["id"], wire["id"]);
    let group = uuid::Uuid::new_v4().to_string();
    let mut offered = pending(&group);
    offered.candidates = vec![candidate];
    let mut map = HashMap::from([("request".into(), offered)]);
    let other_token = opaque_candidate_id("firefox:/not-offered");
    assert!(claim_pending(
      &mut map,
      "request",
      &resolve_candidate_id(&other_token).unwrap()
    )
    .is_err());
    let claimed = claim_pending(&mut map, "request", &resolve_candidate_id(id).unwrap()).unwrap();
    assert_eq!(claimed.request.ids().1, group);
    crate::scheduling::group_state::remove_group(&group);
  }

  #[test]
  fn discovery_is_cached_per_host_until_login_ttl() {
    let now = Instant::now();
    let mut cache = HashMap::new();
    let first = cached_discovery(&mut cache, "instagram.com", now, || candidates(None));
    let second = cached_discovery(&mut cache, "instagram.com", now, || {
      panic!("profile rescan for a concurrent row")
    });
    assert_eq!(second[0].id, first[0].id);
    let reddit = cached_discovery(&mut cache, "www.reddit.com", now, || candidates(None));
    let short = cached_discovery(&mut cache, "redd.it", now, || {
      panic!("alias repeated browser discovery")
    });
    assert_eq!(short[0].id, reddit[0].id);
    let other = cached_discovery(&mut cache, "facebook.com", now, || {
      let mut c = candidates(None);
      c[0].profile = "Other site".into();
      c
    });
    assert_eq!(other[0].profile, "Other site");
    let refreshed = cached_discovery(&mut cache, "instagram.com", now + LOGIN_TTL, || {
      let mut c = candidates(None);
      c[0].profile = "Refreshed".into();
      c
    });
    assert_eq!(refreshed[0].profile, "Refreshed");
  }

  #[test]
  fn safari_without_evidence_requires_a_picker_unless_globally_allowed() {
    let host = format!("safari-{}.instagram.com", uuid::Uuid::new_v4());
    let url = format!("https://{host}/reel/private");
    let mut auth = AuthSettings::default();
    record_host_browser(&mut auth, &host, Some("safari".into()));
    let mut legacy = LoginSession::from_auth(auth.clone(), &url, None);
    assert!(
      legacy.browser.is_none(),
      "legacy Safari entries are not proof of consent"
    );
    assert_eq!(
      legacy
        .decide(Some("signInRequired"), false, false, |_| vec![
          LoginCandidate {
            id: "safari".into(),
            browser: "safari".into(),
            profile: "Default".into(),
            signed_in: false,
            app_bound: false,
          }
        ])
        .0,
      RetryAction::AskLogin
    );
    EXPLICIT_SITES
      .lock()
      .unwrap()
      .insert(site_key(&host).unwrap_or_else(|| host.clone()));
    assert_eq!(
      LoginSession::from_auth(auth, &url, None).browser.as_deref(),
      Some("safari")
    );
    EXPLICIT_SITES
      .lock()
      .unwrap()
      .remove(&site_key(&host).unwrap_or_else(|| host.clone()));
    for (allowed, expected) in [(false, RetryAction::AskLogin), (true, RetryAction::Browser)] {
      let mut session = LoginSession::from_auth(
        AuthSettings {
          always_use_browser_logins: allowed,
          ..Default::default()
        },
        "https://instagram.com/reel/private",
        None,
      );
      let (action, _) = session.decide(Some("signInRequired"), false, false, |_| {
        vec![LoginCandidate {
          id: "safari".into(),
          browser: "safari".into(),
          profile: "Default".into(),
          signed_in: false,
          app_bound: false,
        }]
      });
      assert_eq!(action, expected);
    }
  }

  #[test]
  fn automatic_signed_profiles_need_no_prompt_and_skip_app_bound_chromium() {
    for (blocked, expected) in [(false, "chrome:/signed"), (true, "firefox:/signed")] {
      let mut session = LoginSession::from_auth(
        AuthSettings::default(),
        "https://instagram.com/reel/private",
        None,
      );
      let (action, _) = session.decide(Some("signInRequired"), false, false, |_| {
        vec![
          LoginCandidate {
            id: "chrome:/signed".into(),
            browser: "chrome".into(),
            profile: "Ahmed".into(),
            signed_in: true,
            app_bound: blocked,
          },
          LoginCandidate {
            id: "firefox:/signed".into(),
            browser: "firefox".into(),
            profile: "Ahmed.work".into(),
            signed_in: true,
            app_bound: false,
          },
        ]
      });
      assert_eq!(action, RetryAction::Browser);
      assert_eq!(session.browser.as_deref(), Some(expected));
      assert!(!session.auth.always_use_browser_logins);
    }
  }

  #[test]
  fn remembered_refusal_uses_next_signed_profile_then_offers_the_picker() {
    let mut auth = AuthSettings::default();
    record_host_browser(&mut auth, "www.instagram.com", Some("chrome:/old".into()));
    let mut session = LoginSession::from_auth(auth, "https://www.instagram.com/reel/private", None);
    let choices = || {
      vec![
        LoginCandidate {
          id: "chrome:/old".into(),
          browser: "chrome".into(),
          profile: "Old".into(),
          signed_in: true,
          app_bound: false,
        },
        LoginCandidate {
          id: "firefox:/new".into(),
          browser: "firefox".into(),
          profile: "New".into(),
          signed_in: true,
          app_bound: false,
        },
      ]
    };
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, |_| choices())
        .0,
      RetryAction::Browser
    );
    assert_eq!(session.browser.as_deref(), Some("firefox:/new"));
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, |_| choices())
        .0,
      RetryAction::AskLogin
    );
    assert!(session.browser.is_none());
  }

  #[test]
  fn success_replaces_the_refused_starting_browser_but_not_a_newer_choice() {
    let mut auth = AuthSettings::default();
    record_host_browser(&mut auth, "site.test", Some("chrome:/old".into()));
    remember_success(
      &mut auth,
      "site.test",
      Some("chrome:/old"),
      "firefox:/working",
    );
    assert_eq!(auth.site_browsers["site.test"], "firefox:/working");
    remember_success(&mut auth, "site.test", Some("chrome:/old"), "chrome:/late");
    assert_eq!(auth.site_browsers["site.test"], "firefox:/working");
  }

  #[test]
  fn concurrent_rows_share_one_host_retry_until_the_window_expires() {
    let now = Instant::now();
    let mut budget = HashMap::new();
    assert!(claim_host_retry(&mut budget, "www.instagram.com", now));
    for _ in 0..5 {
      assert!(!claim_host_retry(&mut budget, "m.instagram.com", now));
    }
    assert!(claim_host_retry(&mut budget, "facebook.com", now));
    assert!(claim_host_retry(
      &mut budget,
      "instagram.com",
      now + Duration::from_secs(31)
    ));
  }

  #[test]
  fn only_a_login_refusal_exhausts_a_browser_session() {
    let session = LoginSession::from_auth(
      AuthSettings::default(),
      "https://www.instagram.com/reel/test/",
      Some("chrome".into()),
    );
    for (reason, cancelled, expected) in [
      (None, false, None),
      (Some("signInRequired"), false, Some(true)),
      (Some("signInRequired"), true, None),
    ] {
      assert_eq!(
        session
          .failure_status(&RetryAction::Stop, reason, cancelled, false)
          .map(|status| matches!(status, LoginStatus::Exhausted)),
        expected
      );
    }
  }
  #[test]
  fn cookie_failure_retries_anonymously_without_forgetting_the_remembered_browser() {
    use crate::runners::ytdlp_runner::cookie_source_args;
    for (mode, file, accepted) in [
      ("automatic", None, None),
      ("automatic", None, Some("firefox:/accepted")),
      ("firefox", None, None),
      ("automatic", Some("/manual.txt"), None),
    ] {
      let mut auth = AuthSettings {
        cookie_browser: mode.into(),
        cookie_file: file.map(str::to_owned),
        ..Default::default()
      };
      record_host_browser(&mut auth, "m.facebook.com", Some("chrome:/saved".into()));
      let mut session = LoginSession::from_auth(
        auth.clone(),
        "https://m.facebook.com/watch/",
        accepted.map(str::to_owned),
      );
      let (action, _) = session.decide(None, false, false, |_| panic!("no discovery"));
      assert_eq!(action, RetryAction::Anonymous, "{mode}, {file:?}");
      assert!(cookie_source_args(&auth, session.browser.as_deref()).is_empty());
      assert!(!session.uses_cookies());
      assert!(session.success_patch().is_none());
      assert_eq!(session.auth.site_browsers["facebook.com"], "chrome:/saved");
      assert_eq!(
        session
          .decide(None, false, false, |_| panic!("no discovery"))
          .0,
        RetryAction::Stop
      );
    }
  }

  #[test]
  fn failed_anonymous_retry_reports_the_more_informative_attempt() {
    use crate::parsers::ytdlp_error::{DiagnosticMatcher, YtdlpErrorParser};
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for (cookie_error, anonymous_error, expected) in [
      (
        "ERROR: Unsupported URL",
        "ERROR: Unsupported URL",
        "urlNotSupported",
      ),
      (
        "ERROR: Unsupported URL",
        "ERROR: Login required",
        "signInRequired",
      ),
      (
        "ERROR: Video does not exist",
        "ERROR: Unsupported URL",
        "videoNotFoundOrPrivate",
      ),
      (
        "ERROR: HTTP Error 404",
        "ERROR: Unable to extract info section",
        "notFound404",
      ),
      (
        "ERROR: Unsupported URL",
        "ERROR: private video",
        "signInRequired",
      ),
    ] {
      let mut session = LoginSession::from_auth(
        AuthSettings::default(),
        "https://facebook.com/watch/",
        Some("firefox".into()),
      );
      session.record_failure(&parser.parse_lines(vec![cookie_error], true), false);
      assert_eq!(
        session
          .decide(None, false, false, |_| panic!("no discovery"))
          .0,
        RetryAction::Anonymous
      );
      let diagnostics = parser.parse_lines(vec![anonymous_error], false);
      session.record_failure(&diagnostics, false);
      let reason = parser.login_reason(anonymous_error);
      let action = session
        .decide(reason.as_deref(), false, false, |_| vec![])
        .0;
      assert!(matches!(action, RetryAction::Stop | RetryAction::AskLogin));
      assert_eq!(
        session
          .failure_diagnostics(diagnostics)
          .iter()
          .filter(|event| matches!(event.level, DiagnosticLevel::Error))
          .map(|event| event.code.as_str())
          .collect::<Vec<_>>(),
        vec![expected],
        "{cookie_error}; {anonymous_error}"
      );
    }
  }

  #[test]
  fn network_outages_and_cancellation_do_not_spend_the_anonymous_budget() {
    use crate::parsers::ytdlp_error::{DiagnosticMatcher, YtdlpErrorParser};
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for (error, cancelled) in [
      ("ERROR: getaddrinfo failed", false),
      ("ERROR: Network is unreachable", false),
      ("ERROR: Connection refused", false),
      ("ERROR: Connection timed out", false),
      ("ERROR: Unsupported URL", true),
    ] {
      let mut session = LoginSession::from_auth(
        AuthSettings::default(),
        "https://facebook.com/watch/",
        Some("firefox".into()),
      );
      session.record_failure(&parser.parse_lines(vec![error], true), false);
      assert_eq!(
        session
          .decide(None, false, cancelled, |_| panic!("no discovery"))
          .0,
        RetryAction::Stop,
        "{error}"
      );
      assert!(!session.state.anonymous_attempted);
      if !cancelled {
        assert_eq!(
          crate::parsers::gallery::failure(error),
          crate::parsers::gallery::GalleryError::NetworkDown
        );
      }
    }
  }

  #[test]
  fn terminal_network_failure_keeps_its_diagnostic_after_a_cookie_failure() {
    use crate::parsers::ytdlp_error::{DiagnosticMatcher, YtdlpErrorParser};
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    let mut session = LoginSession::from_auth(
      AuthSettings::default(),
      "https://facebook.com/watch/",
      Some("firefox".into()),
    );
    session.record_failure(
      &parser.parse_lines(vec!["ERROR: Unsupported URL"], true),
      false,
    );
    assert_eq!(
      session
        .decide(None, false, false, |_| panic!("no discovery"))
        .0,
      RetryAction::Anonymous
    );
    // Gallery can classify a transport outage even when it exposes only a generic diagnostic.
    let diagnostics = parser.parse_lines(vec!["ERROR: Request failed"], false);
    session.record_failure(&diagnostics, true);
    assert_eq!(
      session
        .decide(None, false, false, |_| panic!("no discovery"))
        .0,
      RetryAction::Stop
    );
    assert_eq!(
      session
        .failure_diagnostics(diagnostics)
        .last()
        .unwrap()
        .code,
      "unknown"
    );
  }

  #[test]
  fn anonymous_mode_survives_a_gallery_to_video_session_handoff() {
    use crate::runners::{gallery::arguments, ytdlp_runner::cookie_source_args};
    let auth = AuthSettings {
      cookie_browser: "firefox".into(),
      cookie_file: Some("/manual.txt".into()),
      ..Default::default()
    };
    let mut session = LoginSession::from_auth(
      auth.clone(),
      "https://facebook.com/watch/",
      Some(ANONYMOUS_BROWSER.into()),
    );
    assert!(!session.uses_cookies());
    assert!(cookie_source_args(&auth, session.browser.as_deref()).is_empty());
    assert!(!arguments(
      &cookie_source_args(&auth, session.browser.as_deref()),
      std::path::Path::new("/config.json")
    )
    .iter()
    .any(|arg| arg.starts_with("--cookies")));
    assert_eq!(
      session
        .decide(None, false, false, |_| panic!("no discovery"))
        .0,
      RetryAction::Stop
    );
    assert!(session.success_patch().is_none());
  }

  #[test]
  fn anonymous_fetch_ignores_every_cookie_setting_and_does_not_erase_host_memory() {
    use crate::runners::{gallery::arguments, ytdlp_runner::cookie_source_args};
    for (mode, file) in [
      ("automatic", None),
      ("firefox", None),
      ("automatic", Some("/private/manual.txt")),
    ] {
      let mut auth = AuthSettings {
        cookie_browser: mode.into(),
        cookie_file: file.map(str::to_owned),
        ..Default::default()
      };
      record_host_browser(
        &mut auth,
        "www.instagram.com",
        Some("chrome:/remembered".into()),
      );
      let mut session =
        LoginSession::from_auth(auth.clone(), "https://www.instagram.com/p/123/", None);
      session.begin_fetch(Phase::Anonymous);
      assert!(!session.uses_cookies());
      assert!(cookie_source_args(&auth, session.browser.as_deref()).is_empty());
      assert!(!arguments(
        &cookie_source_args(&auth, session.browser.as_deref()),
        std::path::Path::new("/config.json")
      )
      .iter()
      .any(|arg| arg.starts_with("--cookies")));
      assert!(session.success_patch().is_none());
      assert_eq!(
        session.auth.site_browsers["instagram.com"],
        "chrome:/remembered"
      );
    }
  }

  #[test]
  fn fetch_consent_reaches_download_and_retry_without_a_second_prompt() {
    let group = uuid::Uuid::new_v4().to_string();
    let mut request = pending(&group).request;
    request.consent_to_browser("chrome:/selected");
    // Fetch Cleanup removes scheduler state, but consent must reach the download.
    crate::scheduling::group_state::remove_group(&group);
    let item = serde_json::from_value::<crate::models::DownloadItem>(serde_json::json!({
      "id":"login-item", "url":"https://example.com/private", "format":{"trackType":"both"}, "templateContext":{"values":{}}
    })).unwrap();
    let mut foreign = item.clone();
    foreign.url = "https://different.example/private".into();
    let foreign_download = DownloadEntry::from((foreign, group.clone()));
    assert!(
      foreign_download.login_browser.is_none(),
      "same row id cannot transfer consent to another host"
    );
    let download = DownloadEntry::from((item.clone(), group.clone()));
    assert_eq!(download.login_browser.as_deref(), Some("chrome:/selected"));
    assert!(download.login_was_accepted);
    for retry in [download.clone(), download.login_request_entry()] {
      let mut login =
        LoginSession::from_auth(AuthSettings::default(), &retry.url, retry.login_browser);
      assert_eq!(
        login
          .decide(Some("signInRequired"), false, false, |_| panic!(
            "download asked again"
          ))
          .0,
        RetryAction::Stop
      );
    }
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = tauri::test::mock_app();
    app.manage(DownloadSender(tx.clone()));
    remember_row_browser(&group, "waiting-sibling", "example.com", "firefox");
    LoginRequest::Download(download).release(app.handle());
    let retry = DownloadEntry::from((item, group.clone()));
    assert_eq!(
      retry.login_browser.as_deref(),
      Some("chrome:/selected"),
      "failed finish must preserve consent for the retried row"
    );
    crate::scheduling::download_pipeline::finish_download(&tx, &group, "login-item", true);
    assert!(row_browser(&group, "login-item", "https://example.com/private").is_none());
    assert_eq!(
      row_browser(&group, "waiting-sibling", "https://example.com/private").as_deref(),
      Some("firefox")
    );
    // Removing a fetched row from the list cancels its group without downloading it.
    request.consent_to_browser("chrome:/selected");
    crate::scheduling::group_state::cancel_group(&group);
    assert!(row_browser(&group, "login-item", "https://example.com/private").is_none());
    crate::scheduling::group_state::remove_group(&group);
  }

  #[test]
  fn remembered_host_browser_is_used_on_first_fetch_and_download_attempts() {
    use crate::runners::ytdlp_runner::cookie_source_args;
    for (mode, file, host, accepted, expected) in [
      (
        "automatic",
        None,
        "www.instagram.com",
        None,
        Some("chrome:/selected"),
      ),
      ("automatic", None, "other.test", None, None),
      (
        "automatic",
        None,
        "www.instagram.com",
        Some("firefox:/accepted"),
        Some("firefox:/accepted"),
      ),
      ("none", None, "www.instagram.com", None, None),
      ("firefox", None, "www.instagram.com", None, Some("firefox")),
      (
        "automatic",
        Some("/manual.txt"),
        "www.instagram.com",
        None,
        Some("/manual.txt"),
      ),
    ] {
      let mut auth = AuthSettings {
        cookie_browser: mode.into(),
        cookie_file: file.map(str::to_owned),
        ..Default::default()
      };
      record_host_browser(
        &mut auth,
        "www.instagram.com",
        Some("chrome:/selected".into()),
      );
      // Both runners construct their session at this boundary, including gallery fallbacks.
      let session = LoginSession::from_auth(
        auth.clone(),
        &format!("https://{host}/reel/test/"),
        accepted.map(str::to_owned),
      );
      let args = cookie_source_args(&auth, session.browser.as_deref());
      assert_eq!(
        args.get(1).map(String::as_str),
        expected,
        "{mode}, {file:?}, {host}"
      );
      assert_eq!(session.state.browser_attempted, session.browser.is_some());
    }
  }

  #[tokio::test]
  async fn transient_cookie_fetch_retries_once_with_injected_backoff() {
    use std::cell::Cell;
    for (accepted, failure, expected) in [
      (Some("chrome"), CookieFetchFailure::Transient, true),
      (None, CookieFetchFailure::Transient, false),
      (Some("chrome"), CookieFetchFailure::Refused, false),
      (Some("chrome"), CookieFetchFailure::Other, false),
    ] {
      let mut session = LoginSession::from_auth(
        AuthSettings::default(),
        "https://www.instagram.com/reel/test/",
        accepted.map(str::to_owned),
      );
      let waits = Cell::new(0);
      assert_eq!(
        session
          .retry_cookie_fetch(failure, |duration| {
            assert!(!duration.is_zero());
            waits.set(waits.get() + 1);
            std::future::ready(())
          })
          .await,
        expected
      );
      assert_eq!(waits.get(), usize::from(expected));
      assert!(
        !session
          .retry_cookie_fetch(failure, |_| async { panic!("second backoff") })
          .await
      );
    }
  }

  #[test]
  fn older_success_cannot_overwrite_a_newer_same_host_browser_choice() {
    let mut auth = AuthSettings::default();
    record_host_browser(&mut auth, "www.instagram.com", Some("firefox:/new".into()));
    remember_success(
      &mut auth,
      "www.instagram.com",
      Some("chrome:/old"),
      "chrome:/old",
    );
    remember_success(&mut auth, "other.test", None, "chrome:/other");
    assert_eq!(auth.site_browsers["instagram.com"], "firefox:/new");
    assert_eq!(auth.site_browsers["other.test"], "chrome:/other");
    assert!(!auth.always_use_browser_logins);
  }

  #[test]
  fn same_host_waiters_share_one_prompt_and_acceptance_reaches_every_row() {
    let groups: Vec<_> = (0..70).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let mut map = HashMap::new();
    let mut prompts = 0;
    for (i, group) in groups.iter().enumerate() {
      prompts += usize::from(queue_pending(
        &mut map,
        format!("token-{i}"),
        pending(group),
      ));
    }
    assert_eq!((prompts, map.len()), (1, 1));
    let claimed = claim_pending(&mut map, "token-0", "chrome:/fake/Profile 1").unwrap();
    let mut resumed = Vec::new();
    for mut row in claimed.into_rows() {
      row.request.consent_to_browser("chrome:/fake/Profile 1");
      let LoginRequest::Fetch(entry) = row.request else {
        panic!("fetch waiter");
      };
      assert!(entry.login_was_accepted);
      assert_eq!(
        entry.login_browser.as_deref(),
        Some("chrome:/fake/Profile 1")
      );
      resumed.push(entry.group_id);
    }
    assert_eq!(resumed, groups);
    assert!(claim_pending(&mut map, "token-0", "chrome:/fake/Profile 1").is_err());
    for group in groups {
      forget_group(&group);
      crate::scheduling::group_state::remove_group(&group);
    }
  }

  #[test]
  fn cancelling_the_prompt_owner_preserves_other_rows_and_the_single_use_token() {
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    let mut map = HashMap::new();
    assert!(queue_pending(&mut map, "gate".into(), pending(&first)));
    assert!(!queue_pending(
      &mut map,
      "follower".into(),
      pending(&second)
    ));
    remove_pending_group(&mut map, &first);
    assert_eq!(map.len(), 1);
    let claimed = claim_pending(&mut map, "gate", "chrome:/fake/Profile 1").unwrap();
    assert_eq!(claimed.request.ids().1, second);
    assert!(claimed.followers.is_empty());
    crate::scheduling::group_state::remove_group(&first);
    crate::scheduling::group_state::remove_group(&second);
  }

  #[tokio::test]
  async fn unanswered_login_emits_terminal_failure_and_releases_its_request() {
    let group = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    {
      let mut pending_map = PENDING.lock().unwrap();
      let same_host = format!("https://gate-{group}.example/private");
      let mut first_row = pending(&group);
      let mut second_row = pending(&second);
      if let LoginRequest::Fetch(entry) = &mut first_row.request {
        entry.url = same_host.clone();
      }
      if let LoginRequest::Fetch(entry) = &mut second_row.request {
        entry.url = same_host;
      }
      assert!(queue_pending(&mut pending_map, token.clone(), first_row));
      assert!(!queue_pending(
        &mut pending_map,
        "unused-follower".into(),
        second_row
      ));
    }
    let mut fatals = Vec::new();
    let mut released = Vec::new();
    expire_pending(
      &token,
      std::future::ready(()),
      |diagnostic, event| {
        assert_eq!(diagnostic.code, "loginConsentTimeout");
        fatals.push(event);
      },
      |request| released.push(request.ids().1.to_owned()),
    )
    .await;
    assert_eq!(fatals.len(), 2);
    let fatal = &fatals[0];
    assert_eq!(fatal.group_id, group);
    assert_eq!(fatal.id, "login-item");
    assert!(!fatal.internal);
    assert!(fatal.message.contains("timed out"));
    assert_eq!(fatal.details.as_deref(), Some("loginConsentTimeout"));
    assert_eq!(released, vec![group.clone(), second.clone()]);
    assert!(!PENDING.lock().unwrap().contains_key(&token));
    expire_pending(
      &token,
      std::future::ready(()),
      |_, _| panic!("duplicate fatal"),
      |_| panic!("duplicate release"),
    )
    .await;
    crate::scheduling::group_state::remove_group(&group);
    crate::scheduling::group_state::remove_group(&second);
  }
  #[tokio::test]
  async fn accept_and_change_leave_shared_config_unchanged_after_a_failed_attempt() {
    use crate::state::{config::TestConfigHandle, config_models::Config};
    use std::sync::Arc;
    let root = crate::runners::gallery_files::Staging::new().unwrap();
    let app = tauri::test::mock_builder()
      .plugin(tauri_plugin_store::Builder::default().build())
      .build(tauri::test::mock_context(tauri::test::noop_assets()))
      .unwrap();
    let mut initial = Config::default();
    initial
      .auth
      .site_browsers
      .insert("example.com".into(), "firefox:/old".into());
    let config =
      Arc::new(TestConfigHandle::isolated_for_test(app.handle(), &root.path, initial).unwrap());
    app.manage(config.clone());
    let (fetch_tx, mut fetch_rx) = tokio::sync::mpsc::unbounded_channel();
    let (download_tx, _download_rx) = tokio::sync::mpsc::unbounded_channel();
    app.manage(FetchSender(fetch_tx));
    app.manage(DownloadSender(download_tx));
    let before = serde_json::to_value(config.load().as_ref()).unwrap();
    for change_only in [false, true] {
      let group = uuid::Uuid::new_v4().to_string();
      let token = uuid::Uuid::new_v4().to_string();
      let offered = candidates(None);
      let browser = offered[0].id.clone();
      let mut entry = match pending(&group).request {
        LoginRequest::Fetch(entry) => entry,
        _ => unreachable!(),
      };
      if change_only {
        CHOICES.lock().unwrap().insert(
          token.clone(),
          ProfileChoice {
            used: LoginUsing {
              id: entry.id.clone(),
              group_id: group.clone(),
              host: "example.com".into(),
              candidate: offered[0].clone(),
              selected_only: false,
            },
            candidates: offered,
            created: Instant::now(),
          },
        );
      } else {
        PENDING
          .lock()
          .unwrap()
          .insert(token.clone(), pending(&group));
      }
      accept_login(app.handle().clone(), token, opaque_candidate_id(&browser)).unwrap();
      if change_only {
        entry.login_browser = row_browser(&group, &entry.id, &entry.url);
        entry.login_was_accepted = true;
      } else {
        let DispatchRequest::Pipeline(FetchRequest::Resume(queued)) = fetch_rx.try_recv().unwrap()
        else {
          panic!("resume not queued");
        };
        entry = *queued;
      }
      assert_eq!(entry.login_browser.as_deref(), Some(browser.as_str()));
      let mut session =
        LoginSession::from_auth(config.load().auth.clone(), &entry.url, entry.login_browser);
      session.begin_fetch(Phase::Login);
      let outcome = crate::extract::chain::failed_login_for_test(&entry.url, session).await;
      assert_eq!(outcome, Some("signInRequired"));
      assert_eq!(
        serde_json::to_value(app.state::<Arc<TestConfigHandle>>().load().as_ref()).unwrap(),
        before,
        "{change_only}: acceptance and the failed worker must not alter SharedConfig"
      );
      forget_group(&group);
      USED.lock().unwrap().remove(&group);
      crate::scheduling::group_state::remove_group(&group);
    }
    assert!(!root.path.join("config.store.json").exists());
  }

  #[test]
  fn acceptance_of_a_cancelled_owner_still_resumes_live_followers() {
    let app = tauri::test::mock_app();
    let (fetch_tx, mut fetch_rx) = tokio::sync::mpsc::unbounded_channel();
    let (download_tx, _download_rx) = tokio::sync::mpsc::unbounded_channel();
    app.manage(FetchSender(fetch_tx));
    app.manage(DownloadSender(download_tx));
    let cancelled = uuid::Uuid::new_v4().to_string();
    let live = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let mut owner = pending(&cancelled);
    let (_tx, rx) = watch::channel(false);
    owner.cancel = rx;
    owner.followers.push(pending(&live));
    PENDING.lock().unwrap().insert(token.clone(), owner);
    accept_login(
      app.handle().clone(),
      token,
      opaque_candidate_id("chrome:/fake/Profile 1"),
    )
    .unwrap();
    let mut resumed = Vec::new();
    while let Ok(message) = fetch_rx.try_recv() {
      if let DispatchRequest::Pipeline(FetchRequest::Resume(entry)) = message {
        resumed.push(entry.group_id);
      }
    }
    assert_eq!(resumed, vec![live.clone()]);
    forget_group(&cancelled);
    forget_group(&live);
    crate::scheduling::group_state::remove_group(&cancelled);
    crate::scheduling::group_state::remove_group(&live);
  }

  #[test]
  fn anonymous_fetch_success_carries_no_cookies_into_the_download() {
    let group = uuid::Uuid::new_v4().to_string();
    let mut previous = pending(&group).request;
    if let LoginRequest::Fetch(entry) = &mut previous {
      entry.id = "item".into();
      entry.url = "https://redd.it/item".into();
    }
    previous.consent_to_browser("chrome:/remembered");
    let LoginRequest::Fetch(previous) = previous else {
      unreachable!()
    };
    let fresh = crate::scheduling::fetch_pipeline::fresh_fetch_for_test(&previous);
    assert!(
      fresh.login_browser.is_none() && !fresh.login_was_accepted,
      "re-fetches start anonymous even when this row has an accepted browser"
    );
    let mut auth = AuthSettings::default();
    record_host_browser(&mut auth, "reddit.com", Some("chrome:/remembered".into()));
    let mut session = LoginSession::from_auth(auth, "https://redd.it/item", None);
    session.begin_fetch(Phase::Anonymous);
    session.carry_fetch_browser("item", &group);
    let item = serde_json::from_value::<crate::models::DownloadItem>(serde_json::json!({
      "id":"item", "url":"https://old.reddit.com/item", "format":{"trackType":"both"}, "templateContext":{"values":{}}
    })).unwrap();
    let download = DownloadEntry::from((item, group.clone()));
    assert_eq!(download.login_browser.as_deref(), Some("none"));
    forget_group(&group);
  }

  fn pending(group_id: &str) -> PendingLogin {
    PendingLogin {
      followers: Vec::new(),
      announcement: None,
      request: LoginRequest::Fetch(FetchEntry {
        group_id: group_id.into(),
        id: "login-item".into(),
        url: "https://example.com/private".into(),
        total: 1,
        format: None,
        overrides: None,
        login_browser: None,
        login_was_accepted: false,
        prior_attempts: Vec::new(),
        prior_resolved_url: None,
      }),
      candidates: candidates(None),
      cancel: subscribe_group(group_id),
      created: Instant::now(),
      cleanup: CleanupChannels {
        fetch: FetchSender(tokio::sync::mpsc::unbounded_channel().0),
        download: DownloadSender(tokio::sync::mpsc::unbounded_channel().0),
      },
    }
  }
  #[test]
  fn consent_is_single_use_and_cannot_substitute_an_unoffered_profile() {
    let group = uuid::Uuid::new_v4().to_string();
    let mut map = HashMap::new();
    map.insert("token".into(), pending(&group));
    assert!(claim_pending(&mut map, "token", "chrome:/other/private-profile").is_err());
    assert_eq!(map.len(), 1);
    let selected = claim_pending(&mut map, "token", "chrome:/fake/Profile 1").unwrap();
    assert_eq!(selected.request.ids().0, "login-item");
    assert!(map.is_empty());
    assert!(claim_pending(&mut map, "token", "chrome:/fake/Profile 1").is_err());
    let mut choices = HashMap::from([(
      "choice".into(),
      ProfileChoice {
        used: LoginUsing {
          id: "login-item".into(),
          group_id: group.clone(),
          host: "example.com".into(),
          candidate: candidates(None).remove(0),
          selected_only: false,
        },
        candidates: candidates(None),
        created: Instant::now(),
      },
    )]);
    assert!(claim_choice(&mut choices, "choice", "chrome:/other/private-profile").is_err());
    assert_eq!(choices.len(), 1);
    let selected = claim_choice(&mut choices, "choice", "chrome:/fake/Profile 1").unwrap();
    assert_eq!(selected.used.group_id, group);
    assert!(choices.is_empty());
    assert!(claim_choice(&mut choices, "choice", "chrome:/fake/Profile 1").is_err());

    crate::scheduling::group_state::remove_group(&group);
  }
  #[tokio::test]
  async fn cancelling_a_group_invalidates_its_pending_login() {
    let group = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let (fetch, mut fetch_rx) = tokio::sync::mpsc::unbounded_channel();
    let (download, mut download_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut request = pending(&group);
    request.cleanup = CleanupChannels {
      fetch: FetchSender(fetch),
      download: DownloadSender(download),
    };
    PENDING.lock().unwrap().insert(token.clone(), request);
    let cancel = subscribe_group(&group);
    crate::scheduling::group_state::cancel_group(&group);
    assert!(!*cancel.borrow());
    assert!(!PENDING.lock().unwrap().contains_key(&token));
    assert!(
      matches!(fetch_rx.try_recv().unwrap(),DispatchRequest::Cleanup {group_id} if group_id == group)
    );
    assert!(
      matches!(download_rx.try_recv().unwrap(),DispatchRequest::Cleanup {group_id} if group_id == group)
    );
    expire_pending(
      &token,
      std::future::ready(()),
      |_, _| panic!("fatal after cancel"),
      |_| panic!("duplicate cleanup"),
    )
    .await;
    crate::scheduling::group_state::remove_group(&group);
  }
  fn candidates(_: Option<&str>) -> Vec<LoginCandidate> {
    vec![LoginCandidate {
      id: "chrome:/fake/Profile 1".into(),
      browser: "chrome".into(),
      profile: "Profile 1".into(),
      signed_in: false,
      app_bound: false,
    }]
  }
  #[test]
  fn no_cookie_probes_before_refusal_or_after_cancel_and_first_refusal_stops() {
    let mut session = LoginSession::from_auth(
      AuthSettings {
        cookie_browser: "automatic".into(),
        ..Default::default()
      },
      "https://www.instagram.com/p/test",
      None,
    );
    assert_eq!(
      session
        .decide(None, false, false, |_| panic!("unexpected profile probe"))
        .0,
      RetryAction::Stop
    );
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, true, |_| panic!(
          "probe after cancel"
        ))
        .0,
      RetryAction::Stop
    );
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, candidates)
        .0,
      RetryAction::AskLogin
    );
    assert!(session.browser.is_none());
    assert!(session.success_patch().is_none());
  }
  #[test]
  fn opted_in_refusal_tries_each_profile_once_and_success_remembers_the_host() {
    let auth = AuthSettings {
      cookie_browser: "automatic".into(),
      always_use_browser_logins: true,
      ..Default::default()
    };
    let mut session = LoginSession::from_auth(
      auth,
      "https://www.instagram.com/p/private?token=secret",
      None,
    );
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, candidates)
        .0,
      RetryAction::Browser
    );
    assert_eq!(session.browser.as_deref(), Some("chrome:/fake/Profile 1"));
    let patch = session.success_patch().unwrap();
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, candidates)
        .0,
      RetryAction::AskLogin
    );
    let mut config = serde_json::to_value(crate::state::config_models::Config::default()).unwrap();
    crate::state::json_merge(&mut config, &patch);
    let config: crate::state::config_models::Config = serde_json::from_value(config).unwrap();
    assert_eq!(config.auth.site_browsers.len(), 1);
    assert_eq!(
      config.auth.site_browsers["instagram.com"],
      "chrome:/fake/Profile 1"
    );
  }
  #[test]
  fn explicit_browser_and_file_overrides_do_not_trigger_automatic_probes() {
    for auth in [
      AuthSettings {
        cookie_browser: "firefox".into(),
        ..Default::default()
      },
      AuthSettings {
        cookie_file: Some("/manual.txt".into()),
        ..Default::default()
      },
    ] {
      let mut session = LoginSession::from_auth(auth, "https://example.com/", None);
      assert_eq!(
        session
          .decide(Some("signInRequired"), false, false, |_| panic!(
            "manual override probe"
          ))
          .0,
        RetryAction::Stop
      );
    }
  }
}
