use crate::extract::{
  chain::StageResult,
  deadline::{remaining_budget, wait_until, Interrupted, FETCH_TIMEOUT},
  outcome::{Confidence, Outcome, StageId},
};
use crate::logging::LogStoreState;
use crate::models::download::FormatOptions;
use crate::models::{MediaDiagnosticPayload, MediaFatalPayload, ParsedMedia, TrackType};
use crate::parsers::ytdlp_error::{
  redact_diagnostic, CookieFetchFailure, DiagnosticMatcher, YtdlpErrorParser,
};
use crate::parsers::ytdlp_info::parse_ytdlp_info_for_url;
use crate::resolve::{self, Reason, Resolution, ResolvedInput};
use crate::runners::{
  login::{CookieRetry, LoginRequest, LoginSession},
  retry_policy::RetryAction,
  ytdlp_runner::YtdlpRunner,
};
use crate::scheduling::{fetch_pipeline::FetchEntry, group_state::subscribe_group};
use std::fmt;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

#[derive(Debug)]
pub enum YtdlpInfoFetchError {
  LoginPending,
  Unsupported(Reason),
  ReaderFailed(resolve::Failure),
  InvalidDiagnosticRules(String),
  RunnerFailed(String),
  NonZeroExit(i32),
  ParseFailed(String),
}
impl fmt::Display for YtdlpInfoFetchError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::LoginPending => write!(f, "Waiting for browser login consent"),
      Self::Unsupported(reason) => write!(f, "{}", reason.code()),
      Self::ReaderFailed(failure) => write!(f, "{}", failure.code()),
      Self::InvalidDiagnosticRules(e) => write!(f, "Invalid diagnostic rules: {e}"),
      Self::RunnerFailed(e) => write!(f, "yt-dlp invocation failed: {e}"),
      Self::NonZeroExit(code) => write!(f, "yt-dlp exited with code {code}"),
      Self::ParseFailed(e) => write!(f, "Failed to parse yt-dlp output: {e}"),
    }
  }
}
impl std::error::Error for YtdlpInfoFetchError {}

pub async fn run_ytdlp_info_fetch(app: &AppHandle, entry: &FetchEntry) -> StageResult {
  let login = LoginSession::new(
    app,
    &entry.url,
    entry.overrides.as_ref(),
    entry.login_browser.clone(),
    entry.login_was_accepted,
  );
  run_ytdlp_info_with_login(app, entry, login).await
}

pub(crate) async fn run_ytdlp_info_with_login(
  app: &AppHandle,
  entry: &FetchEntry,
  login: LoginSession,
) -> StageResult {
  run_with_resolution(app, entry, login, None).await
}

pub(crate) async fn run_ytdlp_info_resolved(
  app: &AppHandle,
  entry: &FetchEntry,
  login: Option<LoginSession>,
  resolution: Resolution,
) -> StageResult {
  let login = login.unwrap_or_else(|| {
    LoginSession::new(
      app,
      &entry.url,
      entry.overrides.as_ref(),
      entry.login_browser.clone(),
      entry.login_was_accepted,
    )
  });
  run_with_resolution(app, entry, login, Some(resolution)).await
}

async fn run_with_resolution(
  app: &AppHandle,
  entry: &FetchEntry,
  mut login: LoginSession,
  resolution: Option<Resolution>,
) -> StageResult {
  let start = Instant::now();
  if resolve::private_telegram_message(&entry.url) {
    // The public widget cannot read a private channel; do not fetch a guessed public URL.
    let mut stage = StageResult::new(StageId::Ytdlp, Confidence::High);
    stage.record_code("signInRequired", "", false, start);
    stage.terminal = true;
    return stage;
  }
  let deadline = tokio::time::Instant::now() + remaining_budget(Duration::from_secs(140));
  let mut cancel = subscribe_group(&entry.group_id);
  // Both session and evidence must survive a deadline during a later retry.
  let mut stage = StageResult::new(StageId::Ytdlp, Confidence::Medium);
  let outcome = match wait_until(
    run_attempts(app, entry, &mut login, resolution, &mut stage),
    deadline,
    &mut cancel,
  )
  .await
  {
    Ok(outcome) => outcome,
    Err(Interrupted::Cancelled) => Some(Outcome::Cancelled),
    Err(Interrupted::Timeout) => {
      stage.record_timeout(login.has_session, start);
      None
    }
  };
  if let Some(outcome) = outcome {
    stage = stage.finish(outcome, login.has_session, start);
  }
  stage.login = Some(login);
  stage
}

