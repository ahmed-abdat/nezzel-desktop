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
    None => None,
    Some("none") => Some(0),
    Some(name) => {
      let containers: serde_json::Value =
        serde_json::from_slice(&std::fs::read(database.with_file_name("containers.json"))?)?;
      Some(
        containers["identities"]
          .as_array()
          .and_then(|identities| {
            identities
              .iter()
              .find(|identity| identity["name"].as_str() == Some(name))
          })
          .and_then(|identity| identity["userContextId"].as_u64())
          .ok_or("unknown container")?,
      )
    }
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
    if let Some(container) = container {
      let actual = attributes
        .trim_start_matches('^')
        .split('&')
        .find_map(|field| field.strip_prefix("userContextId="))
        .unwrap_or("0");
      if actual.parse::<u64>().ok() != Some(container) {
        continue;
      }
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
