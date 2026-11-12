//! One prepared cookie source for a yt-dlp or gallery-dl run: nothing, the user's cookie file,
//! or a private jar scoped to the link's site, exported from the enabled browser source.
use super::{
  exporter, firefox,
  jar::{private_directory, private_file, scoped_rows, CookieJar, COOKIE_HEADER},
};
use crate::runners::browsers::{CookieSource, CookieStore};
use std::path::PathBuf;

/// Browser export material never enters logs, config, or IPC.
#[derive(Clone)]
pub(in crate::runners) struct CookieAccess {
  pub args: Vec<String>,
  pub has_session: bool,
  consent_app: Option<tauri::AppHandle>,
  usage: Option<crate::runners::login::notice::LoginUsage>,
  _jar: Option<std::sync::Arc<CookieJar>>,
}

impl std::fmt::Debug for CookieAccess {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("CookieAccess")
      .finish_non_exhaustive()
  }
}

pub(in crate::runners) struct CookieContext {
  pub exporter: PathBuf,
  pub directory: PathBuf,
}
impl CookieContext {
  pub fn for_app(app: &tauri::AppHandle) -> Result<Self, &'static str> {
    use tauri::Manager;
    Ok(Self {
      exporter: crate::binaries::binaries_manager::BinariesManager::for_app(app)
        .tool_path("yt-dlp")
        .map_err(|_| COOKIE_READ_ERROR)?,
      directory: app
        .path()
        .app_cache_dir()
        .map_err(|_| COOKIE_READ_ERROR)?
        .join("browser-cookies"),
    })
  }
}
pub(in crate::runners) const COOKIE_READ_ERROR: &str = "browserCookiesUnreadable";

impl CookieAccess {
  pub async fn prepare(
    settings: &crate::state::config_models::AuthSettings,
    browser: Option<&str>,
    source: &str,
    context: &CookieContext,
    mut cancel: tokio::sync::watch::Receiver<bool>,
  ) -> Result<Self, &'static str> {
    use crate::extract::deadline::{remaining_budget, FETCH_TIMEOUT};
    use std::{io::Write, time::Duration};
    let host = reqwest::Url::parse(source)
      .ok()
      .and_then(|url| url.host_str().map(str::to_owned));
    let browser =
      match CookieSource::for_site(settings, browser, host.as_deref().unwrap_or_default())? {
        CookieSource::Anonymous => {
          return Ok(Self {
            args: vec![],
            has_session: false,
            consent_app: None,
            usage: None,
            _jar: None,
          })
        }
        CookieSource::File(file) => {
          let (jar, path, mut scoped) = new_jar(context)?;
          scoped
            .write_all(COOKIE_HEADER.as_bytes())
            .map_err(|_| COOKIE_READ_ERROR)?;
          let rows = scoped_rows(
            std::path::Path::new(&file),
            host.as_deref().ok_or(COOKIE_READ_ERROR)?,
            &mut scoped,
          )
          .map_err(|_| COOKIE_READ_ERROR)?;
          return Ok(Self {
            has_session: rows > 0,
            args: vec!["--cookies".into(), path.to_string_lossy().into_owned()],
            consent_app: None,
            usage: None,
            _jar: Some(std::sync::Arc::new(jar)),
          });
        }
        CookieSource::Browser(browser) => browser,
      };
    if !*cancel.borrow() {
      return Err("cancelled");
    }
    let budget = remaining_budget(Duration::from_secs(60));
    if budget.is_zero() {
      return Err(FETCH_TIMEOUT);
    }
    let host = host.ok_or(COOKIE_READ_ERROR)?;
    std::fs::create_dir_all(context.directory.parent().ok_or(COOKIE_READ_ERROR)?)
      .map_err(|_| COOKIE_READ_ERROR)?;
    private_directory(&context.directory).map_err(|_| COOKIE_READ_ERROR)?;
    let directory = context
      .directory
      .join(format!("attempt-{}", uuid::Uuid::new_v4()));
    private_directory(&directory).map_err(|_| COOKIE_READ_ERROR)?;
    let jar = CookieJar { directory };
    let path = jar.directory.join("cookies.txt");
    let mut scoped = private_file(&path).map_err(|_| COOKIE_READ_ERROR)?;
    scoped
      .write_all(COOKIE_HEADER.as_bytes())
      .map_err(|_| COOKIE_READ_ERROR)?;
    let rows = if browser.kind.store() == CookieStore::Firefox {
      firefox::export_firefox(&browser, &host, &jar, &mut scoped).map_err(|error| {
        if error
          .downcast_ref::<std::io::Error>()
          .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
          "browserProfileMissing"
        } else {
          let message = error.to_string();
          read_error_code(&message)
            .unwrap_or_else(|| exporter::classify_error(browser.kind, &message))
        }
      })?
    } else {
      exporter::export(
        context,
        &browser,
        &host,
        &jar,
        &mut scoped,
        budget,
        &mut cancel,
      )
      .await?
    };
    scoped.flush().map_err(|_| COOKIE_READ_ERROR)?;
    Ok(Self {
      args: vec!["--cookies".into(), path.to_string_lossy().into_owned()],
      has_session: rows > 0,
      consent_app: None,
      usage: None,
      _jar: Some(std::sync::Arc::new(jar)),
    })
  }
}

