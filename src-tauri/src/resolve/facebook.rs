//! Ported from our Android FacebookPostIdentity / FacebookPageParser.
//! Only root Story attachments and an explicitly requested currMedia photo can verify media.
use super::{html, http, Reason, Resolution, Verdict};
use regex::Regex;
use reqwest::Url;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

pub(super) const MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Facebook,
  "facebook-post",
  20,
  |raw| Identity::parse(raw).is_some(),
);
pub(super) const SHARE_MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Facebook,
  "facebook-share",
  30,
  |raw| Identity::parse(raw).is_some_and(|identity| identity.short),
);
pub(super) const PHOTO_MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Facebook,
  "facebook-photo",
  30,
  |raw| Identity::parse(raw).is_some_and(|identity| identity.photo_id.is_some()),
);

const WWW: &str = "https://www.facebook.com";
static SHAPES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
  [
  r"^/share/(?:([pvr])/)?([A-Za-z0-9_-]{1,64})/?$",
  r"^/([A-Za-z0-9_-]{1,64})/?$",
  r"^/(?:groups/)?([A-Za-z0-9._-]{1,100})/(?:posts|permalink)/(?:[^/]{1,512}/)?([0-9]{1,25}|pfbid[A-Za-z0-9]{1,80})/?$",
  r"^/([A-Za-z0-9._-]{1,100})/photos/(?:[a-z]{1,8}\.[0-9]{1,25}/)?([0-9]{1,25})/?$",
  r"^/reel/([0-9]{1,25})/?$",
  r"^/groups/([A-Za-z0-9._-]{1,100})/?$",
  r"^/([A-Za-z0-9._-]{1,100})/videos/(?:[^/]{1,512}/)?([0-9]{1,25})/?$",
  r"^[a-z]{2}-[a-z]{2}\.facebook\.com$",
].iter().map(|p| Regex::new(p).expect("static Facebook grammar")).collect()
});

pub(super) struct Identity {
  pub url: Url,
  pub post_id: Option<String>,
  pub photo_id: Option<String>,
  pub video_id: Option<String>,
  pub short: bool,
  video_share: bool,
  post_surface: bool,
}

impl Identity {
  pub fn parse(input: &str) -> Option<Self> {
    let parsed = http::https_url(input).ok()?;
    let host = parsed.host_str()?;
    let path = parsed.path();
    let mut identity = Self {
      url: parsed.clone(),
      post_id: None,
      photo_id: None,
      video_id: None,
      short: false,
      video_share: false,
      post_surface: false,
    };
    if host == "fb.watch" {
      SHAPES[1].captures(path)?;
      identity.short = true;
      identity.url.set_query(None);
      return Some(identity);
    }
    if !is_host(host) || path.starts_with("/events/") || path.starts_with("/stories/") {
      return None;
    }
    identity.url = Url::parse(&format!("{WWW}{path}")).ok()?;
    if let Some(c) = SHAPES[0].captures(path) {
      identity.short = true;
      identity.video_share = c.get(1).is_some_and(|m| matches!(m.as_str(), "v" | "r"));
      identity.post_surface = true;
    } else if matches!(path, "/story.php" | "/permalink.php") {
      let story = single(&parsed, "story_fbid")?;
      let owner = single(&parsed, "id")?;
      if !numeric(&owner)
        || !(numeric(&story)
          || story.strip_prefix("pfbid").is_some_and(|s| {
            !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric())
          }))
      {
        return None;
      }
      identity
        .url
        .set_query(Some(&format!("story_fbid={story}&id={owner}")));
      identity.post_id = numeric(&story).then_some(story);
      identity.post_surface = true;
    } else if matches!(path, "/photo" | "/photo/" | "/photo.php") {
      let photo = single(&parsed, "fbid").filter(|s| numeric(s))?;
      identity.url = Url::parse(&format!("{WWW}/photo/?fbid={photo}")).ok()?;
      identity.photo_id = Some(photo);
      identity.post_surface = true;
    } else if matches!(path, "/watch" | "/watch/") {
      let video = single(&parsed, "v").filter(|s| numeric(s))?;
      identity.url = Url::parse(&watch(&video)).ok()?;
      identity.video_id = Some(video);
    } else if let Some(c) = SHAPES[4].captures(path) {
      identity.video_id = Some(c[1].into());
    } else if let Some(c) = SHAPES[3].captures(path) {
      identity.photo_id = Some(c[2].into());
      identity.url = Url::parse(&photo_url(&c[2])).ok()?;
      identity.post_surface = true;
    } else if let Some(c) = SHAPES[6].captures(path) {
      identity.video_id = Some(c[2].into());
    } else if let Some(c) = SHAPES[5].captures(path) {
      let post = single(&parsed, "multi_permalinks").filter(|s| numeric(s))?;
      identity.url = Url::parse(&format!("{WWW}/groups/{}/posts/{post}/", &c[1])).ok()?;
      identity.post_id = Some(post);
      identity.post_surface = true;
    } else {
      let c = SHAPES[2].captures(path)?;
      identity.post_id = numeric(&c[2]).then(|| c[2].into());
      identity.post_surface = true;
    }
    Some(identity)
  }
}

