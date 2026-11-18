//! Automatic retries run while the row is parked, outside extraction-stage deadlines.
use super::{
  registry::{park, LoginRequest, PENDING},
  LoginNeeded, LoginStatus, LOGIN_TTL, NEZZEL_SESSION,
};
use crate::{
  runners::{
    browser_cookies::{CookieAccess, CookieContext},
    browser_logins,
    browsers::BrowserKind,
    retry_policy::RetryAction,
  },
  SharedConfig,
};
use std::{
  collections::{HashMap, HashSet},
  sync::{Arc, LazyLock, Mutex, Weak},
  time::Instant,
};
use tauri::{AppHandle, Manager};

struct RowAttempts {
  sources: HashSet<String>,
  attempts: u8,
  opened: bool,
  browser_proven: bool,
  running: bool,
  created: Instant,
}
impl RowAttempts {
  fn new() -> Self {
    Self {
      sources: HashSet::new(),
      attempts: 0,
      opened: false,
      browser_proven: false,
      running: false,
      created: Instant::now(),
    }
  }
  fn claim(&mut self, source: &str) -> bool {
    self.sources.insert(source.into())
  }
  fn session_expired(&mut self) {
    self.browser_proven = false;
  }
  fn may_schedule(&self) -> bool {
    !self.opened && self.attempts < MAX_AUTOMATIC_ATTEMPTS
  }
  fn begin(&mut self) -> Result<(), RetryAction> {
    if self.running {
      return Err(RetryAction::Pending);
    }
    if !self.may_schedule() {
      return Err(RetryAction::Stop);
    }
    self.running = true;
    self.attempts += 1;
    Ok(())
  }
  fn may_open_sign_in(&self) -> bool {
    !self.opened && !self.browser_proven
  }
  fn merge(&mut self, shared: &Self) {
    self.sources.extend(shared.sources.iter().cloned());
    self.browser_proven |= shared.browser_proven;
    self.opened |= shared.opened;
    self.running = false;
  }
  fn complete(&mut self, shared: &Self, result: LoginResult) -> LoginResult {
    // Check this row's history before importing the shared attempt's newly claimed sources.
    // Other rows may still legitimately retry this source for the first time.
    let result = match result {
      Ok(source) if self.sources.contains(&source) => Err("browserLoginExhausted"),
      result => result,
    };
    self.merge(shared);
    result
  }
}
type AttemptKey = (String, String, String);
static ATTEMPTS: LazyLock<Mutex<HashMap<AttemptKey, RowAttempts>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

type LoginResult = Result<String, &'static str>;
struct SharedAttempt {
  result: futures::future::Shared<futures::future::BoxFuture<'static, LoginResult>>,
  state: std::sync::Arc<Mutex<RowAttempts>>,
}
static SHARED_ATTEMPTS: LazyLock<Mutex<HashMap<String, Weak<SharedAttempt>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
const AUTOMATIC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_AUTOMATIC_ATTEMPTS: u8 = 3;

// Weak entries share only an in-flight attempt. Dropping the last waiter cancels its work;
// one cancelled group cannot abort another group's browser selection or sign-in window.
fn shared_attempt<F>(
  host: &str,
  choose: impl FnOnce(std::sync::Arc<Mutex<RowAttempts>>) -> F,
) -> std::sync::Arc<SharedAttempt>
where
  F: std::future::Future<Output = LoginResult> + Send + 'static,
{
  use futures::FutureExt;
  let mut attempts = SHARED_ATTEMPTS.lock().unwrap();
  attempts.retain(|_, attempt| attempt.strong_count() > 0);
  if let Some(attempt) = attempts
    .get(host)
    .and_then(Weak::upgrade)
    .filter(|attempt| attempt.result.peek().is_none())
  {
    return attempt;
  }
  // A host-wide attempt belongs to all waiting rows, not to the first row's history.
  let state = Arc::new(Mutex::new(RowAttempts::new()));
  let work = choose(state.clone());
  let attempt = Arc::new(SharedAttempt {
    state,
    result: work.boxed().shared(),
  });
  attempts.insert(host.to_owned(), Arc::downgrade(&attempt));
  attempt
}

pub(super) fn forget_group(group: &str) {
  ATTEMPTS.lock().unwrap().retain(|(g, _, _), _| g != group);
}

pub(crate) fn source_order(
  kind: BrowserKind,
  app_bound: bool,
  windows: bool,
  remembered: bool,
) -> Option<(u8, bool)> {
  if windows && app_bound {
    return None;
  }
  Some((
    match kind {
      BrowserKind::Firefox => 0,
      BrowserKind::Safari => 2,
      _ => 1,
    },
    !remembered,
  ))
}

