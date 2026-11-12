//! gallery-dl process setup: configuration file, arguments, bundled binary.
use crate::extract::deadline::FETCH_TIMEOUT;
use crate::parsers::gallery::GalleryError;
use crate::runners::{
  browser_cookies::{CookieAccess, CookieContext},
  gallery_files::Staging,
  gallery_process::find_bundled,
  override_resolver::resolve_with_patch,
};
use crate::scheduling::group_state::subscribe_group;
use crate::sites::{site_of, SiteId};
use crate::state::config_models::NetworkSettings;
use crate::stronghold::stronghold_state::{AuthSecrets, StrongholdState};
use crate::SharedConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub(super) fn configuration(
  secrets: &AuthSecrets,
  network: &NetworkSettings,
  source: &str,
) -> Result<Value, GalleryError> {
  // Extractors reuse their requests session for CDNs and redirects. Do not let
  // user-defined headers (including nonstandard API keys) follow those requests.
  if secrets.bearer_token.is_some() || !secrets.headers.is_empty() {
    return Err(GalleryError::Refused("imageHeaderAuthUnavailable"));
  }
  if !matches!(network.impersonate.as_str(), "none" | "") {
    return Err(GalleryError::Refused("imageImpersonationUnavailable"));
  }
  let mut config = json!({"extractor": {}});
  match network.enable_proxy {
    Some(true) => {
      let proxy = network
        .proxy
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .ok_or(GalleryError::Refused("unableToConnectToProxy"))?;
      config["extractor"]["proxy"] = json!(proxy);
    }
    Some(false) => config["extractor"]["proxy-env"] = json!(false),
    None => {}
  }
  let site = reqwest::Url::parse(source)
    .ok()
    .and_then(|u| u.host_str().and_then(site_of));
  // gallery-dl's extractor category for the sign-in this site takes.
  let category = match site {
    Some(SiteId::X) => Some("twitter"),
    Some(SiteId::Instagram) => Some("instagram"),
    Some(SiteId::Reddit) => Some("reddit"),
    Some(SiteId::Bluesky) => Some("bluesky"),
    Some(SiteId::Pinterest) => Some("pinterest"),
    Some(SiteId::Facebook) => Some("facebook"),
    _ => None,
  };
  if let Some(category) = category {
    let mut credentials = serde_json::Map::new();
    if let Some(username) = &secrets.username {
      credentials.insert("username".into(), json!(username));
    }
    if let Some(password) = &secrets.password {
      credentials.insert("password".into(), json!(password));
    }
    config["extractor"][category] = Value::Object(credentials);
  }
  Ok(config)
}

pub(in crate::runners) fn arguments(cookies: &[String], config: &Path) -> Vec<String> {
  let mut args = vec![
    "--config-ignore".into(),
    "-c".into(),
    config.to_string_lossy().into_owned(),
    "--no-colors".into(),
    "--http-timeout".into(),
    "20".into(),
    "--retries".into(),
    "0".into(),
  ];
  args.extend_from_slice(cookies);
  args
}

pub(super) struct GalleryRunner {
  pub(super) path: PathBuf,
  pub(super) args: Vec<String>,
  pub(super) _config: Staging,
  pub(super) _cookies: Option<CookieAccess>,
}

/// Whether a verified gallery-dl copy ships with this build (problem reports).
pub(crate) fn available(app: &AppHandle) -> bool {
  bundled_path(app).is_ok()
}

pub(super) fn bundled_path(app: &AppHandle) -> Result<PathBuf, GalleryError> {
  #[cfg(feature = "link-matrix")]
  if let Some(resources) = app.try_state::<crate::link_matrix::GalleryResources>() {
    return find_bundled(&resources.0.join("harness"), &resources.0);
  }
  find_bundled(
    &std::env::current_exe().map_err(|_| GalleryError::Unavailable)?,
    &app
      .path()
      .resource_dir()
      .map_err(|_| GalleryError::Unavailable)?,
  )
}

