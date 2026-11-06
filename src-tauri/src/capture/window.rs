//! The two capture windows: identity and size, building them, placing the card, and showing an offer.

use super::policy::{self, supported_url};
use super::read::preview;
use super::state::{CaptureState, Offer};
use crate::SharedConfig;
use std::{sync::atomic::Ordering, time::Duration};
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
  let value = Offer {
    id: state.sequence.fetch_add(1, Ordering::Relaxed) + 1,
    url,
    site,
    resolving,
    feed,
    adopt_on_focus: window.adopts_on_focus() && !state.main_is_focused.load(Ordering::Relaxed),
    ..Offer::default()
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
    let reads = !value.url.is_empty() && !value.feed;
    tauri::async_runtime::spawn(async move {
      let state = app.state::<CaptureState>();
      let shown = std::time::Instant::now();
      let mut read_after = (!reads).then_some(Duration::ZERO);
      loop {
        let changed = state.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let read = match state.offers.lock().unwrap().holding(window, id) {
          Some(slot) => slot.metadata.is_some(),
          None => return,
        };
        if read_after.is_none() && read {
          read_after = Some(shown.elapsed());
        }
        let due = card_hide_after(read_after);
        let elapsed = shown.elapsed();
        if elapsed >= due {
          break;
        }
        tokio::select! {
          _ = changed => {}
          _ = tokio::time::sleep(due - elapsed) => {}
        }
      }
      tracing::info!(
        read_ms = read_after.map(|d| d.as_millis() as u64),
        "Capture card timed out"
      );
      if let Some(native) = app.get_webview_window(label) {
        let _ = native.hide();
      }
    });
  }
  Ok(value)
}

/// How long the copied-link card stays up untouched. It never leaves while it still reads the link
/// (the body used to vanish unread on slow reads, #36): at least 8 s, and 5 s after the read lands,
/// up to `read::CARD_LIMIT`.
fn card_hide_after(read_after: Option<Duration>) -> Duration {
  const SHOWN: Duration = Duration::from_secs(8);
  const AFTER_READ: Duration = Duration::from_secs(5);
  const CAP: Duration = super::read::CARD_LIMIT;
  match read_after {
    Some(read) => SHOWN.max(read + AFTER_READ).min(CAP),
    None => CAP,
  }
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
fn card_position(area: WorkArea, logical_height: f64, rtl: bool, top: bool) -> (i32, i32) {
  let width = (CARD_WIDTH * area.scale) as i32;
  let height = (logical_height * area.scale) as i32;
  let gap = (16. * area.scale) as i32;
  let x = area.x
    + if rtl {
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
  let lang = app
    .state::<SharedConfig>()
    .load()
    .appearance
    .language
    .clone();
  let rtl_lang = |code: &str| {
    crate::i18n::RTL_LANGS
      .iter()
      .any(|rtl| code.starts_with(rtl))
  };
  let rtl = crate::i18n::RTL_LANGS.contains(&lang.as_str())
    || (lang == "system" && sys_locale::get_locale().is_some_and(|s| rtl_lang(&s)));
  let (x, y) = card_position(area, logical_height, rtl, cfg!(target_os = "macos"));
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
  fn card_stays_while_it_reads_the_link() {
    let s = Duration::from_secs;
    for (read_after, due) in [
      (Some(s(0)), s(8)), // no read (feed, unsupported) or instant: the usual 8 s
      (Some(s(2)), s(8)),
      (Some(s(6)), s(11)), // slow read: 5 s to act on the body
      (Some(s(30)), s(25)),
      (None, s(25)), // still reading: never before the read limit
    ] {
      assert_eq!(card_hide_after(read_after), due, "{read_after:?}");
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
        card_position(area, height, rtl, top),
        expected,
        "{area:?} h{height} rtl{rtl} top{top}"
      );
    }
  }
}
