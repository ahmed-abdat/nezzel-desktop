use crate::commands::{register_shortcuts_for, unregister_shortcuts};
use crate::i18n::I18nManager;
use crate::state::config_models::Config;
use crate::state::json_handle::JsonStoreHandle;
use crate::state::json_state::JsonBackedState;
use crate::tray::{create_tray_for, destroy_tray};
use crate::{DownloadLimiter, FetchLimiter};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime, Wry};
use tauri_plugin_autostart::ManagerExt;

impl JsonBackedState for Config {
  const STORE_FILE: &'static str = "config.store.json";
  const ROOT_KEY: &'static str = "config";

  fn default_value() -> Self {
    Config::default()
  }

  fn before_initialized(app: &AppHandle<Wry>, value: &mut Self) {
    value.input.migrate_capture_shortcut();
    crate::runners::login::migrate_global_consent(&mut value.auth);
    if value.network.enable_proxy.is_none() {
      value.network.enable_proxy =
        Some(value.network.proxy.as_ref().is_some_and(|v| !v.is_empty()));
    }
    if value.output.download_dir.is_none() {
      let system_downloads = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| std::env::current_dir().expect("couldn’t get current dir"));
      let download_path = crate::runners::ytdlp_args::default_download_dir(&system_downloads);
      value.output.download_dir = Some(download_path.to_string_lossy().into_owned());
    }
  }

  fn on_updated(app: &AppHandle<Wry>, new_value: &Self) {
    notify(app, new_value);
    if let Some(limiter) = app.try_state::<DownloadLimiter>() {
      let limiter = limiter.0.clone();
      let max = new_value.performance.max_concurrency;
      tauri::async_runtime::spawn(async move {
        limiter.resize(max).await;
      });
    }
    if let Some(limiter) = app.try_state::<FetchLimiter>() {
      let limiter = limiter.0.clone();
      let max = new_value.performance.max_concurrency;
      tauri::async_runtime::spawn(async move {
        limiter.resize(max).await;
      });
    }

    // Tray, menus and shortcuts are AppKit objects: macOS aborts the process when they are
    // touched off the main thread, and config writes also come from tokio workers.
    // A queued closure may run after a newer write was applied inline on the main thread, so it
    // applies the latest stored config rather than its own snapshot.
    let ui_app = app.clone();
    let _ = app.run_on_main_thread(move || {
      let app = &ui_app;
      let latest = app.state::<crate::SharedConfig>().load();
      let new_value: &Config = &latest;
      if new_value.input.global_shortcuts {
        register_shortcuts_for(app, &new_value.input);
      } else {
        unregister_shortcuts(app);
      }

      if new_value.system.tray_enabled {
        create_tray_for(app, new_value);
      } else {
        destroy_tray(app);
      }

      if new_value.appearance.language == "system" {
        let i18n_handle = app.state::<I18nManager>();
        i18n_handle.unset_locale();
        if new_value.system.tray_enabled {
          destroy_tray(app);
          create_tray_for(app, new_value);
        }
      } else {
        let i18n_handle = app.state::<I18nManager>();
        i18n_handle.set_locale(&new_value.appearance.language);
        if new_value.system.tray_enabled {
          destroy_tray(app);
          create_tray_for(app, new_value);
        }
      }
    });

    if new_value.system.auto_start_enabled {
      let _ = app.autolaunch().enable();
    } else {
      let _ = app.autolaunch().disable();
    }
  }
}

pub type ConfigHandle = JsonStoreHandle<Config>;

/// Told about every new config synchronously, before the snapshot swap: before any reader can see it.
pub type ConfigListener<R> = fn(&AppHandle<R>, &Config);

struct ConfigListeners<R: Runtime>(Mutex<Vec<ConfigListener<R>>>);

/// A module that must act before other readers see a change (usage's on/off flag) registers here at setup, so
/// config does not need to know who listens.
pub fn listen<R: Runtime>(app: &AppHandle<R>, listener: ConfigListener<R>) {
  // A no-op when already managed.
  app.manage(ConfigListeners::<R>(Mutex::new(Vec::new())));
  if let Ok(mut listeners) = app.state::<ConfigListeners<R>>().0.lock() {
    listeners.push(listener);
  }
}

fn notify<R: Runtime>(app: &AppHandle<R>, config: &Config) {
  // Copied out first: a listener may write the config again without deadlocking here.
  let listeners: Vec<ConfigListener<R>> = app
    .try_state::<ConfigListeners<R>>()
    .and_then(|listeners| listeners.0.lock().ok().map(|list| list.clone()))
    .unwrap_or_default();
  for listener in listeners {
    listener(app, config);
  }
}

#[cfg(test)]
pub(crate) type TestConfigHandle = JsonStoreHandle<Config, tauri::test::MockRuntime>;

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicUsize, Ordering};

  #[test]
  fn listeners_hear_every_change_in_order() {
    static SEEN: AtomicUsize = AtomicUsize::new(0);
    let app = tauri::test::mock_app();
    let handle = app.handle();
    notify(handle, &Config::default());
    listen(handle, |_, config| {
      assert!(config.privacy.send_usage);
      assert_eq!(SEEN.fetch_add(1, Ordering::SeqCst) % 2, 0);
    });
    listen(handle, |_, _| {
      assert_eq!(SEEN.fetch_add(1, Ordering::SeqCst) % 2, 1);
    });
    notify(handle, &Config::default());
    notify(handle, &Config::default());
    assert_eq!(SEEN.load(Ordering::SeqCst), 4);
  }
}

#[cfg(test)]
mod lenient_tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn lenient_resets_only_the_bad_nested_field() {
    let raw = json!({ "performance": { "maxConcurrency": 997, "autoLoadSize": "maybe" } });
    assert!(Config::materialize(&raw).is_err());
    let cfg = Config::materialize_lenient(&raw);
    assert_eq!(cfg.performance.max_concurrency, 997);
    assert_eq!(
      cfg.performance.auto_load_size,
      Config::default().performance.auto_load_size
    );
  }

  #[test]
  fn lenient_resets_an_unknown_enum_string() {
    let raw = json!({ "system": { "closeBehavior": "vanish", "trayEnabled": false } });
    let cfg = Config::materialize_lenient(&raw);
    assert!(!cfg.system.tray_enabled);
  }

  #[test]
  fn lenient_keeps_siblings_of_a_bad_leaf_in_output_video() {
    let raw =
      json!({ "output": { "video": { "container": "not-a-container", "postprocessArgs": "-x" } } });
    assert!(Config::materialize(&raw).is_err());
    let cfg = Config::materialize_lenient(&raw);
    assert_eq!(cfg.output.video.postprocess_args, "-x");
    assert_eq!(
      serde_json::to_value(cfg.output.video.container).unwrap(),
      serde_json::to_value(Config::default().output.video.container).unwrap()
    );
  }

  #[test]
  fn lenient_keeps_valid_list_entries_when_one_is_bad() {
    let raw = json!({ "notifications": { "disabledNotifications": ["queueAdded", "bogus", "videoReady"] } });
    assert!(Config::materialize(&raw).is_err());
    let cfg = Config::materialize_lenient(&raw);
    let kept = serde_json::to_value(&cfg.notifications).unwrap();
    assert_eq!(
      kept["disabledNotifications"],
      json!(["queueAdded", "videoReady"])
    );
  }
}