async fn run_attempts(
  app: &AppHandle,
  entry: &FetchEntry,
  login: &mut LoginSession,
  resolution: Option<Resolution>,
  stage: &mut StageResult,
) -> Option<Outcome> {
  let start = Instant::now();
  let matcher = match DiagnosticMatcher::shared() {
    Ok(matcher) => matcher,
    Err(error) => {
      stage.record_code("unknown", "", login.has_session, start);
      stage.error = Some(YtdlpInfoFetchError::InvalidDiagnosticRules(error));
      return None;
    }
  };
  let parser =
    YtdlpErrorParser::new(&entry.id, &entry.group_id, matcher).with_source_url(&entry.url);
  let mut cancel = subscribe_group(&entry.group_id);
  if !*cancel.borrow() {
    return Some(Outcome::Cancelled);
  }
  let child = entry.is_playlist_child();
  let _slot = match super::reliability::playlist_slot(&entry.url, child, &mut cancel).await {
    Ok(slot) => slot,
    Err(()) => return Some(Outcome::Cancelled),
  };
  let mut refetch = super::retry_policy::RetryState::default();
  let mut input = ResolvedInput::new(&entry.url);
  let resolution = match resolution {
    Some(resolution) => resolution,
    None => tokio::time::timeout(
      remaining_budget(Duration::from_secs(20)),
      resolve::resolve(&entry.url),
    )
    .await
    .unwrap_or_else(|_| {
      tracing::warn!(
        stage = "resolve",
        diagnostic_code = FETCH_TIMEOUT,
        "info fetch resolution timed out"
      );
      resolve::resolution_timeout(&entry.url)
    }),
  };
  if !*cancel.borrow() {
    return Some(Outcome::Cancelled);
  }
  // Preserve public successes; only the login verdict invokes the consent-gated export.
  let resolution = if login.uses_cookies()
    && matches!(
      resolution,
      Resolution::Unsupported {
        reason: Reason::Login
      }
    )
    && crate::resolve::readers(&entry.url).contains(&crate::routing::ReaderId::Threads)
  {
    match login
      .threads_resolution(app, &entry.url, cancel.clone())
      .await
    {
      Ok(result) => result,
      Err(code) => {
        stage.record_code(code, "", false, start);
        return None;
      }
    }
  } else {
    resolution
  };
  if reader_handoff(&entry.url, &resolution, stage, login.has_session, start) {
    return None;
  }
  let mut post_metadata = None;
  match resolution {
    Resolution::Passthrough(url) => input.rewrite(url),
    Resolution::PostMedia {
      url,
      title,
      thumbnail,
      uploader,
    } => {
      post_metadata = Some((title, thumbnail, uploader));
      input.rewrite(url);
    }
    Resolution::Attachments { metadata } => {
      let mut media = match serde_json::from_str::<crate::models::ParsedSingleVideo>(&metadata) {
        Ok(media) => media,
        Err(error) => {
          stage.record_code("unableToExtractIdOrInfo", "", login.has_session, start);
          stage.error = Some(YtdlpInfoFetchError::ParseFailed(error.to_string()));
          return None;
        }
      };
      media.id.clone_from(&entry.id);
      media.fetch_engine = Some(crate::models::parsed::FetchEngine::Reader);
      stage.media = Some(ParsedMedia::Single(media));
      if login.has_session {
        login.succeeded(app, &entry.group_id);
      }
      return Some(Outcome::Verified);
    }
    Resolution::Failed { failure } => {
      stage.error = Some(YtdlpInfoFetchError::ReaderFailed(failure));
      stage.record_code(failure.code(), "", login.has_session, start);
      return None;
    }
    Resolution::Unsupported { reason } => {
      stage.error = Some(YtdlpInfoFetchError::Unsupported(reason));
      stage.record_code(reason.code(), "", login.has_session, start);
      return None;
    }
  }
  loop {
    stage.resolved_url = Some(input.url.clone());
    if !*cancel.borrow() {
      return Some(Outcome::Cancelled);
    }
    let attempt_start = Instant::now();
    login.announce(app, &entry.id, &entry.group_id);
    let format = entry.format.clone().unwrap_or(FormatOptions {
      track_type: TrackType::Both,
      abr: None,
      height: None,
      fps: None,
      audio_encoding: None,
      video_encoding: None,
      audio_track: None,
      video_track: None,
    });
    let runner = YtdlpRunner::new(app)
      .with_format_args(&format, entry.overrides.as_ref())
      .with_input_args(entry.overrides.as_ref())
      .with_input_filter_args(entry.overrides.as_ref())
      .with_auth_args(
        entry.overrides.as_ref(),
        login.browser.as_deref(),
        &entry.url,
      )
      .with_network_args(entry.overrides.as_ref())
      .with_args([
        "-j",
        "--flat-playlist",
        "--playlist-end",
        "100",
        "--lazy-playlist",
      ])
      .with_args(super::reliability::playlist_args(child))
      .with_args(resolve::facebook_photo_share(&entry.url).then_some("--ignore-no-formats-error"))
      .with_resolved_input(&input);
    login.has_session = false;
    let timeout = runner.fetch_info_timeout(&input.url);
    let output = match runner.output_with_timeout(&entry.group_id, timeout).await {
      Ok(Some(output)) => output,
      Ok(None) => return Some(Outcome::Cancelled),
      Err(error) if error == FETCH_TIMEOUT => {
        stage.record_timeout(login.has_session, attempt_start);
        return None;
      }
      Err(error) => {
        if super::browser_cookies::read_error_code(&error).is_some()
          && super::login::automatic::enabled(app)
          && matches!(
            login.failed(
              app,
              LoginRequest::Fetch(entry.clone()),
              Some("signInRequired"),
              false,
              !*cancel.borrow()
            ),
            RetryAction::Pending
          )
        {
          return Some(Outcome::LoginPending);
        }
        let code = super::browser_cookies::read_error_code(&error).unwrap_or("unknown");
        stage.error = Some(YtdlpInfoFetchError::RunnerFailed(error));
        stage.record_code(code, "", login.has_session, attempt_start);
        return None;
      }
    };
    if !*cancel.borrow() {
      return Some(Outcome::Cancelled);
    }
    login.has_session = output.has_session;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let redacted_stderr = redact_diagnostic(&stderr);
    if serde_json::from_slice::<serde_json::Value>(&output.stdout)
      .ok()
      .is_some_and(|value| value["_nezzel_listing_partial"] == true)
    {
      if let Some(mut event) = parser.parse_line("WARNING: NEZZEL_LISTING_PARTIAL") {
        event.code = "listingPartial".into();
        crate::events::emit(
          app,
          crate::events::MEDIA_DIAGNOSTIC,
          crate::models::MediaDiagnosticPayload::from_diagnostic_event(event),
        );
      }
    }
    app.state::<LogStoreState>().write().append_lines(
      app,
      &entry.group_id,
      redacted_stderr.lines(),
    );
    if output.code != 0 {
      // The initial 401 is already recorded. An unsupported curl_cffi target adds no evidence.
      if input.impersonation_unavailable(&stderr) {
        return None;
      }
      if let Some(wrapper) = resolve::reddit_media_redirect(&entry.url, &stderr) {
        if let Resolution::Attachments { metadata } = resolve::resolve(&wrapper).await {
          if let Ok(mut media) = serde_json::from_str::<crate::models::ParsedSingleVideo>(&metadata)
          {
            media.id.clone_from(&entry.id);
            media.fetch_engine = Some(crate::models::parsed::FetchEngine::Reader);
            stage.media = Some(ParsedMedia::Single(media));
            return Some(Outcome::Verified);
          }
        }
      }
      let reason = stderr
        .lines()
        .find_map(|line| parser.login_reason_with_cookies(line, login.has_session));
      if super::facebook_retry::retry(
        app,
        &entry.group_id,
        &mut input,
        login.has_session,
        reason.as_deref(),
        &stderr,
      )
      .await
      {
        continue;
      }
      let cookie_failure = parser.cookie_fetch_failure(&stderr, login.has_session);
      let diagnostics = parser.parse_lines(stderr.lines().collect(), login.has_session);
      let first_error = diagnostics
        .iter()
        .find(|event| matches!(event.level, crate::models::error::DiagnosticLevel::Error));
      stage.record_diagnostics(&diagnostics, login.has_session, attempt_start);
      for event in diagnostics
        .iter()
        .filter(|event| matches!(event.level, crate::models::error::DiagnosticLevel::Warning))
      {
        crate::events::emit(
          app,
          crate::events::MEDIA_DIAGNOSTIC,
          MediaDiagnosticPayload::from_diagnostic_event(event.clone()),
        );
      }
      login.record_failure(&diagnostics, false);
      log_info_failure(first_error, login.has_session, cookie_failure, output.code);
      if !*cancel.borrow() {
        return Some(Outcome::Cancelled);
      }
      match login.retry_cookie_fetch(cookie_failure, &mut cancel).await {
        CookieRetry::Cancelled => return Some(Outcome::Cancelled),
        CookieRetry::Retry => continue,
        CookieRetry::GiveUp => {}
      }
      if input.retry_vimeo_401(&stderr)
        || input.retry_rumble_403(&stderr)
        || input.retry_original(&stderr)
      {
        continue;
      }
      if diagnostics.iter().any(|event| {
        refetch.refetch(
          &entry.url,
          &event.code,
          &event.message,
          false,
          login.uses_cookies(),
        ) == RetryAction::Refetch
      }) {
        let delay = super::reliability::refetch_delay(&entry.url);
        if wait_until(
          tokio::time::sleep(delay),
          tokio::time::Instant::now() + delay + Duration::from_secs(1),
          &mut cancel,
        )
        .await
        .is_err()
        {
          return Some(Outcome::Cancelled);
        }
        continue;
      }
      let reason = if login.has_session {
        (cookie_failure == CookieFetchFailure::Refused).then(|| "signInRequired".to_owned())
      } else {
        stderr.lines().find_map(|line| parser.login_reason(line))
      };
      if login.fetch_managed()
        && login.has_session
        && reason.is_some()
        && super::login::automatic::enabled(app)
        && matches!(
          login.failed(
            app,
            LoginRequest::Fetch(entry.clone()),
            reason.as_deref(),
            false,
            !*cancel.borrow()
          ),
          RetryAction::Pending
        )
      {
        return Some(Outcome::LoginPending);
      }
      if login.fetch_managed()
        || (!login.uses_cookies() && !crate::extract::registry::exclusive(&input.url))
      {
        return None;
      }
      match login.failed(
        app,
        LoginRequest::Fetch(entry.clone()),
        reason.as_deref(),
        false,
        !*cancel.borrow(),
      ) {
        RetryAction::Browser | RetryAction::Anonymous => continue,
        RetryAction::AskLogin | RetryAction::Pending => return Some(Outcome::LoginPending),
        _ => {}
      }
      return None;
    }
    if serde_json::from_str::<serde_json::Value>(&stdout).is_ok_and(|info| {
      facebook_empty_formats(&entry.url, &info, stage, login.has_session, attempt_start)
    }) {
      return None;
    }
    if serde_json::from_str::<serde_json::Value>(&stdout)
      .is_ok_and(|info| crate::parsers::gallery::tiktok_photo_candidate(&entry.url, &info))
    {
      let photo_start = Instant::now();
      let gallery = finish_photo_fetch(
        crate::runners::gallery::photo_info(app, entry, login).await,
        &entry.group_id,
        &entry.id,
        login.has_session,
        photo_start,
      );
      if gallery.media.is_some() {
        login.succeeded(app, &entry.group_id);
      }
      stage.media = gallery.media;
      stage.attempts.extend(gallery.attempts);
      // This is a photo post: an absent gallery result must never publish its soundtrack.
      return None;
    }
    if serde_json::from_str::<serde_json::Value>(&stdout)
      .is_ok_and(|info| crate::files::runner::generic_file_candidate(&input.url, &info))
    {
      // Generic's unknown_video is a direct response, not something ffmpeg can download.
      let mut target = entry.clone();
      target.url.clone_from(&input.url);
      let files = crate::files::runner::after_unsupported(app, &target).await;
      stage.media = files.media;
      stage.attempts.extend(files.attempts);
      return None;
    }
    return match parse_ytdlp_info_for_url(&stdout, entry.id.clone(), Some(&entry.url)) {
      Ok(mut media) => {
        // Post items and their thumbnails come from page JSON: same public-address gate as the readers.
        if let ParsedMedia::Single(video) = &media {
          if !video.attachments.is_empty()
            && !crate::resolve::admit_post_attachments(&video.attachments).await
          {
            stage.error = Some(YtdlpInfoFetchError::ParseFailed(
              "post attachment refused".into(),
            ));
            stage.record_code(
              "unableToExtractIdOrInfo",
              "",
              login.has_session,
              attempt_start,
            );
            return None;
          }
        }
        if let (Some((title, thumbnail, uploader)), ParsedMedia::Single(video)) =
          (&post_metadata, &mut media)
        {
          video.title = title.clone().or_else(|| video.title.take());
          video.thumbnail = thumbnail.clone().or_else(|| video.thumbnail.take());
          video.uploader = uploader.clone().or_else(|| video.uploader.take());
          if let Some(title) = title {
            let count = video.attachments.len();
            for (index, attachment) in video.attachments.iter_mut().enumerate() {
              attachment.alt = Some(format!("{title} ({}/{count})", index + 1));
            }
          }
        }
        crate::diagnostics::send::network_response_received();
        input.retain_fetch_target(&mut media);
        login.succeeded(app, &entry.group_id);
        stage.media = Some(media);
        Some(Outcome::Verified)
      }
      Err(error) => {
        let code = match error.as_str() {
          "playlistEmpty" => "playlistEmpty",
          "profileNotSupported" => "profileNotSupported",
          "signInRequired" => "signInRequired",
          _ => "unableToExtractIdOrInfo",
        };
        stage.error = Some(YtdlpInfoFetchError::ParseFailed(error.to_string()));
        stage.record_code(code, "", login.has_session, attempt_start);
        if code == "signInRequired" && !login.fetch_managed() {
          match login.failed(
            app,
            LoginRequest::Fetch(entry.clone()),
            Some(code),
            false,
            !*cancel.borrow(),
          ) {
            RetryAction::Browser | RetryAction::Anonymous => continue,
            RetryAction::AskLogin | RetryAction::Pending => return Some(Outcome::LoginPending),
            _ => {}
          }
        }
        None
      }
    };
  }
}

