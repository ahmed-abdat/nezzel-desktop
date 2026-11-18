pub mod group_cancel;

pub use group_cancel::*;

/// Called after the frontend has read the durable queue, before it starts any downloads.
#[tauri::command]
pub async fn group_sweep_partials(
  app: tauri::AppHandle,
  saved: Vec<(String, String)>,
  roots: Vec<String>,
) -> Result<(), String> {
  tauri::async_runtime::spawn_blocking(move || {
    crate::runners::ytdlp_download::sweep_download_partials(&app, saved, roots)
  })
  .await
  .map_err(|_| "partialCleanupUnavailable".to_string())
}
