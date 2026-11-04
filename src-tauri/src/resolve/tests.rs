use super::*;

use super::html::generic_fixtures;

#[path = "reddit_tests.rs"]
mod reddit;

#[path = "readers_tests.rs"]
mod readers;

fn facebook_read(html: &str, url: &str) -> Option<Resolution> {
  let identity = facebook::Identity::parse(url)?;
  match facebook::read(html, &identity, &identity) {
    Outcome::Verified(result) => Some(result),
    _ => None,
  }
}

const POST: &str = "https://www.facebook.com/story.php?story_fbid=900000000000001&id=42";
fn fixture(name: &str) -> String {
  std::fs::read_to_string(
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/facebook/").to_owned() + name,
  )
  .unwrap()
}

// Dropping identity binding would select the leading conflicting root or feed decoys.
#[test]
fn facebook_reads_only_the_bound_attachment() {
  for file in ["video-story.html", "video-decoys-post.html"] {
    assert_eq!(
      facebook_read(&fixture(file), POST),
      Some(Resolution::Passthrough(
        "https://www.facebook.com/watch/?v=900000000000002".into()
      ))
    );
  }
  for file in [
    "video-conflict-post.html",
    "video-post.html",
    "reel-page.html",
    "group-text-post.html",
    "link-share-post.html",
    "unavailable.html",
    "login-page.html",
    "video-og-post.html",
  ] {
    assert_eq!(facebook_read(&fixture(file), POST), None, "{file}");
  }
  for file in ["photo-post.html", "album-post.html", "decoys-post.html"] {
    assert_eq!(
      facebook_read(&fixture(file), POST),
      Some(Resolution::Unsupported {
        reason: Reason::PhotosOnly
      }),
      "{file}"
    );
  }
  assert_eq!(
    facebook_read(
      &fixture("video-decoys-post.html"),
      "https://www.facebook.com/a/posts/pfbidUnknown"
    ),
    None
  );
  let oversized_string = format!("<meta property='og:image' content='https://scontent.example.fbcdn.net/1.jpg'><script data-sjs>{{\"RelayPrefetchedStreamCache\":\"{}\"}}</script>", "x".repeat(512 * 1024 + 1));
  let identity = facebook::Identity::parse(POST).unwrap();
  assert!(matches!(
    facebook::read(&oversized_string, &identity, &identity),
    Outcome::Unknown
  ));
}

// Attribute order, HTML entities and relative URLs occur in real publisher metadata.
#[test]
fn generic_video_ignores_cover_images_and_script_decoys() {
  let page = "https://example.com/posts/one";
  for (html, expected) in generic_fixtures::videos() {
    assert_eq!(
      html::generic(html, page),
      expected.map(|url| Resolution::Passthrough(url.into()))
    );
  }
}

// Frontends can build embeds in JS; an unrelated ID or an ambiguous playlist must not win.
#[test]
fn generic_youtube_frontends_require_evidence_and_an_unambiguous_id() {
  for (html, page, expected) in generic_fixtures::frontends() {
    assert_eq!(
      html::generic(html, page),
      expected.map(|url| Resolution::Passthrough(url.into())),
      "{page}: {html}"
    );
  }
  for path in generic_fixtures::SLOTS {
    assert_eq!(
      html::generic(
        "<script>const host = 'youtube.com/embed/';</script>",
        &format!("https://example.com{path}")
      ),
      Some(Resolution::Passthrough(
        "https://www.youtube.com/watch?v=qpoRO378qRY".into()
      )),
      "{path}"
    );
  }
}

// These literals exercise the SSRF boundary, including transition address encodings.
#[test]
fn dns_guard_rejects_special_ranges() {
  for ip in [
    "0.0.0.0",
    "10.1.2.3",
    "100.64.1.1",
    "127.0.0.1",
    "169.254.169.254",
    "172.16.0.1",
    "192.0.0.8",
    "192.168.1.1",
    "198.18.0.1",
    "192.0.2.1",
    "198.51.100.2",
    "203.0.113.1",
    "224.0.0.1",
    "255.255.255.255",
    "::",
    "::1",
    "fc00::1",
    "fe80::1",
    "ff02::1",
    "2001:db8::1",
    "2002:808:808::1",
    "2001::1",
    "3fff::1",
    "::ffff:127.0.0.1",
    "64:ff9b::a00:1",
  ] {
    assert!(!http::public_address(ip.parse().unwrap()), "{ip}");
  }
  for ip in [
    "8.8.8.8",
    "1.1.1.1",
    "2606:4700:4700::1111",
    "::ffff:8.8.8.8",
    "64:ff9b::808:808",
  ] {
    assert!(http::public_address(ip.parse().unwrap()), "{ip}");
  }
}