pub(super) fn is_host(host: &str) -> bool {
  matches!(
    host,
    "facebook.com"
      | "www.facebook.com"
      | "m.facebook.com"
      | "web.facebook.com"
      | "mbasic.facebook.com"
      | "touch.facebook.com"
      | "mobile.facebook.com"
  ) || SHAPES[7].is_match(host)
}

pub(super) fn is_login_wall(url: &Url) -> bool {
  url.host_str().is_some_and(is_host)
    && (matches!(
      url.path().trim_end_matches('/'),
      "/login" | "/login.php" | "/checkpoint" | "/stories"
    ) || url.path().starts_with("/stories/"))
}

pub(super) fn is_wall(url: &Url) -> bool {
  is_login_wall(url)
    || (url.host_str().is_some_and(is_host)
      && matches!(
        url.path().trim_end_matches('/'),
        "/consent" | "/consent.php" | "/privacy/consent"
      ))
}

fn single(url: &Url, name: &str) -> Option<String> {
  let mut values = url.query_pairs().filter(|(key, _)| key == name);
  let value = values.next()?.1.into_owned();
  values.next().is_none().then_some(value)
}
fn numeric(value: &str) -> bool {
  !value.is_empty() && value.len() <= 25 && value.bytes().all(|b| b.is_ascii_digit())
}
pub(super) fn watch(id: &str) -> String {
  format!("{WWW}/watch/?v={id}")
}

#[derive(Default, PartialEq, Eq)]
struct Media {
  photos: BTreeSet<String>,
  photo_urls: Vec<(String, String)>,
  videos: BTreeSet<String>,
  has_video: bool,
  others: usize,
}
impl Media {
  fn empty(&self) -> bool {
    self.photos.is_empty() && self.videos.is_empty() && self.others == 0
  }
}
struct Story<'a> {
  post: Option<&'a str>,
  media: Media,
}

