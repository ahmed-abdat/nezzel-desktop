mod index;
use crate::{
  download_records::{recorded_files, safe_library_root, scanned_files, DownloadRecords},
  models::MediaProgressComplete,
  paths::PathsManager,
  scheduling::download_pipeline::DownloadEntry,
  SharedConfig,
};
pub use index::LibraryRecord;
use index::{kind, regular_file, scan_folder, AttachmentSummary, Index, LibraryDisplay};
use serde::Deserialize;
use std::{
  collections::HashMap,
  path::{Path, PathBuf},
  sync::Mutex,
  time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionMetadata {
  history_id: Option<String>,
  title: Option<String>,
  site: Option<String>,
  thumbnail: Option<String>,
  #[serde(default)]
  display: LibraryDisplay,
}

#[derive(Default)]
struct LibraryData {
  index: Option<Index>,
  metadata: HashMap<String, HashMap<String, CompletionMetadata>>,
}
#[derive(Default)]
pub struct Library(Mutex<LibraryData>);
impl Library {
  pub fn remember(&self, group_id: &str, metadata: HashMap<String, CompletionMetadata>) {
    if let Ok(mut data) = self.0.lock() {
      data
        .metadata
        .entry(group_id.to_owned())
        .or_default()
        .extend(metadata);
    }
  }

  pub fn forget_item(&self, group_id: &str, id: &str) {
    if let Ok(mut data) = self.0.lock() {
      take_metadata(&mut data, group_id, id);
    }
  }

  pub fn forget_group(&self, group_id: &str) {
    if let Ok(mut data) = self.0.lock() {
      data.metadata.remove(group_id);
    }
  }
}

fn take_metadata(data: &mut LibraryData, group_id: &str, id: &str) -> CompletionMetadata {
  let metadata = data
    .metadata
    .get_mut(group_id)
    .and_then(|items| items.remove(id))
    .unwrap_or_default();
  if data.metadata.get(group_id).is_some_and(HashMap::is_empty) {
    data.metadata.remove(group_id);
  }
  metadata
}

fn configured_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
  app
    .state::<SharedConfig>()
    .load()
    .output
    .download_dir
    .as_ref()
    .map(PathBuf::from)
}

fn protected_dirs() -> (Option<PathBuf>, Option<PathBuf>) {
  let home = dirs::home_dir();
  let downloads = crate::runners::ytdlp_args::downloads_dir(dirs::download_dir(), home.clone());
  (home, downloads)
}

fn index_path<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
  app.state::<PathsManager>().app_dir().join("library.json")
}

/// Persist before notifying the UI, including downloads that finish while the window is closed.
/// Called by all engines at their existing successful completion boundary.
pub async fn complete(app: &AppHandle, entry: &DownloadEntry) {
  complete_with(app, entry, None).await;
}

/// `complete` for engines that know how much SponsorBlock removed.
pub async fn complete_with(
  app: &AppHandle,
  entry: &DownloadEntry,
  sponsor_removed_secs: Option<u32>,
) {
  let app_clone = app.clone();
  let entry = entry.clone();
  let id = entry.id.clone();
  let group_id = entry.group_id.clone();
  let result = tauri::async_runtime::spawn_blocking(move || {
    persist_completion(&app_clone, &entry, sponsor_removed_secs)
  })
  .await;
  if !matches!(result, Ok(Ok(()))) {
    // No paths, titles, or source URLs in telemetry/log errors.
    tracing::warn!("Unable to save local download history");
  }
  crate::events::emit(
    app,
    crate::events::MEDIA_COMPLETE,
    MediaProgressComplete {
      id,
      group_id,
      sponsor_removed_secs,
    },
  );
}

/// What the engine knows at completion overrides what the UI sent when the download was queued.
fn apply_engine_facts(
  display: &mut LibraryDisplay,
  entry: &DownloadEntry,
  sponsor_removed_secs: Option<u32>,
) {
  display.track_type = Some(entry.format.track_type.clone());
  display.height = entry.format.height.or(display.height);
  display.fps = entry.format.fps.or(display.fps);
  display.sponsor_removed_secs = sponsor_removed_secs;
}