pub(super) fn schedule(
  app: &AppHandle,
  request: LoginRequest,
  expired: bool,
  failure: Option<crate::models::error::DiagnosticEvent>,
) -> RetryAction {
  let Some(host) = request.host() else {
    return RetryAction::Stop;
  };
  let (id, group) = request.ids();
  let key = (group.to_owned(), id.to_owned(), host.clone());
  {
    let mut rows = ATTEMPTS.lock().unwrap();
    // A slow active download must not acquire a fresh retry budget when the TTL elapses.
    rows.retain(|(group, _, _), row| {
      row.created.elapsed() < LOGIN_TTL || crate::scheduling::group_state::is_group_running(group)
    });
    if expired {
      if let Some(row) = rows.get_mut(&key) {
        row.session_expired();
      }
    }
    if rows.len() >= 128 && !rows.contains_key(&key) {
      return RetryAction::Stop;
    }
    if let Err(action) = rows
      .entry(key.clone())
      .or_insert_with(RowAttempts::new)
      .begin()
    {
      return action;
    }
  }
  let token = uuid::Uuid::new_v4().to_string();
  let payload = LoginNeeded {
    request_id: token.clone(),
    id: id.into(),
    group_id: group.into(),
    host: host.clone(),
    reason: "automaticLogin".into(),
    blocked_browser: None,
    status: LoginStatus::Needed,
    candidates: vec![],
    change_only: false,
  };
  park(app, &host, token.clone(), payload, request.clone(), vec![]);
  let app = app.clone();
  let choose_app = app.clone();
  let choose_host = host.clone();
  let source = request.url().to_owned();
  let attempt = shared_attempt(&host, move |state| async move {
    choose_source(&choose_app, &source, &choose_host, &state).await
  });
  tauri::async_runtime::spawn(async move {
    let mut cancel = crate::scheduling::group_state::subscribe_group(&key.0);
    if !*cancel.borrow() {
      if let Some(pending) = PENDING.lock().unwrap().remove(&token) {
        for row in pending.into_rows() {
          row.request.release(&app);
        }
      }
      if let Some(row) = ATTEMPTS.lock().unwrap().get_mut(&key) {
        row.running = false;
      }
      return;
    }
    let mut result = tokio::select! {
      biased;
      _ = cancel.changed() => Err("cancelled"),
      _ = wait_disabled(&app) => Err("browserLoginDisabled"),
      result = attempt.result.clone() => result,
    };
    if let Some(row) = ATTEMPTS.lock().unwrap().get_mut(&key) {
      let shared = attempt.state.lock().unwrap();
      result = row.complete(&shared, result);
    }
    let pending = PENDING.lock().unwrap().remove(&token);
    let Some(pending) = pending else {
      return;
    };
    for row in pending.into_rows() {
      if !*row.cancel.borrow() {
        row.request.release(&app);
        continue;
      }
      match &result {
        Ok(source) if enabled(&app) => {
          if row.request.clone().enqueue(&app, source.clone()).is_err() {
            row
              .request
              .fail(&app, "Browser login retry could not be queued");
          }
        }
        _ => {
          let (id, group) = row.request.ids();
          crate::events::emit(
            &app,
            crate::events::MEDIA_LOGIN_NEEDED,
            LoginNeeded {
              request_id: uuid::Uuid::new_v4().to_string(),
              id: id.into(),
              group_id: group.into(),
              host: host.clone(),
              reason: if expired {
                "browserSessionExpired"
              } else if failure.is_some() {
                super::LOGIN_FAILED_REASON
              } else {
                "browserLoginRemedy"
              }
              .into(),
              blocked_browser: None,
              status: LoginStatus::Exhausted,
              candidates: vec![],
              change_only: false,
            },
          );
          if let Some(event) = &failure {
            crate::events::emit(
              &app,
              crate::events::MEDIA_DIAGNOSTIC,
              crate::models::MediaDiagnosticPayload::from_diagnostic_event(event.clone()),
            );
          }
          row.request.fail(&app, "Browser login could not be used");
        }
      }
    }
  });
  RetryAction::Pending
}

pub(crate) fn enabled(app: &AppHandle) -> bool {
  super::global_consent(&app.state::<SharedConfig>().load().auth) == Some(true)
}
// Browser IO is bounded separately: an interactive window keeps webview_session's LOGIN_TTL.
async fn choose_phases<B, I>(browser: B, interactive: impl FnOnce() -> I) -> LoginResult
where
  B: std::future::Future<Output = Result<Option<String>, &'static str>>,
  I: std::future::Future<Output = LoginResult>,
{
  match tokio::time::timeout(AUTOMATIC_TIMEOUT, browser)
    .await
    .map_err(|_| "browserLoginTimeout")??
  {
    Some(source) => Ok(source),
    None => interactive().await,
  }
}

