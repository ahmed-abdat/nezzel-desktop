use crate::models::download::{DownloadOverrides, FormatOptions};
use crate::models::payloads::MediaAddWithFormatPayload;
use crate::models::{MediaAddPayload, MediaFatalPayload, PlaylistEntry};
use crate::runners::ytdlp_info::YtdlpInfoFetchError;
use crate::{
  models::{ParsedMedia, ParsedPlaylist},
  scheduling::concurrency::DynamicSemaphore,
  scheduling::dispatcher::{DispatchEntry, DispatchRequest, GenericDispatcher},
};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

#[derive(Clone)]
pub struct FetchSender(pub UnboundedSender<DispatchRequest<FetchRequest>>);

#[derive(Clone)]
pub enum FetchRequest {
  Resume(Box<FetchEntry>),
  Initial {
    group_id: String,
    id: String,
    url: String,
    overrides: Box<Option<DownloadOverrides>>,
  },
  Playlist {
    group_id: String,
    entries: Vec<PlaylistEntry>,
    overrides: Box<Option<DownloadOverrides>>,
  },
  Size {
    group_id: String,
    id: String,
    url: String,
    format: FormatOptions,
  },
  SizePlaylist {
    group_id: String,
    playlist: ParsedPlaylist,
    format: FormatOptions,
  },
}

#[derive(Clone)]
pub struct FetchEntry {
  pub group_id: String,
  pub id: String,
  pub url: String,
  pub total: usize,
  pub format: Option<FormatOptions>,
  pub overrides: Option<DownloadOverrides>,
  pub login_browser: Option<String>,
  pub login_was_accepted: bool,
  /// Redacted evidence survives consent suspension; never stderr, browser ids or cookie material.
  pub prior_attempts: Vec<crate::extract::outcome::Attempt>,
  pub prior_resolved_url: Option<String>,
}

impl DispatchEntry for FetchEntry {
  fn group_id(&self) -> &String {
    &self.group_id
  }
  fn group_key(&self) -> Option<&String> {
    None
  }
  fn set_numbering(&mut self, _autonumber: u64, _group_autonumber: Option<u64>) {}
}

static GROUP_COUNTERS: LazyLock<Mutex<HashMap<String, usize>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn setup_fetch_dispatcher(
  app: &AppHandle,
  sem: Arc<DynamicSemaphore>,
) -> GenericDispatcher<FetchRequest> {
  GenericDispatcher::start(
    app.clone(),
    sem,
    expand_fetch_request,
    |tx: UnboundedSender<DispatchRequest<FetchRequest>>, app: AppHandle, entry: FetchEntry| async move {
      handle_fetch_entry(tx, app, entry).await;
    },
  )
}

fn expand_fetch_request(req: FetchRequest) -> Vec<FetchEntry> {
  match req {
    FetchRequest::Resume(entry) => vec![*entry],
    FetchRequest::Initial {
      group_id,
      id,
      url,
      overrides,
    } => {
      vec![FetchEntry {
        group_id,
        id,
        url,
        total: 1,
        format: None,
        overrides: *overrides,
        login_browser: None,
        login_was_accepted: false,
        prior_attempts: Vec::new(),
        prior_resolved_url: None,
      }]
    }
    FetchRequest::Playlist {
      group_id,
      entries,
      overrides,
    } => {
      let total = entries.len();
      GROUP_COUNTERS
        .lock()
        .unwrap()
        .insert(group_id.clone(), total);
      entries
        .into_iter()
        .map(|e| FetchEntry {
          group_id: group_id.clone(),
          id: Uuid::new_v4().to_string(),
          url: e.video_url,
          total,
          format: None,
          overrides: *overrides.clone(),
          login_browser: None,
          login_was_accepted: false,
          prior_attempts: Vec::new(),
          prior_resolved_url: None,
        })
        .collect()
    }
    FetchRequest::Size {
      group_id,
      id,
      url,
      format,
    } => {
      vec![FetchEntry {
        group_id,
        id,
        url,
        total: 1,
        format: Some(format),
        overrides: None,
        login_browser: None,
        login_was_accepted: false,
        prior_attempts: Vec::new(),
        prior_resolved_url: None,
      }]
    }
    FetchRequest::SizePlaylist {
      group_id,
      playlist,
      format,
    } => {
      let total = playlist.entries.len();
      playlist
        .entries
        .into_iter()
        .map(|e| FetchEntry {
          group_id: group_id.clone(),
          id: Uuid::new_v4().to_string(),
          url: e.video_url,
          total,
          format: Some(format.clone()),
          overrides: None,
          login_browser: None,
          login_was_accepted: false,
          prior_attempts: Vec::new(),
          prior_resolved_url: None,
        })
        .collect()
    }
  }
}

