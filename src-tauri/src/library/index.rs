use crate::download_records::{safe_library_root, FileAuthority};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
  collections::HashSet,
  fs::{self, OpenOptions},
  io::{self, Write},
  path::{Path, PathBuf},
  time::UNIX_EPOCH,
};

const CAP: usize = 5000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRecord {
  pub id: String,
  pub paths: Vec<PathBuf>,
  pub url: String,
  pub title: String,
  pub site: Option<String>,
  pub kind: String,
  pub size: u64,
  pub completed_at: u64,
  pub thumbnail: Option<String>,
  #[serde(default)]
  pub authority: Vec<FileAuthority>,
  #[serde(default)]
  pub display: LibraryDisplay,
  #[serde(default)]
  pub error_code: Option<String>,
}

/// Display facts only: never persist cookie/header overrides or signed attachment URLs.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDisplay {
  pub duration: Option<f64>,
  pub height: Option<u32>,
  pub fps: Option<u32>,
  pub track_type: Option<crate::models::TrackType>,
  pub attachment_summary: Option<AttachmentSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentSummary {
  pub photos: u32,
  pub videos: u32,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Index {
  version: u32,
  pub records: Vec<LibraryRecord>,
  #[serde(default)]
  pub scanned: Vec<LibraryRecord>,
  #[serde(default)]
  excluded: HashSet<String>,
}
impl Default for Index {
  fn default() -> Self {
    Self {
      version: 1,
      records: Vec::new(),
      scanned: Vec::new(),
      excluded: HashSet::new(),
    }
  }
}

impl Index {
  pub fn load(path: &Path) -> io::Result<Self> {
    match fs::read(path) {
      Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
        Ok(mut index) if index.version == 1 => {
          // The WIP format stored scanned files alongside completions, without a source URL.
          let (scanned, completed): (Vec<_>, Vec<_>) = std::mem::take(&mut index.records)
            .into_iter()
            .partition(|row| row.url.is_empty() && row.authority.is_empty());
          index.records = completed;
          index.scanned.extend(scanned);
          index.sort_and_cap();
          Ok(index)
        }
        _ => {
          // Preserve the original before any subsequent write replaces it.
          let mut backup = path.with_extension("json.bak");
          let mut options = OpenOptions::new();
          options.write(true).create_new(true);
          #[cfg(unix)]
          {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
          }
          let mut file = match options.open(&backup) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
              let time = std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
              backup = path.with_extension(format!("json.{time}-{}.bak", uuid::Uuid::new_v4()));
              options.open(backup)?
            }
            Err(e) => return Err(e),
          };
          file.write_all(&bytes)?;
          file.sync_all()?;
          Ok(Self::default())
        }
      },
      Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
      Err(e) => Err(e),
    }
  }

  pub fn save(&self, path: &Path) -> io::Result<()> {
    let parent = path
      .parent()
      .ok_or_else(|| io::Error::other("invalid library path"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".library-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
      let mut options = OpenOptions::new();
      options.write(true).create_new(true);
      #[cfg(unix)]
      {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
      }
      let mut file = options.open(&temporary)?;
      file.write_all(&serde_json::to_vec(self)?)?;
      file.sync_all()?;
      fs::rename(&temporary, path)?;
      #[cfg(unix)]
      fs::File::open(parent)?.sync_all()?;
      Ok(())
    })();
    if result.is_err() {
      let _ = fs::remove_file(temporary);
    }
    result
  }

  fn sort_and_cap(&mut self) {
    self
      .records
      .sort_by_key(|r| std::cmp::Reverse(r.completed_at));
    let (mut saved, mut failed) = (0, 0);
    self.records.retain(|row| {
      let (count, cap) = if row.error_code.is_some() {
        (&mut failed, 500)
      } else {
        (&mut saved, CAP)
      };
      *count += 1;
      *count <= cap
    });
    self
      .scanned
      .sort_by_key(|r| std::cmp::Reverse(r.completed_at));
    self.scanned.truncate(2000);
  }

  pub fn append(&mut self, mut record: LibraryRecord) {
    if record.error_code.is_some()
      && self
        .records
        .iter()
        .any(|row| row.id == record.id && row.error_code.is_none())
    {
      return;
    }
    // A second completed output of the same live item still owns its earlier files.
    // Give the new row a distinct UI identity when paths do not overlap.
    if record.error_code.is_none()
      && self.records.iter().any(|row| {
        row.id == record.id
          && row.error_code.is_none()
          && !row
            .paths
            .iter()
            .any(|p| record.paths.iter().any(|q| path_key(p) == path_key(q)))
      })
    {
      record.id = uuid::Uuid::new_v4().to_string();
    }
    for path in &record.paths {
      self.excluded.remove(&canonical_key(path));
    }
    self.scanned.retain(|r| {
      !r.paths
        .iter()
        .any(|p| record.paths.iter().any(|q| path_key(p) == path_key(q)))
    });
    self.records.retain(|r| {
      r.id != record.id && {
        !r.paths
          .iter()
          .any(|p| record.paths.iter().any(|q| path_key(p) == path_key(q)))
      }
    });
    self.records.push(record);
    self.sort_and_cap();
  }

  pub fn prune(&mut self) {
    for record in self.records.iter_mut().chain(&mut self.scanned) {
      record.paths.retain(|p| !missing_from_available_folder(p));
      record
        .authority
        .retain(|file| record.paths.contains(&file.path));
    }
    self
      .records
      .retain(|r| r.error_code.is_some() || !r.paths.is_empty());
    self.scanned.retain(|r| !r.paths.is_empty());
  }

  pub fn remove(&mut self, paths: &[PathBuf], scan_root: Option<&Path>) {
    let keys: HashSet<_> = paths.iter().map(|p| canonical_key(p)).collect();
    if let Some(root) = scan_root {
      self.excluded.extend(
        paths
          .iter()
          .filter(|p| under_root(p, root))
          .map(|p| canonical_key(p)),
      );
    }
    for record in self.records.iter_mut().chain(&mut self.scanned) {
      record.paths.retain(|p| !keys.contains(&canonical_key(p)));
      record
        .authority
        .retain(|file| !keys.contains(&canonical_key(&file.path)));
    }
    self
      .records
      .retain(|r| r.error_code.is_some() || !r.paths.is_empty());
    self.scanned.retain(|r| !r.paths.is_empty());
    self.prune_excluded(scan_root);
  }

  pub fn prune_excluded(&mut self, root: Option<&Path>) {
    let Some(root) = root else {
      return;
    };
    self
      .excluded
      .retain(|p| under_root(Path::new(p), root) && !missing_from_available_folder(Path::new(p)));
    if self.excluded.len() > 2000 {
      let mut paths: Vec<_> = self.excluded.drain().collect();
      paths.sort();
      paths.truncate(2000);
      self.excluded.extend(paths);
    }
  }

  pub fn merge_scan(&mut self, scanned: Vec<LibraryRecord>) {
    let mut known: HashSet<_> = self
      .records
      .iter()
      .chain(&self.scanned)
      .flat_map(|r| r.paths.iter().map(|p| canonical_key(p)))
      .collect();
    for mut record in scanned {
      record
        .paths
        .retain(|p| !self.excluded.contains(&canonical_key(p)) && known.insert(canonical_key(p)));
      if !record.paths.is_empty() {
        self.scanned.push(record);
      }
    }
    self.sort_and_cap();
  }

  pub fn rows(&self) -> Vec<LibraryRecord> {
    let mut rows: Vec<_> = self.records.iter().chain(&self.scanned).cloned().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.completed_at));
    rows
  }
}

