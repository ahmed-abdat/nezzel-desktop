//! Firefox's cookie database: its layout for sign-in hints, and a site-scoped export of its
//! cookies into the Netscape jar (Firefox values are plain, so no exporter process is needed).
use super::{
  hint::HintSchema,
  jar::{allowed_cookie_host, CookieJar, NetscapeRow},
};
use crate::runners::{
  browser_logins::Os,
  browsers::{BrowserKind, BrowserSource},
};
use rusqlite::{Connection, OpenFlags};
use std::{
  collections::HashSet,
  path::{Path, PathBuf},
};

pub(super) const HINT_SCHEMA: HintSchema = HintSchema {
  table: "moz_cookies",
  host_column: "host",
  expiry: "expiry",
  access: "lastAccessed",
  app_bound,
  expired,
  access_micros,
};

fn app_bound(_: Os, _: &HashSet<String>) -> &'static str {
  "0"
}

fn expired(expires: i64, now: i64) -> bool {
  seconds(expires) <= now
}

/// `lastAccessed` is already microseconds since the Unix epoch.
fn access_micros(access: i64) -> i64 {
  access
}

/// Firefox stores expiry in seconds, newer profiles in milliseconds.
fn seconds(expiry: i64) -> i64 {
  if expiry > 100_000_000_000 {
    expiry / 1000
  } else {
    expiry
  }
}

/// Only public, unambiguous names that survive yt-dlp's argument grammar.
/// Built-in containers use l10nID rather than name (yt-dlp accepts the full l10nID).
pub(in crate::runners) fn containers(profile: &Path) -> Vec<(String, u64)> {
  let data: serde_json::Value = std::fs::read(profile.join("containers.json"))
    .ok()
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default();
  let entries: Vec<_> = data["identities"]
    .as_array()
    .into_iter()
    .flatten()
    .filter(|identity| identity["public"].as_bool() != Some(false))
    .filter_map(|identity| {
      let name = identity["name"].as_str().or_else(|| {
        identity["l10nID"]
          .as_str()
          .filter(|name| name.starts_with("userContext") && name.ends_with(".label"))
      })?;
      let id = identity["userContextId"].as_u64().filter(|id| *id > 0)?;
      if name.is_empty() || name.trim() != name || name == "none" {
        return None;
      }
      let spec = format!("firefox::{}", name);
      (BrowserSource::parse(&spec)?.spec() == spec).then(|| (name.to_owned(), id))
    })
    .collect();
  entries
    .iter()
    .filter(|(name, id)| {
      entries
        .iter()
        .filter(|(other_name, other_id)| other_name == name || other_id == id)
        .count()
        == 1
    })
    .cloned()
    .collect()
}

/// A Netscape cookie cannot represent a private or partitioned browser context.
/// Never promote these rows to an ordinary first-party session.
pub(super) fn cookie_context(attributes: &str, host: &str) -> Option<u64> {
  let mut container = 0;
  for field in attributes.trim_start_matches('^').split('&') {
    if let Some(id) = field.strip_prefix("userContextId=") {
      container = id.parse().ok()?;
    } else if field
      .strip_prefix("firstPartyDomain=")
      .is_some_and(|domain| !domain.is_empty() && !super::same_site(domain, host))
      || field.starts_with("partitionKey=")
      || field
        .strip_prefix("privateBrowsingId=")
        .is_some_and(|id| id != "0")
    {
      return None;
    }
  }
  Some(container)
}

fn firefox_database(
  browser: &BrowserSource,
  host: &str,
) -> Result<(PathBuf, Option<String>), &'static str> {
  let profile = PathBuf::from(browser.profile.as_deref().unwrap_or_default());
  if profile.is_absolute() {
    return Ok((profile.join("cookies.sqlite"), browser.container.clone()));
  }
  let directory = crate::runners::browser_logins::discover(None, host)
    .into_iter()
    .filter(|candidate| candidate.browser == BrowserKind::Firefox)
    .filter_map(|candidate| BrowserSource::parse(&candidate.id)?.profile)
    .find(|path| {
      profile.as_os_str().is_empty() || Path::new(path).file_name() == profile.file_name()
    })
    .ok_or("browserProfileMissing")?;
  Ok((
    Path::new(&directory).join("cookies.sqlite"),
    browser.container.clone(),
  ))
}

pub(super) fn export_firefox(
  browser: &BrowserSource,
  host: &str,
  jar: &CookieJar,
  destination: &mut std::fs::File,
) -> Result<usize, Box<dyn std::error::Error>> {
  let (database, container) = firefox_database(browser, host)?;
  let container = match container.as_deref() {
    // Unscoped legacy selections must not silently merge different accounts.
    None | Some("none") => 0,
    Some(name) => containers(database.parent().ok_or("browserProfileMissing")?)
      .into_iter()
      .find(|(candidate, _)| candidate == name)
      .map(|(_, id)| id)
      .ok_or("browserProfileMissing")?,
  };
  let copy = jar.directory.join("cookies.sqlite");
  std::fs::copy(&database, &copy)?;
  for suffix in ["-wal", "-shm"] {
    let original = database.with_file_name(format!("cookies.sqlite{suffix}"));
    if original.exists() {
      std::fs::copy(
        original,
        jar.directory.join(format!("cookies.sqlite{suffix}")),
      )?;
    }
  }
  let connection = Connection::open_with_flags(
    &copy,
    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
  )?;
  let columns: HashSet<String> = connection
    .prepare("PRAGMA table_info(moz_cookies)")?
    .query_map([], |row| row.get(1))?
    .collect::<Result<_, _>>()?;
  let http_only = if columns.contains("isHttpOnly") {
    "isHttpOnly"
  } else {
    "0"
  };
  let attributes = if columns.contains("originAttributes") {
    "originAttributes"
  } else {
    "''"
  };
  let mut statement = connection.prepare(&format!(
    "SELECT host, name, value, path, expiry, isSecure, {http_only}, {attributes} FROM moz_cookies"
  ))?;
  let rows = statement.query_map([], |row| {
    Ok((
      row.get::<_, String>(0)?,
      row.get::<_, String>(1)?,
      row.get::<_, String>(2)?,
      row.get::<_, String>(3)?,
      row.get::<_, i64>(4)?,
      row.get::<_, bool>(5)?,
      row.get::<_, bool>(6)?,
      row.get::<_, String>(7)?,
    ))
  })?;
  let mut count = 0;
  for row in rows {
    let (domain, name, value, path, expiry, secure, http_only, attributes) = row?;
    if !allowed_cookie_host(&domain, host) {
      continue;
    }
    if cookie_context(&attributes, host) != Some(container) {
      continue;
    }
    NetscapeRow {
      domain: &domain,
      path: &path,
      secure,
      http_only,
      expiry: seconds(expiry),
      name: &name,
      value: &value,
    }
    .write(destination)?;
    count += 1;
  }
  drop(statement);
  drop(connection);
  for name in ["cookies.sqlite", "cookies.sqlite-wal", "cookies.sqlite-shm"] {
    match std::fs::remove_file(jar.directory.join(name)) {
      Ok(()) => {}
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
      Err(error) => return Err(error.into()),
    }
  }
  Ok(count)
}
