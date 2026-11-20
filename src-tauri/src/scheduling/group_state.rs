use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex as StdMutex};
use tokio::sync::watch;

/// A stop, a (re)start and a download start on a group that is not downloading give the group a
/// new generation, so a Cleanup sent for an older run (a pause that was already resumed, a fetch
/// that finished before the download) is ignored. Calls on a running group join its current run.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

struct GroupRun {
  running: watch::Sender<bool>,
  generation: u64,
  /// Downloads of this generation are queued or running (`start_group_run` until
  /// `finish_group_run`).
  downloading: bool,
}

static RUNNING_GROUPS: LazyLock<StdMutex<HashMap<String, GroupRun>>> =
  LazyLock::new(|| StdMutex::new(HashMap::new()));

fn next_generation() -> u64 {
  NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// Marks the group running. `join` decides whether a running group keeps its generation.
fn run_group(group_id: &str, join: impl Fn(&GroupRun) -> bool, downloading: bool) -> u64 {
  let mut map = RUNNING_GROUPS.lock().unwrap();
  match map.get_mut(group_id) {
    Some(run) if *run.running.borrow() && join(run) => {
      run.downloading |= downloading;
      run.generation
    }
    Some(run) => {
      run.generation = next_generation();
      run.downloading = downloading;
      run.running.send_replace(true);
      run.generation
    }
    None => {
      let generation = next_generation();
      map.insert(
        group_id.to_string(),
        GroupRun {
          running: watch::channel(true).0,
          generation,
          downloading,
        },
      );
      generation
    }
  }
}

/// Marks the group running and returns its generation: a new one only if it was not running
/// (absent, paused or stopped), so jobs of the current run keep theirs.
pub fn ensure_group_running(group_id: &str) -> u64 {
  run_group(group_id, |_| true, false)
}

/// A download start: joins a run that is already downloading; otherwise a new generation, so a
/// Cleanup from the group's finished fetch (or finished downloads) that is still queued cannot
/// drop it.
pub fn start_group_run(group_id: &str) -> u64 {
  run_group(group_id, |run| run.downloading, true)
}

/// All downloads of `generation` ended; the next download start begins a new generation.
pub fn finish_group_run(group_id: &str, generation: u64) {
  if let Some(run) = RUNNING_GROUPS.lock().unwrap().get_mut(group_id) {
    if run.generation == generation {
      run.downloading = false;
    }
  }
}

/// The generation a finished fetch's Cleanup carries: `None` (keep it while running) when the
/// group is downloading, so the fetch never ends a download that started before it finished.
pub fn fetch_done_generation(group_id: &str) -> Option<u64> {
  RUNNING_GROUPS
    .lock()
    .unwrap()
    .get(group_id)
    .filter(|run| !run.downloading)
    .map(|run| run.generation)
}

/// Marks the group stopped and returns the generation of this stop.
pub fn cancel_group(group_id: &str) -> u64 {
  let generation = next_generation();
  // Publish cancellation before Cleanup can remove the watch sender.
  {
    let mut map = RUNNING_GROUPS.lock().unwrap();
    if let Some(run) = map.get_mut(group_id) {
      run.running.send_replace(false);
      run.generation = generation;
      run.downloading = false;
    } else {
      map.insert(
        group_id.to_string(),
        GroupRun {
          running: watch::channel(false).0,
          generation,
          downloading: false,
        },
      );
    }
  }
  crate::runners::login::forget_group(group_id);
  generation
}

pub fn remove_group(group_id: &str) {
  RUNNING_GROUPS.lock().unwrap().remove(group_id);
}

pub fn group_generation(group_id: &str) -> Option<u64> {
  RUNNING_GROUPS
    .lock()
    .unwrap()
    .get(group_id)
    .map(|run| run.generation)
}

/// Removes the group only if it is still at `generation` (`None`: only if it is not running).
pub fn remove_group_if(group_id: &str, generation: Option<u64>) -> bool {
  let mut map = RUNNING_GROUPS.lock().unwrap();
  let stale = match (map.get(group_id), generation) {
    (Some(run), Some(generation)) => run.generation == generation,
    (Some(run), None) => !*run.running.borrow(),
    (None, _) => false,
  };
  if stale {
    map.remove(group_id);
  }
  // Login scheduling takes its attempt lock before reading group state.
  drop(map);
  if stale {
    crate::runners::login::automatic::forget_group(group_id);
  }
  stale
}

pub fn subscribe_group(group_id: &str) -> watch::Receiver<bool> {
  let mut map = RUNNING_GROUPS.lock().unwrap();
  if let Some(run) = map.get(group_id) {
    run.running.subscribe()
  } else {
    let (running, rx) = watch::channel(true);
    map.insert(
      group_id.to_string(),
      GroupRun {
        running,
        generation: next_generation(),
        downloading: false,
      },
    );
    rx
  }
}

pub fn is_group_running(group_id: &str) -> bool {
  RUNNING_GROUPS
    .lock()
    .unwrap()
    .get(group_id)
    .map(|run| *run.running.borrow())
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Sets up a group and returns the generation its Cleanup carries.
  type Setup = fn(&str) -> Option<u64>;

  #[test]
  fn remove_group_if_removes_only_the_run_it_was_sent_for() {
    // (name, setup, kept afterwards)
    let cases: [(&str, Setup, bool); 6] = [
      ("current run", |g| Some(ensure_group_running(g)), false),
      (
        "stale: resumed after pause",
        |g| {
          let paused = cancel_group(g);
          ensure_group_running(g);
          Some(paused)
        },
        true,
      ),
      ("current pause", |g| Some(cancel_group(g)), false),
      (
        "stale: fetch finished, then download started",
        |g| {
          let fetched = ensure_group_running(g);
          start_group_run(g);
          Some(fetched)
        },
        true,
      ),
      (
        "no generation, still running",
        |g| {
          ensure_group_running(g);
          None
        },
        true,
      ),
      (
        "no generation, stopped",
        |g| {
          cancel_group(g);
          None
        },
        false,
      ),
    ];
    for (name, setup, kept) in cases {
      let group = format!("generation-{name}");
      let generation = setup(&group);
      assert_eq!(remove_group_if(&group, generation), !kept, "{name}");
      assert_eq!(group_generation(&group).is_some(), kept, "{name}");
      remove_group(&group);
    }
  }

  #[test]
  fn fetch_on_a_running_group_joins_its_run_and_resume_starts_a_new_one() {
    let group = "generation-join";
    let run = start_group_run(group);
    assert_eq!(
      ensure_group_running(group),
      run,
      "media_size/media_info join"
    );
    let paused = cancel_group(group);
    let resumed = ensure_group_running(group);
    assert!(resumed != run && resumed != paused);
    assert_eq!(ensure_group_running(group), resumed);
    remove_group(group);
  }

  #[test]
  fn download_start_joins_a_downloading_run_and_starts_anew_once_it_finished() {
    let group = "generation-download-join";
    let run = start_group_run(group);
    assert_eq!(start_group_run(group), run, "second batch joins");
    finish_group_run(group, run);
    let retry = start_group_run(group);
    assert_ne!(retry, run, "retry after the run finished is a new run");
    remove_group(group);
  }
}
