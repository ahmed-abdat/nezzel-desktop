//! Durable sidecar state. Count bytes only after the part file has been synced.
//! Design: Zajil Download Manager, GPL-3.0-or-later (see Zajil-LICENSE).
use super::{
  auth::StateKey,
  plan::{Segment, Validator},
  sanitize::{reserved_name, sanitize_file_name},
  Error, Result,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
  fs::{File, OpenOptions},
  io::{Read, Write},
  path::{Path, PathBuf},
};
pub(crate) const STATE_SUFFIX: &str = ".nezzel-state";
pub(crate) const PART_SUFFIX: &str = ".nezzel-part";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
  schema: u32,
  pub identity: String,
  pub owner: String,
  pub name: String,
  pub size: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mime: Option<String>,
  pub ranges: bool,
  pub validator: Validator,
  pub segments: Vec<Segment>,
  checksum: String,
}
pub(crate) fn identity(url: &str) -> String {
  hex::encode(Sha256::digest(url.as_bytes()))
}
pub(crate) fn sibling(path: &Path, suffix: &str) -> PathBuf {
  let mut name = path.as_os_str().to_owned();
  name.push(suffix);
  PathBuf::from(name)
}
pub(crate) fn regular(path: &Path, write: bool) -> Result<File> {
  let mut opts = OpenOptions::new();
  opts.read(true).write(write);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
  }
  #[cfg(windows)]
  {
    use std::os::windows::fs::OpenOptionsExt;
    opts.custom_flags(0x00200000);
  }
  let file = opts.open(path)?;
  let meta = file.metadata()?;
  if !meta.is_file() {
    return Err(Error::State);
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    if meta.nlink() != 1 {
      return Err(Error::State);
    }
  }
  #[cfg(windows)]
  {
    use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
      GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
      return Err(Error::Io);
    }
    let info = unsafe { info.assume_init() };
    if meta.file_attributes() & 0x400 != 0 || info.nNumberOfLinks != 1 {
      return Err(Error::State);
    }
  }
  Ok(file)
}
impl State {
  pub fn new(
    identity: String,
    name: String,
    size: Option<u64>,
    ranges: bool,
    validator: Validator,
    segments: Vec<Segment>,
  ) -> Self {
    Self {
      schema: 2,
      identity,
      owner: String::new(),
      name,
      size,
      ranges,
      mime: None,
      validator,
      segments,
      checksum: String::new(),
    }
  }
  pub fn bytes(&self) -> u64 {
    self.segments.iter().map(|s| s.done).sum()
  }
  pub fn valid(&self) -> bool {
    if self.schema != 2
      || self.identity.len() != 64
      || self.name != sanitize_file_name(&self.name)
      || self.segments.is_empty()
      || self.segments.len() > 1024
    {
      return false;
    }
    let mut next = 0u64;
    for s in &self.segments {
      if s.start != next || s.done > u64::MAX - s.start {
        return false;
      }
      match s.end {
        Some(end) if end >= s.start && s.done <= end - s.start => next = end,
        None if self.size.is_none() && self.segments.len() == 1 => return true,
        _ => return false,
      }
    }
    self.size == Some(next)
  }
  pub fn load(path: &Path, key: &StateKey) -> Result<Self> {
    let mut file = regular(path, false)?;
    if file.metadata()?.len() > 1024 * 1024 {
      return Err(Error::State);
    }
    let mut raw = Vec::new();
    std::io::Read::by_ref(&mut file)
      .take(1024 * 1024 + 1)
      .read_to_end(&mut raw)?;
    let mut state: Self = serde_json::from_slice(&raw).map_err(|_| Error::State)?;
    let checksum = std::mem::take(&mut state.checksum);
    let bytes = serde_json::to_vec(&state).map_err(|_| Error::State)?;
    if key.verify(&bytes, &checksum).is_err() || !state.valid() {
      return Err(Error::State);
    }
    state.checksum = checksum;
    Ok(state)
  }
  pub fn save(&self, path: &Path, part: &File, key: &StateKey) -> Result<()> {
    part.sync_data()?;
    let mut state = self.clone();
    state.checksum.clear();
    state.segments.sort_by_key(|s| s.start);
    state.checksum = key.sign(&serde_json::to_vec(&state).map_err(|_| Error::State)?)?;
    let bytes = serde_json::to_vec(&state).map_err(|_| Error::State)?;
    let temporary = sibling(path, &format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
      let mut opts = OpenOptions::new();
      opts.write(true).create_new(true);
      #[cfg(unix)]
      {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
      }
      let mut out = opts.open(&temporary)?;
      out.write_all(&bytes)?;
      out.sync_all()?;
      if path.symlink_metadata().is_ok() {
        regular(path, false)?;
      }
      std::fs::rename(&temporary, path)?;
      sync_directory(path.parent().ok_or(Error::State)?)?;
      Ok(())
    })();
    if result.is_err() {
      let _ = std::fs::remove_file(temporary);
    }
    result
  }
}
/// Durably retire all recorded offsets before any destructive part-file operation.
pub(crate) fn invalidate(path: &Path) -> Result<()> {
  match std::fs::remove_file(path) {
    Ok(()) => sync_directory(path.parent().ok_or(Error::State)?),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
    Err(e) => Err(e.into()),
  }
}
pub(crate) fn sync_directory(dir: &Path) -> Result<()> {
  #[cfg(unix)]
  if let Err(error) = File::open(dir)?.sync_all() {
    if !unsupported_publish(&error) {
      return Err(error.into());
    }
    tracing::warn!(error_kind=?error.kind(),"Filesystem does not support directory sync");
  }
  #[cfg(not(unix))]
  let _ = dir;
  Ok(())
}
pub(crate) struct Target {
  pub final_path: PathBuf,
  pub state_path: PathBuf,
  pub part_path: PathBuf,
  pub part: File,
  pub state: Option<State>,
}
impl Drop for Target {
  fn drop(&mut self) {
    // A cancelled/aborted reservation can precede its first authenticated checkpoint.
    // Remove only our still-identical, exclusively created file; checkpoints remain resumable.
    if self.state_path.symlink_metadata().is_err() {
      let _ = remove_owned(&self.part, &self.part_path);
    }
  }
}
pub(crate) fn resume(dir: &Path, identity: &str, key: &StateKey) -> Result<Option<Target>> {
  std::fs::create_dir_all(dir)?;
  let dir = std::fs::canonicalize(dir)?;
  // Do not trust any paths saved by another process; only names and URL digests are persisted.
  for entry in std::fs::read_dir(&dir)?.take(10_000) {
    let entry = entry?;
    let path = entry.path();
    let Some(name) = path
      .file_name()
      .and_then(|n| n.to_str())
      .and_then(|n| n.strip_suffix(STATE_SUFFIX))
    else {
      continue;
    };
    let Ok(state) = State::load(&path, key) else {
      continue;
    };
    if state.identity != identity || name != state.name {
      continue;
    }
    let final_path = dir.join(name);
    let part_path = sibling(&final_path, PART_SUFFIX);
    let part = match regular(&part_path, true) {
      Ok(part) => part,
      Err(_) => {
        std::fs::remove_file(&path)?;
        continue;
      }
    };
    match part.try_lock() {
      Ok(()) => {}
      Err(std::fs::TryLockError::WouldBlock) => return Err(Error::Busy),
      Err(_) => {
        std::fs::remove_file(&path)?;
        continue;
      }
    }
    if state
      .segments
      .iter()
      .any(|s| s.start + s.done > part.metadata().map_or(0, |m| m.len()))
    {
      std::fs::remove_file(&path)?;
      std::fs::remove_file(&part_path)?;
      continue;
    }
    return Ok(Some(Target {
      final_path,
      state_path: path,
      part_path,
      part,
      state: Some(state),
    }));
  }
  Ok(None)
}
pub(crate) fn target(dir: &Path, identity: &str, name: &str, key: &StateKey) -> Result<Target> {
  if name != sanitize_file_name(name) || reserved_name(name) {
    return Err(Error::State);
  }
  if let Some(target) = resume(dir, identity, key)? {
    return Ok(target);
  }
  let dir = std::fs::canonicalize(dir)?;
  for n in 0..=1000 {
    let candidate = alternative(name, n);
    let final_path = dir.join(candidate);
    let part_path = sibling(&final_path, PART_SUFFIX);
    let state_path = sibling(&final_path, STATE_SUFFIX);
    if final_path.symlink_metadata().is_ok() || state_path.symlink_metadata().is_ok() {
      continue;
    }
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      opts.mode(0o600);
    }
    match opts.open(&part_path) {
      Ok(part) => {
        part.try_lock().map_err(|e| match e {
          std::fs::TryLockError::WouldBlock => Error::Busy,
          _ => Error::Io,
        })?;
        return Ok(Target {
          final_path,
          state_path,
          part_path,
          part,
          state: None,
        });
      }
      Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
      Err(e) => return Err(e.into()),
    }
  }
  Err(Error::Busy)
}
pub(crate) fn alternative(name: &str, n: u32) -> String {
  if n == 0 {
    return name.into();
  }
  let suffix = format!(" ({n})");
  let (stem, extension) = name
    .rfind('.')
    .map(|i| (&name[..i], &name[i..]))
    .filter(|(_, ext)| ext.len() <= 21)
    .unwrap_or((name, ""));
  let budget = super::sanitize::MAX_NAME_BYTES - suffix.len() - extension.len();
  let mut end = stem.len().min(budget);
  while !stem.is_char_boundary(end) {
    end -= 1;
  }
  format!("{}{suffix}{extension}", &stem[..end])
}
pub(crate) fn native_publish(from: &Path, to: &Path) -> std::io::Result<()> {
  #[cfg(target_os = "macos")]
  {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())
      .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())
      .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    if unsafe {
      libc::renameatx_np(
        libc::AT_FDCWD,
        from.as_ptr(),
        libc::AT_FDCWD,
        to.as_ptr(),
        libc::RENAME_EXCL,
      )
    } != 0
    {
      return Err(std::io::Error::last_os_error());
    }
  }
  #[cfg(target_os = "linux")]
  {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())
      .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())
      .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    if unsafe {
      libc::renameat2(
        libc::AT_FDCWD,
        from.as_ptr(),
        libc::AT_FDCWD,
        to.as_ptr(),
        libc::RENAME_NOREPLACE,
      )
    } != 0
    {
      return Err(std::io::Error::last_os_error());
    }
  }
  #[cfg(windows)]
  {
    use std::os::windows::ffi::OsStrExt;
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
      windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
    } == 0
    {
      return Err(std::io::Error::last_os_error());
    }
  }
  #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
  return Err(std::io::ErrorKind::Unsupported.into());
  #[cfg(any(target_os = "macos", target_os = "linux", windows))]
  Ok(())
}

