//! Route production login resumes back to their matrix row, without restarting its deadline.
use crate::{
  models::ParsedMedia,
  runners::{gallery, ytdlp_download::YtdlpDownloadError, ytdlp_info::YtdlpInfoFetchError},
  scheduling::{
    dispatcher::DispatchRequest,
    download_pipeline::{DownloadEntry, DownloadRequest},
    fetch_pipeline::{FetchEntry, FetchRequest},
  },
};
use std::{
  collections::HashMap,
  future::Future,
  sync::{Arc, Mutex},
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub(super) enum Work {
  Fetch(Box<FetchEntry>),
  Download(Box<DownloadEntry>),
  Stopped(&'static str),
}
impl Work {
  fn identity(&self) -> Option<(&str, bool)> {
    match self {
      Self::Fetch(entry) => Some((&entry.id, true)),
      Self::Download(entry) => Some((&entry.id, false)),
      Self::Stopped(_) => None,
    }
  }
}

#[derive(Default, Clone)]
pub(super) struct ResumeRouter(Arc<Mutex<HashMap<String, UnboundedSender<Work>>>>);
impl ResumeRouter {
  pub fn register(&self, group: &str, login_browser: Option<&str>) -> RowRetries {
    let (tx, rx) = mpsc::unbounded_channel();
    self.0.lock().unwrap().insert(group.into(), tx);
    RowRetries {
      router: self.clone(),
      group: group.into(),
      login_browser: login_browser.map(str::to_owned),
      resumes: 0,
      rx,
    }
  }
  fn send(&self, group: &str, work: Work) {
    if let Some(tx) = self.0.lock().unwrap().get(group) {
      let _ = tx.send(work);
    }
  }
  pub fn event(&self, kind: &str, value: &serde_json::Value) {
    let reason = if kind == "media_fatal" {
      Some("Login required: automatic retry failed")
    } else if kind == "media_login_needed" && value["reason"] != "automaticLogin" {
      Some("Login required: browser session or new consent needed")
    } else {
      None
    };
    if let Some(reason) = reason {
      self.send(
        value["groupId"].as_str().unwrap_or_default(),
        Work::Stopped(reason),
      );
    }
  }
  pub async fn drain(
    &self,
    mut fetch: UnboundedReceiver<DispatchRequest<FetchRequest>>,
    mut download: UnboundedReceiver<DispatchRequest<DownloadRequest>>,
  ) {
    loop {
      tokio::select! {
        Some(request) = fetch.recv() => match request {
          DispatchRequest::Pipeline(FetchRequest::Resume(entry)) => {
            let group = entry.group_id.clone();
            self.send(&group, Work::Fetch(entry));
          }
          DispatchRequest::Cleanup { group_id } => self.send(&group_id, Work::Stopped("Login required: fetch retry released")),
          _ => {}
        },
        Some(request) = download.recv() => if let DispatchRequest::Pipeline(DownloadRequest::Resume(entry)) = request {
          let group = entry.group_id.clone();
          self.send(&group, Work::Download(entry));
        },
        else => break,
      }
    }
  }
}

pub(super) enum AttemptError {
  Pending,
  Failed(String),
}
pub(super) struct RowRetries {
  router: ResumeRouter,
  group: String,
  pub login_browser: Option<String>,
  resumes: usize,
  rx: UnboundedReceiver<Work>,
}
impl Drop for RowRetries {
  fn drop(&mut self) {
    self.router.0.lock().unwrap().remove(&self.group);
  }
}
impl RowRetries {
  pub async fn run(
    &mut self,
    app: &tauri::AppHandle,
    work: Work,
  ) -> Result<Option<ParsedMedia>, String> {
    self.follow(work, |work| Box::pin(execute(app, work))).await
  }
  async fn follow<F, Fut>(
    &mut self,
    mut work: Work,
    mut run: F,
  ) -> Result<Option<ParsedMedia>, String>
  where
    F: FnMut(Work) -> Fut,
    Fut: Future<Output = Result<Option<ParsedMedia>, AttemptError>>,
  {
    let identity = work.identity().map(|(id, fetch)| (id.to_owned(), fetch));
    loop {
      match run(work).await {
        Ok(result) => return Ok(result),
        Err(AttemptError::Failed(error)) => return Err(error),
        Err(AttemptError::Pending) if self.login_browser.is_none() => {
          return Err("Login required: new consent needed".into())
        }
        Err(AttemptError::Pending) => {}
      }
      if self.resumes == 2 {
        return Err("Harness automatic-login resume limit: 2 per row".into());
      }
      work = loop {
        let resumed = self
          .rx
          .recv()
          .await
          .ok_or("Harness resume channel closed")?;
        if let Work::Stopped(reason) = resumed {
          return Err(reason.into());
        }
        if resumed.identity().is_some_and(|(id, fetch)| {
          identity
            .as_ref()
            .is_some_and(|expected| expected.0 == id && expected.1 == fetch)
        }) {
          break resumed;
        }
      };
      self.resumes += 1;
    }
  }
}
async fn execute(app: &tauri::AppHandle, work: Work) -> Result<Option<ParsedMedia>, AttemptError> {
  match work {
    Work::Fetch(entry) => Box::pin(gallery::run_media_info(app, &entry))
      .await
      .map_err(|error| match error {
        YtdlpInfoFetchError::LoginPending => AttemptError::Pending,
        error => AttemptError::Failed(format!("fetch: {error:?}: {error}")),
      }),
    Work::Download(entry) => Box::pin(gallery::run_media_download(app.clone(), *entry))
      .await
      .map(|()| None)
      .map_err(|error| match error {
        YtdlpDownloadError::LoginPending => AttemptError::Pending,
        error => AttemptError::Failed(format!("download: {error:?}: {error}")),
      }),
    Work::Stopped(reason) => Err(AttemptError::Failed(reason.into())),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn retry_futures_fit_the_default_worker_stack() {
    fn execute_size<F>(_: impl FnOnce(&'static tauri::AppHandle, Work) -> F) -> usize {
      std::mem::size_of::<F>()
    }
    fn retry_size<F>(
      _: impl FnOnce(&'static mut RowRetries, &'static tauri::AppHandle, Work) -> F,
    ) -> usize {
      std::mem::size_of::<F>()
    }
    for (name, bytes) in [
      ("execute", execute_size(execute)),
      ("resume", retry_size(RowRetries::run)),
    ] {
      println!("{name} future: {bytes} bytes");
      assert!(
        bytes <= 16 * 1024,
        "{name} exceeds nested harness frame budget: {bytes}"
      );
    }
  }

  fn fetch(group: &str, history: bool) -> FetchEntry {
    FetchEntry {
      group_id: group.into(),
      id: group.into(),
      url: "https://instagram.com/p/public/".into(),
      total: 1,
      format: None,
      overrides: None,
      login_browser: history.then(|| "chrome:Default".into()),
      login_was_accepted: history,
      prior_attempts: if history {
        vec![crate::extract::outcome::Attempt::failed(
          crate::extract::outcome::StageId::Ytdlp,
          crate::extract::outcome::AuthUsed::Anonymous,
          crate::extract::outcome::Confidence::Medium,
          crate::extract::outcome::classify(
            "signInRequired",
            "",
            crate::extract::outcome::NetworkScope::StageLocal,
          ),
          0,
        )]
      } else {
        vec![]
      },
      prior_resolved_url: history.then(|| "https://instagram.com/p/resolved/".into()),
    }
  }
  fn download(group: &str) -> DownloadEntry {
    let item = serde_json::from_value(serde_json::json!({
      "id":group,"url":"https://instagram.com/p/public/","format":{"trackType":"both"},"templateContext":{"values":{}}
    })).unwrap();
    DownloadEntry::from((item, group.into()))
  }
  #[tokio::test]
  async fn production_channels_route_fetch_history_and_downloads_to_their_own_row() {
    let router = ResumeRouter::default();
    let mut first = router.register("fetch", Some("chrome:Default"));
    let mut second = router.register("download", Some("chrome:Default"));
    let (fetch_tx, fetch_rx) = mpsc::unbounded_channel();
    let (download_tx, download_rx) = mpsc::unbounded_channel();
    download_tx
      .send(DispatchRequest::Pipeline(DownloadRequest::Resume(
        Box::new(download("download")),
      )))
      .unwrap();
    fetch_tx
      .send(DispatchRequest::Pipeline(FetchRequest::Resume(Box::new(
        fetch("fetch", true),
      ))))
      .unwrap();
    drop(fetch_tx);
    drop(download_tx);
    router.drain(fetch_rx, download_rx).await;
    let mut calls = 0;
    first
      .follow(Work::Fetch(Box::new(fetch("fetch", false))), |work| {
        calls += 1;
        let Work::Fetch(entry) = work else {
          panic!("wrong row or work kind")
        };
        if calls == 2 {
          assert_eq!(entry.login_browser.as_deref(), Some("chrome:Default"));
          assert_eq!(entry.prior_attempts.len(), 1);
          assert_eq!(
            entry.prior_resolved_url.as_deref(),
            Some("https://instagram.com/p/resolved/")
          );
        }
        std::future::ready(if calls == 1 {
          Err(AttemptError::Pending)
        } else {
          Ok(None)
        })
      })
      .await
      .unwrap();
    assert_eq!(calls, 2);
    let mut calls = 0;
    second
      .follow(Work::Download(Box::new(download("download"))), |work| {
        calls += 1;
        assert!(matches!(work, Work::Download(entry) if entry.group_id == "download"));
        std::future::ready(if calls == 1 {
          Err(AttemptError::Pending)
        } else {
          Ok(None)
        })
      })
      .await
      .unwrap();
    assert_eq!(calls, 2);
  }

  #[tokio::test]
  async fn resume_limit_is_shared_by_fetch_and_download() {
    let router = ResumeRouter::default();
    let mut row = router.register("row", Some("chrome:Default"));
    router.send("row", Work::Fetch(Box::new(fetch("row", true))));
    let mut calls = 0;
    row
      .follow(Work::Fetch(Box::new(fetch("row", false))), |_| {
        calls += 1;
        std::future::ready(if calls == 1 {
          Err(AttemptError::Pending)
        } else {
          Ok(None)
        })
      })
      .await
      .unwrap();
    for _ in 0..3 {
      router.send("row", Work::Download(Box::new(download("row"))));
    }
    let mut calls = 0;
    let result = row
      .follow(Work::Download(Box::new(download("row"))), |_| {
        calls += 1;
        std::future::ready(Err(AttemptError::Pending))
      })
      .await;
    assert_eq!(
      result.unwrap_err(),
      "Harness automatic-login resume limit: 2 per row"
    );
    assert_eq!(calls, 2, "the fetch already used one resume");
  }

  #[tokio::test]
  async fn terminal_events_stop_automatic_waiters_and_new_consent_is_never_replayed() {
    for (kind, value) in [
      (
        "media_login_needed",
        serde_json::json!({"groupId":"row","reason":"signInRequired","status":"needed"}),
      ),
      (
        "media_login_needed",
        serde_json::json!({"groupId":"row","reason":"browserLoginRemedy","status":"exhausted"}),
      ),
      ("media_fatal", serde_json::json!({"groupId":"row"})),
    ] {
      let router = ResumeRouter::default();
      let mut row = router.register("row", Some("chrome:Default"));
      router.event(kind, &value);
      let result = row
        .follow(Work::Fetch(Box::new(fetch("row", false))), |_| {
          std::future::ready(Err(AttemptError::Pending))
        })
        .await;
      assert!(result.unwrap_err().starts_with("Login required:"));
      assert_eq!(row.resumes, 0);
    }
    let router = ResumeRouter::default();
    let mut row = router.register("manual", None);
    router.send("manual", Work::Fetch(Box::new(fetch("manual", true))));
    let result = row
      .follow(Work::Fetch(Box::new(fetch("manual", false))), |_| {
        std::future::ready(Err(AttemptError::Pending))
      })
      .await;
    assert_eq!(result.unwrap_err(), "Login required: new consent needed");
    assert_eq!(row.resumes, 0);
  }

  #[tokio::test(start_paused = true)]
  async fn waiting_for_a_resume_keeps_the_original_row_deadline() {
    let router = ResumeRouter::default();
    let mut row = router.register("row", Some("chrome:Default"));
    let began = tokio::time::Instant::now();
    let result = tokio::time::timeout_at(
      began + std::time::Duration::from_secs(10),
      row.follow(Work::Fetch(Box::new(fetch("row", false))), |_| {
        std::future::ready(Err(AttemptError::Pending))
      }),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(began.elapsed(), std::time::Duration::from_secs(10));
    drop(row);
    assert!(router.0.lock().unwrap().is_empty());
  }
}
