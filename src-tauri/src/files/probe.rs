//! The first GET is a probe and remains open as segment zero.
use super::{client::Response, names::suggested_name, plan::Validator, Error, Result};
use reqwest::Url;
#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
  File,
  Page,
  Media,
  Unknown,
}
pub fn classify(url: &Url, mime: &str, disposition: Option<&[u8]>) -> Kind {
  let mime = mime
    .split(';')
    .next()
    .unwrap_or_default()
    .trim()
    .to_ascii_lowercase();
  let ext = url
    .path()
    .rsplit('.')
    .next()
    .unwrap_or_default()
    .to_ascii_lowercase();
  if mime.starts_with("video/")
    || mime.starts_with("audio/")
    || matches!(
      mime.as_str(),
      "application/vnd.apple.mpegurl"
        | "application/x-mpegurl"
        | "application/dash+xml"
        | "application/rss+xml"
        | "application/atom+xml"
    )
    || matches!(ext.as_str(), "m3u8" | "m3u" | "mpd")
  {
    return Kind::Media;
  }
  let attachment = disposition.is_some_and(|d| {
    let value = String::from_utf8_lossy(d);
    let mode = value.split(';').next().unwrap_or_default().trim();
    mode.eq_ignore_ascii_case("attachment")
  });
  if matches!(mime.as_str(), "text/html" | "application/xhtml+xml") && !attachment {
    Kind::Page
  } else if attachment
    || file_extension(url)
    || matches!(
      mime.as_str(),
      "application/pdf"
        | "application/zip"
        | "application/x-zip-compressed"
        | "application/x-msdownload"
        | "application/vnd.microsoft.portable-executable"
        | "application/x-7z-compressed"
        | "application/x-rar-compressed"
        | "application/vnd.rar"
        | "application/gzip"
        | "application/x-gzip"
        | "application/x-apple-diskimage"
        | "application/x-iso9660-image"
        | "application/x-tar"
        | "image/jpeg"
        | "image/png"
        | "image/webp"
        | "image/gif"
        | "image/avif"
    )
  {
    Kind::File
  } else {
    Kind::Unknown
  }
}
pub(crate) fn file_extension(url: &Url) -> bool {
  let decoded = super::names::percent_bytes(url.path());
  let path = String::from_utf8_lossy(&decoded);
  let Some((stem, extension)) = path.rsplit('/').next().unwrap_or_default().rsplit_once('.') else {
    return false;
  };
  if stem.is_empty() {
    return false;
  }
  matches!(
    extension.to_ascii_lowercase().as_str(),
    "zip"
      | "pdf"
      | "exe"
      | "msi"
      | "dmg"
      | "iso"
      | "bin"
      | "7z"
      | "rar"
      | "gz"
      | "tgz"
      | "bz2"
      | "xz"
      | "tar"
      | "zst"
      | "deb"
      | "rpm"
      | "appimage"
      | "apk"
      | "doc"
      | "docx"
      | "xls"
      | "xlsx"
      | "ppt"
      | "pptx"
      | "epub"
      | "txt"
      | "csv"
      | "json"
      | "xml"
      | "jpg"
      | "jpeg"
      | "png"
      | "gif"
      | "webp"
      | "avif"
      | "svg"
      | "psd"
      | "ttf"
      | "otf"
      | "woff"
      | "woff2"
      | "torrent"
      | "img"
  )
}
/// Known extractor sites and host-share pages retain their current routing.
pub(crate) fn should_probe_metadata(input: &str, total: usize, has_format: bool) -> bool {
  total <= 1
    && !has_format
    && should_probe(input)
    && Url::parse(input).is_ok_and(|u| file_extension(&u))
}
pub fn should_probe(input: &str) -> bool {
  let Ok(url) = Url::parse(input) else {
    return false;
  };
  if !matches!(url.scheme(), "http" | "https") {
    return false;
  }
  let host = url
    .host_str()
    .unwrap_or_default()
    .trim_start_matches("www.");
  if [
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "vimeo.com",
    "soundcloud.com",
    "tiktok.com",
    "x.com",
    "twitter.com",
    "instagram.com",
    "reddit.com",
    "redd.it",
    "bsky.app",
    "pinterest.com",
    "pin.it",
    "facebook.com",
    "fb.watch",
    "threads.net",
    "threads.com",
    "linkedin.com",
    "t.me",
    "telegram.me",
    "snapchat.com",
    "twitch.tv",
    "dailymotion.com",
    "drive.google.com",
    "docs.google.com",
    "dropbox.com",
  ]
  .iter()
  .any(|h| host == *h || host.strip_suffix(h).is_some_and(|s| s.ends_with('.')))
  {
    return false;
  }
  !matches!(
    url
      .path()
      .rsplit('.')
      .next()
      .unwrap_or_default()
      .to_ascii_lowercase()
      .as_str(),
    "m3u8"
      | "m3u"
      | "mpd"
      | "mp4"
      | "mp3"
      | "webm"
      | "m4a"
      | "ogg"
      | "flac"
      | "wav"
      | "mov"
      | "mkv"
  )
}
pub struct Probe {
  pub(crate) source: String,
  pub name: String,
  pub size: Option<u64>,
  pub kind: Kind,
  pub(crate) mime: String,
  pub(crate) ranges: bool,
  pub(crate) validator: Validator,
  pub(crate) body: Response,
}
impl Probe {
  pub(crate) fn confirm_unknown(&mut self) -> Result<()> {
    if self
      .body
      .response
      .headers()
      .get_all("content-encoding")
      .iter()
      .any(|v| {
        v.to_str()
          .map_or(true, |v| !v.trim().eq_ignore_ascii_case("identity"))
      })
    {
      return Err(Error::Encoding);
    }
    self.kind = Kind::File;
    Ok(())
  }
  pub(crate) fn from_response(mut body: Response) -> Result<Self> {
    body.ranged = false;
    let response = &body.response;
    if response.status() != reqwest::StatusCode::OK {
      tracing::warn!(
        error_kind = "http_status",
        status = response.status().as_u16(),
        "File probe did not return a complete response"
      );
      return Err(Error::Network);
    }
    let h = response.headers();
    let mime = h
      .get("content-type")
      .and_then(|h| h.to_str().ok())
      .unwrap_or_default();
    let mime = mime
      .split(';')
      .next()
      .unwrap_or_default()
      .trim()
      .to_ascii_lowercase();
    let kind = classify(
      response.url(),
      &mime,
      h.get("content-disposition").map(|h| h.as_bytes()),
    );
    if kind == Kind::File
      && response
        .headers()
        .get_all("content-encoding")
        .iter()
        .any(|v| {
          v.to_str()
            .map_or(true, |v| !v.trim().eq_ignore_ascii_case("identity"))
        })
    {
      return Err(Error::Encoding);
    }
    Ok(Self {
      source: response.url().to_string(),
      name: suggested_name(
        h.get("content-disposition").map(|h| h.as_bytes()),
        response.url(),
        &mime,
      ),
      size: h
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|n| n.parse().ok())
        .or_else(|| response.content_length()),
      kind,
      mime,
      ranges: h.get("accept-ranges").is_some_and(|h| h == "bytes"),
      validator: Validator::from_headers(h),
      body,
    })
  }
}