/// Recognize only our fixed vocabulary, never forward arbitrary runner error text.
pub(in crate::runners) fn read_error_code(error: &str) -> Option<&'static str> {
  match error {
    "browserLoginDisabled" => Some("browserLoginDisabled"),
    "browserLoginTimeout" => Some("browserLoginTimeout"),
    "browserLoginRemedy" => Some("browserLoginRemedy"),
    "browserKeychainDenied" => Some("browserKeychainDenied"),
    "browserDpapiFailed" => Some("browserDpapiFailed"),
    "browserAppBound" => Some("browserAppBound"),
    "browserSafariAccessDenied" => Some("browserSafariAccessDenied"),
    "browserDatabaseLocked" => Some("browserDatabaseLocked"),
    "browserProfileMissing" => Some("browserProfileMissing"),
    COOKIE_READ_ERROR => Some(COOKIE_READ_ERROR),
    _ => None,
  }
}

impl CookieAccess {
  /// Called only after a downloader starts with this jar, never during source discovery/export.
  pub(crate) fn announce_used(&self) {
    if let (Some(app), Some(usage)) = (&self.consent_app, &self.usage) {
      crate::runners::login::notice::used(app, usage);
    }
  }
  pub(crate) fn check_live_consent(&self) -> Result<(), &'static str> {
    if self
      .consent_app
      .as_ref()
      .is_some_and(|app| !crate::runners::login::automatic::enabled(app))
    {
      return Err("browserLoginDisabled");
    }
    Ok(())
  }
  pub(crate) fn has_proof(&self, host: &str) -> bool {
    let Some(path) = self
      .args
      .windows(2)
      .find(|args| args[0] == "--cookies")
      .map(|args| &args[1])
    else {
      return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
      return false;
    };
    let rows: Vec<Vec<&str>> = text
      .lines()
      .map(|line| line.strip_prefix("#HttpOnly_").unwrap_or(line))
      .filter(|line| !line.starts_with('#'))
      .map(|line| line.split('\t').collect())
      .filter(|fields: &Vec<_>| fields.len() == 7)
      .collect();
    super::proof_cookies(
      host,
      rows
        .iter()
        .filter_map(|f| Some((f[0], f[5], f[6], f[4].parse().ok()?))),
      unix_now(),
    )
  }
  pub(crate) fn from_webview(
    context: &CookieContext,
    host: &str,
    cookies: &[tauri::webview::Cookie<'static>],
    ua: &str,
  ) -> Result<Self, &'static str> {
    use std::io::Write;
    let (jar, path, mut file) = new_jar(context)?;
    file
      .write_all(COOKIE_HEADER.as_bytes())
      .map_err(|_| COOKIE_READ_ERROR)?;
    for cookie in cookies {
      let domain = cookie.domain_original().unwrap_or("");
      if !super::jar::allowed_cookie_host(domain, host) {
        continue;
      }
      super::jar::NetscapeRow {
        domain,
        path: cookie.path().unwrap_or("/"),
        secure: cookie.secure() == Some(true),
        http_only: cookie.http_only() == Some(true),
        expiry: cookie.expires_datetime().map_or(0, |t| t.unix_timestamp()),
        name: cookie.name(),
        value: cookie.value(),
      }
      .write(&mut file)
      .map_err(|_| COOKIE_READ_ERROR)?;
    }
    file.flush().map_err(|_| COOKIE_READ_ERROR)?;
    Ok(Self {
      args: vec![
        "--cookies".into(),
        path.to_string_lossy().into_owned(),
        "--user-agent".into(),
        ua.into(),
      ],
      has_session: true,
      consent_app: None,
      usage: None,
      _jar: Some(std::sync::Arc::new(jar)),
    })
  }
  pub(crate) async fn prepare_for_app(
    app: &tauri::AppHandle,
    settings: &crate::state::config_models::AuthSettings,
    browser: Option<&str>,
    source: &str,
    context: &CookieContext,
    mut cancel: tokio::sync::watch::Receiver<bool>,
  ) -> Result<Self, &'static str> {
    let wants_browser = browser != Some(crate::runners::login::ANONYMOUS_BROWSER)
      && (browser.is_some()
        || settings.cookie_file.as_deref().is_none_or(str::is_empty)
          && matches!(
            crate::runners::browsers::BrowserSetting::of(settings),
            crate::runners::browsers::BrowserSetting::Fixed(_)
          ));
    if wants_browser && !crate::runners::login::automatic::enabled(app) {
      return Err("browserLoginDisabled");
    }
    let mut result = if browser == Some(crate::runners::login::NEZZEL_SESSION) {
      let host = reqwest::Url::parse(source)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .ok_or(COOKIE_READ_ERROR)?;
      if !*cancel.borrow() {
        return Err("cancelled");
      }
      tokio::select! {
        biased;
        _ = cancel.changed() => return Err("cancelled"),
        result = crate::runners::login::webview_session::read(app, &host, false) => result?.ok_or("browserLoginRemedy")?,
      }
    } else {
      Self::prepare(settings, browser, source, context, cancel).await?
    };
    if wants_browser && !crate::runners::login::automatic::enabled(app) {
      return Err("browserLoginDisabled");
    }
    if wants_browser && crate::runners::browsers::automatic(settings) {
      let host = reqwest::Url::parse(source)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .ok_or(COOKIE_READ_ERROR)?;
      if !result.has_proof(&host) {
        return Err("browserLoginRemedy");
      }
    }
    if wants_browser {
      result.consent_app = Some(app.clone());
      if let Some(host) = reqwest::Url::parse(source)
        .ok()
        .and_then(|url| url.host_str().and_then(super::site_key))
      {
        if result.has_proof(&host) {
          let kind = if browser == Some(crate::runners::login::NEZZEL_SESSION) {
            Some("nezzel")
          } else {
            match CookieSource::for_site(settings, browser, &host) {
              Ok(CookieSource::Browser(source)) => Some(source.kind.name()),
              _ => None,
            }
          };
          if let Some(kind) = kind {
            result.usage = Some(crate::runners::login::notice::LoginUsage {
              browser: kind,
              host,
            });
          }
        }
      }
    }
    Ok(result)
  }
}
fn unix_now() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_secs() as i64)
}
fn new_jar(context: &CookieContext) -> Result<(CookieJar, PathBuf, std::fs::File), &'static str> {
  std::fs::create_dir_all(context.directory.parent().ok_or(COOKIE_READ_ERROR)?)
    .map_err(|_| COOKIE_READ_ERROR)?;
  private_directory(&context.directory).map_err(|_| COOKIE_READ_ERROR)?;
  let jar = CookieJar {
    directory: context
      .directory
      .join(format!("attempt-{}", uuid::Uuid::new_v4())),
  };
  private_directory(&jar.directory).map_err(|_| COOKIE_READ_ERROR)?;
  let path = jar.directory.join("cookies.txt");
  let file = private_file(&path).map_err(|_| COOKIE_READ_ERROR)?;
  Ok((jar, path, file))
}
