use crate::models::{ParsedMedia, ParsedPlaylist, PlaylistEntry, YtdlpEntry, YtdlpInfo};
use crate::parsers::ytdlp_single::i64_to_u64;

pub fn parse_playlist(info: YtdlpInfo, id: String) -> ParsedMedia {
  let entries = playlist_entries(info.entries.as_deref().unwrap_or_default());

  let thumbnail = info.thumbnails.as_ref().and_then(|thumbs| {
    thumbs
      .iter()
      .max_by_key(|thumb| {
        let w = thumb.width.unwrap_or(0);
        let h = thumb.height.unwrap_or(0);
        w * h
      })
      .and_then(|thumb| thumb.url.clone())
  });

  ParsedMedia::Playlist(ParsedPlaylist {
    id,
    url: info.webpage_url,
    title: info.title,
    thumbnail,
    uploader: info.uploader,
    uploader_id: info.uploader_id,
    playlist_id: info.id,
    playlist_count: i64_to_u64(info.playlist_count),
    entries,
  })
}

fn playlist_entries(items: &[YtdlpEntry]) -> Vec<PlaylistEntry> {
  let mut entries = Vec::new();

  for (idx, entry) in items.iter().enumerate() {
    let url = if let Some(u) = &entry.url {
      u.clone()
    } else {
      entry.webpage_url.clone().unwrap_or_default()
    };

    if !url.is_empty() {
      entries.push(PlaylistEntry {
        filesize: i64_to_u64(entry.filesize),
        video_url: url,
        index: idx,
        title: entry.title.clone(),
        duration: entry
          .duration
          .filter(|value| value.is_finite() && *value >= 0.0),
        thumbnail: entry.thumbnail.clone().or_else(|| {
          entry
            .thumbnails
            .as_ref()?
            .iter()
            .rev()
            .find_map(|thumb| thumb.url.clone())
        }),
      });
    }
  }
  entries
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::ytdlp::YtdlpThumbnail;

  #[test]
  fn flat_entries_keep_metadata_and_original_positions() {
    let entry = |url: Option<&str>, title: Option<&str>, duration| YtdlpEntry {
      filesize: Some(1_000_000),
      url: url.map(str::to_owned),
      webpage_url: None,
      title: title.map(str::to_owned),
      duration,
      thumbnail: None,
      thumbnails: None,
    };
    let mut preview = entry(
      Some("https://example.com/first"),
      Some("First video"),
      Some(65.5),
    );
    preview.thumbnails = Some(vec![YtdlpThumbnail {
      url: Some("https://example.com/thumb.jpg".into()),
      width: None,
      height: None,
    }]);
    let entries = playlist_entries(&[
      preview,
      entry(None, None, None),
      entry(Some("https://example.com/third"), None, Some(f64::NAN)),
    ]);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].filesize, Some(1_000_000));
    assert_eq!(entries[0].title.as_deref(), Some("First video"));
    assert_eq!(entries[0].duration, Some(65.5));
    assert_eq!(
      entries[0].thumbnail.as_deref(),
      Some("https://example.com/thumb.jpg")
    );
    assert_eq!(entries[1].index, 2);
    assert!(entries[1].title.is_none());
    assert!(entries[1].duration.is_none());
    assert!(entries[1].thumbnail.is_none());
  }
}
