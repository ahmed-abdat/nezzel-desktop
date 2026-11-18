use std::{io, path::Path};

pub(crate) fn required(estimate: Option<u64>) -> u64 {
  estimate
    .filter(|size| *size > 0)
    .map(|size| size.saturating_add(size.div_ceil(10)))
    .unwrap_or(200 * 1024 * 1024)
}
pub(crate) fn check(path: &Path, estimate: Option<u64>) -> Result<(), &'static str> {
  let existing = path
    .ancestors()
    .find(|path| path.exists())
    .ok_or("diskSpaceUnavailable")?;
  let free = available(existing).map_err(|_| "diskSpaceUnavailable")?;
  check_available(free, estimate)
}
fn check_available(free: u64, estimate: Option<u64>) -> Result<(), &'static str> {
  if free < required(estimate) {
    Err("diskFull")
  } else {
    Ok(())
  }
}
#[cfg(unix)]
fn available(path: &Path) -> io::Result<u64> {
  use std::os::unix::ffi::OsStrExt;
  let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
  let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
  // SAFETY: NUL-terminated path and writable output buffer; only read on success.
  if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
    return Err(io::Error::last_os_error());
  }
  let stats = unsafe { stats.assume_init() };
  #[allow(clippy::unnecessary_cast)] // statvfs field widths vary across supported Unix targets.
  Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}
#[cfg(windows)]
fn available(path: &Path) -> io::Result<u64> {
  use std::os::windows::ffi::OsStrExt;
  let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
  let mut free = 0;
  // SAFETY: terminated UTF-16 input, valid output, unused outputs explicitly null.
  if unsafe {
    windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
      path.as_ptr(),
      &mut free,
      std::ptr::null_mut(),
      std::ptr::null_mut(),
    )
  } == 0
  {
    return Err(io::Error::last_os_error());
  }
  Ok(free)
}

/// Private, per-attempt plugin; synchronous hooks run before transfer and each processor.
pub(crate) fn plugin(root: &Path) -> Result<crate::runners::gallery_files::Staging, String> {
  let stage = crate::runners::gallery_files::Staging::new().map_err(|_| "diskSpaceUnavailable")?;
  let path = stage.path.join("nezzel/yt_dlp_plugins/postprocessor");
  std::fs::create_dir_all(&path).map_err(|_| "diskSpaceUnavailable")?;
  #[cfg(unix)]
  let root = {
    use std::os::unix::ffi::OsStrExt;
    format!(
      "bytes({})",
      serde_json::to_string(root.as_os_str().as_bytes()).map_err(|_| "diskSpaceUnavailable")?
    )
  };
  #[cfg(not(unix))]
  let root = serde_json::to_string(root.to_str().ok_or("diskSpaceUnavailable")?)
    .map_err(|_| "diskSpaceUnavailable")?;
  let source = format!(
    "NEZZEL_ROOT = {root}\n{}",
    include_str!("../runners/disk_guard.py")
  );
  std::fs::write(path.join("nezzel_disk.py"), source).map_err(|_| "diskSpaceUnavailable")?;
  Ok(stage)
}

#[cfg(test)]
mod tests {
  use super::*;
  #[cfg(unix)]
  #[test]
  fn plugin_checks_the_original_non_utf8_volume_path() {
    use std::os::unix::ffi::OsStringExt;
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&root).unwrap();
    let path = root.join(std::ffi::OsString::from_vec(b"volume-\xff".to_vec()));
    let plugin = plugin(&path).unwrap();
    let source = std::fs::read_to_string(
      plugin
        .path
        .join("nezzel/yt_dlp_plugins/postprocessor/nezzel_disk.py"),
    )
    .unwrap();
    let script = format!(
      "{}\nimport os, sys\nsys.stdout.buffer.write(os.fsencode(NEZZEL_ROOT))",
      source.lines().next().unwrap()
    );
    let output = std::process::Command::new("python3")
      .args(["-c", &script])
      .output()
      .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
      output.status.success(),
      "{}",
      String::from_utf8_lossy(&output.stderr)
    );
    use std::os::unix::ffi::OsStrExt;
    assert_eq!(output.stdout, path.as_os_str().as_bytes());
  }
  #[test]
  fn disk_budget_table() {
    for (estimate, expected) in [
      (None, 209_715_200),
      (Some(0), 209_715_200),
      (Some(100), 110),
      (Some(101), 112),
      (Some(u64::MAX), u64::MAX),
    ] {
      assert_eq!(required(estimate), expected);
      assert_eq!(check_available(expected, estimate), Ok(()));
      assert_eq!(check_available(expected - 1, estimate), Err("diskFull"));
    }
  }
  #[test]
  fn target_volume_is_checked_even_when_folder_does_not_exist() {
    let root = std::env::temp_dir();
    assert!(check(&root.join("nezzel-not-created/child"), Some(1)).is_ok());
    assert!(check(&root, Some(u64::MAX)).is_err());
  }
}
