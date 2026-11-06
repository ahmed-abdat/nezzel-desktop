//! The one way capture sends anything to the main window: a numbered queue it pulls (`capture_pull`).
//! The `capture_wake` event only says "pull now". Linux can hold an event for a hidden window until the
//! next user input (tauri#3654); the main window also pulls every `PULL_MS`, so nothing waits on that.

use super::state::{wait_for_item, CaptureAction, CaptureReport, CaptureState};
use serde::Serialize;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// How long, and how many, messages wait for the main window: it pulls every `PULL_MS`, so anything older
/// belongs to a main window that is gone or stuck.
pub(crate) const OUTBOX_TTL: Duration = Duration::from_secs(120);
pub(crate) const OUTBOX_CAP: usize = 50;
/// How often the main window pulls (`crate::contract`).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const PULL_MS: u64 = 1500;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(super) enum ForMain {
  /// A claim on an offer (`Action::CLAIMS`).
  Action(CaptureAction),
  /// The media item of a row whose claim went out before its read finished. One per row (`Offers::claim`).
  Item(CaptureItem),
  /// The clipboard's supported link for the main window's link field ("" = none). Only the newest waits.
  Prefill { url: String },
  /// A failed read → Report: open Report a problem.
  Report(CaptureReport),
}

#[derive(Clone, Serialize)]
pub(super) struct CaptureItem {
  pub(super) id: u64,
  pub(super) item: Option<serde_json::Value>,
}

/// One queued message. `seq` grows by one per message for the life of the app.
#[derive(Clone, Serialize)]
pub struct Message {
  seq: u64,
  #[serde(flatten)]
  body: ForMain,
}

#[derive(Default)]
pub(super) struct Outbox {
  last_seq: u64,
  queue: VecDeque<(Instant, Message)>,
}

impl Outbox {
  /// Queues `body` as the next seq: drops entries past the TTL and an older prefill, then the oldest past the cap.
  fn push(&mut self, now: Instant, body: ForMain) -> u64 {
    self.forget_expired(now);
    if matches!(body, ForMain::Prefill { .. }) {
      self
        .queue
        .retain(|(_, m)| !matches!(m.body, ForMain::Prefill { .. }));
    }
    self.last_seq += 1;
    let seq = self.last_seq;
    self.queue.push_back((now, Message { seq, body }));
    while self.queue.len() > OUTBOX_CAP {
      self.queue.pop_front();
    }
    seq
  }

  /// What the main window has not handled, oldest first. Asking for what follows `after` acknowledges
  /// everything up to it: that is gone for good. The rest stays until acknowledged (or expired), so a pull
  /// whose answer never arrives loses nothing.
  fn after(&mut self, after: u64, now: Instant) -> Vec<Message> {
    self.forget_expired(now);
    while self.queue.front().is_some_and(|(_, m)| m.seq <= after) {
      self.queue.pop_front();
    }
    self.queue.iter().map(|(_, m)| m.clone()).collect()
  }

  fn forget_expired(&mut self, now: Instant) {
    self
      .queue
      .retain(|(at, _)| now.duration_since(*at) < OUTBOX_TTL);
  }
}

/// Queues `body` for the main window and wakes it.
pub(super) fn send_to_main(app: &AppHandle, body: ForMain) {
  app
    .state::<CaptureState>()
    .pending
    .lock()
    .unwrap()
    .push(Instant::now(), body);
  // Wake-up only: a held or failed event costs at most one pull interval.
  let _ = app.emit_to("main", "capture_wake", ());
}

/// Sends a claim; when its row's item is still being read, the item follows once the read ends.
pub(super) fn emit_action(app: &AppHandle, action: CaptureAction) {
  let (id, awaits_item) = (action.id, action.awaits_item);
  tracing::info!(id, action = %action.action, "Capture action sent to the main window");
  send_to_main(app, ForMain::Action(action));
  if awaits_item {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
      let item = wait_for_item(&app.state::<CaptureState>(), id).await;
      send_to_main(&app, ForMain::Item(CaptureItem { id, item }));
    });
  }
}

