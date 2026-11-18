//! Port of Android's ThreadsPostIdentity / ThreadsPostParser.
//! Read only the requested post's TargetQuery; recommendations and replies cannot supply media.
use super::{html, http, Failure, Reason, Resolution, Verdict};
use crate::models::parsed::{
  AttachmentKind, MediaAttachment, MediaKind, ParsedSingleVideo, SubtitleInventory,
};
use regex::Regex;
use reqwest::Url;
use serde::{de, Deserialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::LazyLock;

pub(super) const MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Threads,
  "threads-post",
  20,
  |raw| Identity::parse(raw).is_some(),
);

const CAP: usize = 4 * 1024 * 1024;
static POST: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^/(?:@|%40)([A-Za-z0-9._]{1,30})/post/([A-Za-z0-9_-]{1,32})(?:/media)?/?$").unwrap()
});
static SHARE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^/(?:share|t)/([A-Za-z0-9_-]{1,64})/?$").unwrap());
static PRELOADER: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"adp_BarcelonaPostPageTargetQueryRelayPreloader_[0-9a-fA-F]{1,64}",\{"__bbox":"#)
    .unwrap()
});
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^/[A-Za-z0-9_./%-]{1,2048}\.(?:jpg|jpeg|png|webp|gif|avif|heic|heif)$").unwrap()
});
static VIDEO: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^/[A-Za-z0-9_./%-]{1,2048}\.mp4$").unwrap());

pub(super) struct Identity {
  pub url: Url,
  pub code: String,
  share: bool,
}
pub(super) fn is_host(host: &str) -> bool {
  matches!(
    host,
    "threads.com" | "www.threads.com" | "threads.net" | "www.threads.net"
  )
}
impl Identity {
  pub fn parse(input: &str) -> Option<Self> {
    let mut url = http::https_url(input).ok()?;
    if !is_host(url.host_str()?) {
      return None;
    }
    if let Some(captures) = SHARE.captures(url.path()) {
      let code = captures[1].to_owned();
      url.set_query(None);
      return Some(Self {
        url,
        code,
        share: true,
      });
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
      share: false,
    })
  }
}

pub(super) async fn read<T: http::Transport>(input: &str, transport: &T) -> Verdict {
  read_with_cache(input, transport, &QUERY).await
}
async fn read_with_cache<T: http::Transport>(
  input: &str,
  transport: &T,
  cache: &QueryCache,
) -> Verdict {
  let Some(mut identity) = Identity::parse(input) else {
    return Verdict::NoMatch;
  };
  // Only share links may redirect, through admitted Threads identities to a bound post.
  let mut retried = false;
  let mut redirects = 0;
  let mut seen = BTreeSet::new();
  let response = loop {
    if !seen.insert(identity.url.clone()) {
      return Verdict::Unknown;
    }
    let response = loop {
      let failure = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        transport.get(&identity.url, CAP),
      )
      .await
      {
        Ok(Ok(response)) => break response,
        Ok(Err(http::FetchError::Network)) => Failure::Network,
        Err(_) => Failure::Timeout,
        Ok(Err(_)) => return Verdict::Unknown,
      };
      if retried {
        return Verdict::Failed(failure);
      }
      retried = true;
    };
    if identity.share && matches!(response.status, 301 | 302 | 303 | 307 | 308) {
      if redirects == 3 {
        return Verdict::Unknown;
      }
      let next = response
        .location
        .as_deref()
        .and_then(|location| identity.url.join(location).ok())
        .and_then(|url| Identity::parse(url.as_str()));
      let Some(next) = next else {
        return Verdict::Unknown;
      };
      identity = next;
      redirects += 1;
      continue;
    }
    break response;
  };
  if identity.share || response.status != 200 || response.body.len() > CAP {
    return Verdict::Unknown;
  }
  let Ok(body) = std::str::from_utf8(&response.body) else {
    return Verdict::Unknown;
  };
  let media = match post_data(body, &identity.code) {
    Ok(Some(media)) => media,
    Err(()) => return changed(),
    Ok(None) if body.contains("BarcelonaGeoBlockedErrorRoot") => return login_required(),
    Ok(None) => match tokio::time::timeout(
      std::time::Duration::from_secs(10),
      graphql(&identity, body, transport, cache),
    )
    .await
    {
      Ok(Ok(Some(media))) => media,
      Ok(Ok(None)) => return login_required(),
      _ => return changed(),
    },
  };
  let Some(post) = parse_media(&media) else {
    return changed();
  };
  if post.no_media {
    return Verdict::Verified(Resolution::Unsupported {
      reason: Reason::NoMedia,
    });
  }
  // yt-dlp and the picker will fetch these URLs themselves. Match the generic reader's
  // public-address admission before handing off any stream or preview URL.
  let mut checked = BTreeSet::new();
  for attachment in &post.attachments {
    for raw in std::iter::once(&attachment.url).chain(attachment.thumbnail.iter()) {
      let Ok(url) = http::https_url(raw) else {
        return Verdict::Unknown;
      };
      if checked.insert(url.host_str().map(str::to_owned))
        && transport.admit_media(&url).await.is_err()
      {
        return Verdict::Unknown;
      }
    }
  }
  match serde_json::to_string(&post.into_media(identity.url.as_str(), &identity.code)) {
    Ok(metadata) => Verdict::Verified(Resolution::Attachments { metadata }),
    Err(_) => Verdict::Unknown,
  }
}

