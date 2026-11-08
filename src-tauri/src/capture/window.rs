//! The two capture windows: identity and size, building them, placing the card, and showing an offer.

use super::policy::{self, supported_url};
use super::read::preview;
use super::state::{CaptureState, Offer};
use crate::{state::config_models::CardPosition, SharedConfig};
use std::{
  sync::atomic::Ordering,
  time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// The two capture windows' native labels (capabilities/capture.json lists the same; the webview gets them
/// from `crate::contract`).
pub const CARD_LABEL: &str = "capture-card";
pub const BAR_LABEL: &str = "capture-bar";

/// The window's offer changed: it reads it again with `capture_snapshot`. The event carries nothing, so one Linux
/// holds for a hidden window (tauri#3654) delays the panel only until its own snapshot poll.
pub(super) fn wake_window(app: &AppHandle, window: CaptureWindow) {
  let _ = app.emit_to(window.label(), "capture_changed", ());
}

/// The two capture windows. Everything that differs between them (label, size, focus) comes from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureWindow {
  /// The copied-link card: raised by the clipboard watcher, never takes focus.
  Card,
  /// The quick bar: opened by the shortcut, takes focus for typing.
  Bar,
}

impl CaptureWindow {
  pub const ALL: [Self; 2] = [Self::Card, Self::Bar];

  pub const fn label(self) -> &'static str {
    match self {
      Self::Card => CARD_LABEL,
      Self::Bar => BAR_LABEL,
    }
  }

  pub fn from_label(label: &str) -> Option<Self> {
    Self::ALL.into_iter().find(|window| window.label() == label)
  }

  pub(super) const fn other(self) -> Self {
    match self {
      Self::Card => Self::Bar,
      Self::Bar => Self::Card,
    }
  }

  /// The native size when built (the card's closed size).
  pub(super) const fn size(self) -> (f64, f64) {
    match self {
      Self::Card => (CARD_WIDTH, CARD_HEIGHT),
      Self::Bar => (BAR_WIDTH, BAR_HEIGHT),
    }
  }

  /// Only the bar takes keyboard focus (and so gives it back when it closes); the card must never
  /// pull focus from the app the user copied in.
  pub(super) const fn focusable(self) -> bool {
    matches!(self, Self::Bar)
  }

  /// A card raised while the main window is in the background is adopted as a row when the main
  /// window gets focus (X11 focuses it as the card is clicked).
  pub(super) const fn adopts_on_focus(self) -> bool {
    matches!(self, Self::Card)
  }
}

/// The copied-link card's native size when closed (the format list grows it, see `capture_fit`). The webview
/// lays the card out at the same size (`crate::contract`).
pub(crate) const CARD_WIDTH: f64 = 360.;
pub(crate) const CARD_HEIGHT: f64 = 160.;
/// Tallest the card may grow with its format list open.
pub(crate) const CARD_MAX_HEIGHT: f64 = 480.;
/// The quick bar's native size when built; the webview then fits the height to its content at this width.
pub(crate) const BAR_WIDTH: f64 = 620.;
pub(crate) const BAR_HEIGHT: f64 = 360.;

/// Builds both capture windows, hidden.
pub(super) fn create(app: &AppHandle) -> tauri::Result<()> {
  for window in CaptureWindow::ALL {
    let (label, (width, height)) = (window.label(), window.size());
    tauri::WebviewWindowBuilder::new(
      app,
      label,
      tauri::WebviewUrl::App(format!("index.html?capture={label}").into()),
    )
    .title("Nezzel")
    .inner_size(width, height)
    .visible(false)
    .decorations(false)
    // Transparent so the panel's rounded corners show (macOS needs macOSPrivateApi in tauri.conf).
    .transparent(true)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .accept_first_mouse(true)
    .focusable(window.focusable())
    .build()?;
  }
  Ok(())
}

