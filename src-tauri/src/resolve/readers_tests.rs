use super::*;

const LINKEDIN: &str = "https://www.linkedin.com/posts/synthetic-activity-123/";
const TELEGRAM: &str = "https://t.me/synthetic_channel/123?embed=1&mode=tme";
fn reader_fixture(site: &str, name: &str) -> String {
  std::fs::read_to_string(format!(
    "{}/tests/fixtures/{site}/{name}.html",
    env!("CARGO_MANIFEST_DIR")
  ))
  .unwrap()
}
async fn page(input: &str, fetched: &'static str, body: &str) -> Option<serde_json::Value> {
  let fake = FakeTransport(std::sync::Mutex::new(
    [(fetched, reply(200, None, body.as_bytes()))].into(),
  ));
  match read_site(input, &fake).await {
    Verdict::Verified(Resolution::Attachments { metadata }) => {
      Some(serde_json::from_str(&metadata).unwrap())
    }
    _ => None,
  }
}
fn urls(media: &serde_json::Value) -> Vec<&str> {
  media["attachments"]
    .as_array()
    .unwrap()
    .iter()
    .map(|a| a["url"].as_str().unwrap())
    .collect()
}

// Omitting either reader, or harvesting OG/previews globally, breaks these real resolver results.
#[tokio::test]
async fn linkedin_bound_android_fixtures_produce_ordered_images() {
  let single = page(
    LINKEDIN,
    LINKEDIN,
    &reader_fixture("linkedin", "discussion-forum-post"),
  )
  .await
  .unwrap();
  assert_eq!(
    urls(&single),
    ["https://media.licdn.com/synthetic-image.jpg"]
  );
  assert_eq!(single["mediaKind"], "image");
  let multi = page(LINKEDIN, LINKEDIN, &reader_fixture("linkedin", "multi"))
    .await
    .unwrap();
  assert_eq!(
    urls(&multi),
    [
      "https://media.licdn.com/asset-one?sig=Ab%2fCD%2B%3d",
      "https://media.licdn.com/asset-two?sig=second"
    ]
  );
  assert_eq!(multi["mediaKind"], "carousel");
  assert_eq!(multi["title"], "Synthetic image post");
  assert_eq!(multi["uploader"], "Synthetic Author");
  let overflow = page(LINKEDIN, LINKEDIN, &reader_fixture("linkedin", "overflow"))
    .await
    .unwrap();
  assert_eq!(
    urls(&overflow),
    [
      "https://media.licdn.com/synthetic-1",
      "https://media.licdn.com/synthetic-2",
      "https://media.licdn.com/synthetic-3",
      "https://media.licdn.com/synthetic-4",
      "https://media.licdn.com/synthetic-5",
      "https://media.licdn.com/synthetic-6"
    ]
  );
}

#[test]
fn private_telegram_messages_require_login_without_claiming_public_posts() {
  for raw in [
    "https://t.me/c/1234567890/7",
    "https://telegram.me/c/1234567890/7/",
    "https://t.me/c/1234567890/42/7",
  ] {
    assert!(telegram::private_message(raw), "{raw}");
  }
  for raw in [
    "https://t.me/bbcrussian/101014",
    "https://t.me/s/bbcrussian/101014",
    "https://t.me/c/not-an-id/7",
    "https://t.me.evil.test/c/123456/7",
  ] {
    assert!(!telegram::private_message(raw), "{raw}");
  }
}

