use crate::models::{ParsedMedia, YtdlpInfo};
use crate::parsers::ytdlp_livestream::parse_livestream;
use crate::parsers::ytdlp_playlist::parse_playlist;
use crate::parsers::ytdlp_single::parse_single;

pub enum InfoType {
  Single,
  Playlist,
  Livestream,
}

pub fn parse_ytdlp_info(json: &str, id: String) -> Result<ParsedMedia, String> {
  parse_ytdlp_info_for_url(json, id, None)
}

/// The original post URL remains authoritative when an extractor hands off to external media.
pub fn parse_ytdlp_info_for_url(
  json: &str,
  id: String,
  source: Option<&str>,
) -> Result<ParsedMedia, String> {
  let mut value: serde_json::Value =
    serde_json::from_str(json).map_err(|e| format!("JSON parse error: {e}"))?;
  let instagram = source
    .or_else(|| value["webpage_url"].as_str())
    .or_else(|| value["original_url"].as_str())
    .and_then(|url| reqwest::Url::parse(url).ok())
    .is_some_and(|url| {
      url
        .host_str()
        .is_some_and(|host| host == "instagram.com" || host.ends_with(".instagram.com"))
        && (url.path().starts_with("/p/") || url.path().starts_with("/reel/"))
    });
  if instagram
    && value["description"]
      .as_str()
      .is_none_or(|caption| caption.trim().is_empty())
  {
    if let Some(username) = value["channel"]
      .as_str()
      .map(|name| name.trim().trim_start_matches('@'))
      .filter(|name| !name.is_empty())
    {
      if value["title"].as_str().is_none_or(|title| {
        title.trim().is_empty()
          || title == format!("Video by {username}")
          || title == format!("Post by {username}")
      }) {
        value["title"] = serde_json::json!(format!("Instagram post by @{username}"));
      }
    }
  }
  let info: YtdlpInfo =
    serde_json::from_value(value.clone()).map_err(|e| format!("JSON parse error: {e}"))?;
  let source = source
    .or_else(|| value["webpage_url"].as_str())
    .or_else(|| value["original_url"].as_str());
  if !info.is_live.unwrap_or(false) && value["entries"].as_array().is_some_and(|e| !e.is_empty()) {
    if let Some(source) = source.filter(|s| super::post::single_post_url(s)) {
      let ParsedMedia::Single(parent) = parse_single(
        serde_json::from_value(value.clone()).map_err(|e| format!("JSON parse error: {e}"))?,
        id.clone(),
      ) else {
        return Err("Expected a post".into());
      };
      if let Some(post) = super::post::from_entries(&value, source, parent) {
        return Ok(post);
      }
    }
  }
  Ok(match detect_info_type(&info) {
    InfoType::Livestream => parse_livestream(info, id),
    InfoType::Playlist => parse_playlist(info, id),
    InfoType::Single => {
      let mut media = parse_single(info, id);
      if let ParsedMedia::Single(single) = &mut media {
        single.media_kind = super::post::media_kind(&value);
      }
      media
    }
  })
}

