//! The yt-dlp update flow: release input validation, daily check and install, and a rollback-safe whole-bundle swap.
use std::{
  cmp::Ordering,
  io,
  path::{Path, PathBuf},
};

use std::{
  collections::HashMap,
  sync::Arc,
  time::{Duration, SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::fs;

use super::binaries_manager::{
  read_metadata, write_metadata, AnyError, BinariesManager, Manifest, ToolComplete, ToolInfo,
  BUNDLED_MANIFEST, SMALL_REQUEST_TIMEOUT,
};
use super::{bundled_tools, YTDLP_PROCESSES};

pub(super) const LATEST_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest";
pub(super) const CURRENT_DIR: &str = ".yt-dlp-current";

pub(super) fn release_tag(location: &str) -> Option<&str> {
  let tag = location
    .strip_prefix("https://github.com/yt-dlp/yt-dlp/releases/tag/")
    .or_else(|| location.strip_prefix("/yt-dlp/yt-dlp/releases/tag/"))?;
  let parts: Vec<_> = tag.split('.').collect();
  if !(3..=4).contains(&parts.len())
    || parts[0].len() != 4
    || parts[1].len() != 2
    || parts[2].len() != 2
    || parts
      .iter()
      .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
  {
    return None;
  }
  let month: u32 = parts[1].parse().ok()?;
  let day: u32 = parts[2].parse().ok()?;
  if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
    return None;
  }
  compare_versions(tag, tag)?;
  Some(tag)
}

pub(super) fn redirect_tag(status: u16, location: Option<&str>) -> Result<&str, &'static str> {
  if !(300..400).contains(&status) {
    return Err("latest yt-dlp release did not redirect");
  }
  let location = location.ok_or("latest yt-dlp release has no Location")?;
  release_tag(location).ok_or("invalid latest yt-dlp release tag")
}

fn backup_base(name: &str) -> Option<String> {
  let (base, id) = name.rsplit_once(".old-")?;
  (!base.is_empty() && uuid::Uuid::parse_str(id).is_ok()).then(|| base.to_string())
}

// Run once before this manager starts any archive preparation, under both locks.
pub(super) fn cleanup_leftovers(bin: &Path) -> io::Result<()> {
  // Interrupted swaps leave `<name>.old-<uuid>`. Restore it when the canonical name is gone and
  // exactly one backup exists; otherwise the backups are stale.
  let mut backups: std::collections::BTreeMap<String, Vec<std::path::PathBuf>> = Default::default();
  for entry in std::fs::read_dir(bin)? {
    let entry = entry?;
    let name = entry.file_name();
    let name = name.to_string_lossy();
    if name.starts_with(".yt-dlp-stage-") || name.starts_with(".extract-") {
      remove_path(&entry.path())?;
    } else if let Some(base) = backup_base(&name) {
      backups.entry(base).or_default().push(entry.path());
    }
  }
  for (base, mut paths) in backups {
    let canonical = bin.join(base);
    paths.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    if !canonical.exists() {
      let Some(newest) = paths.pop() else { continue };
      if std::fs::rename(&newest, &canonical).is_err() {
        continue; // keep it: it may be the only copy
      }
    }
    for path in paths {
      if let Err(error) = remove_path(&path) {
        tracing::info!("old tool backup not removed: {error}");
      }
    }
  }
  Ok(())
}

pub(super) fn cleanup_legacy(bin: &Path) -> io::Result<()> {
  let name = &crate::binaries::bundled_tools::exe_name("yt-dlp");
  if bin.join(CURRENT_DIR).join(name).is_file() {
    remove_path(&bin.join(name))?;
    remove_path(&bin.join("_internal"))?;
  }
  Ok(())
}

fn remove_path(path: &Path) -> io::Result<()> {
  match std::fs::symlink_metadata(path) {
    Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
    Ok(_) => std::fs::remove_file(path),
    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
    Err(error) => Err(error),
  }
}

pub(super) fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
  fn parts(version: &str) -> Option<[u32; 4]> {
    let mut result = [0; 4];
    let values: Vec<_> = version.split('.').collect();
    if !(3..=4).contains(&values.len()) {
      return None;
    }
    for (i, value) in values.iter().enumerate() {
      if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
      }
      result[i] = value.parse().ok()?;
    }
    Some(result)
  }
  Some(parts(a)?.cmp(&parts(b)?))
}

