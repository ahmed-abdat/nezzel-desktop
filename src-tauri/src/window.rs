use crate::state::json_merge;
use crate::SharedPreferences;
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::{
  AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent, Wry,
};
use tokio::sync::Mutex;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPreferencesPatch {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub width: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub height: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub x: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub y: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub prev_x: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub prev_y: Option<i32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub maximized: Option<bool>,
}

impl WindowPreferencesPatch {
  pub fn empty() -> Self {
    Self {
      width: None,
      height: None,
      x: None,
      y: None,
      prev_x: None,
      prev_y: None,
      maximized: None,
    }
  }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesPatch {
  pub window: WindowPreferencesPatch,
}

struct DebouncedPatch {
  pending: Value,
  /// Maximize generation when the first patch of this batch was queued.
  generation: Option<u64>,
  task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct PatchDebouncer {
  inner: Arc<Mutex<DebouncedPatch>>,
  delay: Duration,
  app: AppHandle<Wry>,
  /// Bumped whenever an event sees the window maximized (or can't tell).
  maximized: Arc<AtomicU64>,
}

/// Only a window known to be in its normal state has geometry worth restoring; a failed query
/// counts as maximized (fail closed).
fn is_normal(maximized: Option<bool>) -> bool {
  maximized == Some(false)
}

/// GTK can deliver the Moved/Resized of a maximize before `is_maximized` turns true, so a batch is
/// saved only if no event saw a maximized window since it started and the window is normal now.
/// Dropping a batch keeps the last normal geometry on disk.
fn should_persist(batch: Option<u64>, now: u64, maximized: Option<bool>) -> bool {
  batch.is_none_or(|batch| batch == now) && is_normal(maximized)
}

/// The KWayland title-bar workaround below, never on a maximized window: a non-resizable GTK window
/// pins its size request to the current (maximized) size.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn should_toggle_resizable(
  focused: bool,
  session: crate::capture::clipboard::Session,
  maximized: Option<bool>,
) -> bool {
  focused && session == crate::capture::clipboard::Session::Wayland && is_normal(maximized)
}

fn main_maximized(app: &AppHandle<Wry>) -> Option<bool> {
  app.get_webview_window("main")?.is_maximized().ok()
}

impl PatchDebouncer {
  fn new(app: AppHandle<Wry>, delay: Duration) -> Self {
    Self {
      inner: Arc::new(Mutex::new(DebouncedPatch {
        pending: Value::Object(Default::default()),
        generation: None,
        task: None,
      })),
      delay,
      app,
      maximized: Arc::new(AtomicU64::new(0)),
    }
  }

  /// Records what an event saw; true when the window is normal and its geometry may be queued.
  fn observe(&self, maximized: Option<bool>) -> bool {
    if !is_normal(maximized) {
      self.maximized.fetch_add(1, Ordering::Relaxed);
    }
    is_normal(maximized)
  }

  /// Takes the batch; Some only when it may be saved.
  fn take_batch(&self, g: &mut DebouncedPatch) -> Option<Value> {
    let pending = std::mem::replace(&mut g.pending, Value::Object(Default::default()));
    let batch = g.generation.take();
    let now = self.maximized.load(Ordering::Relaxed);
    let keep = should_persist(batch, now, main_maximized(&self.app));
    (keep && !pending.as_object().is_some_and(|o| o.is_empty())).then_some(pending)
  }

  async fn push(&self, patch: Value) {
    let mut g = self.inner.lock().await;

    if g.generation.is_none() {
      g.generation = Some(self.maximized.load(Ordering::Relaxed));
    }
    json_merge(&mut g.pending, &patch);

    if let Some(t) = g.task.take() {
      t.abort();
    }

    let this = self.clone();
    let app = self.app.clone();
    let delay = self.delay;

    g.task = Some(tauri::async_runtime::spawn(async move {
      tokio::time::sleep(delay).await;

      let Some(to_flush) = ({
        let mut g = this.inner.lock().await;
        g.task = None;
        this.take_batch(&mut g)
      }) else {
        return;
      };

      let prefs = match app.try_state::<SharedPreferences>() {
        Some(h) => h,
        None => {
          tracing::warn!("PreferencesHandle not available during debounced flush");
          return;
        }
      };

      if let Err(e) = prefs.apply_patch(&to_flush, &app) {
        tracing::warn!("Failed to persist debounced window patch: {e}");
      }
    }));
  }