fn reader_handoff(
  original: &str,
  resolution: &Resolution,
  stage: &mut StageResult,
  has_session: bool,
  start: Instant,
) -> bool {
  match resolution {
    Resolution::Passthrough(url)
      if resolve::facebook_photo_target(url) || resolve::tiktok_photo_target(url) =>
    {
      stage.resolved_url = Some(url.clone());
      stage.record_code("photosOnly", "", has_session, start);
      true
    }
    Resolution::Failed { failure } => {
      stage.terminal = true;
      stage.error = Some(YtdlpInfoFetchError::ReaderFailed(*failure));
      stage.record_code(failure.code(), "", false, start);
      if let Some(attempt) = stage.attempts.last_mut() {
        if let Some(reader) = resolve::readers(original).first() {
          attempt.stage = StageId::Reader(*reader);
        }
      }
      true
    }
    Resolution::Unsupported { reason } => {
      stage.terminal = !matches!(reason, Reason::PhotosOnly | Reason::Login);
      stage.error = Some(YtdlpInfoFetchError::Unsupported(*reason));
      stage.record_code(reason.code(), "", has_session, start);
      if matches!(reason, Reason::NoMedia) {
        if let (Some(attempt), Some(reader)) = (
          stage.attempts.last_mut(),
          resolve::readers(original).first(),
        ) {
          attempt.stage = StageId::Reader(*reader);
        }
      }
      true
    }
    _ => false,
  }
}