async fn choose_source(
  app: &AppHandle,
  source: &str,
  host: &str,
  state: &Arc<Mutex<RowAttempts>>,
) -> LoginResult {
  choose_phases(choose_browser_source(app, source, host, state), || async {
    if !enabled(app) {
      return Err("browserLoginDisabled");
    }
    {
      let mut row = state.lock().unwrap();
      if !row.may_open_sign_in() {
        return Err("browserLoginExhausted");
      }
      row.opened = true;
    }
    super::webview_session::read(app, host, true)
      .await?
      .ok_or("browserLoginRemedy")?;
    Ok(NEZZEL_SESSION.into())
  })
  .await
}

async fn choose_browser_source(
  app: &AppHandle,
  source: &str,
  host: &str,
  state: &Arc<Mutex<RowAttempts>>,
) -> Result<Option<String>, &'static str> {
  if !enabled(app) {
    return Err("browserLoginDisabled");
  }
  let try_session = state.lock().unwrap().claim(NEZZEL_SESSION);
  if try_session
    && super::webview_session::read(app, host, false)
      .await
      .ok()
      .flatten()
      .is_some()
  {
    return Ok(Some(NEZZEL_SESSION.into()));
  }
  let host_owned = host.to_owned();
  let remembered = app
    .state::<SharedConfig>()
    .load()
    .auth
    .site_browsers
    .get(host)
    .cloned();
  let discovery_remembered = remembered.clone();
  let mut candidates = tauri::async_runtime::spawn_blocking(move || {
    browser_logins::discover(discovery_remembered.as_deref(), &host_owned)
  })
  .await
  .map_err(|_| "browserCookiesUnreadable")?;
  candidates.retain(|c| {
    source_order(
      c.browser,
      c.app_bound,
      cfg!(windows),
      remembered.as_deref() == Some(c.id.as_str()),
    )
    .is_some()
  });
  candidates.sort_by_key(|c| {
    source_order(
      c.browser,
      c.app_bound,
      cfg!(windows),
      remembered.as_deref() == Some(c.id.as_str()),
    )
  });
  let context = CookieContext::for_app(app)?;
  let (_active, cancel) = tokio::sync::watch::channel(true);
  for candidate in candidates {
    if !enabled(app) {
      return Err("browserLoginDisabled");
    }
    if !state.lock().unwrap().claim(&candidate.id) {
      continue;
    }
    let auth = app.state::<SharedConfig>().load().auth.clone();
    let access =
      CookieAccess::prepare(&auth, Some(&candidate.id), source, &context, cancel.clone()).await;
    if let Ok(access) = access {
      if access.has_proof(host) && enabled(app) {
        state.lock().unwrap().browser_proven = true;
        return Ok(Some(candidate.id));
      }
    }
  }
  Ok(None)
}

