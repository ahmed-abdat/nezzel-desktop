//! The private Netscape cookie jar on disk: owner-only staging directories, site-scoped rows,
//! and the startup sweep that reclaims jars a crash left behind.
use super::{access::CookieContext, cookie_domains};
use std::path::{Path, PathBuf};

pub(super) const COOKIE_HEADER: &str = "# Netscape HTTP Cookie File\n";
pub(super) struct CookieJar {
  pub directory: PathBuf,
}
impl Drop for CookieJar {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.directory);
  }
}

pub(super) fn private_directory(path: &Path) -> std::io::Result<()> {
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
pub(super) fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
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
pub(super) fn sweep_directory(root: &Path) -> std::io::Result<()> {
  if !root.exists() {
    return Ok(());
  }
  private_directory(root)?;
  sweep_named_directories(root, "attempt-")
}
pub(super) fn sweep_named_directories(root: &Path, prefix: &str) -> std::io::Result<()> {
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

pub(super) fn allowed_cookie_host(cookie_host: &str, host: &str) -> bool {
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

/// One cookie as a Netscape jar line; untrusted fields are checked before anything is written.
pub(super) struct NetscapeRow<'a> {
  pub domain: &'a str,
  pub path: &'a str,
  pub secure: bool,
  pub http_only: bool,
  pub expiry: i64,
  pub name: &'a str,
  pub value: &'a str,
}
impl NetscapeRow<'_> {
  pub fn write(&self, destination: &mut std::fs::File) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    if [self.domain, self.name, self.value, self.path]
      .iter()
      .any(|field| field.contains(['\t', '\r', '\n']))
    {
      return Err("invalid cookie field".into());
    }
    writeln!(
      destination,
      "{}{}\t{}\t{}\t{}\t{}\t{}\t{}",
      if self.http_only { "#HttpOnly_" } else { "" },
      self.domain,
      if self.domain.starts_with('.') {
        "TRUE"
      } else {
        "FALSE"
      },
      self.path,
      if self.secure { "TRUE" } else { "FALSE" },
      self.expiry,
      self.name,
      self.value
    )?;
    Ok(())
  }
}

/// Untrusted Netscape fields are checked before any row is allowed into the scoped jar.
pub(super) fn filter_export(
  full: &Path,
  host: &str,
  scoped: &mut std::fs::File,
) -> std::io::Result<usize> {
  let result = scoped_rows(full, host, scoped);
  // Never keep the unscoped export, including on a parse/write failure.
  std::fs::remove_file(full)?;
  result
}

/// Count only validated rows for this site, also for a user-managed jar.
pub(super) fn scoped_rows(
  full: &Path,
  host: &str,
  scoped: &mut impl std::io::Write,
) -> std::io::Result<usize> {
  use std::io::BufRead;
  let mut count = 0;
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
      count += 1;
    }
  }
  Ok(count)
}
