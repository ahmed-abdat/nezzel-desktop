use super::*;
use std::sync::Mutex;

const PAGE: &str = "https://news.example/article";
fn fixture(name: &str) -> String {
  std::fs::read_to_string(format!(
    "{}/tests/fixtures/probe/{name}",
    env!("CARGO_MANIFEST_DIR")
  ))
  .unwrap()
}
fn response(mime: &str, body: &str) -> http::Response {
  http::Response {
    status: 200,
    location: None,
    content_type: Some(mime.into()),
    content_length: Some(body.len().max(1) as u64),
    body: body.as_bytes().into(),
  }
}
struct Fake {
  key: &'static str,
  replies: HashMap<(String, String), http::Response>,
  calls: Mutex<Vec<(String, String)>>,
}
impl Fake {
  fn new(body: &str) -> Self {
    Self {
      key: "public",
      replies: HashMap::from([(("GET".into(), PAGE.into()), response("text/html", body))]),
      calls: Mutex::default(),
    }
  }
  fn with(mut self, method: &str, url: &str, reply: http::Response) -> Self {
    self.replies.insert((method.into(), url.into()), reply);
    self
  }
  fn request(&self, method: &str, url: &Url) -> Result<http::Response, http::FetchError> {
    let key = (method.into(), url.to_string());
    self.calls.lock().unwrap().push(key.clone());
    self
      .replies
      .get(&key)
      .cloned()
      .ok_or(http::FetchError::Status)
  }
}
impl http::Transport for Fake {
  fn cache_key(&self) -> &str {
    self.key
  }
  async fn admit_media(&self, url: &Url) -> Result<(), http::FetchError> {
    let host = url.host_str().unwrap().trim_matches(['[', ']']);
    if host == "localhost"
      || host
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| !http::public_address(ip))
    {
      Err(http::FetchError::Address)
    } else {
      Ok(())
    }
  }
  async fn get(&self, url: &Url, _cap: usize) -> Result<http::Response, http::FetchError> {
    self.request("GET", url)
  }
  async fn head(&self, url: &Url) -> Result<http::Response, http::FetchError> {
    self.request("HEAD", url)
  }
  async fn range(&self, url: &Url, _cap: usize) -> Result<http::Response, http::FetchError> {
    self.request("RANGE", url)
  }
}

#[test]
fn inline_manifest_is_extracted_without_segments() {
  let signals = parse(&fixture("inline.html"), &Url::parse(PAGE).unwrap());
  assert_eq!(
    signals.streams,
    vec![Url::parse("https://cdn.example/master.m3u8?token=x").unwrap()]
  );
}

#[tokio::test]
async fn fixture_ladder_hands_only_verified_candidates_to_engines() {
  for (name, candidates, expected) in [
    (
      "news.html",
      vec![("https://cdn.example/news.mp4?x=1&y=2", "video/mp4")],
      Some("https://cdn.example/news.mp4?x=1&y=2"),
    ),
    (
      "json-ld.html",
      vec![
        ("https://news.example/interview.mp4", "video/mp4"),
        ("https://news.example/interview.mp3", "audio/mpeg"),
        ("https://player.vimeo.com/video/123", "text/html"),
      ],
      Some("https://news.example/interview.mp4"),
    ),
    (
      "embed.html",
      vec![("https://www.youtube.com/embed/dQw4w9WgXcQ", "text/html")],
      Some("https://www.youtube.com/embed/dQw4w9WgXcQ"),
    ),
    (
      "inline.html",
      vec![(
        "https://cdn.example/master.m3u8?token=x",
        "application/vnd.apple.mpegurl",
      )],
      Some("https://cdn.example/master.m3u8?token=x"),
    ),
    ("login.html", vec![], None),
  ] {
    let mut fake = Fake::new(&fixture(name));
    for (url, mime) in candidates {
      fake = fake.with("HEAD", url, response(mime, "media"));
      if http::is_manifest(Some(mime)) {
        fake = fake.with(
          "RANGE",
          url,
          response(mime, "#EXTM3U\n#EXTINF:1,\nchunk.ts"),
        );
      }
    }
    let signals = PageProbe::default()
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .unwrap();
    assert_eq!(
      signals.resolution(),
      expected.map(|s| Resolution::Passthrough(s.into())),
      "{name}"
    );
    if name == "embed.html" {
      assert_eq!(signals.embeds.len(), 1);
      assert_eq!(
        fake.calls.lock().unwrap().as_slice(),
        [
          ("GET".into(), PAGE.into()),
          ("HEAD".into(), expected.unwrap().into())
        ]
      );
    }
  }
}

