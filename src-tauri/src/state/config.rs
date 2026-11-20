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

// Only config inputs consumed by the OS integrations; never persisted.
struct AppliedConfig {
  global_shortcuts: bool,
  capture_shortcut: String,
  language: String,
  tray_enabled: bool,
  auto_start_enabled: bool,
}

impl From<&Config> for AppliedConfig {
  fn from(config: &Config) -> Self {
    Self {
      global_shortcuts: config.input.global_shortcuts,
      capture_shortcut: config.input.capture_shortcut.clone(),
      language: config.appearance.language.clone(),
      tray_enabled: config.system.tray_enabled,
      auto_start_enabled: config.system.auto_start_enabled,
    }
  }
}

#[derive(Default)]
struct LastAppliedConfig(Mutex<Option<AppliedConfig>>);

#[derive(Debug, Default, PartialEq, Eq)]
enum TrayAction {
  #[default]
  None,
  Create,
  Destroy,
  Rebuild,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ConfigActions {
  shortcuts: bool,
  locale: bool,
  tray: TrayAction,
  autolaunch: bool,
}

fn config_actions(applied: Option<&AppliedConfig>, config: &Config) -> ConfigActions {
  let locale = applied.is_none_or(|old| old.language != config.appearance.language);
  // create_tray_for reads only tray_enabled; its menu text uses the active i18n locale.
  let tray = if applied.is_none_or(|old| old.tray_enabled != config.system.tray_enabled) {
    if config.system.tray_enabled {
      TrayAction::Create
    } else {
      TrayAction::Destroy
    }
  } else if locale && config.system.tray_enabled {
    TrayAction::Rebuild
  } else {
    TrayAction::None
  };
  ConfigActions {
    shortcuts: applied.is_none_or(|old| {
      old.global_shortcuts != config.input.global_shortcuts
        || old.capture_shortcut != config.input.capture_shortcut
    }),
    locale,
    tray,
    autolaunch: applied
      .is_none_or(|old| old.auto_start_enabled != config.system.auto_start_enabled),
  }
}

/// Called after setup has initialized locale, tray, autostart and shortcuts once.
pub fn remember_startup_config(app: &AppHandle) {
  let config = app.state::<crate::SharedConfig>().load();
  app.manage(LastAppliedConfig(Mutex::new(Some(AppliedConfig::from(
    config.as_ref(),
  )))));
}

impl JsonBackedState for Config {
  const STORE_FILE: &'static str = "config.store.json";
  const ROOT_KEY: &'static str = "config";

  fn default_value() -> Self {
    Config::default()
  }

