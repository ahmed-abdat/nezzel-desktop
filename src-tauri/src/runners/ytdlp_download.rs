use crate::download_records::DownloadRecords;
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
  login::{CookieRetry, LoginRequest, LoginSession},
  retry_policy::RetryAction,
  transport::{aria_denied, resolve_aria2, single_direct_http, transfer_args},
};
use crate::scheduling::download_pipeline::DownloadEntry;
use crate::scheduling::group_state::subscribe_group;
use crate::SharedConfig;
use std::fmt;
use tauri::{AppHandle, Manager, State};
use tokio::sync::watch;

const PARTIAL_PREFIX: &str = ".nezzel-partials-";
const PARTIAL_OWNER: &str = ".owner.json";

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum PartialState {
  Running,
  Paused,
  Queued,
  Discarded,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PartialOwner {
  group: String,
  item: String,
  state: PartialState,
}

fn partial_name(group: &str, item: &str) -> String {
  use sha2::{Digest, Sha256};
  let mut hash = Sha256::new();
  hash.update((group.len() as u64).to_le_bytes());
  hash.update(group.as_bytes());
  hash.update(item.as_bytes());
  format!("{PARTIAL_PREFIX}{}", hex::encode(hash.finalize()))
}

/// The registry owns resume data; process leases delay deletion until all writers have exited.
/// Paused data lasts for this session; startup removes it because the UI queue is not persisted.
pub(crate) struct DownloadPartial {
  pub path: std::path::PathBuf,
  group: String,
  item: String,
  discarded: std::sync::atomic::AtomicBool,
}
impl DownloadPartial {
  fn open(root: &std::path::Path, group: &str, item: &str) -> Result<Self, String> {
    use std::fs;
    let create = || -> std::io::Result<Self> {
      fs::create_dir_all(root)?;
      let path = root.canonicalize()?.join(partial_name(group, item));
      let mut builder = fs::DirBuilder::new();
      #[cfg(unix)]
      {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
      }
      // A startup sweep may still be deleting this deterministic path. Never adopt an old
      // directory: in-session resumes already hold its lease in PartialDownloads.
      builder.create(&path)?;
      let partial = Self {
        path,
        group: group.to_owned(),
        item: item.to_owned(),
        discarded: false.into(),
      };
      partial.set_state(PartialState::Running)?;
      Ok(partial)
    };
    create().map_err(|_| "Could not prepare download temporary directory".to_owned())
  }
  fn set_state(&self, state: PartialState) -> std::io::Result<()> {
    use std::{fs, io::Write};
    let temporary = self
      .path
      .join(format!(".owner-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
      let mut options = fs::OpenOptions::new();
      options.write(true).create_new(true);
      #[cfg(unix)]
      {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
      }
      let mut file = options.open(&temporary)?;
      let owner = PartialOwner {
        group: self.group.clone(),
        item: self.item.clone(),
        state,
      };
      file.write_all(&serde_json::to_vec(&owner)?)?;
      file.sync_all()?;
      fs::rename(&temporary, self.path.join(PARTIAL_OWNER))
    })();
    if result.is_err() {
      let _ = fs::remove_file(temporary);
    }
    result
  }
  fn discard(&self) {
    self
      .discarded
      .store(true, std::sync::atomic::Ordering::Release);
    if self.set_state(PartialState::Discarded).is_err() {
      tracing::warn!("Unable to mark discarded download partials");
    }
  }
}
impl Drop for DownloadPartial {
  fn drop(&mut self) {
    if self.discarded.load(std::sync::atomic::Ordering::Acquire) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }
}

fn read_partial_owner(path: &std::path::Path) -> Option<PartialOwner> {
  let path = path.join(PARTIAL_OWNER);
  let metadata = std::fs::symlink_metadata(&path).ok()?;
  if !metadata.is_file() || metadata.len() > 16 * 1024 {
    return None;
  }
  serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[derive(Default)]
struct PartialDownloads(
  std::sync::Mutex<std::collections::HashMap<(String, String), std::sync::Arc<DownloadPartial>>>,
);
static PARTIAL_DOWNLOADS: std::sync::LazyLock<PartialDownloads> =
  std::sync::LazyLock::new(PartialDownloads::default);

impl PartialDownloads {
  fn get(
    &self,
    group: &str,
    item: &str,
    cancel: &watch::Receiver<bool>,
    root: &std::path::Path,
  ) -> Result<Option<std::sync::Arc<DownloadPartial>>, String> {
    let mut entries = self.0.lock().unwrap();
    // Cancellation and registration share this lock with pause/discard.
    if !*cancel.borrow() {
      return Ok(None);
    }
    let key = (group.to_owned(), item.to_owned());
    if let Some(partial) = entries.get(&key) {
      partial
        .set_state(PartialState::Running)
        .map_err(|_| "Unable to record partial ownership")?;
      return Ok(Some(partial.clone()));
    }
    let partial = std::sync::Arc::new(DownloadPartial::open(root, group, item)?);
    entries.insert(key, partial.clone());
    Ok(Some(partial))
  }
  fn discard(&self, group: &str) {
    self.0.lock().unwrap().retain(|(id, _), partial| {
      if id != group {
        return true;
      }
      partial.discard();
      false
    });
  }
  fn state(&self, group: &str, state: PartialState) {
    for ((id, _), partial) in self.0.lock().unwrap().iter() {
      if id == group && partial.set_state(state).is_err() {
        tracing::warn!("Unable to record resumable download ownership");
      }
    }
  }
  fn item_state(&self, group: &str, item: &str, state: PartialState) {
    if let Some(partial) = self
      .0
      .lock()
      .unwrap()
      .get(&(group.to_owned(), item.to_owned()))
    {
      if partial.set_state(state).is_err() {
        tracing::warn!("Unable to record resumable download ownership");
      }
    }
  }
  fn complete(&self, group: &str, item: &str) {
    if let Some(partial) = self
      .0
      .lock()
      .unwrap()
      .remove(&(group.to_owned(), item.to_owned()))
    {
      partial.discard();
    }
  }
  fn terminal_codes<'a>(&self, group: &str, item: &str, codes: impl Iterator<Item = &'a str>) {
    if codes
      .into_iter()
      .any(|code| crate::extract::outcome::PERMANENT_CODES.contains(&code))
    {
      self.complete(group, item);
    }
  }
  fn finished(
    &self,
    group: &str,
    item: &str,
    result: &Result<(), YtdlpDownloadError>,
    running: bool,
  ) {
    match result {
      Ok(()) if running => self.complete(group, item),
      Err(YtdlpDownloadError::LoginPending) => self.item_state(group, item, PartialState::Queued),
      Err(
        YtdlpDownloadError::Filtered
        | YtdlpDownloadError::Unsupported(_)
        | YtdlpDownloadError::InvalidArguments(_)
        | YtdlpDownloadError::InvalidDiagnosticRules(_),
      ) => self.complete(group, item),
      Err(YtdlpDownloadError::RunnerError(code)) => {
        self.terminal_codes(group, item, std::iter::once(code.as_str()))
      }
      _ => {}
    }
  }
}

/// Classify before dispatchers start, without recursively deleting on the setup thread.
fn stale_partials(roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
  let mut paths = std::collections::HashSet::new();
  for root in roots {
    let Ok(root) = root.canonicalize() else {
      continue;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
      continue;
    };
    for entry in entries.flatten() {
      let name = entry.file_name();
      let Some(name) = name.to_str() else {
        continue;
      };
      let Some(suffix) = name.strip_prefix(PARTIAL_PREFIX) else {
        continue;
      };
      if suffix.len() != 64 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        continue;
      }
      // Never follow symlinks, recurse into other folders, or delete an unverified prefix match.
      if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
        continue;
      }
      let owned = read_partial_owner(&entry.path())
        .is_some_and(|owner| name == partial_name(&owner.group, &owner.item));
      // The startup registry is empty: Paused/Queued owners cannot return with their old IDs.
      if owned {
        paths.insert(entry.path());
      }
    }
  }
  paths.into_iter().collect()
}

