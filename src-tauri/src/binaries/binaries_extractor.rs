use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use bzip2::read::BzDecoder;
use tar::{Archive as TarArchive, EntryType as TarEntryType};
use tokio::task::JoinError;
use zip::ZipArchive;

#[derive(Debug)]
pub enum ExtractError {
  Io(io::Error),
  Zip(zip::result::ZipError),
  Join(JoinError),
  EntryNotFound {
    wanted: String,
    available: EntriesDisplay,
  },
  AmbiguousArchive(EntriesDisplay),
  PathTraversal(String),
  UnsupportedEntry(String),
}

#[derive(Clone, Debug)]
pub struct EntriesDisplay(pub Vec<String>);

impl fmt::Display for EntriesDisplay {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    const MAX: usize = 20;
    if self.0.len() > MAX {
      write!(f, "{:?} (and {} more…)", &self.0[..MAX], self.0.len() - MAX)
    } else {
      write!(f, "{:?}", self.0)
    }
  }
}

impl fmt::Display for ExtractError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Io(e) => write!(f, "I/O error: {e}"),
      Self::Zip(e) => write!(f, "zip error: {e}"),
      Self::Join(e) => write!(f, "task join error: {e}"),
      Self::EntryNotFound { wanted, available } => {
        write!(f, "entry '{wanted}' not found. available: {available}")
      }
      Self::AmbiguousArchive(available) => {
        write!(
          f,
          "no entry provided and archive has multiple files. available: {available}"
        )
      }
      Self::PathTraversal(p) => write!(f, "archive contains path traversal: {p}"),
      Self::UnsupportedEntry(p) => write!(f, "unsupported archive entry: {p}"),
    }
  }
}

impl Error for ExtractError {
  fn source(&self) -> Option<&(dyn Error + 'static)> {
    match self {
      Self::Io(e) => Some(e),
      Self::Zip(e) => Some(e),
      Self::Join(e) => Some(e),
      _ => None,
    }
  }
}

impl From<io::Error> for ExtractError {
  fn from(e: io::Error) -> Self {
    Self::Io(e)
  }
}
impl From<zip::result::ZipError> for ExtractError {
  fn from(e: zip::result::ZipError) -> Self {
    Self::Zip(e)
  }
}
impl From<JoinError> for ExtractError {
  fn from(e: JoinError) -> Self {
    Self::Join(e)
  }
}

fn ensure_parent(path: &Path) -> io::Result<()> {
  if let Some(p) = path.parent() {
    std::fs::create_dir_all(p)?;
  }
  Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
  let mut name = path.as_os_str().to_owned();
  name.push(format!(".old-{}", uuid::Uuid::new_v4()));
  PathBuf::from(name)
}

// Windows can rename a running or scanned image but not delete or overwrite it, so the live
// file is moved aside first. A failed swap restores it; a crash leaves it as `<file>.old-<uuid>`,
// which `cleanup_leftovers` restores or removes at startup.
fn atomic_file_replace(tmp: &Path, dst: &Path) -> io::Result<()> {
  if !dst.exists() {
    return std::fs::rename(tmp, dst);
  }
  let backup = backup_path(dst);
  std::fs::rename(dst, &backup)?;
  if let Err(e) = std::fs::rename(tmp, dst) {
    if let Err(restore) = std::fs::rename(&backup, dst) {
      tracing::warn!("Could not restore the previous tool after a failed swap: {restore}");
    }
    return Err(e);
  }
  // A running image cannot be deleted on Windows; startup cleanup removes it later.
  let _ = std::fs::remove_file(&backup);
  Ok(())
}

// The old directory is moved aside, not deleted, so a failed swap restores it.
fn atomic_dir_replace(tmp_dir: &Path, dst_dir: &Path) -> io::Result<()> {
  if !dst_dir.exists() {
    return std::fs::rename(tmp_dir, dst_dir);
  }
  let backup = backup_path(dst_dir);
  std::fs::rename(dst_dir, &backup)?;
  if let Err(e) = std::fs::rename(tmp_dir, dst_dir) {
    if let Err(restore) = std::fs::rename(&backup, dst_dir) {
      tracing::warn!("Could not restore the previous tool folder after a failed swap: {restore}");
    }
    return Err(e);
  }
  if std::fs::remove_dir_all(&backup).is_err() {
    tracing::warn!("Could not remove the previous tool folder after the swap");
  }
  Ok(())
}

