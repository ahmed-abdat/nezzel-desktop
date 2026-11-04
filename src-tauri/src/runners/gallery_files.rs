use crate::parsers::gallery::GalleryError;
use std::{
  io::{Read, Write},
  path::{Path, PathBuf},
};

pub struct Staging {
  pub path: PathBuf,
}
impl Staging {
  pub fn new() -> Result<Self, GalleryError> {
    let path = std::env::temp_dir().join(format!("nezzel-images-{}", uuid::Uuid::new_v4()));
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
      use std::os::unix::fs::DirBuilderExt;
      builder.mode(0o700);
    }
    builder.create(&path).map_err(|_| GalleryError::Invalid)?;
    Ok(Self { path })
  }
  pub fn json(&self, filename: &str, value: &serde_json::Value) -> Result<PathBuf, GalleryError> {
    let path = self.path.join(filename);
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|_| GalleryError::Invalid)?;
    serde_json::to_writer(&mut file, value).map_err(|_| GalleryError::Invalid)?;
    Ok(path)
  }
}
impl Drop for Staging {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

pub fn image_type(data: &[u8]) -> Result<&'static str, GalleryError> {
  if data.len() > 128 * 1024 * 1024 {
    return Err(GalleryError::Invalid);
  }
  if data.starts_with(b"\x89PNG\r\n\x1a\n") {
    let mut offset = 8;
    let mut header = false;
    let mut pixels = false;
    while offset + 12 <= data.len() {
      let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
      let end = offset
        .checked_add(12 + size)
        .filter(|end| *end <= data.len())
        .ok_or(GalleryError::Invalid)?;
      let kind = &data[offset + 4..offset + 8];
      let expected = u32::from_be_bytes(data[end - 4..end].try_into().unwrap());
      let mut crc = !0_u32;
      for byte in &data[offset + 4..end - 4] {
        crc ^= *byte as u32;
        for _ in 0..8 {
          crc = (crc >> 1) ^ (0xedb88320 & (0_u32.wrapping_sub(crc & 1)));
        }
      }
      if !crc != expected {
        return Err(GalleryError::Invalid);
      }
      if kind == b"IHDR" {
        if header
          || offset != 8
          || size != 13
          || data[offset + 8..offset + 12] == [0; 4]
          || data[offset + 12..offset + 16] == [0; 4]
        {
          return Err(GalleryError::Invalid);
        }
        header = true;
      }
      if kind == b"IDAT" && size > 0 {
        pixels = true;
      }
      if kind == b"IEND" {
        return if header && pixels && size == 0 && end == data.len() {
          Ok("png")
        } else {
          Err(GalleryError::Invalid)
        };
      }
      offset = end;
    }
  } else if data.starts_with(&[0xff, 0xd8]) && data.ends_with(&[0xff, 0xd9]) {
    let mut offset = 2;
    while offset + 4 <= data.len() && data[offset] == 0xff {
      let marker = data[offset + 1];
      let size = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
      if size < 2 || offset + 2 + size > data.len() {
        break;
      }
      if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
        && size >= 8
        && data[offset + 5..offset + 7] != [0, 0]
        && data[offset + 7..offset + 9] != [0, 0]
      {
        return Ok("jpg");
      }
      offset += size + 2;
    }
  } else if data.len() > 20
    && data.starts_with(b"RIFF")
    && &data[8..12] == b"WEBP"
    && u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize + 8 == data.len()
    && matches!(&data[12..16], b"VP8 " | b"VP8L" | b"VP8X")
  {
    return Ok("webp");
  } else if data.len() > 13
    && (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"))
    && data.last() == Some(&0x3b)
    && data[6..8] != [0, 0]
    && data[8..10] != [0, 0]
  {
    return Ok("gif");
  } else if data.len() >= 54
    && data.starts_with(b"BM")
    && u32::from_le_bytes(data[2..6].try_into().unwrap()) as usize == data.len()
  {
    return Ok("bmp");
  } else if data.len() > 32 && &data[4..8] == b"ftyp" {
    let ftyp = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
    if ftyp >= 16 && ftyp <= data.len() && data[ftyp..].windows(4).any(|v| v == b"mdat") {
      if data[8..ftyp]
        .as_chunks::<4>()
        .0
        .iter()
        .any(|v| v == b"avif" || v == b"avis")
      {
        return Ok("avif");
      }
      if data[8..ftyp]
        .as_chunks::<4>()
        .0
        .iter()
        .any(|v| matches!(v, b"heic" | b"heix" | b"mif1"))
      {
        return Ok("heic");
      }
    }
  }
  Err(GalleryError::Invalid)
}

pub fn read_image(path: &Path) -> Result<Vec<u8>, GalleryError> {
  let metadata = std::fs::symlink_metadata(path).map_err(|_| GalleryError::Invalid)?;
  if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
    return Err(GalleryError::Invalid);
  }
  let mut bytes = Vec::new();
  std::fs::File::open(path)
    .map_err(|_| GalleryError::Invalid)?
    .take(128 * 1024 * 1024 + 1)
    .read_to_end(&mut bytes)
    .map_err(|_| GalleryError::Invalid)?;
  image_type(&bytes)?;
  Ok(bytes)
}

