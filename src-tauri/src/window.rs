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

  if has_size {
    if let Err(e) = window.set_size(PhysicalSize::new(s.width, s.height)) {
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
  }
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

    use crate::state::config_models::CloseBehavior;
    use crate::SharedConfig;
    let cfg = app_handle.state::<SharedConfig>().load();
    api.prevent_close();
    if cfg.system.tray_enabled && matches!(cfg.system.close_behavior, CloseBehavior::Hide) {
      crate::background::close_to_tray(&app_handle);
    } else {
      app_handle.exit(0);
    }
  });
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