pub fn detect_info_type(info: &YtdlpInfo) -> InfoType {
  if info.is_live.unwrap_or(false) {
    InfoType::Livestream
  } else if info.type_.as_deref() == Some("playlist")
    || info.entries.as_ref().is_some_and(|e| !e.is_empty())
  {
    InfoType::Playlist
  } else {
    InfoType::Single
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::{json, Value};

  fn playlist(source: &str) -> Value {
    json!({"_type":"playlist", "webpage_url":source, "title":"Generating 3D with AI is easy…", "entries":[
      {"id":"clip-a", "url":"https://storage.example/video-generator-usage.mp4", "ext":"mp4", "title":"video-generator-usage", "thumbnail":"https://storage.example/a.jpg"},
      {"id":"clip-b", "url":"https://storage.example/Rodin.webm", "ext":"webm", "title":"Rodin", "thumbnail":"https://storage.example/b.jpg"}
    ]})
  }

  #[test]
  fn instagram_ytdlp_fallback_titles_use_the_handle_without_replacing_captions() {
    for (title, description, expected) in [
      (None, None, "Instagram post by @creator"),
      (Some("Video by creator"), None, "Instagram post by @creator"),
      (
        Some("Post by creator"),
        Some(""),
        "Instagram post by @creator",
      ),
      (Some("A real title"), None, "A real title"),
      (
        Some("Video by creator"),
        Some("A caption"),
        "Video by creator",
      ),
    ] {
      let info = json!({"webpage_url":"https://www.instagram.com/p/abc/", "channel":"creator", "uploader":"Display Name", "title":title, "description":description});
      let ParsedMedia::Single(media) = parse_ytdlp_info(&info.to_string(), "row".into()).unwrap()
      else {
        panic!("single post expected");
      };
      assert_eq!(media.title.as_deref(), Some(expected));
    }
  }

  #[test]
  fn single_post_entries_become_one_selectable_post() {
    for source in [
      "https://www.reddit.com/comments/abc123/",
      "https://www.reddit.com/r/test/s/Share123",
      "https://x.com/user/status/123",
      "https://www.threads.com/@user/post/abc",
      "https://www.facebook.com/user/posts/123",
      "https://user.tumblr.com/post/123/title",
      "https://bsky.app/profile/user.test/post/abc",
    ] {
      let media = parse_ytdlp_info(&playlist(source).to_string(), "row".into()).unwrap();
      let ParsedMedia::Single(post) = media else {
        panic!("post expected: {source}");
      };
      assert_eq!(post.attachments.len(), 2);
    }
  }

  #[test]
  fn collection_urls_keep_playlist_range_semantics() {
    for source in [
      "https://www.youtube.com/playlist?list=PL123",
      "https://www.reddit.com/r/test/",
      "https://x.com/user",
      "https://user.tumblr.com/",
      "https://bsky.app/profile/user.test",
    ] {
      assert!(
        matches!(
          parse_ytdlp_info(&playlist(source).to_string(), "row".into()).unwrap(),
          ParsedMedia::Playlist(_)
        ),
        "{source}"
      );
    }
  }

  #[test]
  fn post_entries_keep_parent_title_and_their_own_thumbnails() {
    let ParsedMedia::Single(post) = parse_ytdlp_info(
      &playlist("https://www.reddit.com/comments/abc123/").to_string(),
      "row".into(),
    )
    .unwrap() else {
      panic!("post expected");
    };
    assert_eq!(
      post
        .attachments
        .iter()
        .map(|a| (a.alt.as_deref(), a.thumbnail.as_deref()))
        .collect::<Vec<_>>(),
      vec![
        (
          Some("Generating 3D with AI is easy… (1/2)"),
          Some("https://storage.example/a.jpg")
        ),
        (
          Some("Generating 3D with AI is easy… (2/2)"),
          Some("https://storage.example/b.jpg")
        )
      ]
    );
  }

  #[test]
  fn ytdlp_post_entries_drop_non_https_thumbnails() {
    let source = "https://www.reddit.com/comments/abc123/";
    let mut info = playlist(source);
    info["thumbnail"] = json!("data:image/png;base64,fixture");
    info["entries"][0]["thumbnail"] = json!("http://cdn.example/cover.jpg");
    let ParsedMedia::Single(post) = parse_ytdlp_info(&info.to_string(), "row".into()).unwrap()
    else {
      panic!("post expected");
    };
    assert_eq!(
      (post.thumbnail, post.attachments[0].thumbnail.clone()),
      (None, None)
    );
  }

  #[test]
  fn direct_video_containers_without_codecs_are_video_but_explicit_audio_stays_audio() {
    for (ext, codec, expected) in [
      ("mp4", None, "video"),
      ("webm", None, "video"),
      ("mov", Some("unknown"), "video"),
      ("mp4", Some("none"), "audio"),
      ("m4a", None, "audio"),
      ("mp3", None, "audio"),
      ("opus", None, "audio"),
    ] {
      let value =
        json!({"url":format!("https://cdn.example/clip.{ext}"), "ext":ext, "vcodec":codec});
      let media = parse_ytdlp_info(&value.to_string(), "row".into()).unwrap();
      assert_eq!(
        serde_json::to_value(media).unwrap()["mediaKind"],
        expected,
        "{ext}/{codec:?}"
      );
    }
  }
}