  fn before_initialized(_app: &AppHandle<Wry>, value: &mut Self) {
    value.input.migrate_capture_shortcut();
    crate::runners::login::migrate_global_consent(&mut value.auth);
    if value.network.enable_proxy.is_none() {
      value.network.enable_proxy =
        Some(value.network.proxy.as_ref().is_some_and(|v| !v.is_empty()));
    }
    if value.output.download_dir.is_none() {
      let download_path = crate::runners::ytdlp_args::default_download_dir();
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
  }

  fn on_stored(app: &AppHandle<Wry>, _new_value: &Self) {
    // Tray, menus and shortcuts are AppKit objects: macOS aborts the process when they are
    // touched off the main thread, and config writes also come from tokio workers.
    // A queued closure may run after a newer write was applied inline on the main thread, so it
    // applies the latest stored config rather than its own snapshot.
    let ui_app = app.clone();
    let _ = app.run_on_main_thread(move || {
      let app = &ui_app;
      let latest = app.state::<crate::SharedConfig>().load();
      let new_value: &Config = &latest;
      app.manage(LastAppliedConfig::default());
      let state = app.state::<LastAppliedConfig>();
      let Ok(mut applied) = state.0.lock() else {
        return;
      };
      let actions = config_actions(applied.as_ref(), new_value);
      if actions.shortcuts {
        if new_value.input.global_shortcuts {
          register_shortcuts_for(app, &new_value.input);
        } else {
          unregister_shortcuts(app);
        }
      }

      if actions.locale {
        let i18n_handle = app.state::<I18nManager>();
        if new_value.appearance.language == "system" {
          i18n_handle.unset_locale();
        } else {
          i18n_handle.set_locale(&new_value.appearance.language);
        }
      }
      match actions.tray {
        TrayAction::None => {}
        TrayAction::Create => create_tray_for(app, new_value),
        TrayAction::Destroy => destroy_tray(app),
        TrayAction::Rebuild => {
          destroy_tray(app);
          create_tray_for(app, new_value);
        }
      }

      if actions.autolaunch {
        if new_value.system.auto_start_enabled {
          let _ = app.autolaunch().enable();
        } else {
          let _ = app.autolaunch().disable();
        }
      }
      *applied = Some(AppliedConfig::from(new_value));
    });
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
  use serde_json::json;
  use std::sync::atomic::{AtomicUsize, Ordering};

  #[test]
  fn system_actions_depend_only_on_changed_inputs() {
    // Expected tuple: shortcuts, locale, tray operation, autolaunch.
    let cases = [
      (
        "first application",
        None,
        json!({}),
        (true, true, TrayAction::Create, true),
      ),
      (
        "first application with integrations off",
        None,
        json!({"input":{"globalShortcuts":false},"system":{"trayEnabled":false}}),
        (true, true, TrayAction::Destroy, true),
      ),
      (
        "unchanged",
        Some(json!({})),
        json!({}),
        (false, false, TrayAction::None, false),
      ),
      (
        "language only",
        Some(json!({})),
        json!({"appearance":{"language":"ar"}}),
        (false, true, TrayAction::Rebuild, false),
      ),
      (
        "return to system language",
        Some(json!({"appearance":{"language":"ar"}})),
        json!({"appearance":{"language":"system"}}),
        (false, true, TrayAction::Rebuild, false),
      ),
      (
        "unrelated performance change",
        Some(json!({})),
        json!({"performance":{"maxConcurrency":997}}),
        (false, false, TrayAction::None, false),
      ),
      (
        "unrelated input change",
        Some(json!({})),
        json!({"input":{"watchClipboard":false}}),
        (false, false, TrayAction::None, false),
      ),
      (
        "language while tray off",
        Some(json!({"system":{"trayEnabled":false}})),
        json!({"appearance":{"language":"ar"}}),
        (false, true, TrayAction::None, false),
      ),
      (
        "enable tray with new language creates once",
        Some(json!({"system":{"trayEnabled":false}})),
        json!({"appearance":{"language":"ar"},"system":{"trayEnabled":true}}),
        (false, true, TrayAction::Create, false),
      ),
      (
        "disable tray with new language never recreates",
        Some(json!({})),
        json!({"appearance":{"language":"ar"},"system":{"trayEnabled":false}}),
        (false, true, TrayAction::Destroy, false),
      ),
      (
        "disable shortcuts",
        Some(json!({})),
        json!({"input":{"globalShortcuts":false}}),
        (true, false, TrayAction::None, false),
      ),
      (
        "change shortcut binding",
        Some(json!({})),
        json!({"input":{"captureShortcut":"CommandOrControl+Alt+K"}}),
        (true, false, TrayAction::None, false),
      ),
      (
        "enable autostart",
        Some(json!({})),
        json!({"system":{"autoStartEnabled":true}}),
        (false, false, TrayAction::None, true),
      ),
      (
        "disable autostart",
        Some(json!({"system":{"autoStartEnabled":true}})),
        json!({"system":{"autoStartEnabled":false}}),
        (false, false, TrayAction::None, true),
      ),
    ];
    for (name, old, patch, (shortcuts, locale, tray, autolaunch)) in cases {
      let applied = old
        .as_ref()
        .map(|raw| AppliedConfig::from(&Config::materialize(raw).unwrap()));
      let mut raw = old.unwrap_or_else(|| json!({}));
      crate::state::json_state::json_merge(&mut raw, &patch);
      let config = Config::materialize(&raw).unwrap();
      assert_eq!(
        config_actions(applied.as_ref(), &config),
        ConfigActions {
          shortcuts,
          locale,
          tray,
          autolaunch,
        },
        "{name}"
      );
      // A later queued closure sees the same latest config and must not repeat any work.
      let applied = AppliedConfig::from(&config);
      assert_eq!(
        config_actions(Some(&applied), &config),
        ConfigActions::default(),
        "{name}: already applied"
      );
    }
  }

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
