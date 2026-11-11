use crate::models::parsed::{AttachmentKind, MediaAttachment, MediaKind, ParsedSingleVideo};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalleryError {
  Expired,
  Unsupported,
  Login,
  Refused(&'static str),
  Invalid,
  Unavailable,
  NetworkDown,
  Cancelled,
  Redirect(String),
}

pub(crate) fn expired_media(message: &str) -> bool {
  let lower = message.to_ascii_lowercase();
  [
    "http error 403",
    "403 forbidden",
    "403: forbidden",
    "http error 410",
    "410 gone",
    "410: gone",
    "expiredsignature",
    "signature has expired",
    "expired signature",
    "url has expired",
  ]
  .iter()
  .any(|hint| lower.contains(hint))
}

pub(crate) fn cdn_failure(message: &str) -> GalleryError {
  if expired_media(message) {
    GalleryError::Expired
  } else {
    failure(message)
  }
}

impl GalleryError {
  pub fn cookie_fetch_failure(
    &self,
    has_cookies: bool,
  ) -> crate::parsers::ytdlp_error::CookieFetchFailure {
    use crate::parsers::ytdlp_error::CookieFetchFailure;
    if !has_cookies {
      return CookieFetchFailure::Other;
    }
    match self {
      Self::Login => CookieFetchFailure::Refused,
      Self::Refused("rateLimited429" | "unableToExtractIdOrInfo") => CookieFetchFailure::Transient,
      _ => CookieFetchFailure::Other,
    }
  }
}

#[derive(Debug, Clone)]
pub struct GalleryAsset {
  pub item: MediaAttachment,
  pub index: usize,
  pub file_index: Option<usize>,
  pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct GalleryPost {
  pub source: String,
  pub extract_url: String,
  pub assets: Vec<GalleryAsset>,
  pub title: Option<String>,
  pub uploader: Option<String>,
}

pub fn gallery_first(source: &str) -> bool {
  crate::extract::registry::gallery_first(source)
}

/// Only TikTok post audio can be a photo slideshow; never divert playlists or other audio sites.
pub fn tiktok_photo_candidate(source: &str, info: &Value) -> bool {
  let Ok(url) = reqwest::Url::parse(source) else {
    return false;
  };
  if crate::sites::site_of(url.host_str().unwrap_or_default()) != Some(crate::sites::SiteId::TikTok)
    || !url.path().contains("/video/")
    || info.get("entries").is_some()
  {
    return false;
  }
  if info.get("imagePost").is_some_and(Value::is_object) {
    return true;
  }
  info["formats"].as_array().is_some_and(|formats| {
    !formats.is_empty() && formats.iter().all(|format| format["vcodec"] == "none")
  })
}

/// Total gallery work grows with the admitted selection, but never runs unbounded.
pub fn gallery_deadline(items: usize) -> std::time::Duration {
  std::time::Duration::from_secs(90 + (items.saturating_sub(1).min(117) as u64) * 30)
}

pub fn image_extension(ext: &str) -> bool {
  crate::media_types::ext_in(crate::media_types::IMAGE_EXTS, ext)
}

pub fn failure(message: &str) -> GalleryError {
  if super::ytdlp_error::network_down("", message) {
    return GalleryError::NetworkDown;
  }
  let lower = message.to_ascii_lowercase();
  if [
    "http error 500",
    "http error 502",
    "http error 503",
    "http error 504",
  ]
  .iter()
  .any(|status| lower.contains(status))
  {
    return GalleryError::Refused("server5xx");
  }
  if lower.contains("timed out") {
    return GalleryError::Refused("networkTimeout");
  }
  // Generic cookie advice appended to an empty/throttled response is not a refused session.
  if !lower.contains("authenticationerror") && !lower.contains("authorizationerror") {
    if lower.contains("empty media response") {
      return GalleryError::Refused("unableToExtractIdOrInfo");
    }
    if lower.contains("429 too many")
      || lower.contains("http error 429")
      || lower.contains("rate limit")
      || lower.contains("rate-limit")
    {
      return GalleryError::Refused("rateLimited429");
    }
  }

  if lower.contains("authenticationerror")
    || lower.contains("authorizationerror")
    || lower.contains("login required")
    || lower.contains("loginrequired")
    || lower.contains("cookies are required")
    || lower.contains("'auth_token' cookies")
    || lower.contains("redirect to login page")
  {
    GalleryError::Login
  } else if lower.contains("429 too many")
    || lower.contains("http error 429")
    || lower.contains("rate limit")
  {
    GalleryError::Refused("rateLimited429")
  } else if lower.contains("403 forbidden") || lower.contains("http error 403") {
    GalleryError::Refused("accessForbidden403")
  } else if lower.contains("404 not found")
    || lower.contains("http error 404")
    || lower.contains("notfounderror")
  {
    GalleryError::Refused("notFound404")
  } else if lower.contains("noextractorerror") || lower.contains("unsupported url") {
    GalleryError::Unsupported
  } else {
    GalleryError::Refused("imageSourceUnavailable")
  }
}

pub fn parse_output(
  code: i32,
  stdout: &[u8],
  stderr: &[u8],
  source: &str,
) -> Result<GalleryPost, GalleryError> {
  if code != 0 {
    return Err(failure(&String::from_utf8_lossy(stderr)));
  }
  match parse_gallery(stdout, source) {
    Err(GalleryError::Refused("imageSourceUnavailable")) => {
      Err(failure(&String::from_utf8_lossy(stderr)))
    }
    other => other,
  }
}

fn tiktok_image_identity(source: &str, image: &reqwest::Url) -> Option<String> {
  let page = reqwest::Url::parse(source).ok()?;
  let host = page.host_str()?;
  let cdn = image.host_str()?;
  if !(host == "tiktok.com" || host.ends_with(".tiktok.com"))
    || !(cdn == "tiktokcdn.com" || cdn.ends_with(".tiktokcdn.com"))
  {
    return None;
  }
  let parts: Vec<_> = page.path().trim_matches('/').split('/').collect();
  let [user, "photo" | "video", post_id] = parts.as_slice() else {
    return None;
  };
  if !user.starts_with('@')
    || post_id.is_empty()
    || post_id.len() > 32
    || !post_id.bytes().all(|byte| byte.is_ascii_digit())
  {
    return None;
  }
  // TikTok's imagePost offers p16/p19 mirrors of the same resource. Signatures,
  // CDN hosts and rendition templates may change; the original image hash does not.
  let filename = image.path().rsplit('/').next()?;
  let resource = filename.split(['~', '.']).next()?;
  if !(16..=64).contains(&resource.len()) || !resource.bytes().all(|byte| byte.is_ascii_hexdigit())
  {
    return None;
  }
  Some(format!("tiktok|{post_id}|{resource}"))
}

fn imgur_media_identity(source: &str, media: &reqwest::Url) -> Option<String> {
  let page = reqwest::Url::parse(source).ok()?;
  let host = page.host_str()?;
  if !(host == "imgur.com" || host.ends_with(".imgur.com")) || media.host_str()? != "i.imgur.com" {
    return None;
  }
  let path = media.path().strip_prefix('/')?;
  let (resource, _) = path.rsplit_once('.')?;
  if resource.is_empty()
    || !resource
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
  {
    return None;
  }
  Some(format!("{}|imgur|{resource}", source.split('?').next()?))
}

fn asset(
  record: &[Value],
  source: &str,
  index: usize,
  file_index: Option<usize>,
) -> Result<GalleryAsset, GalleryError> {
  let raw = record
    .get(1)
    .and_then(Value::as_str)
    .ok_or(GalleryError::Invalid)?;
  let meta = record
    .get(2)
    .filter(|v| v.is_object())
    .ok_or(GalleryError::Invalid)?;
  let delegated = raw.starts_with("ytdl:");
  let raw = raw.strip_prefix("ytdl:").unwrap_or(raw);
  let url = reqwest::Url::parse(raw).map_err(|_| GalleryError::Invalid)?;
  if !matches!(url.scheme(), "http" | "https")
    || !url.username().is_empty()
    || url.password().is_some()
  {
    return Err(GalleryError::Invalid);
  }
  let ext = meta["extension"]
    .as_str()
    .unwrap_or_default()
    .to_ascii_lowercase();
  let imgur_identity = imgur_media_identity(source, &url);
  let kind = if !delegated && image_extension(&ext) {
    AttachmentKind::Image
  } else if crate::media_types::ext_in(crate::media_types::GALLERY_VIDEO_EXTS, &ext)
    || (imgur_identity.is_some() && ext == "gifv")
  {
    AttachmentKind::Video
  } else {
    return Err(GalleryError::Invalid);
  };
  let mut stable_query: Vec<_> = url
    .query_pairs()
    .filter(|(key, _)| matches!(key.as_ref(), "v" | "id" | "video_id" | "did" | "cid"))
    .collect();
  stable_query.sort();
  let tiktok_identity = matches!(kind, AttachmentKind::Image)
    .then(|| tiktok_image_identity(source, &url))
    .flatten();
  let direct = delegated
    || tiktok_identity.is_some()
    || (imgur_identity.is_some() && matches!(kind, AttachmentKind::Video));
  let identity = tiktok_identity.or(imgur_identity).unwrap_or_else(|| {
    format!(
      "{}|{}|{}|{:?}",
      source.split('?').next().unwrap_or(source),
      url.host_str().unwrap_or_default(),
      url.path(),
      stable_query
    )
  });
  let file_index = if direct { None } else { file_index };
  Ok(GalleryAsset {
    index,
    file_index,
    metadata: meta.clone(),
    item: MediaAttachment {
      id: hex::encode(Sha256::digest(identity.as_bytes())),
      file_index,
      kind,
      url: raw.into(),
      thumbnail: None,
      alt: meta["alt"]
        .as_str()
        .or_else(|| meta["alt_text"].as_str())
        .map(str::to_owned),
      width: meta["width"].as_u64(),
      height: meta["height"].as_u64(),
    },
  })
}

// A range can shift between extraction and transfer. Bind the downloaded file's
// metadata receipt to the selected identity before anything reaches Downloads.
pub fn verify_receipt(
  bytes: &[u8],
  post: &GalleryPost,
  selected: &GalleryAsset,
) -> Result<(), GalleryError> {
  if bytes.len() > 1024 * 1024 {
    return Err(GalleryError::Invalid);
  }
  let meta: Value = serde_json::from_slice(bytes).map_err(|_| GalleryError::Invalid)?;
  let raw = meta["nezzel_url"].as_str().ok_or(GalleryError::Invalid)?;
  let actual = asset(
    &[Value::from(3), Value::from(raw), meta.clone()],
    &post.extract_url,
    selected.index,
    selected.file_index,
  )?;
  let expected = asset(
    &[
      Value::from(3),
      Value::from(selected.item.url.as_str()),
      selected.metadata.clone(),
    ],
    &post.extract_url,
    selected.index,
    selected.file_index,
  )?;
  if actual.item.id != expected.item.id || !matches!(actual.item.kind, AttachmentKind::Image) {
    return Err(GalleryError::Refused("imageSelectionChanged"));
  }
  Ok(())
}

pub fn parse_gallery(bytes: &[u8], source: &str) -> Result<GalleryPost, GalleryError> {
  if bytes.len() > 8 * 1024 * 1024 {
    return Err(GalleryError::Invalid);
  }
  let records: Vec<Value> = serde_json::from_slice(bytes).map_err(|_| GalleryError::Invalid)?;
  if records.len() > 1024 {
    return Err(GalleryError::Invalid);
  }
  let mut post = GalleryPost {
    source: source.into(),
    extract_url: source.into(),
    assets: Vec::new(),
    title: None,
    uploader: None,
  };
  let mut instagram_username = None;
  let instagram = reqwest::Url::parse(source).is_ok_and(|url| {
    url
      .host_str()
      .is_some_and(|host| host == "instagram.com" || host.ends_with(".instagram.com"))
  });
  let mut redirect = None;
  let mut ids = std::collections::HashSet::new();
  let mut file_index = 0;
  for record in records {
    let record = record.as_array().ok_or(GalleryError::Invalid)?;
    let record_type = record.first().and_then(Value::as_i64);
    if matches!(record_type, Some(2 | 3)) {
      let meta = record.last().ok_or(GalleryError::Invalid)?;
      if post.title.is_none() {
        post.title = ["title", "content", "description"]
          .iter()
          .filter_map(|key| meta[*key].as_str())
          .find_map(|s| s.lines().map(str::trim).find(|line| !line.is_empty()))
          .map(|s| s.chars().take(200).collect());
      }
      if post.uploader.is_none() {
        post.uploader = meta["author"]["name"]
          .as_str()
          .or_else(|| meta["user"]["username"].as_str())
          .or_else(|| meta["user"]["name"].as_str())
          .or_else(|| meta["author"].as_str())
          .or_else(|| meta["username"].as_str())
          .map(str::to_owned);
      }
      if instagram && instagram_username.is_none() {
        instagram_username = meta["username"]
          .as_str()
          .or_else(|| meta["user"]["username"].as_str())
          .map(|username| username.trim().trim_start_matches('@'))
          .filter(|username| !username.is_empty())
          .map(str::to_owned);
      }
    }
    match record_type {
      Some(-1) => return Err(failure(&record.get(1).unwrap_or(&Value::Null).to_string())),
      Some(2) => {}
      Some(3) => {
        if post.assets.len() >= 256 {
          return Err(GalleryError::Invalid);
        }
        file_index += 1;
        // gallery-dl emits the soundtrack/caption alongside the actual visual assets.
        let metadata = record.get(2).ok_or(GalleryError::Invalid)?;
        let raw = record
          .get(1)
          .and_then(Value::as_str)
          .ok_or(GalleryError::Invalid)?;
        if (metadata["category"] == "tiktok"
          && metadata["extension"] == "mp3"
          && metadata["num"] == 0)
          || (metadata["category"] == "pinterest"
            && metadata["extension"] == "txt"
            && raw.starts_with("text:"))
        {
          continue;
        }
        let mut asset = asset(record, source, post.assets.len() + 1, Some(file_index))?;
        if !ids.insert(asset.item.id.clone()) {
          let imgur_variant = reqwest::Url::parse(&asset.item.url)
            .ok()
            .and_then(|url| imgur_media_identity(source, &url))
            .is_some();
          if imgur_variant {
            if let Some(existing) = post
              .assets
              .iter_mut()
              .find(|existing| existing.item.id == asset.item.id)
            {
              if matches!(asset.item.kind, AttachmentKind::Video)
                && (!matches!(existing.item.kind, AttachmentKind::Video)
                  || asset.metadata["extension"] == "mp4")
              {
                asset.index = existing.index;
                *existing = asset;
              }
              continue;
            }
          }
          return Err(GalleryError::Invalid);
        }
        post.assets.push(asset);
      }
      Some(6) => {
        // Only follow same-site URL normalization, never an external link card or a ytdl queue.
        let raw = record
          .get(1)
          .and_then(Value::as_str)
          .ok_or(GalleryError::Invalid)?;
        if raw.starts_with("ytdl:") {
          let video = asset(record, source, post.assets.len() + 1, None)?;
          if post.assets.len() >= 256 || !ids.insert(video.item.id.clone()) {
            return Err(GalleryError::Invalid);
          }
          post.assets.push(video);
          continue;
        }
        let original = reqwest::Url::parse(source).map_err(|_| GalleryError::Invalid)?;
        if let Ok(target) = reqwest::Url::parse(raw) {
          let host = original
            .host_str()
            .unwrap_or_default()
            .trim_start_matches("www.");
          let next = target
            .host_str()
            .unwrap_or_default()
            .trim_start_matches("www.");
          if matches!(target.scheme(), "http" | "https")
            && raw != source
            && (host == next
              || (host == "pin.it" && next == "pinterest.com")
              || (host == "redd.it" && next == "reddit.com"))
          {
            redirect = Some(raw.to_string());
          }
        }
      }
      _ => return Err(GalleryError::Invalid),
    }
  }
  if post.assets.is_empty() {
    return Err(
      redirect
        .map(GalleryError::Redirect)
        .unwrap_or(GalleryError::Unsupported),
    );
  }
  if post.title.is_none() {
    post.title = instagram_username.map(|username| format!("Instagram post by @{username}"));
  }
  Ok(post)
}

impl GalleryPost {
  /// Resolver IDs stay authoritative; gallery-dl transfers only the admitted CDN URL.
  pub fn from_resolved(metadata: &str, source: &str) -> Result<Self, GalleryError> {
    if metadata.len() > 8 * 1024 * 1024 {
      return Err(GalleryError::Invalid);
    }
    let media: ParsedSingleVideo =
      serde_json::from_str(metadata).map_err(|_| GalleryError::Invalid)?;
    Self::from_attachments(media.attachments, source, media.title, media.uploader)
  }

  pub fn from_attachments(
    items: Vec<MediaAttachment>,
    source: &str,
    title: Option<String>,
    uploader: Option<String>,
  ) -> Result<Self, GalleryError> {
    if items.is_empty() || items.len() > 256 {
      return Err(GalleryError::Invalid);
    }
    let mut ids = std::collections::HashSet::new();
    let mut assets = Vec::new();
    for (position, item) in items.into_iter().enumerate() {
      let url = reqwest::Url::parse(&item.url).map_err(|_| GalleryError::Invalid)?;
      if item.id.is_empty()
        || item.id.len() > 512
        || !ids.insert(item.id.clone())
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || item
          .file_index
          .is_some_and(|index| index == 0 || index > 1024)
      {
        return Err(GalleryError::Invalid);
      }
      let extension = if matches!(item.kind, AttachmentKind::Image) {
        url
          .path()
          .rsplit('.')
          .next()
          .filter(|ext| image_extension(ext))
          // Resolver images can have extensionless CDN paths. This is a transfer hint only;
          // the receipt and downloaded bytes are still checked before choosing the saved type.
          .unwrap_or("jpg")
      } else {
        "mp4"
      };
      assets.push(GalleryAsset {
        index: position + 1,
        file_index: item.file_index,
        metadata: serde_json::json!({"extension":extension,"num":item.file_index.unwrap_or(position+1)}),
        item,
      });
    }
    Ok(Self {
      source: source.into(),
      extract_url: source.into(),
      title,
      uploader,
      assets,
    })
  }

  pub fn selected(&self, selection: Option<&[String]>) -> Result<Vec<&GalleryAsset>, GalleryError> {
    if let Some(ids) = selection {
      if ids.is_empty()
        || ids
          .iter()
          .any(|id| !self.assets.iter().any(|asset| &asset.item.id == id))
      {
        return Err(GalleryError::Refused("imageSelectionChanged"));
      }
      Ok(
        self
          .assets
          .iter()
          .filter(|asset| ids.contains(&asset.item.id))
          .collect(),
      )
    } else {
      Ok(self.assets.iter().collect())
    }
  }

  pub fn has_images(&self) -> bool {
    self
      .assets
      .iter()
      .any(|a| matches!(a.item.kind, AttachmentKind::Image))
  }

  pub fn into_media(self, id: String) -> ParsedSingleVideo {
    let videos = self
      .assets
      .iter()
      .any(|a| matches!(a.item.kind, AttachmentKind::Video));
    let kind = if videos && self.has_images() {
      MediaKind::Mixed
    } else if videos {
      MediaKind::Video
    } else if self.assets.len() > 1 {
      MediaKind::Carousel
    } else {
      MediaKind::Image
    };
    ParsedSingleVideo {
      id,
      fetch_engine: None,
      resolved_url: (self.extract_url != self.source).then_some(self.extract_url),
      url: Some(self.source),
      media_kind: Some(kind),
      thumbnail: self
        .assets
        .iter()
        .find(|a| matches!(a.item.kind, AttachmentKind::Image))
        .map(|a| a.item.url.clone()),
      attachments: self.assets.into_iter().map(|a| a.item).collect(),
      title: self.title,
      uploader: self.uploader,
      description: None,
      uploader_id: None,
      views: None,
      comments: None,
      likes: None,
      dislikes: None,
      duration: None,
      rating: None,
      extractor: None,
      video_codecs: vec![],
      audio_codecs: vec![],
      video_tracks: vec![],
      audio_tracks: vec![],
      formats: vec![],
      subtitle_inventory: Default::default(),
      chapters: vec![],
      filesize: None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tiktok_receipt_accepts_the_same_photo_on_another_provider_mirror() {
    let fixture: Value =
      serde_json::from_slice(include_bytes!("fixtures/gallery-tiktok-refresh.json")).unwrap();
    let post = parse_gallery(
      fixture["info"].to_string().as_bytes(),
      "https://www.tiktok.com/@obnbadminton/photo/7583998092605656340",
    )
    .unwrap();
    let mut receipt = fixture["download"][0][2].clone();
    receipt["nezzel_url"] = fixture["download"][0][1].clone();
    assert!(verify_receipt(receipt.to_string().as_bytes(), &post, &post.assets[0]).is_ok());
  }

  #[test]
  fn tiktok_receipt_rejects_a_different_photo_despite_a_matching_position() {
    let fixture: Value =
      serde_json::from_slice(include_bytes!("fixtures/gallery-tiktok-refresh.json")).unwrap();
    let post = parse_gallery(
      fixture["info"].to_string().as_bytes(),
      "https://www.tiktok.com/@obnbadminton/photo/7583998092605656340",
    )
    .unwrap();
    let mut receipt = fixture["download"][0][2].clone();
    receipt["nezzel_url"] = fixture["download"][1][1].clone();
    assert_eq!(
      verify_receipt(receipt.to_string().as_bytes(), &post, &post.assets[0]),
      Err(GalleryError::Refused("imageSelectionChanged"))
    );
  }

  #[test]
  fn tiktok_refreshed_photo_manifest_preserves_info_attachment_ids() {
    let fixture: Value =
      serde_json::from_slice(include_bytes!("fixtures/gallery-tiktok-refresh.json")).unwrap();
    let source = "https://www.tiktok.com/@obnbadminton/photo/7583998092605656340";
    let info = parse_gallery(fixture["info"].to_string().as_bytes(), source).unwrap();
    let ids = info
      .assets
      .iter()
      .map(|asset| asset.item.id.clone())
      .collect::<Vec<_>>();
    for source in [
      source,
      "https://www.tiktok.com/@obnbadminton/video/7583998092605656340",
    ] {
      let download = parse_gallery(fixture["download"].to_string().as_bytes(), source).unwrap();
      assert_eq!(
        download
          .selected(Some(&ids))
          .unwrap()
          .iter()
          .map(|asset| &asset.item.id)
          .collect::<Vec<_>>(),
        ids.iter().collect::<Vec<_>>()
      );
    }
  }

  #[test]
  fn tiktok_images_transfer_the_admitted_cdn_resource_without_reextracting_the_post() {
    let fixture: Value =
      serde_json::from_slice(include_bytes!("fixtures/gallery-tiktok-refresh.json")).unwrap();
    let post = parse_gallery(
      fixture["info"].to_string().as_bytes(),
      "https://www.tiktok.com/@obnbadminton/photo/7583998092605656340",
    )
    .unwrap();
    assert!(post.assets.iter().all(|asset| asset.file_index.is_none()));
  }

  #[test]
  fn imgur_album_exposes_each_media_once_preferring_mp4_to_its_gif_variant() {
    let post = parse_gallery(
      include_bytes!("fixtures/gallery-imgur-album.json"),
      "https://imgur.com/a/VemXvh0",
    )
    .unwrap();
    assert_eq!(
      (
        post
          .assets
          .iter()
          .filter(|asset| matches!(asset.item.kind, AttachmentKind::Image))
          .count(),
        post
          .assets
          .iter()
          .filter(|asset| matches!(asset.item.kind, AttachmentKind::Video))
          .count()
      ),
      (36, 18)
    );
    for resource in ["abcdefgh", "ab_cd-123"] {
      let records = serde_json::json!([[3,format!("https://i.imgur.com/{resource}.gifv"),{"extension":"gifv"}],[3,format!("https://i.imgur.com/{resource}.mp4"),{"extension":"mp4"}]]);
      let post =
        parse_gallery(records.to_string().as_bytes(), "https://imgur.com/a/album").unwrap();
      assert_eq!(
        (
          post.assets.len(),
          post.assets[0].metadata["extension"].as_str()
        ),
        (1, Some("mp4")),
        "{resource}"
      );
    }
  }

  #[test]
  fn instagram_captionless_posts_use_the_username_as_the_fallback_title() {
    for (description, expected) in [
      ("", "Instagram post by @creator"),
      ("  ", "Instagram post by @creator"),
      ("A caption\nSecond line", "A caption"),
    ] {
      let records = serde_json::json!([[2,{"category":"instagram","username":"creator","fullname":"Display Name","description":description}],[3,"https://cdn.example/photo.jpg",{"extension":"jpg","category":"instagram","username":"creator","description":description}]]);
      let media = parse_gallery(
        records.to_string().as_bytes(),
        "https://www.instagram.com/p/abc/",
      )
      .unwrap()
      .into_media("row".into());
      assert_eq!(media.title.as_deref(), Some(expected));
    }
    let records = serde_json::json!([[3,"https://cdn.example/photo.jpg",{"extension":"jpg","category":"instagram","username":"creator","description":""}]]);
    assert_eq!(
      parse_gallery(
        records.to_string().as_bytes(),
        "https://www.instagram.com/p/abc/"
      )
      .unwrap()
      .into_media("row".into())
      .title
      .as_deref(),
      Some("Instagram post by @creator")
    );
  }

  #[test]
  fn tiktok_audio_info_routes_to_photo_discovery_only_for_single_posts() {
    let info: Value =
      serde_json::from_slice(include_bytes!("fixtures/ytdlp-tiktok-photos.json")).unwrap();
    let cases = [
      (
        "https://www.tiktok.com/@obnbadminton/video/7583998092605656340",
        info.clone(),
        true,
      ),
      ("https://soundcloud.com/artist/track", info.clone(), false),
      (
        "https://tiktok.com.evil.test/@u/video/1",
        info.clone(),
        false,
      ),
      (
        "https://www.tiktok.com/@u/video/1",
        serde_json::json!({"entries":[info]}),
        false,
      ),
      (
        "https://www.tiktok.com/@u/video/1",
        serde_json::json!({"formats":[{"vcodec":"h264"},{"vcodec":"none"}]}),
        false,
      ),
      (
        "https://www.tiktok.com/@u/video/1",
        serde_json::json!({"formats":[{}]}),
        false,
      ),
      (
        "https://www.tiktok.com/@u/video/1",
        serde_json::json!({"imagePost":{"images":[{}]}}),
        true,
      ),
    ];
    for (source, info, want) in cases {
      assert_eq!(
        tiktok_photo_candidate(source, &info),
        want,
        "{source}: {info}"
      );
    }
  }

  #[test]
  fn ancillary_only_gallery_is_unsupported_instead_of_empty_success() {
    assert_eq!(
      parse_gallery(
        br#"[[3,"https://cdn.example/audio",{"category":"tiktok","extension":"mp3","num":0}]]"#,
        "https://www.tiktok.com/@u/video/1"
      )
      .unwrap_err(),
      GalleryError::Unsupported
    );
  }

  #[test]
  fn soundtrack_skip_preserves_the_following_image_transfer_range() {
    let post = parse_gallery(br#"[[3,"https://cdn.example/audio",{"category":"tiktok","extension":"mp3","num":0}],[3,"https://cdn.example/image.jpg",{"extension":"jpg"}]]"#, "https://www.tiktok.com/@u/photo/1").unwrap();
    assert_eq!(post.assets[0].file_index, Some(2));
  }

  #[test]
  fn gallery_work_budget_grows_for_large_albums_and_has_a_hard_cap() {
    for (items, seconds) in [
      (0, 90),
      (1, 90),
      (6, 240),
      (54, 1680),
      (256, 3600),
      (usize::MAX, 3600),
    ] {
      assert_eq!(gallery_deadline(items).as_secs(), seconds);
    }
  }

  #[test]
  fn snapchat_pages_use_ytdlp_instead_of_gallery() {
    for source in [
      "https://www.snapchat.com/spotlight/W7_abc",
      "https://www.snapchat.com/@nasa/spotlight/W7_abc",
      "https://www.snapchat.com/@nasa/highlight/04899dc0-8982-44f5-b8bf-d425d5dc3b84",
    ] {
      assert!(!gallery_first(source), "{source}");
    }
  }

  #[test]
  fn tiktok_photo_manifest_ignores_the_soundtrack() {
    let post = parse_gallery(
      include_bytes!("fixtures/gallery-tiktok-photos.json"),
      "https://www.tiktok.com/@obnbadminton/photo/7583998092605656340",
    )
    .unwrap();
    assert_eq!(
      post
        .assets
        .iter()
        .map(|a| matches!(a.item.kind, AttachmentKind::Image))
        .collect::<Vec<_>>(),
      vec![true; 6]
    );
  }

  #[test]
  fn pinterest_video_manifest_ignores_the_text_caption() {
    let post = parse_gallery(
      include_bytes!("fixtures/gallery-pinterest-video.json"),
      "https://www.pinterest.com/pin/1084663891475263837/",
    )
    .unwrap();
    assert_eq!(
      post
        .assets
        .iter()
        .map(|a| matches!(a.item.kind, AttachmentKind::Video))
        .collect::<Vec<_>>(),
      vec![true]
    );
  }

  #[test]
  fn preserves_order_and_stable_ids_across_expiring_url_queries() {
    let source = "https://x.com/user/status/123";
    let first = br#"[[2,{"content":"A post","author":{"name":"Author"}}],[3,"https://cdn.example/a.jpg?token=old",{"extension":"jpg","num":1}],[3,"https://cdn.example/b.mp4",{"extension":"mp4","num":2}]]"#;
    let post = parse_gallery(first, source).unwrap();
    assert_eq!(post.assets.len(), 2);
    assert_eq!(post.assets[1].index, 2);
    assert!(matches!(post.assets[0].item.kind, AttachmentKind::Image));
    assert!(matches!(post.assets[1].item.kind, AttachmentKind::Video));
    let refreshed = parse_gallery(
      &String::from_utf8(first.to_vec())
        .unwrap()
        .replace("old", "new")
        .into_bytes(),
      source,
    )
    .unwrap();
    assert_eq!(post.assets[0].item.id, refreshed.assets[0].item.id);
    let bluesky = parse_gallery(
      include_bytes!("fixtures/gallery-bluesky.json"),
      "https://bsky.app/profile/esa.int/post/3mykccxwzbc2y",
    )
    .unwrap();
    assert_eq!(bluesky.assets.len(), 3);
    assert_ne!(bluesky.assets[0].item.id, bluesky.assets[1].item.id);
  }

  #[test]
  fn embedded_errors_are_failures_even_when_the_process_exits_zero() {
    for (cookies, code) in [(false, "signInRequired"), (true, "accountAccessDenied")] {
      assert_eq!(
        GalleryError::Login.cookie_fetch_failure(cookies),
        if cookies {
          super::super::ytdlp_error::CookieFetchFailure::Refused
        } else {
          super::super::ytdlp_error::CookieFetchFailure::Other
        }
      );
      assert_eq!(
        crate::runners::gallery::error_diagnostic("group", "item", &GalleryError::Login, cookies)
          .code,
        code
      );
    }
    assert_eq!(
      crate::runners::gallery::error_diagnostic("group", "item", &GalleryError::NetworkDown, false)
        .code,
      "networkNameResolutionFailed"
    );

    assert_eq!(parse_output(0, br#"[[-1,{"error":"AbortExtraction","message":"HTTP redirect to login page (https://www.instagram.com/accounts/login/)"}]]"#, b"", "https://instagram.com/p/example").unwrap_err(), GalleryError::Login);
    for (message, code) in [
      ("HTTP Error 504: Gateway Timeout", "server5xx"),
      ("HTTP Error 504: connection timed out", "server5xx"),
      ("The read operation timed out", "networkTimeout"),
      ("Connection timed out", "networkTimeout"),
      ("timeout setting is invalid", "imageSourceUnavailable"),
    ] {
      assert_eq!(failure(message), GalleryError::Refused(code), "{message}");
    }
    assert_eq!(
      failure("post 429123 could not be read"),
      GalleryError::Refused("imageSourceUnavailable")
    );
    assert_eq!(
      parse_gallery(
        br#"[[-1,{"error":"AuthenticationError","message":"Login required"}]]"#,
        "https://x.com/user/status/1"
      )
      .unwrap_err(),
      GalleryError::Login
    );
    assert_eq!(
      parse_gallery(
        br#"[[-1,{"error":"HttpError","message":"429 Too Many Requests"}]]"#,
        "https://x.com/user/status/1"
      )
      .unwrap_err(),
      GalleryError::Refused("rateLimited429")
    );
    assert!(parse_gallery(
      br#"[[3,"file:///etc/passwd",{"extension":"jpg"}]]"#,
      "https://example.com/a"
    )
    .is_err());
  }

  #[test]
  fn selection_rejects_stale_or_empty_ids_and_preserves_source_order() {
    let post = parse_gallery(br#"[[3,"https://cdn.example/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example/b.jpg",{"extension":"jpg"}]]"#, "https://example.com/post").unwrap();
    let ids = vec![
      post.assets[1].item.id.clone(),
      post.assets[0].item.id.clone(),
      post.assets[1].item.id.clone(),
    ];
    assert_eq!(
      post
        .selected(Some(&ids))
        .unwrap()
        .iter()
        .map(|a| a.index)
        .collect::<Vec<_>>(),
      [1, 2]
    );
    assert!(post.selected(Some(&[])).is_err());
    assert!(post.selected(Some(&["stale".into()])).is_err());
    let selected = &post.assets[1];
    assert!(verify_receipt(
      br#"{"nezzel_url":"https://cdn.example/b.jpg?token=new","extension":"jpg"}"#,
      &post,
      selected
    )
    .is_ok());
    assert_eq!(
      verify_receipt(
        br#"{"nezzel_url":"https://cdn.example/a.jpg","extension":"jpg"}"#,
        &post,
        selected
      ),
      Err(GalleryError::Refused("imageSelectionChanged"))
    );
  }

  #[test]
  fn resolver_attachment_ids_remain_authoritative_for_selected_transfer_receipts() {
    let source = "https://www.threads.com/@user/post/abc";
    let mut post = parse_gallery(br#"[[3,"https://cdn.example/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example/v.mp4",{"extension":"mp4"}],[3,"https://cdn.example/b.jpg?token=old",{"extension":"jpg"}]]"#, source).unwrap();
    for (index, asset) in post.assets.iter_mut().enumerate() {
      asset.item.id = format!("threads-slide-{index}");
      // Reader manifests carry CDN assets, without gallery-dl file positions.
      asset.item.file_index = None;
      asset.file_index = None;
    }
    let metadata = serde_json::to_string(&post.into_media("post".into())).unwrap();
    let post = GalleryPost::from_resolved(&metadata, source).unwrap();
    let selected = post.selected(Some(&["threads-slide-2".into()])).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].index, 3);
    assert_eq!(selected[0].file_index, None);
    assert!(verify_receipt(
      br#"{"nezzel_url":"https://cdn.example/b.jpg?token=new","extension":"jpg"}"#,
      &post,
      &post.assets[2]
    )
    .is_ok());
  }

  #[test]
  fn real_x_photo_and_mixed_records_keep_source_positions_and_ignore_link_cards() {
    let source = "https://x.com/karpathy/status/2116957135661945283/photo/1";
    let photo = parse_gallery(include_bytes!("fixtures/gallery-x-repro.json"), source).unwrap();
    assert_eq!(photo.assets.len(), 1);
    assert_eq!(photo.assets[0].item.width, Some(2482));
    assert!(matches!(
      photo.into_media("photo".into()).media_kind,
      Some(MediaKind::Image)
    ));
    let mixed = parse_gallery(
      include_bytes!("fixtures/gallery-x-mixed.json"),
      "https://x.com/AeroGazette/status/1757319514281140602",
    )
    .unwrap();
    assert_eq!(
      mixed.assets.iter().map(|a| a.index).collect::<Vec<_>>(),
      [1, 2, 3]
    );
    let cached = serde_json::to_string(&mixed.clone().into_media("mixed".into())).unwrap();
    let cached = GalleryPost::from_resolved(&cached, &mixed.source).unwrap();
    assert_eq!(
      cached
        .assets
        .iter()
        .map(|a| a.file_index)
        .collect::<Vec<_>>(),
      mixed
        .assets
        .iter()
        .map(|a| a.file_index)
        .collect::<Vec<_>>()
    );
    assert!(matches!(
      mixed.into_media("mixed".into()).media_kind,
      Some(MediaKind::Mixed)
    ));
    let queued = parse_gallery(br#"[[6,"ytdl:https://video.example/watch?id=1",{"extension":"mp4"}],[3,"https://cdn.example/photo.jpg",{"extension":"jpg"}],[6,"https://hackertube.org/link-card",{}]]"#, source).unwrap();
    assert_eq!(queued.assets.len(), 2);
    assert_eq!(queued.assets[1].index, 2);
    assert_eq!(queued.assets[1].file_index, Some(1));
    assert_eq!(queued.assets[0].file_index, None);
    assert!(gallery_first(source));
    assert!(!gallery_first("https://x.com/user/status/1/video/1"));
    assert!(!gallery_first("https://youtube.com/watch?v=1"));
    assert!(!gallery_first("https://x.com/user"));
    assert!(gallery_first("https://example.com/post/1"));
  }

  #[test]
  fn expired_cdn_failures_are_distinct_from_login_and_unrelated_errors() {
    for message in [
      "HTTPError: '403 Forbidden'",
      "HTTP Error 410: Gone",
      "Signature has expired",
    ] {
      assert_eq!(cdn_failure(message), GalleryError::Expired, "{message}");
    }
    for message in [
      "AuthenticationError: Login required",
      "429 Too Many Requests",
      "download https://cdn.example/403/image.jpg",
    ] {
      assert!(!expired_media(message), "{message}");
    }
  }
}

#[cfg(test)]
mod reddit_routing_tests {
  use super::gallery_first;
  #[test]
  fn reddit_posts_reach_the_public_reader_before_gallery_dl() {
    for source in [
      "https://www.reddit.com/comments/abc123/",
      "https://old.reddit.com/comments/abc123/",
      "https://new.reddit.com/comments/abc123/",
      "https://sh.reddit.com/comments/abc123/",
      "https://redd.it/abc123",
      "https://v.redd.it/abcdefgh1234",
    ] {
      assert!(!gallery_first(source), "{source}");
    }
  }
}
