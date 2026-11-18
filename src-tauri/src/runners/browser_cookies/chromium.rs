//! Chromium's cookie database layout (Chrome, Edge, Brave, ...), hints and private snapshots.
//! Chromium cookie values are exported by yt-dlp (`exporter`), never decrypted here.
use super::hint::HintSchema;
use crate::runners::browser_logins::Os;
use std::collections::HashSet;

/// Seconds from 1601-01-01 (Chromium's clock) to the Unix epoch.
const CHROMIUM_EPOCH: i64 = 11_644_473_600;

pub(super) const HINT_SCHEMA: HintSchema = HintSchema {
  table: "cookies",
  host_column: "host_key",
  expiry: "expires_utc",
  access: "last_access_utc",
  app_bound,
  expired,
  access_micros,
};

/// Windows Chromium v20 values are bound to the browser app and cannot be exported.
fn app_bound(os: Os, columns: &HashSet<String>) -> &'static str {
  if matches!(os, Os::Windows) && columns.contains("encrypted_value") {
    "COALESCE(substr(encrypted_value, 1, 3) = x'763230', 0)"
  } else {
    "0"
  }
}

/// Microseconds since 1601; 0 is a session cookie, which never counts as expired.
fn expired(expires: i64, now: i64) -> bool {
  let expires = if expires == 0 {
    0
  } else {
    expires / 1_000_000 - CHROMIUM_EPOCH
  };
  expires != 0 && expires <= now
}

fn access_micros(access: i64) -> i64 {
  access.saturating_sub(CHROMIUM_EPOCH * 1_000_000)
}

/// yt-dlp searches recursively for the newest Cookies file. Chromium's Gemini and extension
/// partitions are separate jars: stage only the real profile DB, never those partitions (#70).
pub(super) fn export_profile(
  browser: &crate::runners::browsers::BrowserSource,
  host: &str,
  parent: &std::path::Path,
) -> Result<
  (
    crate::runners::browsers::BrowserSource,
    super::jar::CookieJar,
  ),
  &'static str,
> {
  use super::jar::{private_directory, private_file, CookieJar};
  use crate::runners::browsers::BrowserSource;
  use std::path::{Path, PathBuf};
  let requested = PathBuf::from(browser.profile.as_deref().unwrap_or_default());
  let profile = if requested.is_absolute() {
    requested
  } else {
    crate::runners::browser_logins::discover(None, host)
      .into_iter()
      .filter(|c| c.browser == browser.kind)
      .filter_map(|c| BrowserSource::parse(&c.id)?.profile)
      .find(|path| {
        requested.as_os_str().is_empty() || Path::new(path).file_name() == requested.file_name()
      })
      .map(PathBuf::from)
      .ok_or("browserProfileMissing")?
  };
  let database = [profile.join("Network/Cookies"), profile.join("Cookies")]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or("browserProfileMissing")?;
  let staging = CookieJar {
    directory: parent.join("chromium"),
  };
  let copied_profile = staging.directory.join("Default");
  let copy = || -> Result<(), Box<dyn std::error::Error>> {
    private_directory(&staging.directory)?;
    private_directory(&copied_profile)?;
    let copy_private = |from: &Path, to: &Path| -> std::io::Result<()> {
      let mut input = std::fs::File::open(from)?;
      let mut output = private_file(to)?;
      std::io::copy(&mut input, &mut output)?;
      Ok(())
    };
    let copied_db = copied_profile.join("Cookies");
    copy_database(&database, &copied_db, copy_private)?;
    let local_state = [
      profile.join("Local State"),
      profile.parent().unwrap_or(&profile).join("Local State"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    if let Some(state) = local_state {
      copy_private(&state, &staging.directory.join("Local State"))?;
      // Opera has no profiles layer: yt-dlp looks in the supplied directory itself.
      if browser.kind == crate::runners::browsers::BrowserKind::Opera {
        copy_private(&state, &copied_profile.join("Local State"))?;
      }
    }
    Ok(())
  };
  copy().map_err(|_| super::access::COOKIE_READ_ERROR)?;
  let mut staged = browser.clone();
  staged.profile = Some(copied_profile.to_string_lossy().into_owned());
  Ok((staged, staging))
}

pub(super) fn copy_database(
  database: &std::path::Path,
  copied_db: &std::path::Path,
  copy: impl Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> Result<(), Box<dyn std::error::Error>> {
  // Preserve committed WAL rows before Chrome can checkpoint and truncate the live WAL.
  let wal = database.with_file_name("Cookies-wal");
  let has_wal = wal.is_file();
  if has_wal {
    copy(&wal, &copied_db.with_file_name("Cookies-wal"))?;
  }
  copy(database, copied_db)?;
  // Checkpoint only the private copy, so yt-dlp's own DB copy includes live WAL rows.
  if has_wal {
    let db = rusqlite::Connection::open(copied_db)?;
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
  }
  Ok(())
}
