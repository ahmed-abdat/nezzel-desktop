//! Site-session metadata and private, site-scoped cookie exports.
use super::{browser_logins::Os, gallery_files::Staging};
use rusqlite::{Connection, OpenFlags};
use std::{
  collections::{HashMap, HashSet},
  path::{Path, PathBuf},
};

const CHROMIUM_EPOCH: i64 = 11_644_473_600;

#[derive(Default)]
pub(super) struct CookieHint {
  pub signed_in: bool,
  pub last_access: i64,
  pub app_bound: bool,
}

pub(super) fn registrable_domain(host: &str) -> Option<String> {
  let host = host.trim_matches('.').to_ascii_lowercase();
  if host.is_empty()
    || !host
      .bytes()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
    || host.parse::<std::net::IpAddr>().is_ok()
  {
    return None;
  }
  psl::domain_str(&host).map(str::to_owned)
}

/// Canonical consent identity; cookie_domains enumerates the permitted cookie aliases.
pub(super) fn site_key(host: &str) -> Option<String> {
  let domain = registrable_domain(host)?;
  Some(match domain.as_str() {
    "redd.it" => "reddit.com".into(),
    "youtu.be" => "youtube.com".into(),
    "threads.com" => "threads.net".into(),
    "twitter.com" => "x.com".into(),
    _ => domain,
  })
}

/// Canonical entries win collisions, then root hosts, www hosts, and other legacy hosts.
/// Existing browser values are moved verbatim; migration never discovers a new profile.
pub(super) fn migrate_site_browsers(browsers: &mut HashMap<String, String>) {
  let mut entries: Vec<_> = std::mem::take(browsers).into_iter().collect();
  entries.sort_by_key(|(host, _)| {
    let canonical = site_key(host).unwrap_or_else(|| host.clone());
    let priority = if host == &canonical {
      0
    } else if registrable_domain(host).as_ref() == Some(host) {
      1
    } else if host.starts_with("www.") {
      2
    } else {
      3
    };
    (priority, host.clone())
  });
  for (host, browser) in entries {
    browsers
      .entry(site_key(&host).unwrap_or(host))
      .or_insert(browser);
  }
}

pub(super) fn same_site(cookie_host: &str, host: &str) -> bool {
  registrable_domain(cookie_host).is_some_and(|domain| Some(domain) == registrable_domain(host))
}

fn cookie_domains(host: &str) -> Vec<String> {
  match site_key(host).as_deref() {
    Some("threads.net") => vec![
      "threads.net".into(),
      "threads.com".into(),
      "instagram.com".into(),
    ],
    Some("x.com") => vec!["x.com".into(), "twitter.com".into()],
    Some("youtube.com") => vec!["youtube.com".into()],
    Some(domain) => vec![domain.into()],
    None => vec![],
  }
}

enum SessionCookies {
  Any(&'static [&'static str]),
  All(&'static [&'static str]),
}
fn session_cookies(host: &str) -> Option<SessionCookies> {
  use SessionCookies::{All, Any};
  match site_key(host).as_deref() {
    Some("instagram.com" | "threads.net" | "tiktok.com") => Some(Any(&["sessionid"])),
    Some("facebook.com") => Some(All(&["c_user", "xs"])),
    Some("x.com") => Some(Any(&["auth_token"])),
    Some("youtube.com") => Some(Any(&["SAPISID", "__Secure-3PSID", "LOGIN_INFO"])),
    Some("reddit.com") => Some(Any(&["reddit_session"])),
    Some("linkedin.com") => Some(Any(&["li_at"])),
    Some("pinterest.com") => Some(Any(&["_pinterest_sess"])),
    Some("vimeo.com") => Some(Any(&["vimeo"])),
    Some("snapchat.com") => Some(Any(&["sc-a-session", "__Host-sc-a-session"])),
    _ => None,
  }
}
pub(super) fn known_login_host(host: &str) -> bool {
  session_cookies(host).is_some()
}
pub(super) fn session_present(host: &str, names: &HashSet<String>) -> bool {
  match session_cookies(host) {
    Some(SessionCookies::Any(required)) => required.iter().any(|name| names.contains(*name)),
    Some(SessionCookies::All(required)) => required.iter().all(|name| names.contains(*name)),
    None => false,
  }
}

