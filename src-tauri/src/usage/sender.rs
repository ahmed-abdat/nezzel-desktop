//! The daily send: one POST a UTC day, first checked 10 s after start, then every 6 hours, and once on quit. No separate network
//! check: an offline attempt fails, keeps the counters and is retried on the next tick (counters older than 7 days
//! are dropped). Debug builds, tests and builds without `NEZZEL_USAGE_URL` never send.

use super::store::payload;
use super::{now_ms, snapshot, Usage};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const FIRST_CHECK: Duration = Duration::from_secs(10);
const EXIT_LIMIT: Duration = Duration::from_secs(2);
const EVERY: Duration = Duration::from_secs(6 * 3600);
const TIMEOUT: Duration = Duration::from_secs(15);

/// One POST of a ping body; true only for the server's 204.
pub trait Post: Send + Sync {
  fn post(&self, body: String) -> impl Future<Output = bool> + Send;
}

pub struct HttpPost {
  client: reqwest::Client,
  url: String,
}

impl HttpPost {
  pub fn new(url: String, version: &str) -> Option<Self> {
    let client = reqwest::Client::builder()
      .redirect(reqwest::redirect::Policy::none())
      .timeout(TIMEOUT)
      .connect_timeout(TIMEOUT)
      .user_agent(format!("Nezzel-Desktop/{version}"))
      .build()
      .ok()?;
    Some(Self { client, url })
  }
}

impl Post for HttpPost {
  async fn post(&self, body: String) -> bool {
    self
      .client
      .post(&self.url)
      .header(reqwest::header::CONTENT_TYPE, "application/json")
      .body(body)
      .send()
      .await
      .is_ok_and(|response| response.status().as_u16() == 204)
  }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Sent {
  Off,
  NotDue,
  Failed,
  Ok,
}

/// A ping built under the state lock, not yet sent.
pub struct Prepared {
  generation: u64,
  sent: BTreeMap<String, u32>,
  once: Vec<String>,
  body: String,
}

impl Usage {
  fn current(&self, generation: u64) -> bool {
    self.on.load(Ordering::SeqCst) && self.generation.load(Ordering::SeqCst) == generation
  }

  /// Builds today's ping, or says why not. `on` is checked under the same lock that builds the payload.
  pub fn prepare(&self, once: Vec<String>, now: u64) -> Result<Prepared, Sent> {
    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    if !self.on.load(Ordering::SeqCst) {
      return Err(Sent::Off);
    }
    state.expire(now);
    let due = state.due(now);
    let built = due.then(|| payload(&state.counts, &once, &self.version));
    self.persist(&state);
    let (sent, body) = built.ok_or(Sent::NotDue)?;
    Ok(Prepared {
      generation: self.generation.load(Ordering::SeqCst),
      sent,
      once,
      body,
    })
  }

  /// Sends a prepared ping. A switch-off before the POST starts sends nothing; one during it drops the POST and
  /// keeps nothing.
  pub async fn deliver<P: Post>(&self, prepared: Prepared, post: &P, now: u64) -> Sent {
    let cancelled = self.cancel.notified();
    tokio::pin!(cancelled);
    cancelled.as_mut().enable();
    if !self.current(prepared.generation) {
      return Sent::Off;
    }
    let ok = tokio::select! {
      biased;
      _ = &mut cancelled => return Sent::Off,
      ok = post.post(prepared.body) => ok,
    };
    if !ok {
      return Sent::Failed;
    }
    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    if !self.current(prepared.generation) {
      return Sent::Off;
    }
    state.sent(&prepared.sent, &prepared.once, now);
    self.persist(&state);
    Sent::Ok
  }

