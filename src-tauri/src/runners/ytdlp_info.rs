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
      stage.record_code("unknown", "", login.uses_cookies(), start);
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
        "info fetch resolution timed out; using original input"
      );
      Resolution::Passthrough(entry.url.clone())
    }),
  };
  if !*cancel.borrow() {
    return Some(Outcome::Cancelled);
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
          stage.record_code("unableToExtractIdOrInfo", "", login.uses_cookies(), start);
          stage.error = Some(YtdlpInfoFetchError::ParseFailed(error.to_string()));
          return None;
        }
      };
      media.id.clone_from(&entry.id);
      media.fetch_engine = Some(crate::models::parsed::FetchEngine::Reader);
      stage.media = Some(ParsedMedia::Single(media));
      return Some(Outcome::Verified);
    }
    Resolution::Unsupported { reason } => {
      stage.error = Some(YtdlpInfoFetchError::Unsupported(reason));
      stage.record_code(reason.code(), "", login.uses_cookies(), start);
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
      .with_args(["-J", "--flat-playlist"])
      .with_resolved_input(&input);
    let timeout = runner.info_timeout();
    let output = match runner.output_with_timeout(&entry.group_id, timeout).await {
      Ok(Some(output)) => output,
      Ok(None) => return Some(Outcome::Cancelled),
      Err(error) if error == FETCH_TIMEOUT => {
        stage.record_timeout(login.uses_cookies(), attempt_start);
        return None;
      }
      Err(error) => {
        stage.error = Some(YtdlpInfoFetchError::RunnerFailed(error));
        stage.record_code("unknown", "", login.uses_cookies(), attempt_start);
        return None;
      }
    };
    if !*cancel.borrow() {
      return Some(Outcome::Cancelled);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let redacted_stderr = redact_diagnostic(&stderr);
    app.state::<LogStoreState>().write().append_lines(
      app,
      &entry.group_id,
      redacted_stderr.lines(),
    );
    if output.code != 0 {
      let cookie_failure = parser.cookie_fetch_failure(&stderr);
      let diagnostics = parser.parse_lines(stderr.lines().collect(), login.uses_cookies());
      let first_error = diagnostics
        .iter()
        .find(|event| matches!(event.level, crate::models::error::DiagnosticLevel::Error));
      stage.record_diagnostics(&diagnostics, login.uses_cookies(), attempt_start);
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
      log_info_failure(
        first_error,
        login.uses_cookies(),
        cookie_failure,
        output.code,
      );
      if !*cancel.borrow() {
        return Some(Outcome::Cancelled);
      }
      match login.retry_cookie_fetch(cookie_failure, &mut cancel).await {
        CookieRetry::Cancelled => return Some(Outcome::Cancelled),
        CookieRetry::Retry => continue,
        CookieRetry::GiveUp => {}
      }
      if input.retry_original(&stderr) {
        continue;
      }
      let reason = if login.uses_cookies() {
        (cookie_failure == CookieFetchFailure::Refused).then(|| "signInRequired".to_owned())
      } else {
        stderr.lines().find_map(|line| parser.login_reason(line))
      };
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
        RetryAction::AskLogin => return Some(Outcome::LoginPending),
        _ => {}
      }
      return None;
    }
    if serde_json::from_str::<serde_json::Value>(&stdout)
      .is_ok_and(|info| crate::parsers::gallery::tiktok_photo_candidate(&entry.url, &info))
    {
      let photo_start = Instant::now();
      let gallery = finish_photo_fetch(
        crate::runners::gallery::photo_info(app, entry, login.browser.as_deref()).await,
        &entry.group_id,
        &entry.id,
        login.uses_cookies(),
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
              login.uses_cookies(),
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
        input.retain_fetch_target(&mut media);
        login.succeeded(app, &entry.group_id);
        stage.media = Some(media);
        Some(Outcome::Verified)
      }
      Err(error) => {
        stage.error = Some(YtdlpInfoFetchError::ParseFailed(error.to_string()));
        stage.record_code(
          "unableToExtractIdOrInfo",
          "",
          login.uses_cookies(),
          attempt_start,
        );
        None
      }
    };
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
      group_id, id, &error,
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::parsers::gallery::GalleryError;

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
