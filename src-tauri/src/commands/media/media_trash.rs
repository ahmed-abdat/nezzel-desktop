use crate::download_records::{
  regular_file_identity, resolve_trash_target, DownloadRecords, TrashRefusal,
};
use std::path::Path;
use tauri::State;

/// Moves one downloaded file to the OS Trash / Recycle Bin. Never deletes outright.
#[tauri::command]
pub fn media_trash_file(
  group_id: String,
  path: String,
  records: State<'_, DownloadRecords>,
) -> Result<(), String> {
  // Log the reason only, never the path (privacy rule).
  let refuse = |refusal: TrashRefusal| {
    tracing::warn!(reason = %refusal, "Delete file refused");
    refusal.code().to_string()
  };
  let target = resolve_trash_target(&records.get(&group_id), Path::new(&path)).map_err(refuse)?;
  // Last look, without following links, right before the move: still the same regular file.
  if regular_file_identity(&target.path).as_ref() != Some(&target.identity) {
    return Err(refuse(TrashRefusal::Replaced));
  }
  trash::delete(&target.path).map_err(|_| {
    // The trash crate's error text can carry the path in forms we can't reliably redact: log nothing of it.
    tracing::warn!("Moving to Trash failed");
    "trashFailed".to_string()
  })
}
