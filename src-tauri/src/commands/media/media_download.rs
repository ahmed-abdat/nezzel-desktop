use crate::library::{CompletionMetadata, Library};
use crate::models::DownloadItem;
use crate::scheduling::dispatcher::DispatchRequest;
use crate::scheduling::download_pipeline::{DownloadRequest, DownloadSender};
use crate::scheduling::group_state::ensure_group_running;
use std::collections::HashMap;
use tauri::State;

#[tauri::command]
pub fn media_download(
  group_id: String,
  items: Vec<DownloadItem>,
  metadata: Option<HashMap<String, CompletionMetadata>>,
  library: State<'_, Library>,
  pipeline: State<'_, DownloadSender>,
) -> String {
  ensure_group_running(&group_id);
  if let Some(metadata) = metadata {
    library.remember(&group_id, metadata);
  }

  pipeline
    .0
    .send(DispatchRequest::Pipeline(DownloadRequest::Batch {
      group_id: group_id.clone(),
      items,
    }))
    .unwrap();

  group_id
}
