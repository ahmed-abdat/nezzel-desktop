//! Resume-state authentication. The random installation key never enters a download folder.
use super::{state, Error, Result};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::{
  fs::OpenOptions,
  io::{Read, Write},
  path::{Path, PathBuf},
  sync::Mutex,
};
static KEY_INIT: Mutex<()> = Mutex::new(());
#[derive(Clone)]
pub(crate) struct StateKey([u8; 32]);
impl StateKey {
  #[cfg(test)]
  pub fn ephemeral() -> Self {
    Self(rand::random())
  }
  pub fn sign(&self, bytes: &[u8]) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).map_err(|_| Error::State)?;
    mac.update(bytes);
    Ok(format!(
      "hmac-sha256:{}",
      hex::encode(mac.finalize().into_bytes())
    ))
  }
  pub fn verify(&self, bytes: &[u8], tag: &str) -> Result<()> {
    let tag = tag.strip_prefix("hmac-sha256:").ok_or(Error::State)?;
    let tag = hex::decode(tag).map_err(|_| Error::State)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).map_err(|_| Error::State)?;
    mac.update(bytes);
    mac.verify_slice(&tag).map_err(|_| Error::State)
  }
  pub fn load(root: &Path) -> Result<Self> {
    let _guard = KEY_INIT.lock().map_err(|_| Error::State)?;
    std::fs::create_dir_all(root)?;
    if !std::fs::symlink_metadata(root)?.file_type().is_dir() {
      return Err(Error::State);
    }
    #[cfg(unix)]
    {
      use std::os::unix::fs::{MetadataExt, PermissionsExt};
      if std::fs::metadata(root)?.uid() != unsafe { libc::geteuid() } {
        return Err(Error::State);
      }
      std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = root.join("resume.key");
    if path.symlink_metadata().is_err() {
      let temporary = root.join(format!(".resume-key-{}.tmp", uuid::Uuid::new_v4()));
      let result = (|| {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
          use std::os::unix::fs::OpenOptionsExt;
          opts.mode(0o600);
        }
        let mut file = opts.open(&temporary)?;
        let bytes: [u8; 32] = rand::random();
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        match state::publish(&temporary, &path) {
          Ok(()) => Ok(()),
          Err(_) if path.symlink_metadata().is_ok() => Ok(()),
          Err(e) => Err(e),
        }
      })();
      let _ = std::fs::remove_file(temporary);
      result?;
    }
    let mut file = state::regular(&path, false)?;
    #[cfg(unix)]
    {
      use std::os::unix::fs::{MetadataExt, PermissionsExt};
      let meta = file.metadata()?;
      if meta.permissions().mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } {
        return Err(Error::State);
      }
    }
    if file.metadata()?.len() != 32 {
      return Err(Error::State);
    }
    let mut bytes = [0; 32];
    file.read_exact(&mut bytes)?;
    Ok(Self(bytes))
  }
}
pub(crate) fn default_root() -> Result<PathBuf> {
  dirs::data_dir()
    .map(|p| p.join("dev.ahmed.nezzel").join("file-engine"))
    .ok_or(Error::Io)
}
