//! Once per ping: OS and CPU family (never the OS version or computer name), settings, app language, install age
//! and engine freshness. Read at send time.

use super::{keys, store::DAY_MS, Usage};
use crate::diagnostics::report::{settings_summary, Token};
use crate::state::config_models::Config;
use tauri::{AppHandle, Manager};

pub struct Facts<'a> {
  pub os: &'a str,
  pub arch: &'a str,
  pub locale: &'a str,
  pub installed_at: Option<u64>,
  pub engine_ok_at: Option<u64>,
  pub now: u64,
}

/// `set.<key>.<value>` from the safe settings summary reports use too (one mapping: what a setting may say).
fn setting(summary: &[(&'static str, Token)], key: &str, name: &str) -> Option<String> {
  summary
    .iter()
    .find(|(setting, _)| *setting == name)
    .map(|(_, value)| format!("set.{key}.{value}"))
}

pub fn snapshot(config: &Config, facts: &Facts) -> Vec<String> {
  let days = |at: u64| facts.now.saturating_sub(at) / DAY_MS;
  let summary = settings_summary(config);
  let set = |key: &str, name: &str| setting(&summary, key, name);
  let language = config.appearance.language.as_str();
  let set_lang = if language == "system" || language.is_empty() {
    Some("set.lang.auto".to_string())
  } else {
    keys::locale(language).map(|code| format!("set.lang.{code}"))
  };
  let arch = match facts.arch {
    "aarch64" => Some("arm64"),
    "x86_64" => Some("x64"),
    _ => None,
  };
  let os = match facts.os {
    "macos" | "windows" | "linux" => Some(facts.os),
    _ => None,
  };
  [
    Some(keys::ACTIVE.to_string()),
    os.map(|os| format!("os.{os}")),
    arch.map(|arch| format!("arch.{arch}")),
    set_lang,
    keys::locale(facts.locale).map(|code| format!("lang.{code}")),
    set("theme", "theme"),
    Some(keys::parallel(config.performance.max_concurrency)),
    set("tray", "tray"),
    set("clipboard", "watchClipboard"),
    set("autostart", "autostart"),
    set("reports", "sendReports"),
    facts.installed_at.map(|at| keys::age(days(at))),
    Some(keys::engine_fresh(facts.engine_ok_at.map(days))),
  ]
  .into_iter()
  .flatten()
  .filter(|key| keys::allowed(key))
  .collect()
}

pub fn keys(app: &AppHandle) -> Vec<String> {
  let config = app.state::<crate::SharedConfig>().load();
  let state = app.state::<Usage>().snapshot_state();
  let locale = app
    .try_state::<crate::i18n::I18nManager>()
    .map(|i18n| i18n.locale())
    .unwrap_or_default();
  snapshot(
    &config,
    &Facts {
      os: std::env::consts::OS,
      arch: std::env::consts::ARCH,
      locale: &locale,
      installed_at: state.installed_at,
      engine_ok_at: state.engine_ok_at,
      now: super::now_ms(),
    },
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn snapshot_names_settings_and_classes_only() {
    let mut config = Config::default();
    config.appearance.language = "pt-BR".into();
    config.performance.max_concurrency = 6;
    let now = 100 * DAY_MS;
    let keys = snapshot(
      &config,
      &Facts {
        os: "macos",
        arch: "aarch64",
        locale: "pt-BR",
        installed_at: Some(now - 3 * DAY_MS),
        engine_ok_at: None,
        now,
      },
    );
    assert_eq!(
      keys,
      vec![
        "active",
        "os.macos",
        "arch.arm64",
        "set.lang.pt-br",
        "lang.pt-br",
        "set.theme.system",
        "set.parallel.4up",
        format!(
          "set.tray.{}",
          if config.system.tray_enabled {
            "on"
          } else {
            "off"
          }
        )
        .as_str(),
        format!(
          "set.clipboard.{}",
          if config.input.watch_clipboard {
            "on"
          } else {
            "off"
          }
        )
        .as_str(),
        format!(
          "set.autostart.{}",
          if config.system.auto_start_enabled {
            "on"
          } else {
            "off"
          }
        )
        .as_str(),
        "set.reports.on",
        "age.d1-7",
        "engine.fresh.bundled",
      ]
    );
  }

  #[test]
  fn unknown_values_are_left_out_not_sent() {
    let mut config = Config::default();
    config.appearance.theme = "my-theme".into();
    config.appearance.language = "system".into();
    let keys = snapshot(
      &config,
      &Facts {
        os: "freebsd",
        arch: "riscv64",
        locale: "xx",
        installed_at: None,
        engine_ok_at: Some(0),
        now: 10 * DAY_MS,
      },
    );
    assert!(keys.contains(&"set.lang.auto".to_string()));
    assert!(!keys
      .iter()
      .any(|k| k.starts_with("os.") || k.starts_with("arch.") || k.starts_with("lang.")));
    assert!(!keys.iter().any(|k| k.starts_with("set.theme.")));
    assert!(keys.contains(&"engine.fresh.d8-30".to_string()));
  }
}
