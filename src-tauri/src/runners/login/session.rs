//! One fetch or download's login attempts: which browser to try next, the transient
//! cookie-read backoff, and the failure diagnostics worth showing.
use super::consent::{
  may_reuse_remembered, remember_row_browser, remember_success, row_browser, CONSENT_WRITES,
  EXPLICIT_SITES,
};
use super::registry::{announce_using, park, used_candidate, LoginRequest};
use super::{
  discovered_candidates, LoginCandidate, LoginNeeded, LoginStatus, LoginUsing, ANONYMOUS_BROWSER,
  APP_BOUND_REASON, LOGIN_FAILED_REASON,
};
use crate::extract::{outcome::Attempt, policy::Phase};
use crate::models::download::DownloadOverrides;
use crate::models::error::{DiagnosticEvent, DiagnosticLevel};
use crate::parsers::ytdlp_error::CookieFetchFailure;
use crate::runners::browser_cookies::{known_login_host, migrate_site_browsers, site_key};
use crate::runners::browser_logins::describe;
use crate::runners::browsers::{self, BrowserSetting};
use crate::runners::override_resolver::resolve_with_patch;
use crate::runners::retry_policy::{RetryAction, RetryFailure, RetryState};
use crate::scheduling::{fetch_pipeline::FetchEntry, group_state::is_group_running};
use crate::state::config_models::AuthSettings;
use crate::SharedConfig;
use std::{
  collections::{HashMap, HashSet},
  sync::{LazyLock, Mutex},
  time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tokio::sync::watch;

/// What a transient cookie-read failure leads to; the engine decides what each means for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieRetry {
  /// The backoff ran: try the same attempt again.
  Retry,
  /// Not a transient failure, or this session/host already used its retry.
  GiveUp,
  /// The group was cancelled (or finished) before or during the backoff.
  Cancelled,
}

