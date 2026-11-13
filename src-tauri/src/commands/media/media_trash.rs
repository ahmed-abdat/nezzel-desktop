use crate::download_records::{
  regular_file_identity, resolve_trash_target, DownloadRecords, TrashRefusal,
};
use crate::runners::ytdlp_files::{
  receipt_directory, remove_missing_receipts, trash_with_receipts,
};
use std::path::Path;
use tauri::State;

/// Moves one downloaded file to the OS Trash / Recycle Bin. Never deletes outright.
#[tauri::command]
pub fn media_trash_file<R: tauri::Runtime>(
  app: tauri::AppHandle<R>,
  group_id: String,
  path: String,
  records: State<'_, DownloadRecords>,
) -> Result<(), String> {
  // Log the reason only, never the path (privacy rule).
  let refuse = |refusal: TrashRefusal| {
    tracing::warn!(reason = %refusal, "Delete file refused");
    refusal.code().to_string()
  };
  let receipts = receipt_directory(&app).map_err(|_| "trashFailed".to_string())?;
  let record = records.get(&group_id);
  let requested = Path::new(&path);
  let target = resolve_trash_target(&record, requested).map_err(|reason| {
    if matches!(reason, TrashRefusal::Missing) {
      let _ = remove_missing_receipts(&record, &receipts, requested);
    }
    refuse(reason)
  })?;
  // Last look, without following links, right before the move: still the same regular file.
  if regular_file_identity(&target.path).as_ref() != Some(&target.identity) {
    return Err(refuse(TrashRefusal::Replaced));
  }
  trash_with_receipts(&target.path, &receipts, |path| {
    trash::delete(path).map_err(std::io::Error::other)
  })
  .map_err(|_| {
    // The trash crate's error text can carry the path in forms we can't reliably redact: log nothing of it.
    tracing::warn!("Moving to Trash failed");
    "trashFailed".to_string()
  })
}
