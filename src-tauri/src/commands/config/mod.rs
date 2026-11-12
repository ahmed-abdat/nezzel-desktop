pub mod config_get;
pub mod config_reset;
pub mod config_set;

pub use config_get::*;
pub use config_reset::*;
pub use config_set::*;

/// Profile paths and the notice lifetime flag are backend-owned; client snapshots cannot replace them.
fn public_config(
  mut config: crate::state::config_models::Config,
) -> crate::state::config_models::Config {
  config.auth.site_browsers.clear();
  config
}
fn client_patch(mut patch: serde_json::Value) -> serde_json::Value {
  if let Some(auth) = patch
    .get_mut("auth")
    .and_then(serde_json::Value::as_object_mut)
  {
    auth.remove("siteBrowsers");
    auth.remove("browserLoginNoticeShown");
  }
  patch
}

fn reset_user_config(config: &mut crate::state::config_models::Config) {
  let notice_shown = config.auth.browser_login_notice_shown;
  // A user who switched browser logins off keeps them off across a Settings reset.
  let opted_out = crate::runners::login::consent::global_consent(&config.auth) == Some(false);
  *config = crate::state::config_models::Config::default();
  config.auth.browser_login_notice_shown = notice_shown;
  if opted_out {
    config.auth.browser_login_consent = Some(false);
    config.auth.always_use_browser_logins = false;
    config.auth.cookie_browser = crate::runners::browsers::NEVER.to_string();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::state::{config_models::Config, json_merge};
  #[test]
  fn ipc_settings_never_expose_or_replace_backend_profile_paths() {
    let mut config = Config::default();
    config.auth.browser_login_notice_shown = true;
    config.auth.site_browsers.insert(
      "instagram.com".into(),
      "chrome:/Users/private-user/Profile 1".into(),
    );
    let public = serde_json::to_value(public_config(config.clone())).unwrap();
    assert!(!public.to_string().contains("private-user"));
    assert!(public["auth"]["siteBrowsers"]
      .as_object()
      .unwrap()
      .is_empty());
    assert_eq!(
      config.auth.site_browsers["instagram.com"],
      "chrome:/Users/private-user/Profile 1"
    );
    let mut stored = serde_json::to_value(config).unwrap();
    json_merge(
      &mut stored,
      &client_patch(
        serde_json::json!({"auth":{"cookieBrowser":"none","siteBrowsers":null,"browserLoginNoticeShown":false}}),
      ),
    );
    assert_eq!(
      stored["auth"]["siteBrowsers"]["instagram.com"],
      "chrome:/Users/private-user/Profile 1"
    );
    assert_eq!(stored["auth"]["cookieBrowser"], "none");
    assert_eq!(
      stored["auth"]["browserLoginNoticeShown"], true,
      "a stale Settings snapshot cannot re-arm a viewed notice"
    );
    let mut reset: Config = serde_json::from_value(stored).unwrap();
    reset_user_config(&mut reset);
    assert!(
      reset.auth.browser_login_notice_shown,
      "reset cannot re-arm a viewed notice"
    );
    assert_eq!(
      reset.auth.browser_login_consent,
      Some(false),
      "reset keeps an explicit browser-login opt-out"
    );
    assert!(!reset.auth.always_use_browser_logins);
    assert!(reset.auth.site_browsers.is_empty());
    let mut fresh = Config::default();
    reset_user_config(&mut fresh);
    assert_eq!(fresh.auth.browser_login_consent, Some(true));
  }
}