async fn handle_fetch_entry(
  tx: UnboundedSender<DispatchRequest<FetchRequest>>,
  app: AppHandle,
  entry: FetchEntry,
) {
  let FetchEntry {
    group_id,
    id,
    total,
    format,
    ..
  } = entry.clone();

  let result = crate::extract::chain::run(&app, &entry).await;

  let result = match result {
    Ok(v) => v,
    Err(YtdlpInfoFetchError::LoginPending) => return,
    Err(e) => {
      tracing::warn!(
        fetch_id = %id,
        group_id = %group_id,
        error = %e,
        "run_ytdlp_info_fetch failed"
      );
      if should_report_to_sentry(&e) {
        sentry::capture_error(&e);
      }

      None
    }
  };

  match result {
    Some(ParsedMedia::Single(single)) => {
      if let Some(format) = format {
        let payload = MediaAddWithFormatPayload {
          group_id: group_id.clone(),
          total,
          item: single,
          format,
        };
        let _ = app.emit("media_size", payload);
      } else {
        let payload = MediaAddPayload {
          group_id: group_id.clone(),
          total,
          item: single,
        };
        let _ = app.emit("media_add", payload);
      }

      let mut counters = GROUP_COUNTERS.lock().unwrap();
      if let Some(cnt) = counters.get_mut(&group_id) {
        *cnt -= 1;
        if *cnt == 0 {
          counters.remove(&group_id);
          let _ = tx.send(DispatchRequest::Cleanup {
            group_id: group_id.clone(),
          });
        }
      }
    }
    Some(ParsedMedia::Playlist(pl)) => {
      if let Some(format) = format {
        let _ = tx.send(DispatchRequest::Pipeline(FetchRequest::SizePlaylist {
          group_id: group_id.clone(),
          playlist: pl,
          format,
        }));
      } else {
        let payload = MediaAddPayload {
          group_id,
          total: pl.entries.len(),
          item: pl,
        };
        let _ = app.emit("media_add", payload);
      }
    }
    Some(ParsedMedia::Livestream(_)) => {
      let payload =
        MediaFatalPayload::internal(group_id.clone(), id, "Livestreams unsupported".into(), None);
      let _ = app.emit("media_fatal", payload);
      let mut counters = GROUP_COUNTERS.lock().unwrap();
      if let Some(cnt) = counters.get_mut(&group_id) {
        *cnt -= 1;
        if *cnt == 0 {
          counters.remove(&group_id);
          let _ = tx.send(DispatchRequest::Cleanup {
            group_id: group_id.clone(),
          });
        }
      }
    }
    None => {
      finish_fetch(&tx, &group_id);
    }
  }
}

pub fn finish_fetch(tx: &UnboundedSender<DispatchRequest<FetchRequest>>, group_id: &str) {
  let mut counters = GROUP_COUNTERS.lock().unwrap();
  if let Some(cnt) = counters.get_mut(group_id) {
    *cnt = cnt.saturating_sub(1);
    if *cnt != 0 {
      return;
    }
    counters.remove(group_id);
  }
  let _ = tx.send(DispatchRequest::Cleanup {
    group_id: group_id.into(),
  });
}

pub fn forget_group_counter(group_id: &str) {
  GROUP_COUNTERS.lock().unwrap().remove(group_id);
}

fn should_report_to_sentry(err: &YtdlpInfoFetchError) -> bool {
  matches!(
    err,
    YtdlpInfoFetchError::InvalidDiagnosticRules(_)
      | YtdlpInfoFetchError::RunnerFailed(_)
      | YtdlpInfoFetchError::ParseFailed(_)
  )
}

#[cfg(test)]
pub(crate) fn fresh_fetch_for_test(entry: &FetchEntry) -> FetchEntry {
  expand_fetch_request(FetchRequest::Initial {
    group_id: entry.group_id.clone(),
    id: entry.id.clone(),
    url: entry.url.clone(),
    overrides: Box::new(entry.overrides.clone()),
  })
  .remove(0)
}
