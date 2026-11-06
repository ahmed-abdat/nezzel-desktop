//! Info and preview stages built from a probed gallery post.
use super::runner::{bundled_path, GalleryRunner};
use crate::extract::{
  chain::StageResult,
  deadline::{remaining_budget, wait_until, Interrupted, FETCH_TIMEOUT},
  outcome::{Confidence, Outcome, StageId},
};
use crate::models::error::{DiagnosticEvent, DiagnosticLevel};
use crate::models::parsed::AttachmentKind;
use crate::models::{MediaDiagnosticPayload, MediaFatalPayload, ParsedMedia};
use crate::parsers::gallery::{GalleryError, GalleryPost};
use crate::runners::{
  login::{CookieRetry, LoginRequest, LoginSession},
  retry_policy::RetryAction,
  ytdlp_runner::YtdlpRunner,
};
use crate::scheduling::{
  fetch_pipeline::FetchEntry,
  group_state::{is_group_running, subscribe_group},
};
use crate::state::config_models::NetworkSettings;
use serde_json::Value;
use std::time::Duration;
use std::time::Instant;
use tauri::AppHandle;

pub(super) fn image_post(
  result: Result<GalleryPost, GalleryError>,
) -> Result<Option<GalleryPost>, GalleryError> {
  match result {
    Ok(post) if post.has_images() => Ok(Some(post)),
    Ok(_) | Err(GalleryError::Unsupported) => Ok(None),
    Err(error) => Err(error),
  }
}

/// One-way fallback: a failed/no-photo probe never invokes yt-dlp again.
pub(crate) async fn photo_info(
  app: &AppHandle,
  entry: &FetchEntry,
  browser: Option<&str>,
) -> Result<Option<ParsedMedia>, GalleryError> {
  let runner = GalleryRunner::new(
    app,
    entry.overrides.as_ref(),
    browser,
    &entry.url,
    &entry.group_id,
  )
  .await?;
  let result = runner.probe(&entry.url, &entry.group_id).await;
  Ok(image_post(result)?.map(|post| ParsedMedia::Single(post.into_media(entry.id.clone()))))
}

pub(super) fn preview_from_post(
  result: Result<GalleryPost, GalleryError>,
  id: String,
) -> Result<Option<Value>, GalleryError> {
  let Some(post) = image_post(result)? else {
    return Ok(None);
  };
  Ok(serde_json::to_value(post.into_media(id)).ok())
}

/// Silent, anonymous metadata only: no LoginSession, diagnostics or user gallery-dl config.
pub async fn preview_media(
  app: &AppHandle,
  source: &str,
  id: String,
  network: &NetworkSettings,
) -> Result<Option<Value>, GalleryError> {
  let runner = GalleryRunner::anonymous(bundled_path(app)?, network, source)?;
  let group = format!("capture-preview-{id}");
  // Match the yt-dlp preview budget, including all gallery redirect attempts.
  let result = tokio::time::timeout(Duration::from_secs(20), runner.probe(source, &group)).await;
  crate::scheduling::group_state::remove_group(&group);
  preview_from_post(result.map_err(|_| GalleryError::Unavailable)?, id)
}

pub(crate) fn error_diagnostic(group: &str, id: &str, error: &GalleryError) -> DiagnosticEvent {
  let code = match error {
    GalleryError::Login => "signInRequired",
    GalleryError::Unavailable => "imageEngineUnavailable",
    GalleryError::Refused(FETCH_TIMEOUT) => "unknown",
    GalleryError::Refused(code) => code,
    GalleryError::Unsupported => "urlNotSupported",
    _ => "imageSourceUnavailable",
  };
  DiagnosticEvent {
    id: id.into(),
    group_id: group.into(),
    level: DiagnosticLevel::Error,
    code: code.into(),
    component: Some("gallery-dl".into()),
    message: String::new(),
    raw: String::new(),
    timestamp: std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map_or(0, |d| d.as_millis()),
  }
}

pub(crate) fn diagnostic(app: &AppHandle, group: &str, id: &str, event: DiagnosticEvent) {
  crate::extract::terminal::emit_terminal(
    app,
    MediaDiagnosticPayload::from_diagnostic_event(event),
    MediaFatalPayload::with_exit(group.into(), id.into(), 1, String::new()),
  );
}

// Compatibility entry point for link_matrix; remove after the download coordinator migrates callers.

#[cfg(feature = "link-matrix")]
pub async fn run_media_info(
  app: &AppHandle,
  entry: &FetchEntry,
) -> Result<Option<ParsedMedia>, crate::runners::ytdlp_info::YtdlpInfoFetchError> {
  crate::extract::chain::run(app, entry).await
}

pub(crate) async fn run_gallery_info(
  app: &AppHandle,
  entry: &FetchEntry,
  login: Option<LoginSession>,
  confidence: Confidence,
) -> StageResult {
  let mut login = login.unwrap_or_else(|| {
    LoginSession::new(
      app,
      &entry.url,
      entry.overrides.as_ref(),
      entry.login_browser.clone(),
      entry.login_was_accepted,
    )
  });
  let start = Instant::now();
  let mut stage = StageResult::new(StageId::Gallery, confidence);
  // Keep evidence outside the timed future: a late timeout must not erase an earlier login cause.
  // Dropping a probe kills its ChildGuard, bounding redirect hops as well as the current subprocess.
  let deadline = tokio::time::Instant::now() + remaining_budget(Duration::from_secs(45));
  let mut cancel = subscribe_group(&entry.group_id);
  let outcome = match wait_until(
    gallery_info_attempts(app, entry, &mut login, &mut stage),
    deadline,
    &mut cancel,
  )
  .await
  {
    Ok(outcome) => outcome,
    Err(Interrupted::Cancelled) => Some(Outcome::Cancelled),
    Err(Interrupted::Timeout) => {
      stage.record_timeout(login.uses_cookies(), start);
      None
    }
  };
  if let Some(outcome) = outcome {
    stage = stage.finish(outcome, login.uses_cookies(), start);
  }
  stage.login = Some(login);
  stage
}

