//! Background mode: closing the main window keeps Nezzel in the tray / menu bar. On macOS the Dock
//! icon goes away while the window is hidden (Accessory) and comes back when it shows (Regular).
//! The first close also shrinks the window into the tray and shows a one-time note anchored there.

use crate::tray::TrayState;
use crate::SharedPreferences;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{
  AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
  WebviewWindowBuilder, WindowEvent,
};

const NOTE_LABEL: &str = "close-note";
const NOTE_WIDTH: f64 = 340.;
/// Matches the prototype's first-close shrink.
const SHRINK_MS: u64 = 380;
const SHRINK_FRAMES: u64 = 22;

#[derive(Default)]
pub struct BackgroundState {
  /// Set from the main window's `prefers-reduced-motion`.
  pub reduce_motion: AtomicBool,
  /// True while the first-close shrink runs: window tracking must not persist those sizes.
  pub animating: AtomicBool,
  /// Where the note goes once its page has measured itself.
  note: std::sync::Mutex<Option<NotePlacement>>,
  /// Show/hide transitions in order: a later show cancels an earlier, still running close.
  pub transitions: Transitions,
}

/// Generation counter for window transitions. A close takes a ticket; any show after it bumps the
/// generation, so the close sees it is stale and must not hide the window or open the note.
#[derive(Default)]
pub struct Transitions(AtomicU64);

impl Transitions {
  pub fn begin(&self) -> u64 {
    self.0.fetch_add(1, Ordering::SeqCst) + 1
  }
  pub fn is_current(&self, ticket: u64) -> bool {
    self.0.load(Ordering::SeqCst) == ticket
  }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge {
  /// Note hangs below the anchor (menu bar, GNOME top bar).
  Top,
  /// Note sits above the anchor (Windows taskbar).
  Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotePlacement {
  pub left: f64,
  /// Arrow position from the note's left edge.
  pub arrow_x: f64,
  /// y of the anchor edge: the note's top (Edge::Top) or bottom (Edge::Bottom).
  pub y: f64,
  pub edge: Edge,
}

/// Logical-pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
  pub x: f64,
  pub y: f64,
  pub w: f64,
  pub h: f64,
}

/// Places a note of `width` so its arrow points at `anchor_x`, opening toward the screen centre
/// and kept 10px inside `area`.
pub fn place_note(anchor_x: f64, y: f64, edge: Edge, area: Area, width: f64) -> NotePlacement {
  let centre = area.x + area.w / 2.;
  let wanted = if anchor_x > centre {
    anchor_x - width + 34.
  } else {
    anchor_x - 34.
  };
  let left = wanted.clamp(
    area.x + 10.,
    (area.x + area.w - width - 10.).max(area.x + 10.),
  );
  NotePlacement {
    left,
    arrow_x: (anchor_x - left).clamp(18., width - 18.),
    y,
    edge,
  }
}

/// cubic-bezier(.55,0,.85,.25) is close to t³: slow start, then it drops into the tray.
fn ease_in(t: f64) -> f64 {
  t * t * t
}

pub fn os_name() -> &'static str {
  if cfg!(target_os = "macos") {
    "mac"
  } else if cfg!(target_os = "windows") {
    "windows"
  } else {
    "linux"
  }
}

#[cfg(target_os = "macos")]
fn set_dock(app: &AppHandle, visible: bool) {
  use tauri::ActivationPolicy;
  let policy = if visible {
    ActivationPolicy::Regular
  } else {
    ActivationPolicy::Accessory
  };
  let _ = app.set_activation_policy(policy);
}

#[cfg(not(target_os = "macos"))]
fn set_dock(_app: &AppHandle, _visible: bool) {}

/// Shows and focuses the main window; on macOS the Dock icon comes back first.
pub fn show_main(app: &AppHandle) {
  let Some(window) = app.get_webview_window("main") else {
    return;
  };
  app.state::<BackgroundState>().transitions.begin();
  set_dock(app, true);
  let _ = window.unminimize();
  let _ = window.show();
  let _ = window.set_focus();
}

