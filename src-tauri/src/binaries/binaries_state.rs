use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct BinariesState {
  running: AtomicBool,
  pub(crate) update_stale: AtomicBool,
}

impl BinariesState {
  /// Seed the snapshot before webview listeners start; do not wait for the next daily check.
  pub(crate) fn from_metadata(path: &std::path::Path, now: u64) -> Self {
    let meta = std::fs::read(path)
      .ok()
      .and_then(|bytes| serde_json::from_slice::<super::binaries_manager::Metadata>(&bytes).ok());
    Self {
      update_stale: AtomicBool::new(
        meta
          .is_some_and(|meta| !meta.is_locked && super::ytdlp_update::update_is_stale(&meta, now)),
      ),
      ..Self::default()
    }
  }

  pub(crate) fn try_start(&self) -> bool {
    self
      .running
      .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
      .is_ok()
  }
  pub(crate) fn finish(&self) {
    self.running.store(false, Ordering::SeqCst);
  }
}

#[derive(Serialize)]
pub struct CheckResult {
  pub tools: Vec<String>,
}