pub(super) fn read(html: &str, page: &Identity, requested: &Identity) -> Verdict {
  let Some(tags) = html::tags(html) else {
    return Verdict::Unknown;
  };
  let scripts: Vec<_> = tags
    .iter()
    .filter(|t| t.name == "script" && t.attrs.contains_key("data-sjs"))
    .collect();
  let mut roots = Vec::new();
  let mut tokens = 0;
  for script in scripts {
    if !json_budget(script.text, &mut tokens) {
      return Verdict::Unknown;
    }
    if let Ok(root) = serde_json::from_str::<Value>(script.text) {
      roots.push((script.text.contains("RelayPrefetchedStreamCache"), root));
    }
  }
  // The page's route and logged-out walls take precedence over preview metadata.
  for (_, root) in &roots {
    if let Some(route) = route(root, 0) {
      if route == "CometErrorRoot.react" {
        return Verdict::Unknown;
      }
      if route.starts_with("CometLogin") || route.contains("Checkpoint") {
        return Verdict::Unknown;
      }
      if route.contains("Consent") {
        return Verdict::Unknown;
      }
    }
  }
  if tags.iter().any(|t| {
    t.name == "form"
      && (t.attrs.get("id").is_some_and(|id| id.contains("consent"))
        || t
          .attrs
          .get("action")
          .is_some_and(|action| action.contains("/consent")))
  }) {
    return Verdict::Unknown;
  }
  if tags.iter().any(|t| {
    t.name == "form"
      && (t.attrs.get("id").is_some_and(|id| id == "login_form")
        || t
          .attrs
          .get("action")
          .is_some_and(|v| v.contains("/login") || v.contains("/checkpoint")))
  }) {
    return Verdict::Unknown;
  }
  let mut stories: BTreeMap<&str, Story<'_>> = BTreeMap::new();
  let mut conflicts = BTreeSet::new();
  let mut currents = Vec::new();
  let mut any_media = false;
  let mut blocks = 0;
  for (relay, root) in &roots {
    if !relay {
      continue;
    }
    blocks += 1;
    if blocks > 128 {
      return Verdict::Unknown;
    }
    let mut data = Vec::new();
    results(root, 0, &mut data);
    for item in data {
      any_media |= has_media(item, 0);
      if let Some(story) = item
        .get("node_v2")
        .or_else(|| item.get("node"))
        .filter(|v| v["__typename"] == "Story")
      {
        let Some(key) = story["post_id"].as_str().or_else(|| story["id"].as_str()) else {
          continue;
        };
        let media = story_media(story);
        if let Some(existing) = stories.get(key) {
          if !existing.media.empty()
            && !media.empty()
            && (existing.media.photos != media.photos
              || existing.media.videos != media.videos
              || existing.media.has_video != media.has_video
              || existing.media.others != media.others)
          {
            conflicts.insert(key);
          }
          if !existing.media.empty() {
            continue;
          }
        }
        stories.insert(
          key,
          Story {
            post: story["post_id"].as_str(),
            media,
          },
        );
        if stories.len() > 64 {
          return Verdict::Unknown;
        }
      }
      if let Some(current) = item.get("currMedia") {
        currents.push(current);
      }
      if currents.len() > 64 {
        return Verdict::Unknown;
      }
    }
  }
  let post_id = requested.post_id.as_deref().or(page.post_id.as_deref());
  let photo_id = requested.photo_id.as_deref().or(page.photo_id.as_deref());
  let matched: Vec<_> = stories
    .iter()
    .filter(|(_, root)| post_id.is_none_or(|id| root.post == Some(id)))
    .collect();
  let bound = if matched.len() == 1 && !conflicts.contains(matched[0].0) {
    Some(&matched[0].1.media)
  } else {
    None
  };
  if let Some(id) = photo_id {
    let selected = currents
      .iter()
      .filter(|v| v["id"] == id)
      .find_map(|v| photo(v))
      .or_else(|| bound?.photo_urls.iter().find(|(key, _)| key == id).cloned());
    return selected.map_or(Verdict::Unknown, |photo| {
      photo_attachments(&[photo], requested)
    });
  }
  if let Some(media) = bound {
    if !media.photo_urls.is_empty() && !media.has_video {
      return photo_attachments(&media.photo_urls, requested);
    }
    // Mixed posts still need gallery to preserve their videos.
    if !media.photos.is_empty() {
      return Verdict::Verified(Resolution::Unsupported {
        reason: Reason::PhotosOnly,
      });
    }
    if (requested.post_surface || requested.video_share)
      && media.others == 0
      && media.videos.len() == 1
    {
      return Verdict::Verified(Resolution::Passthrough(watch(
        media.videos.first().unwrap(),
      )));
    }
  }
  if !any_media
    && page.post_surface
    && !html::metadata(&tags, "og:type")
      .iter()
      .any(|v| v.to_ascii_lowercase().starts_with("video"))
  {
    let images: BTreeSet<_> = html::metadata(&tags, "og:image")
      .into_iter()
      .filter(|url| photo_cdn(url))
      .collect();
    if images.len() == 1 {
      return Verdict::Verified(Resolution::Unsupported {
        reason: Reason::PhotosOnly,
      });
    }
  }
  Verdict::Unknown
}

// Conservative lexical budget before allocating serde_json's tree. Strings count once;
// punctuation/scalar bytes can only overcount JSON tokens, never undercount them.
fn json_budget(text: &str, tokens: &mut usize) -> bool {
  let mut quoted = false;
  let mut escaped = false;
  let mut string_bytes = 0;
  for byte in text.bytes() {
    if quoted {
      string_bytes += 1;
      if string_bytes > 512 * 1024 {
        return false;
      }
      if escaped {
        escaped = false;
      } else if byte == b'\\' {
        escaped = true;
      } else if byte == b'"' {
        quoted = false;
      }
    } else if !byte.is_ascii_whitespace() {
      *tokens += 1;
      if *tokens > 1_000_000 {
        return false;
      }
      if byte == b'"' {
        quoted = true;
        string_bytes = 0;
      }
    }
  }
  true
}