pub(super) fn offer(
  app: &AppHandle,
  window: CaptureWindow,
  text: &str,
  resolving: bool,
) -> Result<Offer, String> {
  let parsed = supported_url(text);
  let (url, site) = parsed.unwrap_or_default();
  let state = app.state::<CaptureState>();
  let label = window.label();
  // Serialize creation with adoption; never make native window calls while holding this lock.
  let mut offers = state.offers.lock().unwrap();
  let feed = !url.is_empty() && policy::is_feed_page(&url);
  let config = app.state::<SharedConfig>().load();
  let value = Offer {
    id: state.sequence.fetch_add(1, Ordering::Relaxed) + 1,
    url,
    site,
    resolving,
    feed,
    adopt_on_focus: window.adopts_on_focus() && !state.main_is_focused.load(Ordering::Relaxed),
    ..Offer::with_network(&config.network)
  };
  offers.replace(window, value.clone());
  drop(offers);
  state.changed.notify_waiters();
  let native = app.get_webview_window(label).ok_or("capture unavailable")?;
  if window == CaptureWindow::Card {
    // A new link starts closed: the card's own size, then its corner.
    let _ = native.set_size(tauri::LogicalSize::new(CARD_WIDTH, CARD_HEIGHT));
    place_card(app, &native, CARD_HEIGHT);
  } else {
    let _ = native.center();
  }
  if window.focusable() && !native.is_visible().unwrap_or(false) {
    let focused = app
      .get_webview_window("main")
      .and_then(|main| main.is_focused().ok())
      .unwrap_or(false);
    state.main_was_focused.store(focused, Ordering::Relaxed);
  }
  native.show().map_err(|_| "capture unavailable")?;
  tracing::debug!(window = label, host = %value.site, "Capture window shown");
  if window.focusable() && !resolving {
    let _ = native.set_focus();
  }
  wake_window(app, window);
  if !resolving && !value.url.is_empty() && !value.feed {
    let app = app.clone();
    let id = value.id;
    let value = value.clone();
    let job = tauri::async_runtime::spawn(async move {
      preview(app, window, value).await;
    });
    state.offers.lock().unwrap().set_job(window, id, job);
  }
  if window == CaptureWindow::Card {
    let app = app.clone();
    let id = value.id;
    tauri::async_runtime::spawn(async move {
      let state = app.state::<CaptureState>();
      loop {
        let changed = state.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let due = {
          let mut offers = state.offers.lock().unwrap();
          let Some(slot) = offers
            .slot_mut(window)
            .as_mut()
            .filter(|slot| slot.offer.id == id)
          else {
            return;
          };
          if slot.card_timer.read_after.is_none() && slot.metadata.is_some() {
            slot.card_timer.read_after = Some(slot.card_timer.shown.elapsed());
          }
          slot
            .card_timer
            .set_keep_open(state.card_keep_open.load(Ordering::Relaxed));
          slot.card_timer.due()
        };
        match due {
          Some(due) if due <= Instant::now() => {
            // Native hide and new offers are serialized on the UI thread. Recheck there so a
            // queued timeout cannot hide a replacement offer or a newly engaged card.
            let ui_app = app.clone();
            let (send, receive) = tokio::sync::oneshot::channel();
            if app
              .run_on_main_thread(move || {
                let state = ui_app.state::<CaptureState>();
                let hide = state
                  .offers
                  .lock()
                  .unwrap()
                  .holding(window, id)
                  .is_some_and(|slot| {
                    slot
                      .card_timer
                      .due()
                      .is_some_and(|due| due <= Instant::now())
                  });
                if hide {
                  if let Some(native) = ui_app.get_webview_window(label) {
                    let _ = native.hide();
                  }
                }
                let _ = send.send(hide);
              })
              .is_err()
            {
              return;
            }
            if receive.await.unwrap_or(true) {
              return;
            }
          }
          Some(due) => tokio::select! {
            _ = changed => {}
            _ = tokio::time::sleep_until(due.into()) => {}
          },
          None => changed.await,
        }
      }
    });
  }

  Ok(value)
}

/// Engagement leases are renewed every 3 s and expire if the webview stops reporting.
const CARD_HOLD_LEASE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CardHold {
  start: Duration,
  end: Option<Duration>,
  /// None while held by the explicit keep-open setting, which needs no heartbeat.
  last_ping: Option<Duration>,
}

/// Hold spans are the union of pointer/focus/menu engagement and the explicit keep-open setting.
#[derive(Debug)]
pub(super) struct CardTimer {
  offer_id: u64,
  shown: Instant,
  read_after: Option<Duration>,
  spans: Vec<CardHold>,
  last_ping: Option<Duration>,
  keep_open: bool,
}

impl CardTimer {
  pub(super) fn new(offer: &Offer) -> Self {
    Self {
      offer_id: offer.id,
      shown: Instant::now(),
      read_after: (offer.url.is_empty() || offer.feed).then_some(Duration::ZERO),
      spans: Vec::new(),
      last_ping: None,
      keep_open: false,
    }
  }

  pub(super) fn engage(&mut self, id: u64, engaged: bool) {
    self.engage_at(id, engaged, self.shown.elapsed());
  }

