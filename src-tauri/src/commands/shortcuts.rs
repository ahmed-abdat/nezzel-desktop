use crate::state::config_models::{DEFAULT_CAPTURE_SHORTCUT, FALLBACK_CAPTURE_SHORTCUT};
use crate::SharedConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub fn register_shortcuts(app_handle: &AppHandle) {
  let cfg_handle = app_handle.state::<SharedConfig>();
  let cfg = cfg_handle.load();
  register_shortcuts_for(app_handle, &cfg.input);
}

pub fn register_shortcuts_for(
  app_handle: &AppHandle,
  input: &crate::state::config_models::InputSettings,
) {
  if !input.global_shortcuts {
    return;
  }

  // The only system-wide shortcut is Quick download. Queue shortcuts (download all, add and download now) are
  // in-app keys handled by the webview, so Nezzel no longer steals ⌃⇧V/⌃⇧D/⌃⇧↩ from other apps.
  register_capture_shortcut(app_handle, &input.capture_shortcut);
}

pub fn unregister_shortcuts(app_handle: &AppHandle) {
  crate::capture::set_shortcut(app_handle, None, false);
  if let Err(e) = app_handle.global_shortcut().unregister_all() {
    tracing::warn!(error = %e, "Failed to unregister global shortcuts");
  }
}

fn on_quick_bar(app: &AppHandle, _: &Shortcut, event: tauri_plugin_global_shortcut::ShortcutEvent) {
  if event.state() == ShortcutState::Pressed {
    crate::capture::open_quick_bar(app, true);
  }
}

/// While a shortcut recorder listens, the OS must not swallow the current Quick download combo (or fire the bar).
/// Pausing only releases the OS hook; resuming restores it silently, and reports a status only when that fails.
static PAUSED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn shortcuts_pause(app: AppHandle, window: WebviewWindow, paused: bool) {
  if window.label() != "main" {
    return;
  }
  PAUSED.store(paused, Ordering::Relaxed);
  let Some(current) = crate::capture::current_shortcut(&app) else {
    return;
  };
  if paused {
    let _ = app.global_shortcut().unregister(current.as_str());
    return;
  }
  if app.global_shortcut().is_registered(current.as_str()) {
    return;
  }
  let requested = app
    .state::<SharedConfig>()
    .load()
    .input
    .capture_shortcut
    .clone();
  if resume_quietly(&current, &requested, default_types_char()) && try_register(&app, &current) {
    // Same combo back: no status event, or a recorder waiting on its own change would take this as the answer.
    return;
  }
  // The combo is gone (taken while paused, or no longer a candidate): register through the candidates, so the
  // default's fallback and OS refusals apply, and report what is really active.
  if let Some(previous) = crate::capture::take_shortcut(&app) {
    let _ = app.global_shortcut().unregister(previous.as_str());
  }
  register_capture_shortcut(&app, &requested);
}

/// After a recorder pause, the paused combo can come back silently only if it is still one the stored setting
/// allows (e.g. not the default on a layout where AltGr+N types a character).
fn resume_quietly(current: &str, requested: &str, default_types_char: bool) -> bool {
  capture_candidates(requested, default_types_char).contains(&current)
}

/// Shortcuts to try, in order, for the stored Quick download shortcut. The default falls back to the
/// earlier 4-key default when the keyboard layout types a character for it (AltGr+N on Windows).
pub fn capture_candidates(requested: &str, default_types_char: bool) -> Vec<&str> {
  if requested != DEFAULT_CAPTURE_SHORTCUT {
    return vec![requested];
  }
  if default_types_char {
    vec![FALLBACK_CAPTURE_SHORTCUT]
  } else {
    vec![DEFAULT_CAPTURE_SHORTCUT, FALLBACK_CAPTURE_SHORTCUT]
  }
}

/// True when Ctrl+Alt+N types a character on the active keyboard layout: Windows reads Ctrl+Alt as AltGr, so
/// grabbing it would stop people typing that letter (Polish ń, and others). Asks the layout itself rather than
/// keeping a list of layout ids. The layout is the foreground window's, i.e. the one the user types in.
#[cfg(windows)]
fn default_types_char() -> bool {
  use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyExW, ToUnicodeEx, MAPVK_VK_TO_VSC,
  };
  const VK_N: u32 = 0x4E;
  const VK_CONTROL: usize = 0x11;
  const VK_MENU: usize = 0x12;
  const VK_LCONTROL: usize = 0xA2;
  const VK_RMENU: usize = 0xA5;
  // Bit 2: don't change the keyboard state (dead keys) while asking (Windows 10 1607+).
  const NO_STATE_CHANGE: u32 = 0x4;
  // SAFETY: plain Win32 queries on buffers that outlive the calls.
  unsafe {
    let layout = GetKeyboardLayout(foreground_thread());
    let scan = MapVirtualKeyExW(VK_N, MAPVK_VK_TO_VSC, layout);
    let mut state = [0u8; 256];
    for key in [VK_CONTROL, VK_MENU, VK_LCONTROL, VK_RMENU] {
      state[key] = 0x80;
    }
    let mut out = [0u16; 8];
    let len = ToUnicodeEx(
      VK_N,
      scan,
      state.as_ptr(),
      out.as_mut_ptr(),
      out.len() as i32,
      NO_STATE_CHANGE,
      layout,
    );
    types_printable(len, &out)
  }
}

/// Thread of the window the user is typing in; 0 (this thread) when there is none.
#[cfg(windows)]
fn foreground_thread() -> u32 {
  use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId,
  };
  // SAFETY: a null process-id pointer is allowed; a null window is checked first.
  unsafe {
    let window = GetForegroundWindow();
    if window.is_null() {
      return 0;
    }
    GetWindowThreadProcessId(window, std::ptr::null_mut())
  }
}

