use super::*;
use super::{reply, FakeTransport};

const CANONICAL: &str = "https://www.reddit.com/comments/abc123/";

#[tokio::test]
async fn reddit_normalizes_post_aliases_before_fallback() {
  let permalink = "https://www.reddit.com/r/test/comments/abc123/title/";
  for (source, page) in [
    (
      "https://www.reddit.com/r/test/comments/abc123/title/?utm_source=share",
      permalink,
    ),
    (
      "https://old.reddit.com/r/test/comments/abc123/title/",
      permalink,
    ),
    ("https://new.reddit.com/comments/abc123/", CANONICAL),
    ("https://sh.reddit.com/comments/abc123/", CANONICAL),
    ("https://redd.it/abc123", CANONICAL),
    ("https://www.reddit.com/gallery/abc123", CANONICAL),
  ] {
    let transport = FakeTransport(std::sync::Mutex::new(
      [(page, reply(403, None, b""))].into(),
    ));
    assert_eq!(
      read_site(source, &transport).await.resolution(source),
      Resolution::Passthrough(CANONICAL.into()),
      "{source}"
    );
  }
}

#[tokio::test]
async fn reddit_share_redirect_resolves_to_the_post_before_fallback() {
  let source = "https://www.reddit.com/r/test/s/Share123/";
  let transport = FakeTransport(std::sync::Mutex::new(
    [
      (
        source,
        reply(
          301,
          Some("/r/test/comments/abc123/title/?share_id=tracking"),
          b"",
        ),
      ),
      (
        "https://www.reddit.com/r/test/comments/abc123/title/",
        reply(403, None, b""),
      ),
    ]
    .into(),
  ));
  assert_eq!(
    read_site(source, &transport).await.resolution(source),
    Resolution::Passthrough(CANONICAL.into())
  );
}

#[tokio::test]
async fn reddit_share_image_preserves_its_readable_permalink_for_download_refresh() {
  let source = "https://www.reddit.com/r/test/s/Share123/";
  let permalink = "https://www.reddit.com/r/test/comments/abc123/title/";
  let photo = b"<shreddit-post id='t3_abc123' post-type='image' content-href='https://i.redd.it/full.jpg'></shreddit-post>";
  let transport = FakeTransport(std::sync::Mutex::new(
    [
      (source, reply(301, Some(permalink), b"")),
      (permalink, reply(200, None, photo)),
    ]
    .into(),
  ));
  let Verdict::Verified(Resolution::Attachments { metadata }) = read_site(source, &transport).await
  else {
    panic!("image post expected");
  };
  let media: serde_json::Value = serde_json::from_str(&metadata).unwrap();
  assert_eq!(media["url"], permalink);
  let refresh = FakeTransport(std::sync::Mutex::new(
    [(permalink, reply(200, None, photo))].into(),
  ));
  assert!(matches!(
    read_site(media["url"].as_str().unwrap(), &refresh).await,
    Verdict::Verified(Resolution::Attachments { .. })
  ));
}

#[tokio::test]
async fn reddit_routes_only_the_requested_posts_media_from_fixtures() {
  let cases: serde_json::Value =
    serde_json::from_str(include_str!("../../tests/fixtures/reddit/posts.json")).unwrap();
  for case in cases.as_array().unwrap() {
    let body = case["html"].as_str().unwrap();
    let transport = FakeTransport(std::sync::Mutex::new(
      [(CANONICAL, reply(200, None, body.as_bytes()))].into(),
    ));
    let result = read_site(CANONICAL, &transport).await.resolution(CANONICAL);
    if let Some(target) = case["target"].as_str() {
      let actual = match result {
        Resolution::PostMedia {
          url,
          title,
          uploader,
          ..
        } => {
          assert_eq!(title.as_deref(), Some("Synthetic post"));
          assert_eq!(uploader.as_deref(), Some("synthetic"));
          url
        }
        _ => panic!("target expected"),
      };
      assert_eq!(actual, target, "{}", case["name"]);
    } else {
      let Resolution::Attachments { metadata } = result else {
        panic!("attachment post expected: {}", case["name"]);
      };
      let media: serde_json::Value = serde_json::from_str(&metadata).unwrap();
      assert_eq!(
        (
          media["mediaKind"].clone(),
          media["attachments"].as_array().unwrap().len()
        ),
        (
          case["kind"].clone(),
          case["count"].as_u64().unwrap() as usize
        ),
        "{}",
        case["name"]
      );
    }
  }
}

