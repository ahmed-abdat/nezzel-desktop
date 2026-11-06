use crate::scheduling::concurrency::DynamicSemaphore;
use crate::scheduling::group_state::{is_group_running, remove_group};
use crate::scheduling::numbering::NumberingManager;
use futures::Future;
use std::{collections::VecDeque, sync::Arc};
use tauri::{AppHandle, Runtime};
use tokio::sync::mpsc;

pub trait DispatchEntry: Clone + Send + Sync + 'static {
  fn group_id(&self) -> &String;
  fn group_key(&self) -> Option<&String>;
  fn set_numbering(&mut self, autonumber: u64, group_autonumber: Option<u64>);
}

#[derive(Clone)]
pub enum DispatchRequest<Req> {
  Pipeline(Req),
  Cleanup { group_id: String },
}

pub struct GenericDispatcher<Req> {
  sender: mpsc::UnboundedSender<DispatchRequest<Req>>,
}

impl<Req> GenericDispatcher<Req>
where
  Req: Send + Sync + Clone + 'static,
{
  pub fn start<Entry, MakeEntries, RunJob, Fut, R>(
    app_handle: AppHandle<R>,
    semaphore: Arc<DynamicSemaphore>,
    make_entries: MakeEntries,
    run_job: RunJob,
  ) -> Self
  where
    R: Runtime,
    Entry: DispatchEntry,
    MakeEntries: Fn(Req) -> Vec<Entry> + Send + Sync + 'static,
    RunJob: Fn(mpsc::UnboundedSender<DispatchRequest<Req>>, AppHandle<R>, Entry) -> Fut
      + Send
      + Sync
      + 'static,
    Fut: Future<Output = ()> + Send + 'static,
  {
    let (tx, mut rx) = mpsc::unbounded_channel::<DispatchRequest<Req>>();
    let tx_loop = tx.clone();
    let sem = semaphore;
    let app_main = app_handle;
    let make_entries = Arc::new(make_entries);
    let run_job = Arc::new(run_job);

    tauri::async_runtime::spawn(async move {
      let mut numbering = NumberingManager::new();
      let mut queues: VecDeque<(String, VecDeque<Entry>)> = VecDeque::new();
      let mut pending_requeue: VecDeque<(String, VecDeque<Entry>)> = VecDeque::new();
      loop {
        while let Ok(req) = rx.try_recv() {
          match req {
            DispatchRequest::Cleanup { group_id } => {
              queues.retain(|(gid, _)| gid != &group_id);
              remove_group(&group_id);
            }
            DispatchRequest::Pipeline(inner) => {
              let mut entries = make_entries(inner.clone());
              if !entries.is_empty() {
                let gid = entries[0].group_id().clone();
                if is_group_running(&gid) {
                  for entry in entries.iter_mut() {
                    let group_key = entry.group_key();
                    let (autonumber, group_autonumber) = numbering.assign_for(group_key);
                    entry.set_numbering(autonumber, group_autonumber);
                  }

                  let q: VecDeque<Entry> = entries.into_iter().collect();
                  queues.push_back((gid, q));
                }
              }
            }
          }
        }

        queues.retain(|(gid, _)| is_group_running(gid));

        if !pending_requeue.is_empty() {
          queues.append(&mut pending_requeue);
        }

        // If no work, block until the next request.
        if queues.is_empty() {
          if let Some(req) = rx.recv().await {
            let _ = tx_loop.send(req);
            continue;
          }
          break;
        }

        while !queues.is_empty() {
          // Grab one permit
          let permit = sem.clone().acquire_owned().await;

          if queues.is_empty() {
            drop(permit);
            continue;
          }

          // Pop the next group and its next entry
          let (group_id, mut q) = queues.pop_front().unwrap();

          // Skip cancelled groups.
          let still_running = is_group_running(&group_id);
          if !still_running {
            drop(permit);
            continue;
          }

          // Grab the entry that is going to be run.
          let entry = q.pop_front().unwrap();

          tracing::trace!(
            group_id = %group_id,
            remaining_in_group = q.len(),
            "Dispatcher: scheduling entry"
          );

          let entry_group_id = entry.group_id().clone();

          // Dispatch the entry with the permit.
          {
            let tx_job = tx_loop.clone();
            let app2 = app_main.clone();
            let run_job = run_job.clone();
            tauri::async_runtime::spawn(async move {
              tracing::trace!(
                group_id = %entry_group_id,
                "Dispatcher: starting entry"
              );
              run_job(tx_job, app2, entry.clone()).await;
              tracing::trace!(
                group_id = %entry_group_id,
                "Dispatcher: finished entry, releasing permit"
              );
              drop(permit);
            });
          }

          // Requeue if more groups are left.
          if !q.is_empty() {
            pending_requeue.push_back((group_id, q));
          }
        }
      }
    });

    Self { sender: tx }
  }

  pub fn sender(&self) -> mpsc::UnboundedSender<DispatchRequest<Req>> {
    self.sender.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::scheduling::concurrency::DynamicSemaphore;
  use crate::scheduling::group_state::ensure_group_running;
  use std::sync::atomic::{AtomicUsize, Ordering};
  use std::sync::Arc;
  use tauri::test::mock_app;
  use tokio::sync::{Mutex, Notify, Semaphore};
  use tokio::time::{timeout, Duration};

  #[derive(Clone)]
  struct TestEntry {
    group_id: String,
    index: usize,
  }

  impl DispatchEntry for TestEntry {
    fn group_id(&self) -> &String {
      &self.group_id
    }
    fn group_key(&self) -> Option<&String> {
      None
    }
    fn set_numbering(&mut self, _autonumber: u64, _group_autonumber: Option<u64>) {}
  }

  /// Waits for the next job event; the timeout only guards against a hang, it asserts no speed.
  async fn next_event(events: &mut mpsc::UnboundedReceiver<bool>) -> bool {
    timeout(Duration::from_secs(30), events.recv())
      .await
      .expect("dispatcher stalled")
      .expect("job events closed")
  }

  /// Jobs hold until the test releases them one at a time (no real sleeps): each round waits until
  /// every free slot is taken, so the peak is reached however slow the machine is.
  async fn run_concurrency_test(max_concurrency: usize, total: usize) -> usize {
    let app = mock_app();
    let group_id = "test-group".to_string();
    ensure_group_running(&group_id);

    let sem = Arc::new(DynamicSemaphore::new(max_concurrency));
    let current = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Semaphore::new(0));
    // true = a job started, false = a job finished.
    let (events_tx, mut events) = mpsc::unbounded_channel::<bool>();

    let current_job = current.clone();
    let max_job = max_seen.clone();
    let release_job = release.clone();

    let dispatcher = GenericDispatcher::start(
      app.handle().clone(),
      sem,
      {
        let group_id = group_id.clone();
        move |count: usize| {
          (0..count)
            .map(|index| TestEntry {
              group_id: group_id.clone(),
              index,
            })
            .collect()
        }
      },
      move |_tx, _app, _entry: TestEntry| {
        let current_job = current_job.clone();
        let max_job = max_job.clone();
        let release_job = release_job.clone();
        let events_tx = events_tx.clone();
        async move {
          let in_flight = current_job.fetch_add(1, Ordering::SeqCst) + 1;
          max_job.fetch_max(in_flight, Ordering::SeqCst);
          let _ = events_tx.send(true);

          release_job.acquire().await.expect("release gate").forget();
          current_job.fetch_sub(1, Ordering::SeqCst);
          let _ = events_tx.send(false);
        }
      },
    );

    dispatcher
      .sender()
      .send(DispatchRequest::Pipeline(total))
      .unwrap();

    let (mut running, mut finished) = (0, 0);
    while finished < total {
      while running < max_concurrency.min(total - finished) {
        if next_event(&mut events).await {
          running += 1;
        }
      }
      release.add_permits(1);
      loop {
        if next_event(&mut events).await {
          running += 1;
        } else {
          running -= 1;
          finished += 1;
          break;
        }
      }
    }

    max_seen.load(Ordering::SeqCst)
  }

  #[tokio::test]
  async fn dispatcher_respects_single_concurrency() {
    let max_seen = run_concurrency_test(1, 3).await;
    assert_eq!(max_seen, 1);
  }

  #[tokio::test]
  async fn dispatcher_allows_two_concurrent_jobs() {
    let max_seen = run_concurrency_test(2, 4).await;
    assert_eq!(max_seen, 2);
  }

  #[tokio::test]
  async fn dispatcher_round_robins_groups_with_single_permit() {
    let app = mock_app();
    let group_a = "group-a".to_string();
    let group_b = "group-b".to_string();
    ensure_group_running(&group_a);
    ensure_group_running(&group_b);

    let sem = Arc::new(DynamicSemaphore::new(1));
    // Hold the only permit until both groups are sent, so the first pick sees both (no real sleeps).
    let held = sem.acquire_owned().await;
    let started: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(Notify::new());
    let total = 4usize;
    let completed = Arc::new(AtomicUsize::new(0));

    let started_job = started.clone();
    let done_job = done.clone();
    let completed_job = completed.clone();

    let dispatcher = GenericDispatcher::start(
      app.handle().clone(),
      sem,
      {
        move |req: (String, usize)| {
          let (group_id, count) = req;
          (0..count)
            .map(|index| TestEntry {
              group_id: group_id.clone(),
              index,
            })
            .collect::<Vec<_>>()
        }
      },
      move |_tx, _app, entry: TestEntry| {
        let started_job = started_job.clone();
        let done_job = done_job.clone();
        let completed_job = completed_job.clone();
        async move {
          {
            let mut guard = started_job.lock().await;
            guard.push(entry.group_id.clone());
          }
          let done_count = completed_job.fetch_add(1, Ordering::SeqCst) + 1;
          if done_count == total {
            done_job.notify_one();
          }
        }
      },
    );

    dispatcher
      .sender()
      .send(DispatchRequest::Pipeline((group_a.clone(), 2)))
      .unwrap();
    dispatcher
      .sender()
      .send(DispatchRequest::Pipeline((group_b.clone(), 2)))
      .unwrap();
    drop(held);

    timeout(Duration::from_secs(30), done.notified())
      .await
      .expect("dispatcher did not complete within timeout");

    let order = started.lock().await;
    assert!(order.len() >= 2, "expected at least two started entries");
    assert_ne!(order[0], order[1], "expected round-robin across groups");
  }

  #[tokio::test]
  async fn dispatcher_does_not_starve_new_group() {
    let app = mock_app();
    let group_a = "group-a".to_string();
    let group_b = "group-b".to_string();
    ensure_group_running(&group_a);
    ensure_group_running(&group_b);

    let sem = Arc::new(DynamicSemaphore::new(1));
    let started: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(Notify::new());
    let completed = Arc::new(AtomicUsize::new(0));
    let total = 4usize;
    let gate = Arc::new(Notify::new());
    let gate_open = Arc::new(AtomicUsize::new(0));
    let first_started = Arc::new(Notify::new());

    let started_job = started.clone();
    let done_job = done.clone();
    let completed_job = completed.clone();
    let gate_job = gate.clone();
    let gate_open_job = gate_open.clone();
    let first_started_job = first_started.clone();
    let group_a_run = group_a.clone();

    let dispatcher = GenericDispatcher::start(
      app.handle().clone(),
      sem,
      {
        move |req: (String, usize)| {
          let (group_id, count) = req;
          (0..count)
            .map(|index| TestEntry {
              group_id: group_id.clone(),
              index,
            })
            .collect::<Vec<_>>()
        }
      },
      move |_tx, _app, entry: TestEntry| {
        let started_job = started_job.clone();
        let done_job = done_job.clone();
        let completed_job = completed_job.clone();
        let gate_job = gate_job.clone();
        let gate_open_job = gate_open_job.clone();
        let first_started_job = first_started_job.clone();
        let group_a = group_a_run.clone();
        async move {
          {
            let mut guard = started_job.lock().await;
            guard.push(format!("{}:{}", entry.group_id, entry.index));
          }

          if entry.group_id == group_a
            && entry.index == 0
            && gate_open_job.fetch_add(1, Ordering::SeqCst) == 0
          {
            first_started_job.notify_one();
            gate_job.notified().await;
          }

          let done_count = completed_job.fetch_add(1, Ordering::SeqCst) + 1;
          if done_count == total {
            done_job.notify_one();
          }
        }
      },
    );

    dispatcher
      .sender()
      .send(DispatchRequest::Pipeline((group_a.clone(), 3)))
      .unwrap();

    timeout(Duration::from_secs(30), first_started.notified())
      .await
      .expect("first job did not start");

    dispatcher
      .sender()
      .send(DispatchRequest::Pipeline((group_b.clone(), 1)))
      .unwrap();
    gate.notify_one();

    timeout(Duration::from_secs(30), done.notified())
      .await
      .expect("dispatcher did not complete within timeout");

    let order = started.lock().await;
    let idx_b = order
      .iter()
      .position(|v| v.starts_with(&group_b))
      .expect("expected group B to start");
    let idx_a_last = order
      .iter()
      .position(|v| v == &format!("{group_a}:2"))
      .expect("expected third group A entry to start");
    assert!(
      idx_b < idx_a_last,
      "expected new group to run before the last queued entry of existing group"
    );
  }
}
