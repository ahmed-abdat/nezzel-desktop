//! Port of Android's ThreadsPostIdentity / ThreadsPostParser.
//! Read only the requested post's TargetQuery; recommendations and replies cannot supply media.
use super::{html, http, Outcome, Resolution};
use crate::models::parsed::{
  AttachmentKind, MediaAttachment, MediaKind, ParsedSingleVideo, SubtitleInventory,
};
use regex::Regex;
use reqwest::Url;
use serde::{de, Deserialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::LazyLock;

pub(super) const MATCHER: crate::extract::registry::Matcher =
  crate::extract::registry::Matcher::reader(
    crate::extract::outcome::ReaderId::Threads,
    "threads-post",
    20,
    |raw| Identity::parse(raw).is_some(),
  );

const CAP: usize = 3 * 512 * 1024;
static POST: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^/(?:@|%40)([A-Za-z0-9._]{1,30})/post/([A-Za-z0-9_-]{1,32})(?:/media)?/?$").unwrap()
});
static PRELOADER: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"adp_BarcelonaPostPageTargetQueryRelayPreloader_[0-9a-fA-F]{1,64}",\{"__bbox":"#)
    .unwrap()
});
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^/[A-Za-z0-9_./%-]{1,2048}\.(?:jpg|jpeg|png|webp|gif|avif)$").unwrap()
});
static VIDEO: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^/[A-Za-z0-9_./%-]{1,2048}\.mp4$").unwrap());

pub(super) struct Identity {
  pub url: Url,
  pub code: String,
}
pub(super) fn is_host(host: &str) -> bool {
  matches!(
    host,
    "threads.com" | "www.threads.com" | "threads.net" | "www.threads.net"
  )
}
impl Identity {
  pub fn parse(input: &str) -> Option<Self> {
    let url = http::https_url(input).ok()?;
    if !is_host(url.host_str()?) {
      return None;
    }
    let captures = POST.captures(url.path())?;
    let user = &captures[1];
    if user.starts_with('.') || user.ends_with('.') {
      return None;
    }
    let code = captures[2].to_owned();
    Some(Self {
      url: Url::parse(&format!("https://www.threads.com/@{user}/post/{code}")).ok()?,
      code,
    })
  }
}

pub(super) async fn read<T: http::Transport>(input: &str, transport: &T) -> Outcome {
  let Some(identity) = Identity::parse(input) else {
    return Outcome::NoMatch;
  };
  // Unlike generic pages, Threads never follows a redirect (login and challenge included).
  let Ok(response) = transport.get(&identity.url, CAP).await else {
    return Outcome::Unknown;
  };
  if response.status != 200 || response.body.len() > CAP {
    return Outcome::Unknown;
  }
  let Ok(body) = std::str::from_utf8(&response.body) else {
    return Outcome::Unknown;
  };
  let Some(post) = parse(body, &identity.code) else {
    return Outcome::Unknown;
  };
  // yt-dlp and the picker will fetch these URLs themselves. Match the generic reader's
  // public-address admission before handing off any stream or preview URL.
  let mut checked = BTreeSet::new();
  for attachment in &post.attachments {
    for raw in std::iter::once(&attachment.url).chain(attachment.thumbnail.iter()) {
      let Ok(url) = http::https_url(raw) else {
        return Outcome::Unknown;
      };
      if checked.insert(url.host_str().map(str::to_owned))
        && transport.admit_media(&url).await.is_err()
      {
        return Outcome::Unknown;
      }
    }
  }
  if !post.carousel
    && post
      .attachments
      .iter()
      .all(|a| matches!(a.kind, AttachmentKind::Video))
  {
    return post.video.map_or(Outcome::Unknown, |url| {
      Outcome::Verified(Resolution::Passthrough(url))
    });
  }
  match serde_json::to_string(&post.into_media(input, &identity.code)) {
    Ok(metadata) => Outcome::Verified(Resolution::Attachments { metadata }),
    Err(_) => Outcome::Unknown,
  }
}