fn remove_stale_partials(paths: Vec<std::path::PathBuf>) {
  for path in paths {
    if std::fs::remove_dir_all(path).is_err() {
      tracing::warn!("Download partial sweep deferred");
    }
  }
}

pub(crate) fn sweep_download_partials(app: &AppHandle) {
  let cfg = app.state::<crate::SharedConfig>().load();
  let prefs = app.state::<crate::SharedPreferences>().load();
  // Only current configured roots are swept; old output folders are not indexed.
  let roots: Vec<_> = [
    cfg.output.download_dir.as_deref(),
    prefs.paths.video_download_dir.as_deref(),
    prefs.paths.audio_download_dir.as_deref(),
  ]
  .into_iter()
  .flatten()
  .map(std::path::PathBuf::from)
  .collect();
  let paths = stale_partials(&roots);
  if !paths.is_empty() {
    tauri::async_runtime::spawn_blocking(move || remove_stale_partials(paths));
  }
}

pub(crate) fn discard_group(group: &str) {
  PARTIAL_DOWNLOADS.discard(group);
}
pub(crate) fn pause_group(group: &str) {
  PARTIAL_DOWNLOADS.state(group, PartialState::Paused);
}
pub(crate) fn download_finished(
  group: &str,
  item: &str,
  result: &Result<(), YtdlpDownloadError>,
  running: bool,
) {
  PARTIAL_DOWNLOADS.finished(group, item, result, running);
}

