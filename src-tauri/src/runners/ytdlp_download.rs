use crate::download_records::DownloadRecords;
use crate::logging::LogStoreState;
use crate::models::download::DownloadSection;
use crate::models::{MediaDiagnosticPayload, MediaFatalPayload, ProgressEvent, TrackType};
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
/// Matching saved queue IDs may adopt verified partials after a restart.
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
      match builder.create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
          let valid = fs::symlink_metadata(&path)?.file_type().is_dir()
            && read_partial_owner(&path).is_some_and(|owner| {
              owner.group == group && owner.item == item && owner.state != PartialState::Discarded
            });
          if !valid {
            return Err(std::io::Error::other("Unverified partial directory"));
          }
        }
        Err(error) => return Err(error),
      }
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

fn preserve_numbering(
  path: &std::path::Path,
  context: &mut crate::runners::template_context::TemplateContext,
) -> Result<(), String> {
  let path = path.join("numbering.json");
  let keys = ["autonumber", "playlist_autonumber"];
  if path.exists() {
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| "Invalid saved numbering")?;
    if !metadata.is_file() || metadata.len() > 1024 {
      return Err("Invalid saved numbering".into());
    }
    let stored: std::collections::HashMap<String, String> =
      serde_json::from_slice(&std::fs::read(&path).map_err(|_| "Cannot read saved numbering")?)
        .map_err(|_| "Invalid saved numbering")?;
    for key in keys {
      if let Some(value) = stored.get(key).filter(|value| value.parse::<u64>().is_ok()) {
        context.values.insert(key.into(), value.clone());
      }
    }
  } else {
    let stored: std::collections::HashMap<_, _> = keys
      .into_iter()
      .filter_map(|key| context.values.get(key).map(|value| (key, value)))
      .collect();
    let bytes = serde_json::to_vec(&stored).map_err(|_| "Cannot save numbering")?;
    std::fs::write(&path, bytes).map_err(|_| "Cannot save numbering")?;
  }
  Ok(())
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
    std::fs::create_dir_all(root).map_err(|_| "Could not prepare download folder")?;
    let canonical_root = root
      .canonicalize()
      .map_err(|_| "Could not resolve download folder")?;
    if entries
      .get(&key)
      .is_some_and(|partial| partial.path.parent() != Some(canonical_root.as_path()))
    {
      if let Some(previous) = entries.remove(&key) {
        previous.discard();
      }
    }
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