  async fn flush_now(&self) {
    let Some(to_flush) = ({
      let mut g = self.inner.lock().await;
      if let Some(t) = g.task.take() {
        t.abort();
      }
      self.take_batch(&mut g)
    }) else {
      return;
    };

    let prefs = match self.app.try_state::<SharedPreferences>() {
      Some(h) => h,
      None => {
        tracing::warn!("PreferencesHandle not available during immediate flush");
        return;
      }
    };

    if let Err(e) = prefs.apply_patch(&to_flush, &self.app) {
      tracing::warn!("Failed to persist window patch (flush_now): {e}");
    }
  }
}

pub fn restore_main_window(app: &AppHandle<Wry>) {
  let prefs_handle = match app.try_state::<SharedPreferences>() {
    Some(h) => h,
    None => {
      tracing::warn!("PreferencesHandle not available, skipping window restore");
      return;
    }
  };

  let window = match app.get_webview_window("main") {
    Some(w) => w,
    None => {
      tracing::warn!("Main window not found, skipping window restore");
      return;
    }
  };

  let prefs = prefs_handle.load();
  let s = &prefs.window;

  let (rx, ry) = if s.maximized {
    (s.prev_x, s.prev_y)
  } else {
    (s.x, s.y)
  };

  let has_size = s.width > 0 && s.height > 0;

  let ok_to_apply_position = if has_size {
    window_rect_is_on_some_monitor(&window, rx, ry, s.width, s.height).unwrap_or_else(|e| {
      tracing::warn!("Monitor check failed, skipping window position restore: {e}");
      false
    })
  } else {
    true
  };

  let restored = has_size.then(|| PhysicalSize::new(s.width, s.height));
  if let Some(size) = restored {
    if let Err(e) = window.set_size(size) {
      tracing::warn!("Failed to restore window size: {e}");
    }
  }

  if ok_to_apply_position {
    if let Err(e) = window.set_position(PhysicalPosition::new(rx, ry)) {
      tracing::warn!("Failed to restore window position: {e}");
    }
  }

  if s.maximized {
    if let Err(e) = window.maximize() {
      tracing::warn!("Failed to restore maximized state: {e}");
    }
  } else {
    fit_main_window_on_screen(&window, restored);
  }
}

/// The window's outer size, its frame (outer minus inner) and whether its position is known.
/// GTK (tao 0.37) caches the outer size only from configure events, which fire once the window is
/// mapped; before that it reports the window origin (~0x0) and the inner size is the default size
/// or the last requested one. #36: the fit saw a 0x0 window, "fitted", and the 900 px window opened
/// under the KDE panel. An outer size smaller than the inner one means "not mapped yet": use the
/// inner size, frame unknown (0) and position unknown.
fn window_extent(outer: (u32, u32), inner: (u32, u32)) -> ((u32, u32), (u32, u32), bool) {
  if outer.0 < inner.0 || outer.1 < inner.1 {
    return (inner, (0, 0), false);
  }
  (outer, (outer.0 - inner.0, outer.1 - inner.1), true)
}

/// The window rect (outer, physical px) moved and shrunk to lie inside `area`, or None when it already
/// fits. A 1280x800 screen has less height than the default 900 px window: Get started sat below the
/// screen edge. Shrinks only as far as the area allows; the OS minimum size still applies.
fn fit_rect(
  rect: (i32, i32, u32, u32),
  area: (i32, i32, u32, u32),
) -> Option<(i32, i32, u32, u32)> {
  let (x, y, w, h) = rect;
  let (ax, ay, aw, ah) = area;
  let fits = x >= ax && y >= ay && x + w as i32 <= ax + aw as i32 && y + h as i32 <= ay + ah as i32;
  if fits || aw == 0 || ah == 0 {
    return None;
  }
  let (w, h) = (w.min(aw), h.min(ah));
  let x = x.clamp(ax, ax + (aw - w) as i32);
  let y = y.clamp(ay, ay + (ah - h) as i32);
  Some((x, y, w, h))
}

/// New position (None: leave it to the compositor) and new outer size.
type FitPlan = (Option<(i32, i32)>, (u32, u32));

/// What to change so the window fits `area`: a new position (only when it is known, i.e. not on
/// Wayland) and a new outer size; None when it already fits.
fn fit_plan(
  position: Option<(i32, i32)>,
  size: (u32, u32),
  area: (i32, i32, u32, u32),
) -> Option<FitPlan> {
  let (x, y) = position.unwrap_or((area.0, area.1));
  let (nx, ny, w, h) = fit_rect((x, y, size.0, size.1), area)?;
  Some((position.map(|_| (nx, ny)), (w, h)))
}

/// Logical px kept free when Linux reports no work area: a panel (KDE 44, GNOME 32) plus a
/// client-side title bar that the outer size does not include.
const LINUX_UNKNOWN_CHROME: f64 = 96.;

type Rect = (i32, i32, u32, u32);

/// The area the window may cover. GDK has no work area on Wayland and returns the whole monitor,
/// so panels are not subtracted (#36: Get started sat under the KDE taskbar at 1280x800). X11 has
/// a real work area (_NET_WORKAREA); a monitor without panels there keeps its full height.
fn usable_area(work: Rect, monitor: Rect, scale: f64, wayland: bool) -> Rect {
  if !wayland || work != monitor {
    return work;
  }
  let reserve = (LINUX_UNKNOWN_CHROME * scale).round() as u32;
  (work.0, work.1, work.2, work.3.saturating_sub(reserve))
}

/// `restored`: the saved inner size just requested, which an unmapped GTK window does not report yet.
fn fit_main_window_on_screen(window: &WebviewWindow, restored: Option<PhysicalSize<u32>>) {
  // An unmapped GTK window has no GdkWindow, so current_monitor falls back to the primary monitor,
  // which GDK Wayland does not name: take the first monitor then.
  let monitor = window
    .current_monitor()
    .ok()
    .flatten()
    .or_else(|| window.primary_monitor().ok().flatten())
    .or_else(|| window.available_monitors().ok()?.into_iter().next());
  let (Some(monitor), Ok(outer), Ok(inner)) = (monitor, window.outer_size(), window.inner_size())
  else {
    tracing::info!("Window fit skipped: no monitor or size");
    return;
  };
  let (outer, frame, mapped) =
    window_extent((outer.width, outer.height), (inner.width, inner.height));
  let outer = match restored {
    Some(s) if !mapped => (s.width, s.height),
    _ => outer,
  };
  // Wayland never reveals a window's real position (tao reports cached numbers) and ignores moves:
  // fit the size only and let the compositor place it.
  let wayland = cfg!(target_os = "linux")
    && crate::capture::clipboard::current_session() == crate::capture::clipboard::Session::Wayland;
  let work = monitor.work_area();
  let area = usable_area(
    (
      work.position.x,
      work.position.y,
      work.size.width,
      work.size.height,
    ),
    (
      monitor.position().x,
      monitor.position().y,
      monitor.size().width,
      monitor.size().height,
    ),
    monitor.scale_factor(),
    wayland,
  );
  let known = (!wayland && mapped)
    .then(|| window.outer_position().ok().map(|p| (p.x, p.y)))
    .flatten();
  tracing::info!(
    monitor = ?(monitor.size().width, monitor.size().height),
    work = ?(work.size.width, work.size.height),
    area = ?(area.2, area.3),
    window = ?outer,
    mapped,
    wayland,
    "Window fit"
  );
  let Some((position, (w, h))) = fit_plan(known, outer, area) else {
    return;
  };
  // set_size takes the inner size: keep the frame the OS adds around it.
  let size = PhysicalSize::new(w.saturating_sub(frame.0), h.saturating_sub(frame.1));
  if let Err(e) = window.set_size(size) {
    tracing::warn!("Failed to fit the window on the screen: {e}");
  }
  if let Some((x, y)) = position {
    let _ = window.set_position(PhysicalPosition::new(x, y));
  }
  tracing::info!(
    width = size.width,
    height = size.height,
    "Window shrunk to fit the screen"
  );
}

pub fn track_main_window(app: &AppHandle<Wry>) {
  let window = match app.get_webview_window("main") {
    Some(w) => w,
    None => {
      tracing::warn!("Main window not found, skipping window tracking");
      return;
    }
  };

  let app_handle = app.clone();
  let window_for_events = window.clone();

  let debouncer = PatchDebouncer::new(app_handle.clone(), Duration::from_millis(300));

  window.on_window_event(move |event| {
    let debouncer = debouncer.clone();
    // The first-close shrink is not a size the user chose.
    if app_handle
      .state::<crate::background::BackgroundState>()
      .animating
      .load(std::sync::atomic::Ordering::Relaxed)
    {
      return;
    }

    match event {
      WindowEvent::Moved(pos) => {
        if !debouncer.observe(window_for_events.is_maximized().ok()) {
          return;
        }

        let patch = PreferencesPatch {
          window: WindowPreferencesPatch {
            x: Some(pos.x),
            y: Some(pos.y),
            prev_x: Some(pos.x),
            prev_y: Some(pos.y),
            ..WindowPreferencesPatch::empty()
          },
        };

        if let Ok(v) = serde_json::to_value(patch) {
          tauri::async_runtime::spawn(async move {
            debouncer.push(v).await;
          });
        }
      }
      WindowEvent::Resized(_) => {
        if !debouncer.observe(window_for_events.is_maximized().ok()) {
          return;
        }

        if let Ok(size) = window_for_events.inner_size() {
          let patch = PreferencesPatch {
            window: WindowPreferencesPatch {
              width: Some(size.width),
              height: Some(size.height),
              ..WindowPreferencesPatch::empty()
            },
          };

          if let Ok(v) = serde_json::to_value(patch) {
            tauri::async_runtime::spawn(async move {
              debouncer.push(v).await;
            });
          }
        }
      }
      WindowEvent::CloseRequested { .. } => {
        tauri::async_runtime::spawn(async move {
          debouncer.flush_now().await;
        });
      }
      #[cfg(target_os = "linux")]
      // Workaround for Wayland environments on Linux:
      // On KWayland, after hiding and showing the window,
      // the title-bar buttons (close, minimize, maximize) may stop working.
      // Toggling the resizable property appears to resolve this issue.
      // Info: https://github.com/tauri-apps/tao/issues/1046
      WindowEvent::Focused(focused) => {
        let maximized = window_for_events.is_maximized().ok();
        debouncer.observe(maximized);
        if !should_toggle_resizable(
          *focused,
          crate::capture::clipboard::current_session(),
          maximized,
        ) {
          return;
        }

        let _ = window_for_events.set_resizable(false);
        let _ = window_for_events.set_resizable(true);
      }
      _ => {}
    }
  });
}

pub fn setup_close_behaviour(app: &AppHandle<Wry>) {
  let app_handle = app.clone();

  let window = match app_handle.get_webview_window("main") {
    Some(w) => w,
    None => {
      tracing::warn!("Main window not found; cannot wire close behavior");
      return;
    }
  };

  window.on_window_event(move |event| {
    let WindowEvent::CloseRequested { api, .. } = event else {
      return;
    };

    use crate::SharedConfig;
    let cfg = app_handle.state::<SharedConfig>().load();
    api.prevent_close();
    if close_hides(
      cfg.system.tray_enabled,
      &cfg.system.close_behavior,
      crate::tray::tray_present(&app_handle),
    ) {
      crate::background::close_to_tray(&app_handle);
    } else {
      app_handle.exit(0);
    }
  });
}

/// Close hides into the tray only when a tray icon exists; otherwise there'd be no way back.
fn close_hides(
  tray_enabled: bool,
  behavior: &crate::state::config_models::CloseBehavior,
  tray_present: bool,
) -> bool {
  use crate::state::config_models::CloseBehavior;
  tray_enabled && tray_present && matches!(behavior, CloseBehavior::Hide)
}

fn rect_intersects_monitor(x: i32, y: i32, w: u32, h: u32, m: &Monitor) -> bool {
  let mp = *m.position();
  let ms = *m.size();

  let m_left = mp.x;
  let m_top = mp.y;
  let m_right = mp.x + ms.width as i32;
  let m_bottom = mp.y + ms.height as i32;

  let left = x;
  let top = y;
  let right = x + w as i32;
  let bottom = y + h as i32;

  left < m_right && right > m_left && top < m_bottom && bottom > m_top
}

fn window_rect_is_on_some_monitor(
  window: &WebviewWindow,
  x: i32,
  y: i32,
  w: u32,
  h: u32,
) -> tauri::Result<bool> {
  let monitors = window.available_monitors()?;
  if monitors.is_empty() {
    return Ok(true);
  }

  Ok(
    monitors
      .iter()
      .any(|m| rect_intersects_monitor(x, y, w, h, m)),
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::capture::clipboard::Session;

  #[test]
  fn close_hides_only_into_a_tray_that_exists() {
    use crate::state::config_models::CloseBehavior::{Exit, Hide};
    for (enabled, behavior, present, hides) in [
      (true, Hide, true, true),
      // Tray setting on, but no appindicator library: no icon, so quit instead of vanishing.
      (true, Hide, false, false),
      (false, Hide, false, false),
      (true, Exit, true, false),
    ] {
      assert_eq!(close_hides(enabled, &behavior, present), hides);
    }
  }

  #[test]
  fn linux_without_a_work_area_keeps_room_for_the_panel_and_title_bar() {
    let screen = (0, 0, 1280, 800);
    // Fedora KDE Wayland at 1280x800: GDK reports the whole monitor as the work area.
    let area = usable_area(screen, screen, 1., true);
    let (_, (_, h)) = fit_plan(None, (800, 900), area).unwrap();
    assert!(h + 44 + 37 <= 800, "panel 44 + title bar 37 must fit: {h}");
    assert!(h >= 650, "still above the minimum height: {h}");
    assert_eq!(usable_area(screen, screen, 2., true).3, 800 - 192);
    // A real work area (X11, macOS, Windows) is used as it is.
    let work = (0, 32, 1280, 768);
    assert_eq!(usable_area(work, screen, 1., true), work);
    // X11 (or any non-Wayland session) reporting the whole monitor: no panels there, keep it.
    assert_eq!(usable_area(screen, screen, 1., false), screen);
    let second = (1280, 0, 1920, 1080);
    assert_eq!(usable_area(second, second, 1., false), second);
  }

  #[test]
  fn an_unmapped_gtk_window_is_fitted_from_its_inner_size() {
    for (outer, inner, expected) in [
      // GTK before the first map: tao reports the origin as the outer size (#36).
      ((0, 0), (800, 900), ((800, 900), (0, 0), false)),
      ((0, 0), (0, 0), ((0, 0), (0, 0), true)),
      // Mapped (or macOS/Windows): the frame is outer minus inner.
      ((800, 937), (800, 900), ((800, 937), (0, 37), true)),
      ((816, 939), (800, 900), ((816, 939), (16, 39), true)),
    ] {
      assert_eq!(window_extent(outer, inner), expected, "{outer:?} {inner:?}");
    }
    // Fedora KDE Wayland 1280x800, first launch: the 0x0 outer size used to skip the fit.
    let screen = (0, 0, 1280, 800);
    let area = usable_area(screen, screen, 1., true);
    let (outer, _, _) = window_extent((0, 0), (800, 900));
    assert_eq!(fit_plan(None, outer, area), Some((None, (800, 704))));
  }

  #[test]
  fn the_window_is_moved_and_shrunk_onto_the_screen_only_when_it_spills() {
    let area = (0, 32, 1280, 736); // 1280x800 with a 32 px top panel
    for (rect, expected) in [
      // Default 800x900 centred: taller than the work area.
      ((240, -50, 800, 900), Some((240, 32, 800, 736))),
      // Fits already: untouched.
      ((100, 100, 800, 600), None),
      // Off the right edge: moved back, same size.
      ((900, 100, 800, 600), Some((480, 100, 800, 600))),
      // Wider and taller than the screen.
      ((0, 0, 2000, 1200), Some((0, 32, 1280, 736))),
    ] {
      assert_eq!(fit_rect(rect, area), expected, "{rect:?}");
    }
    assert_eq!(fit_rect((0, 0, 800, 900), (0, 0, 0, 0)), None);
    // Wayland (position unknown): size only, never a move.
    assert_eq!(fit_plan(None, (800, 900), area), Some((None, (800, 736))));
    assert_eq!(fit_plan(None, (800, 600), area), None);
    assert_eq!(
      fit_plan(Some((240, -50)), (800, 900), area),
      Some((Some((240, 32)), (800, 736)))
    );
  }

  #[test]
  fn geometry_is_saved_only_from_a_batch_that_never_saw_a_maximize() {
    for (batch, now, maximized, expected) in [
      (Some(3), 3, Some(false), true),
      (None, 3, Some(false), true),
      // A maximize was seen after the batch started (GTK queued its Resized as "normal").
      (Some(3), 4, Some(false), false),
      (Some(3), 3, Some(true), false),
      // Query failed: fail closed, keep the last normal geometry.
      (Some(3), 3, None, false),
    ] {
      assert_eq!(
        should_persist(batch, now, maximized),
        expected,
        "{batch:?} {now} {maximized:?}"
      );
    }
  }

  #[test]
  fn resizable_toggle_only_on_wayland_focus_of_a_normal_window() {
    for (focused, session, maximized, expected) in [
      (true, Session::Wayland, Some(false), true),
      (false, Session::Wayland, Some(false), false),
      (true, Session::Wayland, Some(true), false),
      (true, Session::Wayland, None, false),
      (true, Session::X11, Some(false), false),
      (true, Session::Other, Some(false), false),
    ] {
      assert_eq!(
        should_toggle_resizable(focused, session, maximized),
        expected,
        "{focused} {session:?} {maximized:?}"
      );
    }
  }
}
