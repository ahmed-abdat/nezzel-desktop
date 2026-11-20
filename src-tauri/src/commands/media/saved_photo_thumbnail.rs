use crate::download_records::{identity_of, resolve_trash_target, DownloadRecords, GroupRecord};
use image::{ImageFormat, ImageReader};
use std::{
  fs::{File, OpenOptions},
  io::{Cursor, Read},
  path::{Component, Path},
};
use tauri::State;

const MAX_PHOTO_BYTES: u64 = 40 * 1024 * 1024;
// Error codes only: filesystem/decoder errors can contain private paths.
const REFUSED: &str = "thumbnailUnavailable";

/// The path is a child row's identity, never permission to read an arbitrary local file.
#[tauri::command]
pub async fn saved_photo_thumbnail(
  group_id: String,
  path: String,
  records: State<'_, DownloadRecords>,
) -> Result<Vec<u8>, String> {
  let record = records.get(&group_id);
  tauri::async_runtime::spawn_blocking(move || thumbnail(&record, Path::new(&path)))
    .await
    .map_err(|_| REFUSED.to_owned())?
    .map_err(str::to_owned)
}

fn open_photo(record: &GroupRecord, requested: &Path) -> Result<File, &'static str> {
  if requested.components().any(|c| c == Component::ParentDir) {
    return Err(REFUSED);
  }
  // Resolve the engine's stored path, not the caller's spelling or canonical alias.
  let (recorded, _) = record
    .files
    .iter()
    .find(|(path, _)| path.as_os_str() == requested.as_os_str())
    .ok_or(REFUSED)?;
  let extension = recorded
    .extension()
    .and_then(|ext| ext.to_str())
    .ok_or(REFUSED)?;
  if !["jpg", "jpeg", "png", "webp"]
    .iter()
    .any(|ext| extension.eq_ignore_ascii_case(ext))
  {
    return Err(REFUSED);
  }
  let target = resolve_trash_target(record, recorded).map_err(|_| REFUSED)?;
  let mut options = OpenOptions::new();
  options.read(true);
  // Refuse a symlink swapped in between canonicalization and open. Check the opened file's
  // identity too, including replacements or symlinks in a parent directory.
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
  }
  #[cfg(windows)]
  {
    use std::os::windows::fs::OpenOptionsExt;
    options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
  }
  let file = options.open(&target.path).map_err(|_| REFUSED)?;
  let meta = file.metadata().map_err(|_| REFUSED)?;
  if !meta.is_file() || identity_of(&meta) != target.identity || meta.len() > MAX_PHOTO_BYTES {
    return Err(REFUSED);
  }
  Ok(file)
}

fn thumbnail(record: &GroupRecord, requested: &Path) -> Result<Vec<u8>, &'static str> {
  let file = open_photo(record, requested)?;
  let mut bytes = Vec::new();
  file
    .take(MAX_PHOTO_BYTES + 1)
    .read_to_end(&mut bytes)
    .map_err(|_| REFUSED)?;
  if bytes.len() as u64 > MAX_PHOTO_BYTES {
    return Err(REFUSED);
  }
  let mut reader = ImageReader::new(Cursor::new(bytes))
    .with_guessed_format()
    .map_err(|_| REFUSED)?;
  // A small file can still decode to a huge bitmap; several rows decode at once.
  let mut limits = image::Limits::default();
  limits.max_image_width = Some(12_000);
  limits.max_image_height = Some(12_000);
  limits.max_alloc = Some(128 * 1024 * 1024);
  reader.limits(limits);
  if !matches!(
    reader.format(),
    Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP)
  ) {
    return Err(REFUSED);
  }
  let photo = reader.decode().map_err(|_| REFUSED)?;
  let preview = photo.thumbnail(96, 96).to_rgb8();
  let mut jpeg = Cursor::new(Vec::new());
  preview
    .write_to(&mut jpeg, ImageFormat::Jpeg)
    .map_err(|_| REFUSED)?;
  Ok(jpeg.into_inner())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::path::PathBuf;

  struct PhotoDir(PathBuf);
  impl PhotoDir {
    fn new() -> Self {
      let path = std::env::temp_dir().join(format!("nezzel-thumbnail-{}", uuid::Uuid::new_v4()));
      fs::create_dir(&path).unwrap();
      Self(path.canonicalize().unwrap())
    }
    fn record(&self, path: &Path) -> GroupRecord {
      let records = DownloadRecords::default();
      records.allow_root("group", self.0.clone());
      records.record_item("group", "item", path.to_str().unwrap());
      records.seal("group");
      records.get("group")
    }
  }
  impl Drop for PhotoDir {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn thumbnail_path_gate() {
    let dir = PhotoDir::new();
    let saved = dir.0.join("saved.jpg");
    let other = dir.0.join("other.jpg");
    let text = dir.0.join("note.txt");
    let large = dir.0.join("large.png");
    let missing = dir.0.join("missing.webp");
    for path in [&saved, &other, &text, &large, &missing] {
      fs::write(path, b"photo").unwrap();
    }
    File::options()
      .write(true)
      .open(&large)
      .unwrap()
      .set_len(MAX_PHOTO_BYTES + 1)
      .unwrap();
    let saved_record = dir.record(&saved);
    let missing_record = dir.record(&missing);
    fs::remove_file(&missing).unwrap();
    fs::create_dir(dir.0.join("nested")).unwrap();
    let traversal = dir.0.join("nested/../saved.jpg");
    let cases = [
      ("recorded", saved_record.clone(), saved.clone(), true),
      ("different path", saved_record.clone(), other, false),
      ("traversal", saved_record, traversal, false),
      ("missing", missing_record, missing, false),
      ("non-image", dir.record(&text), text, false),
      ("oversized", dir.record(&large), large, false),
      ("wrong group", GroupRecord::default(), saved.clone(), false),
      (
        "unsealed",
        {
          let mut record = dir.record(&saved);
          record.files.insert(saved.clone(), None);
          record
        },
        saved,
        false,
      ),
    ];
    for (name, record, path, allowed) in cases {
      assert_eq!(open_photo(&record, &path).is_ok(), allowed, "{name}");
    }
  }

  #[cfg(unix)]
  #[test]
  fn thumbnail_refuses_symlink_outside_recorded_file() {
    let dir = PhotoDir::new();
    let outside = PhotoDir::new();
    let saved = dir.0.join("saved.jpg");
    let secret = outside.0.join("outside.jpg");
    fs::write(&saved, b"photo").unwrap();
    fs::write(&secret, b"private").unwrap();
    let record = dir.record(&saved);
    fs::remove_file(&saved).unwrap();
    std::os::unix::fs::symlink(secret, &saved).unwrap();
    assert!(open_photo(&record, &saved).is_err());
  }

  #[test]
  fn thumbnail_decodes_to_small_jpeg() {
    let dir = PhotoDir::new();
    for (width, height) in [(192, 48), (48, 192), (8, 4)] {
      for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        let path = dir.0.join(format!("photo.{}", format.extensions_str()[0]));
        image::RgbImage::from_pixel(width, height, image::Rgb([20, 130, 90]))
          .save_with_format(&path, format)
          .unwrap();
        let bytes = thumbnail(&dir.record(&path), &path).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Jpeg);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert!(decoded.width() <= 96 && decoded.height() <= 96);
        assert_eq!(decoded.width() * height, decoded.height() * width);
      }
    }
    let corrupt = dir.0.join("corrupt.png");
    fs::write(&corrupt, b"not an image").unwrap();
    assert!(thumbnail(&dir.record(&corrupt), &corrupt).is_err());
  }
}
