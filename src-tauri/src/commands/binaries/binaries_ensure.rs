use crate::binaries::binaries_manager::BinariesManager;
use crate::binaries::binaries_state::BinariesState;
use tauri::State;

#[tauri::command]
pub async fn binaries_ensure(
  app: tauri::AppHandle,
  binaries_manager: State<'_, BinariesManager>,
  state: State<'_, BinariesState>,
  tools: Option<Vec<String>>,
) -> Result<(), String> {
  if !state.try_start() {
    return Ok(());
  }
  let res = match tools.as_ref() {
    Some(v) => binaries_manager.ensure(Some(v)).await,
    None => binaries_manager.ensure(None).await,
  }
  .map_err(|e| e.to_string());
  state.finish();
  BinariesManager::schedule_ytdlp_update(&app);
  res
}

/// Snapshot covers startup events emitted before the webview's listeners were ready.
#[tauri::command]
pub fn binaries_update_status(state: State<'_, BinariesState>) -> bool {
  state.update_stale.load(std::sync::atomic::Ordering::SeqCst)
}

#[tauri::command]
pub async fn binaries_update_now(manager: State<'_, BinariesManager>) -> Result<(), String> {
  manager
    .run_ytdlp_update(true)
    .await
    .map_err(|_| "toolsUpdateFailed".to_owned())
}
