//! Download records: which files each download group produced, and the authority to open or trash them.
//! Lives below `commands` so the library index and the runners can use it without importing commands.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Which file a path pointed at when the download finished: device + inode on unix, size +
/// modification time elsewhere. A file replaced or re-created under the same name no longer matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity(u64, u64);

#[cfg(unix)]
fn identity_of(meta: &Metadata) -> FileIdentity {
  use std::os::unix::fs::MetadataExt;
  FileIdentity(meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn identity_of(meta: &Metadata) -> FileIdentity {
  let modified = meta
    .modified()
    .ok()
    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
    .map(|elapsed| elapsed.as_nanos() as u64)
    .unwrap_or(0);
  FileIdentity(meta.len(), modified)
}

/// Identity of a regular file at `path`, without following symlinks.
pub(crate) fn regular_file_identity(path: &Path) -> Option<FileIdentity> {
  std::fs::symlink_metadata(path)
    .ok()
    .filter(|meta| meta.file_type().is_file())
    .map(|meta| identity_of(&meta))
}

/// Deletion authority captured from a completed engine write, never reconstructed from a file's parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAuthority {
  pub path: PathBuf,
  pub root: PathBuf,
  pub identity: FileIdentity,
}

/// Both scans and restored deletion grants use the same policy for broad or unavailable roots.
pub(crate) fn safe_library_root(
  root: &Path,
  home: Option<&Path>,
  downloads: Option<&Path>,
) -> Option<PathBuf> {
  if !root.is_absolute() {
    return None;
  }
  let root = root.canonicalize().ok()?;
  if root.parent().is_none()
    || home.into_iter().chain(downloads).any(|p| {
      p.canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .starts_with(&root)
    })
  {
    return None;
  }
  Some(root)
}

/// A scan has no completion receipt. Its only possible root is the configured owned folder.
pub(crate) fn scanned_files(
  paths: &[PathBuf],
  root: Option<&Path>,
  home: Option<&Path>,
  downloads: Option<&Path>,
) -> Vec<FileAuthority> {
  let Some(root) = root.and_then(|root| safe_library_root(root, home, downloads)) else {
    return Vec::new();
  };
  paths
    .iter()
    .filter_map(|path| {
      let identity = regular_file_identity(path)?;
      let path = path.canonicalize().ok()?;
      path.starts_with(&root).then(|| FileAuthority {
        path,
        root: root.clone(),
        identity,
      })
    })
    .collect()
}

/// Snapshot each item's already sealed files and the engine-authorized root that contains it.
pub(crate) fn recorded_files(record: &GroupRecord, item_id: &str) -> Vec<FileAuthority> {
  let mut files: Vec<_> = record
    .item_files
    .get(item_id)
    .into_iter()
    .flatten()
    .filter_map(|path| {
      let target = resolve_trash_target(record, path).ok()?;
      let root = record
        .roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .filter(|r| target.path.starts_with(r))
        .max_by_key(|r| r.components().count())?;
      Some(FileAuthority {
        path: target.path,
        root,
        identity: target.identity,
      })
    })
    .collect();
  files.sort_by(|a, b| a.path.cmp(&b.path));
  files
}

/// Per download group: the folder it was told to save into, and the files yt-dlp reported writing
/// (with their identity once the download has finished).
#[derive(Default, Clone)]
pub struct GroupRecord {
  pub roots: HashSet<PathBuf>,
  pub item_files: HashMap<String, HashSet<PathBuf>>,
  pub files: HashMap<PathBuf, Option<FileIdentity>>,
}

/// What "Delete file" may touch. History rows restore their original sealed receipts.
#[derive(Default)]
pub struct DownloadRecords(Mutex<HashMap<String, GroupRecord>>);

impl DownloadRecords {
  /// Set from our own arguments when the download starts (trusted).
  pub fn allow_root(&self, group_id: &str, root: PathBuf) {
    if let Ok(mut map) = self.0.lock() {
      map
        .entry(group_id.to_string())
        .or_default()
        .roots
        .insert(root);
    }
  }

  /// From yt-dlp output (untrusted text). The file usually doesn't exist yet: its identity is taken
  /// in `seal` when the download succeeds, and it must also sit inside an allowed root.
  pub fn record_item(&self, group_id: &str, item_id: &str, path: &str) {
    if path.is_empty() {
      return;
    }
    if let Ok(mut map) = self.0.lock() {
      let record = map.entry(group_id.to_owned()).or_default();
      let path = PathBuf::from(path);
      record.files.insert(path.clone(), None);
      record
        .item_files
        .entry(item_id.to_owned())
        .or_default()
        .insert(path);
    }
  }

  /// The download exited successfully: remember which file each reported path is now.
  pub fn seal(&self, group_id: &str) {
    if let Ok(mut map) = self.0.lock() {
      if let Some(record) = map.get_mut(group_id) {
        for (path, identity) in record.files.iter_mut() {
          *identity = regular_file_identity(path);
        }
      }
    }
  }

  /// Restore the saved receipt unchanged: restarting must not authorize a replacement file.
  pub fn restore_history(
    &self,
    group_id: &str,
    files: &[FileAuthority],
    home: Option<&Path>,
    downloads: Option<&Path>,
  ) {
    if let Ok(mut map) = self.0.lock() {
      map.entry(group_id.to_owned()).or_insert_with(|| {
        let mut record = GroupRecord::default();
        for file in files {
          if let Some(root) =
            safe_library_root(&file.root, home, downloads).filter(|root| *root == file.root)
          {
            record.roots.insert(root);
            record
              .files
              .insert(file.path.clone(), Some(file.identity.clone()));
          }
        }
        record
      });
    }
  }

  pub fn forget(&self, group_id: &str) {
    if let Ok(mut map) = self.0.lock() {
      map.remove(group_id);
    }
  }

  pub(crate) fn get(&self, group_id: &str) -> GroupRecord {
    self
      .0
      .lock()
      .ok()
      .and_then(|map| map.get(group_id).cloned())
      .unwrap_or_default()
  }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TrashRefusal {
  NotAbsolute,
  NotRecorded,
  Replaced,
  OutsideDownloadFolder,
  NotARegularFile,
  Missing,
}

impl fmt::Display for TrashRefusal {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let text = match self {
      TrashRefusal::NotAbsolute => "path is not absolute",
      TrashRefusal::NotRecorded => "path is not a file this download saved",
      TrashRefusal::Replaced => "file changed since the download finished",
      TrashRefusal::OutsideDownloadFolder => "path is outside this download's folder",
      TrashRefusal::NotARegularFile => "path is not a regular file",
      TrashRefusal::Missing => "file no longer exists",
    };
    f.write_str(text)
  }
}

impl TrashRefusal {
  /// Stable code the UI maps to a message.
  pub fn code(&self) -> &'static str {
    match self {
      TrashRefusal::NotAbsolute
      | TrashRefusal::NotRecorded
      | TrashRefusal::OutsideDownloadFolder => "notOurs",
      TrashRefusal::Replaced | TrashRefusal::NotARegularFile => "changed",
      TrashRefusal::Missing => "missing",
    }
  }
}