fn log_info_failure(
  first_error: Option<&crate::models::error::DiagnosticEvent>,
  has_cookies: bool,
  cookie_failure: CookieFetchFailure,
  exit_code: i32,
) {
  let extractor = first_error
    .and_then(|event| event.component.as_deref())
    .filter(|tag| {
      !tag.is_empty()
        && tag.len() <= 64
        && tag
          .bytes()
          .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
    })
    .unwrap_or("unknown");
  tracing::warn!(
    diagnostic_code = first_error.map(|event| event.code.as_str()).unwrap_or("unknown"),
    extractor,
    has_cookies,
    cookie_failure = ?cookie_failure,
    exit_code,
    "yt-dlp info fetch failed"
  );
}

fn finish_photo_fetch(
  result: Result<Option<ParsedMedia>, crate::parsers::gallery::GalleryError>,
  group_id: &str,
  id: &str,
  has_cookies: bool,
  start: Instant,
) -> StageResult {
  use crate::parsers::gallery::GalleryError;
  let mut gallery = StageResult::new(StageId::Gallery, Confidence::Medium);
  let error = match result {
    Ok(Some(media)) => {
      gallery.media = Some(media);
      return gallery.finish(Outcome::Verified, has_cookies, start);
    }
    Ok(None) | Err(GalleryError::Unsupported) => GalleryError::Refused("photosOnly"),
    Err(GalleryError::Cancelled) => return gallery.finish(Outcome::Cancelled, has_cookies, start),
    Err(GalleryError::Refused(FETCH_TIMEOUT)) => {
      gallery.record_timeout(has_cookies, start);
      return gallery;
    }
    Err(error) => error,
  };
  gallery.record_diagnostics(
    &[crate::runners::gallery::error_diagnostic(
      group_id,
      id,
      &error,
      has_cookies,
    )],
    has_cookies,
    start,
  );
  gallery
}