/// Only verified ownership records may authorize removal; saved identities remain resumable.
fn stale_partials(
  roots: &[std::path::PathBuf],
  saved: &[(String, String)],
) -> Vec<std::path::PathBuf> {
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
      let owned = read_partial_owner(&entry.path()).is_some_and(|owner| {
        (owner.state == PartialState::Discarded
          || !saved
            .iter()
            .any(|(group, item)| group == &owner.group && item == &owner.item))
          && name == partial_name(&owner.group, &owner.item)
      });
      // Only saved queue identities can adopt a previous session’s partials.
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

pub(crate) fn sweep_download_partials(
  app: &AppHandle,
  mut saved: Vec<(String, String)>,
  extra_roots: Vec<String>,
) {
  static STARTUP_SWEEP: std::sync::Once = std::sync::Once::new();
  // Never sweep again after this session can have detached cancellation leases.
  STARTUP_SWEEP.call_once(|| {
    let runner = YtdlpRunner::new(app);
    let mut roots = vec![
      runner.output_root(&TrackType::Video, None),
      runner.output_root(&TrackType::Audio, None),
    ];
    if let Some(root) = app
      .state::<SharedConfig>()
      .load()
      .output
      .download_dir
      .as_ref()
    {
      roots.push(std::path::PathBuf::from(root));
    }
    roots.extend(extra_roots.into_iter().map(std::path::PathBuf::from));
    // The same lock protects registration: a live writer is never an orphan, including
    // downloads started by another window while the frontend restores its snapshot.
    let live = PARTIAL_DOWNLOADS
      .0
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    saved.extend(live.keys().cloned());
    crate::runners::gallery_files::sweep_image_staging(&roots);
    remove_stale_partials(stale_partials(&roots, &saved));
  });
}

pub(crate) fn discard_group(app: &AppHandle, group: &str, additional_roots: &[String]) {
  PARTIAL_DOWNLOADS.discard(group);
  let cfg = app.state::<crate::SharedConfig>().load();
  let prefs = app.state::<crate::SharedPreferences>().load();
  let roots = [
    cfg.output.download_dir.as_deref(),
    prefs.paths.video_download_dir.as_deref(),
    prefs.paths.audio_download_dir.as_deref(),
  ];
  for root in roots
    .into_iter()
    .flatten()
    .chain(additional_roots.iter().map(String::as_str))
  {
    let Ok(entries) = std::fs::read_dir(root) else {
      continue;
    };
    for entry in entries.flatten() {
      if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
        continue;
      }
      let Some(owner) = read_partial_owner(&entry.path()) else {
        continue;
      };
      if owner.group != group
        || entry.file_name() != std::ffi::OsStr::new(&partial_name(group, &owner.item))
      {
        continue;
      }
      // Active writers retain their Arc lease and remove the discarded directory on exit.
      if owner.state == PartialState::Discarded {
        continue;
      }
      if let Ok(partial) = DownloadPartial::open(std::path::Path::new(root), group, &owner.item) {
        partial.discard();
      }
    }
  }
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
    .position(|event| {
      matches!(
        event.code.as_str(),
        "drmProtected" | "signInRequired" | "legalRestriction451" | "rateLimited429"
      )
    })
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

fn refetch_after_failure(
  state: &mut super::retry_policy::RetryState,
  url: &str,
  diagnostics: &[crate::models::error::DiagnosticEvent],
  aria: bool,
  cookies: bool,
) -> bool {
  diagnostics.iter().any(|event| {
    state.refetch(url, &event.code, &event.message, aria, cookies) == RetryAction::Refetch
  })
}

fn impersonation_response(input: &ResolvedInput, line: &str) -> Option<&'static str> {
  let lower = line.to_ascii_lowercase();
  if input.impersonation_unavailable(line) {
    Some("Impersonate target is not available")
  } else if lower.contains("http error 403")
    && reqwest::Url::parse(&input.url).ok().is_some_and(|url| {
      url.host_str().and_then(crate::sites::site_of) == Some(crate::sites::SiteId::Rumble)
    })
  {
    Some("HTTP Error 403")
  } else if lower.contains("http error 401") {
    Some("HTTP Error 401")
  } else {
    None
  }
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
  } else if input.retry_vimeo_401(stderr) || input.retry_rumble_403(stderr) {
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

fn instagram_post(url: &str) -> bool {
  reqwest::Url::parse(url).is_ok_and(|url| {
    url.host_str().and_then(crate::sites::site_of) == Some(crate::sites::SiteId::Instagram)
  })
}

/// yt-dlp's Instagram post extractor sees only a post's videos (none of a mixed carousel's photos,
/// and often nothing signed out). gallery-dl reads the whole post, with the site's login if any.
/// Gallery-delegated children (`resume_parent`) and partial selections never go back to it.
fn instagram_needs_gallery(
  entry: &DownloadEntry,
  input: &ResolvedInput,
  diagnostics: &[crate::models::error::DiagnosticEvent],
) -> bool {
  entry.resume_parent.is_none()
    && instagram_post(&input.url)
    && entry
      .attachment_ids
      .as_ref()
      .is_none_or(|ids| ids.len() >= entry.attachments.len())
    && diagnostics.iter().any(|event| {
      matches!(event.level, crate::models::error::DiagnosticLevel::Error)
        && event
          .component
          .as_deref()
          .is_some_and(|c| c.eq_ignore_ascii_case("instagram"))
        && event
          .message
          .to_ascii_lowercase()
          .contains("no video formats found")
    })
}

/// The gallery's video children reuse this item's id: they must open a fresh partial directory,
/// not adopt the failed yt-dlp one, which the first child's completion would mark discarded
/// while this attempt still holds it.
fn release_partial_for_handoff(
  partials: &PartialDownloads,
  group: &str,
  item: &str,
  held: Option<std::sync::Arc<DownloadPartial>>,
) {
  partials.complete(group, item);
  drop(held);
}

/// The login the gallery retry starts with. The row's remembered choice is keyed by the post's
/// host, so a short link looks it up again under instagram.com. The anonymous marker from an
/// anonymous fetch win is dropped so the site's own session and automatic login stay usable.
fn handoff_browser(
  entry: &DownloadEntry,
  original_url: &str,
  current: Option<String>,
) -> Option<String> {
  if instagram_post(original_url) {
    current
  } else {
    super::login::row_browser(&entry.group_id, &entry.id, &entry.url)
  }
  .filter(|browser| browser != super::login::ANONYMOUS_BROWSER)
}

/// One retry of the whole post through gallery-dl; its own login flow and messages take over.
async fn retry_instagram_with_gallery(
  app: AppHandle,
  mut entry: DownloadEntry,
  login: &LoginSession,
  input: &ResolvedInput,
  held: Option<std::sync::Arc<DownloadPartial>>,
) -> Result<(), YtdlpDownloadError> {
  tracing::info!("Instagram post had no video formats; retrying through gallery-dl");
  release_partial_for_handoff(&PARTIAL_DOWNLOADS, &entry.group_id, &entry.id, held);
  // A short link (v.gd, is.gd) has been resolved: gallery-dl and the login lookup need the post.
  let original_url = std::mem::replace(&mut entry.url, input.url.clone());
  // yt-dlp's attachment list holds only the videos it saw; gallery-dl lists the whole post.
  entry.fetch_engine = Some(crate::models::parsed::FetchEngine::Gallery);
  entry.attachments.clear();
  entry.attachment_ids = None;
  entry.media_kind = None;
  entry.login_browser = handoff_browser(&entry, &original_url, login.browser.clone());
  entry.login_was_accepted = entry.login_browser.is_some();
  Box::pin(crate::runners::gallery::run_media_download(app, entry)).await
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
  })
}

