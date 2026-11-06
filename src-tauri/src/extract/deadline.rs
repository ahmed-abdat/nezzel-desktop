//! Stage and row clocks. One deadline is shared by nested readers and subprocesses.
use super::outcome::StageId;
use std::{future::Future, time::Duration};
use tokio::{sync::watch, time::Instant as Deadline};

// Shared by nested readers and subprocesses; retries cannot reset the stage/row clock.
tokio::task_local! {
  static ACTIVE_DEADLINE: Deadline;
}
pub(crate) const FETCH_TIMEOUT: &str = "fetchTimeout";

pub(crate) fn remaining_budget(cap: Duration) -> Duration {
  ACTIVE_DEADLINE
    .try_with(|deadline| deadline.saturating_duration_since(Deadline::now()).min(cap))
    .unwrap_or(cap)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interrupted {
  Cancelled,
  Timeout,
}

pub(crate) async fn wait_until<T>(
  future: impl Future<Output = T>,
  deadline: Deadline,
  cancel: &mut watch::Receiver<bool>,
) -> Result<T, Interrupted> {
  tokio::pin!(future);
  loop {
    if !*cancel.borrow() {
      return Err(Interrupted::Cancelled);
    }
    tokio::select! {
      biased;
      changed = cancel.changed() => {
        if changed.is_err() || !*cancel.borrow() {
          return Err(Interrupted::Cancelled);
        }
      }
      // Poll nested deadlines first so a stage can return the evidence it retained on expiry.
      result = &mut future => return Ok(result),
      _ = tokio::time::sleep_until(deadline) => return Err(Interrupted::Timeout),
    }
  }
}

/// Run one stage future under `deadline`; nested `remaining_budget` calls see the same clock.
pub(super) async fn within_deadline<T>(
  future: impl Future<Output = T>,
  deadline: Deadline,
  cancel: &mut watch::Receiver<bool>,
) -> Result<T, Interrupted> {
  ACTIVE_DEADLINE
    .scope(deadline, wait_until(future, deadline, cancel))
    .await
}

pub(super) fn stage_budget(stage: StageId, ytdlp: Duration) -> Duration {
  match stage {
    StageId::Files => ytdlp + Duration::from_secs(20),
    StageId::Gallery => Duration::from_secs(45),
    StageId::Reader(_) => Duration::from_secs(20),
    // Readers and PageScan may hand a candidate to a separately bounded yt-dlp process.
    StageId::Ytdlp | StageId::PageScan => ytdlp + Duration::from_secs(20),
  }
}