pub(super) fn emit_unsupported(app: &AppHandle, group_id: &str, id: &str, reason: Reason) {
  use crate::models::error::DiagnosticLevel;
  let message = app
    .state::<crate::i18n::I18nManager>()
    .t(&format!("resolve.{}", reason.code()));
  let timestamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis())
    .unwrap_or(0);
  crate::extract::terminal::emit_terminal(
    app,
    MediaDiagnosticPayload {
      group_id: group_id.into(),
      id: id.into(),
      level: DiagnosticLevel::Error,
      code: reason.code().into(),
      component: Some("resolve".into()),
      message: message.clone(),
      raw: String::new(),
      timestamp,
    },
    MediaFatalPayload::with_exit(group_id.into(), id.into(), 1, message),
  );
}

fn facebook_empty_formats(
  source: &str,
  info: &serde_json::Value,
  stage: &mut StageResult,
  has_session: bool,
  start: Instant,
) -> bool {
  if !resolve::facebook_share(source)
    || !info
      .get("formats")
      .and_then(|formats| formats.as_array())
      .is_some_and(Vec::is_empty)
  {
    return false;
  }
  stage.resolved_url = resolve::facebook_photo_redirect(source, info);
  // Match the ordinary "No video formats found" path; never publish an empty row.
  stage.record_code("photosOnly", "", has_session, start);
  true
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::parsers::gallery::GalleryError;

  #[test]
  fn facebook_empty_video_targets_do_not_publish_downloadless_rows() {
    for path in ["page/posts/pfbidABC", "reel/123", "page/videos/123"] {
      let target = format!("https://www.facebook.com/{path}");
      let info = serde_json::json!({"webpage_url":target, "formats":[]});
      let mut stage = StageResult::new(StageId::Ytdlp, Confidence::Medium);
      assert!(facebook_empty_formats(
        "https://www.facebook.com/share/p/abc/",
        &info,
        &mut stage,
        true,
        Instant::now()
      ));
      assert!(stage.media.is_none());
      assert_eq!(stage.attempts[0].code.unwrap().as_str(), "photosOnly");
      assert_eq!(
        stage.resolved_url.as_deref(),
        path.contains("/posts/").then_some(target.as_str())
      );
    }
  }

  #[test]
  fn reader_verdicts_handoff_photos_and_stop_verified_empty_posts() {
    for (resolution, code, terminal, target) in [
      (
        Resolution::Passthrough("https://www.facebook.com/photo/?fbid=555".into()),
        "photosOnly",
        false,
        Some("https://www.facebook.com/photo/?fbid=555"),
      ),
      (
        Resolution::Passthrough("https://www.tiktok.com/@nasa/photo/123".into()),
        "photosOnly",
        false,
        Some("https://www.tiktok.com/@nasa/photo/123"),
      ),
      (
        Resolution::Unsupported {
          reason: Reason::PhotosOnly,
        },
        "photosOnly",
        false,
        None,
      ),
      (
        Resolution::Unsupported {
          reason: Reason::NoMedia,
        },
        "noMediaFound",
        true,
        None,
      ),
      (
        Resolution::Unsupported {
          reason: Reason::Removed,
        },
        "notFound404",
        true,
        None,
      ),
      (
        Resolution::Unsupported {
          reason: Reason::Login,
        },
        "signInRequired",
        false,
        None,
      ),
      (
        Resolution::Unsupported {
          reason: Reason::Unavailable,
        },
        "mediaUnavailable",
        true,
        None,
      ),
      (
        Resolution::Unsupported {
          reason: Reason::SiteNotSupported,
        },
        "urlNotSupported",
        true,
        None,
      ),
      (
        Resolution::Failed {
          failure: resolve::Failure::Network,
        },
        "networkNameResolutionFailed",
        true,
        None,
      ),
      (
        Resolution::Failed {
          failure: resolve::Failure::Timeout,
        },
        "networkTimeout",
        true,
        None,
      ),
    ] {
      let mut stage = StageResult::new(StageId::Ytdlp, Confidence::High);
      assert!(reader_handoff(
        "https://www.reddit.com/comments/abc123/",
        &resolution,
        &mut stage,
        false,
        Instant::now()
      ));
      if code == "noMediaFound" {
        assert_eq!(
          stage.attempts[0].stage,
          StageId::Reader(crate::routing::ReaderId::Reddit)
        );
      }
      if code == "noMediaFound" {
        for (url, reader) in [
          ("https://redd.it/abc123", crate::routing::ReaderId::Reddit),
          (
            "https://www.threads.com/share/FJ_5OYZ6-",
            crate::routing::ReaderId::Threads,
          ),
        ] {
          let mut attributed = StageResult::new(StageId::Ytdlp, Confidence::High);
          assert!(reader_handoff(
            url,
            &resolution,
            &mut attributed,
            false,
            Instant::now()
          ));
          assert!(attributed.terminal);
          assert_eq!(attributed.attempts[0].stage, StageId::Reader(reader));
          assert_eq!(
            crate::extract::precedence::final_code(&attributed.attempts),
            Some("noMediaFound")
          );
        }
      }
      assert_eq!(stage.terminal, terminal);
      assert_eq!(stage.resolved_url.as_deref(), target);
      assert_eq!(
        crate::extract::precedence::final_code(&stage.attempts),
        Some(code)
      );
    }
  }

  #[test]
  fn info_failure_trace_omits_titles_channel_names_and_raw_engine_output() {
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
      fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
      }
      fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
      }
    }
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = bytes.clone();
    let subscriber = tracing_subscriber::fmt()
      .without_time()
      .with_ansi(false)
      .with_writer(move || Capture(sink.clone()))
      .finish();
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    let events = parser.parse_lines(
      vec![
        "ERROR: [Instagram] private video 'private-title' by private-channel (@private-user)",
        "ERROR: [https://private-host/private-channel] private video 'private-title'",
      ],
      false,
    );
    tracing::subscriber::with_default(subscriber, || {
      for event in &events {
        log_info_failure(Some(event), false, CookieFetchFailure::Refused, 1);
      }
    });
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(log.contains("diagnostic_code=\"signInRequired\""), "{log}");
    assert!(log.contains("extractor=\"Instagram\""), "{log}");
    for secret in [
      "private-title",
      "private-channel",
      "private-user",
      "private-host",
      "private video",
      "ERROR:",
    ] {
      assert!(!log.contains(secret), "{log}");
    }
  }

  #[test]
  fn tiktok_photo_failures_keep_photo_diagnostics_without_publishing_audio() {
    for (result, expected) in [
      (Ok(None), "photosOnly"),
      (Err(GalleryError::Unsupported), "photosOnly"),
      (Err(GalleryError::Unavailable), "imageEngineUnavailable"),
      (Err(GalleryError::Refused(FETCH_TIMEOUT)), "fetchTimeout"),
    ] {
      let stage = finish_photo_fetch(result, "group", "item", false, Instant::now());
      assert!(stage.media.is_none());
      assert_eq!(
        crate::extract::precedence::final_code(&stage.attempts),
        Some(expected)
      );
      assert!(!stage
        .attempts
        .iter()
        .any(|attempt| attempt.outcome == Outcome::Verified));
    }
  }
}
