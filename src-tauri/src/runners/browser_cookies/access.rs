//! One prepared cookie source for a yt-dlp or gallery-dl run: nothing, the user's cookie file,
//! or a private jar scoped to the link's site, exported from the consented browser.
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
    let browser = match CookieSource::select(settings, browser)? {
      CookieSource::Anonymous => {
        return Ok(Self {
          args: vec![],
          has_session: false,
          _jar: None,
        })
      }
      CookieSource::File(file) => {
        return Ok(Self {
          has_session: scoped_rows(
            std::path::Path::new(&file),
            &reqwest::Url::parse(source)
              .ok()
              .and_then(|url| url.host_str().map(str::to_owned))
              .ok_or(COOKIE_READ_ERROR)?,
            &mut std::io::sink(),
          )
          .map_err(|_| COOKIE_READ_ERROR)?
            > 0,
          args: vec!["--cookies".into(), file],
          _jar: None,
        })
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
    let host = reqwest::Url::parse(source)
      .ok()
      .and_then(|url| url.host_str().map(str::to_owned))
      .ok_or(COOKIE_READ_ERROR)?;
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
      _jar: Some(std::sync::Arc::new(jar)),
    })
  }
}

/// Recognize only our fixed vocabulary, never forward arbitrary runner error text.
pub(in crate::runners) fn read_error_code(error: &str) -> Option<&'static str> {
  match error {
    "browserKeychainDenied" => Some("browserKeychainDenied"),
    "browserSafariAccessDenied" => Some("browserSafariAccessDenied"),
    "browserDatabaseLocked" => Some("browserDatabaseLocked"),
    "browserProfileMissing" => Some("browserProfileMissing"),
    COOKIE_READ_ERROR => Some(COOKIE_READ_ERROR),
    _ => None,
  }
}