pub(super) fn pull(state: &CaptureState, after: u64) -> Vec<Message> {
  state.pending.lock().unwrap().after(after, Instant::now())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::capture::state::{adopt_on_focus, claim_action, tests::pending_offer, Action};
  use crate::capture::window::CaptureWindow;

  fn prefill(url: &str) -> ForMain {
    ForMain::Prefill { url: url.into() }
  }
  fn seqs(messages: &[Message]) -> Vec<u64> {
    messages.iter().map(|m| m.seq).collect()
  }

  #[test]
  fn messages_come_in_seq_order_until_acknowledged() {
    let now = Instant::now();
    let mut outbox = Outbox::default();
    for url in ["a", "b"] {
      outbox.push(now, ForMain::Report(CaptureReport::for_test(url)));
    }
    assert_eq!(seqs(&outbox.after(0, now)), [1, 2]);
    // An unanswered pull loses nothing: the same messages until the main window asks past them.
    assert_eq!(seqs(&outbox.after(0, now)), [1, 2]);
    assert_eq!(seqs(&outbox.after(1, now)), [2]);
    outbox.push(now, prefill(""));
    assert_eq!(seqs(&outbox.after(2, now)), [3]);
    assert!(outbox.after(3, now).is_empty());
    // Acknowledged is gone: a main window that starts over (reload) never sees them again.
    assert!(outbox.after(0, now).is_empty());
  }

  #[test]
  fn the_outbox_forgets_old_messages_and_keeps_at_most_the_cap() {
    let start = Instant::now();
    let mut outbox = Outbox::default();
    outbox.push(start, prefill("old"));
    outbox.push(
      start + OUTBOX_TTL,
      ForMain::Report(CaptureReport::for_test("x")),
    );
    assert_eq!(seqs(&outbox.after(0, start + OUTBOX_TTL)), [2]);
    for n in 0..(OUTBOX_CAP + 10) {
      outbox.push(
        start + OUTBOX_TTL,
        ForMain::Report(CaptureReport::for_test(&n.to_string())),
      );
    }
    let kept = outbox.after(0, start + OUTBOX_TTL);
    assert_eq!(kept.len(), OUTBOX_CAP);
    // The oldest went first; seq keeps counting.
    assert_eq!(kept.last().unwrap().seq, OUTBOX_CAP as u64 + 12);
    assert!(outbox.after(0, start + OUTBOX_TTL * 3).is_empty());
  }

  #[test]
  fn only_the_newest_clipboard_prefill_waits() {
    let now = Instant::now();
    let mut outbox = Outbox::default();
    outbox.push(now, prefill("https://a.example/1"));
    outbox.push(now, ForMain::Report(CaptureReport::for_test("r")));
    outbox.push(now, prefill(""));
    let wire: Vec<serde_json::Value> = outbox
      .after(0, now)
      .iter()
      .map(|m| serde_json::to_value(m).unwrap())
      .collect();
    assert_eq!(wire.len(), 2);
    assert_eq!(wire[0]["kind"], "report");
    assert_eq!(
      wire[1],
      serde_json::json!({"seq": 3, "kind": "prefill", "url": ""})
    );
  }

  #[test]
  fn an_action_is_one_flat_message() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, true);
    let action = claim_action(&state, CaptureWindow::Card, 1, Action::Download).unwrap();
    state
      .pending
      .lock()
      .unwrap()
      .push(Instant::now(), ForMain::Action(action));
    let wire = serde_json::to_value(&pull(&state, 0)[0]).unwrap();
    assert_eq!(
      wire,
      serde_json::json!({
        "seq": 1, "kind": "action", "id": 1, "url": "https://www.youtube.com/watch?v=capture",
        "action": "download", "item": null, "choice": null,
      })
    );
  }

  /// Each row's item goes out once: with the row's first claim, never again with an upgrade.
  #[test]
  fn a_row_gets_its_item_once() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, true);
    let (_, adopt) = adopt_on_focus(&state).unwrap();
    assert!(adopt.awaits_item && adopt.item.is_none());
    // The read finishes; the click that upgrades the adopted row carries no second item.
    state.offers.lock().unwrap().record_read(
      CaptureWindow::Card,
      1,
      Some(serde_json::json!({"title": "t"})),
      None,
      None,
    );
    let upgrade = claim_action(&state, CaptureWindow::Card, 1, Action::Download).unwrap();
    assert!(!upgrade.awaits_item && upgrade.item.is_none());

    // Read before the claim: the item rides along and nothing is awaited.
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Bar, false);
    state.offers.lock().unwrap().record_read(
      CaptureWindow::Bar,
      1,
      Some(serde_json::json!({"title": "t"})),
      None,
      None,
    );
    let download = claim_action(&state, CaptureWindow::Bar, 1, Action::Download).unwrap();
    assert!(!download.awaits_item && download.item.is_some());
  }
}