/// A file that may be moved to the Trash, with the identity it must still have when it is moved.
#[derive(Debug, PartialEq, Eq)]
pub struct TrashTarget {
  pub path: PathBuf,
  pub identity: FileIdentity,
}

/// Resolves `requested` to a file that may be moved to the Trash: absolute, a regular file (not a
/// symlink or folder), the same file (canonical path and identity) as one this group's finished
/// download reported, and inside the folder the download was told to save into. So `..` segments,
/// a swapped-in symlink, a replaced file or a path smuggled into yt-dlp's output are all refused.
pub fn resolve_trash_target(
  record: &GroupRecord,
  requested: &Path,
) -> Result<TrashTarget, TrashRefusal> {
  if !requested.is_absolute() {
    return Err(TrashRefusal::NotAbsolute);
  }
  let meta = std::fs::symlink_metadata(requested).map_err(|_| TrashRefusal::Missing)?;
  if !meta.file_type().is_file() {
    return Err(TrashRefusal::NotARegularFile);
  }
  let identity = identity_of(&meta);
  let target = requested
    .canonicalize()
    .map_err(|_| TrashRefusal::Missing)?;
  let sealed = record
    .files
    .iter()
    .filter(|(path, _)| path.canonicalize().map(|p| p == target).unwrap_or(false))
    .map(|(_, sealed)| sealed.clone())
    .collect::<Vec<_>>();
  if sealed.is_empty() {
    return Err(TrashRefusal::NotRecorded);
  }
  if !sealed
    .iter()
    .any(|sealed| sealed.as_ref() == Some(&identity))
  {
    return Err(TrashRefusal::Replaced);
  }
  let inside_root = record.roots.iter().any(|root| {
    root
      .canonicalize()
      .map(|root| target.starts_with(root))
      .unwrap_or(false)
  });
  if !inside_root {
    return Err(TrashRefusal::OutsideDownloadFolder);
  }
  Ok(TrashTarget {
    path: target,
    identity,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;

  fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nezzel-trash-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("downloads")).unwrap();
    dir
  }

  /// A finished download: root allowed, files reported, then sealed.
  fn finished(root: &Path, files: &[&Path]) -> GroupRecord {
    let records = DownloadRecords::default();
    records.allow_root("g", root.to_path_buf());
    for file in files {
      records.record_item("g", "item", file.to_str().unwrap());
    }
    records.seal("g");
    records.get("g")
  }

  fn ok(path: &Path) -> Result<PathBuf, TrashRefusal> {
    Ok(path.canonicalize().unwrap())
  }

  fn resolve(record: &GroupRecord, path: &Path) -> Result<PathBuf, TrashRefusal> {
    resolve_trash_target(record, path).map(|target| target.path)
  }

  #[test]
  fn accepts_a_finished_file_inside_the_download_folder() {
    let dir = sandbox("ok");
    let root = dir.join("downloads");
    fs::create_dir_all(root.join("Channel")).unwrap();
    let file = root.join("Channel/clip.mp4");
    fs::write(&file, b"x").unwrap();
    assert_eq!(resolve(&finished(&root, &[&file]), &file), ok(&file));
  }

  #[test]
  fn refuses_everything_else() {
    let dir = sandbox("refuse");
    let root = dir.join("downloads");
    let clip = root.join("clip.mp4");
    let other = root.join("other.txt");
    let outside = dir.join("secret.txt");
    for f in [&clip, &other, &outside] {
      fs::write(f, b"x").unwrap();
    }
    fs::create_dir_all(root.join("sub")).unwrap();
    let rec = finished(&root, &[&clip, &root.join("sub"), &outside]);

    let cases: Vec<(PathBuf, TrashRefusal)> = vec![
      (other.clone(), TrashRefusal::NotRecorded),
      (PathBuf::from("clip.mp4"), TrashRefusal::NotAbsolute),
      (root.join("sub/../other.txt"), TrashRefusal::NotRecorded),
      (root.join("sub"), TrashRefusal::NotARegularFile),
      (root.join("gone.mp4"), TrashRefusal::Missing),
      // yt-dlp output named a file outside the folder we told it to use (e.g. a crafted
      // "Destination:" line): reported, but still refused.
      (outside.clone(), TrashRefusal::OutsideDownloadFolder),
    ];
    for (path, refusal) in cases {
      assert_eq!(resolve(&rec, &path), Err(refusal), "{}", path.display());
    }
    // No record for the group at all.
    assert_eq!(
      resolve(&GroupRecord::default(), &clip),
      Err(TrashRefusal::NotRecorded)
    );
  }

  #[test]
  fn refuses_a_download_that_never_finished() {
    let dir = sandbox("unsealed");
    let root = dir.join("downloads");
    let clip = root.join("clip.mp4");
    fs::write(&clip, b"x").unwrap();
    let records = DownloadRecords::default();
    records.allow_root("g", root.clone());
    records.record_item("g", "item", clip.to_str().unwrap());
    assert_eq!(
      resolve(&records.get("g"), &clip),
      Err(TrashRefusal::Replaced)
    );
  }

  #[test]
  fn refuses_a_file_replaced_under_the_same_name() {
    let dir = sandbox("replaced");
    let root = dir.join("downloads");
    let clip = root.join("clip.mp4");
    fs::write(&clip, b"x").unwrap();
    let rec = finished(&root, &[&clip]);
    // New file written while the old one still exists, then renamed over it: different identity.
    let newer = root.join("newer.tmp");
    fs::write(&newer, b"other content").unwrap();
    fs::rename(&newer, &clip).unwrap();
    assert_eq!(resolve(&rec, &clip), Err(TrashRefusal::Replaced));
  }

  #[cfg(unix)]
  #[test]
  fn refuses_symlinks() {
    let dir = sandbox("link");
    let root = dir.join("downloads");
    let outside = dir.join("outside.txt");
    fs::write(&outside, b"x").unwrap();
    let link = root.join("clip.mp4");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let rec = finished(&root, &[&link]);
    assert_eq!(resolve(&rec, &link), Err(TrashRefusal::NotARegularFile));
    // The link's target matches by canonical path, but the link was never a regular file we sealed.
    assert_eq!(resolve(&rec, &outside), Err(TrashRefusal::Replaced));
  }

  #[test]
  fn restored_history_refuses_files_replaced_between_sessions() {
    let dir = sandbox("history");
    let root = dir.join("downloads");
    let clip = root.join("clip.mp4");
    fs::write(&clip, b"first").unwrap();
    let original = finished(&root, &[&clip]);
    let saved = serde_json::to_vec(&recorded_files(&original, "item")).unwrap();
    let saved: Vec<FileAuthority> = serde_json::from_slice(&saved).unwrap();
    let restored = DownloadRecords::default();
    restored.restore_history("library:g", &saved, None, None);
    assert_eq!(resolve(&restored.get("library:g"), &clip), ok(&clip));
    let newer = root.join("newer.tmp");
    fs::write(&newer, b"replacement").unwrap();
    fs::rename(&newer, &clip).unwrap();
    let next_session = DownloadRecords::default();
    next_session.restore_history("library:g", &saved, None, None);
    assert_eq!(
      resolve(&next_session.get("library:g"), &clip),
      Err(TrashRefusal::Replaced)
    );
  }

  #[test]
  fn restored_history_uses_the_persisted_download_root_and_rejects_broad_roots() {
    let dir = sandbox("history-roots");
    let home = dir.join("home");
    let downloads = home.join("Downloads");
    let owned = downloads.join("Nezzel");
    fs::create_dir_all(&owned).unwrap();
    let outside = dir.join("secret.mp4");
    fs::write(&outside, b"private").unwrap();
    let saved = FileAuthority {
      path: outside.clone(),
      root: owned.canonicalize().unwrap(),
      identity: regular_file_identity(&outside).unwrap(),
    };
    let records = DownloadRecords::default();
    records.restore_history(
      "g",
      std::slice::from_ref(&saved),
      Some(&home),
      Some(&downloads),
    );
    assert_eq!(
      resolve(&records.get("g"), &outside),
      Err(TrashRefusal::OutsideDownloadFolder)
    );
    for (i, root) in [&home, &downloads, &dir, Path::new("/")]
      .into_iter()
      .enumerate()
    {
      let id = format!("broad-{i}");
      let saved = FileAuthority {
        root: root.to_path_buf(),
        ..saved.clone()
      };
      records.restore_history(&id, &[saved], Some(&home), Some(&downloads));
      assert!(records.get(&id).roots.is_empty());
      assert!(resolve(&records.get(&id), &outside).is_err());
    }
  }

  #[test]
  fn scanned_files_get_authority_only_inside_the_safe_configured_root() {
    let dir = sandbox("scan-roots");
    let home = dir.join("home");
    let downloads = home.join("Downloads");
    let owned = downloads.join("Nezzel");
    fs::create_dir_all(owned.join("sub")).unwrap();
    let clip = owned.join("sub/clip.mp4");
    fs::write(&clip, b"clip").unwrap();
    let files = scanned_files(
      std::slice::from_ref(&clip),
      Some(&owned),
      Some(&home),
      Some(&downloads),
    );
    assert_eq!(files[0].root, owned.canonicalize().unwrap());
    let records = DownloadRecords::default();
    records.restore_history("g", &files, Some(&home), Some(&downloads));
    assert_eq!(resolve(&records.get("g"), &clip), ok(&clip));
    for root in [Some(home.as_path()), Some(downloads.as_path()), None] {
      assert!(scanned_files(
        std::slice::from_ref(&clip),
        root,
        Some(&home),
        Some(&downloads)
      )
      .is_empty());
    }
  }

  #[test]
  fn records_are_per_group_and_forgettable() {
    let records = DownloadRecords::default();
    records.allow_root("g1", PathBuf::from("/a"));
    records.record_item("g1", "item", "/a/b.mp4");
    records.record_item("g1", "item", "");
    assert_eq!(records.get("g1").files.len(), 1);
    assert_eq!(records.get("g1").roots.len(), 1);
    assert!(records.get("g2").files.is_empty());
    records.forget("g1");
    assert!(records.get("g1").files.is_empty());
  }
}
