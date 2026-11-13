//! Publish engine output without treating an unrelated filename as a completed download.
use crate::download_records::{regular_file_identity, FileIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
  fs, io,
  io::{Read, Write},
  path::{Path, PathBuf},
  sync::Mutex,
};
use tauri::Manager;

static PUBLISH: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
struct Owner {
  media: String,
  identity: Option<FileIdentity>,
}

pub(super) struct Published {
  pub path: PathBuf,
  pub reused: bool,
}

pub(super) enum PublishOutcome {
  Files(Vec<Published>),
  Filtered,
}

pub(crate) fn receipt_directory<R: tauri::Runtime>(
  app: &tauri::AppHandle<R>,
) -> io::Result<PathBuf> {
  app
    .path()
    .app_data_dir()
    .map(|path| path.join("media-receipts"))
    .map_err(io::Error::other)
}

#[derive(Default)]
pub(super) struct PendingFiles(Vec<(PathBuf, Option<String>, Option<String>)>);

impl PendingFiles {
  pub fn record(&mut self, line: &str) {
    if let Some(json) = line.strip_prefix("NEZZEL_PUBLISH|") {
      if let Ok(file) = serde_json::from_str(json) {
        self.0.push(file);
      }
    }
  }

  pub fn publish(
    &self,
    stage: &Path,
    anchor: &Path,
    url: &str,
    receipts: &Path,
    cancelled: impl Fn() -> bool,
  ) -> io::Result<PublishOutcome> {
    if cancelled() {
      return Err(io::ErrorKind::Interrupted.into());
    }
    if self.0.is_empty() {
      return Ok(PublishOutcome::Filtered);
    }
    let root = stage.canonicalize()?;
    let mut results = Vec::new();
    let mut sidecars = Vec::new();
    let mut published_sidecars = std::collections::HashSet::new();
    collect_files(&root, &mut sidecars)?;
    let primary_paths: std::collections::HashSet<_> = self
      .0
      .iter()
      .map(|(path, _, _)| path.canonicalize())
      .collect::<io::Result<_>>()?;
    for (source, extractor, id) in &self.0 {
      if cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
      }
      let source = source.canonicalize()?;
      let relative = source
        .strip_prefix(&root)
        .map_err(|_| io::ErrorKind::InvalidData)?;
      if regular_file_identity(&source).is_none() {
        return Err(io::ErrorKind::InvalidData.into());
      }
      let media = digest(&match (extractor, id) {
        (Some(extractor), Some(id)) if !extractor.is_empty() && !id.is_empty() => {
          format!("{extractor}\0{id}")
        }
        _ => url.to_owned(),
      });
      let published = publish_file(
        &source,
        &anchor.join(relative),
        &media,
        receipts,
        &cancelled,
      )?;
      // Saved subtitles/thumbnails follow the primary's collision suffix.
      let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(io::ErrorKind::InvalidData)?;
      let new_stem = published
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(io::ErrorKind::InvalidData)?;
      for sidecar in &sidecars {
        if primary_paths.contains(sidecar) || published_sidecars.contains(sidecar) {
          continue;
        }
        if sidecar.parent() != source.parent() {
          continue;
        }
        let Some(suffix) = sidecar
          .file_name()
          .and_then(|n| n.to_str())
          .and_then(|n| n.strip_prefix(stem))
          .filter(|s| s.starts_with('.'))
        else {
          continue;
        };
        if cancelled() {
          return Err(io::ErrorKind::Interrupted.into());
        }
        match publish_file(
          sidecar,
          &published.path.with_file_name(format!("{new_stem}{suffix}")),
          &media,
          receipts,
          &cancelled,
        ) {
          Ok(_) => {}
          Err(error) if error.kind() == io::ErrorKind::NotFound => {}
          Err(error) => return Err(error),
        }
        published_sidecars.insert(sidecar);
      }
      results.push(published);
    }
    Ok(PublishOutcome::Files(results))
  }
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
  for entry in fs::read_dir(directory)? {
    let entry = entry?;
    let kind = entry.file_type()?;
    if kind.is_symlink() {
      return Err(io::ErrorKind::InvalidData.into());
    }
    if kind.is_dir() {
      collect_files(&entry.path(), files)?;
    } else if kind.is_file() {
      files.push(entry.path());
    }
  }
  Ok(())
}