async fn snapchat_snapshot<T: crate::resolve::http::Transport>(
  source: &str,
  bytes: &[u8],
  transport: Option<&T>,
) -> Option<serde_json::Value> {
  if !snapchat_media_page(source) || bytes.len() > 8 * 1024 * 1024 {
    return None;
  }
  let mut info: serde_json::Value = serde_json::from_slice(bytes).ok()?;
  if !info.is_object() {
    return None;
  }
  fn known_extension(ext: &str) -> bool {
    crate::media_types::ext_in(crate::media_types::MEDIA_EXTS, ext)
      || crate::parsers::gallery::image_extension(ext)
      || matches!(ext, "none" | "m3u8" | "mpd")
  }
  fn urls(value: &serde_json::Value, found: &mut std::collections::BTreeSet<String>) {
    if let Some(raw) = value["url"].as_str() {
      if reqwest::Url::parse(raw).is_ok_and(|url| {
        url
          .host_str()
          .is_some_and(|host| crate::sites::host_is(host, "sc-cdn.net"))
      }) && !value["ext"].as_str().is_some_and(known_extension)
      {
        found.insert(raw.into());
      }
    }
    for key in [
      "formats",
      "requested_formats",
      "requested_downloads",
      "entries",
      "thumbnails",
    ] {
      if let Some(items) = value[key].as_array() {
        for item in items {
          urls(item, found);
        }
      }
    }
  }
  let mut candidates = std::collections::BTreeSet::new();
  urls(&info, &mut candidates);
  let mut types = std::collections::HashMap::new();
  if let Some(transport) = transport.filter(|_| candidates.len() <= 100) {
    // A failed request or the overall sniff deadline must not discard usable metadata.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(20), async {
      for raw in candidates {
        let Ok(url) = reqwest::Url::parse(&raw) else {
          continue;
        };
        let Ok(response) = transport.range(&url, 4096).await else {
          continue;
        };
        if !matches!(response.status, 200 | 206) {
          continue;
        }
        let mime =
          crate::media_types::base_mime(response.content_type.as_deref().unwrap_or_default());
        let ext = if response.body.get(4..8) == Some(b"ftyp") {
          "mp4"
        } else if response.body.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
          "webm"
        } else if response.body.starts_with(&[0xff, 0xd8, 0xff]) {
          "jpg"
        } else if response.body.starts_with(b"\x89PNG\r\n\x1a\n") {
          "png"
        } else {
          match mime.as_str() {
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            "image/jpeg" => "jpg",
            "image/png" => "png",
            _ => continue,
          }
        };
        types.insert(raw, ext);
      }
    })
    .await;
  }
  fn apply(
    value: &mut serde_json::Value,
    types: &std::collections::HashMap<String, &str>,
    thumbnail: bool,
  ) {
    if let Some(url) = value["url"].as_str().filter(|raw| {
      reqwest::Url::parse(raw).is_ok_and(|url| {
        url
          .host_str()
          .is_some_and(|host| crate::sites::host_is(host, "sc-cdn.net"))
      })
    }) {
      let ext = types
        .get(url)
        .copied()
        .unwrap_or(if thumbnail { "jpg" } else { "mp4" });
      for key in ["ext", "video_ext"] {
        if (key == "ext" || value.get(key).is_some())
          && !value[key].as_str().is_some_and(known_extension)
        {
          value[key] = serde_json::json!(ext);
        }
      }
    }
    for key in [
      "formats",
      "requested_formats",
      "requested_downloads",
      "entries",
      "thumbnails",
    ] {
      if let Some(items) = value.get_mut(key).and_then(serde_json::Value::as_array_mut) {
        for item in items {
          apply(item, types, key == "thumbnails");
        }
      }
    }
  }
  apply(&mut info, &types, false);
  Some(info)
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
  let playlist_child = entry.is_playlist_child();
  let _slot =
    match super::reliability::playlist_slot(&entry.url, playlist_child, &mut cancel_rx).await {
      Ok(slot) => slot,
      Err(()) => return Ok(()),
    };
  let mut refetch = super::retry_policy::RetryState::default();
  let recovery_started = tokio::time::Instant::now();
  let mut recovery = super::reliability::Recovery::default();
  let mut input = match entry.take_fetch_input() {
    Some(input) => input,
    None => {
      let resolution = loop {
        let resolution = resolve::resolve(&entry.url).await;
        if matches!(
          resolution,
          Resolution::Failed {
            failure: resolve::Failure::Network | resolve::Failure::Timeout
          }
        ) {
          if let Some(delay) = recovery.network_delay(recovery_started.elapsed()) {
            if !super::reliability::wait(&app, &entry, &mut cancel_rx, delay, false).await {
              return Ok(());
            }
            continue;
          }
        }
        break resolution;
      };
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
    loop {
      let delay = super::reliability::host_delay(&entry.url);
      if delay.is_zero() {
        break;
      }
      if !super::reliability::wait(&app, &entry, &mut cancel_rx, delay, true).await {
        return Ok(());
      }
    }
    crate::events::emit(
      &app,
      crate::events::MEDIA_PROGRESS_STAGE,
      crate::models::MediaProgressStage {
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        stage: crate::models::ProgressStage::Initializing,
      },
    );
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
        .with_args(super::reliability::playlist_args(playlist_child))
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
            if super::reliability::rate_limited(&stderr) {
              if let Some(delay) = recovery.busy_delay() {
                super::reliability::cool_host(&entry.url, delay);
                continue 'attempt;
              }
              super::reliability::fatal(&app, &entry, "rateLimited429");
              return Err(YtdlpDownloadError::NonZeroExit(output.code));
            }
            if super::reliability::transient(&stderr) {
              if let Some(delay) = recovery.network_delay(recovery_started.elapsed()) {
                if !super::reliability::wait(&app, &entry, &mut cancel_rx, delay, false).await {
                  return Ok(());
                }
                continue 'attempt;
              }
              super::reliability::fatal(&app, &entry, "networkTimeout");
              return Err(YtdlpDownloadError::NonZeroExit(output.code));
            }
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
            if instagram_needs_gallery(&entry, &input, &diagnostics) {
              return retry_instagram_with_gallery(app, entry, &login, &input, None).await;
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
            if refetch_after_failure(
              &mut refetch,
              &entry.url,
              &diagnostics,
              aria.is_some(),
              login.uses_cookies(),
            ) {
              if !super::reliability::wait(
                &app,
                &entry,
                &mut cancel_rx,
                super::reliability::refetch_delay(&entry.url),
                true,
              )
              .await
              {
                return Ok(());
              }
              continue 'attempt;
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
      .with_args([
        "--continue",
        "--retry-sleep",
        "http:exp=1:20",
        "--retry-sleep",
        "fragment:exp=1:20",
        "--extractor-retries",
        "2",
      ])
      .with_network_args(entry.overrides.as_ref())
      .with_auth_args(
        entry.overrides.as_ref(),
        login.browser.as_deref(),
        &entry.url,
      )
      .with_cookie_access(Some(&cookies))
      .with_args(super::reliability::playlist_args(playlist_child))
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
    keep_image_container(&entry.url, entry.media_kind, &mut output_args);
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
    preserve_numbering(&temporary.path, &mut entry.template_context)
      .map_err(YtdlpDownloadError::RunnerError)?;
    let _disk_guard = crate::scheduling::disk_space::plugin(&output_root)
      .map_err(YtdlpDownloadError::RunnerError)?;
    let runner = runner.with_args([
      "--plugin-dirs".into(),
      _disk_guard.path.to_string_lossy().into_owned(),
      "--use-postprocessor".into(),
      "NezzelDisk:when=before_dl".into(),
    ]);
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
      let settings = app.state::<crate::SharedConfig>().load();
      let network = super::override_resolver::resolve_with_patch(
        &settings.network,
        entry
          .overrides
          .as_ref()
          .and_then(|overrides| overrides.network.as_ref()),
      );
      let snapshot = if output.code == 0 {
        snapchat_snapshot(
          &entry.url,
          &output.stdout,
          (network.enable_proxy != Some(true)).then_some(&crate::resolve::http::PublicHttp),
        )
        .await
      } else {
        None
      };
      if let Some(info) = snapshot {
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
    let mut network_failure = false;
    let mut busy_failure = false;
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

    let mut watchdog = super::reliability::StallWatch::new(recovery_started.elapsed());
    if let Some(bytes) = super::reliability::partial_bytes(&temporary.path) {
      watchdog.observe_file_bytes(bytes, recovery_started.elapsed());
    }
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
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
      tokio::select! {
        biased;
        _ = tick.tick() => {
          if is_cancelled_now(&cancel_rx) { let _ = child.kill_tree(); return Ok(()); }
          // External downloaders may not print RAW progress. Their temporary files are
          // authoritative byte movement; metadata/log activity never feeds the watchdog.
          if let Some(bytes) = super::reliability::partial_bytes(&temporary.path) {
            if watchdog.observe_file_bytes(bytes, recovery_started.elapsed()) { recovery.progressed(); network_failure = false; busy_failure = false; }
          }
          if watchdog.extraction_expired(recovery_started.elapsed()) || watchdog.stalled(recovery_started.elapsed()) {
            let _ = child.kill_tree();
            // Wait for the writer to exit before reusing its .part files.
            let reaped = tokio::time::timeout(std::time::Duration::from_secs(5), async {
              while let Some(event) = rx.recv().await {
                if matches!(event, YtdlpCommandEvent::Terminated(_)) { return true; }
              }
              false
            }).await.unwrap_or(false);
            if is_cancelled_now(&cancel_rx) { return Ok(()); }
            if watchdog.extraction_expired(recovery_started.elapsed()) {
              super::reliability::fatal(&app, &entry, "networkTimeout");
              return Err(YtdlpDownloadError::RunnerError("networkTimeout".into()));
            }
            if reaped && recovery.restart_stall() { continue 'attempt; }
            super::reliability::fatal(&app, &entry, "downloadStalled");
            return Err(YtdlpDownloadError::RunnerError("downloadStalled".into()));
          }
        }
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
              if watchdog.observe(&line_str, recovery_started.elapsed()) { recovery.progressed(); network_failure = false; busy_failure = false; }
              network_failure |= super::reliability::transient(&line_str);
              busy_failure |= super::reliability::rate_limited(&line_str);
              if let Some(stage) = parse_progress_line(&line_str, &mut progress_parser, &app) { watchdog.stage(&stage); }
              if let Some(event) = error_parser.parse_fetch_line(&line_str, login.has_session) {
                if matches!(event.level, crate::models::error::DiagnosticLevel::Error) {
                  last_error = Some(event.raw.clone());
                }
                if diagnostics.len() < 64 { diagnostics.push(event); }
              }
            }
            YtdlpCommandEvent::Stderr(line) => {
              #[cfg(feature = "link-matrix")]
              crate::link_matrix::observe_gallery_stderr(&line);
              let line_str = String::from_utf8_lossy(&line);
              unsupported |= resolve::unsupported_url(&line_str);
              // Classify without retaining page contents or potentially sensitive URLs.
              let lower = line_str.to_ascii_lowercase();
              if let Some(response) = impersonation_response(&input, &line_str) { vimeo_response = response; }
              expired |= lower.contains("http error 403") || lower.contains("http error 404");
              login_reason = login_reason.or_else(|| error_parser.login_reason(&line_str));
              transport_denied |= aria.is_some() && aria_denied(&line_str);
              media_expired |= selected_media_expired(&entry, &input, &line_str);
              cookie_failure = cookie_failure.combine(error_parser.cookie_fetch_failure(&line_str, login.has_session));
              store_log_line(&line_str, &entry, log_state, &app);
              if watchdog.observe(&line_str, recovery_started.elapsed()) { recovery.progressed(); network_failure = false; busy_failure = false; }
              network_failure |= super::reliability::transient(&line_str);
              busy_failure |= super::reliability::rate_limited(&line_str);
              if let Some(stage) = parse_progress_line(&line_str, &mut progress_parser, &app) { watchdog.stage(&stage); }
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
                    Err(code) if code == "diskFull" || code == "diskSpaceUnavailable" => {
                      super::reliability::fatal(&app, &entry, &code);
                      return Err(YtdlpDownloadError::RunnerError(code));
                    }
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
              if diagnostics.iter().any(|event| event.raw.contains("diskFull")) || last_error.as_ref().is_some_and(|e| e.contains("diskFull")) {
                super::reliability::fatal(&app, &entry, "diskFull");
                return Err(YtdlpDownloadError::RunnerError("diskFull".into()));
              }
              if diagnostics.iter().any(|event| event.raw.contains("diskSpaceUnavailable")) {
                super::reliability::fatal(&app, &entry, "diskSpaceUnavailable");
                return Err(YtdlpDownloadError::RunnerError("diskSpaceUnavailable".into()));
              }
              if busy_failure {
                if let Some(delay) = recovery.busy_delay() {
                  super::reliability::cool_host(&entry.url, delay);
                  continue 'attempt;
                }
                super::reliability::fatal(&app, &entry, "rateLimited429");
                return Err(YtdlpDownloadError::NonZeroExit(term.code.unwrap_or(1)));
              }
              if network_failure {
                if let Some(delay) = recovery.network_delay(recovery_started.elapsed()) {
                  if !super::reliability::wait(&app, &entry, &mut cancel_rx, delay, false).await { return Ok(()); }
                  continue 'attempt;
                }
                super::reliability::fatal(&app, &entry, "networkTimeout");
                return Err(YtdlpDownloadError::NonZeroExit(term.code.unwrap_or(1)));
              }
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
              if instagram_needs_gallery(&entry, &input, &diagnostics) { return retry_instagram_with_gallery(app, entry, &login, &input, Some(temporary)).await; }
              login.record_failure(&diagnostics, false);
              match login.failed(&app, LoginRequest::Download(entry.login_request_entry()), login_reason.as_deref(), transport_denied, is_cancelled_now(&cancel_rx)) {
                RetryAction::Native | RetryAction::Browser | RetryAction::Anonymous => continue 'attempt,
                RetryAction::AskLogin | RetryAction::Pending => return Err(YtdlpDownloadError::LoginPending),
                RetryAction::Stop | RetryAction::Refetch => {}
              }
              if refetch_after_failure(&mut refetch, &entry.url, &diagnostics, aria.is_some(), login.uses_cookies()) {
                if !super::reliability::wait(&app, &entry, &mut cancel_rx, super::reliability::refetch_delay(&entry.url), true).await { return Ok(()); }
                continue 'attempt;
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
    crate::scheduling::disk_space::check(path, std::fs::metadata(path).ok().map(|m| m.len()))?;
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

fn parse_progress_line(
  line: &str,
  progress_parser: &mut YtdlpProgressParser,
  app: &AppHandle,
) -> Option<crate::models::ProgressStage> {
  let mut stage = None;
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
        stage = Some(progress.stage.clone());
        crate::events::emit(app, crate::events::MEDIA_PROGRESS_STAGE, progress);
      }
    }
  }
  stage
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

/// yt-dlp's generic and Wikimedia extractors report a direct still as a "video" with ext jpg;
/// a video container option would remux it into an .mp4 instead of keeping the image file.
/// The extracted kind decides first (a video or audio result keeps its merge/remux whatever its
/// URL ends in); the suffix only identifies a still among results with no media streams.
fn keep_image_container(
  url: &str,
  kind: Option<crate::models::parsed::MediaKind>,
  args: &mut Vec<String>,
) {
  use crate::models::parsed::MediaKind;
  if !matches!(kind, None | Some(MediaKind::Image)) {
    return;
  }
  let is_image = reqwest::Url::parse(url).is_ok_and(|url| {
    let path =
      String::from_utf8_lossy(&crate::files::names::percent_bytes(url.path())).into_owned();
    path
      .rsplit('/')
      .next()
      .and_then(|name| name.rsplit_once('.'))
      .is_some_and(|(_, ext)| crate::parsers::gallery::image_extension(ext))
  });
  if !is_image {
    return;
  }
  let mut kept = Vec::with_capacity(args.len());
  let mut pending = std::mem::take(args).into_iter();
  while let Some(arg) = pending.next() {
    if matches!(
      arg.as_str(),
      "--merge-output-format" | "--remux-video" | "--recode-video" | "--postprocessor-args"
    ) {
      pending.next();
    } else {
      kept.push(arg);
    }
  }
  *args = kept;
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

  #[test]
  fn restored_partials_keep_bytes_and_numbering_but_never_adopt_a_symlink() {
    let output = super::super::gallery_files::Staging::new().unwrap();
    let (_running, cancel) = watch::channel(true);
    let original = PartialDownloads::default();
    let partial = original
      .get("saved", "row", &cancel, &output.path)
      .unwrap()
      .unwrap();
    std::fs::write(partial.path.join("clip.part"), b"downloaded bytes").unwrap();
    let mut context = crate::runners::template_context::TemplateContext {
      values: Default::default(),
    };
    context.values.insert("autonumber".into(), "17".into());
    preserve_numbering(&partial.path, &mut context).unwrap();
    drop(partial);
    drop(original);
    let restored = PartialDownloads::default();
    let partial = restored
      .get("saved", "row", &cancel, &output.path)
      .unwrap()
      .unwrap();
    context.values.insert("autonumber".into(), "1".into());
    preserve_numbering(&partial.path, &mut context).unwrap();
    assert_eq!(context.values["autonumber"], "17");
    assert_eq!(
      std::fs::read(partial.path.join("clip.part")).unwrap(),
      b"downloaded bytes"
    );
    #[cfg(unix)]
    {
      let link = output.path.join(partial_name("unsafe", "row"));
      std::os::unix::fs::symlink(&partial.path, &link).unwrap();
      assert!(restored
        .get("unsafe", "row", &cancel, &output.path)
        .is_err());
    }
    restored.discard("saved");
    drop(partial);
  }

  #[tokio::test]
  async fn startup_sweep_deletes_orphans_and_preserves_saved_and_unverified_paths() {
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
        if matches!(state, PartialState::Discarded | PartialState::Running) {
          stale.push(partial.path.clone());
        }
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
    let paths = stale_partials(
      &[root.clone(), root.join("."), root.clone()],
      &[
        ("group".into(), "paused".into()),
        ("group".into(), "queued".into()),
      ],
    );
    assert_eq!(paths.len(), stale.len());
    for path in &stale {
      assert!(paths.contains(path));
      assert!(
        path.join("video.part").exists(),
        "classification must not delete data"
      );
    }
    // Discarded directories cannot be adopted while their deletion is pending.
    let restarted = PartialDownloads::default();
    assert!(restarted.get("group", "discarded", &cancel, &root).is_err());
    assert!(restarted.get("group", "paused", &cancel, &root).is_ok());
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
    assert!(fresh.path.join("video.part").exists());
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
  fn direct_still_images_are_not_remuxed_into_mp4() {
    use crate::models::parsed::MediaKind;
    let original: Vec<String> = [
      "--output-na-placeholder",
      "",
      "--merge-output-format",
      "mp4",
      "--remux-video",
      "mp4",
      "--postprocessor-args",
      "ffmpeg:-movflags +faststart",
      "--add-metadata",
    ]
    .map(String::from)
    .into();
    for url in [
      "https://commons.wikimedia.org/wiki/Special:FilePath/Fronalpstock_big.jpg",
      "https://commons.wikimedia.org/wiki/File:Fronalpstock_big.jpg",
    ] {
      let mut args = original.clone();
      keep_image_container(url, None, &mut args);
      assert_eq!(
        args,
        ["--output-na-placeholder", "", "--add-metadata"],
        "{url}"
      );
    }
    for (url, kind) in [
      ("https://example.com/clip.mp4", None),
      ("https://example.com/clip.gif", Some(MediaKind::Video)),
      ("https://example.com/slug.jpg", Some(MediaKind::Video)),
    ] {
      let mut args = original.clone();
      keep_image_container(url, kind, &mut args);
      assert_eq!(args, original, "{url}");
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
    let vimeo = YtdlpErrorParser::new("id", "group", DiagnosticMatcher::shared().unwrap())
      .with_source_url("https://vimeo.com/76979871");
    let events = [
      "[hls @ 0x123] Unable to open key file skd://drm",
      "ERROR: ffmpeg exited with code 8",
    ]
    .iter()
    .filter_map(|line| vimeo.parse_fetch_line(line, false))
    .collect();
    assert_eq!(
      download_failure_diagnostics(events).last().unwrap().code,
      "signInRequired"
    );
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
  fn playlist_pacing_uses_fetch_identity_not_template_metadata() {
    for (playlist_child, context, expected) in [
      (
        false,
        serde_json::json!({"playlist_id":"incidental"}),
        false,
      ),
      (true, serde_json::json!({}), true),
    ] {
      let item = serde_json::from_value(serde_json::json!({
        "id":"video", "url":"https://youtube.com/watch?v=1", "format":{"trackType":"both"},
        "playlistChild":playlist_child, "templateContext":{"values":context}
      }))
      .unwrap();
      let entry = DownloadEntry::from((item, "pacing".into()));
      assert_eq!(
        !super::super::reliability::playlist_args(entry.is_playlist_child()).is_empty(),
        expected
      );
    }
  }

  #[test]
  fn configured_aria_blocks_youtube_refetch_in_both_failure_paths() {
    let parser = YtdlpErrorParser::new("id", "group", DiagnosticMatcher::shared().unwrap());
    let diagnostics = parser.parse_lines(vec!["ERROR: HTTP Error 403: Forbidden"], false);
    assert!(!refetch_after_failure(
      &mut Default::default(),
      "https://youtube.com/watch?v=1",
      &diagnostics,
      true,
      false
    ));
    let mut state = Default::default();
    assert!(refetch_after_failure(
      &mut state,
      "https://youtube.com/watch?v=1",
      &diagnostics,
      false,
      false
    ));
    assert!(!refetch_after_failure(
      &mut state,
      "https://youtube.com/watch?v=1",
      &diagnostics,
      false,
      false
    ));
  }

  #[test]
  fn rumble_forbidden_tracking_does_not_replace_vimeo_unauthorized() {
    for url in ["https://vimeo.com/123", "https://rumble.com/embed/123"] {
      let mut input = ResolvedInput::new(url);
      let parser = YtdlpErrorParser::new("id", "group", DiagnosticMatcher::shared().unwrap());
      let lines = [
        "ERROR: HTTP Error 401: Unauthorized",
        "ERROR: HTTP Error 403: Forbidden",
      ];
      let response = lines
        .iter()
        .filter_map(|line| impersonation_response(&input, line))
        .next_back()
        .unwrap();
      let mut diagnostics = parser.parse_lines(lines.to_vec(), false);
      assert!(
        retry_vimeo(&mut input, response, &mut diagnostics, &mut None),
        "{url}: {response}"
      );
      assert!(input.args().contains(&"chrome"));
    }
    let input = ResolvedInput::new("https://rumble.com.evil.test/video");
    assert_eq!(
      impersonation_response(&input, "ERROR: HTTP Error 403"),
      None
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
  fn instagram_post_without_video_formats_goes_to_gallery_once() {
    let parser = YtdlpErrorParser::new("i", "g", DiagnosticMatcher::shared().unwrap());
    let item = serde_json::from_value(serde_json::json!({"id":"post","url":"https://v.gd/LfQhkN","format":{"trackType":"both"},"templateContext":{"values":{}}})).unwrap();
    let entry = DownloadEntry::from((item, "ig".into()));
    let mut child = entry.clone();
    child.resume_parent = Some(Box::new(entry.clone()));
    // yt-dlp's fetch lists the post's one video as a selected attachment (the owner's row).
    let mut whole = entry.clone();
    whole.attachments = serde_json::from_value(serde_json::json!([{"id":"v1","url":"https://www.instagram.com/p/DfxRiPZIXJo/","kind":"video"}])).unwrap();
    whole.attachment_ids = Some(vec!["v1".into()]);
    let mut chosen = whole.clone();
    chosen.attachments.push(chosen.attachments[0].clone());
    let post = ResolvedInput::new("https://www.instagram.com/p/DfxRiPZIXJo/");
    for (entry, line, expected) in [
      (
        &whole,
        "ERROR: [Instagram] DfxRiPZIXJo: No video formats found!; please report this issue",
        true,
      ),
      (
        &entry,
        "ERROR: [Instagram] DfxRiPZIXJo: No video formats found!",
        true,
      ),
      (
        &child,
        "ERROR: [Instagram] DfxRiPZIXJo: No video formats found!",
        false,
      ),
      (
        &chosen,
        "ERROR: [Instagram] DfxRiPZIXJo: No video formats found!",
        false,
      ),
      (
        &entry,
        "ERROR: [youtube] abc: No video formats found!",
        false,
      ),
      (
        &entry,
        "WARNING: [Instagram] DfxRiPZIXJo: No video formats found!",
        false,
      ),
      (
        &entry,
        "ERROR: [Instagram] DfxRiPZIXJo: Restricted Video",
        false,
      ),
    ] {
      let diagnostics = parser.parse_lines(vec![line], false);
      assert_eq!(
        instagram_needs_gallery(entry, &post, &diagnostics),
        expected,
        "{line}"
      );
    }
    // The short link must have resolved to the post before gallery-dl gets it.
    let line = "ERROR: [Instagram] DfxRiPZIXJo: No video formats found!";
    let diagnostics = parser.parse_lines(vec![line], false);
    let unresolved = ResolvedInput::new("https://v.gd/LfQhkN");
    assert!(!instagram_needs_gallery(&entry, &unresolved, &diagnostics));
    // An anonymous fetch win leaves the "none" marker: the retry must not inherit it.
    let anonymous = Some(crate::runners::login::ANONYMOUS_BROWSER.to_owned());
    let mut direct = entry.clone();
    direct.url = "https://www.instagram.com/p/DfxRiPZIXJo/".into();
    assert_eq!(
      handoff_browser(&direct, &direct.url, anonymous.clone()),
      None
    );
    assert_eq!(
      handoff_browser(&direct, &direct.url, Some("chrome:Default".into())),
      Some("chrome:Default".into())
    );
    // A short link's session belonged to v.gd; only an instagram.com row choice may carry over.
    assert_eq!(
      handoff_browser(
        &direct,
        "https://v.gd/LfQhkN",
        Some("chrome:Default".into())
      ),
      None
    );
  }

  #[test]
  fn instagram_gallery_handoff_frees_the_item_partial_for_each_video_child() {
    let output = super::super::gallery_files::Staging::new().unwrap();
    let (_running, cancel) = watch::channel(true);
    // Without the release: child 1 adopts the held partial, completes, and child 2 is refused.
    let partials = PartialDownloads::default();
    let held = partials.get("g", "post", &cancel, &output.path).unwrap();
    let child = partials.get("g", "post", &cancel, &output.path).unwrap();
    partials.complete("g", "post");
    drop(child);
    assert!(partials.get("g", "post", &cancel, &output.path).is_err());
    drop(held);
    // With the release every video child opens and completes its own partial.
    let partials = PartialDownloads::default();
    let held = partials.get("g", "next", &cancel, &output.path).unwrap();
    release_partial_for_handoff(&partials, "g", "next", held);
    for _ in 0..2 {
      let child = partials.get("g", "next", &cancel, &output.path).unwrap();
      partials.complete("g", "next");
      drop(child);
    }
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

  #[tokio::test]
  async fn snapchat_recorded_snapshot_derives_format_for_profile_and_story_children() {
    struct Media;
    impl crate::resolve::http::Transport for Media {
      async fn get(
        &self,
        url: &reqwest::Url,
        _: usize,
      ) -> Result<crate::resolve::http::Response, crate::resolve::http::FetchError> {
        let image = url.path().contains(".256.");
        Ok(crate::resolve::http::Response {
          status: 206,
          location: None,
          content_length: None,
          content_type: Some(if image { "image/jpeg" } else { "video/mp4" }.into()),
          body: if image {
            vec![0xff, 0xd8, 0xff]
          } else {
            b"\x00\x00\x00\x18ftypmp42".to_vec()
          },
        })
      }
    }
    for source in [
      "https://www.snapchat.com/@nasa",
      "https://www.snapchat.com/add/nasa",
      "https://story.snapchat.com/s/story",
      "https://www.snapchat.com/@nasa/spotlight/id",
    ] {
      let info = snapchat_snapshot(
        source,
        include_bytes!("../parsers/fixtures/ytdlp-snapchat.json"),
        Some(&Media),
      )
      .await
      .unwrap();
      assert_eq!(
        (
          info["ext"].as_str(),
          info["formats"][0]["ext"].as_str(),
          info["thumbnails"][0]["ext"].as_str()
        ),
        (Some("mp4"), Some("mp4"), Some("jpg")),
        "{source}"
      );
    }
    assert!(snapchat_snapshot(
      "https://snapchat.com.example.org/video",
      include_bytes!("../parsers/fixtures/ytdlp-snapchat.json"),
      Some(&Media)
    )
    .await
    .is_none());
  }

  #[tokio::test(start_paused = true)]
  async fn snapchat_recorded_snapshot_defaults_when_sniff_is_unavailable() {
    struct Sniff(u8);
    impl crate::resolve::http::Transport for Sniff {
      async fn get(
        &self,
        url: &reqwest::Url,
        _: usize,
      ) -> Result<crate::resolve::http::Response, crate::resolve::http::FetchError> {
        if self.0 == 2 {
          return std::future::pending().await;
        }
        if self.0 == 1 && !url.path().contains(".256.") {
          return Ok(crate::resolve::http::Response {
            status: 206,
            location: None,
            content_length: None,
            content_type: Some("video/webm".into()),
            body: vec![0x1a, 0x45, 0xdf, 0xa3],
          });
        }
        Err(crate::resolve::http::FetchError::Network)
      }
    }
    for (transport, video_ext) in [
      (None, "mp4"),
      (Some(&Sniff(0)), "mp4"),
      (Some(&Sniff(1)), "webm"),
      (Some(&Sniff(2)), "mp4"),
    ] {
      let info = snapchat_snapshot(
        "https://www.snapchat.com/@nasa",
        include_bytes!("../parsers/fixtures/ytdlp-snapchat.json"),
        transport,
      )
      .await
      .expect("keep a usable snapshot without a sniff");
      assert_eq!(info["ext"], video_ext);
      assert_eq!(info["formats"][0]["ext"], video_ext);
      assert_eq!(info["thumbnails"][0]["ext"], "jpg");
    }
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