pub(super) struct Post {
  pub title: Option<String>,
  pub author: Option<String>,
  pub video: Option<String>,
  pub attachments: Vec<MediaAttachment>,
  carousel: bool,
}

impl Post {
  fn into_media(self, source: &str, code: &str) -> ParsedSingleVideo {
    let images = self
      .attachments
      .iter()
      .any(|a| matches!(a.kind, AttachmentKind::Image));
    let videos = self
      .attachments
      .iter()
      .any(|a| matches!(a.kind, AttachmentKind::Video));
    let kind = if images && videos {
      MediaKind::Mixed
    } else if self.carousel {
      MediaKind::Carousel
    } else if images {
      MediaKind::Image
    } else {
      MediaKind::Video
    };
    ParsedSingleVideo {
      id: code.into(),
      fetch_engine: None,
      resolved_url: None,
      media_kind: Some(kind),
      thumbnail: self.attachments.first().and_then(|a| a.thumbnail.clone()),
      attachments: self.attachments,
      url: Some(source.into()),
      title: self.title,
      uploader: self.author,
      extractor: Some("Threads".into()),
      description: None,
      uploader_id: None,
      views: None,
      comments: None,
      likes: None,
      dislikes: None,
      duration: None,
      rating: None,
      video_codecs: vec![],
      audio_codecs: vec![],
      video_tracks: vec![],
      audio_tracks: vec![],
      formats: vec![],
      subtitle_inventory: SubtitleInventory::default(),
      chapters: vec![],
      filesize: None,
    }
  }
}

pub(super) fn parse(body: &str, code: &str) -> Option<Post> {
  if body.len() > CAP {
    return None;
  }
  let tags = html::tags(body)?;
  let mut found = None;
  for script in tags.iter().filter(|t| t.name == "script") {
    for matched in PRELOADER.find_iter(script.text) {
      let start = matched.end() - "{\"__bbox\":".len();
      let root = serde_json::Deserializer::from_str(&script.text[start..])
        .into_iter::<UniqueValue>()
        .next()?
        .ok()?
        .0;
      let media = root.pointer("/__bbox/result/data/media")?;
      if !same_post(media, code) || found.is_some() {
        return None;
      }
      found = Some(media.clone());
    }
  }
  let media = found?;
  let mut post = Post {
    title: first_line(media.pointer("/caption/text")),
    author: first_line(media.pointer("/user/username")),
    video: None,
    attachments: Vec::new(),
    carousel: false,
  };
  let attachment = if media["media_type"] == 19 {
    media.pointer("/text_post_app_info/linked_inline_media")?
  } else {
    &media
  };
  match attachment["media_type"].as_u64()? {
    1 | 2 => add_media(&mut post, attachment),
    8 => {
      post.carousel = true;
      let items = attachment["carousel_media"].as_array()?;
      if items.is_empty() || items.len() > 40 {
        return None;
      }
      for item in items {
        add_media(&mut post, item);
      }
    }
    _ => return None,
  }
  (!post.attachments.is_empty()).then_some(post)
}

// Android enables STRICT_DUPLICATE_DETECTION: conflicting identity/media keys fail closed.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
  fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    struct Visitor;
    impl<'de> de::Visitor<'de> for Visitor {
      type Value = Value;
      fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
      }
      fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
      }
      fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(value.into())
      }
      fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
      }
      fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
      }
      fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(value)
          .map(Value::Number)
          .ok_or_else(|| E::custom("nonfinite number"))
      }
      fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
      }
      fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        Ok(value.into())
      }
      fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<UniqueValue>()? {
          values.push(value.0);
        }
        Ok(Value::Array(values))
      }
      fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some((key, value)) = map.next_entry::<String, UniqueValue>()? {
          if values.insert(key, value.0).is_some() {
            return Err(de::Error::custom("duplicate key"));
          }
        }
        Ok(Value::Object(values))
      }
    }
    deserializer.deserialize_any(Visitor).map(Self)
  }
}

