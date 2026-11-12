use super::*;

#[test]
fn preferring_video_preserves_intervening_stage_order() {
  let mut stages = vec![
    StageId::Files,
    StageId::Gallery,
    StageId::PageScan,
    StageId::Ytdlp,
  ];
  prefer_ytdlp(&mut stages);
  assert_eq!(
    stages,
    [
      StageId::Files,
      StageId::Ytdlp,
      StageId::Gallery,
      StageId::PageScan
    ]
  );
}

struct Case {
  name: &'static str,
  yes: &'static [&'static str],
  no: &'static [&'static str],
}

// The literal cases catch broadened identity/host/path gates, independently of routing order.
#[test]
fn every_matcher_has_offline_positive_and_negative_cases() {
  let cases = [
    Case {
      name: "file-extension",
      yes: &[
        "https://vimeo.com/file.mp4",
        "https://cdn.example/image.JPG",
        "https://files.example/file%2Ezip",
      ],
      no: &[
        "https://files.example/post",
        "https://files.example/file.mp4.exe/more",
      ],
    },
    Case {
      name: "file-share",
      yes: &["https://drive.google.com/file/d/abc123/view"],
      no: &[
        "https://drive.google.com.evil.test/file/d/abc123/view",
        "https://drive.google.com/",
      ],
    },
    Case {
      name: "archive-file",
      yes: &["https://archive.org/details/item"],
      no: &[
        "https://archive.org/download/item/file.txt",
        "https://archive.org.evil.test/details/item",
      ],
    },
    Case {
      name: "file-probe",
      yes: &["https://example.com/post"],
      no: &["https://www.instagram.com/p/123/"],
    },
    Case {
      name: "vimeo-player",
      yes: &[
        "https://vimeo.com/123",
        "https://vimeo.com/123/abc",
        "https://player.vimeo.com/video/123?h=abc",
      ],
      no: &[
        "https://vimeo.com/channels/staffpicks",
        "https://vimeo.com/123/abc?h=other",
      ],
    },
    Case {
      name: "reddit-post",
      yes: &[
        "https://www.reddit.com/r/nasa/comments/abc123/title/",
        "https://reddit.com/gallery/abc123",
        "https://reddit.com/r/nasa/s/AbC123",
        "https://redd.it/abc123",
        "https://v.redd.it/abc123",
      ],
      no: &[
        "https://www.reddit.com/r/nasa/",
        "https://reddit.com/comments/INVALID",
      ],
    },
    Case {
      name: "snapchat-spotlight",
      yes: &["https://www.snapchat.com/spotlight/AbC_123"],
      no: &[
        "https://www.snapchat.com/@nasa/spotlight/AbC_123",
        "https://snapchat.com/spotlight/",
      ],
    },
    Case {
      name: "threads-post",
      yes: &[
        "https://threads.com/@nasa/post/AbC_123",
        "https://www.threads.net/%40nasa/post/AbC_123/media",
      ],
      no: &[
        "https://threads.com/@nasa/",
        "https://threads.com/@.nasa/post/123",
        "http://threads.com/@nasa/post/123",
      ],
    },
    Case {
      name: "linkedin-post",
      yes: &[
        "https://www.linkedin.com/posts/nasa-activity-123-abc",
        "https://lnkd.in/p/AbC123",
        "https://fr.linkedin.com/feed/update/urn:li:activity:123/",
      ],
      no: &[
        "https://www.linkedin.com/in/nasa",
        "https://www.linkedin.com/posts/nasa%2Factivity-123-abc",
      ],
    },
    Case {
      name: "telegram-message",
      yes: &[
        "https://t.me/telegram/443",
        "https://telegram.me/s/telegram/443/2",
      ],
      no: &[
        "https://t.me/telegram",
        "https://t.me/share/443",
        "https://t.me/telegram/0",
      ],
    },
    Case {
      name: "facebook-share",
      yes: &[
        "https://fb.watch/AbC123/",
        "https://www.facebook.com/share/p/AbC123/",
      ],
      no: &[
        "https://facebook.com/watch/?v=123",
        "https://facebook.com/share/",
      ],
    },
    Case {
      name: "facebook-photo",
      yes: &[
        "https://facebook.com/photo/?fbid=123",
        "https://www.facebook.com/nasa/photos/a.12/123/",
      ],
      no: &[
        "https://facebook.com/photo/?fbid=bad",
        "https://facebook.com/watch/?v=123",
      ],
    },
    Case {
      name: "facebook-post",
      yes: &[
        "https://www.facebook.com/nasa/posts/123",
        "https://facebook.com/watch/?v=123",
        "https://facebook.com/story.php?story_fbid=123&id=42",
      ],
      no: &[
        "https://facebook.com/nasa",
        "https://facebook.com/stories/123",
        "https://facebook.com/watch/?v=123&v=456",
      ],
    },
    Case {
      name: "twitter-status",
      yes: &[
        "https://x.com/nasa/status/123",
        "https://mobile.twitter.com/nasa/status/123/photo/1",
      ],
      no: &[
        "https://x.com/nasa",
        "https://twitter.com/nasa/status/123/video/1",
        "https://x.com/nasa/status/",
      ],
    },
    Case {
      name: "instagram-post",
      yes: &[
        "https://www.instagram.com/p/AbC123/",
        "https://m.instagram.com/p/AbC123/",
        "https://fr.instagram.com/p/AbC123/",
      ],
      no: &[
        "https://instagram.com/reels/123",
        "https://instagram.com/p/",
      ],
    },
    Case {
      name: "bluesky-post",
      yes: &[
        "https://bsky.app/profile/nasa/post/123",
        "https://www.bsky.app/profile/nasa/post/123",
      ],
      no: &[
        "https://bsky.app/profile/nasa",
        "https://bsky.app/profile/nasa/post/",
      ],
    },
    Case {
      name: "pinterest-pin",
      yes: &[
        "https://www.pinterest.com/pin/123/",
        "https://m.pinterest.com/pin/123/",
        "https://fr.pinterest.com/pin/123/",
      ],
      no: &[
        "https://pinterest.com/nasa/",
        "https://pin.it/123",
        "https://pinterest.com/pin/",
      ],
    },
    Case {
      name: "tiktok-photo",
      yes: &["https://www.tiktok.com/@nasa/photo/123"],
      no: &[
        "https://tiktok.com/@nasa/video/123",
        "https://tiktok.com/@nasa/photo/",
      ],
    },
    Case {
      name: "tumblr-post",
      yes: &[
        "https://nasa.tumblr.com/post/820337874767675392/",
        "https://www.tumblr.com/nasa/820337874767675392/",
      ],
      no: &[
        "https://tumblr.com/",
        "https://www.tumblr.com/nasa",
        "https://nasa.tumblr.com/archive",
        "https://www.tumblr.com/tagged/2024",
        "https://www.tumblr.com/search/2024",
      ],
    },
    Case {
      name: "imgur-album",
      yes: &[
        "https://imgur.com/a/123",
        "https://www.imgur.com/gallery/123",
      ],
      no: &["https://imgur.com/123", "https://imgur.com/a/"],
    },
    Case {
      name: "youtube-exclusive",
      yes: &[
        "https://m.youtube.com/watch?v=123",
        "https://youtu.be/123",
        "https://www.youtube-nocookie.com/embed/123",
      ],
      no: &[
        "https://vimeo.com/123",
        "https://notyoutube.com/watch?v=123",
      ],
    },
    Case {
      name: "video-host",
      yes: &[
        "https://vimeo.com/123",
        "https://soundcloud.com/nasa/track",
        "https://clips.twitch.tv/Clip",
        "https://dailymotion.com/video/123",
      ],
      no: &["https://example.com/video/123"],
    },
    Case {
      name: "ytdlp-fallback",
      yes: &["https://example.com/post"],
      no: &["file:///video.mp4"],
    },
    Case {
      name: "gallery-fallback",
      yes: &["https://example.com/post"],
      no: &["file:///photo.jpg"],
    },
    Case {
      name: "page-scan",
      yes: &["https://example.com/post"],
      no: &[
        "https://threads.com/@nasa/post/123",
        "https://facebook.com/nasa",
      ],
    },
  ];
  let matchers: Vec<_> = std::iter::once(&FILE_SHARE)
    .chain(REGISTRY.iter().flat_map(|group| group.iter()))
    .collect();
  assert_eq!(
    cases.len(),
    matchers.len(),
    "every registration needs cases"
  );
  for case in cases {
    let matcher = matchers
      .iter()
      .find(|matcher| matcher.name == case.name)
      .expect(case.name);
    for raw in case.yes {
      assert!(
        matcher.matches(&reqwest::Url::parse(raw).unwrap()),
        "{} should match {raw}",
        case.name
      );
      if matcher.confidence == Confidence::High
        && case.name != "file-extension"
        && case.name != "file-share"
      {
        let mut evil = reqwest::Url::parse(raw).unwrap();
        let host = evil.host_str().unwrap().to_owned();
        for lookalike in [format!("{host}.evil.test"), format!("evil-{host}.test")] {
          evil.set_host(Some(&lookalike)).unwrap();
          assert!(!matcher.matches(&evil), "{} must reject {evil}", case.name);
        }
      }
    }
    for raw in case.no {
      assert!(
        !matcher.matches(&reqwest::Url::parse(raw).unwrap()),
        "{} should not match {raw}",
        case.name
      );
    }
  }
}