fn safe_join(root: &Path, candidate: &Path) -> Result<PathBuf, ExtractError> {
  let mut out = PathBuf::from(root);
  for comp in candidate.components() {
    match comp {
      Component::Normal(p) => out.push(p),
      Component::CurDir => {}
      Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
        return Err(ExtractError::PathTraversal(candidate.display().to_string()))
      }
    }
  }
  Ok(out)
}

fn zip_basename(name: &str) -> &str {
  name.rsplit('/').next().unwrap_or(name)
}

pub async fn extract_zip(
  tool: &str,
  canonical: PathBuf,
  archive: &Path,
  out_dir: &Path,
  entry: Option<&str>,
) -> Result<PathBuf, ExtractError> {
  let tool_name = tool.to_string();
  let archive = archive.to_owned();
  let out_dir = out_dir.to_owned();
  let entry = entry.map(str::to_owned);

  tokio::task::spawn_blocking(move || -> Result<PathBuf, ExtractError> {
    let file = std::fs::File::open(&archive)?;
    let mut zip = ZipArchive::new(file)?;

    let mut files = Vec::<(usize, String)>::with_capacity(zip.len());
    for i in 0..zip.len() {
      let e = zip.by_index(i)?;
      if e.is_file() {
        files.push((i, e.name()?.into_owned()));
      }
    }
    let available = EntriesDisplay(files.iter().map(|(_, n)| n.clone()).collect());

    let chosen_index = if let Some(expect) = entry {
      if let Some((idx, _)) = files.iter().find(|(_, n)| n == &expect) {
        *idx
      } else {
        return Err(ExtractError::EntryNotFound {
          wanted: expect,
          available,
        });
      }
    } else if let Some((idx, _)) = files.iter().find(|(_, n)| zip_basename(n) == tool_name) {
      *idx
    } else if files.len() == 1 {
      files[0].0
    } else {
      return Err(ExtractError::AmbiguousArchive(available));
    };

    let mut e = zip.by_index(chosen_index)?;
    let entry_name = e.name()?;
    let base = {
      let b = zip_basename(&entry_name);
      if b.is_empty() {
        tool_name.as_str()
      } else {
        b
      }
    };

    let tmp_out = out_dir.join(format!("{base}.tmp"));
    ensure_parent(&tmp_out)?;
    {
      let mut outfile = std::fs::File::create(&tmp_out)?;
      std::io::copy(&mut e, &mut outfile)?;
    }
    atomic_file_replace(&tmp_out, &canonical)?;
    Ok(canonical)
  })
  .await?
}

pub async fn extract_tar_bz2(
  tool: &str,
  canonical: PathBuf,
  archive: &Path,
  out_dir: &Path,
  entry: Option<&str>,
) -> Result<PathBuf, ExtractError> {
  let tool_name = tool.to_string();
  let archive = archive.to_owned();
  let out_dir = out_dir.to_owned();
  let entry = entry.map(str::to_owned);

  tokio::task::spawn_blocking(move || -> Result<PathBuf, ExtractError> {
    let file = std::fs::File::open(&archive)?;
    let mut tar = TarArchive::new(BzDecoder::new(file));

    let mut files = Vec::<PathBuf>::new();
    for e in tar.entries()? {
      let e = e?;
      if e.header().entry_type().is_file() {
        files.push(e.path()?.into_owned());
      }
    }
    let available = EntriesDisplay(
      files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect(),
    );

    let chosen_path = if let Some(expect) = entry {
      let expect_os = std::ffi::OsString::from(expect.clone());
      if let Some(p) = files.iter().find(|p| p.as_os_str() == expect_os) {
        p.clone()
      } else {
        return Err(ExtractError::EntryNotFound {
          wanted: expect,
          available,
        });
      }
    } else if let Some(p) = files.iter().find(|p| {
      p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == tool_name)
    }) {
      p.clone()
    } else if files.len() == 1 {
      files.remove(0)
    } else {
      return Err(ExtractError::AmbiguousArchive(available));
    };

    let file = std::fs::File::open(&archive)?;
    let mut tar = TarArchive::new(BzDecoder::new(file));

    let base = chosen_path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or(tool_name.as_str());
    let tmp_out = out_dir.join(format!("{base}.tmp"));
    ensure_parent(&tmp_out)?;

    for e in tar.entries()? {
      let mut e = e?;
      if !e.header().entry_type().is_file() {
        continue;
      }
      if e.path()?.as_ref() == chosen_path {
        e.unpack(&tmp_out)?;
        atomic_file_replace(&tmp_out, &canonical)?;
        return Ok(canonical);
      }
    }

    Err(
      io::Error::new(
        io::ErrorKind::NotFound,
        "selected entry vanished during extraction",
      )
      .into(),
    )
  })
  .await?
}