  /// One daily send. `once` = the snapshot keys (value 1). Single-flight: a caller that finds a send running
  /// waits for it, then builds its own ping (usually "not due": the running one sent today's).
  pub async fn send_once<P: Post>(&self, post: &P, once: Vec<String>, now: u64) -> Sent {
    let _one = self.sending.lock().await;
    match self.prepare(once, now) {
      Ok(prepared) => self.deliver(prepared, post, now).await,
      Err(why) => why,
    }
  }
}

fn transport(app: &AppHandle) -> Option<HttpPost> {
  if cfg!(debug_assertions) {
    return None;
  }
  let usage = app.try_state::<Usage>()?;
  usage
    .endpoint
    .clone()
    .and_then(|url| HttpPost::new(url, &usage.version))
}

pub fn start(app: &AppHandle) {
  let Some(post) = transport(app) else {
    return;
  };
  let app = app.clone();
  tauri::async_runtime::spawn(async move {
    tokio::time::sleep(FIRST_CHECK).await;
    loop {
      let usage = app.state::<Usage>();
      let once = snapshot::keys(&app);
      usage.send_once(&post, once, now_ms()).await;
      tokio::time::sleep(EVERY).await;
    }
  });
}

/// Quitting: a short session still sends today's ping (best effort, at most 2 s; nothing if already sent today).
/// A scheduled send still running is waited on within the same 2 s, never doubled.
pub fn on_exit(app: &AppHandle) {
  let Some(post) = transport(app) else {
    return;
  };
  let Some(usage) = app.try_state::<Usage>() else {
    return;
  };
  if !usage.is_on() {
    return;
  }
  let once = snapshot::keys(app);
  let _ = tauri::async_runtime::block_on(async {
    tokio::time::timeout(EXIT_LIMIT, usage.send_once(&post, once, now_ms())).await
  });
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::usage::keys;
  use crate::usage::store::DAY_MS;
  use std::sync::atomic::AtomicUsize;
  use std::sync::{Arc, Mutex};

  const T: u64 = 20_000 * DAY_MS + 3_600_000;

  #[derive(Default)]
  struct Mock {
    ok: bool,
    /// Never answers (a slow server), after running `during`.
    hang: bool,
    bodies: Mutex<Vec<String>>,
    calls: AtomicUsize,
    /// Run during the POST, e.g. to turn the switch off mid-send.
    during: Option<Box<dyn Fn() + Send + Sync>>,
  }

  impl Post for Mock {
    async fn post(&self, body: String) -> bool {
      self.calls.fetch_add(1, Ordering::SeqCst);
      self.bodies.lock().unwrap().push(body);
      if let Some(during) = &self.during {
        during();
      }
      if self.hang {
        std::future::pending::<()>().await;
      }
      self.ok
    }
  }

  fn usage() -> Usage {
    let usage = Usage::new(
      None,
      Some("https://x.test/api/usage".into()),
      "0.1.0-beta.1".into(),
    );
    usage.set_on(true);
    usage
  }

  fn count(usage: &Usage, keys: &[&str]) {
    usage.with_on(|state| {
      for key in keys {
        state.count(key, T);
      }
    });
  }

  fn once() -> Vec<String> {
    vec![keys::ACTIVE.into(), "os.macos".into(), "arch.arm64".into()]
  }

  fn ok() -> Mock {
    Mock {
      ok: true,
      ..Mock::default()
    }
  }

  #[tokio::test]
  async fn a_204_resets_the_sent_counts_and_waits_for_the_next_day() {
    let usage = usage();
    count(
      &usage,
      &[keys::OPEN, keys::FUNNEL_LINK, "sitein:youtube.com"],
    );
    let mock = ok();
    assert_eq!(usage.send_once(&mock, once(), T).await, Sent::Ok);
    assert!(usage.snapshot_state().counts.is_empty());
    let body: serde_json::Value = serde_json::from_str(&mock.bodies.lock().unwrap()[0]).unwrap();
    assert_eq!(body["platform"], "desktop");
    assert_eq!(body["counts"]["sitein:youtube.com"], 1);
    assert_eq!(body["counts"]["active"], 1);
    println!("PAYLOAD {body}");
    // Same UTC day: nothing more is sent.
    assert_eq!(
      usage.send_once(&mock, once(), T + 3_600_000).await,
      Sent::NotDue
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    // Next day: a ping with only the once-keys still goes (active computers per version).
    assert_eq!(usage.send_once(&mock, once(), T + DAY_MS).await, Sent::Ok);
  }

  #[tokio::test]
  async fn an_offline_failure_keeps_the_counters_for_the_next_tick() {
    let usage = usage();
    count(&usage, &[keys::OPEN]);
    assert_eq!(
      usage.send_once(&Mock::default(), once(), T).await,
      Sent::Failed
    );
    assert_eq!(usage.snapshot_state().counts.get(keys::OPEN), Some(&1));
    assert!(usage.snapshot_state().due(T));
    assert_eq!(
      usage.send_once(&ok(), once(), T + 6 * 3_600_000).await,
      Sent::Ok
    );
    assert!(usage.snapshot_state().counts.is_empty());
  }

  #[tokio::test]
  async fn counters_unsent_for_7_days_are_dropped() {
    let usage = usage();
    count(&usage, &[keys::OPEN]);
    let offline = Mock::default();
    usage.send_once(&offline, once(), T + 8 * DAY_MS).await;
    assert!(!offline.bodies.lock().unwrap()[0].contains("\"open\""));
    assert!(usage.snapshot_state().counts.is_empty());
  }

  #[tokio::test]
  async fn switch_off_clears_counts_nothing_and_sends_nothing() {
    let usage = usage();
    count(&usage, &[keys::OPEN]);
    usage.set_on(false);
    usage.count_keys(&[keys::OPEN]);
    let mock = ok();
    assert_eq!(usage.send_once(&mock, once(), T).await, Sent::Off);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    assert!(usage.snapshot_state().counts.is_empty());
  }

  #[tokio::test]
  async fn off_between_the_check_and_the_post_sends_nothing() {
    let usage = usage();
    count(&usage, &[keys::OPEN]);
    let prepared = usage.prepare(once(), T).ok().unwrap();
    usage.set_on(false);
    let mock = ok();
    assert_eq!(usage.deliver(prepared, &mock, T).await, Sent::Off);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    assert!(usage.snapshot_state().counts.is_empty());
  }

  /// Answers only when the test opens the gate (a slow server).
  struct Gated {
    gate: tokio::sync::Notify,
    calls: AtomicUsize,
  }

  impl Post for Gated {
    async fn post(&self, _body: String) -> bool {
      self.calls.fetch_add(1, Ordering::SeqCst);
      self.gate.notified().await;
      true
    }
  }

  #[tokio::test]
  async fn quitting_during_a_scheduled_send_waits_instead_of_sending_twice() {
    let usage = usage();
    count(&usage, &[keys::OPEN]);
    let post = Gated {
      gate: tokio::sync::Notify::new(),
      calls: AtomicUsize::new(0),
    };
    let tick = usage.send_once(&post, once(), T);
    let quit = usage.send_once(&post, once(), T);
    let open = async {
      while post.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
      }
      // The quit call is parked on the single-flight lock, not in a second POST.
      for _ in 0..10 {
        tokio::task::yield_now().await;
      }
      assert_eq!(post.calls.load(Ordering::SeqCst), 1);
      post.gate.notify_one();
    };
    let (tick, quit, ()) = tokio::join!(tick, quit, open);
    assert_eq!((tick, quit), (Sent::Ok, Sent::NotDue));
    assert_eq!(post.calls.load(Ordering::SeqCst), 1);
  }

  #[tokio::test]
  async fn off_during_the_post_drops_it_and_keeps_nothing() {
    for hang in [false, true] {
      let usage = Arc::new(usage());
      count(&usage, &[keys::OPEN]);
      let u = usage.clone();
      let mock = Mock {
        ok: true,
        hang,
        during: Some(Box::new(move || u.set_on(false))),
        ..Mock::default()
      };
      assert_eq!(
        usage.send_once(&mock, once(), T).await,
        Sent::Off,
        "hang {hang}"
      );
      let state = usage.snapshot_state();
      assert!(state.counts.is_empty());
      assert_eq!(state.usage_day, None);
    }
  }
}
