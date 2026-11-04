mod index;
use crate::{
  commands::{recorded_files, safe_library_root, scanned_files, DownloadRecords},
  models::MediaProgressComplete,
  paths::PathsManager,
  scheduling::download_pipeline::DownloadEntry,
  SharedConfig,
};
pub use index::LibraryRecord;
use index::{kind, regular_file, scan_folder, Index};
use serde::Deserialize;
use std::{
  collections::HashMap,
  path::{Path, PathBuf},
  sync::Mutex,
  time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionMetadata {
  title: Option<String>,
  site: Option<String>,
  thumbnail: Option<String>,
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

fn configured_root(app: &AppHandle) -> Option<PathBuf> {
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
  let downloads = dirs::download_dir().or_else(|| home.as_ref().map(|p| p.join("Downloads")));
  (home, downloads)
}

fn index_path(app: &AppHandle) -> PathBuf {
  app.state::<PathsManager>().app_dir().join("library.json")
}

/// Persist before notifying the UI, including downloads that finish while the window is closed.
/// Called by all engines at their existing successful completion boundary.
pub async fn complete(app: &AppHandle, entry: &DownloadEntry) {
  let app_clone = app.clone();
  let entry = entry.clone();
  let id = entry.id.clone();
  let group_id = entry.group_id.clone();
  let result =
    tauri::async_runtime::spawn_blocking(move || persist_completion(&app_clone, &entry)).await;
  if !matches!(result, Ok(Ok(()))) {
    // No paths, titles, or source URLs in telemetry/log errors.
    tracing::warn!("Unable to save local download history");
  }
  let _ = app.emit("media_complete", MediaProgressComplete { id, group_id });
}

fn persist_completion(app: &AppHandle, entry: &DownloadEntry) -> Result<(), String> {
  let Some(records) = app.try_state::<DownloadRecords>() else {
    return Ok(());
  };
  let record = records.get(&entry.group_id);
  let authority = recorded_files(&record, &entry.id);
  let paths: Vec<_> = authority.iter().map(|file| file.path.clone()).collect();
  let library = app.state::<Library>();
  let mut data = library.0.lock().map_err(|_| "library unavailable")?;
  let metadata = take_metadata(&mut data, &entry.group_id, &entry.id);
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
  index.append(LibraryRecord {
    id: entry.id.clone(),
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
  });
  index
    .save(&index_path(app))
    .map_err(|_| "unable to save library".into())
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
pub async fn library_list(
  app: AppHandle,
  scan: Option<bool>,
) -> Result<Vec<LibraryRecord>, String> {
  tauri::async_runtime::spawn_blocking(move || {
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
    let library = app.state::<Library>();
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
pub async fn library_remove(app: AppHandle, paths: Vec<PathBuf>) -> Result<(), String> {
  tauri::async_runtime::spawn_blocking(move || {
    let library = app.state::<Library>();
    let mut data = library.0.lock().map_err(|_| "library unavailable")?;
    let (home, downloads) = protected_dirs();
    let root = configured_root(&app)
      .and_then(|root| safe_library_root(&root, home.as_deref(), downloads.as_deref()));
    let index = load_index(&mut data, &index_path(&app), root.as_deref())?;
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