// An unavailable parent can be an unmounted drive. Only prune a known absent file in an available folder.
fn missing_from_available_folder(path: &Path) -> bool {
  match fs::symlink_metadata(path) {
    Ok(meta) if meta.file_type().is_file() => false,
    Ok(_) => path.parent().is_some_and(Path::is_dir),
    Err(e) if e.kind() == io::ErrorKind::NotFound => path.parent().is_some_and(Path::is_dir),
    Err(_) => false,
  }
}

fn under_root(path: &Path, root: &Path) -> bool {
  canonical_key(path).starts_with(&(canonical_key(root).trim_end_matches('/').to_owned() + "/"))
}

fn canonical_key(path: &Path) -> String {
  let canonical = path
    .canonicalize()
    .ok()
    .or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?)));
  path_key(canonical.as_deref().unwrap_or(path))
}

// Completion and scan paths are already canonical; lexical comparisons must not stat every prior row.
fn path_key(path: &Path) -> String {
  let raw = path.to_string_lossy();
  let normalized = raw.replace('\\', "/");
  let normalized = normalized
    .strip_prefix("//?/UNC/")
    .map(|s| format!("//{s}"))
    .unwrap_or_else(|| {
      normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_owned()
    });
  if normalized.starts_with("//")
    || (normalized.as_bytes().get(1) == Some(&b':') && normalized.as_bytes().get(2) == Some(&b'/'))
  {
    normalized.to_lowercase()
  } else {
    raw.into_owned()
  }
}

