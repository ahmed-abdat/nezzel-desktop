//! The first GET is a probe and remains open as segment zero.
use super::{client::Response, names::suggested_name, plan::Validator, Error, Result};
use crate::media_types::{
  base_mime, ext_in, is_manifest_mime, FILE_EXTS, FILE_IMAGE_MIMES, MANIFEST_EXTS, MEDIA_EXTS,
};
use crate::sites::SiteId;
use reqwest::Url;
#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
  File,
  Page,
  Media,
  Unknown,
}
pub fn classify(url: &Url, mime: &str, disposition: Option<&[u8]>) -> Kind {
  let mime = base_mime(mime);
  let ext = url.path().rsplit('.').next().unwrap_or_default();
  if mime.starts_with("video/")
    || mime.starts_with("audio/")
    || is_manifest_mime(&mime)
    || matches!(
      mime.as_str(),
      "application/rss+xml" | "application/atom+xml"
    )
    || ext_in(MANIFEST_EXTS, ext)
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
    )
    || FILE_IMAGE_MIMES.contains(&mime.as_str())
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
  ext_in(FILE_EXTS, extension)
}
/// Known extractor sites and host-share pages retain their current routing.
pub(crate) fn should_probe_metadata(input: &str, total: usize, has_format: bool) -> bool {
  total <= 1
    && !has_format
    && should_probe(input)
    && Url::parse(input).is_ok_and(|u| file_extension(&u))
}
/// Sites whose links are pages for an extractor, never a direct file to probe.
const EXTRACTOR_SITES: &[SiteId] = {
  use SiteId::*;
  &[
    YouTube,
    Vimeo,
    SoundCloud,
    TikTok,
    X,
    Instagram,
    Reddit,
    Bluesky,
    Pinterest,
    Facebook,
    Threads,
    LinkedIn,
    Telegram,
    Snapchat,
    Twitch,
    Dailymotion,
    GoogleDrive,
    GoogleDocs,
    Dropbox,
  ]
};

pub fn should_probe(input: &str) -> bool {
  let Ok(url) = Url::parse(input) else {
    return false;
  };
  if !matches!(url.scheme(), "http" | "https") {
    return false;
  }
  if crate::sites::site_of(url.host_str().unwrap_or_default())
    .is_some_and(|site| EXTRACTOR_SITES.contains(&site))
  {
    return false;
  }
  let ext = url.path().rsplit('.').next().unwrap_or_default();
  !(ext_in(MANIFEST_EXTS, ext) || ext_in(MEDIA_EXTS, ext))
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