/// Hides the main window into the tray; on macOS the Dock icon goes away, only once it is hidden.
pub fn hide_main(app: &AppHandle) -> bool {
  let hidden = app
    .get_webview_window("main")
    .is_some_and(|window| window.hide().is_ok());
  set_dock(app, !hidden);
  hidden
}

/// Started hidden (sign-in launch): no Dock icon until the window is opened.
pub fn start_hidden(app: &AppHandle) {
  set_dock(app, false);
}

/// Any path that shows the window (tray, shortcut, card) ends in a focus: keep the Dock in step.
pub fn track_dock(app: &AppHandle) {
  let Some(window) = app.get_webview_window("main") else {
    return;
  };
  let handle = app.clone();
  window.on_window_event(move |event| {
    if matches!(event, WindowEvent::Focused(true)) {
      handle.state::<BackgroundState>().transitions.begin();
      set_dock(&handle, true);
    }
  });
}

/// Close button: hide into the tray. The very first time, shrink into the tray and explain.
pub fn close_to_tray(app: &AppHandle) {
  let ticket = app.state::<BackgroundState>().transitions.begin();
  if app
    .state::<SharedPreferences>()
    .load()
    .onboarding
    .close_note_done
  {
    hide_main(app);
    return;
  }
  let app = app.clone();
  std::thread::spawn(move || {
    let current = || {
      app
        .state::<BackgroundState>()
        .transitions
        .is_current(ticket)
    };
    let target = tray_anchor(&app);
    let animate = !app
      .state::<BackgroundState>()
      .reduce_motion
      .load(Ordering::Relaxed)
      && cfg!(not(target_os = "linux"));
    if let (true, Some(window), Some((x, y, _))) = (animate, app.get_webview_window("main"), target)
    {
      shrink_into(&app, &window, x, y, &current);
    }
    // Reopened from the tray (or a shortcut) meanwhile: stay open, keep the note for a later close.
    if !current() || !hide_main(&app) {
      return;
    }
    let prefs = app.state::<SharedPreferences>();
    if let Err(e) = prefs.apply_patch(&json!({ "onboarding": { "closeNoteDone": true } }), &app) {
      tracing::warn!("Failed to save first-close note state: {e}");
    }
    if let Err(e) = open_note(&app, target) {
      tracing::warn!("Failed to show the first-close note: {e}");
    }
  });
}

/// Logical point the window shrinks to and the note points at, plus the side the note opens on.
fn tray_anchor(app: &AppHandle) -> Option<(f64, f64, Edge)> {
  let window = app.get_webview_window("main")?;
  let monitor = window
    .current_monitor()
    .ok()
    .flatten()
    .or_else(|| window.primary_monitor().ok().flatten())?;
  let scale = monitor.scale_factor();
  let full = monitor.size().to_logical::<f64>(scale);
  let origin = monitor.position().to_logical::<f64>(scale);
  let work = monitor.work_area();
  let work_pos = work.position.to_logical::<f64>(scale);
  let work_size = work.size.to_logical::<f64>(scale);

  if cfg!(target_os = "windows") {
    // New tray icons land in the hidden-icons flyout: point at the ^ left of the clock.
    let bar_top = work_pos.y + work_size.height;
    let bar_h = (origin.y + full.height - bar_top).max(40.);
    return Some((
      origin.x + full.width - 150.,
      bar_top + bar_h / 2.,
      Edge::Bottom,
    ));
  }

  let tray_rect = app
    .state::<TrayState>()
    .tray
    .lock()
    .unwrap()
    .clone()
    .and_then(|id| app.tray_by_id(&id))
    .and_then(|tray| tray.rect().ok().flatten());
  if let Some(rect) = tray_rect {
    let p = rect.position.to_logical::<f64>(scale);
    let s = rect.size.to_logical::<f64>(scale);
    if s.width > 0. {
      return Some((p.x + s.width / 2., p.y + s.height, Edge::Top));
    }
  }
  // No tray rect (Linux, or a hidden menu bar): the top end of the screen.
  Some((
    origin.x + full.width - 60.,
    work_pos.y.max(origin.y + 26.),
    Edge::Top,
  ))
}

