//! Internet-origin marks follow Zajil's platform design (GPL-3.0-or-later).
use super::Result;
use std::{fs::File, path::Path};
pub(crate) fn mark(file: &File, path: &Path) {
  // FAT/exFAT and some network shares do not support origin metadata. Like browsers,
  // keep the completed download; never include its path or URL in the warning.
  if let Err(error) = try_mark(file, path) {
    tracing::warn!(
      code = error.code(),
      "Internet-origin metadata could not be written"
    );
  }
}
fn try_mark(file: &File, path: &Path) -> Result<()> {
  #[cfg(target_os = "macos")]
  {
    use std::{
      os::fd::AsRawFd,
      time::{SystemTime, UNIX_EPOCH},
    };
    let seconds = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    let value = format!("0081;{seconds:08x};Nezzel;");
    if unsafe {
      libc::fsetxattr(
        file.as_raw_fd(),
        c"com.apple.quarantine".as_ptr(),
        value.as_ptr().cast(),
        value.len(),
        0,
        0,
      )
    } != 0
    {
      return Err(super::Error::Io);
    }
    let _ = path;
  }
  #[cfg(windows)]
  {
    use std::io::Write;
    let mut ads = path.as_os_str().to_owned();
    ads.push(":Zone.Identifier");
    let mut out = File::create(std::path::PathBuf::from(ads))?;
    out.write_all(b"[ZoneTransfer]\r\nZoneId=3\r\n")?;
    out.sync_all()?;
    let _ = file;
  }
  #[cfg(not(any(target_os = "macos", windows)))]
  let _ = (file, path);
  Ok(())
}
pub(crate) fn free_space(path: &Path, needed: u64) -> Result<()> {
  #[cfg(unix)]
  {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| super::Error::Io)?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
      return Err(super::Error::Io);
    }
    let stat = unsafe { stat.assume_init() };
    if (stat.f_bavail as u128) * (stat.f_frsize as u128) < needed as u128 {
      return Err(super::Error::Space);
    }
  }
  #[cfg(windows)]
  {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    if unsafe {
      windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
        path.as_ptr(),
        &mut available,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
      )
    } == 0
    {
      return Err(super::Error::Io);
    }
    if available < needed {
      return Err(super::Error::Space);
    }
  }
  Ok(())
}