pub(super) struct Post {
  pub title: Option<String>,
  pub author: Option<String>,
  pub attachments: Vec<MediaAttachment>,
  carousel: bool,
  no_media: bool,
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
      playlist_child: false,
      id: code.into(),
      fetch_engine: None,
      resolved_url: None,
      media_kind: Some(kind),
      thumbnail: self.attachments.first().and_then(|a| a.thumbnail.clone()),
      attachments: self.attachments,
      url: Some(source.into()),
      title: self.title.or_else(|| self.author.clone()),
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

#[cfg(test)]
pub(super) fn parse(body: &str, code: &str) -> Option<Post> {
  parse_media(&post_data(body, code).ok()??)
}

// Missing data can be retried with another UA; conflicting/malformed data cannot.
fn post_data(body: &str, code: &str) -> Result<Option<Value>, ()> {
  if body.len() > CAP {
    return Err(());
  }
  let tags = html::tags(body).ok_or(())?;
  let mut found = None;
  for script in tags.iter().filter(|t| t.name == "script") {
    for matched in PRELOADER.find_iter(script.text) {
      let start = matched.end() - "{\"__bbox\":".len();
      let root = serde_json::Deserializer::from_str(&script.text[start..])
        .into_iter::<UniqueValue>()
        .next()
        .ok_or(())?
        .map_err(|_| ())?
        .0;
      let media = root.pointer("/__bbox/result/data/media").ok_or(())?;
      if media.is_null() {
        continue;
      }
      if !same_post(media, code) || found.is_some() {
        return Err(());
      }
      found = Some(media.clone());
    }
  }
  Ok(found)
}

fn parse_media(media: &Value) -> Option<Post> {
  let mut post = Post {
    title: first_line(media.pointer("/caption/text")),
    author: first_line(media.pointer("/user/username")),
    attachments: Vec::new(),
    carousel: false,
    no_media: false,
  };
  let attachment = if media["media_type"] == 19 {
    let linked = media.pointer("/text_post_app_info/linked_inline_media");
    let inline = linked.filter(|value| !value.is_null()).or_else(|| {
      // An explicit quote is part of the requested post, unlike replies or recommendations.
      let quote = ["quoted_attachment_post", "quoted_post"]
        .iter()
        .find_map(|key| {
          media
            .pointer(&format!("/text_post_app_info/share_info/{key}"))
            .filter(|v| !v.is_null())
        })?;
      let identity = Identity::parse(quote["permalink"].as_str()?)?;
      (!identity.share && same_post(quote, &identity.code)).then_some(quote)
    });
    match inline {
      Some(inline) => inline,
      None if linked.is_some() && bound_text_only(media) => {
        post.no_media = true;
        return Some(post);
      }
      None => return None,
    }
  } else {
    media
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
      if post.attachments.len() != items.len() {
        return None;
      }
    }
    _ => return None,
  }
  (!post.attachments.is_empty()).then_some(post)
}

