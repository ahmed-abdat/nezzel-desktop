//! Installer tools are sealed by the app signature; writable repairs carry file hashes.
use super::ytdlp_update::compare_versions;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
  cmp::Ordering,
  collections::HashMap,
  io,
  path::{Component, Path, PathBuf},
};

pub(super) const TOOLS: [&str; 4] = ["yt-dlp", "ffmpeg", "ffprobe", "deno"];
const RECEIPT: &str = ".nezzel-integrity.json";

#[derive(Deserialize, Serialize)]
struct Receipt {
  version: String,
  archive_sha256: String,
  files: HashMap<String, String>,
}

pub(super) fn executable(path: &Path) -> bool {
  let Ok(meta) = std::fs::metadata(path) else {
    return false;
  };
  if !meta.is_file() {
    return false;
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
  }
  #[cfg(not(unix))]
  {
    true
  }
}

pub(super) fn choose_path(
  tool: &str,
  bundled: Option<PathBuf>,
  pinned: &str,
  data: Option<PathBuf>,
  data_version: Option<&str>,
) -> Option<PathBuf> {
  let data_eligible = data_version.is_some_and(|version| {
    if tool == "yt-dlp" {
      compare_versions(version, pinned).is_some_and(|order| {
        if bundled.is_some() {
          order == Ordering::Greater
        } else {
          order != Ordering::Less
        }
      })
    } else {
      version == pinned
    }
  });
  if data_eligible && (tool == "yt-dlp" || bundled.is_none()) {
    data.or(bundled)
  } else {
    bundled
  }
}

fn safe_relative(path: &str) -> bool {
  !path.is_empty()
    && !path.contains(['\\', ':'])
    && Path::new(path)
      .components()
      .all(|part| matches!(part, Component::Normal(_)))
}

fn hash_file(path: &Path) -> io::Result<String> {
  let mut input = std::fs::File::open(path)?;
  let mut digest = Sha256::new();
  use std::io::Read;
  let mut buffer = [0u8; 64 * 1024];
  loop {
    let count = input.read(&mut buffer)?;
    if count == 0 {
      break;
    }
    digest.update(&buffer[..count]);
  }
  Ok(hex::encode(digest.finalize()))
}

pub(super) fn file_hashes(root: &Path) -> io::Result<HashMap<String, String>> {
  fn walk(root: &Path, dir: &Path, files: &mut HashMap<String, String>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
      let entry = entry?;
      if entry.file_type()?.is_symlink() {
        return Err(io::Error::other("tool bundle contains symlink"));
      }
      if entry.file_type()?.is_dir() {
        walk(root, &entry.path(), files)?;
      } else if entry.file_name() != RECEIPT {
        let name = entry
          .path()
          .strip_prefix(root)
          .map_err(io::Error::other)?
          .to_string_lossy()
          .replace('\\', "/");
        files.insert(name, hash_file(&entry.path())?);
      }
    }
    Ok(())
  }
  let mut files = HashMap::new();
  walk(root, root, &mut files)?;
  Ok(files)
}

fn verify_files(root: &Path, files: &HashMap<String, String>) -> bool {
  !files.is_empty()
    && files.iter().all(|(name, expected)| {
      safe_relative(name)
        && hash_file(&root.join(name)).is_ok_and(|actual| actual.eq_ignore_ascii_case(expected))
    })
}

pub(super) fn record_repair(
  root: &Path,
  tool: &str,
  entry: &Path,
  version: &str,
  archive_sha256: &str,
) -> io::Result<()> {
  let files = if tool == "yt-dlp" {
    file_hashes(root)?
  } else {
    HashMap::from([(
      entry
        .file_name()
        .ok_or_else(|| io::Error::other("missing executable name"))?
        .to_string_lossy()
        .into_owned(),
      hash_file(entry)?,
    )])
  };
  let receipt = Receipt {
    version: version.into(),
    archive_sha256: archive_sha256.into(),
    files,
  };
  let path = receipt_path(root, tool);
  let temporary = path.with_extension("json.tmp");
  std::fs::write(&temporary, serde_json::to_vec(&receipt)?)?;
  std::fs::rename(temporary, path)
}