  fn engage_at(&mut self, id: u64, engaged: bool, now: Duration) {
    if self.offer_id != id {
      return;
    }
    self.expire(now);
    self.last_ping = engaged.then_some(now);
    self.record_hold(now);
  }

  pub(super) fn set_keep_open(&mut self, keep_open: bool) {
    self.set_keep_open_at(keep_open, self.shown.elapsed());
  }

  fn set_keep_open_at(&mut self, keep_open: bool, now: Duration) {
    self.expire(now);
    self.keep_open = keep_open;
    self.record_hold(now);
  }

  fn expire(&mut self, now: Duration) {
    if let Some(expiry) = self
      .last_ping
      .map(|ping| ping + CARD_HOLD_LEASE)
      .filter(|expiry| *expiry <= now)
    {
      self.last_ping = None;
      if !self.keep_open {
        if let Some(span) = self.spans.last_mut().filter(|span| span.end.is_none()) {
          span.end = Some(expiry);
        }
      }
    }
  }

  fn record_hold(&mut self, now: Duration) {
    let active = self.spans.last_mut().filter(|span| span.end.is_none());
    if self.keep_open || self.last_ping.is_some() {
      let ping = if self.keep_open { None } else { self.last_ping };
      if let Some(span) = active {
        span.last_ping = ping;
      } else {
        self.spans.push(CardHold {
          start: now,
          end: None,
          last_ping: ping,
        });
      }
    } else if let Some(span) = active {
      span.end = Some(now);
    }
  }

  fn due(&self) -> Option<Instant> {
    card_hide_due(self.read_after, &self.spans, self.keep_open).map(|after| self.shown + after)
  }
}

/// Untouched: at least 8 s, 5 s after the read, capped at the read limit. Unfinished reads and
/// keep-open have no deadline. Open engagement spans provisionally end at the lease expiry;
/// heartbeats move that expiry forward. Every release pauses the budget and grants at least 4 s.
fn card_hide_due(
  read_after: Option<Duration>,
  spans: &[CardHold],
  keep_open: bool,
) -> Option<Duration> {
  if keep_open {
    return None;
  }
  let read = read_after?;
  let mut due = Duration::from_secs(8)
    .max(read + Duration::from_secs(5))
    .min(super::read::CARD_LIMIT);
  for span in spans {
    let end = span
      .end
      .or_else(|| span.last_ping.map(|ping| ping + CARD_HOLD_LEASE))?;
    due = end + due.saturating_sub(span.start).max(Duration::from_secs(4));
  }
  Some(due)
}

/// A monitor's work area in physical pixels, plus its scale factor.
#[derive(Clone, Copy, Debug)]
struct WorkArea {
  x: i32,
  y: i32,
  width: u32,
  height: u32,
  scale: f64,
}

/// Height the card asks for, clamped to its closed size and its open cap; None for nonsense.
pub(super) fn fit_height(requested: f64) -> Option<f64> {
  requested
    .is_finite()
    .then(|| requested.clamp(CARD_HEIGHT, CARD_MAX_HEIGHT))
}

/// Where the card's top-start goes (physical pixels): the top-end corner on macOS (under the menu bar),
/// the bottom-end corner elsewhere (over the taskbar, so a taller card grows upward); the start corner in Arabic.
fn card_position(
  area: WorkArea,
  logical_height: f64,
  rtl: bool,
  top: bool,
  position: CardPosition,
) -> (i32, i32) {
  let width = (CARD_WIDTH * area.scale) as i32;
  let height = (logical_height * area.scale) as i32;
  let gap = (16. * area.scale) as i32;
  let (top, start) = match position {
    CardPosition::Automatic => (top, false),
    CardPosition::TopStart => (true, true),
    CardPosition::TopEnd => (true, false),
    CardPosition::BottomStart => (false, true),
    CardPosition::BottomEnd => (false, false),
  };
  let x = area.x
    + if start != rtl {
      gap
    } else {
      area.width as i32 - width - gap
    };
  let y = area.y
    + if top {
      gap
    } else {
      area.height as i32 - height - gap
    };
  (x, y)
}