#[test]
fn priority_selects_narrow_reader_rules_and_keeps_other_candidates() {
  for (url, name) in [
    ("https://facebook.com/share/p/AbC/", "facebook-share"),
    ("https://facebook.com/photo/?fbid=123", "facebook-photo"),
    ("https://facebook.com/nasa/posts/123", "facebook-post"),
  ] {
    let routes = routes(url);
    assert_eq!(routes[0].name, name);
    assert!(routes.iter().any(|matcher| matcher.stage == StageId::Ytdlp));
    assert!(routes
      .iter()
      .any(|matcher| matcher.stage == StageId::Gallery));
    assert_eq!(
      routes
        .iter()
        .filter(|matcher| matches!(matcher.stage, StageId::Reader(_)))
        .count(),
      1
    );
  }
}

#[test]
fn invalid_urls_claim_nothing_and_youtube_exclusivity_is_host_bound() {
  for url in ["invalid", "file:///a.jpg", "mailto:test@example.com"] {
    assert!(routes(url).is_empty(), "{url}");
  }
  for url in [
    "https://youtube.com.evil.test/watch?v=123",
    "https://evilyoutu.be/123",
    "https://youtube-nocookie.com.evil.test/embed/123",
  ] {
    assert!(!exclusive(url), "{url}");
    assert_eq!(claim(StageId::Gallery, url), Confidence::Low);
  }
}

