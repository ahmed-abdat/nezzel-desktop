//! Which engine handles a download: gallery-dl, yt-dlp or a re-probe.
use crate::models::parsed::{FetchEngine, MediaKind};
use crate::parsers::gallery::{gallery_first, GalleryError};
use crate::scheduling::download_pipeline::DownloadEntry;

pub(super) fn gallery_download(entry: &DownloadEntry) -> bool {
  entry.attachment_ids.is_some()
    || matches!(
      entry.media_kind,
      Some(MediaKind::Image | MediaKind::Carousel | MediaKind::Mixed)
    )
    // A login-walled anonymous preview leaves the kind unknown: probe with gallery-dl first.
    // Known videos and selected video children keep yt-dlp and must not re-probe the parent.
    || (entry.media_kind.is_none() && entry.resume_parent.is_none() && gallery_first(&entry.url))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DownloadEngine {
  Gallery,
  Files,
  Ytdlp,
}

pub(super) fn download_engine(entry: &DownloadEntry) -> DownloadEngine {
  match entry.fetch_engine {
    Some(FetchEngine::Gallery) => DownloadEngine::Gallery,
    Some(FetchEngine::Files) => DownloadEngine::Files,
    Some(FetchEngine::Ytdlp) => DownloadEngine::Ytdlp,
    Some(FetchEngine::Reader) | None => {
      if matches!(entry.media_kind, Some(MediaKind::File)) {
        DownloadEngine::Files
      } else if gallery_download(entry) {
        DownloadEngine::Gallery
      } else {
        DownloadEngine::Ytdlp
      }
    }
  }
}

pub(super) fn probe_with_gallery(entry: &DownloadEntry) -> bool {
  entry.fetch_engine.is_none()
    || (entry.attachments.is_empty()
      && matches!(
        entry.fetch_engine,
        Some(FetchEngine::Gallery | FetchEngine::Reader)
      ))
}

pub(super) fn retry_gallery_login(error: &GalleryError) -> bool {
  !matches!(error, GalleryError::Unsupported | GalleryError::NetworkDown)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::extract::{
    chain::StageResult,
    outcome::{Confidence, Outcome, StageId},
  };
  use crate::models::ParsedMedia;
  use crate::parsers::gallery::parse_output;
  use crate::runners::gallery::info::{image_post, preview_from_post};
  use crate::scheduling::fetch_pipeline::FetchEntry;
  use serde_json::json;
  use std::time::Instant;

  #[test]
  fn unsupported_gallery_errors_never_enter_the_login_retry_loop() {
    for (error, retry) in [
      (GalleryError::Unsupported, false),
      (GalleryError::NetworkDown, false),
      (GalleryError::Login, true),
      (GalleryError::Refused("rateLimited429"), true),
    ] {
      assert_eq!(retry_gallery_login(&error), retry, "{error:?}");
    }
  }

  #[test]
  fn imgur_albums_are_a_high_gallery_claim_but_single_pages_are_not() {
    for (url, expected) in [
      ("https://imgur.com/a/VemXvh0", Confidence::High),
      ("https://imgur.com/gallery/abc", Confidence::High),
      ("https://imgur.com/abc123", Confidence::Low),
    ] {
      assert_eq!(
        crate::extract::registry::claim(StageId::Gallery, url),
        expected,
        "{url}"
      );
    }
    assert_eq!(
      crate::extract::registry::stages_for("https://imgur.com/a/VemXvh0")[..3],
      [StageId::Files, StageId::Gallery, StageId::Ytdlp]
    );
  }

  #[test]
  fn routing_keeps_preview_login_failures_for_the_full_gallery_fetch() {
    let source = "https://www.instagram.com/p/DeAYvsnn4la/";
    for (records, expected_kind) in [
      (
        json!([[3,"https://cdn.example/a.jpg",{"extension":"jpg"}]]),
        Some("image"),
      ),
      (
        json!([[3,"https://cdn.example/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example/b.jpg",{"extension":"jpg"}]]),
        Some("carousel"),
      ),
      (
        json!([[3,"https://cdn.example/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example/v.mp4",{"extension":"mp4"}]]),
        Some("mixed"),
      ),
      (
        json!([[3,"https://cdn.example/v.mp4",{"extension":"mp4"}]]),
        None,
      ),
      (json!([]), None),
    ] {
      let post = parse_output(0, records.to_string().as_bytes(), b"", source);
      let preview = preview_from_post(post, "capture".into()).unwrap();
      assert_eq!(
        preview.as_ref().and_then(|item| item["mediaKind"].as_str()),
        expected_kind
      );
    }
    // No extractor is a video handoff, not a failed attempt that spends login retries.
    assert!(image_post(Err(GalleryError::Unsupported))
      .unwrap()
      .is_none());
    let login = parse_output(
      0,
      br#"[[-1,{"error":"AbortExtraction","message":"HTTP redirect to login page"}]]"#,
      b"",
      source,
    );
    assert_eq!(
      preview_from_post(login, "capture".into()),
      Err(GalleryError::Login)
    );
  }

  #[test]
  fn routing_gallery_fetch_is_independent_of_batch_size_and_selected_format() {
    let mut entry = FetchEntry {
      group_id: "group".into(),
      id: "photo".into(),
      url: "https://www.instagram.com/p/DeAYvsnn4la/".into(),
      total: 1,
      format: None,
      overrides: None,
      login_browser: None,
      login_was_accepted: false,
      prior_attempts: Vec::new(),
      prior_resolved_url: None,
    };
    for (url, total, format, expected) in [
      ("https://www.instagram.com/p/DeAYvsnn4la/", 1, false, true),
      ("https://www.instagram.com/p/DeAYvsnn4la/", 3, false, true),
      ("https://www.instagram.com/p/DeAYvsnn4la/", 1, true, true),
      ("https://x.com/user/status/123", 2, false, true),
      ("https://www.instagram.com/reel/123/", 1, false, false),
      ("https://www.instagram.com/", 1, false, false),
    ] {
      entry.url = url.into();
      entry.total = total;
      entry.format = format.then(|| serde_json::from_value(json!({"trackType":"both"})).unwrap());
      assert_eq!(
        gallery_first(&entry.url),
        expected,
        "{url}, total={total}, format={format}"
      );
    }
  }

  #[test]
  fn routing_gallery_download_reprobes_unknown_posts_and_browser_retries() {
    for (url, kind, expected) in [
      ("https://www.instagram.com/p/DeAYvsnn4la/", None, true),
      (
        "https://www.instagram.com/p/DeAYvsnn4la/",
        Some("video"),
        false,
      ),
      ("https://x.com/user/status/123", Some("video"), false),
      ("https://x.com/user/status/123", None, true),
      ("https://www.instagram.com/reel/123/", Some("video"), false),
      ("https://www.instagram.com/", None, false),
      ("https://www.youtube.com/watch?v=123", Some("video"), false),
      ("https://cdn.example/post", Some("image"), true),
    ] {
      let item = serde_json::from_value(json!({
        "id":"post", "url":url, "format":{"trackType":"both"},
        "mediaKind":kind, "templateContext":{"values":{}}
      }))
      .unwrap();
      let mut entry = DownloadEntry::from((item, "group".into()));
      assert_eq!(gallery_download(&entry), expected, "{url}, kind={kind:?}");
      assert_eq!(
        download_engine(&entry),
        if expected {
          DownloadEngine::Gallery
        } else {
          DownloadEngine::Ytdlp
        }
      );
      entry.login_browser = Some("chrome".into());
      assert_eq!(gallery_download(&entry), expected, "browser retry: {url}");
    }
  }

  #[test]
  fn verified_fetch_engine_overrides_gallery_heuristics() {
    for (engine, kind, expected) in [
      ("gallery", Some("audio"), DownloadEngine::Gallery),
      ("gallery", Some("video"), DownloadEngine::Gallery),
      ("gallery", Some("file"), DownloadEngine::Gallery),
      ("ytdlp", None, DownloadEngine::Ytdlp),
      ("ytdlp", Some("carousel"), DownloadEngine::Ytdlp),
      ("ytdlp", Some("file"), DownloadEngine::Ytdlp),
      ("files", Some("image"), DownloadEngine::Files),
      ("files", None, DownloadEngine::Files),
      ("reader", Some("image"), DownloadEngine::Gallery),
      ("reader", Some("video"), DownloadEngine::Ytdlp),
      ("reader", Some("file"), DownloadEngine::Files),
    ] {
      let item = serde_json::from_value(json!({
        "id":"post", "url":"https://www.instagram.com/p/_FVse5H9Pe/",
        "format":{"trackType":"both"}, "fetchEngine":engine, "mediaKind":kind,
        "templateContext":{"values":{}}
      }))
      .unwrap();
      let entry = DownloadEntry::from((item, "winner-routing".into()));
      assert_eq!(download_engine(&entry), expected, "{engine}, {kind:?}");
      assert_eq!(
        probe_with_gallery(&entry),
        matches!(engine, "gallery" | "reader"),
        "empty manifest: {engine}"
      );
    }
  }

  #[test]
  fn tiktok_video_path_photos_use_the_verified_gallery_without_a_selection() {
    let source = "https://www.tiktok.com/@x/video/7703997545197915412";
    let post = crate::parsers::gallery::parse_gallery(
      include_bytes!("../../parsers/fixtures/gallery-tiktok-photos.json"),
      source,
    )
    .unwrap();
    let mut media = ParsedMedia::Single(post.into_media("photos".into()));
    let attempts = StageResult::<()>::new(StageId::Gallery, Confidence::Medium)
      .finish(Outcome::Verified, false, Instant::now())
      .attempts;
    crate::extract::chain::record_fetch_engine(&mut media, &attempts);
    let mut payload = serde_json::to_value(media).unwrap();
    payload["format"] = json!({"trackType":"both"});
    payload["templateContext"] = json!({"values":{}});
    let entry = DownloadEntry::from((serde_json::from_value(payload).unwrap(), "photos".into()));
    assert!(entry.attachment_ids.is_none());
    assert_eq!(download_engine(&entry), DownloadEngine::Gallery);
    assert!(
      !probe_with_gallery(&entry),
      "the gallery winner skips the download-time probe"
    );
    assert!(matches!(entry.media_kind, Some(MediaKind::Carousel)));
  }
}