async fn wait_disabled(app: &AppHandle) {
  while enabled(app) {
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  fn fresh_attempt() -> RowAttempts {
    RowAttempts {
      sources: HashSet::new(),
      attempts: 0,
      opened: false,
      browser_proven: false,
      running: true,
      created: Instant::now(),
    }
  }

  #[tokio::test(start_paused = true)]
  async fn concurrent_rows_share_one_login_result_and_cancel_independently() {
    use std::sync::{
      atomic::{AtomicUsize, Ordering},
      Arc,
    };
    let host = uuid::Uuid::new_v4().to_string();
    let calls = Arc::new(AtomicUsize::new(0));
    let choose = |state: Arc<Mutex<RowAttempts>>| {
      let calls = calls.clone();
      async move {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        state
          .lock()
          .unwrap()
          .sources
          .insert("chrome:fixture".into());
        Ok("chrome:fixture".to_owned())
      }
    };
    let first = shared_attempt(&host, choose);
    let second = shared_attempt(&host, choose);
    let (a, b) = tokio::join!(first.result.clone(), second.result.clone());
    assert_eq!(a, b);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let retry = shared_attempt(&host, |_| async { Err("browserLoginExhausted") });
    assert_eq!(retry.result.clone().await, Err("browserLoginExhausted"));
    drop(retry);
    assert!(second
      .state
      .lock()
      .unwrap()
      .sources
      .contains("chrome:fixture"));
    drop(first);
    assert_eq!(second.result.clone().await, Ok("chrome:fixture".into()));
    drop(second);
    let next = shared_attempt(&host, |_| async { Err("browserLoginRemedy") });
    assert_eq!(next.result.clone().await, Err("browserLoginRemedy"));
  }

  #[tokio::test(start_paused = true)]
  async fn stalled_login_is_bounded_and_shared_with_waiters() {
    let host = uuid::Uuid::new_v4().to_string();
    let first = shared_attempt(&host, |_| {
      choose_phases(std::future::pending(), || async {
        panic!("a browser timeout must not open a window")
      })
    });
    let second = shared_attempt(&host, |_| async { Ok("must-not-run".into()) });
    drop(first); // A cancelled leader must leave the attempt alive for its follower.
    let result =
      tokio::time::timeout(std::time::Duration::from_secs(31), second.result.clone()).await;
    assert_eq!(result, Ok(Err("browserLoginTimeout")));
  }

  #[tokio::test(start_paused = true)]
  async fn interactive_sign_in_outlives_browser_phase_timeout() {
    let host = uuid::Uuid::new_v4().to_string();
    let attempt = shared_attempt(&host, |state| {
      choose_phases(async { Ok(None) }, || async move {
        state.lock().unwrap().opened = true;
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        Ok(NEZZEL_SESSION.into())
      })
    });
    assert_eq!(attempt.result.clone().await, Ok(NEZZEL_SESSION.into()));
  }

  #[tokio::test(start_paused = true)]
  async fn browser_phase_timeout_does_not_consume_window_budget() {
    let host = uuid::Uuid::new_v4().to_string();
    let attempt = shared_attempt(&host, |_| {
      choose_phases(std::future::pending(), || async {
        panic!("a browser timeout must not open a window")
      })
    });
    assert_eq!(attempt.result.clone().await, Err("browserLoginTimeout"));
    let mut row = fresh_attempt();
    row.merge(&attempt.state.lock().unwrap());
    assert!(row.may_schedule());
    assert!(row.may_open_sign_in());
    assert!(!row.running);
  }

  #[tokio::test]
  async fn rejected_proven_source_reaches_remedy_instead_of_requeueing_the_same_row() {
    let host = uuid::Uuid::new_v4().to_string();
    let mut row = RowAttempts::new();
    let mut queued = Vec::new();
    let mut remedies = Vec::new();
    for _ in 0..2 {
      // The queued download reports signInRequired despite cookies being present.
      row.session_expired();
      assert_eq!(row.begin(), Ok(()));
      let attempt = shared_attempt(&host, |state| async move {
        let mut state = state.lock().unwrap();
        assert!(state.claim("chrome:rejected"));
        state.browser_proven = true;
        Ok("chrome:rejected".into())
      });
      let result = attempt.result.clone().await;
      let shared = attempt.state.lock().unwrap();
      // A different waiting row must still be allowed to use this result once.
      let mut fresh_waiter = RowAttempts::new();
      assert_eq!(
        fresh_waiter.complete(&shared, result.clone()),
        Ok("chrome:rejected".into())
      );
      match row.complete(&shared, result) {
        Ok(source) => queued.push(source),
        Err(reason) => remedies.push(reason),
      }
    }
    assert_eq!(queued, ["chrome:rejected"]);
    assert_eq!(remedies, ["browserLoginExhausted"]);
  }

  #[test]
  fn automatic_attempts_are_capped_even_when_each_download_uses_a_new_source() {
    let mut row = RowAttempts::new();
    for source in ["chrome:one", "firefox:two", "edge:three"] {
      row.session_expired();
      assert_eq!(row.begin(), Ok(()));
      // A duplicate schedule while work is pending must not consume another attempt.
      assert_eq!(row.begin(), Err(RetryAction::Pending));
      let mut shared = RowAttempts::new();
      shared.claim(source);
      shared.browser_proven = true;
      assert_eq!(row.complete(&shared, Ok(source.into())), Ok(source.into()));
    }
    row.session_expired();
    assert_eq!(row.begin(), Err(RetryAction::Stop));
    assert!(!row.running);
    assert_eq!(RowAttempts::new().begin(), Ok(()));
  }

  #[tokio::test]
  async fn shared_attempt_does_not_inherit_first_rows_exhausted_state() {
    let host = uuid::Uuid::new_v4().to_string();
    let mut exhausted = fresh_attempt();
    exhausted.opened = true;
    exhausted.sources.insert("chrome:stale".into());
    let first = shared_attempt(&host, |state| async move {
      let mut state = state.lock().unwrap();
      if !state.may_open_sign_in() || !state.claim("chrome:stale") {
        return Err("browserLoginExhausted");
      }
      state.opened = true;
      Ok(NEZZEL_SESSION.into())
    });
    let second = shared_attempt(&host, |_| async {
      panic!("must join the in-flight attempt")
    });
    assert_eq!(second.result.clone().await, Ok(NEZZEL_SESSION.into()));
    assert_eq!(first.result.clone().await, Ok(NEZZEL_SESSION.into()));
    let shared = first.state.lock().unwrap();
    exhausted.merge(&shared);
    let mut other_row = fresh_attempt();
    other_row.merge(&shared);
    assert!(exhausted.opened && other_row.opened);
    assert!(other_row.sources.contains("chrome:stale"));
  }

  #[test]
  fn proved_browser_exhaustion_never_opens_sign_in() {
    let mut row = RowAttempts {
      sources: HashSet::new(),
      attempts: 0,
      opened: false,
      browser_proven: false,
      running: false,
      created: Instant::now(),
    };
    // An unreadable/empty candidate was claimed, but did not prove a session.
    row.sources.insert("firefox:empty".into());
    assert!(row.may_open_sign_in());
    row.sources.insert("chrome:Default".into());
    row.browser_proven = true;
    assert!(!row.may_open_sign_in());
    row.browser_proven = false;
    row.opened = true;
    assert!(!row.may_open_sign_in());
  }
  #[test]
  fn second_proven_browser_can_be_tried_without_opening_sign_in() {
    let mut row = RowAttempts {
      sources: HashSet::from(["firefox:stale".into()]),
      attempts: 0,
      opened: false,
      browser_proven: true,
      running: false,
      created: Instant::now(),
    };
    assert!(row.may_schedule());
    assert!(row.sources.insert("chrome:Default".into()));
    assert!(!row.may_open_sign_in());
  }
  #[test]
  fn expired_browser_proof_allows_sign_in_once() {
    let mut row = RowAttempts {
      sources: HashSet::from(["chrome:Default".into()]),
      attempts: 0,
      opened: false,
      browser_proven: true,
      running: false,
      created: Instant::now(),
    };
    row.session_expired();
    assert!(row.may_open_sign_in());
    row.opened = true;
    row.session_expired();
    assert!(!row.may_open_sign_in());
  }
  #[test]
  fn cascade_is_firefox_chromium_safari_and_skips_windows_app_bound() {
    for windows in [true, false] {
      let mut sources = [
        BrowserKind::Safari,
        BrowserKind::Chrome,
        BrowserKind::Firefox,
      ];
      sources.sort_by_key(|kind| source_order(*kind, false, windows, false));
      assert_eq!(
        sources,
        [
          BrowserKind::Firefox,
          BrowserKind::Chrome,
          BrowserKind::Safari
        ]
      );
      assert_eq!(
        source_order(BrowserKind::Chrome, true, windows, false).is_none(),
        windows
      );
    }
  }
  #[test]
  fn working_profile_wins_within_its_family_without_reordering_families() {
    let mut profiles = [
      (BrowserKind::Firefox, false),
      (BrowserKind::Firefox, true),
      (BrowserKind::Chrome, true),
    ];
    profiles.sort_by_key(|(kind, remembered)| source_order(*kind, false, false, *remembered));
    assert_eq!(
      profiles,
      [
        (BrowserKind::Firefox, true),
        (BrowserKind::Firefox, false),
        (BrowserKind::Chrome, true)
      ]
    );
  }
  #[test]
  fn retry_sources_and_sign_in_budget_are_independent_for_each_row_in_a_group() {
    let group = uuid::Uuid::new_v4().to_string();
    let first = (group.clone(), "first".into(), "facebook.com".into());
    let second = (group.clone(), "second".into(), "facebook.com".into());
    for key in [&first, &second] {
      ATTEMPTS.lock().unwrap().insert(
        key.clone(),
        RowAttempts {
          sources: HashSet::new(),
          attempts: 0,
          opened: false,
          browser_proven: false,
          running: false,
          created: Instant::now(),
        },
      );
    }
    assert!(ATTEMPTS
      .lock()
      .unwrap()
      .get_mut(&first)
      .unwrap()
      .claim("firefox:/profile"));
    assert!(!ATTEMPTS
      .lock()
      .unwrap()
      .get_mut(&first)
      .unwrap()
      .claim("firefox:/profile"));
    assert!(ATTEMPTS
      .lock()
      .unwrap()
      .get_mut(&second)
      .unwrap()
      .claim("firefox:/profile"));
    ATTEMPTS.lock().unwrap().get_mut(&first).unwrap().opened = true;
    assert!(!ATTEMPTS.lock().unwrap()[&second].opened);
    forget_group(&group);
    assert!(!ATTEMPTS.lock().unwrap().contains_key(&first));
  }
}