#[tokio::test]
async fn srcset_gallery_keeps_largest_images_in_picker_order() {
  let fake = Fake::new(&fixture("gallery.html"))
    .with(
      "HEAD",
      "https://news.example/one-large.jpg",
      response("image/jpeg", "image"),
    )
    .with(
      "HEAD",
      "https://news.example/two-large.jpg",
      response("image/jpeg", "image"),
    );
  let signals = PageProbe::default()
    .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
    .await
    .unwrap();
  assert_eq!(signals.image_count, 2);
  let Some(Resolution::Attachments { metadata }) = signals.resolution() else {
    panic!("missing gallery");
  };
  let parsed: crate::models::ParsedSingleVideo = serde_json::from_str(&metadata).unwrap();
  assert_eq!(
    parsed
      .attachments
      .iter()
      .map(|m| m.url.as_str())
      .collect::<Vec<_>>(),
    [
      "https://news.example/one-large.jpg",
      "https://news.example/two-large.jpg"
    ]
  );
}

#[tokio::test]
async fn cache_coalesces_hits_and_separates_url_auth_and_transport() {
  let probe = PageProbe::default();
  let fake = Fake::new("").with(
    "GET",
    "https://news.example/other",
    response("text/html", ""),
  );
  let (a, b) = tokio::join!(
    probe.page_signals(PAGE, ProbeAuth::Anonymous, &fake),
    probe.page_signals(PAGE, ProbeAuth::Anonymous, &fake)
  );
  assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
  probe
    .page_signals("https://news.example/other", ProbeAuth::Anonymous, &fake)
    .await
    .unwrap();
  probe
    .page_signals(PAGE, ProbeAuth::Context(1), &fake)
    .await
    .unwrap();
  probe
    .page_signals(PAGE, ProbeAuth::Context(2), &fake)
    .await
    .unwrap();
  let other = Fake {
    key: "browser-headers",
    ..Fake::new("")
  };
  probe
    .page_signals(PAGE, ProbeAuth::Anonymous, &other)
    .await
    .unwrap();
  assert_eq!(
    (
      fake.calls.lock().unwrap().len(),
      other.calls.lock().unwrap().len()
    ),
    (4, 1)
  );
  for _ in 0..2 {
    assert!(probe
      .page_signals("https://news.example/missing", ProbeAuth::Anonymous, &fake)
      .await
      .is_err());
  }
  assert_eq!(fake.calls.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn admission_rejects_private_hosts_and_redirects_before_verification() {
  for target in [
    "https://127.0.0.1/media.mp4",
    "https://10.0.0.1/media.mp4",
    "https://[::1]/media.mp4",
    "https://localhost/media.mp4",
    "https://user:secret@cdn.example/media.mp4",
    "http://cdn.example/media.mp4",
  ] {
    let fake = Fake::new(&format!("<video src='{target}'>"));
    let signals = PageProbe::default()
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .unwrap();
    assert!(signals.resolution().is_none(), "{target}");
    assert_eq!(fake.calls.lock().unwrap().len(), 1, "{target}");
  }
  let mut redirect = response("video/mp4", "");
  redirect.status = 302;
  redirect.location = Some("https://127.0.0.1/media.mp4".into());
  let fake =
    Fake::new("<video src='/video.mp4'>").with("HEAD", "https://news.example/video.mp4", redirect);
  let signals = PageProbe::default()
    .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
    .await
    .unwrap();
  assert!(signals.resolution().is_none());
  assert_eq!(fake.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn verification_rejects_html_and_empty_bodies_and_falls_back_when_head_is_unsupported() {
  for (mime, length, status, expected) in [
    ("text/html", 100, 200, false),
    ("video/mp4", 0, 200, false),
    ("video/mp4", 100, 405, true),
  ] {
    let mut head = response(mime, "");
    head.status = status;
    head.content_length = Some(length);
    let fake = Fake::new("<video src='/video.mp4'>")
      .with("HEAD", "https://news.example/video.mp4", head)
      .with(
        "RANGE",
        "https://news.example/video.mp4",
        response("video/mp4", "bytes"),
      );
    let signals = PageProbe::default()
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .unwrap();
    assert_eq!(
      signals.resolution().is_some(),
      expected,
      "{mime} {length} {status}"
    );
  }
}

#[tokio::test]
async fn drm_manifests_stop_before_any_candidate_is_handed_off() {
  for (name, mime) in [
    ("protected.m3u8", "application/vnd.apple.mpegurl"),
    ("protected.mpd", "application/dash+xml"),
  ] {
    let fake = Fake::new("<video src='/manifest'>")
      .with(
        "HEAD",
        "https://news.example/manifest",
        response(mime, "manifest"),
      )
      .with(
        "RANGE",
        "https://news.example/manifest",
        response(mime, &fixture(name)),
      );
    let signals = PageProbe::default()
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .unwrap();
    assert!(signals.drm_protected, "{name}");
    assert!(signals.resolution().is_none());
  }
  assert!(!drm(b"#EXT-X-KEY:METHOD=AES-128,URI=\"key\""));
}

#[test]
fn metadata_signals_cover_audio_and_login_without_treating_article_covers_as_galleries() {
  let page = Url::parse(PAGE).unwrap();
  let signals = parse(
    "<meta property='og:audio' content='/audio.mp3'><audio><source src='/track.mp3'></audio>",
    &page,
  );
  assert_eq!((signals.og.audio.len(), signals.tags.len()), (1, 1));
  assert!(parse(&fixture("login.html"), &page).login_wall);
  for body in [
    "<meta property='og:type' content='article'><meta property='og:image' content='/cover.jpg'>",
    "<meta property='og:type' content='image'><meta property='og:image' content='/avatar.jpg'>",
    "<meta property='og:type' content='image'><meta property='og:image' content='/cover.jpg'><meta property='og:image:width' content='100'>",
  ] {
  assert!(parse(
    body,
    &page
  )
  .candidates()
  .is_empty());
  }
}

#[tokio::test]
async fn reader_redirects_do_not_turn_foreign_attachment_metadata_into_generic_media() {
  for (url, fixture_path) in [
    (
      "https://www.facebook.com/story.php?story_fbid=900000000000001&id=42",
      "facebook/video-decoys-post.html",
    ),
    (
      "https://www.reddit.com/r/test/comments/abc123/title/",
      "reddit/posts.json",
    ),
  ] {
    let body = std::fs::read_to_string(format!(
      "{}/tests/fixtures/{fixture_path}",
      env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut redirect = response("text/html", "");
    redirect.status = 302;
    redirect.location = Some(url.into());
    let fake =
      Fake::new("")
        .with("GET", PAGE, redirect)
        .with("GET", url, response("text/html", &body));
    assert!(PageProbe::default()
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .is_err());
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
  }
}

#[tokio::test]
async fn direct_content_types_are_verified_before_handoff() {
  for (mime, kind) in [
    ("video/mp4", DirectKind::Video),
    ("audio/mpeg", DirectKind::Audio),
    ("image/jpeg", DirectKind::Image),
    ("application/pdf", DirectKind::File),
  ] {
    let fake = Fake::new("")
      .with("GET", PAGE, response(mime, "media"))
      .with("HEAD", PAGE, response(mime, "media"));
    let signals = PageProbe::default()
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .unwrap();
    assert!(signals.direct == Some(kind), "{mime}");
    assert!(signals.resolution().is_some(), "{mime}");
    assert_eq!(
      fake.calls.lock().unwrap().as_slice(),
      [("GET".into(), PAGE.into()), ("HEAD".into(), PAGE.into())]
    );
  }
}

#[tokio::test]
async fn frontend_handoff_preserves_unique_id_and_does_not_probe_embeds_of_embeds() {
  let fake =
    Fake::new(r#"<script>player.src=`https://www.youtube-nocookie.com/embed/${d.vid}`</script>"#)
      .with(
        "HEAD",
        "https://www.youtube.com/embed/dQw4w9WgXcQ",
        response(
          "text/html",
          "<iframe src='https://player.vimeo.com/video/123'></iframe>",
        ),
      );
  // A frontend may supply its ID in the page URL while its script holds only a template.
  let mut fake = fake;
  let url = "https://news.example/watch/dQw4w9WgXcQ";
  let page = fake.replies.remove(&("GET".into(), PAGE.into())).unwrap();
  fake.replies.insert(("GET".into(), url.into()), page);
  let signals = PageProbe::default()
    .page_signals(url, ProbeAuth::Anonymous, &fake)
    .await
    .unwrap();
  assert_eq!(
    signals.resolution(),
    Some(Resolution::Passthrough(
      "https://www.youtube.com/embed/dQw4w9WgXcQ".into()
    ))
  );
  assert_eq!(fake.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn public_media_wins_over_login_chrome_and_strong_wall_titles() {
  let chrome = concat!(
    "<header><form class='login-dropdown' action='/login'>",
    "<input type='password'></form></header><div class='auth-wall'></div>"
  );
  for wall in [
    chrome,
    "<meta property='og:title' content='Log in'><title>Sign in</title>",
  ] {
    let fake = Fake::new(&format!("{wall}<video src='/public.mp4'></video>")).with(
      "HEAD",
      "https://news.example/public.mp4",
      response("video/mp4", "media"),
    );
    let probe = PageProbe::default();
    let signals = probe
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .unwrap();
    let expected = Some(Resolution::Passthrough(
      "https://news.example/public.mp4".into(),
    ));
    assert_eq!(signals.resolution(), expected);
    assert_eq!(
      crate::extract::chain::scan_page(&probe, PAGE, &fake).await,
      Ok(expected)
    );
  }
  let signals = parse(
    "<form id='login-dropdown'><input type='password'></form><div class='login-wall'></div>",
    &Url::parse(PAGE).unwrap(),
  );
  assert!(!signals.login_wall);
  let fake = Fake::new(&fixture("login.html"));
  assert_eq!(
    crate::extract::chain::scan_page(&PageProbe::default(), PAGE, &fake).await,
    Err("signInRequired")
  );
}

#[tokio::test(start_paused = true)]
async fn network_failures_are_retried_instead_of_memoized() {
  struct Recovering {
    calls: std::sync::atomic::AtomicUsize,
    timeout: bool,
  }
  impl http::Transport for Recovering {
    async fn get(&self, _url: &Url, _cap: usize) -> Result<http::Response, http::FetchError> {
      if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
        if self.timeout {
          std::future::pending::<()>().await;
        }
        Err(http::FetchError::Network)
      } else {
        Ok(response("text/html", ""))
      }
    }
  }
  for timeout in [false, true] {
    let fake = Recovering {
      calls: std::sync::atomic::AtomicUsize::new(0),
      timeout,
    };
    let probe = PageProbe::default();
    assert!(matches!(
      probe.page_signals(PAGE, ProbeAuth::Anonymous, &fake).await,
      Err(http::FetchError::Network)
    ));
    assert!(probe
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .is_ok());
    assert!(probe
      .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
      .await
      .is_ok());
    assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
  }
}

#[tokio::test]
async fn octet_stream_video_extensions_verify_and_manifests_still_check_drm() {
  for extension in ["mp4", "webm", "m3u8", "mpd"] {
    for protected in [false, true]
      .into_iter()
      .take(if matches!(extension, "m3u8" | "mpd") {
        2
      } else {
        1
      })
    {
      let url = format!("https://cdn.example/media.{extension}?token=ok");
      let body = if protected {
        fixture(&format!("protected.{extension}"))
      } else if extension == "m3u8" {
        "#EXTM3U\n#EXTINF:1,\nchunk.ts".into()
      } else if extension == "mpd" {
        "<MPD><Period/></MPD>".into()
      } else {
        "video".into()
      };
      let fake = Fake::new(&format!("<video src='{url}'>"))
        .with("HEAD", &url, response("application/octet-stream", "media"))
        .with("RANGE", &url, response("application/octet-stream", &body));
      let signals = PageProbe::default()
        .page_signals(PAGE, ProbeAuth::Anonymous, &fake)
        .await
        .unwrap();
      assert_eq!(signals.drm_protected, protected, "{extension}");
      assert_eq!(signals.resolution().is_some(), !protected, "{extension}");
    }
  }
}

use crate::resolve::html::generic_fixtures;

#[tokio::test]
async fn page_scan_retains_legacy_generic_fixture_results() {
  let cases = generic_fixtures::videos()
    .into_iter()
    .map(|(html, expected)| (html, "https://example.com/posts/one".to_string(), expected))
    .chain(
      generic_fixtures::frontends()
        .into_iter()
        .map(|(html, page, expected)| (html, page.to_string(), expected)),
    )
    .chain(generic_fixtures::SLOTS.iter().map(|path| {
      (
        "<script>const host = 'youtube.com/embed/';</script>",
        format!("https://example.com{path}"),
        Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
      )
    }));
  for (html, page, expected) in cases {
    // The same YouTube video goes to its player endpoint for the one-hop handoff.
    let candidate = expected.map(|raw| {
      let url = Url::parse(raw).unwrap();
      if url.host_str() == Some("www.youtube.com") && url.path() == "/watch" {
        let (_, id) = url.query_pairs().find(|(key, _)| key == "v").unwrap();
        format!("https://www.youtube.com/embed/{id}")
      } else {
        raw.into()
      }
    });
    let mut fake = Fake::new("").with("GET", &page, response("text/html", html));
    if let Some(target) = &candidate {
      let mime = if target.contains("youtube.com/embed/") {
        "text/html"
      } else if target.contains(".webm") {
        "video/webm"
      } else {
        "video/mp4"
      };
      fake = fake.with("HEAD", target, response(mime, "media"));
    }
    assert_eq!(
      crate::extract::chain::scan_page(&PageProbe::default(), &page, &fake).await,
      Ok(candidate.map(Resolution::Passthrough)),
      "{page}: {html}"
    );
  }
}
