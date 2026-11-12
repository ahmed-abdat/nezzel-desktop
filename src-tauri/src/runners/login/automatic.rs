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
  sync::{LazyLock, Mutex},
  time::Instant,
};
use tauri::{AppHandle, Manager};

struct RowAttempts {
  sources: HashSet<String>,
  opened: bool,
  running: bool,
  created: Instant,
}
type AttemptKey = (String, String, String);
static ATTEMPTS: LazyLock<Mutex<HashMap<AttemptKey, RowAttempts>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

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

pub(super) fn schedule(app: &AppHandle, request: LoginRequest) -> RetryAction {
  let Some(host) = request.host() else {
    return RetryAction::Stop;
  };
  let (id, group) = request.ids();
  let key = (group.to_owned(), id.to_owned(), host.clone());
  {
    let mut rows = ATTEMPTS.lock().unwrap();
    rows.retain(|_, row| row.created.elapsed() < LOGIN_TTL);
    if rows.get(&key).is_some_and(|row| row.running) {
      return RetryAction::Pending;
    }
    if rows.get(&key).is_some_and(|row| row.opened) || rows.len() >= 128 && !rows.contains_key(&key)
    {
      return RetryAction::Stop;
    }
    rows.entry(key.clone()).or_insert_with(|| RowAttempts {
      sources: HashSet::new(),
      opened: false,
      running: false,
      created: Instant::now(),
    });
    rows.get_mut(&key).unwrap().running = true;
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
    let result = tokio::select! {
      biased;
      _ = cancel.changed() => Err("cancelled"),
      _ = wait_disabled(&app) => Err("browserLoginDisabled"),
      result = tokio::time::timeout(LOGIN_TTL, choose_source(&app, &request, &host, &key)) => result.unwrap_or(Err("browserLoginTimeout")),
    };
    if let Some(row) = ATTEMPTS.lock().unwrap().get_mut(&key) {
      row.running = false;
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
              reason: "browserLoginRemedy".into(),
              blocked_browser: None,
              status: LoginStatus::Exhausted,
              candidates: vec![],
              change_only: false,
            },
          );
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
fn claim(key: &(String, String, String), source: &str) -> bool {
  ATTEMPTS
    .lock()
    .unwrap()
    .get_mut(key)
    .is_some_and(|row| row.sources.insert(source.into()))
}
async fn choose_source(
  app: &AppHandle,
  request: &LoginRequest,
  host: &str,
  key: &(String, String, String),
) -> Result<String, &'static str> {
  if !enabled(app) {
    return Err("browserLoginDisabled");
  }
  if claim(key, NEZZEL_SESSION)
    && super::webview_session::read(app, host, false)
      .await
      .ok()
      .flatten()
      .is_some()
  {
    return Ok(NEZZEL_SESSION.into());
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
  for candidate in candidates {
    if !enabled(app) {
      return Err("browserLoginDisabled");
    }
    if !claim(key, &candidate.id) {
      continue;
    }
    let auth = app.state::<SharedConfig>().load().auth.clone();
    let access = CookieAccess::prepare(
      &auth,
      Some(&candidate.id),
      request.url(),
      &context,
      crate::scheduling::group_state::subscribe_group(&key.0),
    )
    .await;
    if let Ok(access) = access {
      if access.has_proof(host) && enabled(app) {
        return Ok(candidate.id);
      }
    }
  }
  if !enabled(app) {
    return Err("browserLoginDisabled");
  }
  if !crate::scheduling::group_state::is_group_running(&key.0) {
    return Err("cancelled");
  }
  if let Some(row) = ATTEMPTS.lock().unwrap().get_mut(key) {
    row.opened = true;
  }
  super::webview_session::read(app, host, true)
    .await?
    .ok_or("browserLoginRemedy")?;
  Ok(NEZZEL_SESSION.into())
}

async fn wait_disabled(app: &AppHandle) {
  while enabled(app) {
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
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
          opened: false,
          running: false,
          created: Instant::now(),
        },
      );
    }
    assert!(claim(&first, "firefox:/profile"));
    assert!(!claim(&first, "firefox:/profile"));
    assert!(claim(&second, "firefox:/profile"));
    ATTEMPTS.lock().unwrap().get_mut(&first).unwrap().opened = true;
    assert!(!ATTEMPTS.lock().unwrap()[&second].opened);
    forget_group(&group);
    assert!(!ATTEMPTS.lock().unwrap().contains_key(&first));
  }
}