pub(super) fn cookie_hint(path: &Path, browser: &str, os: Os, host: &str, now: i64) -> CookieHint {
  cookie_hint_with_copy(path, browser, os, host, now, |from, to| {
    std::fs::copy(from, to)
  })
}

pub(super) fn cookie_hint_with_copy(
  path: &Path,
  browser: &str,
  os: Os,
  host: &str,
  now: i64,
  mut copy_file: impl FnMut(&Path, &Path) -> std::io::Result<u64>,
) -> CookieHint {
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
    .unwrap_or_else(|_| CookieHint {
      // This flag also represents an unreadable Windows Chromium jar, not only v20.
      app_bound: matches!(os, Os::Windows)
        && matches!(
          browser,
          "chrome" | "edge" | "brave" | "chromium" | "vivaldi" | "opera"
        ),
      ..Default::default()
    })
}

fn read_hint(
  connection: &Connection,
  browser: &str,
  os: Os,
  host: &str,
  now: i64,
) -> rusqlite::Result<CookieHint> {
  let domains = cookie_domains(host);
  if domains.is_empty() {
    return Ok(CookieHint::default());
  }
  let firefox = browser == "firefox";
  let (table, host_column, expiry, access) = if firefox {
    ("moz_cookies", "host", "expiry", "lastAccessed")
  } else {
    ("cookies", "host_key", "expires_utc", "last_access_utc")
  };
  let columns: HashSet<String> = connection
    .prepare(&format!("PRAGMA table_info({table})"))?
    .query_map([], |row| row.get(1))?
    .collect::<Result<_, _>>()?;
  let access = if columns.contains(access) {
    access
  } else {
    "0"
  };
  let v20 = if !firefox && matches!(os, Os::Windows) && columns.contains("encrypted_value") {
    "COALESCE(substr(encrypted_value, 1, 3) = x'763230', 0)"
  } else {
    "0"
  };
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
      let expires = if firefox {
        if expires > 100_000_000_000 {
          expires / 1000
        } else {
          expires
        }
      } else if expires == 0 {
        0
      } else {
        expires / 1_000_000 - CHROMIUM_EPOCH
      };
      if (firefox || expires != 0) && expires <= now {
        continue;
      }
      hint.app_bound |= app_bound;
      let access = if firefox {
        access
      } else {
        access.saturating_sub(CHROMIUM_EPOCH * 1_000_000)
      };
      hint.last_access = hint.last_access.max(access);
      names.insert(name);
    }
  }
  hint.signed_in = session_present(host, &names);
  Ok(hint)
}

/// Browser export material never enters logs, config, or IPC.
#[derive(Clone)]
pub(super) struct CookieAccess {
  pub args: Vec<String>,
  _jar: Option<std::sync::Arc<CookieJar>>,
}

impl std::fmt::Debug for CookieAccess {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("CookieAccess")
      .finish_non_exhaustive()
  }
}

struct CookieJar {
  directory: PathBuf,
}
impl Drop for CookieJar {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.directory);
  }
}

pub(super) struct CookieContext {
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
const COOKIE_READ_ERROR: &str = "Browser cookies could not be read";
const COOKIE_HEADER: &str = "# Netscape HTTP Cookie File\n";

fn private_directory(path: &Path) -> std::io::Result<()> {
  let mut builder = std::fs::DirBuilder::new();
  #[cfg(unix)]
  {
    use std::os::unix::fs::DirBuilderExt;
    builder.mode(0o700);
  }
  match builder.create(path) {
    Ok(()) => {}
    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
    Err(error) => return Err(error),
  }
  let metadata = std::fs::symlink_metadata(path)?;
  if !metadata.is_dir() || metadata.file_type().is_symlink() {
    return Err(std::io::ErrorKind::PermissionDenied.into());
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    // A predictable cache name must never turn into another user's cookie sink.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
      return Err(std::io::ErrorKind::PermissionDenied.into());
    }
  }
  Ok(())
}
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
  let mut options = std::fs::OpenOptions::new();
  options.create_new(true).write(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  options.open(path)
}