pub(super) fn regular_file(path: &Path) -> Option<fs::Metadata> {
  if !path.is_absolute() {
    return None;
  }
  fs::symlink_metadata(path)
    .ok()
    .filter(|m| m.file_type().is_file())
}

pub(super) fn kind(path: &Path) -> &'static str {
  match path
    .extension()
    .and_then(|e| e.to_str())
    .unwrap_or("")
    .to_ascii_lowercase()
    .as_str()
  {
    "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "flv" | "ts" => "video",
    "mp3" | "m4a" | "aac" | "opus" | "ogg" | "flac" | "wav" | "wma" => "audio",
    "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "heic" | "bmp" | "tiff" => "image",
    _ => "file",
  }
}

fn temporary(name: &str) -> bool {
  let name = name.to_ascii_lowercase();
  let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
  name.starts_with('.')
    || name.ends_with('~')
    || matches!(
      extension,
      "part"
        | "ytdl"
        | "aria2"
        | "tmp"
        | "temp"
        | "crdownload"
        | "download"
        | "nezzel-part"
        | "nezzel-state"
    )
    || extension
      .strip_prefix("part-frag")
      .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub(super) fn scan_folder(
  root: &Path,
  home: Option<&Path>,
  downloads: Option<&Path>,
) -> Vec<LibraryRecord> {
  let Some(root) = safe_library_root(root, home, downloads) else {
    return Vec::new();
  };
  let mut pending = vec![(root, 0)];
  let mut result = Vec::new();
  let mut visited = 0;
  while let Some((dir, depth)) = pending.pop() {
    let Ok(entries) = fs::read_dir(dir) else {
      continue;
    };
    for entry in entries.flatten() {
      visited += 1;
      if visited > 20000 || result.len() >= 2000 {
        return result;
      }
      let name = entry.file_name().to_string_lossy().into_owned();
      if temporary(&name) {
        continue;
      }
      let Ok(file_type) = entry.file_type() else {
        continue;
      };
      if file_type.is_symlink() {
        continue;
      }
      let Ok(meta) = entry.metadata() else {
        continue;
      };
      #[cfg(windows)]
      {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x2 != 0 {
          continue;
        }
      }
      if file_type.is_dir() && depth < 3 {
        pending.push((entry.path(), depth + 1));
      }
      if !file_type.is_file() {
        continue;
      }
      let path = entry.path();
      // Existing UUID scan IDs are kept; merge_scan deduplicates by canonical path.
      result.push(LibraryRecord {
        id: format!(
          "scan-{}",
          hex::encode(Sha256::digest(canonical_key(&path).as_bytes()))
        ),
        kind: kind(&path).into(),
        paths: vec![path],
        url: String::new(),
        title: name,
        site: None,
        size: meta.len(),
        completed_at: meta
          .modified()
          .ok()
          .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
          .map(|d| d.as_millis() as u64)
          .unwrap_or(0),
        thumbnail: None,
        authority: Vec::new(),
        display: LibraryDisplay::default(),
        error_code: None,
      });
    }
  }
  result
}