#[tokio::test]
async fn unmatched_host_is_unchanged_without_a_page_fetch() {
  assert_eq!(
    resolve("https://example.invalid/original?token=secret").await,
    Resolution::Passthrough("https://example.invalid/original?token=secret".into())
  );
}

#[test]
fn facebook_identity_table() {
  for (input, expected) in [
    (
      "https://m.facebook.com/share/p/1AbC/?tracking=1",
      "https://www.facebook.com/share/p/1AbC/",
    ),
    (
      "https://web.facebook.com/share/v/AbC",
      "https://www.facebook.com/share/v/AbC",
    ),
    (
      "https://facebook.com/share/r/Q1/",
      "https://www.facebook.com/share/r/Q1/",
    ),
    (
      "https://fb.watch/aBc_12-/?tracking=1",
      "https://fb.watch/aBc_12-/",
    ),
    (
      "https://facebook.com/story.php?id=42&story_fbid=123&rdid=x",
      "https://www.facebook.com/story.php?story_fbid=123&id=42",
    ),
    (
      "https://m.facebook.com/permalink.php?id=42&story_fbid=pfbid0AbCd",
      "https://www.facebook.com/permalink.php?story_fbid=pfbid0AbCd&id=42",
    ),
    (
      "https://ar-ar.facebook.com/some.page/posts/123/",
      "https://www.facebook.com/some.page/posts/123/",
    ),
    (
      "https://mbasic.facebook.com/groups/g/?multi_permalinks=123&ref=share",
      "https://www.facebook.com/groups/g/posts/123/",
    ),
    (
      "https://touch.facebook.com/watch/?v=777&ref=share",
      "https://www.facebook.com/watch/?v=777",
    ),
    (
      "https://mobile.facebook.com/reel/777/",
      "https://www.facebook.com/reel/777/",
    ),
    (
      "https://m.facebook.com/page/videos/some-title/777/",
      "https://www.facebook.com/page/videos/some-title/777/",
    ),
    (
      "https://web.facebook.com/photo.php?fbid=555&set=a.1",
      "https://www.facebook.com/photo/?fbid=555",
    ),
  ] {
    assert_eq!(
      facebook::Identity::parse(input).unwrap().url.as_str(),
      expected
    );
  }
  for input in [
    "http://facebook.com/share/p/AbC",
    "https://www.facebook.com.evil.example/share/p/AbC",
    "https://user@facebook.com/share/p/AbC",
    "https://facebook.com:8443/share/p/AbC",
    "https://facebook.com/watch/?v=1&v=2",
    "https://facebook.com/photo/?fbid=abc",
    "https://facebook.com/story.php?story_fbid=1",
    "https://facebook.com/events/posts/123",
    "https://facebook.com/stories/posts/123",
    "https://facebook.com/",
    "https://fb.watch/",
    "https://facebook.com/watch/?v=1#fragment",
  ] {
    assert!(facebook::Identity::parse(input).is_none(), "{input}");
  }
}

// Registry selection must leave the exclusive fast path entirely free of reader HTTP calls.
#[tokio::test]
async fn youtube_identity_never_fetches_a_reader_page() {
  for url in [
    "https://www.youtube.com/watch?v=123",
    "https://youtu.be/123",
    "https://youtube.com/playlist?list=123",
    "https://youtube-nocookie.com/embed/123",
  ] {
    let transport = FakeTransport(std::sync::Mutex::new(Default::default()));
    assert_eq!(
      read_site(url, &transport).await.resolution(url),
      Resolution::Passthrough(url.into())
    );
  }
}

