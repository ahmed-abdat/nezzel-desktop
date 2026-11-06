//! Shared, row-scoped page evidence. Raw HTML never leaves the probe.
use crate::resolve::{html, http, Resolution};
use regex::Regex;
use reqwest::Url;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::Mutex;

const CANDIDATE_LIMIT: usize = 24;
static STREAM: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"(?i)(?:https://|//|/)[^\s"'<>`\\]*?\.(?:m3u8|mpd|mp4|webm)(?:\?[^\s"'<>`\\]*)?"#)
    .expect("static stream URL grammar")
});

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectKind {
  Video,
  Audio,
  Image,
  File,
}
#[derive(Default)]
pub(crate) struct Og {
  pub kind: Option<String>,
  pub video: Vec<Url>,
  pub image: Vec<Url>,
  pub audio: Vec<Url>,
}
pub(crate) struct LdMedia {
  pub kind: DirectKind,
  pub content_url: Option<Url>,
  pub embed_url: Option<Url>,
}
pub(crate) struct TagMedia {
  pub kind: DirectKind,
  pub url: Url,
}
struct Candidate {
  url: Url,
  kind: DirectKind,
  embed: bool,
}
pub(crate) struct PageSignals {
  pub final_url: Url,
  pub content_type: Option<String>,
  pub direct: Option<DirectKind>,
  pub og: Og,
  pub ld: Vec<LdMedia>,
  pub tags: Vec<TagMedia>,
  pub embeds: Vec<Url>,
  pub streams: Vec<Url>,
  pub login_wall: bool,
  pub drm_protected: bool,
  pub image_count: usize,
  verified: Vec<Candidate>,
}
impl PageSignals {
  fn empty(final_url: Url) -> Self {
    Self {
      final_url,
      content_type: None,
      direct: None,
      og: Og::default(),
      ld: Vec::new(),
      tags: Vec::new(),
      embeds: Vec::new(),
      streams: Vec::new(),
      login_wall: false,
      drm_protected: false,
      image_count: 0,
      verified: Vec::new(),
    }
  }

  /// Only verified media or one allowlisted player hop is handed to an engine.
  pub(crate) fn resolution(&self) -> Option<Resolution> {
    if self.drm_protected {
      return None;
    }
    if let Some(candidate) = self.verified.iter().find(|c| c.kind != DirectKind::Image) {
      return Some(Resolution::Passthrough(candidate.url.to_string()));
    }
    let images: Vec<_> = self
      .verified
      .iter()
      .filter(|c| !c.embed && c.kind == DirectKind::Image)
      .map(|c| serde_json::json!({"id": c.url.as_str(), "kind":"image", "url":c.url.as_str()}))
      .collect();
    if images.is_empty() {
      return None;
    }
    // Existing attachments flow opens the source-ordered picker for a gallery.
    let metadata = serde_json::json!({
      "id":"page", "url":self.final_url.as_str(), "extractor":"generic",
      "mediaKind":if images.len() > 1 { "carousel" } else { "image" },
      "attachments":images, "videoCodecs":[], "audioCodecs":[], "videoTracks":[],
      "audioTracks":[], "formats":[], "chapters":[],
      "subtitleInventory":{"manualLanguages":[],"autoLanguages":[]},
    })
    .to_string();
    Some(Resolution::Attachments { metadata })
  }

  fn candidates(&self) -> Vec<Candidate> {
    let mut result = Vec::new();
    {
      let mut add = |url: &Url, kind, embed| {
        result.push(Candidate {
          url: url.clone(),
          kind,
          embed,
        })
      };
      if let Some(kind) = self.direct {
        add(&self.final_url, kind, false);
      }
      for url in &self.streams {
        add(url, DirectKind::Video, false);
      }
      for media in &self.ld {
        if media.kind != DirectKind::Image {
          if let Some(url) = &media.content_url {
            add(url, media.kind, false);
          }
        }
      }
      for url in &self.og.video {
        add(url, DirectKind::Video, known_player(url));
      }
      for url in &self.og.audio {
        add(url, DirectKind::Audio, false);
      }
      for media in &self.tags {
        if media.kind != DirectKind::Image {
          add(&media.url, media.kind, false);
        }
      }
      for url in &self.embeds {
        add(url, DirectKind::Video, true);
      }
      for media in &self.ld {
        if let Some(url) = &media.embed_url {
          add(url, media.kind, true);
        }
      }
      // An article's cover never rescues failed video evidence. Photo evidence must agree
      // with og:image, and the parser has already excluded small images and avatars.
    }
    let photo_page = self.tags.iter().any(|m| m.kind == DirectKind::Image)
      || self.ld.iter().any(|m| m.kind == DirectKind::Image)
      || self
        .og
        .kind
        .as_deref()
        .is_some_and(|s| matches!(s, "photo" | "image"));
    if result.is_empty()
      && photo_page
      && self.og.kind.as_deref() != Some("article")
      && !self.og.image.is_empty()
    {
      let mut add = |url: &Url, kind, embed| {
        result.push(Candidate {
          url: url.clone(),
          kind,
          embed,
        })
      };
      for media in &self.tags {
        if media.kind == DirectKind::Image {
          add(&media.url, media.kind, false);
        }
      }
      for media in &self.ld {
        if media.kind == DirectKind::Image {
          if let Some(url) = &media.content_url {
            add(url, DirectKind::Image, false);
          }
        }
      }
      for url in &self.og.image {
        add(url, DirectKind::Image, false);
      }
    }
    let mut seen = HashSet::new();
    result.retain(|c| {
      (c.kind != DirectKind::Image
        || self.direct == Some(DirectKind::Image)
        || !decorative_image(&c.url))
        && seen.insert(c.url.clone())
    });
    result.truncate(CANDIDATE_LIMIT);
    result
  }
}

/// Identity only; credentials belong to the supplied scoped transport, never to the cache.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProbeAuth {
  Anonymous,
  #[cfg(test)]
  Context(u64),
}
#[derive(PartialEq, Eq, Hash)]
struct ProbeKey {
  url: String,
  auth: ProbeAuth,
  transport: String,
}
/// Owned by one row. Holding the lock across the bounded fetch coalesces concurrent readers.
#[derive(Default)]
pub(crate) struct PageProbe {
  cache: Mutex<HashMap<ProbeKey, Result<Arc<PageSignals>, http::FetchError>>>,
}
impl PageProbe {
  pub(crate) async fn page_signals<T: http::Transport>(
    &self,
    url: &str,
    auth: ProbeAuth,
    transport: &T,
  ) -> Result<Arc<PageSignals>, http::FetchError> {
    let key = ProbeKey {
      url: url.into(),
      auth,
      transport: transport.cache_key().into(),
    };
    let mut cache = self.cache.lock().await;
    if let Some(signals) = cache.get(&key) {
      return signals.clone();
    }
    let result = tokio::time::timeout(Duration::from_secs(10), load(url, transport))
      .await
      .map_err(|_| http::FetchError::Network)
      .and_then(|r| r)
      .map(Arc::new);
    // A later consumer may recover from DNS, connection or deadline failures.
    if !matches!(result, Err(http::FetchError::Network)) {
      cache.insert(key, result.clone());
    }
    result
  }
}

async fn load<T: http::Transport>(
  url: &str,
  transport: &T,
) -> Result<PageSignals, http::FetchError> {
  let page = http::fetch(transport, http::https_url(url)?, false).await?;
  if crate::resolve::reader_bound(page.url.as_str()) {
    return Err(http::FetchError::InvalidUrl);
  }
  let mut signals = if page.media {
    PageSignals::empty(page.url.clone())
  } else {
    parse(
      std::str::from_utf8(&page.body).map_err(|_| http::FetchError::Encoding)?,
      &page.url,
    )
  };
  signals.content_type = page.content_type;
  if page.media {
    signals.direct = direct_kind(signals.content_type.as_deref(), &page.url);
  }
  signals.drm_protected |=
    http::manifest_response(&page.url, signals.content_type.as_deref()) && drm(&page.body);
  if signals.drm_protected {
    return Ok(signals);
  }
  for mut candidate in signals.candidates() {
    if let Ok(inspected) = http::inspect(transport, candidate.url.clone()).await {
      let response = inspected.response;
      if !(200..300).contains(&response.status)
        || response.content_length == Some(0)
        || (response.content_length.is_none() && response.body.is_empty())
      {
        continue;
      }
      let mime = response.content_type.as_deref();
      if http::manifest_response(&inspected.url, mime) && drm(&response.body) {
        signals.drm_protected = true;
        signals.verified.clear();
        break;
      }
      if candidate.embed {
        if !known_player(&inspected.url)
          || !mime.is_some_and(|m| {
            m.split(';')
              .next()
              .is_some_and(|m| matches!(m.trim(), "text/html" | "application/xhtml+xml"))
          })
        {
          continue;
        }
      } else {
        let Some(kind) = direct_kind(mime, &inspected.url) else {
          continue;
        };
        // An HTML error or a JPEG poster cannot masquerade as a video.
        if candidate.kind != kind
          && !(candidate.kind == DirectKind::Video && http::manifest_response(&inspected.url, mime))
        {
          continue;
        }
        if http::manifest_response(&inspected.url, mime) && response.body.is_empty() {
          continue;
        }
        candidate.kind = kind;
      }
      candidate.url = inspected.url;
      let complete = candidate.kind != DirectKind::Image;
      signals.verified.push(candidate);
      if complete {
        break;
      }
    }
  }
  Ok(signals)
}

fn direct_kind(mime: Option<&str>, url: &Url) -> Option<DirectKind> {
  let mime = mime?.split(';').next()?.trim().to_ascii_lowercase();
  if http::manifest_response(url, Some(&mime))
    || mime.starts_with("video/")
    || (http::is_octet_stream(Some(&mime))
      && [".mp4", ".webm"]
        .iter()
        .any(|extension| url.path().to_ascii_lowercase().ends_with(extension)))
  {
    Some(DirectKind::Video)
  } else if mime.starts_with("audio/") {
    Some(DirectKind::Audio)
  } else if mime.starts_with("image/") {
    Some(DirectKind::Image)
  } else if matches!(
    mime.as_str(),
    "application/pdf" | "application/octet-stream"
  ) {
    Some(DirectKind::File)
  } else {
    None
  }
}
fn drm(body: &[u8]) -> bool {
  let text = String::from_utf8_lossy(body).to_ascii_lowercase();
  // AES-128 is ordinary HLS encryption, not a DRM system.
  text.contains("method=sample-aes")
    || text.contains("<contentprotection")
    || text.contains(":contentprotection")
}
fn media_url(page: &Url, value: &str) -> Option<Url> {
  Url::parse(&html::media_url(page, value)?).ok()
}
fn decorative_image(url: &Url) -> bool {
  let path = url.path().to_ascii_lowercase();
  ["logo", "avatar", "icon", "sprite"]
    .iter()
    .any(|marker| path.contains(marker))
}
fn known_player(url: &Url) -> bool {
  let host = url.host_str().unwrap_or("");
  let path = url.path();
  let domain =
    |root: &str| host == root || host.strip_suffix(root).is_some_and(|s| s.ends_with('.'));
  ((domain("youtube.com") || domain("youtube-nocookie.com")) && path.starts_with("/embed/"))
    || (host == "player.vimeo.com" && path.starts_with("/video/"))
    || (domain("dailymotion.com") && path.starts_with("/embed/video/"))
    || (host == "geo.dailymotion.com" && path.starts_with("/player/"))
    || (host == "cdn.jwplayer.com" && path.starts_with("/players/"))
    || host == "players.brightcove.net"
    || (domain("wistia.net") && path.starts_with("/embed/"))
    || (domain("streamable.com") && path.starts_with("/e/"))
    || (domain("facebook.com") && path.starts_with("/plugins/video"))
    || host == "player.twitch.tv"
    || (host == "w.soundcloud.com" && path.starts_with("/player/"))
}

fn parse(body: &str, page: &Url) -> PageSignals {
  let mut signals = PageSignals::empty(page.clone());
  let Some(tags) = html::all_tags(body) else {
    return signals;
  };
  signals.og.kind = html::metadata(&tags, "og:type")
    .first()
    .map(|s| (*s).into());
  for (properties, output) in [
    (
      &[
        "og:video:secure_url",
        "og:video:url",
        "og:video",
        "twitter:player:stream",
      ][..],
      &mut signals.og.video,
    ),
    (
      &["og:audio:secure_url", "og:audio:url", "og:audio"][..],
      &mut signals.og.audio,
    ),
    (
      &["og:image:secure_url", "og:image:url", "og:image"][..],
      &mut signals.og.image,
    ),
  ] {
    for property in properties {
      for raw in html::metadata(&tags, property) {
        if let Some(url) = media_url(page, raw) {
          push_url(output, url);
        }
      }
    }
  }
  if ["og:image:width", "og:image:height"]
    .iter()
    .any(|property| {
      html::metadata(&tags, property)
        .iter()
        .any(|value| value.parse::<u64>().is_ok_and(|n| n < 200))
    })
  {
    signals.og.image.clear();
  }
  let wall_title = |title: &str| {
    matches!(
      title.trim().to_ascii_lowercase().as_str(),
      "log in" | "login" | "sign in" | "log in to continue" | "sign in to continue"
    )
  };
  signals.login_wall = html::metadata(&tags, "og:title")
    .iter()
    .any(|s| wall_title(s))
    || tags.iter().any(|t| t.name == "title" && wall_title(t.text));
  let mut in_media = None;
  let mut images = Vec::new();
  for tag in &tags {
    if tag.closing {
      if matches!(tag.name.as_str(), "video" | "audio") {
        in_media = None;
      }
      continue;
    }
    match tag.name.as_str() {
      "video" | "audio" => {
        let kind = if tag.name == "video" {
          DirectKind::Video
        } else {
          DirectKind::Audio
        };
        in_media = Some(kind);
        if let Some(url) = tag.attrs.get("src").and_then(|s| media_url(page, s)) {
          signals.tags.push(TagMedia { kind, url });
        }
      }
      "source" if in_media.is_some() => {
        if let (Some(kind), Some(url)) = (
          in_media,
          tag.attrs.get("src").and_then(|s| media_url(page, s)),
        ) {
          signals.tags.push(TagMedia { kind, url });
        }
      }
      "img" | "source" => {
        let small = ["width", "height"].iter().any(|key| {
          tag
            .attrs
            .get(*key)
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|n| n < 200)
        });
        let decoration = ["src", "alt", "class", "id"].iter().any(|key| {
          tag.attrs.get(*key).is_some_and(|s| {
            ["logo", "avatar", "icon", "sprite"]
              .iter()
              .any(|m| s.to_ascii_lowercase().contains(m))
          })
        });
        if small || decoration {
          continue;
        }
        let largest = tag.attrs.get("srcset").and_then(|s| srcset(s, page));
        let sized = ["width", "height"].iter().all(|key| {
          tag
            .attrs
            .get(*key)
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|n| n >= 200)
        });
        if let Some(url) = largest.or_else(|| {
          sized
            .then(|| tag.attrs.get("src").and_then(|s| media_url(page, s)))
            .flatten()
        }) {
          push_url(&mut images, url);
        }
      }
      "iframe" => {
        if let Some(url) = tag
          .attrs
          .get("src")
          .and_then(|s| media_url(page, s))
          .filter(known_player)
        {
          push_url(&mut signals.embeds, url);
        }
      }
      "script" => {
        if tag
          .attrs
          .get("type")
          .is_some_and(|s| s.eq_ignore_ascii_case("application/ld+json"))
        {
          if let Ok(value) = serde_json::from_str::<Value>(tag.text) {
            ld_media(&value, page, &mut signals.ld, 0);
          }
        } else {
          let text = html::entities(tag.text)
            .replace("\\/", "/")
            .replace("\\u002F", "/")
            .replace("\\u002f", "/")
            .replace("\\u0026", "&");
          for found in STREAM.find_iter(&text).take(CANDIDATE_LIMIT) {
            if let Some(url) = media_url(page, found.as_str()) {
              push_url(&mut signals.streams, url);
            }
          }
        }
      }
      _ => {}
    }
  }
  // Keep the legacy generic parser reachable on the already-fetched body. This
  // retains its relative media URLs and unique YouTube frontend-ID selection.
  if let Some(Resolution::Passthrough(target)) = html::generic(body, page.as_str()) {
    if let Ok(target) = Url::parse(&target) {
      if target.host_str() == Some("www.youtube.com") && target.path() == "/watch" {
        if let Some((_, id)) = target.query_pairs().find(|(key, _)| key == "v") {
          if let Ok(embed) = Url::parse(&format!("https://www.youtube.com/embed/{id}")) {
            signals.embeds.retain(|url| url != &embed);
            signals.embeds.insert(0, embed);
          }
        }
      } else if !signals.tags.iter().any(|media| media.url == target) {
        signals.tags.push(TagMedia {
          kind: DirectKind::Video,
          url: target,
        });
      }
    }
  }
  // Arbitrary page images are not a photo gallery without corroborating social metadata.
  if images.iter().any(|url| signals.og.image.contains(url)) {
    signals.tags.extend(images.into_iter().map(|url| TagMedia {
      kind: DirectKind::Image,
      url,
    }));
  }
  let mut images: HashSet<_> = signals.og.image.iter().collect();
  images.extend(
    signals
      .tags
      .iter()
      .filter(|m| m.kind == DirectKind::Image)
      .map(|m| &m.url),
  );
  images.extend(
    signals
      .ld
      .iter()
      .filter(|m| m.kind == DirectKind::Image)
      .filter_map(|m| m.content_url.as_ref()),
  );
  signals.image_count = images.len();
  signals
}
fn push_url(urls: &mut Vec<Url>, url: Url) {
  if urls.len() < CANDIDATE_LIMIT && !urls.contains(&url) {
    urls.push(url);
  }
}
fn srcset(value: &str, page: &Url) -> Option<Url> {
  value
    .split(',')
    .filter_map(|item| {
      let mut parts = item.split_ascii_whitespace();
      let url = media_url(page, parts.next()?)?;
      let descriptor = parts.next().unwrap_or("1x");
      let weight = descriptor
        .strip_suffix('w')
        .or_else(|| descriptor.strip_suffix('x'))?
        .parse::<f64>()
        .ok()?;
      (weight.is_finite() && weight > 0.0).then_some((weight, url))
    })
    .max_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, url)| url)
}
fn ld_media(value: &Value, page: &Url, result: &mut Vec<LdMedia>, depth: usize) {
  if depth > 32 || result.len() >= CANDIDATE_LIMIT {
    return;
  }
  match value {
    Value::Array(values) => {
      for value in values {
        ld_media(value, page, result, depth + 1);
      }
    }
    Value::Object(object) => {
      let types: Vec<_> = match object.get("@type") {
        Some(Value::String(kind)) => vec![kind.as_str()],
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
      };
      let kind = if types.contains(&"VideoObject") {
        Some(DirectKind::Video)
      } else if types.contains(&"AudioObject") {
        Some(DirectKind::Audio)
      } else if types.contains(&"ImageObject") {
        Some(DirectKind::Image)
      } else if types.contains(&"MediaObject") {
        Some(DirectKind::Video)
      } else {
        None
      };
      if let Some(kind) = kind {
        result.push(LdMedia {
          kind,
          content_url: object
            .get("contentUrl")
            .and_then(Value::as_str)
            .and_then(|s| media_url(page, s)),
          embed_url: object
            .get("embedUrl")
            .and_then(Value::as_str)
            .and_then(|s| media_url(page, s))
            .filter(known_player),
        });
      }
      for value in object.values() {
        ld_media(value, page, result, depth + 1);
      }
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests;
