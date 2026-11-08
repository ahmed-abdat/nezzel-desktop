//! Link capture: the copied-link card and the Quick download bar, from the link to the main window's queue.

mod browser;
pub(crate) mod choice;
pub mod clipboard;
mod commands;
mod outbox;
mod policy;
pub(crate) mod preview;
mod read;
mod state;
mod usage_keys;
mod watch;
mod window;

pub use choice::Choice;
#[cfg(test)]
pub(crate) use choice::GENERIC_HEIGHTS;
pub use commands::*;
#[cfg(test)]
pub(crate) use outbox::PULL_MS;
pub use state::{Action, CaptureState};
pub use watch::open_quick_bar;
#[cfg(test)]
pub(crate) use window::{
  BAR_HEIGHT, BAR_LABEL, BAR_WIDTH, CARD_HEIGHT, CARD_LABEL, CARD_MAX_HEIGHT, CARD_WIDTH,
};

use outbox::emit_action;
use state::adopt_on_focus;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use watch::watch_clipboard;

#[cfg(feature = "link-matrix")]
pub(crate) fn matrix_normalize(text: &str) -> Option<String> {
  policy::supported_url(text).map(|(url, _)| url).or_else(|| {
    let url = reqwest::Url::parse(text.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| text.trim().to_string())
  })
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
  app.manage(CaptureState::default());
  let keep_open = app
    .state::<crate::SharedConfig>()
    .load()
    .input
    .card_keep_open;
  app
    .state::<CaptureState>()
    .card_keep_open
    .store(keep_open, Ordering::Relaxed);
  // Config listeners receive the new value before the shared snapshot swaps. Publish the flag
  // directly before waking the timer, rather than waking it to read the previous snapshot.
  crate::state::config::listen(app, |app, config| {
    let state = app.state::<CaptureState>();
    {
      let mut offers = state.offers.lock().unwrap();
      if let Some(slot) = offers.slot_mut(window::CaptureWindow::Card).as_mut() {
        slot.card_timer.set_keep_open(config.input.card_keep_open);
      }
      state
        .card_keep_open
        .store(config.input.card_keep_open, Ordering::Relaxed);
    }
    state.changed.notify_waiters();
  });
  window::create(app)?;
  // The user may click into the main window while the bar is still open; closing the bar must
  // then leave Nezzel in front instead of hiding it.
  if let Some(main) = app.get_webview_window("main") {
    app
      .state::<CaptureState>()
      .main_is_focused
      .store(main.is_focused().unwrap_or(true), Ordering::Relaxed);
    let handle = app.clone();
    main.on_window_event(move |event| {
      if let tauri::WindowEvent::Focused(focused) = event {
        handle
          .state::<CaptureState>()
          .main_is_focused
          .store(*focused, Ordering::Relaxed);
      }
      if matches!(event, tauri::WindowEvent::Focused(true)) {
        handle.state::<CaptureState>().clipboard_wake.notify_one();
        handle
          .state::<CaptureState>()
          .main_was_focused
          .store(true, Ordering::Relaxed);
        let adopted = adopt_on_focus(&handle.state::<CaptureState>());
        if let Some((window, action)) = adopted {
          tracing::info!("Copied-link card adopted: the main window got focus");
          if let Some(card) = handle.get_webview_window(window.label()) {
            let _ = card.hide();
          }
          emit_action(&handle, action);
        }
      }
    });
  }
  let app = app.clone();
  tauri::async_runtime::spawn(watch_clipboard(app));
  Ok(())
}
