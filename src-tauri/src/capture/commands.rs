//! The capture windows' and the main window's capture commands.

use super::choice::quick_choice;
use super::clipboard;
use super::outbox::{emit_action, pull, send_to_main, ForMain, Message};
use super::policy::supported_url;
use super::state::{claim_action, dismiss_offer, Action, CaptureState, Offer};
use super::watch::{data_control, read_current};
use super::window::{close_after_action, fit_height, offer, place_card, CaptureWindow, CARD_WIDTH};
use crate::SharedConfig;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, WebviewWindow};

/// Which capture window sent a command; any other window is refused.
fn capture_window(window: &WebviewWindow) -> Result<CaptureWindow, String> {
  CaptureWindow::from_label(window.label()).ok_or_else(|| "unavailable".into())
}

fn main_only(window: &WebviewWindow) -> Result<(), String> {
  if window.label() == "main" {
    Ok(())
  } else {
    Err("unavailable".into())
  }
}

#[tauri::command]
pub fn capture_ready(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
  main_only(&window)?;
  app
    .state::<CaptureState>()
    .ready
    .store(true, Ordering::Relaxed);
  Ok(())
}

/// The clipboard text, read with the same readers as the watcher (GTK first on Linux). `supported_only`
/// (the Add field's auto-fill): just the supported link, or "" when there is none or nothing can be read.
/// Without it (Ctrl/⌘+V outside the field, the Paste button): the raw text, or an error.
#[tauri::command]
pub async fn capture_read_clipboard(
  app: AppHandle,
  window: WebviewWindow,
  supported_only: Option<bool>,
) -> Result<String, String> {
  main_only(&window)?;
  let read = read_current(&app).await;
  if supported_only.unwrap_or(false) {
    return Ok(
      read
        .ok()
        .and_then(|text| supported_url(&text))
        .map(|(url, _)| url)
        .unwrap_or_default(),
    );
  }
  read.map_err(|e| format!("clipboard {}", e.kind()))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePlatform {
  pub session: crate::platform::Session,
  /// A copy in another app can raise the card while Nezzel is in the background.
  pub background_clipboard: bool,
  /// System shortcut command: installed package binary, or the persistent AppImage path.
  pub capture_command: String,
  /// Whose words the tray copy uses (Linux; `other` elsewhere).
  pub desktop: crate::platform::Desktop,
}

fn system_shortcut_command(appimage: Option<&str>) -> String {
  match appimage.filter(|path| !path.is_empty()) {
    // Quote the installed AppImage, never its temporary mounted executable.
    Some(path) => format!("'{}' --capture", path.replace('\'', "'\"'\"'")),
    None => "/usr/bin/nezzel --capture".into(),
  }
}

#[tauri::command]
pub async fn capture_platform(app: AppHandle) -> CapturePlatform {
  let session = crate::platform::current_session();
  CapturePlatform {
    session,
    background_clipboard: clipboard::background_clipboard(session, data_control(&app).await),
    capture_command: system_shortcut_command(std::env::var("APPIMAGE").ok().as_deref()),
    desktop: crate::platform::current_desktop_blocking().await,
  }
}

#[tauri::command]
pub fn capture_snapshot(app: AppHandle, window: WebviewWindow) -> Offer {
  let Ok(window) = capture_window(&window) else {
    return Offer::default();
  };
  app
    .state::<CaptureState>()
    .offers
    .lock()
    .unwrap()
    .current(window)
    .cloned()
    .unwrap_or_default()
}

#[tauri::command]
pub fn capture_edit(app: AppHandle, window: WebviewWindow, text: String) -> Result<(), String> {
  if capture_window(&window)? != CaptureWindow::Bar {
    return Err("unavailable".into());
  }
  offer(&app, CaptureWindow::Bar, &text, false)?;
  Ok(())
}

/// Failed read → "Retry": read the same link again in the same bar or card (a fresh offer id).
#[tauri::command]
pub fn capture_retry(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
  let window = capture_window(&window)?;
  let state = app.state::<CaptureState>();
  let url = state
    .offers
    .lock()
    .unwrap()
    .retry_url(window)
    .ok_or("nothing to retry")?;
  // The fresh offer retires the failed one (its read, item and claim).
  offer(&app, window, &url, false)?;
  Ok(())
}

/// The card's format list opened or closed: resize to `height` (clamped) and keep the card on its corner,
/// so on Windows and Linux it grows upward from the taskbar instead of past the screen edge.
#[tauri::command]
pub fn capture_fit(app: AppHandle, window: WebviewWindow, height: f64) -> Result<(), String> {
  let height = fit_height(height)
    .filter(|_| CaptureWindow::from_label(window.label()) == Some(CaptureWindow::Card))
    .ok_or("capture unavailable")?;
  window
    .set_size(tauri::LogicalSize::new(CARD_WIDTH, height))
    .map_err(|_| "capture unavailable")?;
  place_card(&app, &window, height);
  Ok(())
}

/// Messages for the main window after `after`, the last seq it handled (which acknowledges everything up to it).
/// It pulls on `capture_wake`, on focus and every `PULL_MS`.
#[tauri::command]
pub fn capture_pull(
  app: AppHandle,
  window: WebviewWindow,
  after: u64,
) -> Result<Vec<Message>, String> {
  main_only(&window)?;
  Ok(pull(&app.state::<CaptureState>(), after))
}

#[tauri::command]
pub async fn capture_action(
  app: AppHandle,
  window: WebviewWindow,
  id: u64,
  action: String,
  // A `choice::Choice` object; read leniently so a bad one means the default choice, not a lost download.
  choice: Option<serde_json::Value>,
) -> Result<(), String> {
  let native = window;
  let window = capture_window(&native)?;
  let Some(action) = Action::parse(&action).filter(|action| Action::FROM_WINDOW.contains(action))
  else {
    return Err("unavailable".into());
  };
  let state = app.state::<CaptureState>();
  if action == Action::Report {
    let report = state
      .offers
      .lock()
      .unwrap()
      .take_report(window, id)
      .ok_or("nothing to report")?;
    state.changed.notify_waiters();
    native.hide().map_err(|_| "capture unavailable")?;
    if let Some(main) = app.get_webview_window("main") {
      let _ = main.show();
      let _ = main.set_focus();
    }
    send_to_main(&app, ForMain::Report(report));
    return Ok(());
  }
  if action == Action::Dismiss {
    // Closing never fails: hide first, then settle whatever offer this window still holds.
    close_after_action(&app, &native, window, false);
    dismiss_offer(&state, window, id);
    for key in super::usage_keys::action_keys(window.label(), Action::Dismiss, None) {
      crate::usage::count(&app, key);
    }
    return Ok(());
  }
  // The rows this offer actually showed; a card row it did not show falls back to the default choice.
  let offered = state.offers.lock().unwrap().shown_choices(window, id);
  let mut payload = claim_action(&state, window, id, action)?;
  if action == Action::Download {
    payload.choice = quick_choice(choice, &offered);
  }
  for key in super::usage_keys::action_keys(window.label(), action, payload.choice) {
    crate::usage::count(&app, key);
  }
  if let Some(other) = payload.retired {
    if let Some(other) = app.get_webview_window(other.label()) {
      let _ = other.hide();
    }
  }
  // Claimed already: a failed hide must not lose the download.
  close_after_action(&app, &native, window, action == Action::Options);
  emit_action(&app, payload);
  Ok(())
}

#[tauri::command]
pub async fn capture_write_clipboard(
  app: AppHandle,
  window: WebviewWindow,
  text: String,
) -> Result<(), String> {
  main_only(&window)?;
  clipboard::write_text(&app, text, true).await
}

/// Welcome "Try it" sample: a short, long-lived public video ("Me at the zoo").
pub const TRY_LINK: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

/// Welcome "Try it": copies the fixed sample link, and the clipboard watcher brings up the real card
/// for it like any copied link (dedupe included). Only with the card setting on; never reads the clipboard.
#[tauri::command]
pub async fn capture_try_link(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
  main_only(&window)?;
  if !app.state::<SharedConfig>().load().input.watch_clipboard {
    return Err("card off".into());
  }
  clipboard::write_text(&app, TRY_LINK.to_owned(), false).await
}

#[cfg(test)]
mod tests {
  use super::system_shortcut_command;

  #[test]
  fn system_shortcut_uses_the_package_binary_or_quotes_the_appimage_path() {
    for (appimage, expected) in [
      (None, "/usr/bin/nezzel --capture"),
      (Some(""), "/usr/bin/nezzel --capture"),
      (
        Some("/home/me/My Apps/Nezzel.AppImage"),
        "'/home/me/My Apps/Nezzel.AppImage' --capture",
      ),
      (
        Some("/home/me/Nezzel's.AppImage"),
        "'/home/me/Nezzel'\"'\"'s.AppImage' --capture",
      ),
    ] {
      assert_eq!(system_shortcut_command(appimage), expected);
    }
  }
}
