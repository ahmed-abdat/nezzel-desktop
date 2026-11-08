//! Login requests waiting for the user, profile-change choices, and the Tauri commands that answer them.
use super::consent::{forget_group_rows, remember_row_browser};
use super::{
  discovered_candidates, resolve_candidate_id, LoginCandidate, LoginNeeded, LoginStatus,
  LoginUsing, APP_BOUND_REASON, CHANGE_PROFILE_REASON, DISCOVERY, LOGIN_TTL,
};
use crate::models::error::DiagnosticLevel;
use crate::models::{MediaDiagnosticPayload, MediaFatalPayload};
use crate::runners::browser_cookies::site_key;
use crate::scheduling::{
  dispatcher::DispatchRequest,
  download_pipeline::{DownloadEntry, DownloadRequest, DownloadSender},
  fetch_pipeline::{FetchEntry, FetchRequest, FetchSender},
  group_state::{is_group_running, subscribe_group},
};
use std::{
  collections::HashMap,
  sync::{LazyLock, Mutex},
  time::Instant,
};
use tauri::{AppHandle, Manager};
use tokio::sync::watch;

pub(super) struct ProfileChoice {
  pub(super) used: LoginUsing,
  pub(super) candidates: Vec<LoginCandidate>,
  pub(super) created: Instant,
}
pub(super) static USED: LazyLock<Mutex<HashMap<String, (LoginUsing, Instant)>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) static CHOICES: LazyLock<Mutex<HashMap<String, ProfileChoice>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) fn announce_using<R: tauri::Runtime>(app: &AppHandle<R>, used: LoginUsing) {
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
  crate::events::emit(app, crate::events::MEDIA_LOGIN_USING, used);
}