pub(super) async fn gallery_info_attempts(
  app: &AppHandle,
  entry: &FetchEntry,
  login: &mut LoginSession,
  stage: &mut StageResult,
) -> Option<Outcome> {
  loop {
    let attempt_start = Instant::now();
    login.announce(app, &entry.id, &entry.group_id);
    let (result, cookies) = match GalleryRunner::new(
      app,
      entry.overrides.as_ref(),
      login.browser.as_deref(),
      &entry.url,
      &entry.group_id,
    )
    .await
    {
      Ok(runner) => {
        let result = runner.probe(&entry.url, &entry.group_id).await;
        (result, runner._cookies.clone())
      }
      Err(error) => (Err(error), None),
    };
    if matches!(result, Err(GalleryError::Unsupported)) {
      stage.record_code("urlNotSupported", "", login.uses_cookies(), attempt_start);
      return None;
    }
    match image_post(result) {
      Ok(None) => return Some(Outcome::NoMatch),
      Ok(Some(post)) => {
        let mixed = post
          .assets
          .iter()
          .any(|a| matches!(a.item.kind, AttachmentKind::Video));
        let video_source = post
          .extract_url
          .split("/photo/")
          .next()
          .unwrap_or(&post.extract_url)
          .to_string();
        let mut media = post.into_media(entry.id.clone());
        if mixed {
          let probe = YtdlpRunner::new(app)
            .with_auth_args(
              entry.overrides.as_ref(),
              login.browser.as_deref(),
              &entry.url,
            )
            .with_cookie_access(cookies.as_ref())
            .with_network_args(entry.overrides.as_ref())
            .with_args(["-J", "--yes-playlist", "--playlist-items", "1"])
            .with_args(["--", video_source.as_str()])
            .output(&entry.group_id)
            .await;
          if let Ok(Some(output)) = probe {
            if output.code == 0 && output.stdout.len() <= 8 * 1024 * 1024 {
              if let Ok(mut value) = serde_json::from_slice::<Value>(&output.stdout) {
                if let Some(first) = value["entries"].as_array().and_then(|v| v.first()).cloned() {
                  value = first;
                }
                if let Ok(ParsedMedia::Single(video)) =
                  crate::parsers::ytdlp_info::parse_ytdlp_info(&value.to_string(), entry.id.clone())
                {
                  media.formats = video.formats;
                  media.video_tracks = video.video_tracks;
                  media.audio_tracks = video.audio_tracks;
                  media.video_codecs = video.video_codecs;
                  media.audio_codecs = video.audio_codecs;
                  media.subtitle_inventory = video.subtitle_inventory;
                }
              }
            }
          }
        }
        login.succeeded(app, &entry.group_id);
        stage.media = Some(ParsedMedia::Single(media));
        return Some(Outcome::Verified);
      }
      Err(GalleryError::Cancelled) => return Some(Outcome::Cancelled),
      Err(GalleryError::Refused(FETCH_TIMEOUT)) => {
        stage.record_timeout(login.uses_cookies(), attempt_start);
        return None;
      }
      Err(error) => {
        let mut cancel = subscribe_group(&entry.group_id);
        if !*cancel.borrow() {
          return Some(Outcome::Cancelled);
        }
        let diagnostics = vec![error_diagnostic(&entry.group_id, &entry.id, &error)];
        if error == GalleryError::NetworkDown {
          stage.record_code(
            "networkNameResolutionFailed",
            "",
            login.uses_cookies(),
            attempt_start,
          );
        } else {
          stage.record_diagnostics(&diagnostics, login.uses_cookies(), attempt_start);
        }
        match login
          .retry_cookie_fetch(error.cookie_fetch_failure(), &mut cancel)
          .await
        {
          CookieRetry::Cancelled => return Some(Outcome::Cancelled),
          CookieRetry::Retry => continue,
          CookieRetry::GiveUp => {}
        }
        let reason = (error == GalleryError::Login
          || (!login.uses_cookies() && error == GalleryError::Refused("unableToExtractIdOrInfo")))
        .then_some("signInRequired");
        login.record_failure(&diagnostics, error == GalleryError::NetworkDown);
        if login.fetch_managed() || !login.uses_cookies() {
          return None;
        }
        match if error == GalleryError::NetworkDown {
          RetryAction::Stop
        } else {
          login.failed(
            app,
            LoginRequest::Fetch(entry.clone()),
            reason,
            false,
            !is_group_running(&entry.group_id),
          )
        } {
          RetryAction::Browser | RetryAction::Anonymous => continue,
          RetryAction::AskLogin => return Some(Outcome::LoginPending),
          _ => {}
        }
        return None;
      }
    }
  }
}
