pub mod config_get;
pub mod config_reset;
pub mod config_set;

pub use config_get::*;
pub use config_reset::*;
pub use config_set::*;

/// Site profile paths are backend-owned. The webview only sees opaque login candidates.
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
  }
  patch
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::state::{config_models::Config, json_merge};
  #[test]
  fn ipc_settings_never_expose_or_replace_backend_profile_paths() {
    let mut config = Config::default();
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
      &client_patch(serde_json::json!({"auth":{"cookieBrowser":"none","siteBrowsers":null}})),
    );
    assert_eq!(
      stored["auth"]["siteBrowsers"]["instagram.com"],
      "chrome:/Users/private-user/Profile 1"
    );
    assert_eq!(stored["auth"]["cookieBrowser"], "none");
  }
}
