use crate::models::download::{DownloadOverrides, FormatOptions};
use crate::models::DownloadItem;
use crate::models::SubtitleInventory;
use crate::runners::template_context::TemplateContext;
use crate::runners::ytdlp_download::YtdlpDownloadError;
use crate::scheduling::concurrency::DynamicSemaphore;
use crate::scheduling::dispatcher::{DispatchEntry, DispatchRequest, GenericDispatcher};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub struct DownloadSender(pub UnboundedSender<DispatchRequest<DownloadRequest>>);

#[derive(Clone)]
pub enum DownloadRequest {
  Resume(Box<DownloadEntry>),
  Batch {
    group_id: String,
    items: Vec<DownloadItem>,
  },
}

#[derive(Clone)]
pub struct DownloadEntry {
  pub group_id: String,
  pub id: String,
  pub url: String,
  pub resolved: Option<Box<crate::resolve::ResolvedInput>>,
  pub format: FormatOptions,
  pub fetch_engine: Option<crate::models::parsed::FetchEngine>,
  pub attachment_ids: Option<Vec<String>>,
  pub attachments: Vec<crate::models::parsed::MediaAttachment>,
  pub media_kind: Option<crate::models::parsed::MediaKind>,
  pub video_index: Option<usize>,
  pub emit_complete: bool,
  pub resume_parent: Option<Box<DownloadEntry>>,
  pub subtitle_inventory: Option<SubtitleInventory>,
  pub overrides: Option<DownloadOverrides>,
  pub template_context: TemplateContext,
  pub login_browser: Option<String>,
  pub login_was_accepted: bool,
}

impl From<(DownloadItem, String)> for DownloadEntry {
  fn from(item: (DownloadItem, String)) -> Self {
    let login_browser = crate::runners::login::row_browser(&item.1, &item.0.id, &item.0.url);
    let login_was_accepted = login_browser.is_some();
    let resolved = item.0.resolved_url.as_ref().map(|target| {
      let mut input = crate::resolve::ResolvedInput::new(&item.0.url);
      input.rewrite(target.clone());
      Box::new(input)
    });
    Self {
      group_id: item.1,
      id: item.0.id,
      url: item.0.url,
      resolved,
      format: item.0.format,
      fetch_engine: item.0.fetch_engine,
      attachment_ids: item.0.attachment_ids,
      attachments: item.0.attachments,
      media_kind: item.0.media_kind,
      video_index: None,
      emit_complete: true,
      resume_parent: None,
      subtitle_inventory: item.0.subtitle_inventory,
      overrides: item.0.overrides,
      template_context: item.0.template_context,
      login_browser,
      login_was_accepted,
    }
  }
}

impl DownloadEntry {
  pub(crate) fn take_fetch_input(&mut self) -> Option<crate::resolve::ResolvedInput> {
    self.resolved.take().map(|input| *input).or_else(|| {
      // Rows saved before resolvedUrl was retained still need their site reader.
      // New rows carry a target even when the reader passed the source through unchanged.
      (self.fetch_engine == Some(crate::models::parsed::FetchEngine::Ytdlp)
        && crate::extract::registry::readers(&self.url).is_empty())
      .then(|| crate::resolve::ResolvedInput::new(&self.url))
    })
  }

  pub fn login_request_entry(&self) -> Self {
    self.resume_parent.as_deref().unwrap_or(self).clone()
  }
}

impl DispatchEntry for DownloadEntry {
  fn group_id(&self) -> &String {
    &self.group_id
  }
  fn group_key(&self) -> Option<&String> {
    self.template_context.values.get("playlist_id")
  }
  fn set_numbering(&mut self, autonumber: u64, group_autonumber: Option<u64>) {
    if self.login_browser.is_some() {
      return;
    }
    self
      .template_context
      .values
      .insert("autonumber".to_string(), autonumber.to_string());
    if let Some(group_autonumber) = group_autonumber {
      self.template_context.values.insert(
        "playlist_autonumber".to_string(),
        group_autonumber.to_string(),
      );
    }
  }
}

static DOWNLOAD_COUNTERS: LazyLock<Mutex<HashMap<String, usize>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn setup_download_dispatcher(
  app: &AppHandle,
  sem: Arc<DynamicSemaphore>,
) -> GenericDispatcher<DownloadRequest> {
  GenericDispatcher::start(
    app.clone(),
    sem,
    |req: DownloadRequest| match req {
      DownloadRequest::Resume(entry) => vec![*entry],
      DownloadRequest::Batch { group_id, items } => {
        let total = items.len();
        DOWNLOAD_COUNTERS
          .lock()
          .unwrap()
          .insert(group_id.clone(), total);
        items
          .into_iter()
          .map(|item| DownloadEntry::from((item, group_id.clone())))
          .collect()
      }
    },
    |tx, app: AppHandle, entry: DownloadEntry| async move {
      tracing::info!(download_id = %entry.id, "Starting download");
      crate::usage::download_started(&app, &entry);

      let result = crate::runners::gallery::run_media_download(app.clone(), entry.clone()).await;
      let succeeded = result.is_ok();
      // A stop (pause, cancel, remove) returns Ok with the group no longer running.
      let end = match &result {
        Ok(()) if !crate::scheduling::group_state::is_group_running(&entry.group_id) => {
          crate::usage::store::End::Cancelled
        }
        Ok(()) => crate::usage::store::End::Finished,
        Err(_) => crate::usage::store::End::Failed,
      };
      if !matches!(result, Err(YtdlpDownloadError::LoginPending)) {
        crate::usage::download_ended(&app, &entry, end);
      }
      if let Err(e) = result {
        if matches!(e, YtdlpDownloadError::LoginPending) {
          return;
        }
        tracing::warn!(
          download_id = %entry.id,
          group_id = %entry.group_id,
          error = %e,
          "Failed to run media download",
        );
      }

      if let Some(library) = app.try_state::<crate::library::Library>() {
        library.forget_item(&entry.group_id, &entry.id);
      }
      finish_download(&tx, &entry.group_id, &entry.id, succeeded);
    },
  )
}

pub fn finish_download(
  tx: &UnboundedSender<DispatchRequest<DownloadRequest>>,
  group_id: &str,
  id: &str,
  succeeded: bool,
) {
  if succeeded {
    crate::runners::login::forget_row_browser(group_id, id);
  }
  let mut counters = DOWNLOAD_COUNTERS.lock().unwrap();
  if let Some(cnt) = counters.get_mut(group_id) {
    *cnt = cnt.saturating_sub(1);
    if *cnt == 0 {
      counters.remove(group_id);
      let _ = tx.send(DispatchRequest::Cleanup {
        group_id: group_id.into(),
      });
    }
  }
}

pub fn forget_group_counter(group_id: &str) {
  DOWNLOAD_COUNTERS.lock().unwrap().remove(group_id);
}