fn digest(value: &str) -> String {
  hex::encode(Sha256::digest(value.as_bytes()))
}

fn receipt_path(receipts: &Path, destination: &Path) -> io::Result<PathBuf> {
  let canonical = destination.canonicalize().or_else(|_| {
    Ok::<_, io::Error>(
      destination
        .parent()
        .ok_or(io::ErrorKind::InvalidInput)?
        .canonicalize()?
        .join(destination.file_name().ok_or(io::ErrorKind::InvalidInput)?),
    )
  })?;
  let key = canonical.to_string_lossy();
  #[cfg(windows)]
  let key = key.to_lowercase();
  Ok(receipts.join(format!("{}.json", digest(&key))))
}

fn legacy_receipt(destination: &Path) -> io::Result<PathBuf> {
  let name = destination
    .file_name()
    .and_then(|n| n.to_str())
    .ok_or(io::ErrorKind::InvalidInput)?;
  Ok(destination.with_file_name(format!(".nezzel-media-{}.json", digest(name))))
}

fn read_owner(path: &Path) -> Option<Owner> {
  let metadata = fs::symlink_metadata(path).ok()?;
  if !metadata.is_file() || metadata.len() > 4096 {
    return None;
  }
  serde_json::from_slice(&fs::read(path).ok()?).ok()
}

pub(crate) fn remove_receipts(receipts: &Path, destination: &Path) -> io::Result<()> {
  for path in [
    receipt_path(receipts, destination)?,
    legacy_receipt(destination)?,
  ] {
    match fs::remove_file(path) {
      Ok(()) => {}
      Err(error) if error.kind() == io::ErrorKind::NotFound => {}
      Err(error) => return Err(error),
    }
  }
  Ok(())
}

pub(crate) fn remove_missing_receipts(
  record: &crate::download_records::GroupRecord,
  receipts: &Path,
  destination: &Path,
) -> io::Result<()> {
  let inside_root = destination
    .parent()
    .and_then(|parent| parent.canonicalize().ok())
    .is_some_and(|parent| {
      record.roots.iter().any(|root| {
        root
          .canonicalize()
          .is_ok_and(|root| parent.starts_with(root))
      })
    });
  if destination.is_absolute()
    && inside_root
    && record.files.get(destination).is_some_and(Option::is_some)
    && fs::symlink_metadata(destination).is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
  {
    remove_receipts(receipts, destination)?;
  }
  Ok(())
}

pub(crate) fn trash_with_receipts(
  destination: &Path,
  receipts: &Path,
  trash: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
  trash(destination)?;
  if remove_receipts(receipts, destination).is_err() {
    tracing::warn!("Unable to remove download ownership receipt");
  }
  Ok(())
}

