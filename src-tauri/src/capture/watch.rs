//! Where links come from: the clipboard watcher (the copied-link card) and the Quick download bar.

use super::outbox::{send_to_main, ForMain};
use super::policy::{self, supported_url};
use super::state::CaptureState;
use super::window::{offer, CaptureWindow};
use super::{browser, clipboard};
use crate::SharedConfig;
use std::{
  sync::atomic::Ordering,
  time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

/// Whether the compositor offers data-control; only probed in Wayland sessions.
pub(super) async fn data_control(app: &AppHandle) -> bool {
  *app
    .state::<CaptureState>()
    .data_control
    .get_or_init(|| async {
      crate::platform::current_session() == crate::platform::Session::Wayland
        && clipboard::probe_data_control_bounded().await
    })
    .await
}

pub(super) async fn watch_clipboard(app: AppHandle) {
  let session = crate::platform::current_session();
  let data_control = data_control(&app).await;
  let order = clipboard::read_order(session, data_control);
  let desktop = crate::platform::current_desktop_blocking().await;
  tracing::info!(
    session = ?session,
    data_control,
    background = clipboard::background_clipboard(session, data_control),
    readers = ?order,
    desktop = desktop.as_str(),
    "Clipboard watch started"
  );
  let background = clipboard::background_clipboard(session, data_control);
  let mut diag = clipboard::Diagnostics::default();
  let mut last_prefill = String::new();
  let mut failures = ShowFailures::default();
  let mut tick = tokio::time::interval(Duration::from_millis(500));
  // A slow read (bounded by READ_LIMIT) skips the ticks it overran instead of firing them in a burst.
  tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  loop {
    let state = app.state::<CaptureState>();
    tokio::select! {
      _ = tick.tick() => {},
      _ = state.clipboard_wake.notified() => {},
    }
    // Without data-control, GTK may return a cached selection while we're in the background.
    // Wait for focus before observing it; focus also wakes this loop for an immediate read.
    if !background && !state.main_is_focused.load(Ordering::Relaxed) {
      continue;
    }
    let ready = state.ready.load(Ordering::Relaxed);
    diag.note("ready", if ready { "true" } else { "false" });
    if !ready {
      continue;
    }
    let cfg = app.state::<SharedConfig>().load();
    let enabled = cfg.input.watch_clipboard || cfg.input.auto_fill_clipboard;
    diag.note("enabled", if enabled { "true" } else { "false" });
    if !enabled {
      continue;
    }
    // Clipboard failures are expected (empty, non-text, locked by another process).
    let (read, reader) = clipboard::read_text(&app, order).await;
    diag.note(
      "read",
      match (&read, reader) {
        (Ok(_), Some(clipboard::Reader::Gtk)) => "ok-gtk",
        (Ok(_), _) => "ok-plugin",
        (Err(e), _) => e.kind(),
      },
    );
    if !background && !state.main_is_focused.load(Ordering::Relaxed) {
      continue;
    }
    let (candidate, prefill) = match read {
      Ok(text) => {
        // Lock after reading: capture_write_clipboard records its own write under this lock first.
        let mut policy = state.policy.lock().unwrap();
        let prefill = supported_url(&text).map(|(url, _)| url).unwrap_or_default();
        let decision = policy.decide(&text, state.started.elapsed());
        diag.note("policy", decision.kind());
        (decision.offer().map(|candidate| (candidate, text)), prefill)
      }
      Err(_) => (None, String::new()),
    };
    if prefill != last_prefill {
      if cfg.input.auto_fill_clipboard {
        send_to_main(
          &app,
          ForMain::Prefill {
            url: prefill.clone(),
          },
        );
      }
      last_prefill = prefill;
    }
    if let Some(((url, _), text)) = candidate {
      if !cfg.input.watch_clipboard {
        // The card is off: consume the link so turning the card on later doesn't offer an old copy.
        state
          .policy
          .lock()
          .unwrap()
          .acknowledge(&text, state.started.elapsed());
      } else if !failures.may_retry(&text) {
        diag.note("card", "show-failed-backoff");
      } else {
        // Do not replace a quick-bar interaction with a background offer.
        if app
          .get_webview_window(CaptureWindow::Bar.label())
          .is_some_and(|w| w.is_visible().unwrap_or(false))
        {
          diag.note("card", "skipped-bar-open");
        } else {
          let shown = offer(&app, CaptureWindow::Card, &url, false);
          failures.record(&text, shown.is_ok());
          if shown.is_ok() {
            state
              .policy
              .lock()
              .unwrap()
              .acknowledge(&text, state.started.elapsed());
            crate::usage::count(&app, crate::usage::keys::CLIP_SHOWN);
            // This is an event, not a state: Diagnostics suppresses repeated "shown" values.
            tracing::info!("Copied-link card shown");
          }
          diag.note(
            "card",
            if shown.is_ok() {
              "shown"
            } else {
              "show-failed"
            },
          );
        }
      }
    }
  }
}

/// A copied link whose card failed to show is retried a few times, spaced out, instead of every tick.
#[derive(Default)]
struct ShowFailures {
  text: String,
  count: u8,
  at: Option<Instant>,
}

impl ShowFailures {
  const MAX: u8 = 3;
  const GAP: Duration = Duration::from_secs(5);

  fn may_retry(&self, text: &str) -> bool {
    self.may_retry_at(text, Instant::now())
  }

  fn may_retry_at(&self, text: &str, now: Instant) -> bool {
    if self.text != text || self.count == 0 {
      return true;
    }
    self.count < Self::MAX && self.at.is_none_or(|at| now.duration_since(at) >= Self::GAP)
  }

  fn record(&mut self, text: &str, shown: bool) {
    self.record_at(text, shown, Instant::now());
  }

  fn record_at(&mut self, text: &str, shown: bool, now: Instant) {
    if shown {
      *self = Self::default();
    } else {
      if self.text != text {
        self.text = text.to_owned();
        self.count = 0;
      }
      self.count += 1;
      self.at = Some(now);
    }
  }
}

/// Raw clipboard text through the session's readers. Async on purpose: a sync command runs on the main
/// thread, where arboard can deadlock on Linux and GTK could not answer its own selection.
pub(super) async fn read_current(app: &AppHandle) -> Result<String, clipboard::ReadError> {
  let order = clipboard::read_order(crate::platform::current_session(), data_control(app).await);
  clipboard::read_text(app, order).await.0
}

pub fn open_quick_bar(app: &AppHandle, browser_first: bool) {
  let request_id = app
    .state::<CaptureState>()
    .quick_request
    .fetch_add(1, Ordering::Relaxed)
    + 1;
  let app = app.clone();
  tauri::async_runtime::spawn(async move {
    // Resolve the foreground identity BEFORE focusing Nezzel. The second command may
    // request Automation permission, so show the explanation before executing it.
    let request = if browser_first {
      browser::request().await
    } else {
      None
    };
    let clipboard = read_current(&app).await.unwrap_or_default();
    if app
      .state::<CaptureState>()
      .quick_request
      .load(Ordering::Relaxed)
      != request_id
    {
      return;
    }
    if let Some(card) = app.get_webview_window(CaptureWindow::Card.label()) {
      let _ = card.hide();
    }
    let pending_id = if request.is_some() {
      offer(&app, CaptureWindow::Bar, &clipboard, true)
        .ok()
        .map(|offer| offer.id)
    } else {
      None
    };
    let browser_url = if let Some(command) = request {
      browser::resolve(command).await
    } else {
      None
    };
    let state = app.state::<CaptureState>();
    if state.quick_request.load(Ordering::Relaxed) != request_id {
      return;
    }
    if let Some(id) = pending_id {
      // Dismissal or typing while the OS prompt is open supersedes the browser result.
      if !state.offers.lock().unwrap().holds(CaptureWindow::Bar, id) {
        return;
      }
    }
    let mut text = browser_url
      .filter(|s| supported_url(s).is_some())
      .unwrap_or_else(|| clipboard.clone());
    // On a feed page, prefer a clipboard post URL from the same site.
    if let (Some((url, site)), Some((_, clip_site))) =
      (supported_url(&text), supported_url(&clipboard))
    {
      if policy::is_feed_page(&url)
        && same_site(&site, &clip_site)
        && !supported_url(&clipboard).is_some_and(|(u, _)| policy::is_feed_page(&u))
      {
        text = clipboard;
      }
    }
    let _ = offer(&app, CaptureWindow::Bar, &text, false);
  });
}

/// `www.` and mobile `m.` hosts are the same site as the bare domain.
fn same_site(a: &str, b: &str) -> bool {
  let key = |s: &str| {
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.strip_prefix("m.").unwrap_or(s).to_string()
  };
  key(a) == key(b)
}

#[cfg(test)]
mod tests {
  use super::ShowFailures;
  use std::time::{Duration, Instant};

  #[test]
  fn failed_card_retries_spaced_out_and_gives_up() {
    let t0 = Instant::now();
    let mut f = ShowFailures::default();
    assert!(f.may_retry_at("a", t0));
    f.record_at("a", false, t0);
    assert!(!f.may_retry_at("a", t0 + Duration::from_millis(500)));
    assert!(f.may_retry_at("a", t0 + ShowFailures::GAP));
    f.record_at("a", false, t0 + ShowFailures::GAP);
    f.record_at("a", false, t0 + ShowFailures::GAP * 2);
    assert!(!f.may_retry_at("a", t0 + ShowFailures::GAP * 10));
    // A different link is offered right away; a shown card clears the record.
    assert!(f.may_retry_at("b", t0));
    f.record_at("a", true, t0);
    assert!(f.may_retry_at("a", t0));
  }
}
