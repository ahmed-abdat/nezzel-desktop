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

impl FetchEntry {
  pub(crate) fn is_playlist_child(&self) -> bool {
    self.total > 1
      || GROUP_COUNTERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&self.group_id)
  }
}

/// An unavailable child is a skipped row; its siblings still own the group counter.
pub(crate) fn child_terminal(
  entry: &FetchEntry,
  diagnostic: &mut crate::models::MediaDiagnosticPayload,
  fatal: &mut MediaFatalPayload,
) {
  if entry.is_playlist_child()
    && matches!(
      diagnostic.code.as_str(),
      "mediaUnavailable" | "videoNotFoundOrPrivate" | "notFound404"
    )
  {
    diagnostic.level = crate::models::error::DiagnosticLevel::Warning;
    diagnostic.code = "input_filter_skipped".into();
    fatal.exit_code = Some(0);
  }
}

fn live_refusal(
  group_id: String,
  id: String,
  message: String,
) -> (crate::models::MediaDiagnosticPayload, MediaFatalPayload) {
  let fatal = MediaFatalPayload::with_exit(group_id.clone(), id.clone(), 1, message.clone());
  let diagnostic = crate::models::MediaDiagnosticPayload {
    group_id,
    id,
    level: crate::models::error::DiagnosticLevel::Error,
    code: "liveNow".into(),
    component: Some("extract".into()),
    message: message.clone(),
    raw: message,
    timestamp: fatal.timestamp,
  };
  (diagnostic, fatal)
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
      GROUP_COUNTERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(group_id.clone(), total);
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

      None
    }
  };

  if format.is_none() {
    let engine = match &result {
      Some(ParsedMedia::Single(single)) => Some(single.fetch_engine),
      Some(ParsedMedia::Playlist(_)) => Some(None),
      _ => None,
    };
    crate::usage::options_shown(&app, &group_id, &id, engine);
  }

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
            generation: crate::scheduling::group_state::fetch_done_generation(&group_id),
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
      use tauri::Manager;
      let message = app
        .state::<crate::i18n::I18nManager>()
        .t("errors.runner.liveNow");
      let (diagnostic, fatal) = live_refusal(group_id.clone(), id, message);
      crate::extract::terminal::emit_terminal(&app, diagnostic, fatal);
      let mut counters = GROUP_COUNTERS.lock().unwrap();
      if let Some(cnt) = counters.get_mut(&group_id) {
        *cnt -= 1;
        if *cnt == 0 {
          counters.remove(&group_id);
          let _ = tx.send(DispatchRequest::Cleanup {
            group_id: group_id.clone(),
            generation: crate::scheduling::group_state::fetch_done_generation(&group_id),
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
    generation: crate::scheduling::group_state::fetch_done_generation(group_id),
  });
}

pub fn forget_group_counter(group_id: &str) {
  GROUP_COUNTERS.lock().unwrap().remove(group_id);
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

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn live_refusal_is_a_normal_localized_fatal() {
    let (diagnostic, payload) = live_refusal(
      "group".into(),
      "row".into(),
      "localized live message".into(),
    );
    assert_eq!(diagnostic.code, "liveNow");
    assert_eq!(diagnostic.message, payload.message);
    assert!(!payload.internal);
    assert_eq!(payload.message, "localized live message");
    assert_eq!(payload.exit_code, Some(1));
  }

  #[test]
  fn unavailable_child_is_skipped_and_only_last_child_cleans_up() {
    let group = "unavailable-child-test";
    let entry = expand_fetch_request(FetchRequest::Playlist {
      group_id: group.into(),
      overrides: Box::new(None),
      entries: (0..2)
        .map(|index| PlaylistEntry {
          filesize: None,
          title: None,
          duration: None,
          thumbnail: None,
          video_url: format!("https://youtube.com/watch?v={index}"),
          index,
        })
        .collect(),
    })
    .remove(0);
    let mut fatal =
      MediaFatalPayload::with_exit(group.into(), "child".into(), 1, "Unavailable".into());
    let mut diagnostic = crate::models::MediaDiagnosticPayload {
      group_id: group.into(),
      id: "child".into(),
      level: crate::models::error::DiagnosticLevel::Error,
      code: "mediaUnavailable".into(),
      component: None,
      message: "Unavailable".into(),
      raw: String::new(),
      timestamp: 0,
    };
    child_terminal(&entry, &mut diagnostic, &mut fatal);
    assert_eq!(diagnostic.code, "input_filter_skipped");
    assert_eq!(diagnostic.message, "Unavailable");
    assert_eq!(fatal.exit_code, Some(0));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    finish_fetch(&tx, group);
    assert!(rx.try_recv().is_err());
    finish_fetch(&tx, group);
    assert!(
      matches!(rx.try_recv(), Ok(DispatchRequest::Cleanup { group_id, .. }) if group_id == group)
    );
  }

  #[test]
  fn finished_fetch_cleanup_does_not_end_a_download_already_started() {
    use crate::scheduling::group_state::{
      ensure_group_running, is_group_running, remove_group, remove_group_if, start_group_run,
    };
    let group = "fetch-done-after-download-start";
    ensure_group_running(group);
    // The first fetched row is downloading while the group's last fetch is still running.
    start_group_run(group);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    finish_fetch(&tx, group);
    let Ok(DispatchRequest::Cleanup { generation, .. }) = rx.try_recv() else {
      panic!("expected Cleanup");
    };
    assert!(!remove_group_if(group, generation));
    assert!(is_group_running(group));
    remove_group(group);
  }
}