/// Extract one explicit tool from the pinned FFmpeg `.tar.xz` distribution.
pub async fn extract_tar_xz(
  tool: &str,
  canonical: PathBuf,
  archive: &Path,
  out_dir: &Path,
  entry: Option<&str>,
) -> Result<PathBuf, ExtractError> {
  let tool = tool.to_owned();
  let archive = archive.to_owned();
  let out_dir = out_dir.to_owned();
  let entry = entry.map(str::to_owned);
  tokio::task::spawn_blocking(move || -> Result<PathBuf, ExtractError> {
    let reader = lzma_rust2::XzReader::new(std::fs::File::open(archive)?, false);
    let mut tar = TarArchive::new(reader);
    let mut available = Vec::new();
    for member in tar.entries()? {
      let mut member = member?;
      let path = member.path()?.into_owned();
      safe_join(&out_dir, &path)?;
      let chosen = entry.as_ref().map_or_else(
        || path.file_name().is_some_and(|name| name == tool.as_str()),
        |expected| path == Path::new(expected),
      );
      if chosen {
        if !member.header().entry_type().is_file() {
          return Err(ExtractError::UnsupportedEntry(path.display().to_string()));
        }
        let temporary = canonical.with_extension("xz.tmp");
        ensure_parent(&temporary)?;
        member.unpack(&temporary)?;
        atomic_file_replace(&temporary, &canonical)?;
        return Ok(canonical);
      }
      available.push(path.to_string_lossy().into_owned());
    }
    Err(ExtractError::EntryNotFound {
      wanted: entry.unwrap_or(tool),
      available: EntriesDisplay(available),
    })
  })
  .await?
}