pub(crate) fn unsupported_publish(error: &std::io::Error) -> bool {
  if error.kind() == std::io::ErrorKind::Unsupported {
    return true;
  }
  #[cfg(unix)]
  if error
    .raw_os_error()
    .is_some_and(|n| [libc::ENOSYS, libc::EINVAL, libc::ENOTSUP].contains(&n))
  {
    return true;
  }
  #[cfg(windows)]
  if error
    .raw_os_error()
    .is_some_and(|n| [1, 50, 87].contains(&n))
  {
    return true;
  }
  false
}
pub(crate) fn publish(from: &Path, to: &Path) -> Result<()> {
  publish_with(from, to, native_publish, |from, to| {
    std::fs::hard_link(from, to)
  })
}
pub(crate) fn publish_with(
  from: &Path,
  to: &Path,
  rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
  link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
  if to
    .file_name()
    .and_then(|n| n.to_str())
    .is_none_or(reserved_name)
  {
    return Err(Error::State);
  }
  match rename(from, to) {
    Ok(()) => {}
    Err(e) if unsupported_publish(&e) => {
      match link(from, to) {
        Ok(()) => std::fs::remove_file(from)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(Error::Io),
        Err(_) => {
          let mut source = regular(from, false)?;
          let mut opts = OpenOptions::new();
          opts.read(true).write(true).create_new(true);
          #[cfg(unix)]
          {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
          }
          let mut destination = opts.open(to)?;
          // A replacing rename after a create-new check has a clobber race. Keep our
          // exclusive destination handle and copy instead; set origin metadata first.
          super::mark::mark(&destination, to);
          let result: Result<()> = (|| {
            std::io::copy(&mut source, &mut destination)?;
            destination.sync_all()?;
            Ok(())
          })();
          if result.is_err() {
            let _ = remove_owned(&destination, to);
          }
          result?;
          std::fs::remove_file(from)?;
        }
      }
    }
    Err(e) => return Err(e.into()),
  }
  sync_directory(to.parent().ok_or(Error::State)?)
}
pub(crate) fn remove_owned(file: &File, path: &Path) -> Result<()> {
  let other = regular(path, false)?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    let (a, b) = (file.metadata()?, other.metadata()?);
    if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
      return Err(Error::State);
    }
  }
  #[cfg(windows)]
  {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
      GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let identity = |f: &File| -> Result<(u32, u32, u32)> {
      let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
      if unsafe { GetFileInformationByHandle(f.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
        return Err(Error::Io);
      }
      let info = unsafe { info.assume_init() };
      Ok((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
      ))
    };
    if identity(file)? != identity(&other)? {
      return Err(Error::State);
    }
  }
  drop(other);
  std::fs::remove_file(path)?;
  Ok(())
}

pub(crate) fn discard(dir: &Path, identity: &str, owner: &str, key: &StateKey) -> Result<()> {
  if !dir.is_dir() {
    return Ok(());
  }
  let dir = std::fs::canonicalize(dir)?;
  for entry in std::fs::read_dir(&dir)?.take(10_000) {
    let path = entry?.path();
    let Some(name) = path
      .file_name()
      .and_then(|n| n.to_str())
      .and_then(|n| n.strip_suffix(STATE_SUFFIX))
    else {
      continue;
    };
    let Ok(state) = State::load(&path, key) else {
      continue;
    };
    if state.identity != identity || state.owner != owner || state.name != name {
      continue;
    }
    let part_path = sibling(&dir.join(name), PART_SUFFIX);
    match regular(&part_path, true) {
      Ok(part) => {
        part.try_lock().map_err(|e| match e {
          std::fs::TryLockError::WouldBlock => Error::Busy,
          _ => Error::Io,
        })?;
        remove_owned(&part, &part_path)?;
      }
      Err(_) if part_path.symlink_metadata().is_err() => {}
      Err(e) => return Err(e),
    }
    std::fs::remove_file(path)?;
  }
  sync_directory(&dir)
}
