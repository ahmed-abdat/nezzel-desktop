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

fn check_output_space(
  root: Result<std::path::PathBuf, &'static str>,
  estimate: Option<u64>,
) -> Result<(), &'static str> {
  super::disk_space::check(&root?, estimate)
}

#[derive(Clone)]
pub struct DownloadSender(pub UnboundedSender<DispatchRequest<DownloadRequest>>);

#[derive(Clone)]
pub enum DownloadRequest {
  Resume(Box<DownloadEntry>),
  Batch {
    group_id: String,
    /// From `ensure_group_running` in the command that queued it.
    generation: u64,
    items: Vec<DownloadItem>,
  },
}

#[derive(Clone)]
pub struct DownloadEntry {
  pub group_id: String,
  pub playlist_child: bool,
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
  /// The group run that queued this entry (set by `DownloadRequest::Batch`).
  pub generation: u64,
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
      playlist_child: item.0.playlist_child,
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
      generation: 0,
    }
  }
}

impl DownloadEntry {
  pub(crate) fn is_playlist_child(&self) -> bool {
    self.playlist_child
  }

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
  fn generation(&self) -> Option<u64> {
    Some(self.generation)
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

/// Per group: the run generation and how many of its downloads are still to finish.
static DOWNLOAD_COUNTERS: LazyLock<Mutex<HashMap<String, (u64, usize)>>> =
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
      DownloadRequest::Batch {
        group_id,
        generation,
        items,
      } => {
        count_batch(&group_id, generation, items.len());
        items
          .into_iter()
          .map(|item| DownloadEntry {
            generation,
            ..DownloadEntry::from((item, group_id.clone()))
          })
          .collect()
      }
    },
    |tx, app: AppHandle, entry: DownloadEntry| async move {
      tracing::info!(download_id = %entry.id, "Starting download");
      let started_generation = crate::scheduling::group_state::group_generation(&entry.group_id);
      if let Some(progress) = starting_progress(
        &entry.id,
        &entry.group_id,
        crate::scheduling::group_state::is_group_running(&entry.group_id),
      ) {
        crate::events::emit(&app, crate::events::MEDIA_PROGRESS_STAGE, progress);
      }
      crate::usage::download_started(&app, &entry);

      let _awake = super::keep_awake::acquire(&app);
      let root = if entry.fetch_engine == Some(crate::models::parsed::FetchEngine::Files) {
        crate::files::runner::root(&app, &entry).map_err(|error| error.code())
      } else {
        Ok(
          crate::runners::ytdlp_runner::YtdlpRunner::new(&app)
            .output_root(&entry.format.track_type, entry.overrides.as_ref()),
        )
      };
      let estimate = entry
        .template_context
        .values
        .get("filesize")
        .and_then(|size| size.parse().ok());
      let result = match check_output_space(root, estimate) {
        Ok(()) => crate::runners::gallery::run_media_download(app.clone(), entry.clone()).await,
        Err(code) => {
          crate::runners::reliability::fatal(&app, &entry, code);
          Err(YtdlpDownloadError::RunnerError(code.into()))
        }
      };
      // A pause that was resumed before this job ended still stopped this job.
      let running = still_running(&entry.group_id, started_generation);
      crate::runners::ytdlp_download::download_finished(
        &entry.group_id,
        &entry.id,
        &result,
        running,
      );
      let succeeded = result.is_ok();
      // A stop (pause, cancel, remove) returns Ok with the group no longer running.
      let end = match &result {
        Ok(()) if !running => crate::usage::store::End::Cancelled,
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
      finish_download(&tx, &entry.group_id, &entry.id, entry.generation, succeeded);
    },
  )
}

/// A pause that was resumed before the job ended still stopped it.
fn still_running(group_id: &str, started_generation: Option<u64>) -> bool {
  crate::scheduling::group_state::is_group_running(group_id)
    && crate::scheduling::group_state::group_generation(group_id) == started_generation
}

/// Batches of the same run add up; a newer run replaces the count of the run it resumed.
fn count_batch(group_id: &str, generation: u64, total: usize) {
  if total == 0 {
    return;
  }
  let mut counters = DOWNLOAD_COUNTERS.lock().unwrap();
  match counters.get_mut(group_id) {
    Some((counted, count)) if *counted == generation => *count = count.saturating_add(total),
    _ => {
      counters.insert(group_id.to_owned(), (generation, total));
    }
  }
}

/// `generation`: the run that queued the job; a job of an older run does not count down this one.
pub fn finish_download(
  tx: &UnboundedSender<DispatchRequest<DownloadRequest>>,
  group_id: &str,
  id: &str,
  generation: u64,
  succeeded: bool,
) {
  if succeeded {
    crate::runners::login::forget_row_browser(group_id, id);
  }
  let mut counters = DOWNLOAD_COUNTERS.lock().unwrap();
  if let Some((counted, cnt)) = counters.get_mut(group_id) {
    if *counted != generation {
      return;
    }
    *cnt = cnt.saturating_sub(1);
    if *cnt == 0 {
      counters.remove(group_id);
      crate::scheduling::group_state::finish_group_run(group_id, generation);
      let _ = tx.send(DispatchRequest::Cleanup {
        group_id: group_id.into(),
        generation: Some(generation),
      });
    }
  }
}

