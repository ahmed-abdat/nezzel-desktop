use crate::i18n::I18nManager;
use crate::models::payloads::ShortcutPayload;
use crate::SharedConfig;
use std::sync::Mutex;
use tauri::menu::{MenuBuilder, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconId};
use tauri::{AppHandle, Emitter, Manager};

pub struct TrayState {
  pub tray: Mutex<Option<TrayIconId>>,
  pub active: Mutex<Option<MenuItem<tauri::Wry>>>,
  pub counts: Mutex<(usize, Vec<String>)>,
}

pub fn create_tray(app: &AppHandle) {
  let cfg = app.state::<SharedConfig>().load();
  create_tray_for(app, &cfg);
}

pub fn create_tray_for(app: &AppHandle, cfg: &crate::state::config_models::Config) {
  let tray_state = app.state::<TrayState>();
  let mut guard = tray_state.tray.lock().unwrap();

  if guard.is_some() {
    return;
  }

  if !cfg.system.tray_enabled {
    return;
  }

  if !indicator_available() {
    return;
  }

  let i18n_handle = app.state::<I18nManager>();

  let quit_i = match MenuItem::with_id(app, "quit", i18n_handle.t("tray.quit"), true, None::<&str>)
  {
    Ok(i) => i,
    Err(e) => {
      tracing::warn!("Failed to create quit menu item: {e}");
      return;
    }
  };

  let hide_toggle_i = match MenuItem::with_id(
    app,
    "hide_toggle",
    i18n_handle.t("tray.showOrHide"),
    true,
    None::<&str>,
  ) {
    Ok(i) => i,
    Err(e) => {
      tracing::warn!("Failed to create hide toggle menu item: {e}");
      return;
    }
  };

  let add_to_queue_i = match MenuItem::with_id(
    app,
    "add_to_queue",
    i18n_handle.t("tray.addToQueue"),
    true,
    Some(if cfg!(target_os = "macos") {
      i18n_handle.t("tray.shortcuts.ctrlShiftV")
    } else {
      i18n_handle.t("tray.shortcuts.altShiftV")
    }),
  ) {
    Ok(i) => i,
    Err(e) => {
      tracing::warn!("Failed to create add_to_queue menu item: {e}");
      return;
    }
  };

  let download_i = match MenuItem::with_id(
    app,
    "download",
    i18n_handle.t("tray.downloadQueue"),
    true,
    Some(if cfg!(target_os = "macos") {
      i18n_handle.t("tray.shortcuts.ctrlShiftEnter")
    } else {
      i18n_handle.t("tray.shortcuts.altShiftEnter")
    }),
  ) {
    Ok(i) => i,
    Err(e) => {
      tracing::warn!("Failed to create download menu item: {e}");
      return;
    }
  };

  let add_and_download_i = match MenuItem::with_id(
    app,
    "add_and_download",
    i18n_handle.t("tray.addAndDownload"),
    true,
    Some(if cfg!(target_os = "macos") {
      i18n_handle.t("tray.shortcuts.ctrlShiftD")
    } else {
      i18n_handle.t("tray.shortcuts.altShiftD")
    }),
  ) {
    Ok(i) => i,
    Err(e) => {
      tracing::warn!("Failed to create add_and_download menu item: {e}");
      return;
    }
  };

  let paste_i = match MenuItem::with_id(
    app,
    "capture_paste",
    i18n_handle.t("tray.pasteLink"),
    true,
    None::<&str>,
  ) {
    Ok(item) => item,
    Err(_) => return,
  };
  let active_i = match MenuItem::with_id(
    app,
    "capture_active",
    {
      let (count, titles) = &*tray_state.counts.lock().unwrap();
      active_text(app, *count, titles)
    },
    false,
    None::<&str>,
  ) {
    Ok(item) => item,
    Err(_) => return,
  };
  *tray_state.active.lock().unwrap() = Some(active_i.clone());

  let menu = match MenuBuilder::new(app)
    .item(&paste_i)
    .item(&active_i)
    .separator()
    .item(&add_to_queue_i)
    .item(&add_and_download_i)
    .item(&download_i)
    .separator()
    .item(&hide_toggle_i)
    .item(&quit_i)
    .build()
  {
    Ok(m) => m,
    Err(e) => {
      tracing::warn!("Failed to build tray menu: {e}");
      return;
    }
  };

  let tray = match TrayIconBuilder::new()
    .menu(&menu)
    .icon(app.default_window_icon().unwrap().clone())
    .show_menu_on_left_click(cfg!(target_os = "macos"))
    .on_menu_event(|app, event| match event.id.as_ref() {
      "quit" => app.exit(0),
      "capture_paste" => crate::capture::open_quick_bar(app, false),

      "hide_toggle" => {
        if let Some(window) = app.get_webview_window("main") {
          if window.is_visible().unwrap_or(false) {
            crate::background::hide_main(app);
          } else {
            crate::background::show_main(app);
          }
        }
      }

      "add_to_queue" => {
        let _ = app.emit(
          "shortcut_action",
          ShortcutPayload {
            action: "media_add",
          },
        );
      }

      "download" => {
        let _ = app.emit(
          "shortcut_action",
          ShortcutPayload {
            action: "download_all",
          },
        );
      }

      "add_and_download" => {
        let _ = app.emit(
          "shortcut_action",
          ShortcutPayload {
            action: "media_add_and_download",
          },
        );
      }

      _ => {}
    })
    .build(app)
  {
    Ok(t) => t,
    Err(e) => {
      tracing::warn!("Failed to create tray icon: {e}");
      return;
    }
  };

  #[cfg(target_os = "windows")]
  {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};

    tray.on_tray_icon_event(|tray, event| {
      if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
      } = event
      {
        crate::background::show_main(tray.app_handle());
      }
    });
  }

  *guard = Some(tray.id().clone());
}

