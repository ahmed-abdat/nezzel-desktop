//! Sign-in hints from a browser cookie database: host, name, expiry and access time only.
//! Cookie values are never read here.
use super::{
  chromium, cookie_domains, firefox, is_session_cookie, known_login_host, same_site,
  session_present,
};
use crate::runners::{
  browser_logins::{BrowserEnv, Os},
  browsers::{BrowserKind, CookieStore},
  gallery_files::Staging,
};
use rusqlite::{Connection, OpenFlags};
use std::{collections::HashSet, path::Path};

#[derive(Default)]
pub(in crate::runners) struct CookieHint {
  pub signed_in: bool,
  pub last_access: i64,
  pub app_bound: bool,
}

/// The table, columns and clocks one cookie store uses; `chromium` and `firefox` own theirs.
pub(super) struct HintSchema {
  pub table: &'static str,
  pub host_column: &'static str,
  pub expiry: &'static str,
  pub access: &'static str,
  /// SQL for "this row is app-bound", given the OS and the table's columns.
  pub app_bound: fn(Os, &HashSet<String>) -> &'static str,
  /// Whether a row's stored expiry is past `now` (Unix seconds).
  pub expired: fn(i64, i64) -> bool,
  /// A row's stored access time as microseconds since the Unix epoch.
  pub access_micros: fn(i64) -> i64,
}

pub(in crate::runners) fn cookie_hint(
  env: &BrowserEnv,
  path: &Path,
  browser: BrowserKind,
  host: &str,
  now: i64,
) -> CookieHint {
  let os = env.os;
  let copy_file = env.copy_file;
  // Prefer a private snapshot, including WAL, without writing to the live profile.
  let copied: Result<CookieHint, Box<dyn std::error::Error>> = (|| {
    let staging = Staging::new().map_err(|_| "private staging unavailable")?;
    let copy = staging.path.join("cookies.sqlite");
    copy_file(path, &copy)?;
    for suffix in ["-wal", "-shm"] {
      let source = path.with_file_name(format!(
        "{}{suffix}",
        path.file_name().unwrap_or_default().to_string_lossy()
      ));
      if source.is_file() {
        copy_file(
          &source,
          &staging.path.join(format!("cookies.sqlite{suffix}")),
        )?;
      }
    }
    let connection = Connection::open_with_flags(
      &copy,
      OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(read_hint(&connection, browser, os, host, now)?)
  })();
  copied
    .or_else(|_| {
      // Windows can deny a file copy while allowing SQLite's read-only, lock-free URI.
      // immutable=1 never creates journals or shared-memory files beside the browser DB.
      let mut uri =
        reqwest::Url::from_file_path(path).map_err(|_| "invalid cookie database path")?;
      uri
        .query_pairs_mut()
        .append_pair("mode", "ro")
        .append_pair("immutable", "1");
      let connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
          | OpenFlags::SQLITE_OPEN_NO_MUTEX
          | OpenFlags::SQLITE_OPEN_URI,
      )?;
      Ok::<_, Box<dyn std::error::Error>>(read_hint(&connection, browser, os, host, now)?)
    })
    // An unreadable jar is not evidence of a signed-in, app-bound session.
    .unwrap_or_default()
}

fn read_hint(
  connection: &Connection,
  browser: BrowserKind,
  os: Os,
  host: &str,
  now: i64,
) -> rusqlite::Result<CookieHint> {
  let domains = cookie_domains(host);
  if domains.is_empty() {
    return Ok(CookieHint::default());
  }
  let schema = if browser.store() == CookieStore::Firefox {
    &firefox::HINT_SCHEMA
  } else {
    &chromium::HINT_SCHEMA
  };
  let HintSchema {
    table,
    host_column,
    expiry,
    ..
  } = schema;
  let columns: HashSet<String> = connection
    .prepare(&format!("PRAGMA table_info({table})"))?
    .query_map([], |row| row.get(1))?
    .collect::<Result<_, _>>()?;
  let access = if columns.contains(schema.access) {
    schema.access
  } else {
    "0"
  };
  let v20 = (schema.app_bound)(os, &columns);
  let sql = format!("SELECT {host_column}, name, {expiry}, {access}, {v20} FROM {table} WHERE {host_column} = ?1 OR {host_column} = '.' || ?1 OR {host_column} LIKE '%.' || ?1");
  let mut statement = connection.prepare(&sql)?;
  let mut names = HashSet::new();
  let mut hint = CookieHint::default();
  for domain in domains {
    let rows = statement.query_map([&domain], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, i64>(2)?,
        row.get::<_, i64>(3)?,
        row.get::<_, bool>(4)?,
      ))
    })?;
    for row in rows {
      let (cookie_host, name, expires, access, app_bound) = row?;
      if !same_site(&cookie_host, &domain) {
        continue;
      }
      if (schema.expired)(expires, now) {
        continue;
      }
      // Listed sites: only a session cookie makes the jar an unreadable login (visitor cookies don't).
      // Unlisted sites have no session names, so any v20 row still marks the jar unreadable.
      hint.app_bound |= app_bound && (!known_login_host(host) || is_session_cookie(host, &name));
      hint.last_access = hint.last_access.max((schema.access_micros)(access));
      names.insert(name);
    }
  }
  hint.signed_in = session_present(host, &names);
  Ok(hint)
}
