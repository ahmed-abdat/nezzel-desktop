use crate::commands::DownloadRecords;
use crate::logging::LogStoreState;
use crate::models::download::DownloadSection;
use crate::models::{MediaDiagnosticPayload, MediaFatalPayload, ProgressEvent};
use crate::parsers::ytdlp_error::{
  redact_diagnostic, CookieFetchFailure, DiagnosticMatcher, YtdlpErrorParser,
};
use crate::parsers::ytdlp_progress::{progress_category_for_track_type, YtdlpProgressParser};
use crate::resolve::{self, Reason, Resolution, ResolvedInput};
use crate::runners::ytdlp_info::emit_unsupported;
use crate::runners::ytdlp_runner::{YtdlpCommandEvent, YtdlpRunner};
use crate::runners::{
  login::{LoginRequest, LoginSession},
  retry_policy::RetryAction,
  transport::{aria_denied, resolve_aria2, single_direct_http, transfer_args},
};
use crate::scheduling::download_pipeline::DownloadEntry;
use crate::scheduling::group_state::subscribe_group;
use crate::SharedConfig;
use std::fmt;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::watch;

#[derive(Debug)]
pub enum YtdlpDownloadError {
  ExpiredMedia,
  LoginPending,
  Unsupported(Reason),
  InvalidArguments(String),
  InvalidDiagnosticRules(String),
  SpawnFailed(String),
  RunnerError(String),
  NonZeroExit(i32),
  EventStreamEnded,
}

impl fmt::Display for YtdlpDownloadError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::ExpiredMedia => write!(f, "Selected media URL expired"),
      Self::LoginPending => write!(f, "Waiting for browser login consent"),
      Self::Unsupported(reason) => write!(f, "{}", reason.code()),
      Self::InvalidArguments(e) => write!(f, "Invalid yt-dlp arguments: {e}"),
      Self::InvalidDiagnosticRules(e) => write!(f, "Invalid diagnostic rules: {e}"),
      Self::SpawnFailed(e) => write!(f, "Failed to spawn yt-dlp: {e}"),
      Self::RunnerError(e) => write!(f, "yt-dlp runner error: {e}"),
      Self::NonZeroExit(code) => write!(f, "yt-dlp exited with code {code}"),
      Self::EventStreamEnded => write!(f, "yt-dlp event stream ended unexpectedly"),
    }
  }
}

impl std::error::Error for YtdlpDownloadError {}

fn selected_media_expired(entry: &DownloadEntry, input: &ResolvedInput, message: &str) -> bool {
  entry.resume_parent.is_some()
    && input.original != entry.url
    && crate::parsers::gallery::expired_media(message)
}

pub(super) fn selected_video_args(entry: &DownloadEntry) -> Vec<String> {
  if let Some(index) = entry.video_index {
    vec![
      "--yes-playlist".into(),
      "--playlist-items".into(),
      index.to_string(),
    ]
  } else if entry.resume_parent.is_some() {
    // X /video/N selects one attachment only when playlists are disabled.
    let mut args = vec!["--no-playlist".into()];
    if entry.resolved.as_ref().is_some_and(|input| {
      reqwest::Url::parse(&input.url).is_ok_and(|url| {
        url.host_str() == Some("i.imgur.com")
          && (url.path().ends_with(".mp4") || url.path().ends_with(".webm"))
      })
    }) {
      // These are already selected CDN files. The Imgur page extractor can give
      // every album child the same title, causing subsequent files to be skipped.
      args.push("--force-generic-extractor".into());
    }
    args
  } else {
    Vec::new()
  }
}

fn snapchat_media_page(source: &str) -> bool {
  reqwest::Url::parse(source).is_ok_and(|url| {
    url
      .host_str()
      .is_some_and(|host| host == "snapchat.com" || host.ends_with(".snapchat.com"))
      && url.path_segments().is_some_and(|segments| {
        segments
          .into_iter()
          .any(|part| matches!(part, "spotlight" | "highlight"))
      })
  })
}