impl GalleryRunner {
  pub(super) async fn new(
    app: &AppHandle,
    overrides: Option<&crate::models::download::DownloadOverrides>,
    browser: Option<&str>,
    source: &str,
    group: &str,
  ) -> Result<Self, GalleryError> {
    let path = bundled_path(app)?;
    let config = app.state::<SharedConfig>().load();
    let patch = overrides.and_then(|o| o.auth.as_ref());
    let auth = resolve_with_patch(&config.auth, patch);
    let secrets = if browser == Some(crate::runners::login::ANONYMOUS_BROWSER) {
      AuthSecrets::default()
    } else {
      let secrets = app
        .try_state::<StrongholdState>()
        .and_then(|s| s.load_auth_secrets().ok())
        .unwrap_or_default();
      resolve_with_patch(&secrets, patch)
    };
    let network = resolve_with_patch(&config.network, overrides.and_then(|o| o.network.as_ref()));
    let context = CookieContext::for_app(app).map_err(|_| GalleryError::Invalid)?;
    let cookies = CookieAccess::prepare_for_app(
      app,
      &auth,
      browser,
      source,
      &context,
      subscribe_group(group),
    )
    .await
    .map_err(|error| match error {
      "cancelled" => GalleryError::Cancelled,
      FETCH_TIMEOUT => GalleryError::Refused(FETCH_TIMEOUT),
      _ => super::super::browser_cookies::read_error_code(error)
        .map_or(GalleryError::Invalid, GalleryError::Refused),
    })?;
    let mut runner = Self::configured(path, &secrets, &network, source)?;
    // Developer runs must neither inherit the owner's gallery cache nor write outside their run.
    #[cfg(feature = "link-matrix")]
    runner.args.extend([
      "--cache-file".into(),
      app
        .state::<crate::paths::PathsManager>()
        .app_dir()
        .join("gallery-cache.sqlite3")
        .to_string_lossy()
        .into_owned(),
    ]);
    runner.args.extend(cookies.args.iter().cloned());
    runner._cookies = Some(cookies);
    Ok(runner)
  }
  pub(super) fn anonymous(
    path: PathBuf,
    network: &NetworkSettings,
    source: &str,
  ) -> Result<Self, GalleryError> {
    // Background previews must never inherit saved cookies, browser choices or credentials.
    Self::configured(path, &AuthSecrets::default(), network, source)
  }
  pub(super) fn configured(
    path: PathBuf,
    secrets: &AuthSecrets,
    network: &NetworkSettings,
    source: &str,
  ) -> Result<Self, GalleryError> {
    let settings = configuration(secrets, network, source)?;
    let stage = Staging::new()?;
    let config = stage.json("gallery.json", &settings)?;
    Ok(Self {
      path,
      args: arguments(&[], &config),
      _config: stage,
      _cookies: None,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn credentials_never_appear_in_sidecar_arguments() {
    let secrets = AuthSecrets {
      username: Some("private-user".into()),
      password: Some("private-password".into()),
      ..Default::default()
    };
    let network = NetworkSettings {
      enable_proxy: Some(true),
      proxy: Some("http://proxy-user:proxy-password@proxy.invalid:8080".into()),
      ..Default::default()
    };
    let runner = GalleryRunner::configured(
      PathBuf::from("gallery-dl"),
      &secrets,
      &network,
      "https://x.com/user/status/1",
    )
    .unwrap();
    assert!(!runner.args.iter().any(|arg| [
      "private-user",
      "private-password",
      "proxy-user",
      "proxy-password"
    ]
    .iter()
    .any(|secret| arg.contains(secret))));
    let config =
      PathBuf::from(&runner.args[runner.args.iter().position(|a| a == "-c").unwrap() + 1]);
    let value: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["extractor"]["twitter"]["username"], "private-user");
    assert_eq!(
      value["extractor"]["twitter"]["password"],
      "private-password"
    );
    assert!(value["extractor"].get("username").is_none());
    assert!(value["extractor"].get("headers").is_none());
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
      );
    }
    drop(runner);
    assert!(!config.exists());
    let unknown = configuration(
      &secrets,
      &NetworkSettings::default(),
      "https://cdn.example/photo.jpg",
    )
    .unwrap();
    assert!(!unknown.to_string().contains("private-password"));
  }

  #[test]
  fn custom_authorization_headers_are_rejected_before_gallery_can_forward_them() {
    for secrets in [
      AuthSecrets {
        bearer_token: Some("private-token".into()),
        ..Default::default()
      },
      AuthSecrets {
        headers: vec!["X-API-Key: private-key".into()],
        ..Default::default()
      },
    ] {
      assert_eq!(
        configuration(
          &secrets,
          &NetworkSettings::default(),
          "https://x.com/user/status/1"
        )
        .unwrap_err(),
        GalleryError::Refused("imageHeaderAuthUnavailable")
      );
    }
  }

  // Cookie consent (which source a run uses) lives in `browsers::CookieSource::select`.
  #[test]
  fn proxy_overrides_are_explicit_and_anonymous_runs_carry_no_cookies() {
    let secrets = AuthSecrets::default();
    let mut network = NetworkSettings {
      enable_proxy: Some(false),
      ..Default::default()
    };
    assert_eq!(
      configuration(&secrets, &network, "https://x.com/user/status/1").unwrap()["extractor"]
        ["proxy-env"],
      false
    );
    network.enable_proxy = Some(true);
    network.proxy = Some("http://proxy.invalid:8080".into());
    assert_eq!(
      configuration(&secrets, &network, "https://x.com/user/status/1").unwrap()["extractor"]
        ["proxy"],
      "http://proxy.invalid:8080"
    );
    let anonymous = GalleryRunner::anonymous(
      PathBuf::from("gallery-dl"),
      &network,
      "https://x.com/user/status/1",
    )
    .unwrap();
    assert!(anonymous.args.iter().any(|arg| arg == "--config-ignore"));
    assert!(!anonymous
      .args
      .iter()
      .any(|arg| arg.starts_with("--cookies")));
    let config = &anonymous.args[anonymous.args.iter().position(|arg| arg == "-c").unwrap() + 1];
    let settings: Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    assert_eq!(
      settings,
      json!({"extractor": {"proxy": "http://proxy.invalid:8080", "twitter": {}}})
    );
    network.proxy = Some("  ".into());
    assert_eq!(
      GalleryRunner::anonymous(
        PathBuf::from("must-not-run"),
        &network,
        "https://x.com/user/status/1",
      )
      .err(),
      Some(GalleryError::Refused("unableToConnectToProxy"))
    );
  }
}