fn story_media(story: &Value) -> Media {
  let attachments = story
    .get("attachments")
    .and_then(Value::as_array)
    .filter(|a| !a.is_empty())
    .or_else(|| {
      story
        .pointer("/comet_sections/content/story/attachments")
        .and_then(Value::as_array)
    });
  let mut result = Media::default();
  let Some(attachments) = attachments else {
    return result;
  };
  if attachments.len() > 64 {
    result.others += 1;
  }
  for entry in attachments.iter().take(64) {
    let styled = entry.pointer("/styles/attachment").unwrap_or(&Value::Null);
    if let Some(nodes) = styled
      .get("all_subattachments")
      .or_else(|| entry.get("all_subattachments"))
    {
      result.others += 1;
      for node in nodes["nodes"].as_array().into_iter().flatten().take(64) {
        let value = &node["media"];
        if let Some(photo) = photo(value) {
          if result.photos.insert(photo.0.clone()) {
            result.photo_urls.push(photo);
          }
        }
        if value["__typename"] == "Video" {
          result.has_video = true;
          if let Some(id) = video_id(value) {
            result.videos.insert(id.into());
          }
        }
      }
      continue;
    }
    if let Some(photo) = photo(&styled["media"]).or_else(|| photo(&entry["media"])) {
      if result.photos.insert(photo.0.clone()) {
        result.photo_urls.push(photo);
      }
      continue;
    }
    let nodes: Vec<_> = [&styled["media"], &entry["media"]]
      .into_iter()
      .filter(|v| v["__typename"] == "Video")
      .collect();
    result.has_video |= !nodes.is_empty();
    let ids: BTreeSet<_> = nodes.iter().filter_map(|v| video_id(v)).collect();
    if !nodes.is_empty() && ids.len() == 1 && nodes.iter().all(|v| video_id(v).is_some()) {
      result.videos.insert(ids.first().unwrap().to_string());
    } else {
      result.others += 1;
    }
  }
  result
}
fn photo(value: &Value) -> Option<(String, String)> {
  if value["__typename"] != "Photo" {
    return None;
  }
  let url = ["viewer_image", "photo_image", "image"]
    .iter()
    .find_map(|key| value[*key]["uri"].as_str().filter(|v| photo_cdn(v)))?;
  let id = match value["id"].as_str().filter(|id| numeric(id)) {
    Some(id) => id.into(),
    None => {
      let mut stable = reqwest::Url::parse(url).ok()?;
      stable.set_query(None);
      format!(
        "photo:{}",
        hex::encode(Sha256::digest(stable.as_str().as_bytes()))
      )
    }
  };
  Some((id, url.into()))
}
fn photo_attachments(photos: &[(String, String)], source: &Identity) -> Verdict {
  let attachments: Vec<_> = photos
    .iter()
    .map(|(id, url)| serde_json::json!({"id":id, "kind":"image", "url":url, "thumbnail":url}))
    .collect();
  let metadata = serde_json::json!({
    "id":source.post_id.as_deref().or(source.photo_id.as_deref()).unwrap_or(&photos[0].0),
    "url":source.url.as_str(), "extractor":"Facebook",
    "mediaKind":if photos.len() > 1 { "carousel" } else { "image" },
    "attachments":attachments, "thumbnail":photos[0].1,
    "videoCodecs":[], "audioCodecs":[], "videoTracks":[], "audioTracks":[],
    "formats":[], "chapters":[], "subtitleInventory":{"manualLanguages":[],"autoLanguages":[]}
  })
  .to_string();
  Verdict::Verified(Resolution::Attachments { metadata })
}
fn photo_cdn(input: &str) -> bool {
  http::https_url(input)
    .ok()
    .and_then(|url| url.host_str().map(str::to_owned))
    .is_some_and(|host| {
      let host = host
        .strip_prefix("z-")
        .and_then(|h| h.split_once('-'))
        .filter(|(prefix, _)| {
          !prefix.is_empty()
            && prefix.len() <= 4
            && prefix.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .map(|(_, h)| h)
        .unwrap_or(&host);
      host.starts_with("scontent") && host.ends_with(".fbcdn.net")
    })
}
fn video_id(value: &Value) -> Option<&str> {
  value["id"].as_str().filter(|id| numeric(id))
}
fn results<'a>(value: &'a Value, depth: usize, out: &mut Vec<&'a Value>) {
  if depth > 128 {
    return;
  }
  if let Some(data) = value
    .pointer("/__bbox/result/data")
    .filter(|v| v.is_object())
  {
    out.push(data);
  }
  match value {
    Value::Object(map) => {
      for child in map.values() {
        results(child, depth + 1, out);
      }
    }
    Value::Array(list) => {
      for child in list {
        results(child, depth + 1, out);
      }
    }
    _ => {}
  }
}
fn has_media(value: &Value, depth: usize) -> bool {
  if depth > 128 {
    return false;
  }
  if value
    .get("__typename")
    .and_then(Value::as_str)
    .is_some_and(|v| matches!(v, "Photo" | "Video" | "Story"))
  {
    return true;
  }
  match value {
    Value::Object(map) => map.values().any(|v| has_media(v, depth + 1)),
    Value::Array(list) => list.iter().any(|v| has_media(v, depth + 1)),
    _ => false,
  }
}
fn route(value: &Value, depth: usize) -> Option<&str> {
  if depth > 128 {
    return None;
  }
  value
    .pointer("/initialRouteInfo/route/rootView/resource/__dr")
    .and_then(Value::as_str)
    .or_else(|| match value {
      Value::Object(map) => map.values().find_map(|v| route(v, depth + 1)),
      Value::Array(list) => list.iter().find_map(|v| route(v, depth + 1)),
      _ => None,
    })
}

fn photo_url(id: &str) -> String {
  format!("{WWW}/photo/?fbid={id}")
}