/// Called before dispatchers start: only our private UUID staging directories are removed.
pub(crate) fn sweep_cookie_exports(app: &tauri::AppHandle) {
  // Release builds use panic=abort: destructors cannot clean up after a crash.
  // Only these startup sweeps reclaim abandoned exports and discovery snapshots.
  if let Ok(context) = CookieContext::for_app(app) {
    let _ = sweep_directory(&context.directory);
  }
  let _ = sweep_named_directories(&std::env::temp_dir(), "nezzel-images-");
}
fn sweep_directory(root: &Path) -> std::io::Result<()> {
  if !root.exists() {
    return Ok(());
  }
  private_directory(root)?;
  sweep_named_directories(root, "attempt-")
}
fn sweep_named_directories(root: &Path, prefix: &str) -> std::io::Result<()> {
  let metadata = std::fs::symlink_metadata(root)?;
  if !metadata.is_dir() || metadata.file_type().is_symlink() {
    return Err(std::io::ErrorKind::PermissionDenied.into());
  }
  for entry in std::fs::read_dir(root)? {
    let entry = entry?;
    let name = entry.file_name();
    let Some(name) = name.to_str().and_then(|name| name.strip_prefix(prefix)) else {
      continue;
    };
    if uuid::Uuid::parse_str(name).is_ok()
      && entry.file_type()?.is_dir()
      && private_directory(&entry.path()).is_ok()
    {
      std::fs::remove_dir_all(entry.path())?;
    }
  }
  Ok(())
}

fn allowed_cookie_host(cookie_host: &str, host: &str) -> bool {
  let cookie_host = cookie_host.trim_start_matches('.').to_ascii_lowercase();
  if !cookie_host
    .bytes()
    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
  {
    return false;
  }
  cookie_domains(host)
    .iter()
    .any(|domain| cookie_host == *domain || cookie_host.ends_with(&format!(".{domain}")))
    || (host.parse::<std::net::IpAddr>().is_ok() && cookie_host == host)
}

fn firefox_database(browser: &str, host: &str) -> Result<(PathBuf, Option<String>), &'static str> {
  let specification = browser.strip_prefix("firefox").ok_or(COOKIE_READ_ERROR)?;
  let specification = specification.strip_prefix(':').unwrap_or(specification);
  let (profile, container) = specification
    .rsplit_once("::")
    .map_or((specification, None), |(profile, container)| {
      (profile, Some(container.to_owned()))
    });
  let profile = PathBuf::from(profile);
  if profile.is_absolute() {
    return Ok((profile.join("cookies.sqlite"), container));
  }
  let candidate = super::browser_logins::discover(None, host)
    .into_iter()
    .find(|candidate| {
      candidate.browser == "firefox"
        && (profile.as_os_str().is_empty()
          || candidate
            .id
            .strip_prefix("firefox:")
            .is_some_and(|path| Path::new(path).file_name() == profile.file_name()))
    })
    .ok_or(COOKIE_READ_ERROR)?;
  Ok((
    Path::new(
      candidate
        .id
        .strip_prefix("firefox:")
        .ok_or(COOKIE_READ_ERROR)?,
    )
    .join("cookies.sqlite"),
    container,
  ))
}

fn export_firefox(
  browser: &str,
  host: &str,
  jar: &CookieJar,
  destination: &mut std::fs::File,
) -> Result<(), Box<dyn std::error::Error>> {
  use std::io::Write;
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
    if [&domain, &name, &value, &path]
      .iter()
      .any(|field| field.contains(['\t', '\r', '\n']))
    {
      return Err("invalid cookie field".into());
    }
    writeln!(
      destination,
      "{}{}\t{}\t{}\t{}\t{}\t{}\t{}",
      if http_only { "#HttpOnly_" } else { "" },
      domain,
      if domain.starts_with('.') {
        "TRUE"
      } else {
        "FALSE"
      },
      path,
      if secure { "TRUE" } else { "FALSE" },
      if expiry > 100_000_000_000 {
        expiry / 1000
      } else {
        expiry
      },
      name,
      value
    )?;
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
  Ok(())
}