/// Explicit user action on a row that has already used site login. Never a background scan.
#[tauri::command]
pub async fn media_login_choices(group_id: String) -> Result<LoginNeeded, String> {
  tauri::async_runtime::spawn_blocking(move || choices_for_group(group_id))
    .await
    .map_err(|_| "Browser discovery failed".to_owned())?
}
pub(super) fn choices_for_group(group_id: String) -> Result<LoginNeeded, String> {
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
  let blocked_browser = candidates.iter().find(|c| c.app_bound).map(|c| c.browser);
  let candidates: Vec<_> = candidates.into_iter().filter(|c| !c.app_bound).collect();
  let request_id = uuid::Uuid::new_v4().to_string();
  let payload = LoginNeeded {
    request_id: request_id.clone(),
    id: used.id.clone(),
    group_id: used.group_id.clone(),
    host: used.host.clone(),
    reason: if blocked_browser.is_some() {
      APP_BOUND_REASON.into()
    } else {
      CHANGE_PROFILE_REASON.into()
    },
    blocked_browser,
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
pub(super) fn claim_choice(
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
  pub(super) fn ids(&self) -> (&str, &str) {
    match self {
      Self::Fetch(e) => (&e.id, &e.group_id),
      Self::Download(e) => (&e.id, &e.group_id),
    }
  }
  fn fatal_payload(&self, message: &str) -> MediaFatalPayload {
    let (id, group) = self.ids();
    MediaFatalPayload::internal(group.into(), id.into(), message.into(), None)
  }
  pub(super) fn timeout_payloads(&self) -> (MediaDiagnosticPayload, MediaFatalPayload) {
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
  pub(super) fn fail<R: tauri::Runtime>(&self, app: &AppHandle<R>, message: &str) {
    if message.contains("timed out") || message.contains("expired") {
      let (diagnostic, fatal) = self.timeout_payloads();
      crate::events::emit(app, crate::events::MEDIA_DIAGNOSTIC, diagnostic);
      crate::events::emit(app, crate::events::MEDIA_FATAL, fatal);
    } else {
      crate::events::emit(app, crate::events::MEDIA_FATAL, self.fatal_payload(message));
    }
    self.release(app);
  }
  pub(super) fn host(&self) -> Option<String> {
    let url = match self {
      Self::Fetch(e) => &e.url,
      Self::Download(e) => &e.url,
    };
    reqwest::Url::parse(url).ok().and_then(|u| {
      u.host_str()
        .map(|host| site_key(host).unwrap_or_else(|| host.to_owned()))
    })
  }
  pub(super) fn release<R: tauri::Runtime>(&self, app: &AppHandle<R>) {
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

  pub(super) fn consent_to_browser(&mut self, browser: &str) {
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
  pub(super) fn enqueue<R: tauri::Runtime>(
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
pub(super) struct PendingLogin {
  pub(super) followers: Vec<PendingLogin>,
  pub(super) announcement: Option<(AppHandle, LoginNeeded)>,
  pub(super) request: LoginRequest,
  pub(super) candidates: Vec<LoginCandidate>,
  pub(super) cancel: watch::Receiver<bool>,
  pub(super) created: Instant,
  pub(super) cleanup: CleanupChannels,
}
impl PendingLogin {
  pub(super) fn into_rows(mut self) -> Vec<Self> {
    let mut rows = std::mem::take(&mut self.followers);
    rows.insert(0, self);
    rows
  }
}

/// Same-host waiting rows share one offered profile list and one consent token.
pub(super) fn queue_pending(
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

pub(super) struct CleanupChannels {
  pub(super) fetch: FetchSender,
  pub(super) download: DownloadSender,
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
/// The deadline is injectable so timeout effects are tested without wall-clock sleeps.
pub(super) async fn expire_pending(
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
        crate::events::emit(&app, crate::events::MEDIA_DIAGNOSTIC, diagnostic);
        crate::events::emit(&app, crate::events::MEDIA_FATAL, fatal);
      },
      |request| request.release(&app),
    )
    .await;
  });
}

/// Parks a row until the user answers. Expired or cancelled requests are cleared first; a
/// full table drops its oldest request unless this host already waits. Returns false when
/// the row joined a same-host request that was already announced.
pub(super) fn park(
  app: &AppHandle,
  host: &str,
  request_id: String,
  payload: LoginNeeded,
  request: LoginRequest,
  candidates: Vec<LoginCandidate>,
) -> bool {
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
      .any(|p| p.request.host().as_deref() == Some(host))
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
      announcement: Some((app.clone(), payload)),
      cancel: subscribe_group(request.ids().1),
      request,
      candidates,
      created: Instant::now(),
      cleanup: CleanupChannels {
        fetch: app.state::<FetchSender>().inner().clone(),
        download: app.state::<DownloadSender>().inner().clone(),
      },
    },
  );
  if announce {
    schedule_expiry(app.clone(), request_id);
  }
  announce
}

/// The profile this group last announced, while it is still the session's browser.
pub(super) fn used_candidate(group_id: &str, browser: Option<&str>) -> Option<LoginCandidate> {
  USED
    .lock()
    .unwrap()
    .get(group_id)
    .filter(|(used, _)| browser == Some(used.candidate.id.as_str()))
    .map(|(used, _)| used.candidate.clone())
}

pub(super) static PENDING: LazyLock<Mutex<HashMap<String, PendingLogin>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) fn claim_pending(
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
#[tauri::command]
pub fn media_login_accept(
  app: AppHandle,
  request_id: String,
  browser_id: String,
) -> Result<(), String> {
  accept_login(app, request_id, browser_id)
}

pub(super) fn accept_login<R: tauri::Runtime>(
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
        crate::events::emit(&app, crate::events::MEDIA_DIAGNOSTIC, diagnostic);
        crate::events::emit(&app, crate::events::MEDIA_FATAL, fatal);
      }
      row.request.release(&app);
    }
  }
}
pub(super) fn remove_pending_group(map: &mut HashMap<String, PendingLogin>, group_id: &str) {
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
        crate::events::emit(app, crate::events::MEDIA_LOGIN_NEEDED, payload.clone());
      }
    }
    map.insert(token, next);
  }
}

pub fn forget_group(group_id: &str) {
  forget_group_rows(group_id);
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