/// The row displays the last diagnostic; ffmpeg's exit code must not hide a proven cause.
fn filtered_download_diagnostics(
  parser: &YtdlpErrorParser,
  diagnostics: &[crate::models::error::DiagnosticEvent],
) -> Vec<crate::models::error::DiagnosticEvent> {
  let mut skipped: Vec<_> = diagnostics
    .iter()
    .filter(|event| event.code == "input_filter_skipped")
    .cloned()
    .collect();
  if skipped.is_empty() {
    skipped.extend(parser.parse_line("[download] Skipping: nothing matched input filters"));
  }
  if let Some(mut terminal) = skipped.last().cloned() {
    // Preserve the skipped-items detail and use the existing localized terminal row message.
    terminal.code = "rejectedByFilter".into();
    skipped.push(terminal);
  }
  skipped
}

fn download_failure_diagnostics(
  mut events: Vec<crate::models::error::DiagnosticEvent>,
) -> Vec<crate::models::error::DiagnosticEvent> {
  let cause = events
    .iter()
    .position(|event| event.code == "drmProtected")
    .or_else(|| {
      events
        .last()
        .filter(|event| matches!(event.code.as_str(), "unknown" | "ffmpegFailed"))
        .and_then(|_| {
          events
            .iter()
            .position(|event| matches!(event.code.as_str(), "diskFull" | "outputNameTooLong"))
            .or_else(|| {
              events
                .iter()
                .position(|event| event.code == "accessForbidden403")
            })
        })
    });
  if let Some(index) = cause {
    let cause = events.remove(index);
    events.push(cause);
  }
  events
}

/// Preserve the original HTTP failure if this yt-dlp build cannot impersonate Chrome.
fn retry_vimeo(
  input: &mut ResolvedInput,
  stderr: &str,
  diagnostics: &mut Vec<crate::models::error::DiagnosticEvent>,
  original: &mut Option<Vec<crate::models::error::DiagnosticEvent>>,
) -> bool {
  if input.impersonation_unavailable(stderr) {
    if let Some(events) = original.as_ref() {
      diagnostics.clone_from(events);
    }
  } else if input.retry_vimeo_401(stderr) {
    *original = Some(diagnostics.clone());
    return true;
  }
  false
}

