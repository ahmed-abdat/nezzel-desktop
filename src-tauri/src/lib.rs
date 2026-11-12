mod background;
mod binaries;
mod capture;
mod commands;
#[cfg(test)]
mod contract;
mod diagnostics;
mod download_records;
mod events;
mod extract;
pub mod files;
mod i18n;
mod library;
mod logging;
mod media_types;
mod menu;
mod models;
mod net;
mod parsers;
mod paths;
mod platform;
mod resolve;
mod routing;
mod runners;
mod scheduling;
mod sites;
mod state;
mod stronghold;
mod tray;
mod usage;
mod window;

#[cfg(feature = "link-matrix")]
#[path = "../examples/link_matrix/runner.rs"]
pub mod link_matrix;

use crate::binaries::binaries_manager::BinariesManager;
use crate::binaries::binaries_state::BinariesState;
use crate::commands::*;
use crate::download_records::DownloadRecords;
use crate::i18n::I18nManager;
use crate::logging::LogStoreState;
use crate::menu::setup_menu;
use crate::paths::PathsManager;
use crate::scheduling::concurrency::DynamicSemaphore;
use crate::scheduling::download_pipeline::{setup_download_dispatcher, DownloadSender};
use crate::scheduling::fetch_pipeline::{setup_fetch_dispatcher, FetchSender};
use crate::state::config::ConfigHandle;
use crate::state::preferences::PreferencesHandle;
use crate::tray::{create_tray, TrayState};
use crate::window::{restore_main_window, setup_close_behaviour, track_main_window};
use sentry::ClientInitGuard;
use std::sync::{Arc, Mutex as StdMutex};
use stronghold::stronghold_state;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::Mutex;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::{fmt, prelude::*};

type SharedConfig = Arc<ConfigHandle>;
type SharedPreferences = Arc<PreferencesHandle>;

#[derive(Clone)]
pub struct DownloadLimiter(pub Arc<DynamicSemaphore>);

#[derive(Clone)]
pub struct FetchLimiter(pub Arc<DynamicSemaphore>);

