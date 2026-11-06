//! One file-type vocabulary (extension / MIME) shared by URL routing, the files probe, the gallery
//! parser and the resolver. Leaf module: imports nothing from the crate. Each caller keeps its own
//! policy (which sets it consults, and in what order); only the sets live here.

/// Streaming manifests and playlists.
pub(crate) const MANIFEST_EXTS: &[&str] = &["m3u8", "m3u", "mpd"];
/// Plain audio/video containers.
pub(crate) const MEDIA_EXTS: &[&str] = &[
  "mp4", "mp3", "webm", "m4a", "ogg", "flac", "wav", "mov", "mkv",
];
/// Downloadable files (archives, documents, installers, images, fonts).
pub(crate) const FILE_EXTS: &[&str] = &[
  "zip", "pdf", "exe", "msi", "dmg", "iso", "bin", "7z", "rar", "gz", "tgz", "bz2", "xz", "tar",
  "zst", "deb", "rpm", "appimage", "apk", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "epub",
  "txt", "csv", "json", "xml", "jpg", "jpeg", "png", "gif", "webp", "avif", "svg", "psd", "ttf",
  "otf", "woff", "woff2", "torrent", "img",
];
/// Still images the gallery parser accepts.
pub(crate) const IMAGE_EXTS: &[&str] =
  &["jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "bmp"];
/// Video extensions the gallery parser accepts.
pub(crate) const GALLERY_VIDEO_EXTS: &[&str] = &["mp4", "webm", "mov", "m4v", "mkv", "m3u8"];

/// MIME types of HLS / DASH manifests.
pub(crate) const MANIFEST_MIMES: &[&str] = &[
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "application/dash+xml",
];
/// Image MIME types the files probe treats as downloadable files.
pub(crate) const FILE_IMAGE_MIMES: &[&str] = &[
  "image/jpeg",
  "image/png",
  "image/webp",
  "image/gif",
  "image/avif",
];

/// Case-insensitive membership of `ext` (no dot) in one of the extension sets.
pub(crate) fn ext_in(set: &[&str], ext: &str) -> bool {
  set.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// MIME without parameters, trimmed and lowercased: `"Text/HTML; charset=x"` -> `"text/html"`.
pub(crate) fn base_mime(mime: &str) -> String {
  mime
    .split(';')
    .next()
    .unwrap_or_default()
    .trim()
    .to_ascii_lowercase()
}

/// `base` is an already-normalised MIME (see `base_mime`).
pub(crate) fn is_manifest_mime(base: &str) -> bool {
  MANIFEST_MIMES.contains(&base)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn extension_sets() {
    let cases: &[(&[&str], &str, bool)] = &[
      (MANIFEST_EXTS, "M3U8", true),
      (MANIFEST_EXTS, "mp4", false),
      (MEDIA_EXTS, "MKV", true),
      (MEDIA_EXTS, "m3u8", false),
      (FILE_EXTS, "pdf", true),
      (FILE_EXTS, "mp4", false),
      (IMAGE_EXTS, "heic", true),
      (IMAGE_EXTS, "svg", false),
      (GALLERY_VIDEO_EXTS, "m4v", true),
      (GALLERY_VIDEO_EXTS, "mp3", false),
    ];
    for (set, ext, want) in cases {
      assert_eq!(ext_in(set, ext), *want, "{ext}");
    }
  }

  #[test]
  fn mime_normalisation() {
    let cases = [
      ("Text/HTML; charset=utf-8", "text/html"),
      ("  video/mp4 ", "video/mp4"),
      ("", ""),
    ];
    for (input, want) in cases {
      assert_eq!(base_mime(input), want, "{input}");
    }
    assert!(is_manifest_mime("application/dash+xml"));
    assert!(!is_manifest_mime("audio/mpegurl"));
  }
}