pub async fn extract_zip_bundle(
  archive: &Path,
  out_parent: &Path,
  final_folder_name: Option<&str>,
  entry_relative: &Path,
  rename_entry_to: Option<&str>,
) -> Result<(PathBuf, PathBuf), ExtractError> {
  let archive = archive.to_owned();
  let out_parent = out_parent.to_owned();
  let final_folder_name = final_folder_name.map(str::to_string);
  let entry_relative = entry_relative.to_path_buf();
  let rename_entry_to = rename_entry_to.map(str::to_string);

  tokio::task::spawn_blocking(move || -> Result<(PathBuf, PathBuf), ExtractError> {
    let file = std::fs::File::open(&archive)?;
    let mut zip = ZipArchive::new(file)?;

    let temp_root = out_parent.join(format!(".extract-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root)?;

    for i in 0..zip.len() {
      let mut e = zip.by_index(i)?;
      let entry_name = e.name()?;

      let rel_path = Path::new(entry_name.as_ref());
      let out_path = safe_join(&temp_root, rel_path)?;

      if e.is_dir() {
        std::fs::create_dir_all(&out_path)?;
        continue;
      }
      ensure_parent(&out_path)?;
      let mut outfile = std::fs::File::create(&out_path)?;
      std::io::copy(&mut e, &mut outfile)?;
    }

    let extracted_root = detect_extracted_root(&temp_root)?;

    let final_dir = {
      let name = if let Some(name) = final_folder_name.clone() {
        name
      } else {
        extracted_root.file_name().map_or_else(
          || "bundle".to_string(),
          |n| n.to_string_lossy().into_owned(),
        )
      };
      out_parent.join(name)
    };

    atomic_dir_replace(&extracted_root, &final_dir)?;

    let entry_abs_initial = final_dir.join(&entry_relative);
    if !entry_abs_initial.exists() {
      return Err(ExtractError::EntryNotFound {
        wanted: entry_relative.to_string_lossy().into_owned(),
        available: list_tree(&final_dir, 200),
      });
    }

    let entry_abs = if let Some(new_name) = &rename_entry_to {
      let target = entry_abs_initial
        .parent()
        .map_or_else(|| final_dir.join(new_name), |p| p.join(new_name));
      std::fs::rename(&entry_abs_initial, &target)?;
      target
    } else {
      entry_abs_initial
    };

    Ok((final_dir, entry_abs))
  })
  .await?
}

pub async fn extract_tar_bz2_bundle(
  archive: &Path,
  out_parent: &Path,
  final_folder_name: Option<&str>,
  entry_relative: &Path,
  rename_entry_to: Option<&str>,
) -> Result<(PathBuf, PathBuf), ExtractError> {
  let archive = archive.to_owned();
  let out_parent = out_parent.to_owned();
  let final_folder_name = final_folder_name.map(str::to_string);
  let entry_relative = entry_relative.to_path_buf();
  let rename_entry_to = rename_entry_to.map(str::to_string);

  tokio::task::spawn_blocking(move || -> Result<(PathBuf, PathBuf), ExtractError> {
    let file = std::fs::File::open(&archive)?;
    let mut tar = TarArchive::new(BzDecoder::new(file));

    let temp_root = out_parent.join(format!(".extract-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root)?;

    for entry in tar.entries()? {
      let mut entry = entry?;
      let path_in_tar = entry.path()?.into_owned();

      let out_path = safe_join(&temp_root, &path_in_tar)?;

      match entry.header().entry_type() {
        TarEntryType::Directory => {
          std::fs::create_dir_all(&out_path)?;
        }
        TarEntryType::Regular => {
          ensure_parent(&out_path)?;
          entry.unpack(&out_path)?;
        }
        other => {
          return Err(ExtractError::UnsupportedEntry(format!(
            "{} ({:?})",
            path_in_tar.display(),
            other
          )));
        }
      }
    }

    let extracted_root = detect_extracted_root(&temp_root)?;

    let final_dir = {
      let name = if let Some(name) = final_folder_name.clone() {
        name
      } else {
        extracted_root.file_name().map_or_else(
          || "bundle".to_string(),
          |n| n.to_string_lossy().into_owned(),
        )
      };
      out_parent.join(name)
    };

    atomic_dir_replace(&extracted_root, &final_dir)?;

    let entry_abs_initial = final_dir.join(&entry_relative);
    if !entry_abs_initial.exists() {
      return Err(ExtractError::EntryNotFound {
        wanted: entry_relative.to_string_lossy().into_owned(),
        available: list_tree(&final_dir, 200),
      });
    }

    let entry_abs = if let Some(new_name) = &rename_entry_to {
      let target = entry_abs_initial
        .parent()
        .map_or_else(|| final_dir.join(new_name), |p| p.join(new_name));
      std::fs::rename(&entry_abs_initial, &target)?;
      target
    } else {
      entry_abs_initial
    };

    Ok((final_dir, entry_abs))
  })
  .await?
}

fn detect_extracted_root(temp_root: &Path) -> Result<PathBuf, ExtractError> {
  let mut entries = Vec::new();
  for e in std::fs::read_dir(temp_root)? {
    entries.push(e?.path());
  }
  if entries.len() == 1 && entries[0].is_dir() {
    Ok(entries[0].clone())
  } else {
    Ok(temp_root.to_path_buf())
  }
}

fn list_tree(root: &Path, max: usize) -> EntriesDisplay {
  fn walk(acc: &mut Vec<String>, base: &Path, p: &Path, max: usize) {
    if acc.len() >= max {
      return;
    }
    let rel = p.strip_prefix(base).unwrap_or(p);
    acc.push(rel.to_string_lossy().into_owned());
    if let Ok(meta) = std::fs::metadata(p) {
      if meta.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
          for ch in rd.flatten().map(|e| e.path()) {
            if acc.len() >= max {
              break;
            }
            walk(acc, base, &ch, max);
          }
        }
      }
    }
  }
  let mut v = Vec::new();
  walk(&mut v, root, root, max);
  EntriesDisplay(v)
}

#[cfg(test)]
mod tests {
  use super::{atomic_dir_replace, atomic_file_replace, extract_zip, extract_zip_bundle};
  use std::io::Write;
  use std::path::Path;
  use zip::write::SimpleFileOptions;

  fn swap_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nezzel-swap-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
  }

  #[test]
  fn file_replace_overwrites_existing_file() {
    let root = swap_dir();
    let (tmp, dst) = (root.join("tool.tmp"), root.join("tool"));
    std::fs::write(&dst, b"old").unwrap();
    std::fs::write(&tmp, b"new").unwrap();
    atomic_file_replace(&tmp, &dst).unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), b"new");
    assert!(!tmp.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn failed_file_replace_keeps_the_original() {
    let root = swap_dir();
    let dst = root.join("tool");
    std::fs::write(&dst, b"old").unwrap();
    assert!(atomic_file_replace(&root.join("missing"), &dst).is_err());
    assert_eq!(std::fs::read(&dst).unwrap(), b"old");
    let names = std::fs::read_dir(&root).unwrap().count();
    assert_eq!(names, 1);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn dir_replace_swaps_and_leaves_no_backup() {
    let root = swap_dir();
    let (tmp, dst) = (root.join("tmp"), root.join("deno"));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(tmp.join("new.txt"), b"new").unwrap();
    std::fs::write(dst.join("old.txt"), b"old").unwrap();
    atomic_dir_replace(&tmp, &dst).unwrap();
    assert!(dst.join("new.txt").exists());
    assert!(!dst.join("old.txt").exists());
    let names: Vec<_> = std::fs::read_dir(&root)
      .unwrap()
      .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
      .collect();
    assert_eq!(names, vec!["deno".to_string()]);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn failed_dir_replace_keeps_the_original() {
    let root = swap_dir();
    let dst = root.join("deno");
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(dst.join("old.txt"), b"old").unwrap();
    assert!(atomic_dir_replace(&root.join("missing"), &dst).is_err());
    assert_eq!(std::fs::read(dst.join("old.txt")).unwrap(), b"old");
    std::fs::remove_dir_all(root).unwrap();
  }

  #[tokio::test]
  async fn extracts_exact_xz_manifest_entry_without_installing_other_members() {
    let root = std::env::temp_dir().join(format!("nezzel-xz-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let archive = root.join("ffmpeg.tar.xz");
    let writer = lzma_rust2::XzWriter::new(
      std::fs::File::create(&archive).unwrap(),
      lzma_rust2::XzOptions::with_preset(1),
    )
    .unwrap();
    let mut tar = tar::Builder::new(writer);
    for (name, bytes) in [
      ("build/bin/ffmpeg", b"ffmpeg".as_slice()),
      ("build/bin/ffprobe", b"probe".as_slice()),
    ] {
      let mut header = tar::Header::new_gnu();
      header.set_size(bytes.len() as u64);
      header.set_mode(0o755);
      header.set_cksum();
      tar.append_data(&mut header, name, bytes).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
    let canonical = root.join("ffprobe");
    super::extract_tar_xz(
      "ffprobe",
      canonical.clone(),
      &archive,
      &root,
      Some("build/bin/ffprobe"),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&canonical).unwrap(), b"probe");
    assert!(!root.join("build").exists());
    assert!(super::extract_tar_xz(
      "ffmpeg",
      root.join("missing"),
      &archive,
      &root,
      Some("absent")
    )
    .await
    .is_err());
    assert!(!root.join("missing").exists());
    std::fs::remove_dir_all(root).unwrap();
  }

  #[tokio::test]
  async fn extracts_unicode_zip_members_as_single_tools_and_bundles() {
    let root = std::env::temp_dir().join(format!("nezzel-zip-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let archive = root.join("tools.zip");
    let member = "bundle/nested/أداة";
    let payload = b"tool executable";
    let file = std::fs::File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    writer
      .start_file(member, SimpleFileOptions::default())
      .unwrap();
    writer.write_all(payload).unwrap();
    writer.finish().unwrap();

    let canonical = root.join("tool");
    let installed = extract_zip("tool", canonical.clone(), &archive, &root, Some(member))
      .await
      .unwrap();
    assert_eq!(installed, canonical);
    assert_eq!(std::fs::read(installed).unwrap(), payload);

    let (bundle, entry) = extract_zip_bundle(
      &archive,
      &root,
      Some("installed-bundle"),
      Path::new("nested/أداة"),
      Some("tool"),
    )
    .await
    .unwrap();
    assert_eq!(bundle, root.join("installed-bundle"));
    assert_eq!(entry, bundle.join("nested/tool"));
    assert_eq!(std::fs::read(entry).unwrap(), payload);
    std::fs::remove_dir_all(root).unwrap();
  }
}