#[cfg(not(windows))]
fn default_types_char() -> bool {
  false
}

/// ToUnicodeEx result → whether it typed a visible character (not a control code, not nothing).
#[cfg_attr(not(windows), allow(dead_code))]
fn types_printable(len: i32, out: &[u16]) -> bool {
  // Negative: a dead key, which still produces a character on the next key press.
  if len < 0 {
    return true;
  }
  out.iter().take(len as usize).any(|&unit| unit >= 0x20)
}

fn try_register(app: &AppHandle, accelerator: &str) -> bool {
  let ok = accelerator.parse::<Shortcut>().is_ok_and(|shortcut| {
    app
      .global_shortcut()
      .on_shortcut(shortcut, on_quick_bar)
      .is_ok()
  });
  if !ok {
    // A half-registered combo (parsed, then refused by the OS) must not linger.
    let _ = app.global_shortcut().unregister(accelerator);
  }
  ok
}

fn shortcut_kind(accelerator: &str) -> &'static str {
  match accelerator {
    DEFAULT_CAPTURE_SHORTCUT => "default",
    FALLBACK_CAPTURE_SHORTCUT => "fallback",
    _ => "custom",
  }
}

/// Rebind the Quick download shortcut.
pub fn register_capture_shortcut(app: &AppHandle, requested: &str) {
  if let Some(reason) =
    crate::capture::clipboard::global_shortcut_reason(crate::capture::clipboard::current_session())
  {
    // The plugin's X11 grab "succeeds" under Wayland but never fires: say so instead of claiming the combo.
    if let Some(previous) = crate::capture::take_shortcut(app) {
      let _ = app.global_shortcut().unregister(previous.as_str());
    }
    tracing::info!(
      reason,
      "Quick download shortcut unavailable in this session"
    );
    crate::capture::set_shortcut_unsupported(app, reason);
    return;
  }
  let candidates = capture_candidates(requested, default_types_char());
  if let Some(previous) = crate::capture::take_shortcut(app) {
    if candidates.contains(&previous.as_str()) {
      crate::capture::set_shortcut(app, Some(previous), false);
      return;
    }
    let _ = app.global_shortcut().unregister(previous.as_str());
  }
  let active = candidates
    .into_iter()
    .find(|accelerator| try_register(app, accelerator));
  // Registering validates the combo with the OS; while a recorder listens, release the hook again right away.
  if let Some(active) = active {
    // Kinds only: a recorded combo is the user's own setting.
    tracing::info!(
      session = ?crate::capture::clipboard::current_session(),
      combo = shortcut_kind(active),
      "Quick download shortcut registered"
    );
    if PAUSED.load(Ordering::Relaxed) {
      let _ = app.global_shortcut().unregister(active);
    }
  }
  crate::capture::set_shortcut(app, active.map(String::from), active.is_none());
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn capture_candidates_table() {
    let custom = "CommandOrControl+Alt+K";
    let cases: [(&str, bool, &[&str]); 5] = [
      (
        DEFAULT_CAPTURE_SHORTCUT,
        false,
        &[DEFAULT_CAPTURE_SHORTCUT, FALLBACK_CAPTURE_SHORTCUT],
      ),
      (DEFAULT_CAPTURE_SHORTCUT, true, &[FALLBACK_CAPTURE_SHORTCUT]),
      (custom, false, &[custom]),
      // A layout that types AltGr+N only matters for the default; a recorded combo is the user's choice.
      (custom, true, &[custom]),
      (
        FALLBACK_CAPTURE_SHORTCUT,
        true,
        &[FALLBACK_CAPTURE_SHORTCUT],
      ),
    ];
    for (requested, types_char, expected) in cases {
      assert_eq!(
        capture_candidates(requested, types_char),
        expected,
        "{requested} types_char={types_char}"
      );
    }
  }

  #[test]
  fn resume_quietly_table() {
    let custom = "CommandOrControl+Alt+K";
    let cases = [
      (
        DEFAULT_CAPTURE_SHORTCUT,
        DEFAULT_CAPTURE_SHORTCUT,
        false,
        true,
      ),
      (
        FALLBACK_CAPTURE_SHORTCUT,
        DEFAULT_CAPTURE_SHORTCUT,
        false,
        true,
      ),
      // The layout now types AltGr+N: the default must not come back, the fallback may.
      (
        DEFAULT_CAPTURE_SHORTCUT,
        DEFAULT_CAPTURE_SHORTCUT,
        true,
        false,
      ),
      (
        FALLBACK_CAPTURE_SHORTCUT,
        DEFAULT_CAPTURE_SHORTCUT,
        true,
        true,
      ),
      (custom, custom, true, true),
      // Paused on the old combo, setting already changed: register the new one and report it.
      (DEFAULT_CAPTURE_SHORTCUT, custom, false, false),
    ];
    for (current, requested, types_char, expected) in cases {
      assert_eq!(
        resume_quietly(current, requested, types_char),
        expected,
        "{current} -> {requested} types_char={types_char}"
      );
    }
  }

  #[test]
  fn types_printable_table() {
    let n_acute = [0x0144u16, 0, 0, 0];
    let ctrl_n = [0x000Eu16, 0, 0, 0];
    let cases: [(i32, &[u16], bool); 4] = [
      (1, &n_acute, true),
      (0, &n_acute, false),
      (1, &ctrl_n, false),
      (-1, &[0u16; 4], true),
    ];
    for (len, out, expected) in cases {
      assert_eq!(types_printable(len, out), expected, "len={len} out={out:?}");
    }
  }
}