fn new_receipt(path: &Path) -> io::Result<fs::File> {
  let mut options = fs::OpenOptions::new();
  options.write(true).create_new(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  options.open(path)
}

fn publish_file(
  source: &Path,
  destination: &Path,
  media: &str,
  receipts: &Path,
  cancelled: &impl Fn() -> bool,
) -> io::Result<Published> {
  let parent = destination.parent().ok_or(io::ErrorKind::InvalidInput)?;
  fs::create_dir_all(parent)?;
  fs::create_dir_all(receipts)?;
  let name = destination
    .file_name()
    .and_then(|n| n.to_str())
    .ok_or(io::ErrorKind::InvalidInput)?;
  let stem = destination
    .file_stem()
    .and_then(|n| n.to_str())
    .ok_or(io::ErrorKind::InvalidInput)?;
  let extension = destination
    .extension()
    .and_then(|e| e.to_str())
    .map(|e| format!(".{e}"))
    .unwrap_or_default();
  for number in 0..10_000 {
    if cancelled() {
      return Err(io::ErrorKind::Interrupted.into());
    }
    let guard = PUBLISH.lock().unwrap_or_else(|e| e.into_inner());
    let name = if number == 0 {
      name.to_owned()
    } else {
      let suffix = format!("-{number:03}{extension}");
      let mut end = stem.len().min(255usize.saturating_sub(suffix.len()));
      while !stem.is_char_boundary(end) {
        end -= 1;
      }
      format!("{}{suffix}", &stem[..end])
    };
    let path = parent.join(&name);
    let receipt = receipt_path(receipts, &path)?;
    let legacy = legacy_receipt(&path)?;
    if let Some(owner) = read_owner(&receipt).or_else(|| read_owner(&legacy)) {
      if let Some(identity) = regular_file_identity(&path) {
        if owner.media == media && owner.identity.as_ref() == Some(&identity) {
          if !receipt.try_exists()? {
            let mut file = new_receipt(&receipt)?;
            file.write_all(&serde_json::to_vec(&owner)?)?;
            file.sync_all()?;
          }
          if read_owner(&legacy).is_some() {
            fs::remove_file(&legacy)?;
          }
          return Ok(Published { path, reused: true });
        }
      } else if owner.identity.is_some() && !path.try_exists()? {
        let _ = fs::remove_file(&receipt);
      }
    }
    if fs::symlink_metadata(&path).is_ok() {
      continue;
    }
    let mut owner_file = match new_receipt(&receipt) {
      Ok(file) => file,
      Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
      Err(error) => return Err(error),
    };
    // The app-data receipt reserves this candidate. Slow I/O never holds the global lock.
    drop(guard);
    if let Err(error) = transfer_file(
      source,
      &path,
      cancelled,
      |from, to| fs::hard_link(from, to),
      crate::files::rename_without_replace,
    ) {
      fs::remove_file(&receipt)?;
      if error.kind() == io::ErrorKind::AlreadyExists {
        continue;
      }
      return Err(error);
    }
    let owner = Owner {
      media: media.into(),
      identity: regular_file_identity(&path),
    };
    owner_file.write_all(&serde_json::to_vec(&owner)?)?;
    owner_file.sync_all()?;
    if read_owner(&legacy).is_some() {
      fs::remove_file(&legacy)?;
    }
    return Ok(Published {
      path,
      reused: false,
    });
  }
  Err(io::Error::new(
    io::ErrorKind::AlreadyExists,
    "No unused download filename",
  ))
}

fn transfer_file(
  source: &Path,
  destination: &Path,
  cancelled: &impl Fn() -> bool,
  link: impl FnOnce(&Path, &Path) -> io::Result<()>,
  mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
  if cancelled() {
    return Err(io::ErrorKind::Interrupted.into());
  }
  match link(source, destination) {
    Ok(()) => return Ok(()),
    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error),
    Err(_) => {}
  }
  match rename(source, destination) {
    Ok(()) => return Ok(()),
    Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {}
    Err(error) if crate::files::unsupported_publish(&error) => {
      return copy_to_new_file(source, destination, cancelled)
    }
    Err(error) => return Err(error),
  }
  let temporary = destination.with_file_name(format!(".nezzel-copy-{}.part", uuid::Uuid::new_v4()));
  let result = (|| {
    copy_to_new_file(source, &temporary, cancelled)?;
    if cancelled() {
      return Err(io::ErrorKind::Interrupted.into());
    }
    match rename(&temporary, destination) {
      Err(error) if crate::files::unsupported_publish(&error) => {
        copy_to_new_file(&temporary, destination, cancelled)
      }
      result => result,
    }
  })();
  let _ = fs::remove_file(&temporary);
  result
}