struct FakeTransport(std::sync::Mutex<std::collections::VecDeque<(&'static str, http::Response)>>);
impl http::Transport for FakeTransport {
  async fn admit_media(&self, _url: &reqwest::Url) -> Result<(), http::FetchError> {
    Ok(())
  }
  async fn get(&self, url: &reqwest::Url, _cap: usize) -> Result<http::Response, http::FetchError> {
    let (expected, response) = self
      .0
      .lock()
      .unwrap()
      .pop_front()
      .expect("unexpected extra request");
    assert_eq!(url.as_str(), expected);
    Ok(response)
  }
}
fn reply(status: u16, location: Option<&str>, body: &[u8]) -> http::Response {
  http::Response {
    status,
    location: location.map(str::to_owned),
    body: body.into(),
    content_type: Some("text/html".into()),
    content_length: Some(body.len() as u64),
  }
}

// These exercise the real redirect state machine, without any network calls.
#[tokio::test]
async fn redirects_resolve_shortlinks_but_refuse_loops_downgrades_and_extra_hops() {
  let short = "https://www.facebook.com/share/p/AbC/";
  let fake = FakeTransport(std::sync::Mutex::new(
    [
      (
        short,
        reply(
          302,
          Some("/story.php?story_fbid=123&id=42&tracking=x"),
          b"ignored",
        ),
      ),
      (
        "https://www.facebook.com/story.php?story_fbid=123&id=42",
        reply(200, None, b"post"),
      ),
    ]
    .into(),
  ));
  let page = http::fetch(&fake, short.parse().unwrap(), true)
    .await
    .unwrap();
  assert_eq!(
    page.url.as_str(),
    "https://www.facebook.com/story.php?story_fbid=123&id=42"
  );
  assert_eq!(page.body, b"post");
  for target in [
    "http://www.facebook.com/a/posts/123",
    short,
    "https://evil.example/a/posts/123",
    "https://www.facebook.com/",
    "https://www.facebook.com:8443/a/posts/123",
  ] {
    let fake = FakeTransport(std::sync::Mutex::new(
      [(short, reply(302, Some(target), b""))].into(),
    ));
    assert!(http::fetch(&fake, short.parse().unwrap(), true)
      .await
      .is_err());
  }
  let fake = FakeTransport(std::sync::Mutex::new(
    [
      ("https://example.com/0", reply(302, Some("/1"), b"")),
      ("https://example.com/1", reply(302, Some("/2"), b"")),
      ("https://example.com/2", reply(302, Some("/3"), b"")),
      ("https://example.com/3", reply(302, Some("/4"), b"")),
    ]
    .into(),
  ));
  assert!(matches!(
    http::fetch(&fake, "https://example.com/0".parse().unwrap(), false).await,
    Err(http::FetchError::Redirect)
  ));
}

#[tokio::test]
async fn page_budget_and_login_wall_precede_metadata() {
  let url = "https://example.com/post";
  let fake = FakeTransport(std::sync::Mutex::new(
    [(url, reply(200, None, &vec![b'x'; 1024 * 1024 + 1]))].into(),
  ));
  assert!(matches!(
    http::fetch(&fake, url.parse().unwrap(), false).await,
    Err(http::FetchError::BodyLimit)
  ));
  for fixture_name in ["login-page.html", "unavailable.html"] {
    let identity = facebook::Identity::parse(POST).unwrap();
    assert!(matches!(
      facebook::read(&fixture(fixture_name), &identity, &identity),
      Outcome::Unknown
    ));
  }
  let identity = facebook::Identity::parse(POST).unwrap();
  assert!(matches!(
    facebook::read(&fixture("mixed-post.html"), &identity, &identity),
    Outcome::Verified(Resolution::Passthrough(url)) if url == "https://www.facebook.com/watch/?v=900000000000003"
  ));
  let missing_video_id = fixture("mixed-post.html").replace("900000000000003", "invalid-video-id");
  assert!(matches!(
    facebook::read(&missing_video_id, &identity, &identity),
    Outcome::Unknown
  ));
  let multiple_videos = fixture("mixed-post.html").replace(
    "\"nodes\":[",
    "\"nodes\":[{\"media\":{\"__typename\":\"Video\",\"id\":\"900000000000099\"}},",
  );
  assert!(matches!(
    facebook::read(&multiple_videos, &identity, &identity),
    Outcome::Unknown
  ));
  let consent = "<meta property='og:image' content='https://scontent.example.fbcdn.net/1.jpg'><form id='consent_form' action='/privacy/consent/'></form>";
  assert!(matches!(
    facebook::read(consent, &identity, &identity),
    Outcome::Unknown
  ));
}

#[tokio::test]
async fn facebook_regional_and_login_wrappers_keep_post_binding() {
  let short = "https://www.facebook.com/share/p/AbC/";
  let fake = FakeTransport(std::sync::Mutex::new([
    (short, reply(302, Some("https://web.facebook.com/share/p/AbC/"), b"")),
    ("https://web.facebook.com/share/p/AbC/", reply(302, Some("/login/?next=https%3A%2F%2Fwww.facebook.com%2Fstory.php%3Fstory_fbid%3D900000000000001%26id%3D42"), b"")),
    (POST, reply(200, None, fixture("photo-post.html").as_bytes())),
  ].into()));
  assert!(matches!(
    read_site(short, &fake).await,
    Outcome::Verified(Resolution::Unsupported {
      reason: Reason::PhotosOnly
    })
  ));
  let moved = "https://www.facebook.com/reel/900000000000789";
  let fake = FakeTransport(std::sync::Mutex::new(
    [
      (POST, reply(302, Some(moved), b"")),
      (
        moved,
        reply(200, None, fixture("reel-page.html").as_bytes()),
      ),
    ]
    .into(),
  ));
  assert!(matches!(read_site(POST, &fake).await, Outcome::Unknown));
  let fake = FakeTransport(std::sync::Mutex::new(
    [
      ("https://fb.watch/AbC/", reply(302, Some(moved), b"")),
      (
        moved,
        reply(200, None, fixture("reel-page.html").as_bytes()),
      ),
    ]
    .into(),
  ));
  assert!(
    matches!(read_site("https://fb.watch/AbC/", &fake).await, Outcome::Verified(Resolution::Passthrough(url)) if url == "https://www.facebook.com/watch/?v=900000000000789")
  );
  let fake = FakeTransport(std::sync::Mutex::new(
    [(short, reply(302, Some("/checkpoint/"), b""))].into(),
  ));
  assert!(matches!(read_site(short, &fake).await, Outcome::Unknown));
}

#[tokio::test]
async fn generic_fallback_keeps_native_errors_and_ambiguous_facebook_pages() {
  assert_eq!(
    after_unsupported("https://example.invalid/a", "ERROR: login required").await,
    None
  );
  assert_eq!(
    after_unsupported("https://www.facebook.com/feed", "ERROR: Unsupported URL:").await,
    None
  );
}

#[test]
fn streaming_and_dns_admission_fail_closed() {
  let mut body = b"abc".to_vec();
  http::append(&mut body, b"de", 5).unwrap();
  assert_eq!(
    http::append(&mut body, b"f", 5),
    Err(http::FetchError::BodyLimit)
  );
  assert_eq!(body, b"abcde");
  assert_eq!(
    http::admit_addresses(vec![
      "8.8.8.8:443".parse().unwrap(),
      "10.0.0.1:443".parse().unwrap()
    ]),
    Err(http::FetchError::Address)
  );
  assert_eq!(
    http::admit_addresses(vec![]),
    Err(http::FetchError::Address)
  );
}

#[tokio::test]
async fn shortlink_reader_verifies_photos_video_and_preserves_unknown() {
  let short = "https://www.facebook.com/share/p/AbC/";
  for (fixture_name, expected) in [
    (
      "album-post.html",
      Resolution::Unsupported {
        reason: Reason::PhotosOnly,
      },
    ),
    (
      "video-story.html",
      Resolution::Passthrough("https://www.facebook.com/watch/?v=900000000000002".into()),
    ),
  ] {
    let fake = FakeTransport(std::sync::Mutex::new(
      [
        (short, reply(302, Some(POST), b"")),
        (POST, reply(200, None, fixture(fixture_name).as_bytes())),
      ]
      .into(),
    ));
    assert!(
      matches!(read_site(short, &fake).await, Outcome::Verified(result) if result == expected)
    );
  }
  let fake = FakeTransport(std::sync::Mutex::new(
    [(POST, reply(404, None, b"error"))].into(),
  ));
  assert!(matches!(read_site(POST, &fake).await, Outcome::Unknown));
  let fake = FakeTransport(std::sync::Mutex::new([].into()));
  assert!(matches!(
    read_site("https://example.invalid/", &fake).await,
    Outcome::NoMatch
  ));
}

#[test]
fn only_unsupported_errors_enable_generic_and_rewritten_expiry_retries_original_once() {
  assert!(unsupported_url(
    "ERROR: [generic] Unsupported URL: https://example.com/a"
  ));
  assert!(!unsupported_url(
    "WARNING: Unsupported URL: https://example.com/a"
  ));
  assert!(!unsupported_url("ERROR: Login required"));
  let mut input = ResolvedInput::new("https://example.com/page");
  input.rewrite("https://cdn.example.com/clip.mp4?token=sensitive".into());
  assert_eq!(
    input.source_page.as_deref(),
    Some("https://example.com/page")
  );
  assert!(!input.retry_original("ERROR: HTTP Error 500: server error"));
  assert!(input.retry_original("ERROR: HTTP Error 403: Forbidden"));
  assert_eq!(input.url, "https://example.com/page");
  assert_eq!(input.source_page, None);
  assert!(!input.retry_original("ERROR: HTTP Error 404: Not Found"));
  input.rewrite("https://cdn.example.com/refreshed.mp4".into());
  assert!(!input.retry_original("ERROR: HTTP Error 403: Forbidden"));
}

#[test]
fn resolved_downloads_and_login_resumes_use_the_rewritten_target_and_referer() {
  for (original, target, referer) in [
    (
      "https://vimeo.com/76979871",
      "https://player.vimeo.com/video/76979871",
      "https://vimeo.com/",
    ),
    (
      "https://www.snapchat.com/spotlight/W7_abc123",
      "https://www.snapchat.com/@creator/spotlight/W7_abc123",
      "https://www.snapchat.com/spotlight/W7_abc123",
    ),
    (
      "https://www.facebook.com/reel/123456",
      "https://www.facebook.com/watch/?v=123456",
      "https://www.facebook.com/reel/123456",
    ),
    (
      "https://www.reddit.com/r/nasa/comments/1wtjy4r/nasas_view_program_running_on_an_univac_1108_to/",
      "https://v.redd.it/clip/DASHPlaylist.mpd?token=signed",
      "https://www.reddit.com/r/nasa/comments/1wtjy4r/nasas_view_program_running_on_an_univac_1108_to/",
    ),
    (
      "https://example.com/post",
      "https://cdn.example.com/video.mp4",
      "https://example.com/post",
    ),
  ] {
    let mut input = ResolvedInput::new(original);
    input.rewrite(target.into());
    // Browser-consent resumes clone the resolved input into the queued download entry.
    let resumed = input.clone();
    assert_eq!(resumed.args(), ["--referer", referer, "--", target]);

    // The published fetch row must survive the IPC download handoff, rather
    // than discarding the target and re-extracting its login-walled source page.
    let mut media = crate::parsers::ytdlp_info::parse_ytdlp_info(
      &serde_json::json!({"webpage_url":target, "title":"Reader video"}).to_string(),
      "resolved-row".into(),
    ).unwrap();
    input.retain_fetch_target(&mut media);
    let attempts = crate::extract::chain::StageResult::<()>::new(
      crate::extract::outcome::StageId::Ytdlp,
      crate::extract::outcome::Confidence::Medium,
    ).finish(crate::extract::outcome::Outcome::Verified, false, std::time::Instant::now()).attempts;
    crate::extract::chain::record_fetch_engine(&mut media, &attempts);
    let mut row = serde_json::to_value(media).unwrap();
    row["format"] = serde_json::json!({"trackType":"both"});
    row["templateContext"] = serde_json::json!({"values":{}});
    let item = serde_json::from_value(row).unwrap();
    let mut entry = crate::scheduling::download_pipeline::DownloadEntry::from((item, "resolved-row".into()));
    assert_eq!(entry.url, original);
    let resolved = entry.resolved.as_ref().expect("fetched target must bypass the download resolver");
    assert_eq!(resolved.args(), ["--referer", referer, "--", target]);
    let resumed = entry.login_request_entry();
    assert_eq!(resumed.resolved.as_ref().unwrap().args(), ["--referer", referer, "--", target]);
    assert_eq!(entry.take_fetch_input().unwrap().args(), ["--referer", referer, "--", target]);
  }
  let input = ResolvedInput::new("https://example.com/video");
  assert_eq!(input.args(), ["--", "https://example.com/video"]);

  // A reader's unchanged passthrough is also a verified target, distinct from
  // an old saved row that has never carried resolvedUrl.
  let source = "https://www.linkedin.com/posts/user-activity-1234567890123456789-abc";
  let mut media = crate::parsers::ytdlp_info::parse_ytdlp_info(
    &serde_json::json!({"webpage_url":source}).to_string(),
    "passthrough".into(),
  )
  .unwrap();
  ResolvedInput::new(source).retain_fetch_target(&mut media);
  let mut row = serde_json::to_value(media).unwrap();
  row["fetchEngine"] = serde_json::json!("ytdlp");
  row["format"] = serde_json::json!({"trackType":"both"});
  row["templateContext"] = serde_json::json!({"values":{}});
  let mut entry = crate::scheduling::download_pipeline::DownloadEntry::from((
    serde_json::from_value(row).unwrap(),
    "passthrough".into(),
  ));
  assert_eq!(entry.take_fetch_input().unwrap().args(), ["--", source]);
}

#[test]
fn saved_reader_rows_without_a_target_keep_resolution_but_native_rows_skip_it() {
  for (url, needs_reader) in [
    (
      "https://www.reddit.com/r/nasa/comments/1wtjy4r/title/",
      true,
    ),
    ("https://www.youtube.com/watch?v=123", false),
  ] {
    let item = serde_json::from_value(serde_json::json!({
      "id":"saved-row", "url":url, "fetchEngine":"ytdlp",
      "format":{"trackType":"both"}, "templateContext":{"values":{}}
    }))
    .unwrap();
    let mut entry =
      crate::scheduling::download_pipeline::DownloadEntry::from((item, "saved-row".into()));
    assert_eq!(entry.take_fetch_input().is_none(), needs_reader, "{url}");
  }
}

#[tokio::test]
async fn generic_media_uses_the_fetched_response_instead_of_the_extension() {
  let url = "https://8.8.8.8/clip.mp4";
  let mut media = reply(200, None, b"");
  media.content_type = Some("video/mp4".into());
  let fake = FakeTransport(std::sync::Mutex::new(
    [
      (url, reply(302, Some("/stream"), b"")),
      ("https://8.8.8.8/stream", media),
    ]
    .into(),
  ));
  assert!(
    matches!(read_generic(url, &fake).await, Outcome::Verified(Resolution::Passthrough(target)) if target == "https://8.8.8.8/stream")
  );
}

#[tokio::test]
async fn vimeo_pages_use_player_with_unlisted_hash_and_vimeo_referer() {
  for (url, expected) in [
    (
      "https://vimeo.com/13739299",
      "https://player.vimeo.com/video/13739299",
    ),
    (
      "https://www.vimeo.com/13739299/abc123",
      "https://player.vimeo.com/video/13739299?h=abc123",
    ),
    (
      "https://vimeo.com/13739299?h=abc123",
      "https://player.vimeo.com/video/13739299?h=abc123",
    ),
    (
      "https://player.vimeo.com/video/13739299?h=abc123",
      "https://player.vimeo.com/video/13739299?h=abc123",
    ),
  ] {
    let fake = FakeTransport(std::sync::Mutex::new(Default::default()));
    let resolution = read_site(url, &fake).await.resolution(url);
    assert_eq!(resolution, Resolution::Passthrough(expected.into()));
    let mut input = ResolvedInput::new(url);
    input.rewrite(expected.into());
    assert_eq!(input.source_page.as_deref(), Some("https://vimeo.com/"));
  }
  for url in [
    "https://vimeo.com/channels/staffpicks",
    "https://vimeo.com.evil.test/13739299",
    "https://vimeo.com/13739299/not/a/hash",
  ] {
    let fake = FakeTransport(std::sync::Mutex::new(Default::default()));
    assert_eq!(
      read_site(url, &fake).await.resolution(url),
      Resolution::Passthrough(url.into())
    );
  }
}

#[tokio::test]
async fn snapchat_bare_spotlight_uses_only_canonical_for_the_same_id() {
  const URL: &str = "https://www.snapchat.com/spotlight/W7_abc123";
  for (body, expected) in [
    (
      r#"<meta property="og:url" content="https://www.snapchat.com/@nasa/spotlight/W7_abc123">"#,
      "https://www.snapchat.com/@nasa/spotlight/W7_abc123",
    ),
    (
      r#"<meta content="https://www.snapchat.com/@nasa/spotlight/other" property="og:url">"#,
      URL,
    ),
    (
      r#"<meta property="og:url" content="https://evil.test/@nasa/spotlight/W7_abc123">"#,
      URL,
    ),
    (
      r#"<script>const decoy = '<meta property="og:url" content="https://www.snapchat.com/@decoy/spotlight/W7_abc123">';</script>"#,
      URL,
    ),
    (
      r#"<meta property="og:url" content="https://www.snapchat.com/@nasa/spotlight/W7_abc123"><meta property="og:url" content="https://www.snapchat.com/@other/spotlight/W7_abc123">"#,
      URL,
    ),
    ("<html>Unavailable</html>", URL),
  ] {
    let fake = FakeTransport(std::sync::Mutex::new(
      [(URL, reply(200, None, body.as_bytes()))].into(),
    ));
    assert_eq!(
      read_site(URL, &fake).await.resolution(URL),
      Resolution::Passthrough(expected.into())
    );
  }
  // Already canonical, highlights and lookalike hosts stay on yt-dlp without a preflight.
  for url in [
    "https://www.snapchat.com/@nasa/spotlight/W7_abc123",
    "https://www.snapchat.com/@nasa/highlight/123",
    "https://snapchat.com.evil.test/spotlight/W7_abc123",
  ] {
    let fake = FakeTransport(std::sync::Mutex::new(Default::default()));
    assert_eq!(
      read_site(url, &fake).await.resolution(url),
      Resolution::Passthrough(url.into())
    );
  }
}

// A Threads video must resolve through the same site dispatch used by metadata/download.
#[tokio::test]
async fn threads_video_uses_the_largest_progressive_variant() {
  let html = include_str!("../../tests/fixtures/threads/video.html");
  let transport = FakeTransport(std::sync::Mutex::new(
    [(
      "https://www.threads.com/@synthetic_user/post/SynthVid0001",
      reply(200, None, html.as_bytes()),
    )]
    .into(),
  ));
  assert_eq!(
    read_site(
      "https://www.threads.net/@synthetic_user/post/SynthVid0001",
      &transport
    )
    .await
    .resolution("original"),
    Resolution::Passthrough(
      "https://instagram.fexample.fna.fbcdn.net/o1/v/t2/f2/synthetic_large.mp4".into()
    )
  );
}

// The shortlink's accepted video hop is enough: do not fetch its login-limited page.
#[tokio::test]
async fn facebook_video_hop_resolves_before_reading_the_landing_page() {
  let hop: serde_json::Value = serde_json::from_str(include_str!(
    "../../tests/fixtures/facebook/share-video-hop.json"
  ))
  .unwrap();
  let short = "https://www.facebook.com/share/v/1DJM2xDCFg/";
  let transport = FakeTransport(std::sync::Mutex::new(
    [
      (
        short,
        reply(
          hop["status"].as_u64().unwrap() as u16,
          hop["location"].as_str(),
          b"",
        ),
      ),
      (
        "https://www.facebook.com/BrockIndependentSchoolDistrict/videos/867018935385508/",
        reply(400, None, b"login-limited"),
      ),
    ]
    .into(),
  ));
  assert_eq!(
    read_site(short, &transport).await.resolution(short),
    Resolution::Passthrough("https://www.facebook.com/watch/?v=867018935385508".into())
  );
  assert_eq!(
    transport.0.lock().unwrap().len(),
    1,
    "landing page must remain unrequested"
  );
}

// Saved NASA post uses media_type 19; only its own explicit inline media is admitted.
#[test]
fn threads_reads_the_bound_posts_linked_inline_video() {
  let page = include_str!("../../tests/fixtures/threads/linked-inline-video.html");
  assert_eq!(
    threads::parse(page, "DXVbTUPl-Qy")
      .and_then(|p| p.video)
      .as_deref(),
    Some("https://scontent.cdninstagram.com/v/inline.mp4")
  );
}

// These recorded post roots are photos; crawler og:type=video.other must not override them.
#[test]
fn facebook_bound_photos_win_over_misleading_crawler_video_metadata() {
  for (file, url) in [
    (
      "fb-government-photo.html",
      "https://www.facebook.com/governostp/posts/830881617425000",
    ),
    (
      "fb-ministry-photo.html",
      "https://www.facebook.com/mic.gov.sl/posts/616512838936732",
    ),
  ] {
    assert_eq!(
      facebook_read(&fixture(file), url),
      Some(Resolution::Unsupported {
        reason: Reason::PhotosOnly
      })
    );
  }
}

#[test]
fn threads_identity_and_fixture_binding_fail_closed() {
  for input in [
    "https://threads.net/@nasa/post/DXVbTUPl-Qy?x=1",
    "https://www.threads.com/%40nasa/post/DXVbTUPl-Qy/media",
  ] {
    assert_eq!(
      threads::Identity::parse(input).unwrap().url.as_str(),
      "https://www.threads.com/@nasa/post/DXVbTUPl-Qy"
    );
  }
  for input in [
    "http://threads.com/@nasa/post/abc",
    "https://user@threads.com/@nasa/post/abc",
    "https://threads.com:8443/@nasa/post/abc",
    "https://threads.com.evil.example/@nasa/post/abc",
    "https://threads.com/@.nasa/post/abc",
    "https://threads.com/@nasa",
    "https://threads.com/@nasa/post/abc#fragment",
  ] {
    assert!(threads::Identity::parse(input).is_none(), "{input}");
  }
  for file in [
    "media-null",
    "duplicate-code",
    "wrong-code",
    "missing-preloader",
    "truncated",
  ] {
    let path = format!(
      "{}/tests/fixtures/threads/{file}.html",
      env!("CARGO_MANIFEST_DIR")
    );
    assert!(
      threads::parse(&std::fs::read_to_string(path).unwrap(), "SynthPost01").is_none(),
      "{file}"
    );
  }
}

// Missing the attachment branch loses images/mixed media to yt-dlp's Unsupported URL.
#[tokio::test]
async fn threads_images_and_mixed_carousels_reuse_the_attachment_model() {
  for (fixture, code, kind, count) in [
    (
      include_str!("../../tests/fixtures/threads/image.html"),
      "SynthPost01",
      "image",
      1,
    ),
    (
      include_str!("../../tests/fixtures/threads/carousel-images.html"),
      "SynthCarr01",
      "carousel",
      2,
    ),
    (
      include_str!("../../tests/fixtures/threads/carousel-mixed.html"),
      "SynthCarr01",
      "mixed",
      4,
    ),
  ] {
    let requested: &'static str =
      Box::leak(format!("https://www.threads.com/@synthetic_user/post/{code}").into_boxed_str());
    let transport = FakeTransport(std::sync::Mutex::new(
      [(requested, reply(200, None, fixture.as_bytes()))].into(),
    ));
    let Resolution::Attachments { metadata } =
      read_site(requested, &transport).await.resolution(requested)
    else {
      panic!("expected attachment metadata for {kind}");
    };
    let media: crate::models::parsed::ParsedSingleVideo = serde_json::from_str(&metadata).unwrap();
    assert_eq!(media.attachments.len(), count);
    let value = serde_json::to_value(&media).unwrap();
    assert_eq!(value["mediaKind"], kind);
    assert_eq!(media.title.as_deref(), Some("Synthetic caption"));
    assert_eq!(media.uploader.as_deref(), Some("synthetic_user"));
    assert_eq!(
      value["attachments"][0]["url"],
      if kind == "image" {
        "https://scontent.cdninstagram.com/v/t51/synthetic_full.jpg?oe=6F000000"
      } else {
        "https://scontent.cdninstagram.com/v/t51/slide_a.jpg?oe=6F000000"
      }
    );
    if kind == "mixed" {
      assert_eq!(value["attachments"][1]["kind"], "video");
      assert_eq!(
        value["attachments"][1]["url"],
        "https://instagram.fexample.fna.fbcdn.net/o1/v/t2/f2/clip_large.mp4"
      );
      assert_eq!(value["attachments"][2]["kind"], "image");
      assert_eq!(value["attachments"][3]["kind"], "video");
    }
  }
}

#[tokio::test]
async fn threads_refuses_redirects_oversized_bodies_and_unbound_previews() {
  let url = "https://www.threads.com/@nasa/post/DXVbTUPl-Qy";
  for response in [
    reply(302, Some("https://www.threads.com/login"), b""),
    reply(200, None, &vec![b'x'; 3 * 512 * 1024 + 1]),
    reply(
      200,
      None,
      b"<meta property='og:video' content='https://scontent.cdninstagram.com/decoy.mp4'>",
    ),
  ] {
    let transport = FakeTransport(std::sync::Mutex::new([(url, response)].into()));
    assert!(matches!(read_site(url, &transport).await, Outcome::Unknown));
    assert!(transport.0.lock().unwrap().is_empty());
  }
  assert_eq!(
    after_unsupported(url, "ERROR: Unsupported URL:").await,
    None
  );
  let video = include_str!("../../tests/fixtures/threads/video.html");
  for replacement in [
    "http://scontent.cdninstagram.com/bad.mp4",
    "https://scontent.cdninstagram.com.evil.example/bad.mp4",
    "https://127.0.0.1/private.mp4",
    "https://user@scontent.cdninstagram.com/bad.mp4",
  ] {
    let page = video
      .replace(
        "https://instagram.fexample.fna.fbcdn.net/o1/v/t2/f2/synthetic_small.mp4",
        replacement,
      )
      .replace(
        "https://instagram.fexample.fna.fbcdn.net/o1/v/t2/f2/synthetic_large.mp4",
        replacement,
      );
    assert!(threads::parse(&page, "SynthVid0001").is_none());
  }
}

// A syntactically admitted CDN name may still resolve into a private address.
#[tokio::test]
async fn threads_refuses_media_when_cdn_dns_is_not_public() {
  struct RefusingCdn(FakeTransport);
  impl http::Transport for RefusingCdn {
    async fn get(
      &self,
      url: &reqwest::Url,
      cap: usize,
    ) -> Result<http::Response, http::FetchError> {
      http::Transport::get(&self.0, url, cap).await
    }
    async fn admit_media(&self, _url: &reqwest::Url) -> Result<(), http::FetchError> {
      Err(http::FetchError::Address)
    }
  }
  for (url, html) in [
    (
      "https://www.threads.com/@synthetic_user/post/SynthVid0001",
      include_str!("../../tests/fixtures/threads/video.html"),
    ),
    (
      "https://www.threads.com/@synthetic_user/post/SynthPost01",
      include_str!("../../tests/fixtures/threads/image.html"),
    ),
  ] {
    let transport = RefusingCdn(FakeTransport(std::sync::Mutex::new(
      [(url, reply(200, None, html.as_bytes()))].into(),
    )));
    assert!(matches!(read_site(url, &transport).await, Outcome::Unknown));
  }
}
