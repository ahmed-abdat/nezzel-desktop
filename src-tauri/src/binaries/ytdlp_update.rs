//! Release input validation and a rollback-safe, whole-bundle install transaction.
use std::{
  cmp::Ordering,
  io,
  path::{Path, PathBuf},
};

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

// Run once before this manager starts any archive preparation, under both locks.
pub(super) fn cleanup_leftovers(bin: &Path) -> io::Result<()> {
  for entry in std::fs::read_dir(bin)? {
    let entry = entry?;
    let name = entry.file_name();
    let name = name.to_string_lossy();
    if name.starts_with(".yt-dlp-stage-") || name.starts_with(".extract-") {
      remove_path(&entry.path())?;
    }
  }
  Ok(())
}

pub(super) fn cleanup_legacy(bin: &Path) -> io::Result<()> {
  let name = if cfg!(windows) {
    "yt-dlp.exe"
  } else {
    "yt-dlp"
  };
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

pub(crate) fn executable(bin: &Path) -> PathBuf {
  let name = if cfg!(windows) {
    "yt-dlp.exe"
  } else {
    "yt-dlp"
  };
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

#[cfg(test)]
mod tests {
  use super::*;

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
    let name = if cfg!(windows) {
      "yt-dlp.exe"
    } else {
      "yt-dlp"
    };
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
    let name = if cfg!(windows) {
      "yt-dlp.exe"
    } else {
      "yt-dlp"
    };
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
    let binary = if cfg!(windows) {
      "yt-dlp.exe"
    } else {
      "yt-dlp"
    };
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
}