/// Wait before reading browser cookies again after a transient failure.
pub(super) const COOKIE_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct LoginSession {
  pub(super) fetch_managed: bool,
  pub(super) fetch_browser: Option<String>,
  pub(super) fetch_gate_used: bool,
  pub(super) fetch_attempts: Vec<Attempt>,
  pub(super) fetch_resolved_url: Option<String>,
  pub state: RetryState,
  pub browser: Option<String>,
  /// True only after preparation found site-scoped cookie rows for this attempt.
  pub has_session: bool,
  pub(super) auth: AuthSettings,
  pub(super) host: String,
  pub(super) cookie_fetch_retried: bool,
  pub(super) just_accepted: bool,
  pub(super) attempted: HashSet<String>,
  pub(super) starting_browser: Option<String>,
  pub(super) selected: Option<LoginCandidate>,
  pub(super) blocked_browser: Option<browsers::BrowserKind>,
  pub(super) network_down: bool,
  pub(super) best_failure: Option<DiagnosticEvent>,
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
  pub(super) fn from_auth(mut auth: AuthSettings, url: &str, accepted: Option<String>) -> Self {
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
      (browsers::automatic(&auth) && may_reuse_remembered(&auth, &host))
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
      has_session: false,
      auth,
      host,
      cookie_fetch_retried: false,
      just_accepted,
      starting_browser,
      selected: None,
      blocked_browser: None,
      network_down: false,
      best_failure: None,
    }
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
    if reason == "drmProtected"
      && (!browsers::automatic(&self.auth)
        || !discovered_candidates(&self.host, self.starting_browser.as_deref())
          .iter()
          .any(|candidate| !candidate.app_bound))
    {
      return RetryAction::Stop;
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
  pub(super) fn carry_fetch_browser(&self, id: &str, group: &str) {
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
    self.emit_failure(app);
    crate::events::emit(
      app,
      crate::events::MEDIA_LOGIN_NEEDED,
      LoginNeeded {
        request_id: uuid::Uuid::new_v4().to_string(),
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        host: self.host.clone(),
        reason: LOGIN_FAILED_REASON.into(),
        blocked_browser: None,
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
    let cached = used_candidate(group_id, self.browser.as_deref());
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
      || matches!(BrowserSetting::of(&self.auth), BrowserSetting::Fixed(_))
  }

  /// Retain only redacted diagnostics, never cookie contents or source URLs.
  pub fn record_failure(&mut self, diagnostics: &[DiagnosticEvent], network_down: bool) {
    self.network_down = network_down
      || diagnostics.iter().any(|event| {
        matches!(event.level, DiagnosticLevel::Error)
          && crate::parsers::ytdlp_error::network_down(&event.code, &event.message)
      });
    if self.network_down {
      crate::diagnostics::send::network_response_failed();
    }
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

  /// One backoff per session and per host window after a transient cookie-read failure,
  /// raced against the group's cancel signal (checked first, as every engine did inline).
  pub async fn retry_cookie_fetch(
    &mut self,
    failure: CookieFetchFailure,
    cancel: &mut watch::Receiver<bool>,
  ) -> CookieRetry {
    tokio::select! {
      biased;
      _ = cancel.changed() => CookieRetry::Cancelled,
      retry = self.cookie_backoff(failure) => {
        if retry { CookieRetry::Retry } else { CookieRetry::GiveUp }
      }
    }
  }

  async fn cookie_backoff(&mut self, failure: CookieFetchFailure) -> bool {
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
    tokio::time::sleep(COOKIE_RETRY_DELAY).await;
    true
  }

  pub(super) fn failure_status(
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

  pub(super) fn failure_reason<'a>(
    &self,
    reason: Option<&'a str>,
    candidates: &[LoginCandidate],
  ) -> &'a str {
    if self.blocked_browser.is_some() && !candidates.iter().any(|c| c.signed_in) {
      APP_BOUND_REASON
    } else {
      reason.unwrap_or(LOGIN_FAILED_REASON)
    }
  }

  fn emit_failure(&self, app: &AppHandle) {
    if let Some(event) = &self.best_failure {
      crate::events::emit(
        app,
        crate::events::MEDIA_DIAGNOSTIC,
        crate::models::MediaDiagnosticPayload::from_diagnostic_event(event.clone()),
      );
    }
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
        reason: self.failure_reason(reason, &candidates).into(),
        blocked_browser: self.blocked_browser,
        status,
        candidates: candidates.clone(),
        change_only: false,
      };
      if !exhausted
        && !park(
          app,
          &self.host,
          request_id,
          payload.clone(),
          request,
          candidates,
        )
      {
        return RetryAction::AskLogin;
      }
      self.emit_failure(app);
      crate::events::emit(app, crate::events::MEDIA_LOGIN_NEEDED, payload);
      if exhausted {
        return RetryAction::Stop;
      }
    }
    action
  }
  pub(super) fn decide(
    &mut self,
    reason: Option<&str>,
    aria_denied: bool,
    cancelled: bool,
    find: impl FnOnce(Option<&str>) -> Vec<LoginCandidate>,
  ) -> (RetryAction, Vec<LoginCandidate>) {
    let automatic = browsers::automatic(&self.auth);
    let timed_out = self
      .best_failure
      .as_ref()
      .is_some_and(|event| matches!(event.code.as_str(), "networkTimeout" | "fetchTimeout"));
    if cancelled
      || self.network_down
      || timed_out
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
          network_down: self.network_down || timed_out,
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
    self.blocked_browser = candidates
      .iter()
      .find(|c| c.app_bound)
      .map(|c| c.browser)
      .or(self.blocked_browser);
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
  pub(super) fn success_patch(&self) -> Option<serde_json::Value> {
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
    "accountAccessDenied"
    | "browserCookiesUnreadable"
    | "browserKeychainDenied"
    | "browserSafariAccessDenied"
    | "browserDatabaseLocked"
    | "browserProfileMissing" => 4,
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
const HOST_RETRY_WINDOW: Duration = Duration::from_secs(30);
static TRANSIENT_RETRIES: LazyLock<Mutex<HashMap<String, Instant>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) fn claim_host_retry(
  budget: &mut HashMap<String, Instant>,
  host: &str,
  now: Instant,
) -> bool {
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
#[cfg(test)]
pub(crate) fn test_consented_session() -> LoginSession {
  LoginSession::from_auth(
    AuthSettings::default(),
    "https://www.instagram.com/p/123/",
    Some("firefox".into()),
  )
}