pub(super) fn place_card(app: &AppHandle, window: &WebviewWindow, logical_height: f64) {
  let Some(monitor) = window
    .current_monitor()
    .ok()
    .flatten()
    .or_else(|| window.primary_monitor().ok().flatten())
  else {
    return;
  };
  let work = monitor.work_area();
  let area = WorkArea {
    x: work.position.x,
    y: work.position.y,
    width: work.size.width,
    height: work.size.height,
    scale: monitor.scale_factor(),
  };
  let config = app.state::<SharedConfig>().load();
  let lang = &config.appearance.language;
  let rtl_lang = |code: &str| {
    crate::i18n::RTL_LANGS
      .iter()
      .any(|rtl| code.starts_with(rtl))
  };
  let rtl = crate::i18n::RTL_LANGS.contains(&lang.as_str())
    || (lang == "system" && sys_locale::get_locale().is_some_and(|s| rtl_lang(&s)));
  let (x, y) = card_position(
    area,
    logical_height,
    rtl,
    cfg!(target_os = "macos"),
    config.input.card_position,
  );
  let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Hides a capture window after the user acted in it. The focusable bar, opened over another app, gives
/// focus back to that app: macOS makes the main window key when the focused bar hides, pulling Nezzel
/// over the browser. `keep_app_in_front` ("Options", which shows the main window) skips that.
pub(super) fn close_after_action(
  app: &AppHandle,
  native: &WebviewWindow,
  window: CaptureWindow,
  keep_app_in_front: bool,
) {
  let state = app.state::<CaptureState>();
  // Read before hiding: the main window's focus event from the hide would flip the flag.
  let return_to_previous_app =
    window.focusable() && !keep_app_in_front && !state.main_was_focused.load(Ordering::Relaxed);
  let _ = native.hide();
  #[cfg(target_os = "macos")]
  if return_to_previous_app {
    let _ = app.hide();
  }
  #[cfg(not(target_os = "macos"))]
  let _ = return_to_previous_app;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn card_stays_while_it_reads_the_link_or_is_engaged() {
    let s = Duration::from_secs;
    // Read completion, engagement spans relative to showing, keep-open → relative deadline.
    for (name, read_after, spans, keep_open, due) in [
      ("instant", Some(s(0)), vec![], false, Some(s(8))),
      ("quick read", Some(s(2)), vec![], false, Some(s(8))),
      ("slow read", Some(s(6)), vec![], false, Some(s(11))),
      ("untouched cap", Some(s(23)), vec![], false, Some(s(25))),
      ("unfinished read", None, vec![], false, None),
      (
        "stale engagement lease",
        Some(s(0)),
        vec![(s(2), None)],
        false,
        Some(s(18)),
      ),
      (
        "stale lease release grace",
        Some(s(0)),
        vec![(s(7), None)],
        false,
        Some(s(21)),
      ),
      (
        "pause remaining",
        Some(s(0)),
        vec![(s(2), Some(s(32)))],
        false,
        Some(s(38)),
      ),
      (
        "release grace",
        Some(s(0)),
        vec![(s(7), Some(s(30)))],
        false,
        Some(s(34)),
      ),
      (
        "late click",
        Some(s(0)),
        vec![(s(9), Some(s(30)))],
        false,
        Some(s(34)),
      ),
      (
        "repeated holds",
        Some(s(0)),
        vec![(s(2), Some(s(10))), (s(12), Some(s(20)))],
        false,
        Some(s(24)),
      ),
      (
        "read during hold",
        Some(s(20)),
        vec![(s(7), Some(s(30)))],
        false,
        Some(s(48)),
      ),
      (
        "reading on release",
        None,
        vec![(s(7), Some(s(30)))],
        false,
        None,
      ),
      ("keep open", Some(s(0)), vec![], true, None),
    ] {
      let spans: Vec<_> = spans
        .into_iter()
        .map(|(start, end)| CardHold {
          start,
          end,
          last_ping: Some(start),
        })
        .collect();
      assert_eq!(card_hide_due(read_after, &spans, keep_open), due, "{name}");
    }
  }

  #[test]
  fn card_timer_engagement_is_idempotent_and_renews_its_lease() {
    let s = Duration::from_secs;
    let mut timer = CardTimer::new(&Offer {
      id: 1,
      ..Offer::default()
    });
    timer.engage_at(1, true, s(2));
    timer.engage_at(1, true, s(3));
    timer.engage_at(1, false, s(4));
    assert_eq!(
      timer.spans,
      vec![CardHold {
        start: s(2),
        end: Some(s(4)),
        last_ping: Some(s(3))
      }]
    );
  }

  #[test]
  fn card_timer_stale_offer_id_cannot_engage_or_release_its_replacement() {
    let s = Duration::from_secs;
    let mut timer = CardTimer::new(&Offer {
      id: 2,
      ..Offer::default()
    });
    timer.engage_at(1, true, s(2));
    assert!(timer.spans.is_empty());
    timer.engage_at(2, true, s(3));
    timer.engage_at(1, false, s(4));
    assert_eq!(timer.due(), Some(timer.shown + s(18)));
  }

  #[test]
  fn card_timer_heartbeat_moves_the_deadline_and_a_late_ping_starts_a_new_span() {
    let s = Duration::from_secs;
    let mut timer = CardTimer::new(&Offer {
      id: 1,
      ..Offer::default()
    });
    timer.engage_at(1, true, s(7));
    timer.engage_at(1, true, s(10));
    assert_eq!(timer.due(), Some(timer.shown + s(24)));
    timer.engage_at(1, true, s(25));
    assert_eq!(timer.spans[0].end, Some(s(20)));
    assert_eq!(timer.due(), Some(timer.shown + s(39)));
  }

  #[test]
  fn card_timer_keep_open_release_preserves_remaining_time_or_grants_grace() {
    let s = Duration::from_secs;
    for (start, end, expected) in [(2, 30, 36), (20, 60, 64)] {
      let mut timer = CardTimer::new(&Offer {
        id: 1,
        ..Offer::default()
      });
      timer.set_keep_open_at(true, s(start));
      assert_eq!(timer.due(), None);
      timer.set_keep_open_at(false, s(end));
      assert_eq!(timer.due(), Some(timer.shown + s(expected)));
    }
    // An expired engagement lease must not release an overlapping explicit keep-open hold.
    let mut timer = CardTimer::new(&Offer {
      id: 1,
      ..Offer::default()
    });
    timer.engage_at(1, true, s(2));
    timer.set_keep_open_at(true, s(3));
    timer.set_keep_open_at(false, s(30));
    assert_eq!(timer.due(), Some(timer.shown + s(36)));
  }

  #[test]
  fn card_position_resolves_each_logical_corner_in_both_directions_and_platforms() {
    let area = WorkArea {
      x: -1920,
      y: 25,
      width: 1920,
      height: 1080,
      scale: 1.,
    };
    for top in [false, true] {
      for rtl in [false, true] {
        for (position, at_top, at_start) in [
          (CardPosition::Automatic, top, false),
          (CardPosition::TopStart, true, true),
          (CardPosition::TopEnd, true, false),
          (CardPosition::BottomStart, false, true),
          (CardPosition::BottomEnd, false, false),
        ] {
          let expected = (
            if at_start != rtl { -1904 } else { -376 },
            if at_top { 41 } else { 929 },
          );
          assert_eq!(
            card_position(area, 160., rtl, top, position),
            expected,
            "{position:?} rtl{rtl} top{top}"
          );
        }
      }
    }
  }

  #[test]
  fn card_fit_clamps_and_keeps_the_card_on_its_corner() {
    assert_eq!(fit_height(100.), Some(CARD_HEIGHT));
    assert_eq!(fit_height(300.), Some(300.));
    assert_eq!(fit_height(9_000.), Some(CARD_MAX_HEIGHT));
    assert_eq!(fit_height(f64::NAN), None);
    assert_eq!(fit_height(f64::INFINITY), None);
    let area = |x: i32, y: i32, width: u32, height: u32, scale: f64| WorkArea {
      x,
      y,
      width,
      height,
      scale,
    };
    // (area, logical height, rtl, top) → physical top-start of the card
    let cases = [
      // 1x, bottom-end: right edge − 360 − 16, bottom − height − 16
      (area(0, 0, 1920, 1040, 1.), 160., false, false, (1544, 864)),
      // taller card grows upward, keeps its bottom gap
      (area(0, 0, 1920, 1040, 1.), 480., false, false, (1544, 544)),
      // 1.5x: everything scales (360*1.5 = 540, 16*1.5 = 24)
      (
        area(0, 0, 2880, 1560, 1.5),
        300.,
        false,
        false,
        (2316, 1086),
      ),
      // 2x Arabic: start corner
      (area(0, 0, 2560, 1600, 2.), 160., true, false, (32, 1248)),
      // macOS: top corner, height does not move it
      (area(0, 25, 1440, 875, 2.), 480., false, true, (688, 57)),
      // secondary monitor left of the primary (negative origin), and one to the right
      (
        area(-1920, 0, 1920, 1080, 1.),
        160.,
        false,
        false,
        (-376, 904),
      ),
      (
        area(1920, -200, 1280, 1024, 1.),
        160.,
        true,
        false,
        (1936, 648),
      ),
    ];
    for (area, height, rtl, top, expected) in cases {
      assert_eq!(
        card_position(area, height, rtl, top, CardPosition::Automatic),
        expected,
        "{area:?} h{height} rtl{rtl} top{top}"
      );
    }
  }
}