#[tokio::test]
async fn telegram_bound_android_fixtures_exclude_previews_replies_and_other_posts() {
  for (name, expected) in [
    (
      "single",
      vec!["https://cdn1.telesco.pe/file/syntheticPhoto1"],
    ),
    (
      "album",
      vec![
        "https://cdn1.telesco.pe/file/syntheticPhoto1",
        "https://cdn1.telesco.pe/file/syntheticPhoto2",
      ],
    ),
  ] {
    let body = reader_fixture("telegram", name);
    let media = page(
      "https://telegram.me/synthetic_channel/123?single",
      TELEGRAM,
      &body,
    )
    .await
    .unwrap();
    assert_eq!(urls(&media), expected);
    assert_eq!(media["title"], "Synthetic caption");
  }
  let single = reader_fixture("telegram", "single");
  let styled_caption = single.replace(
    "Synthetic caption",
    "First <b>styled</b> caption<br>Second line",
  );
  assert_eq!(
    page(TELEGRAM, TELEGRAM, &styled_caption).await.unwrap()["title"],
    "First styled caption"
  );
  // Live Telegram replies contain a stray quote after href; it must not swallow the album.
  let malformed_reply = single.replace("Synthetic caption</div>", "Synthetic caption</div><a class='tgme_widget_message_reply' href='https://t.me/synthetic_channel/122' \">Quoted post</a>");
  assert_eq!(
    urls(&page(TELEGRAM, TELEGRAM, &malformed_reply).await.unwrap()),
    ["https://cdn1.telesco.pe/file/syntheticPhoto1"]
  );
  let photo = "<a class='tgme_widget_message_photo_wrap' style=\"background-image:url('https://cdn1.telesco.pe/file/syntheticDecoy')\"></a>";
  let body = single.replace("Synthetic caption</div>", &format!("Synthetic caption</div><div class='tgme_widget_message_reply'>{photo}</div><div class='tgme_widget_message' data-post='synthetic_channel/999'>{photo}</div>"));
  assert_eq!(
    urls(&page(TELEGRAM, TELEGRAM, &body).await.unwrap()),
    ["https://cdn1.telesco.pe/file/syntheticPhoto1"]
  );
  for name in ["no-media", "private", "too-big", "video"] {
    assert!(
      page(TELEGRAM, TELEGRAM, &reader_fixture("telegram", name))
        .await
        .is_none(),
      "{name}"
    );
  }
}

#[tokio::test]
async fn linkedin_association_refuses_conflicts_avatars_and_semantic_decoys() {
  let body = reader_fixture("linkedin", "discussion-forum-post");
  let asset = "https://media.licdn.com/synthetic-image.jpg";
  let markup =
    format!("<ul class=\"feed-images-content\"><li><img data-delayed-url=\"{asset}\"></li></ul>");
  let mut cases = vec![
    body.replace("rel=\"canonical\" href=\"https://www.linkedin.com/posts/synthetic-activity-123\"", "rel=\"canonical\" href=\"https://www.linkedin.com/posts/other-activity-999\""),
    body.replace("\"@type\":\"DiscussionForumPosting\"", "\"@type\":\"ImageObject\""),
    body.replace("\"@type\":\"DiscussionForumPosting\"", "\"@type\":\"DigitalDocument\""),
    body.replace("content=\"https://media.licdn.com/synthetic-image.jpg\"", "content=\"https://media.licdn.com/avatar\""),
    body.replace("<li>", "<li data-test-id='related-content'>"),
    body.replace("<img data-delayed-url", "<a href='https://www.linkedin.com/posts/other-activity-999'><img data-delayed-url").replace("</li>", "</a></li>"),
    body.replace("data-activity-urn=\"urn:li:activity:123\"", "data-activity-urn=\"urn:li:activity:999\""),
    body.replace("<article ", "<article hidden "),
    body.replace("<article ", "<article aria-hidden=' true ' "),
    body.replace("</ul>", &format!("</ul>{markup}")),
    body.replace("</li>", "Advertisement</li>"),
    body.replace("\"identifier\":", &format!("\"author\":{{\"image\":\"{asset}\"}},\"identifier\":")),
    body.replace("\"@id\":", "\"url\":\"https://www.linkedin.com/posts/other-activity-999\",\"@id\":"),
    body.replace("</head>", "<script type='application/ld+json'>{\"@type\":\"DiscussionForumPosting\",\"@id\":\"https://www.linkedin.com/posts/synthetic-activity-123\",\"@id\":\"https://www.linkedin.com/posts/other-activity-999\"}</script></head>"),
  ];
  for wrapper in [
    "template",
    "noscript",
    "script type='text/plain'",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "plaintext",
  ] {
    cases.push(body.replace(
      &markup,
      &format!(
        "<{wrapper}>{markup}</{}>",
        wrapper.split(' ').next().unwrap()
      ),
    ));
  }
  for unsafe_url in [
    "http://media.licdn.com/image",
    "https://media.licdn.com.evil.test/image",
    "https://user@media.licdn.com/image",
    "https://media.licdn.com:444/image",
    "/relative",
  ] {
    cases.push(body.replace(asset, unsafe_url));
  }
  cases.push(format!(
    "{}{}{}",
    "<div>".repeat(257),
    body,
    "</div>".repeat(257)
  ));
  cases.push(" ".repeat(6 * 1024 * 1024 + 1));
  for (i, case) in cases.iter().enumerate() {
    assert!(
      page(LINKEDIN, LINKEDIN, case).await.is_none(),
      "accepted decoy {i}"
    );
  }
}