pub(super) fn checksum_for(sums: &str, asset: &str) -> Option<String> {
  let mut found = None;
  for line in sums.lines() {
    let mut fields = line.split_whitespace();
    let (Some(hash), Some(name)) = (fields.next(), fields.next()) else {
      continue;
    };
    if name.strip_prefix('*').unwrap_or(name) != asset {
      continue;
    }
    if found.is_some()
      || fields.next().is_some()
      || hash.len() != 64
      || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
      return None;
    }
    found = Some(hash.to_ascii_lowercase());
  }
  found
}

pub(super) fn check_due(last_check: Option<u64>, now: u64) -> bool {
  last_check.is_none_or(|last| now.saturating_sub(last) >= 24 * 60 * 60)
}

/// Persist the first failed attempt across restarts; a successful check/install clears it.
fn record_update_health(
  meta: &mut super::binaries_manager::Metadata,
  now: u64,
  success: bool,
  offline: bool,
) -> bool {
  if success {
    meta.ytdlp_failure_since = None;
  } else {
    meta.ytdlp_failure_since.get_or_insert(now);
  }
  !offline && update_is_stale(meta, now)
}

pub(super) fn update_is_stale(meta: &super::binaries_manager::Metadata, now: u64) -> bool {
  meta
    .ytdlp_failure_since
    .is_some_and(|since| now.saturating_sub(since) >= 7 * 86400)
}