#[derive(Debug)]
pub enum YtdlpDownloadError {
  Filtered,
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
      Self::Filtered => write!(f, "input_filter_skipped"),
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

/// Keep embedding sidecars out of the save folder after yt-dlp's final move.
fn stage_thumbnails(
  args: &mut Vec<String>,
) -> Result<Option<crate::runners::gallery_files::Staging>, YtdlpDownloadError> {
  if !args.iter().any(|arg| arg == "--embed-thumbnail")
    || args.iter().any(|arg| arg == "--write-thumbnail")
  {
    return Ok(None);
  }
  let stage = crate::runners::gallery_files::Staging::new()
    .map_err(|e| YtdlpDownloadError::RunnerError(format!("{e:?}")))?;
  // Explicit retention makes MoveFiles move the sidecar to our owned path, even if
  // an embedding postprocessor skips it. Staging removes it when the attempt ends.
  args.extend([
    "--write-thumbnail".into(),
    "--output".into(),
    format!("thumbnail:{}/%(id)s.%(ext)s", stage.path.display()),
  ]);
  Ok(Some(stage))
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
  let matcher = match DiagnosticMatcher::shared() {
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
        Resolution::Passthrough(url) if resolve::facebook_photo_target(&url) => {
          entry.fetch_engine = Some(crate::models::parsed::FetchEngine::Gallery);
          input.rewrite(url);
          entry.resolved = Some(Box::new(input));
          return Box::pin(crate::runners::gallery::run_media_download(app, entry)).await;
        }
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
        Resolution::Unsupported {
          reason: Reason::PhotosOnly,
        } => {
          entry.fetch_engine = Some(crate::models::parsed::FetchEngine::Gallery);
          return Box::pin(crate::runners::gallery::run_media_download(app, entry)).await;
        }
        Resolution::Failed { failure } => {
          let mut event = super::gallery::error_diagnostic(
            &entry.group_id,
            &entry.id,
            &crate::parsers::gallery::GalleryError::Refused(failure.code()),
            false,
          );
          event.component = Some("resolve".into());
          let message = failure.code().to_owned();
          crate::extract::terminal::emit_terminal(
            &app,
            MediaDiagnosticPayload::from_diagnostic_event(event),
            MediaFatalPayload::with_exit(entry.group_id.clone(), entry.id.clone(), 1, message),
          );
          return Err(YtdlpDownloadError::RunnerError(failure.code().into()));
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
  let mut vimeo_diagnostics = None;
  'attempt: loop {
    if is_cancelled_now(&cancel_rx) {
      return Ok(());
    }
    login.announce(&app, &entry.id, &entry.group_id);
    // Browser-consent resumes retain both the resolved target and its original page/referer.
    entry.resolved = Some(Box::new(input.clone()));
    login.has_session = false;
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
          if super::browser_cookies::read_error_code(&error).is_some()
            && super::login::automatic::enabled(&app)
            && matches!(
              login.failed(
                &app,
                LoginRequest::Download(entry.login_request_entry()),
                Some("signInRequired"),
                false,
                false
              ),
              RetryAction::Pending
            )
          {
            return Err(YtdlpDownloadError::LoginPending);
          }
          if let Some(code) = super::browser_cookies::read_error_code(&error) {
            let event = super::gallery::error_diagnostic(
              &entry.group_id,
              &entry.id,
              &crate::parsers::gallery::GalleryError::Refused(code),
              false,
            );
            super::gallery::diagnostic(&app, &entry.group_id, &entry.id, event);
          } else {
            crate::events::emit(
              &app,
              crate::events::MEDIA_FATAL,
              MediaFatalPayload::with_exit(
                entry.group_id.clone(),
                entry.id.clone(),
                1,
                format!("Download failed for group {}", entry.group_id),
              ),
            );
          }
          return Err(YtdlpDownloadError::RunnerError(error));
        }
      }
    };
    login.has_session = cookies.has_session;
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
            if let Some(event) = error_parser.parse_fetch_line(line, login.has_session) {
              tracing::warn!(diagnostic_code = %event.code, "yt-dlp download probe diagnostic");
            }
          }
          app.state::<LogStoreState>().write().append_lines(
            &app,
            &entry.group_id,
            redact_diagnostic(&stderr).lines(),
          );
          if output.code != 0 {
            let reason = stderr
              .lines()
              .find_map(|line| error_parser.login_reason_with_cookies(line, login.has_session));
            if super::facebook_retry::retry(
              &app,
              &entry.group_id,
              &mut input,
              login.has_session,
              reason.as_deref(),
              &stderr,
            )
            .await
            {
              continue 'attempt;
            }
            if selected_media_expired(&entry, &input, &stderr) {
              return Err(YtdlpDownloadError::ExpiredMedia);
            }
            let failure = error_parser.cookie_fetch_failure(&stderr, login.has_session);
            let mut diagnostics =
              error_parser.parse_lines(stderr.lines().collect(), login.has_session);
            if retry_vimeo(
              &mut input,
              &stderr,
              &mut diagnostics,
              &mut vimeo_diagnostics,
            ) {
              continue 'attempt;
            }
            let failure_details = if input.impersonation_unavailable(&stderr) {
              diagnostics
                .iter()
                .map(|event| event.raw.as_str())
                .collect::<Vec<_>>()
                .join("\n")
            } else {
              redact_diagnostic(&stderr)
            };
            login.record_failure(&diagnostics, false);
            match login.retry_cookie_fetch(failure, &mut cancel_rx).await {
              CookieRetry::Cancelled => return Ok(()),
              CookieRetry::Retry => continue 'attempt,
              CookieRetry::GiveUp => {}
            }
            if input.retry_original(&stderr) {
              continue 'attempt;
            }
            let reason = if login.has_session {
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
              RetryAction::AskLogin | RetryAction::Pending => {
                return Err(YtdlpDownloadError::LoginPending)
              }
              _ => {}
            }
            PARTIAL_DOWNLOADS.terminal_codes(
              &entry.group_id,
              &entry.id,
              diagnostics.iter().map(|event| event.code.as_str()),
            );
            for event in download_failure_diagnostics(login.failure_diagnostics(diagnostics)) {
              crate::events::emit(
                &app,
                crate::events::MEDIA_DIAGNOSTIC,
                MediaDiagnosticPayload::from_diagnostic_event(event),
              );
            }
            crate::events::emit(
              &app,
              crate::events::MEDIA_FATAL,
              MediaFatalPayload::with_exit(
                entry.group_id.clone(),
                entry.id.clone(),
                output.code,
                failure_details.chars().take(8192).collect(),
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
    let mut output_args = runner
      .output_args(&entry.format, entry.overrides.as_ref())
      .map_err(YtdlpDownloadError::InvalidArguments)?;
    let output_root = runner.output_root(&entry.format.track_type, entry.overrides.as_ref());
    if let Some(records) = app.try_state::<DownloadRecords>() {
      records.allow_root(&entry.group_id, output_root.clone());
    }
    let Some(temporary) = PARTIAL_DOWNLOADS
      .get(&entry.group_id, &entry.id, &cancel_rx, &output_root)
      .map_err(YtdlpDownloadError::RunnerError)?
    else {
      return Ok(());
    };
    let _thumbnail_stage = stage_thumbnails(&mut output_args)?;
    let runner = runner.with_args(selected_video_args(&entry));
    let (runner, publication_anchor) = runner
      .with_args(output_args)
      .with_args(super::ytdlp_args::filename_metadata_args(
        &entry.url,
        &entry.template_context,
      ))
      .with_location_args(
        &entry.format.track_type,
        &entry.template_context,
        entry.overrides.as_ref(),
      )
      .with_temporary_files(temporary.clone());
    let mut pending_files = super::ytdlp_files::PendingFiles::default();
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
    let mut vimeo_response = "";
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
        crate::events::emit(
          &app,
          crate::events::MEDIA_FATAL,
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
              pending_files.record(&line_str);
              cookie_failure = cookie_failure.combine(error_parser.cookie_fetch_failure(&line_str, login.has_session));
              store_log_line(&line_str, &entry, log_state, &app);
              parse_progress_line(&line_str, &mut progress_parser, &app);
              if let Some(event) = error_parser.parse_fetch_line(&line_str, login.has_session) {
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
              if lower.contains("http error 401") { vimeo_response = "HTTP Error 401"; }
              if input.impersonation_unavailable(&line_str) { vimeo_response = "Impersonate target is not available"; }
              expired |= lower.contains("http error 403") || lower.contains("http error 404");
              login_reason = login_reason.or_else(|| error_parser.login_reason(&line_str));
              transport_denied |= aria.is_some() && aria_denied(&line_str);
              media_expired |= selected_media_expired(&entry, &input, &line_str);
              cookie_failure = cookie_failure.combine(error_parser.cookie_fetch_failure(&line_str, login.has_session));
              store_log_line(&line_str, &entry, log_state, &app);
              parse_progress_line(&line_str, &mut progress_parser, &app);
              if let Some(event) = error_parser.parse_fetch_line(&line_str, login.has_session) {
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
                let stage = temporary.path.join("output");
                let anchor = publication_anchor.clone();
                let source_url = entry.url.clone();
                let receipts = super::ytdlp_files::receipt_directory(&app)
                  .map_err(|error| YtdlpDownloadError::RunnerError(error.to_string()))?;
                let publish_cancel = cancel_rx.clone();
                let outcome = tauri::async_runtime::spawn_blocking(move || {
                  pending_files.publish(&stage, &anchor, &source_url, &receipts, || is_cancelled_now(&publish_cancel))
                }).await.map_err(|error| YtdlpDownloadError::RunnerError(error.to_string()))?;
                let published = match outcome {
                  Ok(super::ytdlp_files::PublishOutcome::Files(files)) => files,
                  Ok(super::ytdlp_files::PublishOutcome::Filtered) => {
                    let skipped = filtered_download_diagnostics(&error_parser, &diagnostics);
                    let message = skipped.last().map(|event| event.message.clone()).unwrap_or_default();
                    for event in skipped {
                      crate::events::emit(&app, crate::events::MEDIA_DIAGNOSTIC, MediaDiagnosticPayload::from_diagnostic_event(event));
                    }
                    crate::events::emit(&app, crate::events::MEDIA_FATAL,
                      MediaFatalPayload::with_exit(entry.group_id.clone(), entry.id.clone(), 0, message));
                    PARTIAL_DOWNLOADS.complete(&entry.group_id, &entry.id);
                    return Err(YtdlpDownloadError::Filtered);
                  }
                  Err(_) if is_cancelled_now(&cancel_rx) => return Ok(()),
                  Err(error) => {
                    emit_internal_fatal(&app, &entry, "Could not save download without replacing an existing file".into(), None);
                    return Err(YtdlpDownloadError::RunnerError(error.to_string()));
                  }
                };
                for file in published {
                  let destination = crate::models::MediaDestination {
                    id: entry.id.clone(), group_id: entry.group_id.clone(), is_merged: false,
                    destination: crate::models::progress::MediaDestinationPath {
                      path: file.path.to_string_lossy().into_owned(), confidence: 100, already_downloaded: file.reused,
                    },
                  };
                  if let Some(records) = app.try_state::<DownloadRecords>() {
                    records.record_item(&entry.group_id, &entry.id, &destination.destination.path);
                  }
                  crate::events::emit(&app, crate::events::MEDIA_DESTINATION, destination);
                }
                if let Some(records) = app.try_state::<DownloadRecords>() {
                  records.seal(&entry.group_id);
                }
                PARTIAL_DOWNLOADS.complete(&entry.group_id, &entry.id);
                login.succeeded(&app, &entry.group_id);
                let path = if aria.is_some() { "aria2" } else { "native" };
                tracing::info!(transport = path, "Download transport completed");
                crate::events::emit(&app, crate::events::MEDIA_TRANSPORT, serde_json::json!({"id":entry.id,"groupId":entry.group_id,"path":path}));
                if entry.emit_complete { crate::library::complete(&app, &entry).await; }
                return Ok(());
              }

              if is_cancelled_now(&cancel_rx) { return Ok(()); }
              if media_expired { return Err(YtdlpDownloadError::ExpiredMedia); }
              let share_error = diagnostics.iter().map(|event| event.raw.as_str()).collect::<Vec<_>>().join("\n");
              if super::facebook_retry::retry(&app, &entry.group_id, &mut input, login.has_session, login_reason.as_deref(), &share_error).await { continue 'attempt; }
              if retry_vimeo(&mut input, vimeo_response, &mut diagnostics, &mut vimeo_diagnostics) { continue 'attempt; }
              if input.impersonation_unavailable(vimeo_response) {
                last_error = diagnostics.iter().rev().find(|event| matches!(event.level, crate::models::error::DiagnosticLevel::Error)).map(|event| event.raw.clone());
              }
              match login.retry_cookie_fetch(cookie_failure, &mut cancel_rx).await {
                CookieRetry::Cancelled => return Ok(()),
                CookieRetry::Retry => continue 'attempt,
                CookieRetry::GiveUp => {}
              }
              let login_reason = if login.has_session {
                (cookie_failure == CookieFetchFailure::Refused).then(|| "signInRequired".to_owned())
              } else { login_reason };
              if expired && input.retry_original("HTTP Error 403") { continue 'attempt; }
              login.record_failure(&diagnostics, false);
              match login.failed(&app, LoginRequest::Download(entry.login_request_entry()), login_reason.as_deref(), transport_denied, is_cancelled_now(&cancel_rx)) {
                RetryAction::Native | RetryAction::Browser | RetryAction::Anonymous => continue 'attempt,
                RetryAction::AskLogin | RetryAction::Pending => return Err(YtdlpDownloadError::LoginPending),
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
              PARTIAL_DOWNLOADS.terminal_codes(&entry.group_id, &entry.id, diagnostics.iter().map(|event| event.code.as_str()));
              for event in download_failure_diagnostics(login.failure_diagnostics(diagnostics)) { crate::events::emit(&app, crate::events::MEDIA_DIAGNOSTIC, MediaDiagnosticPayload::from_diagnostic_event(event)); }
              let exit = term.code.unwrap_or(1);
              let mut fatal = MediaFatalPayload::with_exit(
                  entry.group_id.clone(),
                  entry.id.clone(),
                  exit,
                  format!("Download failed for group {}", entry.group_id),
              );
              fatal.details = last_error;
              crate::events::emit(&app, crate::events::MEDIA_FATAL, fatal);
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
        crate::events::emit(
          app,
          crate::events::MEDIA_PROGRESS_STAGE,
          crate::models::MediaProgressStage {
            id: entry.id.clone(),
            group_id: entry.group_id.clone(),
            stage: crate::models::ProgressStage::Reencoding,
          },
        );
      }
      crate::events::emit(
        app,
        crate::events::MEDIA_PROGRESS,
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
  crate::events::emit(
    app,
    crate::events::MEDIA_FATAL,
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
      // Only publication can grant a final path; engine destinations are private staging files.
      ProgressEvent::Destination(_) => {}
      ProgressEvent::Progress(progress) => {
        crate::usage::speed_sample(app, &progress.group_id, &progress.id, progress.speed_bps);
        crate::events::emit(app, crate::events::MEDIA_PROGRESS, progress);
      }
      ProgressEvent::StageChange(progress) => {
        crate::events::emit(app, crate::events::MEDIA_PROGRESS_STAGE, progress);
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
  fn zero_output_keeps_the_filter_warning_instead_of_a_save_error() {
    let parser = YtdlpErrorParser::new("row", "group", DiagnosticMatcher::shared().unwrap());
    for (line, expected) in [
      (None, "Skipped by input filter"),
      (
        Some("[download] Video does not pass filter (!is_live), skipping .."),
        "Skipped by advanced input filter",
      ),
      (
        Some("[download] File is larger than max-filesize (123 > 100), skipping"),
        "Skipped by maximum filesize filter",
      ),
      (
        Some("[download] 2022-01-05 upload date is not in range 2026-06-18 to 9999-12-31"),
        "Skipped by upload date filter",
      ),
    ] {
      let diagnostics: Vec<_> = line
        .and_then(|line| parser.parse_fetch_line(line, false))
        .into_iter()
        .collect();
      let skipped = filtered_download_diagnostics(&parser, &diagnostics);
      assert_eq!(skipped.len(), 2);
      assert_eq!(skipped[0].code, "input_filter_skipped");
      assert_eq!(skipped[0].message, expected);
      assert_eq!(skipped[1].code, "rejectedByFilter");
      assert!(matches!(
        skipped[0].level,
        crate::models::error::DiagnosticLevel::Warning
      ));
    }
  }

  #[test]
  fn partials_survive_pause_and_discard_only_the_cancelled_item_after_exit() {
    let partials = PartialDownloads::default();
    let (running, cancel) = watch::channel(true);
    let output = super::super::gallery_files::Staging::new().unwrap();
    let stage = partials
      .get("group", "item", &cancel, &output.path)
      .unwrap()
      .unwrap();
    let path = stage.path.clone();
    for name in [
      "video.f137.mp4.part",
      "video.mp4.ytdl",
      "video.temp.mp4",
      "video.mp4.part-Frag1",
    ] {
      std::fs::write(path.join(name), b"partial").unwrap();
    }
    let unrelated = partials
      .get("other", "item", &cancel, &output.path)
      .unwrap()
      .unwrap();
    std::fs::write(unrelated.path.join("keep.part"), b"other download").unwrap();
    std::fs::write(output.path.join("completed.mp4"), b"saved").unwrap();
    // Pause releases the worker, but keeps the receipt and the same resume directory.
    partials.state("group", PartialState::Paused);
    drop(stage);
    let resumed = partials
      .get("group", "item", &cancel, &output.path)
      .unwrap()
      .unwrap();
    assert_eq!(resumed.path, path);
    assert!(path.join("video.f137.mp4.part").exists());
    running.send(false).unwrap();
    partials.discard("group");
    assert!(path.exists(), "the active process still owns its directory");
    assert!(partials
      .get("group", "late", &cancel, &output.path)
      .unwrap()
      .is_none());
    drop(resumed); // Last worker/process owner has exited.
    assert!(!path.exists());
    assert!(unrelated.path.join("keep.part").exists());
    assert!(output.path.join("completed.mp4").exists());
    partials.complete("other", "item");
    let other_path = unrelated.path.clone();
    drop(unrelated);
    assert!(!other_path.exists());
  }

  #[tokio::test]
  async fn startup_sweep_deletes_all_owned_states_off_thread_and_preserves_unverified_paths() {
    use std::fs;
    let fixture = super::super::gallery_files::Staging::new().unwrap();
    let root = fixture.path.join("configured");
    let outside = fixture.path.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let (_running, cancel) = watch::channel(true);
    let mut stale = Vec::new();
    {
      let partials = PartialDownloads::default();
      for (item, state) in [
        ("running", PartialState::Running),
        ("paused", PartialState::Paused),
        ("queued", PartialState::Queued),
        ("discarded", PartialState::Discarded),
      ] {
        let partial = partials
          .get("group", item, &cancel, &root)
          .unwrap()
          .unwrap();
        fs::write(partial.path.join("video.part"), b"old session").unwrap();
        partials.item_state("group", item, state);
        stale.push(partial.path.clone());
      }
    }
    let owner = PartialOwner {
      group: "other".into(),
      item: "item".into(),
      state: PartialState::Paused,
    };
    let malformed = root.join(partial_name("malformed", "item"));
    let missing_owner = root.join(partial_name("missing", "item"));
    let mismatched = root.join(partial_name("mismatched", "item"));
    let legacy_uuid = root.join(format!("{PARTIAL_PREFIX}{}", uuid::Uuid::new_v4()));
    let short = root.join(format!("{PARTIAL_PREFIX}abc"));
    let non_hex = root.join(format!("{PARTIAL_PREFIX}{}", "g".repeat(64)));
    let external = outside.join(partial_name("other", "item"));
    let nested = root.join("nested").join(partial_name("other", "item"));
    let different_prefix = root.join(".nezzel-partialsbackup");
    let preserved = [
      &malformed,
      &missing_owner,
      &mismatched,
      &legacy_uuid,
      &short,
      &non_hex,
      &external,
      &nested,
      &different_prefix,
    ];
    for dir in preserved {
      fs::create_dir_all(dir).unwrap();
      fs::write(dir.join("keep.part"), b"unverified").unwrap();
    }
    for dir in [
      &mismatched,
      &legacy_uuid,
      &short,
      &non_hex,
      &external,
      &nested,
    ] {
      fs::write(dir.join(PARTIAL_OWNER), serde_json::to_vec(&owner).unwrap()).unwrap();
    }
    fs::write(malformed.join(PARTIAL_OWNER), b"not json").unwrap();
    fs::write(root.join("saved.mp4"), b"completed").unwrap();
    #[cfg(unix)]
    {
      std::os::unix::fs::symlink(&external, root.join(partial_name("other", "item"))).unwrap();
    }
    // Duplicate/aliased roots must not schedule a second deletion of a later reused path.
    let paths = stale_partials(&[root.clone(), root.join("."), root.clone()]);
    assert_eq!(paths.len(), stale.len());
    for path in &stale {
      assert!(paths.contains(path));
      assert!(
        path.join("video.part").exists(),
        "classification must not delete data"
      );
    }
    // A new registry must not adopt an old path while background deletion is pending.
    let restarted = PartialDownloads::default();
    assert!(restarted.get("group", "paused", &cancel, &root).is_err());
    tokio::task::spawn_blocking(move || remove_stale_partials(paths))
      .await
      .unwrap();
    for path in stale {
      assert!(!path.exists());
    }
    for dir in preserved {
      assert!(dir.join("keep.part").exists());
    }
    assert!(root.join("saved.mp4").exists());
    #[cfg(unix)]
    assert!(root
      .join(partial_name("other", "item"))
      .symlink_metadata()
      .unwrap()
      .file_type()
      .is_symlink());
    let fresh = restarted
      .get("group", "paused", &cancel, &root)
      .unwrap()
      .unwrap();
    assert!(!fresh.path.join("video.part").exists());
    restarted.complete("group", "paused");
  }

  #[test]
  fn terminal_failures_release_partials_while_retryable_failures_and_pause_keep_them() {
    let output = super::super::gallery_files::Staging::new().unwrap();
    let (_running, cancel) = watch::channel(true);
    let partials = PartialDownloads::default();
    for (item, result, running, retained) in [
      (
        "removed",
        Err(YtdlpDownloadError::Unsupported(Reason::NoMedia)),
        true,
        false,
      ),
      (
        "config",
        Err(YtdlpDownloadError::InvalidArguments("invalid".into())),
        true,
        false,
      ),
      (
        "network",
        Err(YtdlpDownloadError::RunnerError("networkTimeout".into())),
        true,
        true,
      ),
      ("paused", Ok(()), false, true),
      ("success", Ok(()), true, false),
    ] {
      let partial = partials
        .get("group", item, &cancel, &output.path)
        .unwrap()
        .unwrap();
      let path = partial.path.clone();
      std::fs::write(path.join("video.part"), b"bytes").unwrap();
      partials.finished("group", item, &result, running);
      assert!(path.exists(), "process lease delays deletion");
      drop(partial);
      assert_eq!(path.exists(), retained, "{item}");
    }
    let partial = partials
      .get("group", "http", &cancel, &output.path)
      .unwrap()
      .unwrap();
    let path = partial.path.clone();
    partials.terminal_codes("group", "http", ["networkTimeout"].into_iter());
    assert!(path.exists());
    partials.terminal_codes("group", "http", ["notFound404"].into_iter());
    drop(partial);
    assert!(!path.exists());
    partials.discard("group");
  }

  #[test]
  fn embedding_thumbnails_are_temporary_unless_explicitly_saved() {
    let mut args = vec!["--embed-thumbnail".into()];
    let directory;
    {
      let stage = stage_thumbnails(&mut args).unwrap().unwrap();
      directory = stage.path.clone();
      assert!(args.contains(&"--write-thumbnail".to_string()));
      assert_eq!(args[args.len() - 2], "--output");
      assert_eq!(
        args.last().unwrap(),
        &format!("thumbnail:{}/%(id)s.%(ext)s", directory.display())
      );
      std::fs::write(directory.join("leftover.jpg"), b"thumbnail").unwrap();
    }
    assert!(!directory.exists());
    for mut args in [
      vec!["--embed-thumbnail".into(), "--write-thumbnail".into()],
      vec![],
    ] {
      let original = args.clone();
      assert!(stage_thumbnails(&mut args).unwrap().is_none());
      assert_eq!(args, original);
    }
  }

  #[test]
  fn transfer_refusal_precedes_the_generic_ffmpeg_exit_in_the_row() {
    let parser = YtdlpErrorParser::new("id", "group", DiagnosticMatcher::shared().unwrap());
    // Streaming downloads parse each line independently before publishing the batch.
    for (line, code) in [
      ("[in#0] Unable to open key file skd://drm", "drmProtected"),
      (
        "[https @ 0x123] HTTP error 403 Forbidden",
        "accessForbidden403",
      ),
    ] {
      let events = [line, "ERROR: ffmpeg exited with code 8"]
        .iter()
        .filter_map(|line| parser.parse_fetch_line(line, false))
        .collect();
      assert_eq!(
        download_failure_diagnostics(events).last().unwrap().code,
        code
      );
    }
    let events = [
      "[https @ 0x123] HTTP error 403 Forbidden",
      "ERROR: No space left on device",
      "ERROR: ffmpeg exited with code 8",
    ]
    .iter()
    .filter_map(|line| parser.parse_fetch_line(line, false))
    .collect();
    assert_eq!(
      download_failure_diagnostics(events).last().unwrap().code,
      "diskFull"
    );
  }

  #[test]
  fn vimeo_retry_preserves_the_first_error_when_chrome_is_unavailable() {
    let mut input = ResolvedInput::new("https://player.vimeo.com/video/123");
    let parser = YtdlpErrorParser::new("id", "group", DiagnosticMatcher::shared().unwrap());
    let first = "ERROR: [vimeo] 123: Unable to download webpage: HTTP Error 401: Unauthorized";
    let mut diagnostics = parser.parse_lines(vec![first], false);
    let original = diagnostics[0].raw.clone();
    let mut saved = None;
    assert!(retry_vimeo(&mut input, first, &mut diagnostics, &mut saved));
    let unavailable = "ERROR: Impersonate target chrome is not available";
    diagnostics = parser.parse_lines(vec![unavailable], false);
    assert!(!retry_vimeo(
      &mut input,
      unavailable,
      &mut diagnostics,
      &mut saved
    ));
    assert_eq!(diagnostics[0].raw, original);
    assert!(!retry_vimeo(
      &mut input,
      first,
      &mut diagnostics,
      &mut saved
    ));
  }

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