// A link-preview image belongs to the external page, not to this post. Require the
// complete empty-media shape; missing/hidden/unavailable representations remain inconclusive.
fn bound_text_only(media: &Value) -> bool {
  let empty = |value: Option<&Value>| {
    value.is_some_and(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
  };
  let info = &media["text_post_app_info"];
  empty(media.get("carousel_media"))
    && empty(media.get("video_versions"))
    && (media.get("image_versions2").is_some_and(Value::is_null)
      || empty(media.pointer("/image_versions2/candidates")))
    && info["is_post_unavailable"] != true
    && [
      "attachment_tombstone_info",
      "post_tombstone_info",
      "snippet_attachment_info",
      "platform_podcast_info",
      "platform_podcast_episode_info",
    ]
    .iter()
    .all(|key| info[key].is_null())
    && ["quoted_attachment_post", "quoted_post", "reposted_post"]
      .iter()
      .all(|key| info["share_info"][key].is_null())
    && media["giphy_media_info"].is_null()
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

fn changed() -> Verdict {
  Verdict::Verified(Resolution::Unsupported {
    reason: Reason::Unavailable,
  })
}
fn login_required() -> Verdict {
  Verdict::Verified(Resolution::Unsupported {
    reason: Reason::Login,
  })
}

#[derive(Clone)]
struct Query {
  doc: String,
  providers: BTreeSet<String>,
}
struct QueryCache(tokio::sync::Mutex<QueryState>);
struct QueryState {
  value: Option<(Query, tokio::time::Instant)>,
  retry_after: Option<tokio::time::Instant>,
}
impl QueryCache {
  const fn new() -> Self {
    Self(tokio::sync::Mutex::const_new(QueryState {
      value: None,
      retry_after: None,
    }))
  }
  #[cfg(test)]
  fn set(&self, query: Option<Query>) -> Result<(), ()> {
    self.0.try_lock().map_err(|_| ())?.value = query.map(|q| (q, tokio::time::Instant::now()));
    Ok(())
  }
}
static OPERATION: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"__d\("BarcelonaPostPageTargetQuery_threadsRelayOperation",\[\],\(function\([^)]*\)\{\w+\.exports="(\d+)""#).unwrap()
});
static PROVIDER: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(__relay_internal__pv__\w+relayprovider):").unwrap());
static LSD: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#""LSD",\[\],\{"token":"([^"\\]{1,256})""#).unwrap());
static DTSG: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#""DTSGInitialData",\[\],\{"token":"([^"\\]{1,512})""#).unwrap());
static QUERY: QueryCache = QueryCache::new();

async fn discover<T: http::Transport>(transport: &T, lsd: &str) -> Option<Query> {
  let mut url = Url::parse("https://www.threads.com/ajax/bootloader-endpoint/").ok()?;
  url.query_pairs_mut().extend_pairs([
    ("modules", "BarcelonaPostPageTargetQuery$Parameters.threads"),
    ("__a", "1"),
    ("__comet_req", "122"),
    ("__user", "0"),
    ("lsd", lsd),
  ]);
  let response = transport.threads_api(&url, CAP, lsd, None).await.ok()?;
  if response.status != 200 || response.body.len() > CAP {
    return None;
  }
  let body = std::str::from_utf8(&response.body)
    .ok()?
    .trim_start()
    .strip_prefix("for (;;);")
    .unwrap_or(std::str::from_utf8(&response.body).ok()?);
  let root: Value = serde_json::from_str(body).ok()?;
  let resources = root.pointer("/hrp/hsrp/hblp/rsrcMap")?.as_object()?;
  if resources.len() > 8 {
    return None;
  }
  let mut doc = None;
  let mut providers = BTreeSet::new();
  for resource in resources.values().filter(|v| v["type"] == "js") {
    let url = http::https_url(resource["src"].as_str()?).ok()?;
    if url.host_str() != Some("static.cdninstagram.com") || !url.path().starts_with("/rsrc.php/") {
      return None;
    }
    let response = transport.get(&url, CAP).await.ok()?;
    if response.status != 200 || response.body.len() > CAP {
      return None;
    }
    let script = std::str::from_utf8(&response.body).ok()?;
    if let Some(found) = OPERATION.captures(script) {
      doc = Some(found[1].to_owned());
    }
    if let Some(start) = script.find("__d(\"BarcelonaPostPageTargetQuery$Parameters.threads\"") {
      let block = &script[start..];
      let block = &block[..block[4..].find("__d(").map_or(block.len(), |n| n + 4)];
      providers.extend(PROVIDER.captures_iter(block).map(|c| c[1].to_owned()));
    }
  }
  if providers.is_empty() || providers.len() > 128 {
    return None;
  }
  Some(Query {
    doc: doc?,
    providers,
  })
}