fn update_offline(mut error: &(dyn std::error::Error + 'static)) -> bool {
  loop {
    let text = error.to_string().to_ascii_lowercase();
    if error
      .downcast_ref::<reqwest::Error>()
      .is_some_and(|error| error.is_connect() || error.is_timeout())
      || text.contains("error sending request")
      || text.contains("timed out")
      || crate::parsers::ytdlp_error::network_down("", &text)
      || text.contains("dns error")
      || text.contains("no such host")
      || text.contains("failed to lookup address")
    {
      return true;
    }
    match error.source() {
      Some(source) => error = source,
      None => return false,
    }
  }
}

pub(crate) fn executable(bin: &Path) -> PathBuf {
  let name = &crate::binaries::bundled_tools::exe_name("yt-dlp");
  let current = bin.join(CURRENT_DIR).join(name);
  if current.is_file() {
    current
  } else {
    let backup = bin.join(CURRENT_DIR).with_extension("previous").join(name);
    if backup.is_file() {
      backup
    } else {
      bin.join(name)
    }
  }
}

/// Recover an interrupted transaction before planning another install.
pub(super) fn recover_bundle(current: &Path) -> io::Result<()> {
  let backup = current.with_extension("previous");
  if backup.exists() {
    if current.exists() {
      remove_path(&backup)?;
    } else {
      std::fs::rename(backup, current)?;
    }
  }
  Ok(())
}

/// Call only while holding the process write lock. Keep the backup until metadata commits.
pub(super) fn swap_bundle(
  staged: &Path,
  current: &Path,
  commit: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
  let backup = current.with_extension("previous");
  recover_bundle(current)?;
  let had_previous = current.exists();
  if had_previous {
    std::fs::rename(current, &backup)?;
  }
  if let Err(error) = std::fs::rename(staged, current) {
    if had_previous {
      std::fs::rename(&backup, current)?;
    }
    return Err(error);
  }
  if let Err(error) = commit() {
    std::fs::rename(current, staged)?;
    if had_previous {
      std::fs::rename(&backup, current)?;
    }
    return Err(error);
  }
  if had_previous {
    let _ = std::fs::remove_dir_all(backup);
  }
  Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct CheckAttempt {
  pub(super) checked_at: u64,
  pub(super) previous: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum InstallOutcome {
  Installed(Option<String>),
  UpToDate,
  Busy,
  Locked,
}

pub(super) fn binary_client(
  enabled: Option<bool>,
  proxy: Option<&str>,
) -> Result<Option<reqwest::Client>, AnyError> {
  let mut builder = reqwest::Client::builder()
    .https_only(true)
    .connect_timeout(Duration::from_secs(15))
    .read_timeout(Duration::from_secs(30))
    .redirect(reqwest::redirect::Policy::custom(|attempt| {
      if attempt
        .previous()
        .first()
        .is_some_and(|url| url.as_str() == LATEST_URL)
      {
        attempt.stop()
      } else {
        reqwest::redirect::Policy::limited(10).redirect(attempt)
      }
    }));
  if enabled == Some(true) {
    let Some(proxy) = proxy.filter(|value| !value.is_empty()) else {
      return Ok(None);
    };
    builder =
      builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "invalid binary proxy setting")?);
  }
  Ok(Some(builder.build()?))
}

async fn commit_ytdlp(
  bin: &Path,
  staged: &Path,
  version: &str,
  attempt: Option<CheckAttempt>,
  operation: Arc<tokio::sync::Mutex<()>>,
  processes: Arc<tokio::sync::RwLock<()>>,
) -> Result<InstallOutcome, AnyError> {
  let operation = operation.lock_owned().await;
  let meta_path = bin.join("metadata.json");
  let mut meta = read_metadata(&meta_path).await?;
  if meta.is_locked {
    return Ok(InstallOutcome::Locked);
  }
  // Another installer may have won while the archive was downloading.
  if meta.versions.get("yt-dlp").is_some_and(|installed| {
    compare_versions(installed, version).is_some_and(|order| order != Ordering::Less)
      && bundled_tools::verified_repair(&executable(bin), "yt-dlp", installed, "", "")
  }) {
    return Ok(InstallOutcome::UpToDate);
  }
  let Ok(process_guard) = processes.try_write_owned() else {
    if let Some(attempt) =
      attempt.filter(|attempt| meta.ytdlp_last_check == Some(attempt.checked_at))
    {
      meta.ytdlp_last_check = attempt.previous;
      write_metadata(&meta_path, &serde_json::to_vec(&meta)?)?;
    }
    return Ok(InstallOutcome::Busy);
  };
  let old = meta.versions.insert("yt-dlp".into(), version.into());
  let bytes = serde_json::to_vec(&meta)?;
  let current = bin.join(CURRENT_DIR);
  let source = staged.to_path_buf();
  let bin = bin.to_path_buf();
  // Keep both locks through rollback/cleanup even if the async caller is cancelled.
  tokio::task::spawn_blocking(move || {
    let _operation = operation;
    let _process_guard = process_guard;
    swap_bundle(&source, &current, || write_metadata(&meta_path, &bytes))?;
    for result in [cleanup_leftovers(&current), cleanup_legacy(&bin)] {
      if let Err(error) = result {
        tracing::info!("yt-dlp cleanup deferred: {error}");
      }
    }
    Ok::<_, std::io::Error>(())
  })
  .await??;
  Ok(InstallOutcome::Installed(old))
}

impl BinariesManager {
  pub(super) async fn maintain_ytdlp(&self) {
    self
      .maintenance
      .get_or_init(|| async {
        let operation = self.operation.clone().lock_owned().await;
        match self
          .load_metadata(&self.bin_dir.join("metadata.json"))
          .await
        {
          Ok(meta) if meta.is_locked => return,
          Err(error) => {
            tracing::info!("yt-dlp maintenance deferred: {error}");
            return;
          }
          _ => {}
        }
        let Ok(process_guard) = YTDLP_PROCESSES.clone().try_write_owned() else {
          return;
        };
        let bin = self.bin_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
          let _operation = operation;
          let _process_guard = process_guard;
          std::fs::create_dir_all(&bin)?;
          recover_bundle(&bin.join(CURRENT_DIR))?;
          // This runs once, before any prepare call can create a live stage directory.
          cleanup_leftovers(&bin)?;
          cleanup_legacy(&bin)
        })
        .await;
        match result {
          Ok(Ok(())) => {}
          Ok(Err(error)) => tracing::info!("yt-dlp maintenance deferred: {error}"),
          Err(error) => tracing::info!("yt-dlp maintenance failed: {error}"),
        }
      })
      .await;
  }

  /// Spawn daily work without awaiting it from startup.
  pub fn schedule_ytdlp_update(app: &AppHandle<Wry>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
      let manager = app.state::<Self>();
      let _ = manager.run_ytdlp_update(false).await;
    });
  }

  /// One updater at a time, including explicit retries. Never pass update errors into IPC.
  pub(crate) async fn run_ytdlp_update(&self, force: bool) -> Result<(), &'static str> {
    let Ok(_check) = self.update_check.try_lock() else {
      return if force {
        Err("toolsUpdateFailed")
      } else {
        Ok(())
      };
    };
    let result = self.update_ytdlp(force).await;
    if matches!(result, Ok(false)) {
      return if force {
        Err("toolsUpdateFailed")
      } else {
        Ok(())
      };
    }
    let offline = result
      .as_ref()
      .err()
      .is_some_and(|error| update_offline(error.as_ref()));
    let success = result.is_ok();
    if !success {
      tracing::info!(offline, "yt-dlp update failed");
      crate::usage::engine_updated(&self.app, false);
    }
    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_err(|_| "toolsUpdateFailed")?
      .as_secs();
    let _operation = self.operation.lock().await;
    let meta_path = self.bin_dir.join("metadata.json");
    let mut meta = self
      .load_metadata(&meta_path)
      .await
      .map_err(|_| "toolsUpdateFailed")?;
    if offline {
      crate::diagnostics::send::network_response_failed();
    }
    let stale = record_update_health(&mut meta, now, success, offline);
    self
      .save_metadata(&meta_path, &meta)
      .map_err(|_| "toolsUpdateFailed")?;
    self
      .app
      .state::<super::binaries_state::BinariesState>()
      .update_stale
      .store(stale, std::sync::atomic::Ordering::SeqCst);
    crate::events::emit(&self.app, "binary_update_stale", stale);
    result.map(|_| ()).map_err(|_| "toolsUpdateFailed")
  }

  async fn update_ytdlp(&self, force: bool) -> Result<bool, AnyError> {
    self.maintain_ytdlp().await;
    let Some(client) = self.client()? else {
      return Ok(false);
    };
    let (old, attempt) = {
      let _operation = self.operation.lock().await;
      if !self
        .app
        .state::<crate::SharedConfig>()
        .load()
        .update
        .update_binaries
      {
        return Ok(false);
      }
      let meta_path = self.bin_dir.join("metadata.json");
      let mut meta = self.load_metadata(&meta_path).await?;
      let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
      if meta.is_locked || (!force && !check_due(meta.ytdlp_last_check, now)) {
        return Ok(false);
      }
      let manifest: Manifest = serde_json::from_slice(BUNDLED_MANIFEST)?;
      let Some(info) = manifest.tools.get("yt-dlp") else {
        return Ok(false);
      };
      let Some(path) = self.resolved_tool("yt-dlp", info, &meta) else {
        return Ok(false);
      };
      let old = if self.bundled_paths().get("yt-dlp") == Some(&path) {
        info.version.clone()
      } else {
        meta
          .versions
          .get("yt-dlp")
          .cloned()
          .ok_or("yt-dlp version missing")?
      };
      if YTDLP_PROCESSES.try_write().is_err() {
        tracing::info!("yt-dlp update deferred: process active");
        return Ok(false);
      }
      let attempt = CheckAttempt {
        checked_at: now,
        previous: meta.ytdlp_last_check,
      };
      // Persist failures too, but never hold the operation mutex during network work.
      meta.ytdlp_last_check = Some(now);
      self.save_metadata(&meta_path, &meta)?;
      (old, attempt)
    };
    let response = client
      .get(LATEST_URL)
      .timeout(SMALL_REQUEST_TIMEOUT)
      .send()
      .await?;
    // Reuse this existing HTTP exchange as online evidence; never add a connectivity probe.
    crate::diagnostics::send::network_response_received();
    let response = response.error_for_status()?;
    let tag = redirect_tag(
      response.status().as_u16(),
      response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok()),
    )?
    .to_string();
    if compare_versions(&tag, &old).ok_or("invalid installed yt-dlp version")? != Ordering::Greater
    {
      tracing::info!("yt-dlp up to date");
      return Ok(true);
    }
    let manifest = self.fetch_manifest(&client).await?;
    let arch = Self::current_platform();
    let pinned = manifest
      .tools
      .get("yt-dlp")
      .ok_or("yt-dlp absent from manifest")?;
    let (_, pinned_file) =
      Self::select_file(&pinned.files, &arch).ok_or("no yt-dlp asset for platform")?;
    let asset = pinned_file
      .url
      .rsplit('/')
      .next()
      .filter(|name| !name.is_empty())
      .ok_or("yt-dlp asset name missing")?;
    if !asset
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
      return Err("invalid yt-dlp asset name".into());
    }
    let base = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{tag}");
    let sums = client
      .get(format!("{base}/SHA2-256SUMS"))
      .timeout(SMALL_REQUEST_TIMEOUT)
      .send()
      .await?
      .error_for_status()?
      .text()
      .await?;
    let mut file = pinned_file.clone();
    file.sha256 = checksum_for(&sums, asset).ok_or("yt-dlp asset checksum absent or invalid")?;
    file.url = format!("{base}/{asset}");
    file.version = None;
    let info = ToolInfo {
      version: tag.clone(),
      files: HashMap::from([(arch.clone(), file)]),
    };
    match self
      .install_ytdlp(&client, &arch, &info, Some(attempt))
      .await?
    {
      InstallOutcome::Installed(previous) => {
        tracing::info!("yt-dlp updated {} -> {tag}", previous.unwrap_or(old));
        crate::usage::engine_updated(&self.app, true);
      }
      InstallOutcome::UpToDate => tracing::info!("yt-dlp up to date"),
      InstallOutcome::Busy => return Ok(false),
      InstallOutcome::Locked => return Ok(false),
    }
    Ok(true)
  }

  pub(super) async fn install_ytdlp(
    &self,
    client: &reqwest::Client,
    arch: &str,
    info: &ToolInfo,
    attempt: Option<CheckAttempt>,
  ) -> Result<InstallOutcome, AnyError> {
    let staged = self
      .bin_dir
      .join(format!(".yt-dlp-stage-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staged).await?;
    let result = async {
      // Network and extraction prepare an isolated directory without the operation mutex.
      self
        .install_single_tool(client, &staged, arch, "yt-dlp", info)
        .await
        .map_err(|error| format!("{}: {}", error.stage, error.error))?;
      let outcome = commit_ytdlp(
        &self.bin_dir,
        &staged,
        &info.version,
        attempt,
        self.operation.clone(),
        YTDLP_PROCESSES.clone(),
      )
      .await?;
      if let Ok(mut verified) = self.verified_repairs.lock() {
        verified.clear();
      }
      if matches!(
        outcome,
        InstallOutcome::Installed(_) | InstallOutcome::UpToDate
      ) {
        let _ = self.app.emit(
          "binary_download_complete",
          ToolComplete {
            tool: "yt-dlp".into(),
          },
        );
      }
      Ok::<_, AnyError>(outcome)
    }
    .await;
    let _ = fs::remove_dir_all(staged).await;
    result
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::binaries::binaries_manager::Metadata;

  #[test]
  fn location_accepts_only_stable_release_tags() {
    for tag in ["2026.11.01", "2026.11.01.1"] {
      let location = format!("https://github.com/yt-dlp/yt-dlp/releases/tag/{tag}");
      assert_eq!(release_tag(&location), Some(tag));
    }
    assert_eq!(
      release_tag("/yt-dlp/yt-dlp/releases/tag/2026.11.01"),
      Some("2026.11.01")
    );
    for bad in [
      "../2026.11.01",
      "2026.1.01",
      "2026.11.01/extra",
      "2026.11.01?x=y",
      "2026.11.01#x",
      "2026.11.01\r\nX: evil",
      "2026.13.01",
      "2026.11.00",
      "2026.11.01.",
      "2026.11.01.1.2",
      "nightly",
      "%2e%2e",
    ] {
      assert!(
        release_tag(&format!(
          "https://github.com/yt-dlp/yt-dlp/releases/tag/{bad}"
        ))
        .is_none(),
        "{bad}"
      );
    }
    assert!(release_tag("https://evil.test/yt-dlp/yt-dlp/releases/tag/2026.11.01").is_none());
  }

  #[test]
  fn versions_compare_numeric_parts() {
    for (a, b, want) in [
      ("2026.10.26", "2026.11.01", Ordering::Less),
      ("2026.11.01", "2026.11.01.1", Ordering::Less),
      ("2026.9.30", "2026.10.01", Ordering::Less),
      ("2026.11.01.10", "2026.11.01.2", Ordering::Greater),
      ("2026.11.01", "2026.11.01.0", Ordering::Equal),
    ] {
      assert_eq!(compare_versions(a, b), Some(want));
    }
    assert_eq!(compare_versions("broken", "2026.11.01"), None);
  }

  #[test]
  fn checksums_require_exact_asset_and_valid_hash() {
    let hash = "a".repeat(64);
    let other = "b".repeat(64);
    let sums = format!("{other}  yt-dlp_macos_legacy.zip\n{hash}  yt-dlp_macos.zip\n");
    assert_eq!(checksum_for(&sums, "yt-dlp_macos.zip"), Some(hash.clone()));
    assert_eq!(
      checksum_for(&format!("{hash} *yt-dlp_win.zip\r\n"), "yt-dlp_win.zip"),
      Some(hash)
    );
    assert_eq!(checksum_for(&sums, "yt-dlp_macos"), None);
    assert_eq!(
      checksum_for("not-a-sha256  yt-dlp_macos.zip", "yt-dlp_macos.zip"),
      None
    );
    assert_eq!(
      checksum_for(
        &format!("{sums}{other}  yt-dlp_macos.zip\n"),
        "yt-dlp_macos.zip"
      ),
      None
    );
  }

  #[test]
  fn update_failures_notice_after_seven_days_and_recovery_resets_the_streak() {
    for (message, offline) in [
      ("Network is unreachable", true),
      ("dns error", true),
      ("Connection timed out", true),
      (
        "error sending request for url (https://github.com/yt-dlp/yt-dlp/releases/latest)",
        true,
      ),
      ("checksum mismatch", false),
    ] {
      assert_eq!(update_offline(&std::io::Error::other(message)), offline);
    }
    let mut meta = super::super::binaries_manager::Metadata::default();
    let day = 86400;
    assert!(!record_update_health(&mut meta, 100, false, false));
    assert!(!record_update_health(
      &mut meta,
      100 + 7 * day - 1,
      false,
      false
    ));
    assert!(!record_update_health(&mut meta, 100 + 7 * day, false, true));
    assert!(record_update_health(&mut meta, 100 + 7 * day, false, false));
    let stored = serde_json::to_vec(&meta).unwrap();
    let mut reloaded = serde_json::from_slice(&stored).unwrap();
    assert!(record_update_health(
      &mut reloaded,
      100 + 8 * day,
      false,
      false
    ));
    assert!(!record_update_health(
      &mut reloaded,
      100 + 8 * day,
      true,
      false
    ));
    assert!(!record_update_health(
      &mut reloaded,
      100 + 9 * day,
      false,
      false
    ));
    assert!(!record_update_health(&mut reloaded, 99, false, false));
  }

  #[test]
  fn daily_gate_counts_attempts_including_failures() {
    assert!(check_due(None, 100));
    assert!(!check_due(Some(100), 86499));
    assert!(check_due(Some(100), 86500));
    assert!(!check_due(Some(200), 100));
    // Persist the attempt before any work, regardless of its result.
    let attempted_at = Some(86500);
    assert!(!check_due(attempted_at, 86501));
  }

  #[test]
  fn swap_failure_restores_old_bundle_and_success_replaces_whole_bundle() {
    let root = std::env::temp_dir().join(format!("nezzel-swap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let current = root.join(CURRENT_DIR);
    let staged = root.join("staged");
    std::fs::create_dir_all(&current).unwrap();
    let name = &crate::binaries::bundled_tools::exe_name("yt-dlp");
    std::fs::write(root.join(name), b"legacy").unwrap();
    std::fs::write(current.join(name), b"old").unwrap();
    std::fs::write(current.join("old-runtime"), b"runtime").unwrap();
    assert!(swap_bundle(&staged, &current, || Ok(())).is_err());
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"old");
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(staged.join(name), b"new").unwrap();
    assert!(swap_bundle(&staged, &current, || Err(io::Error::other(
      "metadata write failed"
    )))
    .is_err());
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"old");
    swap_bundle(&staged, &current, || Ok(())).unwrap();
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"new");
    assert!(!current.join("old-runtime").exists());
    assert_eq!(executable(&root), current.join(name));
    assert_eq!(std::fs::read(executable(&root)).unwrap(), b"new");
    assert_eq!(std::fs::read(root.join(name)).unwrap(), b"legacy");
    std::fs::remove_dir_all(root).unwrap();
  }
  #[test]
  fn leftover_backup_recovers_missing_current_and_discards_stale_backup() {
    let root = std::env::temp_dir().join(format!("nezzel-recover-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let current = root.join(CURRENT_DIR);
    let backup = current.with_extension("previous");
    let staged = root.join("staged");
    let name = &crate::binaries::bundled_tools::exe_name("yt-dlp");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join(name), b"old").unwrap();
    assert_eq!(executable(&root), backup.join(name));
    assert!(swap_bundle(&staged, &current, || Ok(())).is_err());
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"old");
    assert!(!backup.exists());
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join(name), b"stale").unwrap();
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(staged.join(name), b"new").unwrap();
    swap_bundle(&staged, &current, || Ok(())).unwrap();
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"new");
    assert!(!backup.exists());
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn release_redirect_requires_location_in_the_ytdlp_repository() {
    assert_eq!(
      redirect_tag(
        302,
        Some("https://github.com/yt-dlp/yt-dlp/releases/tag/2026.11.01")
      ),
      Ok("2026.11.01")
    );
    assert_eq!(
      redirect_tag(302, None),
      Err("latest yt-dlp release has no Location")
    );
    assert_eq!(
      redirect_tag(
        302,
        Some("https://github.com/other/tool/releases/tag/2026.11.01")
      ),
      Err("invalid latest yt-dlp release tag")
    );
    assert_eq!(
      redirect_tag(200, Some("/yt-dlp/yt-dlp/releases/tag/2026.11.01")),
      Err("latest yt-dlp release did not redirect")
    );
  }

  #[test]
  fn cleanup_removes_only_staging_and_a_replaced_legacy_bundle() {
    let root = std::env::temp_dir().join(format!("nezzel-cleanup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for name in [
      ".yt-dlp-stage-interrupted",
      ".extract-interrupted",
      "_internal",
      "other-tool",
    ] {
      std::fs::create_dir_all(root.join(name)).unwrap();
      std::fs::write(root.join(name).join("data"), b"payload").unwrap();
    }
    let binary = &crate::binaries::bundled_tools::exe_name("yt-dlp");
    std::fs::write(root.join(binary), b"legacy").unwrap();
    std::fs::write(root.join("deno"), b"deno").unwrap();
    cleanup_leftovers(&root).unwrap();
    assert!(!root.join(".yt-dlp-stage-interrupted").exists());
    assert!(!root.join(".extract-interrupted").exists());
    cleanup_legacy(&root).unwrap();
    assert!(root.join(binary).exists());
    assert!(root.join("_internal").exists());
    std::fs::create_dir_all(root.join(CURRENT_DIR)).unwrap();
    std::fs::write(root.join(CURRENT_DIR).join(binary), b"current").unwrap();
    cleanup_legacy(&root).unwrap();
    assert!(!root.join(binary).exists());
    assert!(!root.join("_internal").exists());
    assert!(root.join("deno").exists());
    assert!(root.join("other-tool/data").exists());
    assert_eq!(std::fs::read(executable(&root)).unwrap(), b"current");
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn proxy_enabled_without_address_skips_all_binary_requests() {
    for proxy in [None, Some("")] {
      assert!(super::binary_client(Some(true), proxy).unwrap().is_none());
    }
    assert!(super::binary_client(Some(false), Some("http://["))
      .unwrap()
      .is_some());
    assert!(
      super::binary_client(Some(true), Some("http://127.0.0.1:8123"))
        .unwrap()
        .is_some()
    );
    assert!(
      super::binary_client(Some(true), Some("socks5h://127.0.0.1:1080"))
        .unwrap()
        .is_some()
    );
    assert!(super::binary_client(Some(true), Some("http://[")).is_err());
  }

  #[tokio::test]
  async fn busy_commit_restores_check_time_and_rechecks_concurrent_version() {
    use super::{CheckAttempt, InstallOutcome};
    use std::sync::Arc;
    let root = std::env::temp_dir().join(format!("nezzel-busy-{}", uuid::Uuid::new_v4()));
    let current = root.join(super::CURRENT_DIR);
    let staged = root.join("staged");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&staged).unwrap();
    let name = &crate::binaries::bundled_tools::exe_name("yt-dlp");
    std::fs::write(current.join(name), b"old").unwrap();
    std::fs::write(staged.join(name), b"candidate").unwrap();
    let meta_path = root.join("metadata.json");
    let mut meta = Metadata::default();
    meta.versions.insert("yt-dlp".into(), "2026.10.26".into());
    meta.ytdlp_last_check = Some(86500);
    super::write_metadata(&meta_path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    let operation = Arc::new(tokio::sync::Mutex::new(()));
    let processes = Arc::new(tokio::sync::RwLock::new(()));
    let running = processes.clone().read_owned().await;
    let attempt = Some(CheckAttempt {
      checked_at: 86500,
      previous: Some(100),
    });
    assert_eq!(
      super::commit_ytdlp(
        &root,
        &staged,
        "2026.11.01",
        attempt,
        operation.clone(),
        processes.clone()
      )
      .await
      .unwrap(),
      InstallOutcome::Busy
    );
    let restored: Metadata = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    assert_eq!(restored.ytdlp_last_check, Some(100));
    assert_eq!(restored.versions["yt-dlp"], "2026.10.26");
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"old");
    meta.ytdlp_last_check = Some(99999);
    super::write_metadata(&meta_path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    assert_eq!(
      super::commit_ytdlp(
        &root,
        &staged,
        "2026.11.01",
        attempt,
        operation.clone(),
        processes.clone()
      )
      .await
      .unwrap(),
      InstallOutcome::Busy
    );
    let superseded: Metadata = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    assert_eq!(superseded.ytdlp_last_check, Some(99999));
    drop(running);
    // A different installer won while this candidate was downloading.
    meta.versions.insert("yt-dlp".into(), "2026.12.01".into());
    meta.ytdlp_last_check = Some(99999);
    std::fs::write(current.join(name), b"newer").unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(current.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    super::bundled_tools::record_repair(
      &current,
      "yt-dlp",
      &current.join(name),
      "2026.12.01",
      "verified-archive",
    )
    .unwrap();
    super::write_metadata(&meta_path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    assert_eq!(
      super::commit_ytdlp(&root, &staged, "2026.11.01", attempt, operation, processes)
        .await
        .unwrap(),
      InstallOutcome::UpToDate
    );
    let preserved: Metadata = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    assert_eq!(preserved.ytdlp_last_check, Some(99999));
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"newer");
    std::fs::remove_dir_all(root).unwrap();
  }

  #[tokio::test]
  async fn committed_bundle_merges_metadata_and_cleans_legacy_files() {
    use super::{InstallOutcome, CURRENT_DIR};
    use std::sync::Arc;
    let root = std::env::temp_dir().join(format!("nezzel-commit-{}", uuid::Uuid::new_v4()));
    let staged = root.join("staged");
    std::fs::create_dir_all(staged.join(".extract-empty")).unwrap();
    std::fs::create_dir_all(root.join("_internal")).unwrap();
    let name = &crate::binaries::bundled_tools::exe_name("yt-dlp");
    std::fs::write(root.join(name), b"old").unwrap();
    std::fs::write(staged.join(name), b"new").unwrap();
    let mut meta = Metadata::default();
    meta.versions.insert("yt-dlp".into(), "2026.10.26".into());
    meta.versions.insert("deno".into(), "2.6.0".into());
    meta.ytdlp_last_check = Some(100);
    let path = root.join("metadata.json");
    super::write_metadata(&path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    let result = super::commit_ytdlp(
      &root,
      &staged,
      "2026.11.01",
      None,
      Arc::new(tokio::sync::Mutex::new(())),
      Arc::new(tokio::sync::RwLock::new(())),
    )
    .await
    .unwrap();
    assert_eq!(result, InstallOutcome::Installed(Some("2026.10.26".into())));
    let committed: Metadata = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(committed.versions["yt-dlp"], "2026.11.01");
    assert_eq!(committed.versions["deno"], "2.6.0");
    assert_eq!(committed.ytdlp_last_check, Some(100));
    assert_eq!(
      std::fs::read(root.join(CURRENT_DIR).join(name)).unwrap(),
      b"new"
    );
    assert!(!root.join(name).exists());
    assert!(!root.join("_internal").exists());
    assert!(!root.join(CURRENT_DIR).join(".extract-empty").exists());
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn metadata_preserves_failed_check_time_and_reads_legacy_files() {
    let mut meta: Metadata =
      serde_json::from_slice(br#"{"versions":{"yt-dlp":"2026.10.26"},"is_locked":false}"#).unwrap();
    assert!(check_due(meta.ytdlp_last_check, 100));
    let root = std::env::temp_dir().join(format!("nezzel-check-time-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("metadata.json");
    // The attempt is committed before network work, which may subsequently fail.
    meta.ytdlp_last_check = Some(100);
    write_metadata(&path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    let reloaded: Metadata = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(!check_due(reloaded.ytdlp_last_check, 101));
    assert_eq!(reloaded.versions["yt-dlp"], "2026.10.26");
    assert!(!path.with_extension("json.tmp").exists());
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn cleanup_restores_a_lone_backup_and_drops_stale_ones() {
    let root = std::env::temp_dir().join(format!("nezzel-old-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let id = |n: u8| format!("00000000-0000-4000-8000-00000000000{n}");
    // Lone backup, canonical gone: restored.
    std::fs::write(root.join(format!("ffmpeg.old-{}", id(1))), b"kept").unwrap();
    // Canonical present: backups removed.
    std::fs::write(root.join("deno"), b"deno").unwrap();
    std::fs::write(root.join(format!("deno.old-{}", id(2))), b"stale").unwrap();
    // Two backups, canonical gone: the newest is restored.
    let older = root.join(format!("ffprobe.old-{}", id(3)));
    let newer = root.join(format!("ffprobe.old-{}", id(4)));
    std::fs::write(&older, b"a").unwrap();
    std::fs::write(&newer, b"b").unwrap();
    let now = std::time::SystemTime::now();
    std::fs::File::options()
      .write(true)
      .open(&older)
      .unwrap()
      .set_modified(now - std::time::Duration::from_secs(60))
      .unwrap();
    std::fs::File::options()
      .write(true)
      .open(&newer)
      .unwrap()
      .set_modified(now)
      .unwrap();
    // Not a backup name: untouched.
    std::fs::write(root.join("x.old-notes"), b"mine").unwrap();
    cleanup_leftovers(&root).unwrap();
    assert_eq!(std::fs::read(root.join("ffmpeg")).unwrap(), b"kept");
    assert_eq!(std::fs::read(root.join("deno")).unwrap(), b"deno");
    assert_eq!(std::fs::read(root.join("ffprobe")).unwrap(), b"b");
    assert!(root.join("x.old-notes").exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 4);
    std::fs::remove_dir_all(root).unwrap();
  }
}