fn normalize_snapchat_extensions(info: &mut serde_json::Value) {
  fn snapchat_cdn_url(value: &serde_json::Value) -> bool {
    value
      .as_str()
      .and_then(|raw| reqwest::Url::parse(raw).ok())
      .is_some_and(|url| {
        url
          .host_str()
          .is_some_and(|host| host == "sc-cdn.net" || host.ends_with(".sc-cdn.net"))
      })
  }
  fn known_extension(ext: &str) -> bool {
    matches!(
      ext.to_ascii_lowercase().as_str(),
      "mp4"
        | "mkv"
        | "webm"
        | "flv"
        | "mov"
        | "avi"
        | "m4v"
        | "ts"
        | "m2ts"
        | "3gp"
        | "3g2"
        | "ogv"
        | "m3u8"
        | "mpd"
        | "mp3"
        | "m4a"
        | "aac"
        | "ogg"
        | "opus"
        | "wav"
        | "flac"
        | "none"
    ) || crate::parsers::gallery::image_extension(ext)
  }
  if snapchat_cdn_url(&info["url"]) {
    for key in ["ext", "video_ext"] {
      if (key == "ext" || info.get(key).is_some())
        && !info[key].as_str().is_some_and(known_extension)
      {
        info[key] = serde_json::json!("mp4");
      }
    }
  }
  for key in [
    "formats",
    "requested_formats",
    "requested_downloads",
    "entries",
  ] {
    if let Some(items) = info.get_mut(key).and_then(serde_json::Value::as_array_mut) {
      for item in items {
        normalize_snapchat_extensions(item);
      }
    }
  }
  if let Some(thumbnails) = info
    .get_mut("thumbnails")
    .and_then(serde_json::Value::as_array_mut)
  {
    for thumbnail in thumbnails {
      if snapchat_cdn_url(&thumbnail["url"])
        && !thumbnail["ext"].as_str().is_some_and(known_extension)
      {
        let ext = thumbnail["url"]
          .as_str()
          .and_then(|url| reqwest::Url::parse(url).ok())
          .and_then(|url| {
            url
              .path()
              .rsplit_once('.')
              .map(|(_, ext)| ext.to_ascii_lowercase())
          })
          .filter(|ext| crate::parsers::gallery::image_extension(ext))
          .unwrap_or_else(|| "jpg".into());
        thumbnail["ext"] = serde_json::json!(ext);
      }
    }
  }
}

pub async fn run_ytdlp_download(
  app: AppHandle,
  entry: DownloadEntry,
) -> Result<(), YtdlpDownloadError> {
  let login = LoginSession::new(
    &app,
    &entry.url,
    entry.overrides.as_ref(),
    entry.login_browser.clone(),
    entry.login_was_accepted,
  );
  run_ytdlp_download_with_login(app, entry, login).await
}

pub(super) async fn run_ytdlp_download_with_login(
  app: AppHandle,
  entry: DownloadEntry,
  login: LoginSession,
) -> Result<(), YtdlpDownloadError> {
  run_download_attempts(app, entry, login, None).await
}

pub(super) async fn run_ytdlp_download_with_cookies(
  app: AppHandle,
  entry: DownloadEntry,
  cookies: Option<super::browser_cookies::CookieAccess>,
) -> Result<(), YtdlpDownloadError> {
  let login = LoginSession::new(
    &app,
    &entry.url,
    entry.overrides.as_ref(),
    entry.login_browser.clone(),
    entry.login_was_accepted,
  );
  run_download_attempts(app, entry, login, cookies).await
}