/// Untrusted Netscape fields are checked before any row is allowed into the scoped jar.
fn filter_export(full: &Path, host: &str, scoped: &mut std::fs::File) -> std::io::Result<()> {
  use std::io::{BufRead, Write};
  let result = (|| {
    for line in std::io::BufReader::new(std::fs::File::open(full)?).lines() {
      let line = line?;
      let fields = line.strip_prefix("#HttpOnly_").unwrap_or(&line);
      if fields.starts_with('#') || fields.is_empty() {
        continue;
      }
      let parts: Vec<_> = fields.split('\t').collect();
      if parts.len() != 7
        || !matches!(parts[1], "TRUE" | "FALSE")
        || !matches!(parts[3], "TRUE" | "FALSE")
        || parts[4].parse::<i64>().is_err()
      {
        return Err(std::io::ErrorKind::InvalidData.into());
      }
      if allowed_cookie_host(parts[0], host) {
        writeln!(scoped, "{line}")?;
      }
    }
    Ok(())
  })();
  // Never keep the unscoped export, including on a parse/write failure.
  let removed = std::fs::remove_file(full);
  result.and(removed)
}

impl CookieAccess {
  pub async fn prepare(
    settings: &crate::state::config_models::AuthSettings,
    browser: Option<&str>,
    source: &str,
    context: &CookieContext,
    mut cancel: tokio::sync::watch::Receiver<bool>,
  ) -> Result<Self, &'static str> {
    use crate::extract::chain::{remaining_budget, wait_until, Interrupted, FETCH_TIMEOUT};
    use std::{io::Write, process::Stdio, time::Duration};
    let args = super::ytdlp_runner::cookie_source_args(settings, browser);
    if args
      .first()
      .is_none_or(|arg| arg != "--cookies-from-browser")
    {
      return Ok(Self { args, _jar: None });
    }
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
    let browser = &args[1];
    if browser == "firefox" || browser.starts_with("firefox:") {
      export_firefox(browser, &host, &jar, &mut scoped).map_err(|_| COOKIE_READ_ERROR)?;
    } else {
      let full = jar.directory.join("full.txt");
      private_file(&full)
        .and_then(|mut file| file.write_all(COOKIE_HEADER.as_bytes()))
        .map_err(|_| COOKIE_READ_ERROR)?;
      let info = jar.directory.join("empty.json");
      private_file(&info).and_then(|mut file| file.write_all(br#"{"_type":"playlist","id":"cookie-export","title":"cookie-export","extractor":"generic","extractor_key":"Generic","entries":[]}"#)).map_err(|_| COOKIE_READ_ERROR)?;
      let mut command = std::process::Command::new(&context.exporter);
      command
        .args([
          "--ignore-config",
          "--no-cache-dir",
          "--cookies-from-browser",
          browser,
          "--cookies",
        ])
        .arg(&full)
        .args([
          "--skip-download",
          "--no-clean-info-json",
          "--load-info-json",
        ])
        .arg(&info)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
      super::ytdlp_process::configure_command(&mut command).map_err(|_| COOKIE_READ_ERROR)?;
      let deadline = tokio::time::Instant::now() + budget;
      let status = match wait_until(run_cookie_export(command), deadline, &mut cancel).await {
        Ok(status) => status?,
        Err(Interrupted::Cancelled) => return Err("cancelled"),
        Err(Interrupted::Timeout) => return Err(FETCH_TIMEOUT),
      };
      if !status.success() {
        return Err(COOKIE_READ_ERROR);
      }
      filter_export(&full, &host, &mut scoped).map_err(|_| COOKIE_READ_ERROR)?;
      std::fs::remove_file(info).map_err(|_| COOKIE_READ_ERROR)?;
    }
    scoped.flush().map_err(|_| COOKIE_READ_ERROR)?;
    Ok(Self {
      args: vec!["--cookies".into(), path.to_string_lossy().into_owned()],
      _jar: Some(std::sync::Arc::new(jar)),
    })
  }
}

// The exporter may start Keychain helpers: cancel the entire process tree, as for downloads.
struct ExportProcess(Option<super::ytdlp_process::PlatformProcess>);
impl Drop for ExportProcess {
  fn drop(&mut self) {
    if let Some(process) = &self.0 {
      super::ytdlp_process::kill_platform_process(process);
      #[cfg(unix)]
      if let Some(pgid) = process.pgid {
        unsafe {
          libc::killpg(pgid, libc::SIGKILL);
        }
      }
    }
  }
}
async fn run_cookie_export(
  mut command: std::process::Command,
) -> Result<std::process::ExitStatus, &'static str> {
  let process_lock = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
  let mut child = command.spawn().map_err(|_| COOKIE_READ_ERROR)?;
  let platform = match super::ytdlp_process::platform_process_from_child(&child) {
    Ok(platform) => platform,
    Err(_) => {
      let _ = child.kill();
      let _ = child.wait();
      return Err(COOKIE_READ_ERROR);
    }
  };
  let mut guard = ExportProcess(Some(platform));
  let (sender, receiver) = tokio::sync::oneshot::channel();
  std::thread::spawn(move || {
    let _process_lock = process_lock;
    let result = child.wait();
    let _ = sender.send(result);
  });
  let status = receiver
    .await
    .map_err(|_| COOKIE_READ_ERROR)?
    .map_err(|_| COOKIE_READ_ERROR)?;
  guard.0 = None;
  Ok(status)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::state::config_models::AuthSettings;
  use std::io::Write;
  use tokio::sync::watch;