/// # Panics
///
/// Will panic if an error occurs during tauri setup.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let app = tauri::Builder::default()
    .plugin(tauri_plugin_notification::init())
    .plugin(tauri_plugin_updater::Builder::new().build())
    .plugin(tauri_plugin_store::Builder::new().build())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_clipboard_manager::init())
    .plugin(tauri_plugin_keyring::init())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_global_shortcut::Builder::new().build())
    .plugin(tauri_plugin_autostart::init(
      MacosLauncher::LaunchAgent,
      Some(vec!["--auto-start"]),
    ))
    .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
      // `nezzel --capture` (a system shortcut on Wayland, where global key grabs don't work) opens Quick download.
      if wants_capture(&args) {
        capture::open_quick_bar(app, true);
      } else {
        reopen_window(app)
      }
    }))
    .setup(|app| {
      let handle = app.handle();

      // Our own project's DSN comes from NEZZEL_SENTRY_DSN at build time only. Empty or malformed
      // means no client at all (a runtime SENTRY_DSN is never read). Only diagnostics::send
      // builds events (scrubbed); the Settings switch is checked there.
      if let Some(sentry_client) = diagnostics::send::init(option_env!("NEZZEL_SENTRY_DSN")) {
        handle.manage::<ClientInitGuard>(sentry_client);
      }

      init_tracing();

      // setup runtime mode detection / path management
      let path_handle = PathsManager::new(handle);
      handle.manage(path_handle.clone());
      diagnostics::setup(handle, &path_handle);
      handle.manage(diagnostics::PendingCrash::default());
      runners::browser_cookies::sweep_cookie_exports(handle);

      // setup config management
      let config_handle = ConfigHandle::init(handle)?;
      let shared_config = Arc::new(config_handle);
      handle.manage::<SharedConfig>(shared_config);
      usage::setup(handle, path_handle.app_dir());

      let preferences_handle = PreferencesHandle::init(handle)?;
      let shared_preferences = Arc::new(preferences_handle);
      handle.manage::<SharedPreferences>(shared_preferences);

      // Register command state before any webview can invoke commands or capture can start work.
      // setup i18n management
      handle.manage(I18nManager::new(handle));

      // setup window management
      handle.manage(background::BackgroundState::default());

      // setup tray
      handle.manage(TrayState {
        tray: StdMutex::new(None),
        active: StdMutex::new(None),
        counts: StdMutex::new((0, Vec::new())),
      });
      // manage update store
      handle.manage(Mutex::new(UpdateStore::default()));

      // manage log store
      handle.manage(LogStoreState::new());
      handle.manage(DownloadRecords::default());
      handle.manage(library::Library::default());

      runners::ytdlp_download::sweep_download_partials(handle);

      // setup dispatchers
      let cfg_snapshot = handle.state::<SharedConfig>().load();
      let max_concurrency = cfg_snapshot.performance.max_concurrency;
      let download_limiter = Arc::new(DynamicSemaphore::new(max_concurrency));
      let fetch_limiter = Arc::new(DynamicSemaphore::new(max_concurrency));
      handle.manage(DownloadLimiter(download_limiter.clone()));
      handle.manage(FetchLimiter(fetch_limiter.clone()));

      let fetch_dispatcher = setup_fetch_dispatcher(handle, fetch_limiter);
      handle.manage(FetchSender(fetch_dispatcher.sender()));
      let download_dispatcher = setup_download_dispatcher(handle, download_limiter);
      handle.manage(DownloadSender(download_dispatcher.sender()));

      // setup binaries
      handle.manage(BinariesState::from_metadata(
        &path_handle.bin_dir().join("metadata.json"),
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .map_or(0, |duration| duration.as_secs()),
      ));
      handle.manage(BinariesManager::new(handle));
      // set up stronghold
      let app_path = path_handle.app_dir();
      let stronghold_path = app_path.join("vault.hold");

      handle.manage(stronghold_state::StrongholdState::new(stronghold_path));
      handle.manage(capture::CaptureState::default());

      create_main_window(handle, &path_handle)?;

      // Window hooks and capture windows need the main window to exist.
      background::track_dock(handle);
      restore_main_window(handle);
      track_main_window(handle);
      setup_close_behaviour(handle);

      capture::setup(handle)?;
      if wants_capture(&std::env::args().collect::<Vec<_>>()) {
        capture::open_quick_bar(handle, true);
      }

      create_tray(handle);

      // setup autostart
      init_autostart(handle);

      BinariesManager::schedule_ytdlp_update(handle);

      // async init stronghold
      #[cfg(not(all(target_os = "macos", debug_assertions)))]
      {
        let state_ref = handle.state::<stronghold_state::StrongholdState>();
        stronghold_state::init_on_startup(handle, &state_ref);
      }

      // configure app menu
      setup_menu(handle);

      // register shortcuts
      register_shortcuts(handle);

      Ok(())
    })
    .invoke_handler({
      let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        app_ready,
        commands::shortcuts_pause,
        capture::capture_ready,
        capture::capture_read_clipboard,
        capture::capture_platform,
        capture::capture_snapshot,
        capture::capture_edit,
        capture::capture_retry,
        capture::capture_action,
        capture::capture_fit,
        capture::capture_pull,
        capture::capture_write_clipboard,
        commands::shortcuts::capture_shortcut_status,
        tray::capture_tray_progress,
        capture::capture_try_link,
        background::close_note_ready,
        background::close_note_done,
        background::window_motion_pref,
        media_size,
        media_info,
        media_playlist_expand,
        media_download,
        media_trash_file,
        library::library_list,
        library::library_record_failure,
        library::library_remove,
        crate::runners::login::registry::media_login_accept,
        crate::runners::login::registry::media_login_choices,
        crate::runners::login::registry::media_login_dismiss,
        crate::runners::login::notice::media_login_notice,
        crate::runners::transport::engine_availability,
        group_cancel,
        logging_subscribe,
        logging_unsubscribe,
        config_get,
        config_reset,
        config_set,
        preferences_get,
        preferences_reset,
        preferences_set,
        binaries_check,
        binaries_update_status,
        binaries_update_now,
        binaries_ensure,
        updater_check,
        updater_download,
        updater_install,
        stronghold_init,
        stronghold_status,
        stronghold_keys,
        stronghold_get,
        stronghold_set,
        get_platform,
        notify,
        diagnostics::diagnostics_report,
        diagnostics::diagnostics_send,
        diagnostics::diagnostics_download_failed,
        diagnostics::diagnostics_crash_check,
        diagnostics::diagnostics_send_crash,
        diagnostics::diagnostics_log,
        usage::usage_count,
      ];
      move |invoke: tauri::ipc::Invoke| {
        if invoke
          .message
          .webview_ref()
          .label()
          .starts_with("site-login-")
        {
          return false;
        }
        handler(invoke)
      }
    })
    .build(tauri::generate_context!())
    .expect("error while running tauri application");

  app.run(|_app_handle, _event| {
    // The clipboard plugin drops its handle while exiting; a read after that panics inside the plugin.
    // Only on the final Exit: ExitRequested can be declined while the app lives on in the tray.
    if matches!(_event, tauri::RunEvent::Exit) {
      capture::clipboard::stop_reads();
    }
    if matches!(_event, tauri::RunEvent::Exit) {
      runners::ytdlp_process::terminate_all_live(std::time::Duration::from_millis(1500));
      diagnostics::on_clean_exit(_app_handle);
      usage::on_exit(_app_handle);
    }
    #[cfg(target_os = "macos")]
    {
      if let tauri::RunEvent::Reopen { .. } = _event {
        if _app_handle
          .get_webview_window("main")
          .is_some_and(|w| !w.is_visible().unwrap_or(true))
        {
          reopen_window(_app_handle);
        }
      }
    }
  });
}