/// True when a tray icon is live, so hiding the window leaves a way back.
pub fn tray_present(app: &AppHandle) -> bool {
  app
    .try_state::<TrayState>()
    .is_some_and(|s| s.tray.lock().unwrap().is_some())
}

/// Linux tray icons go through libappindicator, which `dlopen`s one of these at first use and
/// panics (an abort in release) when none loads. Same names, same order as libappindicator-sys
/// 0.9 with its `backcompat` feature (on through tray-icon's defaults): versioned, then bare `.so`.
#[cfg(any(target_os = "linux", test))]
const INDICATOR_LIBS: [&str; 4] = [
  "libayatana-appindicator3.so.1",
  "libappindicator3.so.1",
  "libayatana-appindicator3.so",
  "libappindicator3.so",
];

/// Whether a tray icon can be built here. On Linux without an appindicator library we skip the
/// tray (logged once) instead of letting the app die at startup.
fn indicator_available() -> bool {
  #[cfg(target_os = "linux")]
  {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
      let found = first_loadable(&INDICATOR_LIBS, dlopen_probe).is_some();
      if !found {
        tracing::warn!(
          "No appindicator library ({}); running without a tray icon",
          INDICATOR_LIBS.join(" or ")
        );
      }
      found
    })
  }
  #[cfg(not(target_os = "linux"))]
  true
}

#[cfg(target_os = "linux")]
fn dlopen_probe(name: &str) -> bool {
  let Ok(c) = std::ffi::CString::new(name) else {
    return false;
  };
  // SAFETY: c is a valid NUL-terminated string. The handle stays open on purpose: the crate opens
  // the same library right after, so this costs nothing and runs no unload code.
  !unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_LAZY) }.is_null()
}

#[cfg(any(target_os = "linux", test))]
fn first_loadable<'a>(names: &[&'a str], probe: impl Fn(&str) -> bool) -> Option<&'a str> {
  names.iter().copied().find(|n| probe(n))
}

pub fn destroy_tray(app: &AppHandle) {
  let tray_state = app.state::<TrayState>();
  let mut guard = tray_state.tray.lock().unwrap();

  let tray_id = guard.take();
  if let Some(tray_id) = tray_id {
    let _ = app.remove_tray_by_id::<TrayIconId>(&tray_id);
  }
}

#[tauri::command]
pub fn capture_tray_progress(
  app: AppHandle,
  window: tauri::WebviewWindow,
  count: usize,
  titles: Vec<String>,
) -> Result<(), String> {
  if window.label() != "main" {
    return Err("unavailable".into());
  }
  let state = app.state::<TrayState>();
  *state.counts.lock().unwrap() = (count, titles.clone());
  if let Some(item) = state.active.lock().unwrap().as_ref() {
    let _ = item.set_text(active_text(&app, count, &titles));
  }
  Ok(())
}

fn active_text(app: &AppHandle, count: usize, titles: &[String]) -> String {
  let label = app.state::<I18nManager>().t_with(
    "tray.activeDownloads",
    Some(&std::collections::HashMap::from([(
      "n".into(),
      count.to_string(),
    )])),
  );
  let details = titles
    .iter()
    .take(5)
    .map(|s| {
      s.chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect::<String>()
    })
    .collect::<Vec<_>>()
    .join(", ");
  if details.is_empty() {
    label
  } else {
    format!("{label}: {details}")
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn any_library_the_backend_can_load_counts_as_a_tray() {
    for (present, found) in [
      (
        &["libayatana-appindicator3.so.1"][..],
        Some("libayatana-appindicator3.so.1"),
      ),
      (
        &["libappindicator3.so.1", "libayatana-appindicator3.so"][..],
        Some("libappindicator3.so.1"),
      ),
      // Only an unversioned .so (some bundles ship just that): the backend loads it, so do we.
      (
        &["libayatana-appindicator3.so"][..],
        Some("libayatana-appindicator3.so"),
      ),
      (&["libappindicator3.so"][..], Some("libappindicator3.so")),
      (&[][..], None),
    ] {
      assert_eq!(
        first_loadable(&INDICATOR_LIBS, |n| present.contains(&n)),
        found
      );
    }
  }
}