pub fn finalize(
  source: &Path,
  destination: &Path,
  cancelled: impl Fn() -> bool,
) -> Result<(), GalleryError> {
  finalize_with_link(source, destination, cancelled, |from, to| {
    std::fs::hard_link(from, to)
  })
}

fn finalize_with_link(
  source: &Path,
  destination: &Path,
  cancelled: impl Fn() -> bool,
  link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), GalleryError> {
  if cancelled() {
    return Err(GalleryError::Cancelled);
  }
  let parent = destination.parent().ok_or(GalleryError::Invalid)?;
  std::fs::create_dir_all(parent).map_err(|_| GalleryError::Refused("permissionDenied"))?;
  let temp = parent.join(format!(".nezzel-{}.part", uuid::Uuid::new_v4()));
  let result = (|| {
    let bytes = read_image(source)?;
    let mut file = std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(&temp)
      .map_err(|_| GalleryError::Refused("permissionDenied"))?;
    file
      .write_all(&bytes)
      .and_then(|_| file.sync_all())
      .map_err(|_| GalleryError::Refused("permissionDenied"))?;
    if cancelled() {
      return Err(GalleryError::Cancelled);
    }
    // An atomic create without replacement: an existing file/symlink is never overwritten.
    match link(&temp, destination) {
      Ok(()) => Ok(()),
      Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
        // A retry can recognize its own completed file without changing any existing bytes.
        if std::fs::symlink_metadata(destination)
          .is_ok_and(|meta| meta.is_file() && meta.len() == bytes.len() as u64)
          && read_image(destination).is_ok_and(|previous| previous == bytes)
        {
          Ok(())
        } else {
          Err(GalleryError::Refused("imageFileConflict"))
        }
      }
      Err(_) => {
        // FAT/exFAT and some network shares have no hard-link support. Claim
        // the final name exclusively; never copy into an existing file.
        let mut final_file = match std::fs::OpenOptions::new()
          .write(true)
          .create_new(true)
          .open(destination)
        {
          Ok(file) => file,
          Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return if std::fs::symlink_metadata(destination)
              .is_ok_and(|m| m.is_file() && m.len() == bytes.len() as u64)
              && read_image(destination).is_ok_and(|previous| previous == bytes)
            {
              Ok(())
            } else {
              Err(GalleryError::Refused("imageFileConflict"))
            };
          }
          Err(_) => return Err(GalleryError::Refused("permissionDenied")),
        };
        let copied = (|| {
          for chunk in bytes.chunks(64 * 1024) {
            if cancelled() {
              return Err(GalleryError::Cancelled);
            }
            final_file
              .write_all(chunk)
              .map_err(|_| GalleryError::Refused("permissionDenied"))?;
          }
          final_file
            .sync_all()
            .map_err(|_| GalleryError::Refused("permissionDenied"))?;
          if cancelled() {
            return Err(GalleryError::Cancelled);
          }
          Ok(())
        })();
        drop(final_file);
        if copied.is_err() {
          let _ = std::fs::remove_file(destination);
        }
        copied
      }
    }
  })();
  let _ = std::fs::remove_file(&temp);
  result
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn validates_bytes_and_never_overwrites_an_existing_file() {
    let stage = Staging::new().unwrap();
    let source = stage.path.join("source.png");
    let destination = stage.path.join("saved.png");
    let png = include_bytes!("../../../src/assets/placeholder.png");
    std::fs::write(&source, png).unwrap();
    assert_eq!(image_type(png).unwrap(), "png");
    finalize(&source, &destination, || false).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), png);
    finalize(&source, &destination, || false).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), png);
    std::fs::write(&destination, b"an unrelated existing file").unwrap();
    assert!(finalize(&source, &destination, || false).is_err());
    assert_eq!(
      std::fs::read(&destination).unwrap(),
      b"an unrelated existing file"
    );
    assert!(finalize(&source, &stage.path.join("cancelled.png"), || true).is_err());
    assert!(!stage.path.join("cancelled.png").exists());
    assert!(image_type(b"<html>login required</html>").is_err());
    let mut corrupted = png.to_vec();
    corrupted[30] ^= 1;
    assert!(image_type(&corrupted).is_err());
  }

  #[test]
  fn filesystems_without_hard_links_can_save_without_replacing_existing_files() {
    let stage = Staging::new().unwrap();
    let source = stage.path.join("source.png");
    let destination = stage.path.join("saved.png");
    let png = include_bytes!("../../../src/assets/placeholder.png");
    std::fs::write(&source, png).unwrap();
    let no_links = |_: &Path, _: &Path| Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
    finalize_with_link(&source, &destination, || false, no_links).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), png);
    finalize_with_link(&source, &destination, || false, no_links).unwrap();
    std::fs::write(&destination, b"unrelated").unwrap();
    assert!(finalize_with_link(&source, &destination, || false, no_links).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"unrelated");
    let checks = std::cell::Cell::new(0);
    let cancelled = stage.path.join("cancelled-copy.png");
    assert_eq!(
      finalize_with_link(
        &source,
        &cancelled,
        || {
          checks.set(checks.get() + 1);
          checks.get() >= 3
        },
        no_links
      ),
      Err(GalleryError::Cancelled)
    );
    assert!(!cancelled.exists());
  }
}
