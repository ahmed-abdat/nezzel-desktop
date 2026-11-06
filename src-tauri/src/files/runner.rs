//! Integration with existing metadata, group cancellation and progress events.
use super::{
  engine::{Engine, Event},
  probe::{self, Kind, Probe},
  state, Error, Result,
};
use crate::extract::{
  chain::StageResult,
  outcome::{Confidence, Outcome, StageId},
};
use crate::{
  download_records::DownloadRecords,
  models::{
    progress::{
      MediaDestination, MediaDestinationPath, MediaProgress, MediaProgressStage, ProgressCategory,
      ProgressStage,
    },
    MediaDiagnosticPayload, MediaFatalPayload, ParsedMedia, ParsedSingleVideo,
  },
  scheduling::{
    download_pipeline::DownloadEntry, fetch_pipeline::FetchEntry, group_state::subscribe_group,
  },
  SharedConfig,
};
use std::{
  collections::HashMap,
  path::PathBuf,
  sync::{LazyLock, Mutex},
  time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
struct Cached {
  group: String,
  identity: String,
  created: Instant,
  probe: Probe,
}
static PROBES: LazyLock<Mutex<HashMap<String, Cached>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
#[derive(Clone)]
struct Partial {
  engine: Engine,
  directory: PathBuf,
  identity: String,
  owner: String,
}
static PARTIALS: LazyLock<Mutex<HashMap<String, Vec<Partial>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
pub(crate) fn discard_group(group: &str) {
  let partials = PARTIALS
    .lock()
    .ok()
    .and_then(|mut p| p.remove(group))
    .unwrap_or_default();
  for partial in partials {
    tauri::async_runtime::spawn(async move {
      if let Err(error) = partial
        .engine
        .discard(partial.directory, partial.identity, partial.owner)
        .await
      {
        tracing::warn!(code = error.code(), "File partial cleanup failed");
      }
    });
  }
}
const TTL: Duration = Duration::from_secs(25);
pub(crate) fn forget(group: &str) {
  if let Ok(mut probes) = PROBES.lock() {
    probes.retain(|_, p| p.group != group);
  }
}
fn metadata(id: &str, url: &str, probe: &Probe) -> Result<ParsedSingleVideo> {
  serde_json::from_value(serde_json::json!({
    "id":id,"mediaKind":"file","url":url,"title":probe.name,"filesize":probe.size,
    "formats":[],"audioCodecs":[],"videoCodecs":[],"audioTracks":[],"videoTracks":[],"chapters":[],
    "subtitleInventory":{"manualLanguages":[],"autoLanguages":[]}
  }))
  .map_err(|_| Error::State)
}
/// Anonymous capture previews use the same file metadata shape as foreground reads.
pub(crate) async fn preview(url: &str, id: String) -> Result<serde_json::Value> {
  let (_active, cancel) = tokio::sync::watch::channel(true);
  let mut probe = Engine::new()?.probe(url, cancel).await?;
  let media = metadata(&id, url, &probe)?;
  probe.body.release();
  serde_json::to_value(media).map_err(|_| Error::State)
}

pub(crate) async fn info(app: &AppHandle, entry: &FetchEntry) -> StageResult {
  metadata_stage(app, entry, false).await
}
pub(crate) async fn after_unsupported(app: &AppHandle, entry: &FetchEntry) -> StageResult {
  metadata_stage(app, entry, true).await
}
async fn metadata_stage(app: &AppHandle, entry: &FetchEntry, unsupported: bool) -> StageResult {
  let start = Instant::now();
  let mut stage = StageResult::new(
    StageId::Files,
    if unsupported {
      Confidence::Low
    } else {
      Confidence::High
    },
  );
  let result = info_impl(app, entry, unsupported).await;
  if !*subscribe_group(&entry.group_id).borrow() {
    return stage.finish(Outcome::Cancelled, false, start);
  }
  match result {
    Some(Ok(media)) => {
      stage.media = Some(media);
      stage.finish(Outcome::Verified, false, start)
    }
    Some(Err(Error::Cancelled)) => stage.finish(Outcome::Cancelled, false, start),
    Some(Err(error)) => {
      let code = error.code();
      stage.record_code(code, "", false, start);
      stage
    }
    None if super::hosts::is_share(&entry.url) => {
      stage.record_code(Error::State.code(), "", false, start);
      stage
    }
    None => stage.finish(Outcome::NoMatch, false, start),
  }
}
async fn info_impl(
  app: &AppHandle,
  entry: &FetchEntry,
  unsupported: bool,
) -> Option<Result<ParsedMedia>> {
  let shared = super::hosts::is_share(&entry.url);
  if !(shared
    || probe::should_probe_metadata(&entry.url, entry.total, entry.format.is_some())
    || unsupported && entry.total <= 1 && entry.format.is_none() && probe::should_probe(&entry.url))
  {
    return None;
  }
  // A CONNECT proxy resolves destinations itself, so reqwest cannot enforce our public-IP
  // boundary through it. Keep existing media routing; File transfers fail explicitly on proxy.
  let cfg = app.state::<SharedConfig>().load();
  let network = crate::runners::override_resolver::resolve_with_patch(
    &cfg.network,
    entry.overrides.as_ref().and_then(|o| o.network.as_ref()),
  );
  if network.enable_proxy == Some(true) {
    return shared.then_some(Err(Error::Proxy));
  }
  {
    let mut cache = PROBES.lock().ok()?;
    cache.retain(|_, p| p.created.elapsed() < TTL);
    if cache.len() >= 4 {
      let key = cache
        .iter()
        .min_by_key(|(_, p)| p.created)
        .map(|(k, _)| k.clone());
      if let Some(key) = key {
        cache.remove(&key);
      }
    }
  }
  let cancel = subscribe_group(&entry.group_id);
  let mut probe = match probe_metadata_impl(
    &Engine::new().ok()?,
    &entry.url,
    cancel.clone(),
    unsupported,
  )
  .await?
  {
    Ok(p) => p,
    Err(error) => return Some(Err(error)),
  };
  if !*cancel.borrow() {
    return None;
  }
  let media = metadata(&entry.id, &entry.url, &probe).ok()?;
  probe.body.release();
  let created = Instant::now();
  PROBES.lock().ok()?.insert(
    entry.id.clone(),
    Cached {
      group: entry.group_id.clone(),
      identity: state::identity(&entry.url),
      created,
      probe,
    },
  );
  let id = entry.id.clone();
  tauri::async_runtime::spawn(async move {
    tokio::time::sleep(TTL).await;
    if let Ok(mut cache) = PROBES.lock() {
      if cache.get(&id).is_some_and(|p| p.created == created) {
        cache.remove(&id);
      }
    }
  });
  Some(Ok(ParsedMedia::Single(media)))
}
#[cfg(test)]
pub(crate) async fn probe_metadata(
  engine: &Engine,
  url: &str,
  cancel: tokio::sync::watch::Receiver<bool>,
) -> Option<Result<Probe>> {
  probe_metadata_impl(engine, url, cancel, false).await
}
async fn probe_metadata_impl(
  engine: &Engine,
  url: &str,
  cancel: tokio::sync::watch::Receiver<bool>,
  unsupported: bool,
) -> Option<Result<Probe>> {
  if super::hosts::is_share(url) {
    return Some(engine.probe(url, cancel).await);
  }
  match tokio::time::timeout(Duration::from_secs(5), engine.probe(url, cancel)).await {
    Ok(Ok(mut probe)) if probe.kind == Kind::File || unsupported && probe.kind == Kind::Unknown => {
      probe.confirm_unknown().ok()?;
      Some(Ok(probe))
    }
    _ => None,
  }
}
/// Subfolder of the download folder that file-share downloads go to (the inspector shows the same path).
pub(crate) const FILES_FOLDER: &str = "Files";

fn root(app: &AppHandle, entry: &DownloadEntry) -> Result<PathBuf> {
  if let Some(dir) = entry
    .overrides
    .as_ref()
    .and_then(|o| o.output.as_ref())
    .and_then(|o| o.download_dir.as_deref())
    .filter(|s| !s.is_empty())
  {
    return Ok(PathBuf::from(dir));
  }
  let cfg = app.state::<SharedConfig>().load();
  let root = match cfg.output.download_dir.as_deref() {
    Some(dir) if !dir.is_empty() => PathBuf::from(dir),
    _ => app
      .path()
      .download_dir()
      .map_err(|_| Error::Io)?
      .join("Nezzel"),
  };
  Ok(root.join(FILES_FOLDER))
}
pub(crate) async fn download(app: AppHandle, entry: DownloadEntry) -> Result<()> {
  let cancel = subscribe_group(&entry.group_id);
  if !*cancel.borrow() {
    return Ok(());
  }
  let result = attempt(&app, &entry, cancel.clone()).await;
  if let Err(error) = &result {
    if matches!(error, Error::Cancelled) || !*cancel.borrow() {
      return Ok(());
    }
    diagnostic(&app, &entry.group_id, &entry.id, error);
  }
  result
}
async fn attempt(
  app: &AppHandle,
  entry: &DownloadEntry,
  cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
  let config = app.state::<SharedConfig>().load();
  let network = crate::runners::override_resolver::resolve_with_patch(
    &config.network,
    entry.overrides.as_ref().and_then(|o| o.network.as_ref()),
  );
  if network.enable_proxy == Some(true) {
    return Err(Error::Proxy);
  }
  let dir = root(app, entry)?;
  let probe = PROBES
    .lock()
    .map_err(|_| Error::State)?
    .remove(&entry.id)
    .filter(|p| p.created.elapsed() < TTL && p.identity == state::identity(&entry.url))
    .map(|p| p.probe);
  crate::events::emit(
    app,
    crate::events::MEDIA_PROGRESS_STAGE,
    MediaProgressStage {
      id: entry.id.clone(),
      group_id: entry.group_id.clone(),
      stage: ProgressStage::Downloading,
    },
  );
  let engine = Engine::new()?.for_file_row().with_state_directory(
    app
      .path()
      .app_data_dir()
      .map_err(|_| Error::Io)?
      .join("file-engine"),
  );
  {
    // Registration and the cancellation recheck share the cleanup registry's mutex:
    // remove cannot miss a transfer that was between its first check and registration.
    let mut partials = PARTIALS.lock().map_err(|_| Error::State)?;
    if !*cancel.borrow() {
      return Err(Error::Cancelled);
    }
    let partial = Partial {
      engine: engine.clone(),
      directory: dir.clone(),
      identity: state::identity(&entry.url),
      owner: state::identity(&entry.id),
    };
    let group = partials.entry(entry.group_id.clone()).or_default();
    group.retain(|p| {
      p.identity != partial.identity || p.owner != partial.owner || p.directory != partial.directory
    });
    group.push(partial);
  }
  let path = engine
    .download(
      &entry.url,
      &dir,
      &entry.id,
      probe,
      cancel.clone(),
      &mut |event| {
        if !*cancel.borrow() {
          return;
        }
        if let Event::Progress {
          bytes,
          total,
          speed,
        } = event
        {
          crate::usage::speed_sample(app, &entry.group_id, &entry.id, Some(speed));
          crate::events::emit(
            app,
            crate::events::MEDIA_PROGRESS,
            MediaProgress {
              id: entry.id.clone(),
              group_id: entry.group_id.clone(),
              category: ProgressCategory::Other,
              percentage: total
                .filter(|n| *n > 0)
                .map(|n| bytes as f64 / n as f64 * 100.0),
              speed_bps: Some(speed),
              eta_secs: total
                .filter(|n| *n >= bytes && speed > 0.0)
                .map(|n| ((n - bytes) as f64 / speed) as u64),
            },
          );
        }
      },
    )
    .await?;
  if !*cancel.borrow() {
    return Err(Error::Cancelled);
  }
  if let Ok(mut partials) = PARTIALS.lock() {
    if let Some(group) = partials.get_mut(&entry.group_id) {
      let identity = state::identity(&entry.url);
      let owner = state::identity(&entry.id);
      group.retain(|p| p.identity != identity || p.owner != owner || p.directory != dir);
      if group.is_empty() {
        partials.remove(&entry.group_id);
      }
    }
  }
  if let Some(records) = app.try_state::<DownloadRecords>() {
    records.allow_root(&entry.group_id, dir);
    records.record_item(&entry.group_id, &entry.id, &path.to_string_lossy());
    records.seal(&entry.group_id);
  }
  crate::events::emit(
    app,
    crate::events::MEDIA_DESTINATION,
    MediaDestination {
      id: entry.id.clone(),
      group_id: entry.group_id.clone(),
      is_merged: false,
      destination: MediaDestinationPath {
        confidence: 100,
        path: path.to_string_lossy().into_owned(),
      },
    },
  );
  if entry.emit_complete {
    crate::library::complete(app, entry).await;
  }
  Ok(())
}

pub(crate) fn diagnostic(app: &AppHandle, group: &str, id: &str, error: &Error) {
  let code = error.code();
  let message = app
    .state::<crate::i18n::I18nManager>()
    .t(&format!("files.errors.{code}"));
  crate::extract::terminal::emit_terminal(
    app,
    MediaDiagnosticPayload {
      id: id.into(),
      group_id: group.into(),
      level: crate::models::error::DiagnosticLevel::Error,
      code: code.into(),
      component: Some("files".into()),
      message: message.clone(),
      raw: String::new(),
      timestamp: std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis()),
    },
    MediaFatalPayload::with_exit(group.into(), id.into(), 1, message),
  );
}