fn create_main_window(app: &AppHandle, paths: &PathsManager) -> tauri::Result<()> {
  let window_config = app
    .config()
    .app
    .windows
    .first()
    .expect("missing main window config");

  #[cfg(not(windows))]
  let _ = paths;

  let builder = tauri::WebviewWindowBuilder::from_config(app, window_config)?;

  #[cfg(windows)]
  let builder = if paths.is_portable_app() {
    builder.data_directory(paths.app_dir().join("webview"))
  } else {
    builder
  };

  builder.build()?;

  Ok(())
}

pub fn init_tracing() {
  let fmt_layer = fmt::layer().with_filter(
    Targets::new()
      .with_target("tauri_plugin_updater", LevelFilter::OFF)
      .with_target(
        "tao::platform_impl::platform::event_loop::runner",
        LevelFilter::OFF,
      )
      .with_target("h2", LevelFilter::OFF)
      .with_target("hyper_util", LevelFilter::OFF)
      .with_default(tracing_levels()),
  );

  // Scrubbed copy for the on-disk log and reports. No Sentry layer: tracing events are not
  // scrubbed enough to send; diagnostics::send builds every outgoing event.
  let report_layer = fmt::layer()
    .with_ansi(false)
    .with_writer(diagnostics::file_log::ReportLog)
    .with_filter(
      Targets::new()
        .with_target("tauri_plugin_updater", LevelFilter::OFF)
        .with_target(
          "tao::platform_impl::platform::event_loop::runner",
          LevelFilter::OFF,
        )
        .with_target("h2", LevelFilter::OFF)
        .with_target("hyper_util", LevelFilter::OFF)
        .with_default(LevelFilter::INFO),
    );

  tracing_subscriber::registry()
    .with(fmt_layer)
    .with(report_layer)
    .init();
}

pub fn init_autostart(app: &AppHandle) {
  let cfg_handle = app.state::<SharedConfig>();
  let cfg = cfg_handle.load();
  let autostart_manager = app.autolaunch();
  let is_enabled = autostart_manager.is_enabled().unwrap_or(false);

  if cfg.system.auto_start_enabled && !is_enabled {
    let _ = autostart_manager.enable();
  } else if !cfg.system.auto_start_enabled && is_enabled {
    let _ = autostart_manager.disable();
  }
}

/// `--capture` anywhere after the program name.
fn wants_capture(args: &[String]) -> bool {
  args.iter().skip(1).any(|arg| arg == "--capture")
}

pub fn reopen_window(app: &AppHandle) {
  background::show_main(app);
}

#[cfg(debug_assertions)]
const fn tracing_levels() -> LevelFilter {
  LevelFilter::DEBUG
}

#[cfg(not(debug_assertions))]
fn tracing_levels() -> LevelFilter {
  LevelFilter::INFO
}