fn first_line(value: Option<&Value>) -> Option<String> {
  let line = value?.as_str()?.lines().next()?.trim();
  (!line.is_empty()).then(|| line.chars().take(512).collect())
}

fn same_post(media: &Value, code: &str) -> bool {
  let posted = media["code"].as_str().filter(|s| !s.is_empty());
  let pk = media.get("pk").and_then(|v| {
    v.as_str()
      .map(str::to_owned)
      .or_else(|| v.as_u64().map(|n| n.to_string()))
  });
  if posted.is_none() && pk.is_none() {
    return false;
  }
  posted.is_none_or(|s| s == code)
    && pk.is_none_or(|pk| media_id(code).as_deref() == Some(pk.as_str()))
}

// Decimal digits keep Android's arbitrary-precision shortcode semantics without a new dependency.
fn media_id(code: &str) -> Option<String> {
  const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let mut digits = vec![0u16];
  for byte in code.bytes() {
    let mut carry = ALPHABET.bytes().position(|b| b == byte)? as u16;
    for digit in &mut digits {
      let value = *digit * 64 + carry;
      *digit = value % 10;
      carry = value / 10;
    }
    while carry > 0 {
      digits.push(carry % 10);
      carry /= 10;
    }
  }
  Some(
    digits
      .iter()
      .rev()
      .map(|n| char::from(b'0' + *n as u8))
      .collect(),
  )
}

fn cdn_url(value: &Value, video: bool) -> Option<String> {
  let raw = value["url"].as_str()?;
  let url = http::https_url(raw).ok()?;
  let host = url.host_str()?;
  if !(host.ends_with(".cdninstagram.com") || host.ends_with(".fbcdn.net"))
    || url.path().contains("..")
    || !(if video { &VIDEO } else { &IMAGE }).is_match(url.path())
  {
    return None;
  }
  Some(url.into())
}

fn add_media(post: &mut Post, media: &Value) {
  let video = match media["media_type"].as_u64() {
    Some(1) => false,
    Some(2) => true,
    _ => return,
  };
  let versions = if video {
    media.get("video_versions")
  } else {
    media.pointer("/image_versions2/candidates")
  };
  let mut best = None;
  for candidate in versions.and_then(Value::as_array).into_iter().flatten() {
    let Some(url) = cdn_url(candidate, video) else {
      continue;
    };
    let score = candidate["width"]
      .as_u64()
      .unwrap_or(0)
      .saturating_mul(if video {
        candidate["height"].as_u64().unwrap_or(0)
      } else {
        1
      });
    if best.as_ref().is_none_or(|(size, _, _)| score > *size) {
      best = Some((score, url, candidate));
    }
  }
  let Some((_, url, candidate)) = best else {
    return;
  };
  if video && post.video.is_none() {
    post.video = Some(url.clone());
  }
  let mut stable = Url::parse(&url).expect("admitted CDN URL");
  stable.set_query(None);
  let id = media["pk"]
    .as_str()
    .or_else(|| media["id"].as_str())
    .map(str::to_owned)
    .unwrap_or_else(|| stable.to_string());
  let thumbnail = if video {
    media
      .pointer("/image_versions2/candidates")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter_map(|c| cdn_url(c, false))
      .next()
  } else {
    versions
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter(|c| {
        c["width"]
          .as_u64()
          .is_some_and(|w| w > 0 && w < candidate["width"].as_u64().unwrap_or(0))
      })
      .filter_map(|c| cdn_url(c, false).map(|u| (c["width"].as_u64().unwrap_or(0), u)))
      .min_by_key(|(w, _)| *w)
      .map(|(_, u)| u)
      .or_else(|| Some(url.clone()))
  };
  post.attachments.push(MediaAttachment {
    id,
    file_index: None,
    kind: if video {
      AttachmentKind::Video
    } else {
      AttachmentKind::Image
    },
    url,
    width: candidate["width"].as_u64(),
    height: candidate["height"].as_u64(),
    thumbnail,
    alt: post.title.clone(),
  });
}
