//! A bounded list on a single-post URL is one selectable post, not a channel/playlist range.
use crate::models::parsed::{AttachmentKind, MediaAttachment, MediaKind};
use crate::models::{ParsedMedia, ParsedSingleVideo};
use crate::sites::{site_by_domain, SiteId};
use reqwest::Url;
use serde_json::Value;
use std::collections::HashSet;

pub(crate) fn single_post_url(raw: &str) -> bool {
  if crate::resolve::reddit::is_post_url(raw) {
    return true;
  }
  let Ok(url) = Url::parse(raw) else {
    return false;
  };
  if !matches!(url.scheme(), "https" | "http")
    || !url.username().is_empty()
    || url.password().is_some()
  {
    return false;
  }
  let host = url
    .host_str()
    .unwrap_or_default()
    .trim_start_matches("www.");
  let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
  match host {
    "youtube.com" | "youtu.be" | "soundcloud.com" => false,
    host if site_by_domain(host) == Some(SiteId::X) => {
      matches!(parts.as_slice(), [_, "status", id, ..] if !id.is_empty())
    }
    host if site_by_domain(host) == Some(SiteId::Threads) => {
      matches!(parts.as_slice(), [user, "post", id, ..] if user.starts_with('@') && !id.is_empty())
    }
    "bsky.app" => matches!(parts.as_slice(), ["profile", _, "post", id] if !id.is_empty()),
    "facebook.com" | "m.facebook.com" | "web.facebook.com" => {
      matches!(
        parts.as_slice(),
        ["reel", _]
          | [_, "posts" | "videos", _, ..]
          | ["groups", _, "posts" | "permalink", _, ..]
          | ["share", "p" | "v" | "r", _]
      ) || matches!(
        url.path(),
        "/story.php" | "/permalink.php" | "/watch/" | "/watch"
      ) && url
        .query_pairs()
        .any(|(k, _)| matches!(k.as_ref(), "story_fbid" | "v"))
    }
    "tumblr.com" => {
      matches!(parts.as_slice(), [_, id, ..] if id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty())
    }
    host if host.ends_with(".tumblr.com") => matches!(parts.as_slice(), ["post", _, ..]),
    "instagram.com" => matches!(parts.as_slice(), ["p" | "reel", _, ..]),
    _ => parts
      .windows(2)
      .any(|p| matches!(p[0], "post" | "posts" | "status" | "comments") && !p[1].is_empty()),
  }
}

fn extension(value: &Value) -> Option<String> {
  value["ext"]
    .as_str()
    .map(str::to_ascii_lowercase)
    .or_else(|| {
      let url = Url::parse(value["url"].as_str()?).ok()?;
      Some(url.path().rsplit('.').next()?.to_ascii_lowercase())
    })
}
fn video_format(value: &Value) -> bool {
  let codec = value["vcodec"].as_str();
  if codec == Some("none") {
    return false;
  }
  if codec.is_some_and(|c| !matches!(c, "unknown" | "" | "images"))
    || value["height"].as_u64().is_some_and(|h| h > 0)
  {
    return true;
  }
  // Only direct HTTP media may infer a picture from its container; explicit audio always wins.
  let direct = value["url"]
    .as_str()
    .and_then(|s| Url::parse(s).ok())
    .is_some_and(|u| matches!(u.scheme(), "http" | "https"));
  direct
    && matches!(
      extension(value).as_deref(),
      Some("mp4" | "webm" | "mov" | "mkv" | "m4v")
    )
}
pub(super) fn media_kind(value: &Value) -> Option<MediaKind> {
  let formats = value["formats"].as_array();
  if video_format(value) || formats.is_some_and(|f| f.iter().any(video_format)) {
    return Some(MediaKind::Video);
  }
  if value["vcodec"] == "none"
    || matches!(
      extension(value).as_deref(),
      Some("m4a" | "mp3" | "opus" | "ogg" | "aac" | "flac" | "wav")
    )
  {
    return Some(MediaKind::Audio);
  }
  None
}

pub(super) fn from_entries(
  value: &Value,
  source: &str,
  mut parent: ParsedSingleVideo,
) -> Option<ParsedMedia> {
  if !single_post_url(source) {
    return None;
  }
  let entries = value["entries"].as_array()?;
  if entries.is_empty() || entries.len() > 256 {
    return None;
  }
  let mut ids = HashSet::new();
  let mut attachments = Vec::new();
  for (index, entry) in entries.iter().enumerate() {
    let raw = entry["url"]
      .as_str()
      .filter(|s| !s.is_empty())
      .or_else(|| entry["webpage_url"].as_str())?;
    let mut stable = crate::resolve::http::https_url(raw).ok()?;
    stable.set_query(None);
    let id = entry["id"]
      .as_str()
      .filter(|s| !s.is_empty())
      .unwrap_or(stable.as_str())
      .to_owned();
    if !ids.insert(id.clone()) {
      return None;
    }
    let kind = if extension(entry).is_some_and(|s| crate::parsers::gallery::image_extension(&s)) {
      AttachmentKind::Image
    } else {
      AttachmentKind::Video
    };
    let thumbnail = entry["thumbnail"]
      .as_str()
      .or_else(|| entry["thumbnails"].as_array()?.last()?["url"].as_str())
      .filter(|raw| crate::resolve::http::https_url(raw).is_ok())
      .map(str::to_owned)
      .or_else(|| matches!(kind, AttachmentKind::Image).then(|| raw.to_owned()));
    let alt = parent
      .title
      .as_ref()
      .map(|title| format!("{title} ({}/{})", index + 1, entries.len()));
    attachments.push(MediaAttachment {
      id,
      file_index: None,
      kind,
      url: raw.into(),
      thumbnail,
      alt,
      width: entry["width"].as_u64(),
      height: entry["height"].as_u64(),
    });
  }
  let images = attachments
    .iter()
    .any(|a| matches!(a.kind, AttachmentKind::Image));
  let videos = attachments
    .iter()
    .any(|a| matches!(a.kind, AttachmentKind::Video));
  parent.media_kind = Some(if images && videos {
    MediaKind::Mixed
  } else if attachments.len() > 1 {
    MediaKind::Carousel
  } else if images {
    MediaKind::Image
  } else {
    MediaKind::Video
  });
  parent.url = Some(source.into());
  parent.thumbnail = parent
    .thumbnail
    .filter(|raw| crate::resolve::http::https_url(raw).is_ok())
    .or_else(|| attachments.first()?.thumbnail.clone());
  parent.attachments = attachments;
  Some(ParsedMedia::Single(parent))
}