#[tokio::test]
async fn linkedin_shortlinks_bridge_share_images_and_keep_video_on_ytdlp() {
  let short = "https://lnkd.in/p/synthetic";
  let share = "https://www.linkedin.com/posts/synthetic-share-456/";
  let body = reader_fixture("linkedin", "share");
  let fake = FakeTransport(std::sync::Mutex::new(
    [
      (short, reply(301, Some(share), b"")),
      (share, reply(200, None, body.as_bytes())),
    ]
    .into(),
  ));
  let Verdict::Verified(Resolution::Attachments { metadata }) = read_site(short, &fake).await
  else {
    panic!("share not read");
  };
  let media: serde_json::Value = serde_json::from_str(&metadata).unwrap();
  assert_eq!(
    urls(&media),
    ["https://media.licdn.com/asset-one?sig=Ab%2fCD%2B%3d"]
  );
  for mutation in [
    body.replace("urn:li:share:456", "urn:li:share:999"),
    body.replace(
      "data-featured-activity-urn=\"urn:li:activity:123\"",
      "data-featured-activity-urn=\"urn:li:activity:999\"",
    ),
  ] {
    assert!(page(share, share, &mutation).await.is_none());
  }
  let video = "<link rel='canonical' href='https://www.linkedin.com/posts/synthetic-activity-123/'><meta property='og:type' content='video.other'><meta property='og:image' content='https://media.licdn.com/poster'><script type='application/ld+json'>{\"@type\":\"VideoObject\",\"@id\":\"https://www.linkedin.com/posts/synthetic-activity-123/\",\"thumbnailUrl\":\"https://media.licdn.com/poster\"}</script><video src='https://media.licdn.com/video.mp4'></video>";
  let fake = FakeTransport(std::sync::Mutex::new(
    [
      (short, reply(301, Some(LINKEDIN), b"")),
      (LINKEDIN, reply(200, None, video.as_bytes())),
    ]
    .into(),
  ));
  assert!(
    matches!(read_site(short, &fake).await, Verdict::Verified(Resolution::Passthrough(url)) if url == LINKEDIN)
  );
  for target in [
    "https://evil.test/posts/synthetic-activity-123",
    "http://www.linkedin.com/posts/synthetic-activity-123",
    "https://www.linkedin.com/login",
    "https://www.linkedin.com/posts/other-activity-999/",
  ] {
    let fake = FakeTransport(std::sync::Mutex::new(
      [(LINKEDIN, reply(302, Some(target), b""))].into(),
    ));
    assert!(matches!(read_site(LINKEDIN, &fake).await, Verdict::Unknown));
  }
}

#[tokio::test]
async fn linkedin_extensionless_share_photo_reaches_selected_image_download() {
  let share = "https://www.linkedin.com/posts/example-share-456-photo/";
  let body = reader_fixture("linkedin", "share-image-object");
  let media = page(
    "https://www.linkedin.com/posts/example-share-456-photo/?highlightedUpdateUrn=urn%3Ali%3Aactivity%3A999&utm_source=share",
    share,
    &body,
  )
  .await
  .unwrap();
  assert_eq!(media["mediaKind"], "image");
  assert_eq!(
    urls(&media),
    ["https://media.licdn.com/feedshare-shrink_800/example?e=123&v=beta"]
  );
  assert_eq!(media["url"], share);
  // LinkedIn CDN paths have no extension. Successful metadata must also reach the image runner.
  let post = crate::parsers::gallery::GalleryPost::from_resolved(&media.to_string(), share)
    .expect("owned extensionless photo must be downloadable");
  let selected = post
    .selected(Some(&[media["attachments"][0]["id"]
      .as_str()
      .unwrap()
      .into()]))
    .unwrap();
  assert_eq!(selected.len(), 1);
  assert_eq!(selected[0].file_index, None);
  for (url, accepted) in [
    (
      "https://media.licdn.com/feedshare-shrink_800/example?e=456&v=beta",
      true,
    ),
    ("https://media.licdn.com/other-post", false),
  ] {
    let receipt = serde_json::json!({"nezzel_url": url, "extension": "png"});
    assert_eq!(
      crate::parsers::gallery::verify_receipt(receipt.to_string().as_bytes(), &post, selected[0])
        .is_ok(),
      accepted
    );
  }
  for mutation in [
    body.replace("urn:li:share:456", "urn:li:share:777"),
    body.replace("feedshare-shrink_800/example?e=123&v=beta", "unowned-image"),
  ] {
    assert!(page(share, share, &mutation).await.is_none());
  }
}