async fn run_download_attempts(
  app: AppHandle,
  mut entry: DownloadEntry,
  mut login: LoginSession,
  inherited_cookies: Option<super::browser_cookies::CookieAccess>,
) -> Result<(), YtdlpDownloadError> {
  let inherited_browser = login.browser.clone();
  static RULES_JSON: &str = include_str!("../diagnostic_rules.json");
  let matcher = match DiagnosticMatcher::from_json(RULES_JSON) {
    Ok(matcher) => matcher,
    Err(err) => {
      emit_internal_fatal(
        &app,
        &entry,
        format!("Invalid diagnostic rules: {err}"),
        Some(err.to_string()),
      );
      return Err(YtdlpDownloadError::InvalidDiagnosticRules(err.to_string()));
    }
  };

  let error_parser =
    YtdlpErrorParser::new(&entry.id, &entry.group_id, matcher).with_source_url(&entry.url);
  let mut cancel_rx = subscribe_group(&entry.group_id);
  if is_cancelled_now(&cancel_rx) {
    return Ok(());
  }
  let mut input = match entry.take_fetch_input() {
    Some(input) => input,
    None => {
      let resolution = resolve::resolve(&entry.url).await;
      if is_cancelled_now(&cancel_rx) {
        return Ok(());
      }
      let mut input = ResolvedInput::new(&entry.url);
      match resolution {
        Resolution::Passthrough(url) => input.rewrite(url),
        Resolution::PostMedia { url, title, .. } => {
          if let Some(title) = title {
            entry
              .template_context
              .values
              .insert("fulltitle".into(), title.clone());
            entry.template_context.values.insert("title".into(), title);
          }
          input.rewrite(url);
        }
        Resolution::Attachments { metadata } => {
          let media: crate::models::ParsedSingleVideo = serde_json::from_str(&metadata)
            .map_err(|e| YtdlpDownloadError::InvalidArguments(e.to_string()))?;
          entry.fetch_engine = Some(crate::models::parsed::FetchEngine::Reader);
          entry.media_kind = media.media_kind;
          entry.attachments = media.attachments;
          if entry.attachment_ids.is_none() {
            entry.attachment_ids = Some(
              entry
                .attachments
                .iter()
                .map(|asset| asset.id.clone())
                .collect(),
            );
          }
          return Box::pin(crate::runners::gallery::run_media_download(app, entry)).await;
        }
        Resolution::Unsupported { reason } => {
          emit_unsupported(&app, &entry.group_id, &entry.id, reason);
          return Err(YtdlpDownloadError::Unsupported(reason));
        }
      }
      input
    }
  };
  let mut generic_tried = false;
  'attempt: loop {
    if is_cancelled_now(&cancel_rx) {
      return Ok(());
    }
    login.announce(&app, &entry.id, &entry.group_id);
    // Browser-consent resumes retain both the resolved target and its original page/referer.
    entry.resolved = Some(Box::new(input.clone()));
    let cookies = if let Some(cookies) = inherited_cookies
      .as_ref()
      .filter(|_| login.browser == inherited_browser)
    {
      cookies.clone()
    } else {
      match YtdlpRunner::new(&app)
        .with_auth_args(
          entry.overrides.as_ref(),
          login.browser.as_deref(),
          &entry.url,
        )
        .prepare_cookie_access(&entry.group_id)
        .await
      {
        Ok(cookies) => cookies,
        Err(error) => {
          if is_cancelled_now(&cancel_rx) {
            return Ok(());
          }
          let _ = app.emit(
            "media_fatal",
            MediaFatalPayload::with_exit(
              entry.group_id.clone(),
              entry.id.clone(),
              1,
              "Browser cookies could not be read".into(),
            ),
          );
          return Err(YtdlpDownloadError::RunnerError(error));
        }
      }
    };
    let performance = app.state::<SharedConfig>().load().performance.clone();
    let mut aria = if performance.aria2_enabled && !login.state.native_fallback {
      resolve_aria2(&app)
    } else {
      None
    };
    if aria.is_some() {
      let probe = YtdlpRunner::new(&app)
        .with_format_args(&entry.format, entry.overrides.as_ref())
        .with_input_args(entry.overrides.as_ref())
        .with_args(selected_video_args(&entry))
        .with_auth_args(
          entry.overrides.as_ref(),
          login.browser.as_deref(),
          &entry.url,
        )
        .with_cookie_access(Some(&cookies))
        .with_network_args(entry.overrides.as_ref())
        .with_args(["-J", "--flat-playlist", "--skip-download"])
        .with_resolved_input(&input)
        .output(&entry.group_id)
        .await;
      match probe {
        Ok(None) => return Ok(()),
        Ok(Some(output)) => {
          let stderr = String::from_utf8_lossy(&output.stderr);
          for line in stderr.lines() {
            if let Some(event) = error_parser.parse_fetch_line(line, login.uses_cookies()) {
              tracing::warn!(diagnostic_code = %event.code, "yt-dlp download probe diagnostic");
            }
          }
          app.state::<LogStoreState>().write().append_lines(
            &app,
            &entry.group_id,
            redact_diagnostic(&stderr).lines(),
          );
          if output.code != 0 {
            if selected_media_expired(&entry, &input, &stderr) {
              return Err(YtdlpDownloadError::ExpiredMedia);
            }
            let failure = error_parser.cookie_fetch_failure(&stderr);
            let diagnostics =
              error_parser.parse_lines(stderr.lines().collect(), login.uses_cookies());
            login.record_failure(&diagnostics, false);
            if tokio::select! {
              biased;
              _ = cancel_rx.changed() => return Ok(()),
              retry = login.retry_cookie_fetch(failure, tokio::time::sleep) => retry,
            } {
              continue 'attempt;
            }
            if input.retry_original(&stderr) {
              continue 'attempt;
            }
            let reason = if login.uses_cookies() {
              (failure == CookieFetchFailure::Refused).then(|| "signInRequired".to_owned())
            } else {
              stderr
                .lines()
                .find_map(|line| error_parser.login_reason(line))
            };
            match login.failed(
              &app,
              LoginRequest::Download(entry.login_request_entry()),
              reason.as_deref(),
              false,
              is_cancelled_now(&cancel_rx),
            ) {
              RetryAction::Browser | RetryAction::Anonymous => continue 'attempt,
              RetryAction::AskLogin => return Err(YtdlpDownloadError::LoginPending),
              _ => {}
            }
            for event in login.failure_diagnostics(diagnostics) {
              let _ = app.emit(
                "media_diagnostic",
                MediaDiagnosticPayload::from_diagnostic_event(event),
              );
            }
            let _ = app.emit(
              "media_fatal",
              MediaFatalPayload::with_exit(
                entry.group_id.clone(),
                entry.id.clone(),
                output.code,
                redact_diagnostic(&stderr).chars().take(8192).collect(),
              ),
            );
            return Err(YtdlpDownloadError::NonZeroExit(output.code));
          }
          if output.code != 0
            || !serde_json::from_slice(&output.stdout)
              .ok()
              .is_some_and(|v| single_direct_http(&v, &input.url))
          {
            aria = None;
          }
        }
        Err(_) => aria = None,
      }
    }
    let runner = YtdlpRunner::new(&app)
      .with_progress_args()
      .with_network_args(entry.overrides.as_ref())
      .with_auth_args(
        entry.overrides.as_ref(),
        login.browser.as_deref(),
        &entry.url,
      )
      .with_cookie_access(Some(&cookies))
      .with_subtitle_args(entry.overrides.as_ref(), entry.subtitle_inventory.as_ref())
      .with_sponsorblock_args(entry.overrides.as_ref())
      .with_format_args(&entry.format, entry.overrides.as_ref())
      .with_input_filter_args(entry.overrides.as_ref())
      .with_input_args(entry.overrides.as_ref())
      .with_args(transfer_args(
        performance.concurrent_fragments,
        aria.as_deref(),
        login.state.native_fallback,
      ));
    #[cfg(target_os = "macos")]
    let keep_original_codec = app
      .state::<SharedConfig>()
      .load()
      .output
      .video
      .keep_original_codec;
    #[cfg(target_os = "macos")]
    let final_paths = if keep_original_codec {
      None
    } else {
      match super::gallery_files::Staging::new() {
        Ok(stage) => Some(stage),
        Err(_) => {
          tracing::warn!("QuickTime compatibility check unavailable; original download kept");
          None
        }
      }
    };
    #[cfg(target_os = "macos")]
    let runner = if let Some(paths) = &final_paths {
      runner.with_args([
        "--print-to-file".into(),
        "after_move:%(filepath)j".into(),
        paths
          .path
          .join("paths.jsonl")
          .to_string_lossy()
          .into_owned(),
      ])
    } else {
      runner
    };
    let output_args = runner
      .output_args(&entry.format, entry.overrides.as_ref())
      .map_err(YtdlpDownloadError::InvalidArguments)?;
    if let Some(records) = app.try_state::<DownloadRecords>() {
      records.allow_root(
        &entry.group_id,
        runner.output_root(&entry.format.track_type, entry.overrides.as_ref()),
      );
    }
    let runner = runner.with_args(selected_video_args(&entry));
    let runner = runner.with_args(output_args).with_location_args(
      &entry.format.track_type,
      &entry.template_context,
      entry.overrides.as_ref(),
    );
    // Refresh the source immediately before transfer. Snapchat's opaque CDN suffix is
    // not a file format; normalize only that provider's video/thumbnail metadata.
    let snapchat_info = if snapchat_media_page(&entry.url) {
      let Some(output) = YtdlpRunner::new(&app)
        .with_auth_args(
          entry.overrides.as_ref(),
          login.browser.as_deref(),
          &entry.url,
        )
        .with_cookie_access(Some(&cookies))
        .with_network_args(entry.overrides.as_ref())
        .with_args(["-J", "--no-playlist"])
        .with_resolved_input(&input)
        .output(&entry.group_id)
        .await
        .map_err(YtdlpDownloadError::RunnerError)?
      else {
        return Ok(());
      };
      // Failed extraction/snapshot validation returns to the ordinary runner's diagnostics and retries.
      let snapshot = if output.code == 0 && output.stdout.len() <= 8 * 1024 * 1024 {
        serde_json::from_slice::<serde_json::Value>(&output.stdout)
          .ok()
          .filter(serde_json::Value::is_object)
      } else {
        None
      };
      if let Some(mut info) = snapshot {
        normalize_snapchat_extensions(&mut info);
        let stage = crate::runners::gallery_files::Staging::new()
          .map_err(|e| YtdlpDownloadError::RunnerError(format!("{e:?}")))?;
        let path = stage
          .json("snapchat.json", &info)
          .map_err(|e| YtdlpDownloadError::RunnerError(format!("{e:?}")))?;
        Some((stage, path))
      } else {
        None
      }
    } else {
      None
    };
    let runner = if let Some((_, path)) = &snapchat_info {
      runner.with_args([
        "--load-info-json".into(),
        path.to_string_lossy().into_owned(),
      ])
    } else {
      runner.with_resolved_input(&input)
    };
    let mut login_reason = None;
    let mut cookie_failure = CookieFetchFailure::Other;
    let mut transport_denied = false;
    let mut media_expired = false;
    let mut unsupported = false;
    let mut expired = false;
    let mut diagnostics = Vec::new();
    let mut last_error = None;
    let partial_download_duration_secs = entry
      .overrides
      .as_ref()
      .and_then(|overrides| overrides.output.as_ref())
      .and_then(|output| output.partial_download.as_ref())
      .and_then(|partial| partial.section.as_ref())
      .and_then(download_section_duration_secs)
      .filter(|value| *value > 0.0);
    let mut progress_parser = YtdlpProgressParser::new(
      &entry.id,
      &entry.group_id,
      progress_category_for_track_type(&entry.format.track_type),
      partial_download_duration_secs,
    );

    let (mut rx, child) = match runner.spawn(&entry.group_id).await {
      Ok(result) => result,
      Err(err) => {
        let message = format!("Failed to spawn yt-dlp: {err}");
        let _ = app.emit(
          "media_fatal",
          MediaFatalPayload::with_exit(entry.group_id.clone(), entry.id.clone(), 1, message),
        );
        return Err(YtdlpDownloadError::SpawnFailed(err));
      }
    };
    loop {
      tokio::select! {
        event = rx.recv() => {
          let Some(event) = event else {
            break;
          };

          if is_cancelled_now(&cancel_rx) {
            tracing::info!("Cancelled processing for group_id {}", entry.group_id);
            let _ = child.kill_tree();
            return Ok(());
          }

          let log_state = app.state::<LogStoreState>();

          match event {
            YtdlpCommandEvent::Stdout(line) => {
              let line_str = String::from_utf8_lossy(&line);
              cookie_failure = cookie_failure.combine(error_parser.cookie_fetch_failure(&line_str));
              store_log_line(&line_str, &entry, log_state, &app);
              parse_progress_line(&line_str, &mut progress_parser, &app);
              if let Some(event) = error_parser.parse_fetch_line(&line_str, login.uses_cookies()) {
                if matches!(event.level, crate::models::error::DiagnosticLevel::Error) {
                  last_error = Some(event.raw.clone());
                }
                if diagnostics.len() < 64 { diagnostics.push(event); }
              }
            }
            YtdlpCommandEvent::Stderr(line) => {
              let line_str = String::from_utf8_lossy(&line);
              unsupported |= resolve::unsupported_url(&line_str);
              // Classify without retaining page contents or potentially sensitive URLs.
              let lower = line_str.to_ascii_lowercase();
              expired |= lower.contains("http error 403") || lower.contains("http error 404");
              login_reason = login_reason.or_else(|| error_parser.login_reason(&line_str));
              transport_denied |= aria.is_some() && aria_denied(&line_str);
              media_expired |= selected_media_expired(&entry, &input, &line_str);
              cookie_failure = cookie_failure.combine(error_parser.cookie_fetch_failure(&line_str));
              store_log_line(&line_str, &entry, log_state, &app);
              parse_progress_line(&line_str, &mut progress_parser, &app);
              if let Some(event) = error_parser.parse_fetch_line(&line_str, login.uses_cookies()) {
                tracing::warn!(diagnostic_code = %event.code, error = %redact_diagnostic(&line_str).lines().next().unwrap_or_default(), "yt-dlp download diagnostic");
                if matches!(event.level, crate::models::error::DiagnosticLevel::Error) {
                  last_error = Some(event.raw.clone());
                }
                if diagnostics.len() < 64 { diagnostics.push(event); }
              }
            }
            YtdlpCommandEvent::Terminated(term) => {
              if term.code == Some(0) {
                #[cfg(target_os = "macos")]
                if let Some(paths) = &final_paths {
                  match make_quicktime_compatible(&app, &entry, &paths.path.join("paths.jsonl"), &mut cancel_rx).await {
                    Ok(true) => {}
                    Ok(false) => return Ok(()),
                    Err(_) => {
                      tracing::warn!("QuickTime compatibility conversion failed; original download kept");
                    }
                  }
                }
                if is_cancelled_now(&cancel_rx) { return Ok(()); }
                if let Some(records) = app.try_state::<DownloadRecords>() {
                  records.seal(&entry.group_id);
                }
                login.succeeded(&app, &entry.group_id);
                let path = if aria.is_some() { "aria2" } else { "native" };
                tracing::info!(transport = path, "Download transport completed");
                let _ = app.emit("media_transport", serde_json::json!({"id":entry.id,"groupId":entry.group_id,"path":path}));
                if entry.emit_complete { crate::library::complete(&app, &entry).await; }
                return Ok(());
              }

              if is_cancelled_now(&cancel_rx) { return Ok(()); }
              if media_expired { return Err(YtdlpDownloadError::ExpiredMedia); }
              if tokio::select! {
                biased;
                _ = cancel_rx.changed() => return Ok(()),
                retry = login.retry_cookie_fetch(cookie_failure, tokio::time::sleep) => retry,
              } { continue 'attempt; }
              let login_reason = if login.uses_cookies() {
                (cookie_failure == CookieFetchFailure::Refused).then(|| "signInRequired".to_owned())
              } else { login_reason };
              if expired && input.retry_original("HTTP Error 403") { continue 'attempt; }
              login.record_failure(&diagnostics, false);
              match login.failed(&app, LoginRequest::Download(entry.login_request_entry()), login_reason.as_deref(), transport_denied, is_cancelled_now(&cancel_rx)) {
                RetryAction::Native | RetryAction::Browser | RetryAction::Anonymous => continue 'attempt,
                RetryAction::AskLogin => return Err(YtdlpDownloadError::LoginPending),
                RetryAction::Stop => {}
              }
              if unsupported && !generic_tried {
                generic_tried = true;
                match resolve::after_unsupported(&entry.url, "ERROR: Unsupported URL:").await {
                  Some(Resolution::Passthrough(url)) if url != input.url => { input.rewrite(url); continue 'attempt; }
                  Some(Resolution::Unsupported { reason }) => {
                    if is_cancelled_now(&cancel_rx) { return Ok(()); }
                    if !login.has_better_failure(reason.code()) {
                      emit_unsupported(&app, &entry.group_id, &entry.id, reason);
                      return Err(YtdlpDownloadError::Unsupported(reason));
                    }
                  }
                  _ => {}
                }
              }
              for event in login.failure_diagnostics(diagnostics) { let _ = app.emit("media_diagnostic", MediaDiagnosticPayload::from_diagnostic_event(event)); }
              let exit = term.code.unwrap_or(1);
              let mut fatal = MediaFatalPayload::with_exit(
                  entry.group_id.clone(),
                  entry.id.clone(),
                  exit,
                  format!("Download failed for group {}", entry.group_id),
              );
              fatal.details = last_error;
              let _ = app.emit("media_fatal", fatal);
              return Err(YtdlpDownloadError::NonZeroExit(exit));
            }
            YtdlpCommandEvent::Error(err) => {
              let _ = child.kill_tree();
              let msg = format!("Download failed for group {}: {}", entry.group_id, err);
              emit_internal_fatal(&app, &entry, msg.clone(), Some(err.clone()));

              return Err(YtdlpDownloadError::RunnerError(err));
            }
          }
        }
        _ = cancel_rx.changed() => {
          if is_cancelled_now(&cancel_rx) {
            tracing::info!("Cancelled processing for group_id {}", entry.group_id);
            let _ = child.kill_tree();
            return Ok(());
          }
        }
      }
    }

    let _ = child.kill_tree();
    emit_internal_fatal(
      &app,
      &entry,
      "yt-dlp event stream ended unexpectedly".to_string(),
      None,
    );
    return Err(YtdlpDownloadError::EventStreamEnded);
  }
}

#[cfg(target_os = "macos")]
async fn make_quicktime_compatible(
  app: &AppHandle,
  entry: &DownloadEntry,
  path_file: &std::path::Path,
  cancel: &mut watch::Receiver<bool>,
) -> Result<bool, String> {
  let paths = super::quicktime::read_final_paths(path_file)?;
  let paths = paths.iter().filter(|path| {
    path
      .extension()
      .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
  });
  let manager = crate::binaries::binaries_manager::BinariesManager::for_app(app);
  for path in paths {
    if is_cancelled_now(cancel) {
      return Ok(false);
    }
    let ffmpeg = manager
      .tool_path("ffmpeg")
      .map_err(|_| "QuickTime encoder requires repair")?;
    let ffprobe = manager
      .tool_path("ffprobe")
      .map_err(|_| "QuickTime codec probe requires repair")?;
    let completed = super::quicktime::convert_file(&ffmpeg, &ffprobe, path, cancel, |percentage| {
      if percentage.is_none() {
        let _ = app.emit(
          "media_progress_stage",
          crate::models::MediaProgressStage {
            id: entry.id.clone(),
            group_id: entry.group_id.clone(),
            stage: crate::models::ProgressStage::Reencoding,
          },
        );
      }
      let _ = app.emit(
        "media_progress",
        crate::models::MediaProgress {
          id: entry.id.clone(),
          group_id: entry.group_id.clone(),
          category: crate::models::ProgressCategory::Video,
          percentage,
          speed_bps: None,
          eta_secs: None,
        },
      );
    })
    .await;
    match completed {
      Ok(true) => {}
      Ok(false) => return Ok(false),
      Err(_) => tracing::warn!("QuickTime compatibility conversion failed; original download kept"),
    }
  }
  Ok(!is_cancelled_now(cancel))
}

fn emit_internal_fatal(
  app: &AppHandle,
  entry: &DownloadEntry,
  message: String,
  details: Option<String>,
) {
  let _ = app.emit(
    "media_fatal",
    MediaFatalPayload::internal(entry.group_id.clone(), entry.id.clone(), message, details),
  );
}

fn is_cancelled_now(cancel_rx: &watch::Receiver<bool>) -> bool {
  !*cancel_rx.borrow()
}

fn store_log_line(
  line: &str,
  entry: &DownloadEntry,
  log_state: State<LogStoreState>,
  app: &AppHandle,
) {
  if line.is_empty() || line.starts_with("RAW") {
    return;
  }
  let mut store = log_state.write();
  store.append_line(app, &entry.group_id, &redact_diagnostic(line));
}

fn parse_progress_line(line: &str, progress_parser: &mut YtdlpProgressParser, app: &AppHandle) {
  let progress_events = progress_parser.parse_line(line);

  for progress_event in progress_events {
    match progress_event {
      ProgressEvent::Destination(destination) => {
        if let Some(records) = app.try_state::<DownloadRecords>() {
          records.record_item(
            &destination.group_id,
            &destination.id,
            &destination.destination.path,
          );
        }
        app.emit("media_destination", destination).ok();
      }
      ProgressEvent::Progress(progress) => {
        app.emit("media_progress", progress).ok();
      }
      ProgressEvent::StageChange(progress) => {
        app.emit("media_progress_stage", progress).ok();
      }
    }
  }
}

fn download_section_duration_secs(section: &DownloadSection) -> Option<f64> {
  let start = parse_clock_secs(&section.start)?;
  let end = parse_clock_secs(&section.end)?;
  (end > start).then_some(end - start)
}

fn parse_clock_secs(value: &str) -> Option<f64> {
  let mut total = 0.0;
  let parts = value.trim().split(':').collect::<Vec<_>>();
  if parts.len() != 3 {
    return None;
  }

  for (index, part) in parts.iter().enumerate() {
    let component = part.trim().parse::<f64>().ok()?;
    total += component
      * match index {
        0 => 3600.0,
        1 => 60.0,
        2 => 1.0,
        _ => unreachable!(),
      };
  }

  Some(total)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn expired_selected_cdn_video_returns_to_its_parent_before_login_or_fatal_events() {
    let item = serde_json::from_value(serde_json::json!({"id":"post","url":"https://www.threads.com/@user/post/abc","format":{"trackType":"both"},"templateContext":{"values":{}}})).unwrap();
    let mut entry = DownloadEntry::from((item, "expired-child".into()));
    let source = ResolvedInput::new(&entry.url);
    let mut cdn = ResolvedInput::new("https://cdn.example/video.mp4?token=old");
    cdn.source_page = Some(entry.url.clone());
    entry.resume_parent = Some(Box::new(entry.clone()));
    assert!(selected_media_expired(
      &entry,
      &cdn,
      "ERROR: HTTP Error 410: Gone"
    ));
    assert!(!selected_media_expired(
      &entry,
      &source,
      "ERROR: HTTP Error 403: Forbidden"
    ));
    assert!(!selected_media_expired(
      &entry,
      &cdn,
      "ERROR: Login required"
    ));
    entry.resume_parent = None;
    assert!(!selected_media_expired(
      &entry,
      &cdn,
      "ERROR: HTTP Error 403: Forbidden"
    ));
  }

  #[test]
  fn snapchat_opaque_cdn_suffixes_use_mp4_video_and_jpg_thumbnail_metadata() {
    let mut info: serde_json::Value =
      serde_json::from_slice(include_bytes!("../parsers/fixtures/ytdlp-snapchat.json")).unwrap();
    normalize_snapchat_extensions(&mut info);
    assert_eq!(
      (
        info["ext"].as_str(),
        info["formats"][0]["ext"].as_str(),
        info["thumbnails"][0]["ext"].as_str()
      ),
      (Some("mp4"), Some("mp4"), Some("jpg"))
    );
    let ext = "opaqueSuffix";
    let mut info = serde_json::json!({"ext":ext,"video_ext":ext,"url":"https://cf-st.sc-cdn.net/d/resource.other?signature=fixture","requested_downloads":[{"ext":ext,"url":"https://sc-cdn.net/media"}],"entries":[{"requested_formats":[{"ext":ext,"url":"https://cf-st.sc-cdn.net/media"}]}],"thumbnails":[{"ext":ext,"url":"https://cf-st.sc-cdn.net/resource.other"}]});
    normalize_snapchat_extensions(&mut info);
    assert_eq!(
      (
        info["ext"].as_str(),
        info["video_ext"].as_str(),
        info["requested_downloads"][0]["ext"].as_str(),
        info["entries"][0]["requested_formats"][0]["ext"].as_str(),
        info["thumbnails"][0]["ext"].as_str()
      ),
      (
        Some("mp4"),
        Some("mp4"),
        Some("mp4"),
        Some("mp4"),
        Some("jpg")
      ),
      "{ext}"
    );
  }

  #[test]
  fn snapchat_extension_normalization_preserves_other_providers_and_safe_formats() {
    let mut info = serde_json::json!({"ext":"webm","url":"https://cf-st.sc-cdn.net/video.IRZXSOY","formats":[{"ext":"IRZXSOY","url":"https://sc-cdn.net.evil.test/video.IRZXSOY"}],"thumbnails":[{"ext":"png","url":"https://example.com/thumb.png"},{"ext":"png","url":"https://cf-st.sc-cdn.net/thumb.png"}]});
    let expected = info.clone();
    normalize_snapchat_extensions(&mut info);
    assert_eq!(info, expected);
  }

  #[test]
  fn snapchat_metadata_workaround_does_not_apply_to_other_sites() {
    for (source, want) in [
      ("https://www.snapchat.com/@nasa/highlight/id", true),
      ("https://www.snapchat.com/@nasa/spotlight/id", true),
      ("https://www.snapchat.com/spotlight/id", true),
      ("https://story.snapchat.com/@nasa/highlight/id", true),
      ("https://snapchat.com.evil.test/@nasa/highlight/id", false),
      ("https://example.com/snapchat.com/highlight/id", false),
      ("https://www.tiktok.com/@u/video/1", false),
      ("https://www.pinterest.com/pin/1", false),
      ("https://imgur.com/a/1", false),
    ] {
      assert_eq!(snapchat_media_page(source), want, "{source}");
    }
  }
}
