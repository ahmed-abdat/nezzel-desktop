use super::{http, Resolution, Verdict};
use crate::models::parsed::{
  AttachmentKind, MediaAttachment, MediaKind, ParsedSingleVideo, SubtitleInventory,
};
use std::collections::BTreeSet;

pub(super) fn line(text: &str) -> Option<String> {
  let line = text.lines().next()?.trim();
  (!line.is_empty()).then(|| line.chars().take(512).collect())
}
pub(super) async fn admit_attachments<T: http::Transport>(
  transport: &T,
  items: &[MediaAttachment],
) -> bool {
  if items.is_empty() {
    return false;
  }
  let mut checked = BTreeSet::new();
  for item in items {
    for raw in std::iter::once(&item.url).chain(item.thumbnail.iter()) {
      let Ok(url) = http::https_url(raw) else {
        return false;
      };
      if checked.insert(url.host_str().map(str::to_owned))
        && transport.admit_media(&url).await.is_err()
      {
        return false;
      }
    }
  }
  true
}

pub(super) async fn attachments<T: http::Transport>(
  transport: &T,
  source: &str,
  id: &str,
  extractor: &str,
  title: Option<String>,
  author: Option<String>,
  items: Vec<MediaAttachment>,
) -> Verdict {
  if !admit_attachments(transport, &items).await {
    return Verdict::Unknown;
  }
  let images = items
    .iter()
    .any(|a| matches!(a.kind, AttachmentKind::Image));
  let videos = items
    .iter()
    .any(|a| matches!(a.kind, AttachmentKind::Video));
  let kind = if images && videos {
    MediaKind::Mixed
  } else if items.len() > 1 {
    MediaKind::Carousel
  } else if images {
    MediaKind::Image
  } else {
    MediaKind::Video
  };
  let media = ParsedSingleVideo {
    id: id.into(),
    fetch_engine: None,
    resolved_url: None,
    media_kind: Some(kind),
    thumbnail: items.first().and_then(|a| a.thumbnail.clone()),
    attachments: items,
    url: Some(source.into()),
    title,
    uploader: author,
    extractor: Some(extractor.into()),
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
  };
  serde_json::to_string(&media).map_or(Verdict::Unknown, |metadata| {
    Verdict::Verified(Resolution::Attachments { metadata })
  })
}
pub(super) fn image(url: String, title: Option<String>) -> MediaAttachment {
  // Signed parameters change, but the source attachment path remains its identity.
  let mut stable = reqwest::Url::parse(&url).expect("admitted media URL");
  stable.set_query(None);
  MediaAttachment {
    id: stable.into(),
    file_index: None,
    kind: AttachmentKind::Image,
    thumbnail: Some(url.clone()),
    url,
    alt: title,
    width: None,
    height: None,
  }
}