#[test]
fn video_hosts_try_ytdlp_before_gallery_but_photo_paths_keep_gallery_first() {
  for (url, first) in [
    ("https://x.com/nasa/status/123", StageId::Gallery),
    ("https://twitter.com/nasa/status/123", StageId::Gallery),
    ("https://www.tiktok.com/@nasa/video/123", StageId::Ytdlp),
    ("https://www.tiktok.com/@nasa/photo/123", StageId::Gallery),
    ("https://www.instagram.com/p/123", StageId::Gallery),
  ] {
    let stages: Vec<_> = stages_for(url)
      .into_iter()
      .filter(|stage| matches!(stage, StageId::Gallery | StageId::Ytdlp))
      .collect();
    assert_eq!(stages[0], first, "{url}");
  }
}

#[test]
fn legacy_capture_preference_is_independent_of_fetch_confidence() {
  for (url, preferred) in [
    ("https://flickr.com/photos/nasa/123", true),
    ("https://artstation.com/artwork/abc", true),
    ("https://imgur.com/abc123", true),
    ("https://archive.twitter.com/nasa/status/123", true),
    ("https://archive.x.com/nasa/status/123", true),
    ("https://twitch.tv/channel", true),
    ("https://dailymotion.com/video/abc", true),
    ("https://www.facebook.com/nasa/posts/123", true),
    ("https://example.com/reel/123", false),
    ("https://example.com/reels/123", false),
    ("https://vimeo.com/123", false),
    ("https://soundcloud.com/nasa/track", false),
    ("https://www.snapchat.com/spotlight/123", false),
    ("https://stories.snapchat.com/123", false),
    ("https://www.reddit.com/r/nasa/comments/123/title/", false),
    ("https://www.tiktok.com/@nasa/video/123", false),
    ("https://twitter.com/nasa/status/123/video/1", false),
    ("https://x.com/nasa", false),
    ("https://instagram.com/nasa", false),
    ("https://bsky.app/profile/nasa", false),
    ("https://pinterest.com/nasa", false),
  ] {
    assert_eq!(
      crate::parsers::gallery::gallery_first(url),
      preferred,
      "{url}"
    );
    assert_eq!(
      claim(StageId::Gallery, url),
      Confidence::Low,
      "fetch claim must stay Low: {url}"
    );
  }
  // A lookalike is not X (sites.rs dot boundary): its `/video/` path is not an X video.
  assert!(crate::parsers::gallery::gallery_first(
    "https://nottwitter.com/nasa/status/123/video/1"
  ));
}

#[test]
fn archive_items_try_files_without_losing_extractor_fallback() {
  let routes = super::routes("https://archive.org/details/example-item");
  assert_eq!(routes[0].stage, StageId::Files);
  assert_eq!(routes[0].confidence, Confidence::High);
  assert!(routes.iter().any(|r| r.stage == StageId::Ytdlp));
  assert!(routes.iter().all(|r| !r.exclusive));
}
