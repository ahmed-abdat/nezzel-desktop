//! Where links come from: the clipboard watcher (the copied-link card) and the Quick download bar.

use super::outbox::{send_to_main, ForMain};
use super::policy::{self, supported_url};
use super::state::CaptureState;
use super::window::{offer, offer_with, CaptureWindow};
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

/// Only use the optional helper when the built-in data-control reader is unavailable.
pub(super) async fn wl_paste(app: &AppHandle) -> &clipboard::WlPaste {
  app
    .state::<CaptureState>()
    .inner()
    .wl_paste
    .get_or_init(|| async {
      if crate::platform::current_session() == crate::platform::Session::Wayland
        && !data_control(app).await
      {
        clipboard::WlPaste::probe().await
      } else {
        clipboard::WlPaste::default()
      }
    })
    .await
}

pub(super) async fn watch_clipboard(app: AppHandle) {
  let session = crate::platform::current_session();
  let data_control = data_control(&app).await;
  let tool = wl_paste(&app).await;
  let fallback = tool.background;
  let order = if fallback {
    &[
      clipboard::Reader::WlPaste,
      clipboard::Reader::Gtk,
      clipboard::Reader::Plugin,
    ][..]
  } else {
    clipboard::read_order(session, data_control)
  };
  // Observe before UI readiness or feature switches: stale launch text never becomes a new copy.
  let input = app.state::<SharedConfig>().load().input.clone();
  let mut baselined = if input.watch_clipboard || input.auto_fill_clipboard {
    let text = startup_baseline(|| async { clipboard::read_text(&app, order).await.0 }).await;
    app
      .state::<CaptureState>()
      .policy
      .lock()
      .unwrap()
      .baseline(&text);
    true
  } else {
    false
  };
  let desktop = crate::platform::current_desktop_blocking().await;
  tracing::info!(
    session = ?session,
    data_control,
    background = clipboard::background_clipboard(session, data_control) || tool.background,
    readers = ?order,
    desktop = desktop.as_str(),
    "Clipboard watch started"
  );
  let background = clipboard::background_clipboard(session, data_control) || tool.background;
  let mut diag = clipboard::Diagnostics::default();
  let mut last_prefill = String::new();
  let mut failures = ShowFailures::default();
  let mut tick = tokio::time::interval(Duration::from_millis(if fallback { 1000 } else { 500 }));
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
      // Turning capture on later must baseline what was copied while it was off.
      baselined = false;
      continue;
    }
    // Clipboard failures are expected (empty, non-text, locked by another process).
    // GTK/Xwayland can return stale text without focus. In the background only the helper is safe.
    let focused = state.main_is_focused.load(Ordering::Relaxed);
    let readers = if fallback && !focused {
      &[clipboard::Reader::WlPaste][..]
    } else {
      order
    };
    let (read, reader) = clipboard::read_text(&app, readers).await;
    diag.note(
      "read",
      match (&read, reader) {
        (Ok(_), Some(clipboard::Reader::Gtk)) => "ok-gtk",
        (Ok(_), Some(clipboard::Reader::WlPaste)) => "ok-wl-paste",
        (Ok(_), _) => "ok-plugin",
        (Err(e), _) => e.kind(),
      },
    );
    if session == crate::platform::Session::Wayland
      && !data_control
      && reader != Some(clipboard::Reader::WlPaste)
      && !state.main_is_focused.load(Ordering::Relaxed)
    {
      continue;
    }
    let (candidate, prefill) = match read {
      Ok(text) => {
        // Lock after reading: capture_write_clipboard records its own write under this lock first.
        let mut policy = state.policy.lock().unwrap();
        if !baselined {
          policy.baseline(&text);
          baselined = true;
          continue;
        }
        let prefill = supported_url(&text).map(|(url, _)| url).unwrap_or_default();
        let decision = policy.decide(&text, state.started.elapsed());
        diag.note("policy", decision.kind());
        (decision.offer().map(|candidate| (candidate, text)), prefill)
      }
      Err(clipboard::ReadError::Empty) => {
        state.policy.lock().unwrap().baseline("");
        baselined = true;
        (None, String::new())
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

/// Briefly retry a locked launch clipboard. Exhaustion baselines empty text now, never a later copy.
async fn startup_baseline<F, Fut>(mut read: F) -> String
where
  F: FnMut() -> Fut,
  Fut: std::future::Future<Output = Result<String, clipboard::ReadError>>,
{
  let gap = Duration::from_millis(200);
  for attempt in 0..3 {
    match tokio::time::timeout(gap, read()).await {
      Ok(Ok(text)) => return text,
      Ok(Err(clipboard::ReadError::Empty)) => return String::new(),
      _ => {}
    }
    if attempt < 2 {
      tokio::time::sleep(gap).await;
    }
  }
  String::new()
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
  let order = if wl_paste(app).await.background {
    &[
      clipboard::Reader::WlPaste,
      clipboard::Reader::Gtk,
      clipboard::Reader::Plugin,
    ][..]
  } else {
    clipboard::read_order(crate::platform::current_session(), data_control(app).await)
  };
  clipboard::read_text(app, order).await.0
}

pub fn open_quick_bar(app: &AppHandle, browser_first: bool) {
  // The single-instance plugin can call this before setup has registered capture state.
  let Some(state) = app.try_state::<CaptureState>() else {
    return;
  };
  let request_id = state.quick_request.fetch_add(1, Ordering::Relaxed) + 1;
  let app = app.clone();
  tauri::async_runtime::spawn(async move {
    // Resolve the foreground identity BEFORE focusing Nezzel. macOS: one osascript reads the frontmost
    // app and its tab, so the bar shows "Reading the active browser tab" first (shown, not focused, so
    // the browser stays frontmost). The tab read may ask for Automation; the bar explains it meanwhile.
    let own = app.config().identifier.clone();
    let (mut frontmost, request) = if browser_first {
      let request = browser::request(&own).await;
      (request.frontmost, request.command)
    } else {
      ("skipped", None)
    };
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
    // Empty text: the reading state, never an older clipboard link that may be replaced a moment later.
    let pending_id = if request.is_some() {
      offer(&app, CaptureWindow::Bar, "", true)
        .ok()
        .map(|offer| offer.id)
    } else {
      None
    };
    let started = std::time::Instant::now();
    let (mut browser_read, clipboard) = tokio::join!(
      read_browser(request, &own, &mut frontmost),
      read_current(&app)
    );
    let clipboard = clipboard.unwrap_or_default();
    let mut reset = "no";
    if browser::after_read(
      browser_read.as_deref().map_err(|e| *e),
      browser::reset_used(),
      cfg!(target_os = "macos"),
    ) == browser::AfterRead::ResetAndRetry
      && browser::claim_reset()
    {
      reset = if browser::reset_automation(&app.config().identifier).await {
        "ok"
      } else {
        "failed"
      };
      // macOS asks again on this read; the bar keeps its "reading the browser" state meanwhile.
      let retry = browser::request(&own).await.command;
      browser_read = read_browser(retry, &own, &mut frontmost).await;
    }
    let denied = browser::after_read(
      browser_read.as_deref().map_err(|e| *e),
      true,
      cfg!(target_os = "macos"),
    ) == browser::AfterRead::Denied;
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
    let (text, source) = if denied {
      // An old clipboard link from another site is not what the user pointed at.
      (String::new(), "denied")
    } else {
      choose(browser_read.as_deref().ok(), &clipboard)
    };
    let browser = match &browser_read {
      Ok(url) if supported_url(url).is_some() => "ok",
      Ok(_) => "unsupported",
      Err(reason) => reason,
    };
    // Log words only: never the browser URL or the clipboard.
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
      frontmost,
      browser,
      reset,
      source,
      ms,
      "Quick download bar opened"
    );
    let _ = offer_with(&app, CaptureWindow::Bar, &text, false, denied);
  });
}

/// The quick bar's text and where it came from ("browser", "clipboard", or "none" when neither holds a
/// supported link): the browser tab first; on a feed page, a clipboard post URL from the same site.
fn choose(browser_url: Option<&str>, clipboard: &str) -> (String, &'static str) {
  let (mut text, mut source) = match browser_url.filter(|s| supported_url(s).is_some()) {
    Some(url) => (url.to_owned(), "browser"),
    None => (clipboard.to_owned(), "clipboard"),
  };
  if let (Some((url, site)), Some((clip_url, clip_site))) =
    (supported_url(&text), supported_url(clipboard))
  {
    if policy::is_feed_page(&url)
      && same_site(&site, &clip_site)
      && !policy::is_feed_page(&clip_url)
    {
      (text, source) = (clipboard.to_owned(), "clipboard");
    }
  }
  if supported_url(&text).is_none() {
    source = "none";
  }
  (text, source)
}

/// Runs a browser read. On macOS the output also names the frontmost app, which replaces `frontmost`.
async fn read_browser(
  command: Option<tokio::process::Command>,
  own: &str,
  frontmost: &mut &'static str,
) -> Result<String, &'static str> {
  let Some(command) = command else {
    return Err("skipped");
  };
  let read = browser::resolve(command).await;
  if !cfg!(target_os = "macos") {
    return read;
  }
  match read {
    Ok(output) => {
      let (app, read) = browser::parse_mac(&output, own);
      *frontmost = app;
      read
    }
    Err(reason) => {
      *frontmost = "unknown";
      Err(reason)
    }
  }
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
  use super::{choose, startup_baseline, ShowFailures};
  use crate::capture::{
    clipboard::ReadError,
    policy::{CapturePolicy, Decision},
  };
  use std::time::{Duration, Instant};

  #[tokio::test(start_paused = true)]
  async fn launch_baseline_retries_transient_failures_without_swallowing_later_copies() {
    let old = "https://youtu.be/old";
    let new = "https://youtu.be/new";
    for reads in [
      vec![Err(ReadError::Occupied), Ok(old.to_owned())],
      vec![
        Err(ReadError::Occupied),
        Err(ReadError::Other),
        Err(ReadError::Timeout),
      ],
      vec![Err(ReadError::Empty)],
    ] {
      let mut reads = std::collections::VecDeque::from(reads);
      let baseline = startup_baseline(|| std::future::ready(reads.pop_front().unwrap())).await;
      assert!(reads.is_empty());
      let mut policy = CapturePolicy::default();
      policy.baseline(&baseline);
      if !baseline.is_empty() {
        assert_eq!(policy.decide(old, Duration::ZERO), Decision::Unchanged);
      }
      assert!(policy.decide(new, Duration::ZERO).offer().is_some());
    }
  }

  #[tokio::test(start_paused = true)]
  async fn hung_launch_baseline_has_a_bounded_retry_window() {
    let mut attempts = 0;
    let start = tokio::time::Instant::now();
    let baseline = startup_baseline(|| {
      attempts += 1;
      async { std::future::pending::<Result<String, ReadError>>().await }
    })
    .await;
    assert!(baseline.is_empty());
    assert_eq!(attempts, 3);
    assert_eq!(start.elapsed(), Duration::from_secs(1));
  }

  #[test]
  fn quick_bar_prefers_the_browser_tab_and_says_where_the_link_came_from() {
    let photo = "https://www.tiktok.com/@user/photo/7312345678901234567";
    let youtube = "https://youtu.be/abc";
    let tiktok_post = "https://www.tiktok.com/@user/video/1";
    for (browser, clipboard, text, source) in [
      // A TikTok photo tab wins over an older copied YouTube link.
      (Some(photo), youtube, photo, "browser"),
      // No browser read (not a browser, Automation denied): the clipboard link.
      (None, youtube, youtube, "clipboard"),
      (Some("https://example.com/x"), youtube, youtube, "clipboard"),
      // A feed tab yields to a copied post from the same site.
      (
        Some("https://www.tiktok.com/foryou"),
        tiktok_post,
        tiktok_post,
        "clipboard",
      ),
      (
        Some("https://www.tiktok.com/foryou"),
        youtube,
        "https://www.tiktok.com/foryou",
        "browser",
      ),
      // Nothing usable: an empty bar, and the log says so.
      (None, "hello", "hello", "none"),
    ] {
      assert_eq!(
        choose(browser, clipboard),
        (text.to_owned(), source),
        "{browser:?} {clipboard}"
      );
    }
  }

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