#[tokio::test]
async fn reddit_never_admits_another_posts_or_blurred_media() {
  for html in [
    "<shreddit-post id='t3_wrong' post-type='image' content-href='https://i.redd.it/decoy.jpg'></shreddit-post>",
    "<shreddit-post id='t3_abc123' post-type='image' nsfw content-href='https://i.redd.it/blur.jpg'></shreddit-post>",
    "<shreddit-post id='t3_abc123' post-type='image' content-href='https://preview.redd.it/image.jpg?blur=40'></shreddit-post>",
    "<shreddit-post id='t3_abc123' post-type='image'><shreddit-post id='t3_other' post-type='image' content-href='https://i.redd.it/decoy.jpg'><img src='https://i.redd.it/decoy.jpg'></shreddit-post></shreddit-post>",
  ] {
    let transport = FakeTransport(std::sync::Mutex::new([(CANONICAL, reply(200, None, html.as_bytes()))].into()));
    assert_eq!(read_site(CANONICAL, &transport).await.resolution(CANONICAL), Resolution::Passthrough(CANONICAL.into()));
  }
}

#[tokio::test]
async fn reddit_redirects_reject_external_hosts_and_changed_post_ids() {
  for target in [
    "https://evil.example/comments/abc123/",
    "http://www.reddit.com/comments/abc123/",
    "https://www.reddit.com/login",
    "https://www.reddit.com/comments/other123/",
  ] {
    let transport = FakeTransport(std::sync::Mutex::new(
      [(CANONICAL, reply(302, Some(target), b""))].into(),
    ));
    assert_eq!(
      read_site(CANONICAL, &transport).await.resolution(CANONICAL),
      Resolution::Passthrough(CANONICAL.into())
    );
  }
}

#[tokio::test]
async fn reddit_challenge_is_answered_once_on_the_same_page_without_cookies() {
  let body = br#"<form method="get" action="/comments/abc123/"><input name="jsc_token" value="token"><input name="solution" value=""></form><script>const x=await(async e=>e+e)("abcdef12")</script>"#;
  let photo = b"<shreddit-post id='t3_abc123' post-type='image' content-href='https://i.redd.it/full.jpg'></shreddit-post>";
  let transport = FakeTransport(std::sync::Mutex::new(
    [
      (CANONICAL, reply(200, None, body)),
      (
        "https://www.reddit.com/comments/abc123/?jsc_token=token&solution=abcdef12abcdef12",
        reply(200, None, photo),
      ),
    ]
    .into(),
  ));
  assert!(matches!(
    read_site(CANONICAL, &transport).await,
    Verdict::Verified(Resolution::Attachments { .. })
  ));
}

#[tokio::test]
async fn reddit_video_shortlink_binds_its_media_id_after_the_post_redirect() {
  let video = "https://www.reddit.com/video/abcdefgh1234/";
  let cases: serde_json::Value =
    serde_json::from_str(include_str!("../../tests/fixtures/reddit/posts.json")).unwrap();
  let html = cases
    .as_array()
    .unwrap()
    .iter()
    .find(|c| c["name"] == "video")
    .unwrap()["html"]
    .as_str()
    .unwrap();
  let transport = FakeTransport(std::sync::Mutex::new(
    [
      (video, reply(302, Some(CANONICAL), b"")),
      (CANONICAL, reply(200, None, html.as_bytes())),
    ]
    .into(),
  ));
  assert!(
    matches!(read_site("https://v.redd.it/abcdefgh1234", &transport).await, Verdict::Verified(Resolution::PostMedia { url, .. }) if url == "https://v.redd.it/abcdefgh1234/DASHPlaylist.mpd")
  );
}

#[tokio::test]
async fn reddit_requires_public_media_dns_before_handoff() {
  struct Refusing(FakeTransport);
  impl http::Transport for Refusing {
    async fn get(
      &self,
      url: &reqwest::Url,
      cap: usize,
    ) -> Result<http::Response, http::FetchError> {
      http::Transport::get(&self.0, url, cap).await
    }
    async fn admit_media(&self, _: &reqwest::Url) -> Result<(), http::FetchError> {
      Err(http::FetchError::Address)
    }
  }
  let cases: serde_json::Value =
    serde_json::from_str(include_str!("../../tests/fixtures/reddit/posts.json")).unwrap();
  for case in cases.as_array().unwrap() {
    let transport = Refusing(FakeTransport(std::sync::Mutex::new(
      [(
        CANONICAL,
        reply(200, None, case["html"].as_str().unwrap().as_bytes()),
      )]
      .into(),
    )));
    assert_eq!(
      read_site(CANONICAL, &transport).await.resolution(CANONICAL),
      Resolution::Passthrough(CANONICAL.into())
    );
  }
}