fn shrink_into(
  app: &AppHandle,
  window: &WebviewWindow,
  tx: f64,
  ty: f64,
  current: &dyn Fn() -> bool,
) {
  let state = app.state::<BackgroundState>();
  let Ok(scale) = window.scale_factor() else {
    return;
  };
  let (Ok(pos), Ok(size), Ok(inner)) = (
    window.outer_position(),
    window.outer_size(),
    window.inner_size(),
  ) else {
    return;
  };
  let pos = pos.to_logical::<f64>(scale);
  let size = size.to_logical::<f64>(scale);
  let inner = inner.to_logical::<f64>(scale);
  let min = app
    .config()
    .app
    .windows
    .first()
    .and_then(|w| Some((w.min_width?, w.min_height?)));

  state.animating.store(true, Ordering::Relaxed);
  let _ = window.set_min_size(None::<LogicalSize<f64>>);
  let (cx, cy) = (pos.x + size.width / 2., pos.y + size.height / 2.);
  let frame = Duration::from_millis(SHRINK_MS / SHRINK_FRAMES);
  for i in 1..=SHRINK_FRAMES {
    if !current() {
      break;
    }
    let e = ease_in(i as f64 / SHRINK_FRAMES as f64);
    let k = 1. - e * 0.94;
    let (w, h) = (inner.width * k, inner.height * k);
    let (x, y) = (cx + (tx - cx) * e, cy + (ty - cy) * e);
    let _ = window.set_size(LogicalSize::new(w.max(1.), h.max(1.)));
    let _ = window.set_position(LogicalPosition::new(x - w / 2., y - h / 2.));
    std::thread::sleep(frame);
  }
  // Full size again: hidden right after this (close), or visible again (cancelled by a reopen).
  if current() {
    let _ = window.hide();
  }
  let _ = window.set_size(LogicalSize::new(inner.width, inner.height));
  let _ = window.set_position(LogicalPosition::new(pos.x, pos.y));
  if let Some((w, h)) = min {
    let _ = window.set_min_size(Some(LogicalSize::new(w, h)));
  }
  // Let the restore events pass before tracking resumes.
  std::thread::sleep(Duration::from_millis(120));
  state.animating.store(false, Ordering::Relaxed);
}

