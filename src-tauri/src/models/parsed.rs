use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum ParsedMedia {
  Single(ParsedSingleVideo),
  Playlist(ParsedPlaylist),
  Livestream(ParsedLivestream),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
  File,
  Video,
  Audio,
  Image,
  Carousel,
  Mixed,
}

/// The engine that verified this row; absent on rows saved before winner routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FetchEngine {
  Gallery,
  Ytdlp,
  Files,
  Reader,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentKind {
  Image,
  Video,
}

/// Attachments stay in source order. IDs identify the source attachment, not its position.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAttachment {
  pub id: String,
  /// Original gallery-dl file position; absent for reader-resolved CDN assets.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub file_index: Option<usize>,
  pub kind: AttachmentKind,
  pub url: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub thumbnail: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub alt: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub width: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub height: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCodec {
  pub id: String,
  pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFormat {
  pub id: String,
  pub abr: Option<u64>,
  pub height: Option<u64>,
  pub fps: Option<u64>,
  pub audio_codecs: Vec<MediaCodec>,
  pub video_codecs: Vec<MediaCodec>,
  pub audio_track_ids: Vec<String>,
  pub video_track_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTrack {
  pub id: String,
  pub label: String,
  pub language: Option<String>,
  pub language_preference: Option<i64>,
  pub format_note: Option<String>,
  pub format: Option<String>,
  pub audio_channels: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleInventory {
  pub manual_languages: Vec<String>,
  pub auto_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
  pub title: String,
  pub start_time: f64,
  pub end_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSingleVideo {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fetch_engine: Option<FetchEngine>,
  /// Successful fetch target, kept separately from the original source page.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub resolved_url: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub media_kind: Option<MediaKind>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub attachments: Vec<MediaAttachment>,
  pub url: Option<String>,
  pub title: Option<String>,
  pub thumbnail: Option<String>,
  pub description: Option<String>,
  pub uploader_id: Option<String>,
  pub uploader: Option<String>,
  pub views: Option<u64>,
  pub comments: Option<u64>,
  pub likes: Option<u64>,
  pub dislikes: Option<u64>,
  pub duration: Option<f64>,
  pub rating: Option<f64>,
  pub extractor: Option<String>,
  pub video_codecs: Vec<MediaCodec>,
  pub audio_codecs: Vec<MediaCodec>,
  pub video_tracks: Vec<MediaTrack>,
  pub audio_tracks: Vec<MediaTrack>,
  pub formats: Vec<MediaFormat>,
  pub subtitle_inventory: SubtitleInventory,
  pub chapters: Vec<Chapter>,
  pub filesize: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPlaylist {
  pub id: String,
  pub url: Option<String>,
  pub title: Option<String>,
  pub thumbnail: Option<String>,
  pub uploader: Option<String>,
  pub uploader_id: Option<String>,
  pub entries: Vec<PlaylistEntry>,
  pub playlist_id: Option<String>,
  pub playlist_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
  pub video_url: String,
  pub index: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedLivestream {
  pub id: String,
  pub url: Option<String>,
  pub title: Option<String>,
  pub uploader: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::ParsedMedia;
  use serde_json::json;

  #[test]
  fn single_media_preserves_image_kinds_and_ordered_attachments() {
    for kind in ["image", "carousel", "mixed"] {
      let attachments = if kind == "image" {
        json!([{"id": "photo-2", "kind": "image", "url": "https://example.com/2.jpg", "width": 1200}])
      } else {
        json!([
          {"id": "photo-2", "kind": "image", "url": "https://example.com/2.jpg", "width": 1200},
          {"id": "item-1", "kind": if kind == "mixed" { "video" } else { "image" }, "url": "https://example.com/1"}
        ])
      };
      let input = json!({
        "type": "Single", "id": "post", "mediaKind": kind,
        "audioCodecs": [], "videoCodecs": [], "audioTracks": [], "videoTracks": [],
        "formats": [], "subtitleInventory": {"manualLanguages": [], "autoLanguages": []}, "chapters": [],
        "attachments": attachments
      });
      let parsed: ParsedMedia = serde_json::from_value(input).unwrap();
      let output = serde_json::to_value(parsed).unwrap();
      assert_eq!(output["mediaKind"], kind);
      assert_eq!(output["attachments"], attachments);
    }
  }
}