async fn graphql<T: http::Transport>(
  identity: &Identity,
  body: &str,
  transport: &T,
  cache: &QueryCache,
) -> Result<Option<Value>, ()> {
  let lsd = LSD.captures(body).ok_or(())?[1].to_owned();
  let mut state = cache.0.lock().await;
  if state
    .value
    .as_ref()
    .is_some_and(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(86400))
  {
    state.value = None;
  }
  if state.value.is_none()
    && state
      .retry_after
      .is_some_and(|at| at > tokio::time::Instant::now())
  {
    return Err(());
  }
  for attempt in 0..2 {
    if state.value.is_none() {
      // Set before awaiting: cancellation also observes the discovery backoff.
      state.retry_after = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(30));
      let query =
        tokio::time::timeout(std::time::Duration::from_secs(7), discover(transport, &lsd))
          .await
          .ok()
          .flatten()
          .ok_or(())?;
      state.value = Some((query, tokio::time::Instant::now()));
      state.retry_after = None;
    }
    let query = &state.value.as_ref().ok_or(())?.0;
    match query_post(identity, body, transport, &lsd, query).await {
      Ok(media) => return Ok(media),
      Err(()) => {
        state.value = None;
        if attempt == 1 {
          state.retry_after =
            Some(tokio::time::Instant::now() + std::time::Duration::from_secs(30));
        }
      }
    }
  }
  Err(())
}
async fn query_post<T: http::Transport>(
  identity: &Identity,
  body: &str,
  transport: &T,
  lsd: &str,
  query: &Query,
) -> Result<Option<Value>, ()> {
  let mut variables = serde_json::Map::new();
  for key in &query.providers {
    variables.insert(key.clone(), false.into());
  }
  variables.insert("postID".into(), media_id(&identity.code).ok_or(())?.into());
  let mut encoded = Url::parse("https://www.threads.com/").map_err(|_| ())?;
  encoded.query_pairs_mut().extend_pairs([
    ("lsd", lsd),
    ("doc_id", query.doc.as_str()),
    ("fb_api_req_friendly_name", "BarcelonaPostPageTargetQuery"),
    ("server_timestamps", "true"),
    ("variables", &Value::Object(variables).to_string()),
  ]);
  if transport.has_session() {
    let dtsg = DTSG.captures(body).ok_or(())?[1].to_owned();
    encoded.query_pairs_mut().append_pair("fb_dtsg", &dtsg);
  }
  let form = encoded.query().ok_or(())?;
  let url = Url::parse("https://www.threads.com/api/graphql").map_err(|_| ())?;
  let response = transport
    .threads_api(&url, CAP, lsd, Some(form))
    .await
    .map_err(|_| ())?;
  if response.status != 200 || response.body.len() > CAP {
    return Err(());
  }
  let root: UniqueValue = serde_json::from_slice(&response.body).map_err(|_| ())?;
  if root.0.get("errors").is_some() {
    return Err(());
  }
  let media = root.0.pointer("/data/media").ok_or(())?;
  if media.is_null() {
    return Ok(None);
  }
  if !same_post(media, &identity.code) {
    return Err(());
  }
  Ok(Some(media.clone()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::{collections::VecDeque, sync::Mutex};
  const CODE: &str = "Dfx3XnMjokc";
  const URL: &str = "https://www.threads.com/@fixture/post/Dfx3XnMjokc";
  const QUOTE: &str =
    include_str!("../../tests/fixtures/threads/re-2026-11-16/ssr-targetquery-quote-post.json");
  fn page(json: &str) -> String {
    format!("<script>[\"adp_BarcelonaPostPageTargetQueryRelayPreloader_ab\",{json}]</script>")
  }
  fn response(body: &str) -> http::Response {
    http::Response {
      status: 200,
      body: body.as_bytes().to_vec(),
      content_type: Some("text/html".into()),
      location: None,
      content_length: None,
    }
  }
  struct Mock(Mutex<VecDeque<(bool, String)>>);
  impl Mock {
    fn pop(&self, api: bool) -> http::Response {
      let (expected, body) = self.0.lock().unwrap().pop_front().expect("extra request");
      assert_eq!(expected, api);
      response(&body)
    }
  }
  impl http::Transport for Mock {
    async fn get(&self, url: &Url, cap: usize) -> Result<http::Response, http::FetchError> {
      assert_eq!(cap, CAP);
      assert!(
        url.host_str() == Some("www.threads.com")
          || url.host_str() == Some("static.cdninstagram.com")
      );
      Ok(self.pop(false))
    }
    async fn threads_api(
      &self,
      url: &Url,
      cap: usize,
      lsd: &str,
      form: Option<&str>,
    ) -> Result<http::Response, http::FetchError> {
      assert_eq!(cap, CAP);
      assert_eq!(lsd, "fixture-token");
      if let Some(form) = form {
        assert_eq!(url.path(), "/api/graphql");
        let parsed = Url::parse(&format!("https://www.threads.com/?{form}")).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs["doc_id"], "123456789");
        let vars: Value = serde_json::from_str(&pairs["variables"]).unwrap();
        assert_eq!(vars["postID"], media_id(CODE).unwrap());
        assert_eq!(vars["__relay_internal__pv__Examplerelayprovider"], false);
      } else {
        assert_eq!(url.path(), "/ajax/bootloader-endpoint/");
      }
      Ok(self.pop(true))
    }
    async fn admit_media(&self, _url: &Url) -> Result<(), http::FetchError> {
      Ok(())
    }
  }
  #[test]
  fn threads_live_shapes_parse_bound_video_quote_and_heic() {
    for (json, code, count, video) in [
      (QUOTE, CODE, 1, true),
      (
        include_str!(
          "../../tests/fixtures/threads/re-2026-11-16/ssr-targetquery-linked-inline-video.json"
        ),
        "DXVbTUPl-Qy",
        1,
        true,
      ),
      (
        include_str!(
          "../../tests/fixtures/threads/re-2026-11-16/ssr-targetquery-carousel-heic.json"
        ),
        "DfxGlmfDHmb",
        3,
        false,
      ),
    ] {
      let post = parse(&page(json), code).expect(code);
      assert_eq!(post.attachments.len(), count);
      assert_eq!(post.title.as_deref(), Some("Outer fixture caption"));
      for a in post.attachments {
        assert_eq!(matches!(a.kind, AttachmentKind::Video), video);
        if !video {
          assert!(a.url.contains(".heic"));
        }
      }
      assert!(parse(&page(json), "WrongCode").is_none());
    }
    let altered = QUOTE.replace("DfwNADCjeqs", "WrongCode");
    assert!(parse(&page(&altered), CODE).is_none());
  }
  #[test]
  fn threads_page_accepts_three_megabytes_decoded() {
    let mut body = page(QUOTE);
    body.push_str(&" ".repeat(3 * 1024 * 1024));
    assert!(parse(&body, CODE).is_some());
    body.push_str(&" ".repeat(CAP));
    assert!(parse(&body, CODE).is_none());
  }

  #[tokio::test]
  async fn threads_restricted_page_requests_login_without_api() {
    let mock = Mock(Mutex::new(
      [(
        false,
        include_str!(
          "../../tests/fixtures/threads/re-2026-11-16/ssr-restricted-geoblocked-props.json"
        )
        .into(),
      )]
      .into(),
    ));
    assert_eq!(
      read_with_cache(URL, &mock, &QueryCache::new())
        .await
        .resolution(URL),
      Resolution::Unsupported {
        reason: Reason::Login
      }
    );
    assert!(mock.0.lock().unwrap().is_empty());
  }
  #[tokio::test]
  async fn threads_shell_discovers_query_once_and_posts_bound_variables() {
    let shell = r#"<script>["LSD",[],{"token":"fixture-token"}]</script>"#;
    let boot = r#"for (;;);{"hrp":{"hsrp":{"hblp":{"rsrcMap":{"a":{"type":"js","src":"https://static.cdninstagram.com/rsrc.php/v4/query.js"}}}}}}"#;
    let js = r#"__d("BarcelonaPostPageTargetQuery_threadsRelayOperation",[],(function(a){a.exports="123456789"}),null);__d("BarcelonaPostPageTargetQuery$Parameters.threads",[],(function(a){var p={__relay_internal__pv__Examplerelayprovider:false};}),null);"#;
    let result =
      include_str!("../../tests/fixtures/threads/re-2026-11-16/graphql-targetquery-response.json");
    let mock = Mock(Mutex::new(
      [
        (false, shell.into()),
        (true, boot.into()),
        (false, js.into()),
        (true, result.into()),
        (false, shell.into()),
        (true, result.into()),
      ]
      .into(),
    ));
    let cache = QueryCache::new();
    for _ in 0..2 {
      assert!(matches!(
        read_with_cache(URL, &mock, &cache).await,
        Verdict::Verified(Resolution::Attachments { .. })
      ));
    }
    assert!(mock.0.lock().unwrap().is_empty());
  }

  #[tokio::test(start_paused = true)]
  async fn threads_discovery_recovers_after_failure_backoff() {
    let shell = r#"<script>["LSD",[],{"token":"fixture-token"}]</script>"#;
    let boot = r#"for (;;);{"hrp":{"hsrp":{"hblp":{"rsrcMap":{"a":{"type":"js","src":"https://static.cdninstagram.com/rsrc.php/v4/query.js"}}}}}}"#;
    let js = r#"__d("BarcelonaPostPageTargetQuery_threadsRelayOperation",[],(function(a){a.exports="123456789"}),null);__d("BarcelonaPostPageTargetQuery$Parameters.threads",[],(function(a){var p={__relay_internal__pv__Examplerelayprovider:false};}),null);"#;
    let data =
      include_str!("../../tests/fixtures/threads/re-2026-11-16/graphql-targetquery-response.json");
    let cache = QueryCache::new();
    let mock = Mock(Mutex::new(
      [
        (true, "invalid".into()),
        (true, boot.into()),
        (false, js.into()),
        (true, data.into()),
      ]
      .into(),
    ));
    let id = Identity::parse(URL).unwrap();
    assert!(graphql(&id, shell, &mock, &cache).await.is_err());
    assert!(graphql(&id, shell, &mock, &cache).await.is_err());
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    assert!(graphql(&id, shell, &mock, &cache).await.unwrap().is_some());
    assert!(mock.0.lock().unwrap().is_empty());
  }

  #[tokio::test(start_paused = true)]
  async fn threads_query_refreshes_on_error_wrong_post_and_expiry() {
    let shell = r#"<script>["LSD",[],{"token":"fixture-token"}]</script>"#;
    let boot = r#"for (;;);{"hrp":{"hsrp":{"hblp":{"rsrcMap":{"a":{"type":"js","src":"https://static.cdninstagram.com/rsrc.php/v4/query.js"}}}}}}"#;
    let js = r#"__d("BarcelonaPostPageTargetQuery_threadsRelayOperation",[],(function(a){a.exports="123456789"}),null);__d("BarcelonaPostPageTargetQuery$Parameters.threads",[],(function(a){var p={__relay_internal__pv__Examplerelayprovider:false};}),null);"#;
    let data =
      include_str!("../../tests/fixtures/threads/re-2026-11-16/graphql-targetquery-response.json");
    for failure in [
      Some(r#"{"errors":[{"message":"unknown document"}]}"#.to_owned()),
      Some(data.replace(CODE, "WrongCode")),
      None,
    ] {
      let cache = QueryCache::new();
      cache
        .set(Some(Query {
          doc: "123456789".into(),
          providers: ["__relay_internal__pv__Examplerelayprovider".into()].into(),
        }))
        .unwrap_or_else(|_| panic!("cache"));
      let mut replies = VecDeque::new();
      if let Some(body) = failure {
        replies.push_back((true, body));
      } else {
        tokio::time::advance(std::time::Duration::from_secs(86401)).await;
      }
      replies.extend([(true, boot.into()), (false, js.into()), (true, data.into())]);
      let mock = Mock(Mutex::new(replies));
      assert!(
        graphql(&Identity::parse(URL).unwrap(), shell, &mock, &cache)
          .await
          .unwrap()
          .is_some()
      );
      assert!(mock.0.lock().unwrap().is_empty());
    }
  }
  #[tokio::test]
  async fn threads_signed_graphql_uses_page_csrf_token() {
    struct Signed(Mock);
    impl http::Transport for Signed {
      fn has_session(&self) -> bool {
        true
      }
      async fn get(&self, url: &Url, cap: usize) -> Result<http::Response, http::FetchError> {
        http::Transport::get(&self.0, url, cap).await
      }
      async fn threads_api(
        &self,
        url: &Url,
        cap: usize,
        lsd: &str,
        form: Option<&str>,
      ) -> Result<http::Response, http::FetchError> {
        let encoded = Url::parse(&format!("https://www.threads.com/?{}", form.unwrap())).unwrap();
        assert!(encoded
          .query_pairs()
          .any(|(key, value)| key == "fb_dtsg" && value == "fixture-csrf"));
        http::Transport::threads_api(&self.0, url, cap, lsd, form).await
      }
      async fn admit_media(&self, _url: &Url) -> Result<(), http::FetchError> {
        Ok(())
      }
    }
    let cache = QueryCache::new();
    cache
      .set(Some(Query {
        doc: "123456789".into(),
        providers: ["__relay_internal__pv__Examplerelayprovider".into()].into(),
      }))
      .unwrap_or_else(|_| panic!("cache"));
    let mock=Signed(Mock(Mutex::new([
      (false,r#"<script>["LSD",[],{"token":"fixture-token"}],["DTSGInitialData",[],{"token":"fixture-csrf"}]</script>"#.into()),
      (true,include_str!("../../tests/fixtures/threads/re-2026-11-16/graphql-targetquery-response.json").into())
    ].into())));
    assert!(matches!(
      read_with_cache(URL, &mock, &cache).await,
      Verdict::Verified(Resolution::Attachments { .. })
    ));
    assert!(mock.0 .0.lock().unwrap().is_empty());
  }

  #[tokio::test]
  async fn threads_graphql_null_requires_login_and_wrong_post_is_temporary() {
    for (body, reason) in [
      (
        include_str!(
          "../../tests/fixtures/threads/re-2026-11-16/graphql-restricted-anonymous-response.json"
        )
        .to_owned(),
        Reason::Login,
      ),
      (
        include_str!(
          "../../tests/fixtures/threads/re-2026-11-16/graphql-targetquery-response.json"
        )
        .replace(CODE, "WrongCode"),
        Reason::Unavailable,
      ),
    ] {
      let cache = QueryCache::new();
      cache
        .set(Some(Query {
          doc: "123456789".into(),
          providers: ["__relay_internal__pv__Examplerelayprovider".into()].into(),
        }))
        .unwrap_or_else(|_| panic!("cache"));
      let mock = Mock(Mutex::new(
        [
          (
            false,
            r#"<script>["LSD",[],{"token":"fixture-token"}]</script>"#.into(),
          ),
          (true, body),
        ]
        .into(),
      ));
      if reason == Reason::Unavailable {
        mock
          .0
          .lock()
          .unwrap()
          .push_back((true, "discovery unavailable".into()));
      }
      assert_eq!(
        read_with_cache(URL, &mock, &cache).await.resolution(URL),
        Resolution::Unsupported { reason }
      );
    }
  }
}