#[tokio::test]
async fn telegram_identity_media_policy_and_video_handoff() {
  for input in [
    "https://t.me/c/123/456",
    "https://t.me/share/123",
    "https://t.me/abcd/123",
    "https://t.me/synthetic_channel/0",
    "http://t.me/synthetic_channel/123",
    "https://t.me.evil.test/synthetic_channel/123",
    "https://user@t.me/synthetic_channel/123",
  ] {
    let fake = FakeTransport(std::sync::Mutex::new([].into()));
    assert!(
      matches!(read_site(input, &fake).await, Verdict::NoMatch),
      "{input}"
    );
  }
  let single = reader_fixture("telegram", "single");
  for input in [
    "https://t.me/s/synthetic_channel/123",
    "https://t.me/synthetic_channel/77/123?single",
  ] {
    assert_eq!(
      urls(&page(input, TELEGRAM, &single).await.unwrap()),
      ["https://cdn1.telesco.pe/file/syntheticPhoto1"]
    );
  }
  for host in ["cdn1.telesco.pe.evil.test", "127.0.0.1", "localhost"] {
    assert!(
      page(TELEGRAM, TELEGRAM, &single.replace("cdn1.telesco.pe", host))
        .await
        .is_none()
    );
  }
  for body in [
    single.replace("synthetic_channel/123", "synthetic_channel/999"),
    single.replace("syntheticPhoto1", "syntheticPhoto1?query=1"),
    single.replace(
      "<div class=\"tgme_widget_message\"",
      "<div hidden class=\"tgme_widget_message\"",
    ),
    format!("{single}{single}"),
    " ".repeat(1024 * 1024 + 1),
  ] {
    assert!(page(TELEGRAM, TELEGRAM, &body).await.is_none());
  }
  let fake = FakeTransport(std::sync::Mutex::new(
    [(
      TELEGRAM,
      reply(200, None, reader_fixture("telegram", "video").as_bytes()),
    )]
    .into(),
  ));
  assert!(
    matches!(read_site(TELEGRAM,&fake).await, Verdict::Verified(Resolution::Passthrough(url)) if url == "https://t.me/synthetic_channel/123")
  );
  for status in [302, 403, 404, 410] {
    let fake = FakeTransport(std::sync::Mutex::new(
      [(
        TELEGRAM,
        reply(status, Some("https://evil.test"), single.as_bytes()),
      )]
      .into(),
    ));
    assert!(matches!(read_site(TELEGRAM, &fake).await, Verdict::Unknown));
  }
}

#[tokio::test]
async fn readers_reject_media_dns_and_generic_fallback_for_owned_sites() {
  struct Blocked;
  impl http::Transport for Blocked {
    async fn get(
      &self,
      url: &reqwest::Url,
      _cap: usize,
    ) -> Result<http::Response, http::FetchError> {
      Ok(reply(
        200,
        None,
        reader_fixture(
          if url.host_str() == Some("t.me") {
            "telegram"
          } else {
            "linkedin"
          },
          if url.host_str() == Some("t.me") {
            "single"
          } else {
            "multi"
          },
        )
        .as_bytes(),
      ))
    }
    async fn admit_media(&self, _url: &reqwest::Url) -> Result<(), http::FetchError> {
      Err(http::FetchError::Address)
    }
  }
  for input in [LINKEDIN, TELEGRAM] {
    assert!(matches!(read_site(input, &Blocked).await, Verdict::Unknown));
    assert_eq!(
      after_unsupported(input, "ERROR: Unsupported URL:").await,
      None
    );
  }
}

#[test]
fn entities_decode_short_forms_and_leave_bare_ampersands() {
  assert_eq!(html::entities("a&amp;b&lt;c&#39;"), "a&b<c'");
  // A ';' far after '&' is not an entity terminator.
  assert_eq!(
    html::entities("Q&A then much later; ok"),
    "Q&A then much later; ok"
  );
  let hostile = "&".repeat(200_000);
  assert_eq!(html::entities(&hostile), hostile);
}