fn receipt_path(root: &Path, tool: &str) -> PathBuf {
  root.join(if tool == "yt-dlp" {
    RECEIPT.into()
  } else {
    format!(".{tool}-integrity.json")
  })
}

pub(super) fn verified_repair(
  entry: &Path,
  tool: &str,
  version: &str,
  pinned_sha: &str,
  pinned_version: &str,
) -> bool {
  let Some(root) = entry.parent() else {
    return false;
  };
  let Ok(bytes) = std::fs::read(receipt_path(root, tool)) else {
    return false;
  };
  let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) else {
    return false;
  };
  executable(entry) && receipt.version == version &&
    // Newer yt-dlp receipts come from the existing upstream checksum-verified updater.
    (version != pinned_version || receipt.archive_sha256 == pinned_sha) &&
    receipt.files.contains_key(&entry.file_name().unwrap_or_default().to_string_lossy().into_owned()) &&
    verify_files(root, &receipt.files)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolution_order_requires_verified_newer_ytdlp() {
    let bundled = PathBuf::from("bundled/yt-dlp");
    let data = PathBuf::from("data/yt-dlp");
    for (tool, bundled_ok, data_version, data_ok, expected) in [
      ("yt-dlp", true, None, false, Some(&bundled)),
      ("yt-dlp", true, Some("2026.11.01"), true, Some(&data)),
      ("yt-dlp", true, Some("2026.09.01"), true, Some(&bundled)),
      ("yt-dlp", true, Some("2026.10.26"), true, Some(&bundled)),
      ("yt-dlp", true, Some("2026.11.01"), false, Some(&bundled)),
      ("yt-dlp", false, None, false, None),
      ("yt-dlp", false, Some("2026.10.26"), true, Some(&data)),
      ("deno", true, Some("2026.11.01"), true, Some(&bundled)),
      ("deno", false, Some("2026.10.26"), true, Some(&data)),
    ] {
      assert_eq!(
        choose_path(
          tool,
          bundled_ok.then_some(bundled.clone()),
          "2026.10.26",
          data_ok.then_some(data.clone()),
          data_version
        ),
        expected.cloned(),
        "{tool}: bundled={bundled_ok}, version={data_version:?}, verified={data_ok}"
      );
    }
  }

  #[test]
  fn repair_receipt_binds_version_and_archive_and_rejects_changed_bytes() {
    let root = std::env::temp_dir().join(format!("nezzel-repair-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join(format!("yt-dlp{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&entry, b"verified executable").unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    record_repair(&root, "yt-dlp", &entry, "2026.10.26", "pinned-sha").unwrap();
    assert!(verified_repair(
      &entry,
      "yt-dlp",
      "2026.10.26",
      "pinned-sha",
      "2026.10.26"
    ));
    assert!(!verified_repair(
      &entry,
      "yt-dlp",
      "2026.10.26",
      "other-archive",
      "2026.10.26"
    ));
    assert!(!verified_repair(
      &entry,
      "yt-dlp",
      "2026.11.01",
      "pinned-sha",
      "2026.10.26"
    ));
    std::fs::write(&entry, b"corrupted executable").unwrap();
    assert!(!verified_repair(
      &entry,
      "yt-dlp",
      "2026.10.26",
      "pinned-sha",
      "2026.10.26"
    ));
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn integrity_checks_dependencies_and_rejects_sha_mismatch_or_missing_files() {
    let root = std::env::temp_dir().join(format!("nezzel-bundle-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("_internal")).unwrap();
    std::fs::write(root.join("yt-dlp"), b"executable").unwrap();
    std::fs::write(root.join("_internal/python"), b"library").unwrap();
    let files = file_hashes(&root).unwrap();
    assert!(verify_files(&root, &files));
    std::fs::write(root.join("_internal/python"), b"corrupt").unwrap();
    assert!(!verify_files(&root, &files));
    std::fs::remove_file(root.join("_internal/python")).unwrap();
    assert!(!verify_files(&root, &files));
    std::fs::remove_dir_all(root).unwrap();
  }
}