fn persist_completion<R: Runtime>(
  app: &AppHandle<R>,
  entry: &DownloadEntry,
  sponsor_removed_secs: Option<u32>,
) -> Result<(), String> {
  let Some(records) = app.try_state::<DownloadRecords>() else {
    return Ok(());
  };
  let record = records.get(&entry.group_id);
  let authority = recorded_files(&record, &entry.id);
  let paths: Vec<_> = authority.iter().map(|file| file.path.clone()).collect();
  let library = app.try_state::<Library>().ok_or("library unavailable")?;
  let mut data = library.0.lock().map_err(|_| "library unavailable")?;
  let mut metadata = take_metadata(&mut data, &entry.group_id, &entry.id);
  if paths.is_empty() {
    return Ok(());
  }
  let (home, downloads) = protected_dirs();
  let root = configured_root(app)
    .and_then(|root| safe_library_root(&root, home.as_deref(), downloads.as_deref()));
  let index = load_index(&mut data, &index_path(app), root.as_deref())?;
  let title = metadata.title.unwrap_or_else(|| {
    paths[0]
      .file_name()
      .unwrap_or_default()
      .to_string_lossy()
      .into_owned()
  });
  let size = paths
    .iter()
    .filter_map(|p| regular_file(p))
    .map(|m| m.len())
    .sum();
  let kind = kind(&paths[0]).to_owned();
  apply_engine_facts(&mut metadata.display, entry, sponsor_removed_secs);
  if !entry.attachments.is_empty() {
    let selected: Vec<_> = entry
      .attachments
      .iter()
      .filter(|a| {
        entry
          .attachment_ids
          .as_ref()
          .is_none_or(|ids| ids.contains(&a.id))
      })
      .collect();
    metadata.display.attachment_summary = Some(AttachmentSummary {
      photos: selected
        .iter()
        .filter(|a| matches!(a.kind, crate::models::parsed::AttachmentKind::Image))
        .count() as u32,
      videos: selected
        .iter()
        .filter(|a| matches!(a.kind, crate::models::parsed::AttachmentKind::Video))
        .count() as u32,
    });
  }
  index.append(LibraryRecord {
    id: metadata
      .history_id
      .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
    paths,
    url: entry.url.clone(),
    title,
    site: metadata.site,
    kind,
    size,
    completed_at: SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_millis() as u64,
    thumbnail: metadata.thumbnail,
    authority,
    display: metadata.display,
    error_code: None,
  });
  index
    .save(&index_path(app))
    .map_err(|_| "unable to save library".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFailure {
  id: String,
  url: String,
  kind: String,
  error_code: String,
  #[serde(flatten)]
  metadata: CompletionMetadata,
}

/// Local failed rows have no file authority and are never considered downloaded files.
#[tauri::command]
pub async fn library_record_failure<R: Runtime>(
  app: AppHandle<R>,
  row: LibraryFailure,
) -> Result<(), String> {
  tauri::async_runtime::spawn_blocking(move || {
    let library = app.try_state::<Library>().ok_or("library unavailable")?;
    let mut data = library.0.lock().map_err(|_| "library unavailable")?;
    let index = load_index(&mut data, &index_path(&app), None)?;
    index.append(LibraryRecord {
      id: row.id,
      paths: Vec::new(),
      url: row.url,
      title: row.metadata.title.unwrap_or_default(),
      site: row.metadata.site,
      kind: row.kind,
      size: 0,
      thumbnail: row.metadata.thumbnail,
      completed_at: SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64,
      authority: Vec::new(),
      display: row.metadata.display,
      error_code: Some(row.error_code),
    });
    index
      .save(&index_path(&app))
      .map_err(|_| "unable to save library".into())
  })
  .await
  .map_err(|_| "library unavailable".to_owned())?
}

fn load_index<'a>(
  data: &'a mut LibraryData,
  path: &Path,
  root: Option<&Path>,
) -> Result<&'a mut Index, String> {
  if data.index.is_none() {
    data.index = Some(Index::load(path).map_err(|_| "unable to load library")?);
  }
  let index = data.index.as_mut().ok_or("library unavailable")?;
  index.prune_excluded(root);
  Ok(index)
}

/// Two phases let the UI display indexed rows before the bounded folder scan finishes.
#[tauri::command]
pub async fn library_list<R: Runtime>(
  app: AppHandle<R>,
  scan: Option<bool>,
) -> Result<Vec<LibraryRecord>, String> {
  tauri::async_runtime::spawn_blocking(move || {
    let library = app.try_state::<Library>().ok_or("library unavailable")?;
    let (home, downloads) = protected_dirs();
    let root = configured_root(&app)
      .and_then(|root| safe_library_root(&root, home.as_deref(), downloads.as_deref()));
    let scanned = if scan == Some(true) {
      root
        .as_ref()
        .map(|root| scan_folder(root, home.as_deref(), downloads.as_deref()))
        .unwrap_or_default()
    } else {
      Vec::new()
    };
    let mut data = library.0.lock().map_err(|_| "library unavailable")?;
    let index = load_index(&mut data, &index_path(&app), root.as_deref())?;
    index.prune();
    index.merge_scan(scanned);
    index
      .save(&index_path(&app))
      .map_err(|_| "unable to save library")?;
    let records = app.state::<DownloadRecords>();
    for row in &index.records {
      records.restore_history(
        &format!("library:{}", row.id),
        &row.authority,
        home.as_deref(),
        downloads.as_deref(),
      );
    }
    for row in &index.scanned {
      let files = scanned_files(
        &row.paths,
        root.as_deref(),
        home.as_deref(),
        downloads.as_deref(),
      );
      records.restore_history(
        &format!("library:{}", row.id),
        &files,
        home.as_deref(),
        downloads.as_deref(),
      );
    }
    Ok(index.rows())
  })
  .await
  .map_err(|_| "library unavailable".to_owned())?
}

/// Hides rows durably; never deletes files. Exclusions prevent the next scan restoring removed rows.
#[tauri::command]
pub async fn library_remove<R: Runtime>(
  app: AppHandle<R>,
  paths: Vec<PathBuf>,
  ids: Option<Vec<String>>,
) -> Result<(), String> {
  tauri::async_runtime::spawn_blocking(move || {
    let library = app.try_state::<Library>().ok_or("library unavailable")?;
    let mut data = library.0.lock().map_err(|_| "library unavailable")?;
    let (home, downloads) = protected_dirs();
    let root = configured_root(&app)
      .and_then(|root| safe_library_root(&root, home.as_deref(), downloads.as_deref()));
    let index = load_index(&mut data, &index_path(&app), root.as_deref())?;
    if let Some(ids) = ids {
      index
        .records
        .retain(|row| row.error_code.is_none() || !ids.contains(&row.id));
    }
    index.remove(&paths, root.as_deref());
    index
      .save(&index_path(&app))
      .map_err(|_| "unable to save library".into())
  })
  .await
  .map_err(|_| "library unavailable".to_owned())?
}

#[cfg(test)]
mod tests;