fn open_note(app: &AppHandle, anchor: Option<(f64, f64, Edge)>) -> tauri::Result<()> {
  let window = app.get_webview_window("main");
  let monitor = window
    .as_ref()
    .and_then(|w| w.current_monitor().ok().flatten())
    .or_else(|| window.as_ref()?.primary_monitor().ok().flatten());
  let placement = match (anchor, monitor) {
    (Some((x, y, edge)), Some(m)) => {
      let scale = m.scale_factor();
      let pos = m.position().to_logical::<f64>(scale);
      let size = m.size().to_logical::<f64>(scale);
      let area = Area {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
      };
      let y = if edge == Edge::Top { y + 6. } else { y - 26. };
      place_note(x, y, edge, area, NOTE_WIDTH)
    }
    _ => NotePlacement {
      left: 40.,
      arrow_x: NOTE_WIDTH - 40.,
      y: 40.,
      edge: Edge::Top,
    },
  };
  *app.state::<BackgroundState>().note.lock().unwrap() = Some(placement);
  let edge = if placement.edge == Edge::Top {
    "top"
  } else {
    "bottom"
  };
  let url = format!(
    "index.html?window={NOTE_LABEL}&os={}&edge={edge}&nx={}",
    os_name(),
    placement.arrow_x.round()
  );
  if let Some(old) = app.get_webview_window(NOTE_LABEL) {
    let _ = old.destroy();
  }
  let note = WebviewWindowBuilder::new(app, NOTE_LABEL, WebviewUrl::App(url.into()))
    .title("Nezzel")
    .inner_size(NOTE_WIDTH, 200.)
    .visible(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .build()?;
  let handle = app.clone();
  note.on_window_event(move |event| {
    // Clicking anywhere else dismisses it, like a menu-bar popover.
    if matches!(event, WindowEvent::Focused(false)) {
      if let Some(w) = handle.get_webview_window(NOTE_LABEL) {
        if w.is_visible().unwrap_or(false) {
          let _ = w.close();
        }
      }
    }
  });
  Ok(())
}

/// The note page measured itself: size it, anchor it at the tray and give it focus.
#[tauri::command]
pub fn close_note_ready(app: AppHandle, window: WebviewWindow, height: f64) -> Result<(), String> {
  if window.label() != NOTE_LABEL {
    return Err("unavailable".into());
  }
  let placement = app
    .state::<BackgroundState>()
    .note
    .lock()
    .unwrap()
    .ok_or("unavailable")?;
  let height = height.clamp(80., 400.);
  let top = match placement.edge {
    Edge::Top => placement.y,
    Edge::Bottom => placement.y - height,
  };
  let _ = window.set_size(LogicalSize::new(NOTE_WIDTH, height));
  let _ = window.set_position(LogicalPosition::new(placement.left, top));
  window.show().map_err(|e| e.to_string())?;
  let _ = window.set_focus();
  Ok(())
}

/// "Got it" / Escape close the note; "Quit Nezzel" quits.
#[tauri::command]
pub fn close_note_done(app: AppHandle, window: WebviewWindow, quit: bool) -> Result<(), String> {
  if window.label() != NOTE_LABEL {
    return Err("unavailable".into());
  }
  let _ = window.destroy();
  if quit {
    app.exit(0);
  }
  Ok(())
}

#[tauri::command]
pub fn window_motion_pref(
  window: WebviewWindow,
  state: tauri::State<'_, BackgroundState>,
  reduce: bool,
) -> Result<(), String> {
  if window.label() != "main" {
    return Err("unavailable".into());
  }
  state.reduce_motion.store(reduce, Ordering::Relaxed);
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_reopen_cancels_a_running_close() {
    let t = Transitions::default();
    let close = t.begin();
    assert!(t.is_current(close));
    // Tray click / focus during the shrink.
    t.begin();
    assert!(!t.is_current(close));
    // A newer close is current again.
    let again = t.begin();
    assert!(t.is_current(again));
  }

  #[test]
  fn note_points_at_the_tray_and_stays_on_screen() {
    let screen = Area {
      x: 0.,
      y: 0.,
      w: 1440.,
      h: 900.,
    };
    // Menu bar icon on the right: note opens leftward, arrow over the icon.
    let p = place_note(1300., 30., Edge::Top, screen, 340.);
    assert_eq!(p.left, 1300. - 340. + 34.);
    assert_eq!(p.left + p.arrow_x, 1300.);
    // Icon at the far edge: note clamps inside the screen, arrow still on the icon.
    let p = place_note(1435., 30., Edge::Top, screen, 340.);
    assert_eq!(p.left, 1440. - 340. - 10.);
    assert_eq!(p.arrow_x, 340. - 18.);
    // Icon on the left (RTL desktops): note opens rightward.
    let p = place_note(80., 30., Edge::Top, screen, 340.);
    assert_eq!(p.left, 46.);
    // Second monitor to the left of the primary one.
    let left_screen = Area {
      x: -1920.,
      ..screen
    };
    let p = place_note(-100., 30., Edge::Top, left_screen, 340.);
    assert!(p.left >= -1920. + 10. && p.left + 340. <= -1920. + 1440. - 10.);
  }
}