fn copy_to_new_file(
  source: &Path,
  destination: &Path,
  cancelled: &impl Fn() -> bool,
) -> io::Result<()> {
  if cancelled() {
    return Err(io::ErrorKind::Interrupted.into());
  }
  let mut input = fs::File::open(source)?;
  let mut target = fs::OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(destination)?;
  let result = (|| {
    let mut buffer = [0u8; 64 * 1024];
    loop {
      if cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
      }
      let count = input.read(&mut buffer)?;
      if count == 0 {
        break;
      }
      target.write_all(&buffer[..count])?;
    }
    target.sync_all()?;
    if cancelled() {
      return Err(io::ErrorKind::Interrupted.into());
    }
    Ok(())
  })();
  if result.is_err() {
    let _ = crate::files::remove_owned_output(&target, destination);
  }
  result
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::runners::gallery_files::Staging;

  #[test]
  fn unsupported_rename_uses_exclusive_copy_for_initial_and_final_publication() {
    use std::cell::Cell;
    let errors = [io::ErrorKind::Unsupported.into()];
    #[cfg(unix)]
    let errors = errors
      .into_iter()
      .chain([libc::ENOTSUP, libc::EINVAL, libc::ENOSYS].map(io::Error::from_raw_os_error))
      .collect::<Vec<_>>();
    #[cfg(windows)]
    let errors = errors
      .into_iter()
      .chain([1, 50, 87].map(io::Error::from_raw_os_error))
      .collect::<Vec<_>>();
    for unsupported in errors {
      for cross_volume in [false, true] {
        let root = Staging::new().unwrap();
        let source = root.path.join("source");
        let target = root.path.join("Title.mp4");
        fs::write(&source, b"media").unwrap();
        let calls = Cell::new(0);
        transfer_file(
          &source,
          &target,
          &|| false,
          |_, _| Err(io::ErrorKind::Unsupported.into()),
          |_, _| {
            let first = calls.replace(calls.get() + 1) == 0;
            if first && cross_volume {
              Err(io::ErrorKind::CrossesDevices.into())
            } else {
              Err(unsupported.raw_os_error().map_or_else(
                || io::Error::from(unsupported.kind()),
                io::Error::from_raw_os_error,
              ))
            }
          },
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"media");
        assert_eq!(calls.get(), if cross_volume { 2 } else { 1 });
        fs::write(&source, b"different").unwrap();
        let retry_calls = Cell::new(0);
        let error = transfer_file(
          &source,
          &target,
          &|| false,
          |_, _| Err(io::ErrorKind::Unsupported.into()),
          |_, _| {
            let first = retry_calls.replace(retry_calls.get() + 1) == 0;
            if first && cross_volume {
              Err(io::ErrorKind::CrossesDevices.into())
            } else {
              Err(io::ErrorKind::Unsupported.into())
            }
          },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&target).unwrap(), b"media");
      }
    }
  }

  #[test]
  fn multi_output_publishes_shared_sidecars_once_and_tolerates_a_missing_sidecar() {
    for (moved_after_publication, missing_before_publication) in
      [(false, false), (true, false), (false, true)]
    {
      let root = Staging::new().unwrap();
      let output = root.path.join("stage");
      let saved = root.path.join("saved");
      let receipts = root.path.join("receipts");
      fs::create_dir(&output).unwrap();
      fs::create_dir(&saved).unwrap();
      let mut pending = PendingFiles::default();
      for (name, bytes) in [
        ("Title.mp4", b"video".as_slice()),
        ("Title.mp3", b"audio".as_slice()),
      ] {
        let source = output.join(name);
        fs::write(&source, bytes).unwrap();
        pending.record(&format!(
          "NEZZEL_PUBLISH|{}",
          serde_json::json!([source, "Youtube", "same-item"])
        ));
      }
      fs::write(saved.join("Title.mp3"), b"unrelated").unwrap();
      for name in ["Title.en.vtt", "Title.jpg"] {
        fs::write(output.join(name), name).unwrap();
      }
      let outcome = pending
        .publish(&output, &saved, "", &receipts, || {
          // Simulate sidecars being moved, or disappearing before they can be published.
          for name in ["Title.en.vtt", "Title.jpg"] {
            if moved_after_publication && saved.join(name).exists()
              || missing_before_publication && saved.join("Title.mp4").exists()
            {
              let _ = fs::remove_file(output.join(name));
            }
          }
          false
        })
        .unwrap();
      let PublishOutcome::Files(files) = outcome else {
        panic!("unexpected filter")
      };
      assert_eq!(files.len(), 2);
      assert_eq!(fs::read(saved.join("Title.mp4")).unwrap(), b"video");
      assert_eq!(fs::read(saved.join("Title-001.mp3")).unwrap(), b"audio");
      assert_eq!(fs::read(saved.join("Title.mp3")).unwrap(), b"unrelated");
      for name in ["Title.en.vtt", "Title.jpg"] {
        assert_eq!(saved.join(name).exists(), !missing_before_publication);
        if !missing_before_publication {
          assert_eq!(fs::read(saved.join(name)).unwrap(), name.as_bytes());
        }
      }
      assert!(!saved.join("Title-001.en.vtt").exists());
      assert!(!saved.join("Title-001.jpg").exists());
    }
  }

  #[test]
  fn filesystems_without_hard_links_move_the_original_inode_without_copying() {
    let root = Staging::new().unwrap();
    let source = root.path.join("source");
    let target = root.path.join("Title.mp4");
    fs::write(&source, b"media").unwrap();
    let identity = regular_file_identity(&source).unwrap();
    transfer_file(
      &source,
      &target,
      &|| false,
      |_, _| Err(io::ErrorKind::Unsupported.into()),
      crate::files::rename_without_replace,
    )
    .unwrap();
    assert!(!source.exists());
    assert_eq!(regular_file_identity(&target), Some(identity));
    assert_eq!(fs::read(&target).unwrap(), b"media");
    fs::write(&source, b"different").unwrap();
    assert!(transfer_file(
      &source,
      &target,
      &|| false,
      |_, _| Err(io::ErrorKind::Unsupported.into()),
      crate::files::rename_without_replace
    )
    .is_err());
    assert_eq!(fs::read(&target).unwrap(), b"media");
    assert_eq!(fs::read(&source).unwrap(), b"different");
  }

  #[test]
  fn cross_volume_copy_is_cancellable_and_removes_only_its_own_partial() {
    use std::cell::Cell;
    let root = Staging::new().unwrap();
    let source = root.path.join("source");
    fs::write(&source, vec![7u8; 256 * 1024]).unwrap();
    for cancel_after in [None, Some(2)] {
      let target = root.path.join("Title.mp4");
      let calls = Cell::new(0);
      let result = transfer_file(
        &source,
        &target,
        &|| {
          let call = calls.get();
          calls.set(call + 1);
          cancel_after.is_some_and(|limit| call >= limit)
        },
        |_, _| Err(io::ErrorKind::Unsupported.into()),
        |from, to| {
          if from == source {
            Err(io::ErrorKind::CrossesDevices.into())
          } else {
            crate::files::rename_without_replace(from, to)
          }
        },
      );
      if cancel_after.is_some() {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(!target.exists());
      } else {
        result.unwrap();
        assert_eq!(fs::read(&target).unwrap(), fs::read(&source).unwrap());
        fs::remove_file(&target).unwrap();
      }
      assert!(source.exists());
    }
  }

  #[test]
  fn ownership_receipts_do_not_appear_beside_media() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let source = root.path.join("source");
    let destination = root.path.join("saved/Title.mp4");
    fs::write(&source, b"media").unwrap();
    publish_file(&source, &destination, "one", &receipts, &|| false).unwrap();
    let names: Vec<_> = fs::read_dir(destination.parent().unwrap())
      .unwrap()
      .map(|entry| entry.unwrap().file_name())
      .collect();
    assert_eq!(names, ["Title.mp4"]);
    let private = receipt_path(&receipts, &destination).unwrap();
    let legacy = legacy_receipt(&destination).unwrap();
    fs::rename(&private, &legacy).unwrap();
    assert!(
      publish_file(&source, &destination, "one", &receipts, &|| false)
        .unwrap()
        .reused
    );
    assert!(private.exists() && !legacy.exists());
    // Aliases of one canonical path share a receipt; equal basenames in other folders do not.
    let alias = root.path.join("saved/../saved/Title.mp4");
    assert!(
      publish_file(&source, &alias, "one", &receipts, &|| false)
        .unwrap()
        .reused
    );
    let other = root.path.join("other/Title.mp4");
    assert!(
      !publish_file(&source, &other, "one", &receipts, &|| false)
        .unwrap()
        .reused
    );
    assert_ne!(
      receipt_path(&receipts, &destination).unwrap(),
      receipt_path(&receipts, &other).unwrap()
    );
  }

  #[test]
  fn deleting_media_removes_private_and_legacy_receipts_only_after_a_successful_trash() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let source = root.path.join("source");
    let destination = root.path.join("saved/Title.mp4");
    fs::write(&source, b"media").unwrap();
    publish_file(&source, &destination, "one", &receipts, &|| false).unwrap();
    let records = crate::download_records::DownloadRecords::default();
    records.allow_root("group", destination.parent().unwrap().to_path_buf());
    records.record_item("group", "row", destination.to_str().unwrap());
    records.seal("group");
    let record = records.get("group");
    let private = receipt_path(&receipts, &destination).unwrap();
    let legacy = legacy_receipt(&destination).unwrap();
    let receipt_bytes = fs::read(&private).unwrap();
    fs::write(&legacy, &receipt_bytes).unwrap();
    assert!(trash_with_receipts(&destination, &receipts, |_| Err(
      io::ErrorKind::PermissionDenied.into()
    ))
    .is_err());
    assert!(destination.exists() && private.exists() && legacy.exists());
    trash_with_receipts(&destination, &receipts, |path| fs::remove_file(path)).unwrap();
    assert!(!destination.exists() && !private.exists() && !legacy.exists());
    // A row whose file was deleted by the old build can still shed its orphan receipts.
    fs::write(&private, &receipt_bytes).unwrap();
    fs::write(&legacy, &receipt_bytes).unwrap();
    let mut unowned = record.clone();
    unowned.roots.clear();
    remove_missing_receipts(&unowned, &receipts, &destination).unwrap();
    assert!(private.exists() && legacy.exists());
    remove_missing_receipts(&record, &receipts, &destination).unwrap();
    assert!(!private.exists() && !legacy.exists());
  }

  #[test]
  fn filtered_downloads_need_no_stage_directory_or_output_receipts() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let pending = PendingFiles::default();
    assert!(matches!(
      pending.publish(
        &root.path.join("never-created"),
        &root.path.join("saved"),
        "",
        &receipts,
        || false
      ),
      Ok(PublishOutcome::Filtered)
    ));
    assert!(!root.path.join("saved").exists());
  }

  #[test]
  fn media_collisions_preserve_existing_bytes_and_only_reuse_the_same_item() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let source = root.path.join("source");
    let target = root.path.join("saved/فيديو.mp4");
    for (id, bytes, name, reused) in [
      (
        "Facebook:1248407763608788",
        b"first".as_slice(),
        "فيديو.mp4",
        false,
      ),
      (
        "Facebook:other",
        b"second".as_slice(),
        "فيديو-001.mp4",
        false,
      ),
      (
        "Facebook:1248407763608788",
        b"first".as_slice(),
        "فيديو.mp4",
        true,
      ),
      (
        "Instagram:1248407763608788",
        b"third".as_slice(),
        "فيديو-002.mp4",
        false,
      ),
    ] {
      // A fresh engine output, as on a new row or after restarting the app.
      let _ = fs::remove_file(&source);
      fs::write(&source, bytes).unwrap();
      let published = publish_file(&source, &target, id, &receipts, &|| false).unwrap();
      assert_eq!(published.path.file_name().unwrap(), name);
      assert_eq!(published.reused, reused);
      assert_eq!(fs::read(published.path).unwrap(), bytes);
    }
    assert_eq!(fs::read(&target).unwrap(), b"first");
    assert_eq!(
      fs::read(target.with_file_name("فيديو-001.mp4")).unwrap(),
      b"second"
    );
  }

  #[test]
  fn legacy_files_and_replaced_receipts_are_never_adopted() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let source = root.path.join("source");
    let target = root.path.join("Title.mp4");
    fs::write(&source, b"new").unwrap();
    fs::write(&target, b"legacy").unwrap();
    let first = publish_file(&source, &target, "one", &receipts, &|| false).unwrap();
    assert_eq!(first.path.file_name().unwrap(), "Title-001.mp4");
    fs::rename(&first.path, root.path.join("moved.mp4")).unwrap();
    fs::write(&first.path, b"replacement").unwrap();
    let next = publish_file(&source, &target, "one", &receipts, &|| false).unwrap();
    assert_eq!(next.path.file_name().unwrap(), "Title-002.mp4");
    assert_eq!(fs::read(&target).unwrap(), b"legacy");
    assert_eq!(fs::read(first.path).unwrap(), b"replacement");
  }

  #[test]
  fn staged_media_receipts_publish_primary_and_sidecars_without_exposing_private_paths() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let output = root.path.join("stage/output");
    let final_root = root.path.join("saved");
    for (id, bytes, expected, reused) in [
      ("one", b"first".as_slice(), "Title.mp4", false),
      ("two", b"second".as_slice(), "Title-001.mp4", false),
      ("one", b"first".as_slice(), "Title.mp4", true),
    ] {
      let source = output.join("nested/Title.mp4");
      fs::create_dir_all(source.parent().unwrap()).unwrap();
      // Replace rather than mutate an inode already linked to the saved file.
      let _ = fs::remove_file(&source);
      let _ = fs::remove_file(source.with_extension("en.vtt"));
      fs::write(&source, bytes).unwrap();
      fs::write(source.with_extension("en.vtt"), bytes).unwrap();
      let mut pending = PendingFiles::default();
      pending.record(&format!(
        "NEZZEL_PUBLISH|{}",
        serde_json::json!([source, "Facebook", id])
      ));
      let published = pending
        .publish(
          &output,
          &final_root,
          "https://fb.watch/alias/",
          &receipts,
          || false,
        )
        .unwrap();
      let PublishOutcome::Files(published) = published else {
        panic!("unexpected filter outcome")
      };
      assert_eq!(published.len(), 1);
      assert_eq!(published[0].path, final_root.join("nested").join(expected));
      assert_eq!(published[0].reused, reused);
      assert_eq!(fs::read(&published[0].path).unwrap(), bytes);
      assert_eq!(
        fs::read(published[0].path.with_extension("en.vtt")).unwrap(),
        bytes
      );
    }
  }

  #[test]
  fn concurrent_items_cannot_claim_the_same_filename() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let target = root.path.join("Title.mp4");
    let jobs: Vec<_> = ["one", "two"]
      .into_iter()
      .map(|id| {
        let source = root.path.join(id);
        fs::write(&source, id).unwrap();
        let target = target.clone();
        let receipts = receipts.clone();
        std::thread::spawn(move || {
          publish_file(&source, &target, id, &receipts, &|| false)
            .unwrap()
            .path
        })
      })
      .collect();
    let paths: Vec<_> = jobs.into_iter().map(|job| job.join().unwrap()).collect();
    assert_ne!(paths[0], paths[1]);
    assert_eq!(fs::read(&paths[0]).unwrap(), b"one");
    assert_eq!(fs::read(&paths[1]).unwrap(), b"two");
  }

  #[test]
  fn cancelled_or_escaped_stage_receipts_cannot_publish() {
    let root = Staging::new().unwrap();
    let receipts = root.path.join("receipts");
    let stage = root.path.join("stage");
    fs::create_dir(&stage).unwrap();
    let outside = root.path.join("outside.mp4");
    fs::write(&outside, b"outside").unwrap();
    let mut pending = PendingFiles::default();
    pending.record(&format!(
      "NEZZEL_PUBLISH|{}",
      serde_json::json!([outside, "Facebook", "one"])
    ));
    for cancelled in [false, true] {
      assert!(pending
        .publish(&stage, &root.path.join("saved"), "", &receipts, || {
          cancelled
        })
        .is_err());
      assert!(!root.path.join("saved").exists());
    }
  }
}