  fn context(root: &Path) -> CookieContext {
    CookieContext {
      exporter: root.join("must-not-run"),
      directory: root.join("scoped"),
    }
  }
  fn firefox_fixture(root: &Path) -> String {
    let profile = root.join("profile");
    std::fs::create_dir(&profile).unwrap();
    let db = Connection::open(profile.join("cookies.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, originAttributes TEXT);").unwrap();
    for host in [
      ".reddit.com",
      "www.reddit.com",
      "google.com",
      "bank.example",
      "reddit.com.evil.example",
      "notreddit.com",
      ".instagram.com",
      "threads.net",
      "threads.com",
      "www.instagram.com.evil.example",
      ".x.com",
      ".twitter.com",
      "eviltwitter.com",
    ] {
      db.execute(
        "INSERT INTO moz_cookies VALUES (?1, 'fixture', 'synthetic', '/', 4102444800, 1, 1, '')",
        [host],
      )
      .unwrap();
    }
    format!("firefox:{}", profile.display())
  }
  fn domains(path: &Path) -> Vec<String> {
    let contents = std::fs::read_to_string(path).unwrap();
    let mut domains: Vec<_> = contents
      .lines()
      .filter_map(|line| {
        line
          .strip_prefix("#HttpOnly_")
          .or_else(|| (!line.starts_with('#') && !line.is_empty()).then_some(line))
      })
      .map(|line| line.split('\t').next().unwrap().to_owned())
      .collect();
    domains.sort();
    domains
  }

  #[tokio::test]
  async fn scoped_cookie_files_restrict_both_engines_and_are_removed_after_use() {
    let root = Staging::new().unwrap();
    let browser = firefox_fixture(&root.path);
    let context = context(&root.path);
    let (_tx, cancel) = watch::channel(true);
    for (host, expected) in [
      ("www.reddit.com", vec![".reddit.com", "www.reddit.com"]),
      (
        "threads.com",
        vec![".instagram.com", "threads.com", "threads.net"],
      ),
      ("x.com", vec![".twitter.com", ".x.com"]),
      ("twitter.com", vec![".twitter.com", ".x.com"]),
    ] {
      let access = CookieAccess::prepare(
        &AuthSettings::default(),
        Some(&browser),
        &format!("https://{host}/post"),
        &context,
        cancel.clone(),
      )
      .await
      .unwrap();
      assert_eq!(access.args[0], "--cookies");
      assert!(!access
        .args
        .iter()
        .any(|arg| arg == "--cookies-from-browser"));
      let path = PathBuf::from(&access.args[1]);
      assert_eq!(domains(&path), expected);
      assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1,
        "only the scoped jar survives preparation"
      );
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
          std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
          0o600
        );
      }
      let gallery = crate::runners::gallery::arguments(&access.args, Path::new("config.json"));
      assert!(gallery.windows(2).any(|args| args == access.args));
      assert!(!gallery.iter().any(|arg| arg == "--cookies-from-browser"));
      drop(access);
      assert!(!path.exists());
    }
    assert_eq!(std::fs::read_dir(context.directory).unwrap().count(), 0);
  }

  #[tokio::test]
  async fn declined_consent_creates_no_file_and_manual_cookie_file_is_unchanged() {
    let root = Staging::new().unwrap();
    let context = context(&root.path);
    let settings = AuthSettings::default();
    let (_tx, cancel) = watch::channel(true);
    for browser in [None, Some(super::super::login::ANONYMOUS_BROWSER)] {
      let args = CookieAccess::prepare(
        &settings,
        browser,
        "https://reddit.com/post",
        &context,
        cancel.clone(),
      )
      .await
      .unwrap();
      assert!(args.args.is_empty());
      assert!(!context.directory.exists());
    }
    let manual = root.path.join("manual.txt");
    std::fs::write(&manual, b"user-managed jar").unwrap();
    let settings = AuthSettings {
      cookie_file: Some(manual.to_string_lossy().into_owned()),
      ..Default::default()
    };
    let args = CookieAccess::prepare(&settings, None, "https://reddit.com/post", &context, cancel)
      .await
      .unwrap();
    assert_eq!(args.args, ["--cookies", manual.to_str().unwrap()]);
    drop(args);
    assert_eq!(std::fs::read(manual).unwrap(), b"user-managed jar");
    assert!(!context.directory.exists());
  }

  #[tokio::test]
  async fn unreadable_firefox_never_falls_back_and_removes_partial_exports() {
    let root = Staging::new().unwrap();
    let context = context(&root.path);
    let browser = format!("firefox:{}", root.path.display());
    let (_tx, cancel) = watch::channel(true);
    assert!(CookieAccess::prepare(
      &AuthSettings::default(),
      Some(&browser),
      "https://reddit.com/post",
      &context,
      cancel
    )
    .await
    .is_err());
    assert_eq!(std::fs::read_dir(context.directory).unwrap().count(), 0);
  }

  #[test]
  fn filtering_removes_the_full_export_on_success_and_parse_error_and_sweep_removes_stale_jars() {
    let root = Staging::new().unwrap();
    for invalid in [false, true] {
      let full = root.path.join("full.txt");
      std::fs::write(&full, if invalid { "malformed jar\n" } else { "# Netscape HTTP Cookie File\n#HttpOnly_.reddit.com\tTRUE\t/\tTRUE\t4102444800\tfixture\tsynthetic\ngoogle.com\tFALSE\t/\tTRUE\t4102444800\tfixture\tsynthetic\n" }).unwrap();
      let path = root.path.join(format!("scoped-{invalid}"));
      let mut scoped = private_file(&path).unwrap();
      scoped.write_all(COOKIE_HEADER.as_bytes()).unwrap();
      assert_eq!(
        filter_export(&full, "reddit.com", &mut scoped).is_err(),
        invalid
      );
      assert!(!full.exists());
      if !invalid {
        assert_eq!(domains(&path), [".reddit.com"]);
      }
    }
    let stale = root.path.join(format!("attempt-{}", uuid::Uuid::new_v4()));
    private_directory(&stale).unwrap();
    std::fs::write(stale.join("full.txt"), b"synthetic").unwrap();
    let snapshot = root
      .path
      .join(format!("nezzel-images-{}", uuid::Uuid::new_v4()));
    private_directory(&snapshot).unwrap();
    std::fs::write(snapshot.join("cookies.sqlite"), b"synthetic snapshot").unwrap();
    let keep = root.path.join("unrelated.txt");
    std::fs::write(&keep, b"keep").unwrap();
    sweep_directory(&root.path).unwrap();
    assert!(!stale.exists());
    sweep_named_directories(&root.path, "nezzel-images-").unwrap();
    assert!(
      !snapshot.exists(),
      "aborted discovery snapshots must be swept"
    );
    assert!(keep.exists());
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let other = root
        .path
        .join(format!("nezzel-images-{}", uuid::Uuid::new_v4()));
      std::fs::create_dir(&other).unwrap();
      std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
      sweep_named_directories(&root.path, "nezzel-images-").unwrap();
      assert!(
        other.exists(),
        "shared temp entries that are not private must be left alone"
      );
    }
  }

  #[cfg(unix)]
  fn exporter_fixture(root: &Path, mode: &str) -> CookieContext {
    use std::os::unix::fs::PermissionsExt;
    let exporter = root.join("exporter");
    std::fs::write(&exporter, format!(r#"#!/bin/sh
set -eu
full=''
info=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --cookies) full="$2"; shift 2 ;;
    --cookies-from-browser) shift 2 ;;
    --load-info-json) info="$2"; shift 2 ;;
    --ignore-config|--no-cache-dir|--skip-download|--no-clean-info-json) shift ;;
    *) exit 90 ;;
  esac
