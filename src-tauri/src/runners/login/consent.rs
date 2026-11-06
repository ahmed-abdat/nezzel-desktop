//! Which browser a host or a row may use: remembered site choices, this run's explicit
//! picks, and the row handoff from fetch to download.
use crate::runners::browser_cookies::{known_login_host, migrate_site_browsers, site_key};
use crate::runners::browsers::{BrowserKind, BrowserSource};
use crate::state::config_models::AuthSettings;
use std::{
  collections::{HashMap, HashSet},
  sync::{LazyLock, Mutex},
};

// Legacy unknown-host and Safari entries may have come from old automatic selection.
// A current picker choice authorizes reusing those entries for fetch -> download.
pub(super) static EXPLICIT_SITES: LazyLock<Mutex<HashSet<String>>> =
  LazyLock::new(|| Mutex::new(HashSet::new()));
pub(super) fn may_reuse_remembered(auth: &AuthSettings, host: &str) -> bool {
  (known_login_host(host)
    && auth
      .site_browsers
      .get(host)
      .and_then(|browser| BrowserSource::parse(browser))
      .map(|source| source.kind)
      != Some(BrowserKind::Safari))
    || auth.always_use_browser_logins
    || EXPLICIT_SITES.lock().unwrap().contains(host)
}

struct RowBrowser {
  browser: String,
  host: String,
}
static ROW_BROWSERS: LazyLock<Mutex<HashMap<(String, String), RowBrowser>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) fn remember_row_browser(group: &str, id: &str, host: &str, browser: &str) {
  let mut rows = ROW_BROWSERS.lock().unwrap();
  rows.insert(
    (group.into(), id.into()),
    RowBrowser {
      browser: browser.into(),
      host: site_key(host).unwrap_or_else(|| host.into()),
    },
  );
}
pub(crate) fn row_browser(group: &str, id: &str, url: &str) -> Option<String> {
  let url = reqwest::Url::parse(url).ok()?;
  let host = site_key(url.host_str()?).unwrap_or_else(|| url.host_str().unwrap_or_default().into());
  ROW_BROWSERS
    .lock()
    .unwrap()
    .get(&(group.into(), id.into()))
    .filter(|choice| choice.host == host)
    .map(|choice| choice.browser.clone())
}

pub(super) fn forget_group_rows(group_id: &str) {
  ROW_BROWSERS
    .lock()
    .unwrap()
    .retain(|(group, _), _| group != group_id);
}

/// Download termination releases row consent; fetch Cleanup must preserve the handoff.
pub(crate) fn forget_row_browser(group: &str, id: &str) {
  ROW_BROWSERS
    .lock()
    .unwrap()
    .remove(&(group.into(), id.into()));
}

/// Serialises read-compare-write updates of `auth.siteBrowsers`.
pub(super) static CONSENT_WRITES: Mutex<()> = Mutex::new(());
#[cfg(test)]
pub(super) fn record_host_browser(auth: &mut AuthSettings, host: &str, browser: Option<String>) {
  let host = site_key(host).unwrap_or_else(|| host.into());
  match browser {
    Some(browser) => {
      auth.site_browsers.insert(host.clone(), browser);
    }
    None => {
      auth.site_browsers.remove(&host);
    }
  }
}

pub(super) fn remember_success(
  auth: &mut AuthSettings,
  host: &str,
  previous: Option<&str>,
  browser: &str,
) {
  // A fallback may replace the starting browser, never another row's newer choice.
  migrate_site_browsers(&mut auth.site_browsers);
  let host = site_key(host).unwrap_or_else(|| host.into());
  if auth.site_browsers.get(&host).map(String::as_str) == previous {
    auth.site_browsers.insert(host, browser.into());
  }
}
