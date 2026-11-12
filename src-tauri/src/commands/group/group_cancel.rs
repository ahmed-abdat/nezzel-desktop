use crate::download_records::DownloadRecords;
use crate::logging::LogStoreState;
use crate::scheduling::dispatcher::DispatchRequest;
use crate::scheduling::download_pipeline::DownloadSender;
use crate::scheduling::fetch_pipeline::FetchSender;
use crate::scheduling::group_state::cancel_group;
use tauri::State;

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects each State as its own argument.
pub fn group_cancel(
  app: tauri::AppHandle,
  group_id: String,
  discard_files: Option<bool>,
  log_state: State<'_, LogStoreState>,
  fetch_sender: State<'_, FetchSender>,
  download_sender: State<'_, DownloadSender>,
  records: State<'_, DownloadRecords>,
  library: State<'_, crate::library::Library>,
) {
  cancel_group(&group_id);
  if discard_files != Some(true) {
    crate::runners::ytdlp_download::pause_group(&group_id);
  }
  library.forget_group(&group_id);
  crate::files::runner::forget(&group_id);
  if discard_files == Some(true) {
    // Removed from the list (not a pause): its job records go, so a reused ID counts its own end.
    crate::usage::forget_group(&app, &group_id);
    crate::files::runner::discard_group(&group_id);
    crate::runners::ytdlp_download::discard_group(&group_id);
  }
  records.forget(&group_id);

  // Send cleanup requests for items that were already dispatched.
  let _ = fetch_sender.0.send(DispatchRequest::Cleanup {
    group_id: group_id.clone(),
  });
  let _ = download_sender.0.send(DispatchRequest::Cleanup {
    group_id: group_id.clone(),
  });

  let mut store = log_state.write();
  store.remove_group(&group_id);
}