done
[ -f "$info" ]
printf '%s\n' '# Netscape HTTP Cookie File' '.reddit.com	TRUE	/	TRUE	4102444800	fixture	synthetic' 'google.com	FALSE	/	TRUE	4102444800	fixture	synthetic' > "$full"
case '{mode}' in
  error) exit 1 ;;
  wait) exec /usr/bin/tail -f /dev/null ;;
esac
"#)).unwrap();
    std::fs::set_permissions(&exporter, std::fs::Permissions::from_mode(0o700)).unwrap();
    CookieContext {
      exporter,
      directory: root.join("scoped"),
    }
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn chromium_exporter_passes_only_scoped_args_and_erases_full_jar() {
    let root = Staging::new().unwrap();
    let context = exporter_fixture(&root.path, "success");
    let (_tx, cancel) = watch::channel(true);
    let access = CookieAccess::prepare(
      &AuthSettings::default(),
      Some("chrome:/synthetic"),
      "https://reddit.com/post",
      &context,
      cancel,
    )
    .await
    .unwrap();
    assert_eq!(access.args[0], "--cookies");
    let path = PathBuf::from(&access.args[1]);
    assert_eq!(domains(&path), [".reddit.com"]);
    assert!(!path.with_file_name("full.txt").exists());
    assert_eq!(
      std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
      1
    );
    let child = access.clone();
    assert_eq!(access.args, child.args);
    drop(access);
    assert!(path.exists());
    drop(child);
    assert!(!path.exists());
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn cancelling_an_active_export_removes_the_full_and_scoped_files() {
    let root = Staging::new().unwrap();
    let context = exporter_fixture(&root.path, "wait");
    let (tx, cancel) = watch::channel(true);
    let settings = AuthSettings::default();
    let prepare = CookieAccess::prepare(
      &settings,
      Some("chrome:/synthetic"),
      "https://reddit.com/post",
      &context,
      cancel,
    );
    tokio::pin!(prepare);
    let cancel_after_write = async {
      loop {
        let exported = std::fs::read_dir(&context.directory)
          .into_iter()
          .flatten()
          .flatten()
          .any(|entry| {
            std::fs::metadata(entry.path().join("full.txt"))
              .is_ok_and(|metadata| metadata.len() > COOKIE_HEADER.len() as u64)
          });
        if exported {
          tx.send(false).unwrap();
          break;
        }
        tokio::task::yield_now().await;
      }
    };
    tokio::select! {
      result = &mut prepare => { assert!(result.is_ok(), "export returned before cancellation"); panic!("export must wait for cancellation"); },
      () = cancel_after_write => { assert!(matches!(prepare.await, Err("cancelled"))); },
    }
    assert_eq!(std::fs::read_dir(&context.directory).unwrap().count(), 0);
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn exporter_failure_removes_every_cookie_file() {
    let root = Staging::new().unwrap();
    let context = exporter_fixture(&root.path, "error");
    let (_tx, cancel) = watch::channel(true);
    let result = CookieAccess::prepare(
      &AuthSettings::default(),
      Some("chrome:/synthetic"),
      "https://reddit.com/post",
      &context,
      cancel,
    )
    .await;
    assert!(matches!(result, Err(COOKIE_READ_ERROR)));
    assert_eq!(std::fs::read_dir(&context.directory).unwrap().count(), 0);
  }

  #[cfg(unix)]
  #[tokio::test(start_paused = true)]
  async fn exporter_timeout_removes_every_cookie_file() {
    let root = Staging::new().unwrap();
    let context = exporter_fixture(&root.path, "wait");
    let (_tx, cancel) = watch::channel(true);
    let result = CookieAccess::prepare(
      &AuthSettings::default(),
      Some("chrome:/synthetic"),
      "https://reddit.com/post",
      &context,
      cancel,
    )
    .await;
    assert!(matches!(result, Err(crate::extract::chain::FETCH_TIMEOUT)));
    assert_eq!(std::fs::read_dir(&context.directory).unwrap().count(), 0);
  }
}