pub fn forget_group_counter(group_id: &str) {
  DOWNLOAD_COUNTERS.lock().unwrap().remove(group_id);
}

/// Called only after the dispatcher has acquired a download slot.
fn starting_progress(
  id: &str,
  group_id: &str,
  running: bool,
) -> Option<crate::models::MediaProgressStage> {
  running.then(|| crate::models::MediaProgressStage {
    id: id.to_owned(),
    group_id: group_id.to_owned(),
    stage: crate::models::ProgressStage::Initializing,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn started_job_reports_initializing_for_its_item_and_group() {
    let progress = starting_progress("item", "group", true).unwrap();
    assert_eq!(progress.id, "item");
    assert_eq!(progress.group_id, "group");
    assert_eq!(progress.stage, crate::models::ProgressStage::Initializing);
  }

  #[test]
  fn paused_or_cancelled_job_does_not_report_starting() {
    assert!(starting_progress("item", "group", false).is_none());
  }
  fn counter(group: &str) -> Option<(u64, usize)> {
    let counters = DOWNLOAD_COUNTERS.lock().unwrap();
    counters.get(group).copied()
  }

  #[test]
  fn batches_of_one_run_add_up_and_an_older_run_does_not_count_down_the_new_one() {
    let group = "counter-generations";
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    count_batch(group, 1, 2);
    count_batch(group, 1, 1);
    assert_eq!(counter(group), Some((1, 3)));

    // Resumed as run 2 while a job of run 1 was still ending.
    count_batch(group, 2, 1);
    finish_download(&tx, group, "old", 1, false);
    assert_eq!(counter(group), Some((2, 1)));
    assert!(rx.try_recv().is_err(), "a stale finish must not clean up");

    finish_download(&tx, group, "new", 2, false);
    assert!(!DOWNLOAD_COUNTERS.lock().unwrap().contains_key(group));
    assert!(matches!(
      rx.try_recv(),
      Ok(DispatchRequest::Cleanup { group_id, .. }) if group_id == group
    ));
  }

  #[test]
  fn fetch_command_on_a_downloading_group_keeps_its_job_running_and_counted_once() {
    use crate::scheduling::group_state::{
      cancel_group, ensure_group_running, group_generation, remove_group, start_group_run,
    };
    let group = "counter-fetch-joins";
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let generation = start_group_run(group);
    count_batch(group, generation, 1);
    let started = group_generation(group);

    // media_size / media_info / media_playlist_expand on the same group while it downloads.
    ensure_group_running(group);
    assert!(still_running(group, started), "job must end as finished");
    assert_eq!(counter(group), Some((generation, 1)));
    finish_download(&tx, group, "item", generation, true);
    assert!(matches!(
      rx.try_recv(),
      Ok(DispatchRequest::Cleanup { generation: Some(g), .. }) if g == generation
    ));

    // Pause then resume: the killed job reads as stopped, the resumed run is a new generation.
    let generation = start_group_run(group);
    let started = group_generation(group);
    cancel_group(group);
    let resumed = start_group_run(group);
    assert_ne!(resumed, generation);
    assert!(!still_running(group, started));
    remove_group(group);
  }

  #[test]
  fn second_download_batch_on_a_running_group_joins_its_run() {
    use crate::scheduling::group_state::{group_generation, remove_group, start_group_run};
    let group = "counter-second-batch";
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let first = start_group_run(group);
    count_batch(group, first, 2);
    let started = group_generation(group);

    let second = start_group_run(group);
    assert_eq!(second, first);
    count_batch(group, second, 1);
    assert_eq!(counter(group), Some((first, 3)));
    assert!(
      still_running(group, started),
      "first batch's jobs end as finished"
    );

    for id in ["a", "b", "c"] {
      assert!(rx.try_recv().is_err(), "no Cleanup before the last job");
      finish_download(&tx, group, id, first, true);
    }
    assert!(matches!(rx.try_recv(), Ok(DispatchRequest::Cleanup { .. })));
    remove_group(group);
  }

  #[test]
  fn last_finish_cleans_up_the_run_it_belongs_to() {
    use crate::scheduling::group_state::{cancel_group, remove_group, start_group_run};
    let group = "counter-cleanup-generation";
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let run = start_group_run(group);
    count_batch(group, run, 1);
    // The group moved on (here: a pause) before the last job ended.
    cancel_group(group);
    count_batch(group, run, 1);
    finish_download(&tx, group, "item", run, false);
    assert!(matches!(
      rx.try_recv(),
      Ok(DispatchRequest::Cleanup { generation: Some(g), .. }) if g == run
    ));
    remove_group(group);
  }

  #[test]
  fn an_empty_batch_does_not_replace_the_running_count() {
    let group = "counter-empty-batch";
    count_batch(group, 1, 2);
    count_batch(group, 2, 0);
    assert_eq!(counter(group), Some((1, 2)));
    forget_group_counter(group);
  }

  #[test]
  fn preserves_the_destination_resolution_error() {
    assert_eq!(
      super::check_output_space(Err("fileIo"), None),
      Err("fileIo")
    );
  }
}