#[tokio::test]
async fn reddit_does_not_answer_altered_challenges_or_repeat_a_challenge() {
  const BODY: &str = r#"<form method="get" action="/comments/abc123/"><input name="jsc_token" value="token"><input name="solution" value=""></form><script>const x=await(async e=>e+e)("abcdef12")</script>"#;
  for body in [
    BODY.replace("e+e", "e+f"),
    BODY.replace(
      "action=\"/comments/abc123/\"",
      "action=\"https://evil.example/comments/abc123/\"",
    ),
    BODY.replace("method=\"get\"", "method=\"post\""),
  ] {
    let transport = FakeTransport(std::sync::Mutex::new(
      [(CANONICAL, reply(200, None, body.as_bytes()))].into(),
    ));
    assert_eq!(
      read_site(CANONICAL, &transport).await.resolution(CANONICAL),
      Resolution::Passthrough(CANONICAL.into())
    );
  }
  let transport = FakeTransport(std::sync::Mutex::new(
    [
      (CANONICAL, reply(200, None, BODY.as_bytes())),
      (
        "https://www.reddit.com/comments/abc123/?jsc_token=token&solution=abcdef12abcdef12",
        reply(200, None, BODY.as_bytes()),
      ),
    ]
    .into(),
  ));
  assert_eq!(
    read_site(CANONICAL, &transport).await.resolution(CANONICAL),
    Resolution::Passthrough(CANONICAL.into())
  );
}

#[tokio::test]
async fn reddit_body_cap_preserves_the_login_fallback() {
  let transport = FakeTransport(std::sync::Mutex::new(
    [(
      CANONICAL,
      reply(200, None, &vec![b'x'; 4 * 1024 * 1024 + 1]),
    )]
    .into(),
  ));
  assert_eq!(
    read_site(CANONICAL, &transport).await.resolution(CANONICAL),
    Resolution::Passthrough(CANONICAL.into())
  );
}

#[test]
fn reddit_identities_reject_unsafe_and_non_post_urls() {
  for source in [
    "http://www.reddit.com/comments/abc123/",
    "https://user@www.reddit.com/comments/abc123/",
    "https://www.reddit.com:444/comments/abc123/",
    "https://www.reddit.com.evil.example/comments/abc123/",
    "https://www.reddit.com/comments/abc123/#fragment",
    "https://www.reddit.com/r/test/",
    "https://www.reddit.com/user/test/",
  ] {
    assert!(!crate::resolve::reddit::is_post_url(source), "{source}");
  }
}

#[tokio::test]
async fn reddit_only_bound_link_and_text_posts_prove_no_media() {
  for (kind, id, href, expected) in [
    ("link", "abc123", "https://www.nytimes.com/article", true),
    ("link", "abc123", "https://www.reddit.com/r/games", false),
    (
      "link",
      "abc123",
      "https://www.reddit.com/gallery/abc123",
      false,
    ),
    (
      "link",
      "abc123",
      "https://cdn.example.org/clip.MP4?key=1",
      false,
    ),
    ("link", "abc123", "https://cdn.example.org/photo.jpg", false),
    ("text", "abc123", "", true),
    ("link", "abc123", "http://example.org/article", true),
    ("link", "abc123", "", true),
    ("link", "other123", "https://www.nytimes.com/article", false),
    ("unknown", "abc123", "", false),
  ] {
    let body = format!(
      "<shreddit-post id='t3_{id}' post-type='{kind}' content-href='{href}'></shreddit-post>"
    );
    let transport = FakeTransport(std::sync::Mutex::new(
      [(CANONICAL, reply(200, None, body.as_bytes()))].into(),
    ));
    let result = read_site(CANONICAL, &transport).await.resolution(CANONICAL);
    assert_eq!(
      result,
      if expected {
        Resolution::Unsupported {
          reason: Reason::NoMedia,
        }
      } else {
        Resolution::Passthrough(CANONICAL.into())
      }
    );
  }
}

#[tokio::test]
async fn reddit_text_post_keeps_inline_body_images() {
  let body = b"<shreddit-post id='t3_abc123' post-type='text'><div slot='text-body'><img src='https://i.redd.it/one.jpg'><img src='https://i.redd.it/two.png'></div></shreddit-post>";
  let transport = FakeTransport(std::sync::Mutex::new(
    [(CANONICAL, reply(200, None, body))].into(),
  ));
  let Resolution::Attachments { metadata } =
    read_site(CANONICAL, &transport).await.resolution(CANONICAL)
  else {
    panic!("inline images must remain downloadable");
  };
  let media: serde_json::Value = serde_json::from_str(&metadata).unwrap();
  assert_eq!(
    media["attachments"]
      .as_array()
      .unwrap()
      .iter()
      .map(|i| i["url"].as_str().unwrap())
      .collect::<Vec<_>>(),
    ["https://i.redd.it/one.jpg", "https://i.redd.it/two.png"]
  );
}

#[tokio::test]
async fn reddit_link_posts_still_forward_known_media_hosts() {
  let body = b"<shreddit-post id='t3_abc123' post-type='link' content-href='https://www.youtube.com/watch?v=abc123'></shreddit-post>";
  let transport = FakeTransport(std::sync::Mutex::new(
    [(CANONICAL, reply(200, None, body))].into(),
  ));
  assert!(
    matches!(read_site(CANONICAL, &transport).await.resolution(CANONICAL),
    Resolution::PostMedia { url, .. } if url == "https://www.youtube.com/watch?v=abc123")
  );
}
