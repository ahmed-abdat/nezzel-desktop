use super::{
  names::suggested_name,
  probe::{classify, Kind},
};
use reqwest::Url;

#[test]
fn content_disposition_names_are_safe_and_preserve_arabic() {
  let url = Url::parse("https://files.example/report%20one.pdf").unwrap();
  for (header, expected) in [
    (None, "report one.pdf"),
    (
      Some("attachment; filename=x.nezzel-state"),
      "x.nezzel-state.download",
    ),
    (
      Some("attachment; filename=x.NEZZEL-PART"),
      "x.NEZZEL-PART.download",
    ),
    (
      Some("attachment; filename=x.nezzel-state.123.tmp"),
      "x.nezzel-state.123.tmp.download",
    ),
    (
      Some("attachment; filename=plain.pdf; filename*=UTF-8''%D8%AA%D9%82%D8%B1%D9%8A%D8%B1.pdf"),
      "تقرير.pdf",
    ),
    (
      Some("attachment; filename*=ISO-8859-1''caf%E9.pdf"),
      "café.pdf",
    ),
    (Some("attachment; filename=\"../secret.pdf\""), "secret.pdf"),
    (
      Some("attachment; filename=\"C:\\tmp\\NUL.txt\""),
      "_NUL.txt",
    ),
    (
      Some("attachment; filename=\"report.pdf:Zone.Identifier. \""),
      "report.pdf_Zone.Identifier",
    ),
    (
      Some("attachment; filename=\"invoice\u{202e}cod.exe\""),
      "invoicecod.exe",
    ),
    (Some("attachment; filename=\"a;b.pdf\""), "a;b.pdf"),
    (
      Some("attachment; filename*=bogus''bad; filename=good.pdf"),
      "good.pdf",
    ),
    (Some("attachment; filename=\"%D8%AA.pdf\""), "ت.pdf"),
    (
      Some("attachment; filename=\"=?UTF-8?B?2KoucGRm?=\""),
      "ت.pdf",
    ),
  ] {
    assert_eq!(
      suggested_name(header.map(str::as_bytes), &url, "application/pdf"),
      expected,
      "{header:?}"
    );
  }
}

#[test]
fn routing_keeps_media_pages_and_manifests_out_of_the_file_engine() {
  for (path, mime, header, expected) in [
    ("a.zip", "application/zip", None, Kind::File),
    ("get?id=1", "application/octet-stream", None, Kind::Unknown),
    ("watch?id=1", "", None, Kind::Unknown),
    ("view", "application/pdf", None, Kind::File),
    ("view", "application/json", None, Kind::Unknown),
    ("pdf", "", None, Kind::Unknown),
    (
      "view",
      "",
      Some("inline; filename=index.pdf"),
      Kind::Unknown,
    ),
    ("view", "", Some("filename=index.pdf"), Kind::Unknown),
    ("report.%70df", "application/octet-stream", None, Kind::File),
    (
      "/watch.foo",
      "application/octet-stream",
      None,
      Kind::Unknown,
    ),
    ("report.pdf", "text/html; charset=utf-8", None, Kind::Page),
    (
      "view",
      "text/html",
      Some("attachment; filename=page.html"),
      Kind::File,
    ),
    ("view", "application/xhtml+xml", None, Kind::Page),
    (
      "view",
      "text/html",
      Some("inline; filename=\"index.html\""),
      Kind::Page,
    ),
    ("a.pdf", "", None, Kind::File),
    ("photo.jpg", "image/jpeg", None, Kind::File),
    ("a.mp4", "video/mp4", None, Kind::Media),
    ("a.mp3", "audio/mpeg", None, Kind::Media),
    ("live", "application/dash+xml", None, Kind::Media),
    ("live.m3u8", "text/plain", None, Kind::Media),
    ("feed", "application/rss+xml", None, Kind::Media),
  ] {
    assert_eq!(
      classify(
        &Url::parse(&format!("https://files.example/{path}")).unwrap(),
        mime,
        header.map(str::as_bytes)
      ),
      expected,
      "{path} {mime}"
    );
  }
}

#[test]
fn only_unclaimed_file_routes_are_probed() {
  assert!(!super::probe::should_probe_metadata(
    "https://files.example/a.zip",
    2,
    false
  ));
  assert!(!super::probe::should_probe_metadata(
    "https://files.example/a.zip",
    1,
    true
  ));
  assert!(!super::probe::should_probe_metadata(
    "https://files.example/download?token=secret",
    1,
    false
  ));
  for (url, expected) in [
    (
      "https://github.com/org/app/releases/download/v1/app.dmg",
      true,
    ),
    ("http://files.example/a.zip", true),
    ("https://files.example/download?token=secret", true),
    ("https://files.example/a.jpg", true),
    ("https://i.redd.it/abc123.jpg", true),
    ("https://preview.redd.it/a.jpg?width=640&s=key", true),
    ("https://external-preview.redd.it/a.jpg?s=key", true),
    ("https://preview.redd.it/a", false),
    ("https://i.imgur.com/abc123.jpg", true),
    ("https://www.youtube.com/watch?v=1", false),
    ("https://www.kwai.com/@nasa/video/123456789", false),
    ("https://x.com/user/status/123", false),
    ("https://www.linkedin.com/posts/example", false),
    ("https://drive.google.com/file/d/id/view", false),
    ("https://www.dropbox.com/scl/fi/id/a.zip", false),
    ("https://files.example/live.m3u8?token=secret", false),
    ("https://files.example/a.mp4", false),
    ("ftp://files.example/a.zip", false),
  ] {
    assert_eq!(super::probe::should_probe(url), expected, "{url}");
  }
}

#[tokio::test]
async fn public_transport_refuses_private_literal_destinations_before_connecting() {
  let engine = super::engine::Engine::new().unwrap();
  let (_tx, rx) = tokio::sync::watch::channel(true);
  for url in [
    "http://127.0.0.1/a.zip",
    "https://169.254.169.254/a.zip",
    "https://[::ffff:127.0.0.1]/a.zip",
    "https://[64:ff9b::a00:1]/a.zip",
    "https://user:password@files.example/a.zip",
  ] {
    assert!(matches!(
      engine.probe(url, rx.clone()).await,
      Err(super::Error::Address)
    ));
  }
}

#[test]
fn sidecars_reject_corruption_links_and_out_of_bounds_segments() {
  use super::{
    plan::{Segment, Validator},
    state,
  };
  let dir = std::env::temp_dir().join(format!("nezzel-state-{}", uuid::Uuid::new_v4()));
  std::fs::create_dir(&dir).unwrap();
  let key = super::auth::StateKey::ephemeral();
  let mut target = state::target(
    &dir,
    &state::identity("https://files.example/a.zip"),
    "a.zip",
    &key,
  )
  .unwrap();
  target.part.set_len(128).unwrap();
  let mut saved = state::State::new(
    state::identity("https://files.example/a.zip"),
    "a.zip".into(),
    Some(128),
    true,
    Validator::default(),
    vec![Segment {
      start: 0,
      end: Some(128),
      done: 64,
    }],
  );
  saved.save(&target.state_path, &target.part, &key).unwrap();
  assert_eq!(
    state::State::load(&target.state_path, &key)
      .unwrap()
      .bytes(),
    64
  );
  #[cfg(unix)]
  {
    use std::os::unix::fs::{symlink, PermissionsExt};
    assert_eq!(
      target.part.metadata().unwrap().permissions().mode() & 0o777,
      0o600
    );
    let hard = dir.join("hard-state");
    std::fs::hard_link(&target.state_path, &hard).unwrap();
    assert!(state::State::load(&target.state_path, &key).is_err());
    std::fs::remove_file(hard).unwrap();
    let link = dir.join("state-link");
    symlink(&target.state_path, &link).unwrap();
    assert!(state::State::load(&link, &key).is_err());
    let part_link = dir.join("part-link");
    symlink(&target.part_path, &part_link).unwrap();
    assert!(state::regular(&part_link, true).is_err());
  }
  saved.segments[0].done = 129;
  saved.save(&target.state_path, &target.part, &key).unwrap();
  assert!(state::State::load(&target.state_path, &key).is_err());
  std::fs::write(&target.state_path, b"corrupt JSON").unwrap();
  assert!(state::State::load(&target.state_path, &key).is_err());
  std::fs::write(&target.final_path, b"keep existing").unwrap();
  assert!(state::publish(&target.part_path, &target.final_path).is_err());
  assert_eq!(std::fs::read(&target.final_path).unwrap(), b"keep existing");
  assert!(target.part_path.exists());
  target.state = None;
  drop(target);
  std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn filename_limits_keep_arabic_extension_and_safe_joiners() {
  use super::sanitize::sanitize_file_name;
  let name = sanitize_file_name(&format!("{}.pdf", "ت".repeat(180)));
  assert!(name.len() <= 200 && name.ends_with(".pdf"));
  assert_eq!(
    sanitize_file_name("می\u{200c}خواهم.pdf"),
    "می\u{200c}خواهم.pdf"
  );
  assert_eq!(sanitize_file_name(".\u{200c}hidden.exe. "), "hidden.exe");
  assert_eq!(sanitize_file_name("nul .txt"), "_nul .txt");
}

#[test]
fn file_names_reserve_every_windows_device_name_and_keep_state_names_out() {
  use super::sanitize::sanitize_file_name;
  for (raw, expected) in [
    ("CON", "_CON"),
    ("nul.txt", "_nul.txt"),
    ("CONIN$", "_CONIN$"),
    ("COM0.txt", "_COM0.txt"),
    ("COM\u{b9}", "_COM\u{b9}"),
    ("lpt9", "_lpt9"),
    ("LPT10", "LPT10"),
    ("CONSOLE", "CONSOLE"),
    ("a<b>c:d\"e|f?g*h", "a_b_c_d_e_f_g_h"),
    ("../../etc/passwd", "passwd"),
    ("C:\\Users\\me\\clip.mp4", "clip.mp4"),
    ("   ", "download"),
    ("..", "download"),
    ("clip.mp4.nezzel-part", "clip.mp4.nezzel-part.download"),
    ("a\u{202e}gpj.exe", "agpj.exe"),
    ("CONOUT$.mp4", "_CONOUT$.mp4"),
    ("LPT\u{b3}.txt", "_LPT\u{b3}.txt"),
    ("com1 .txt", "_com1 .txt"),
    ("nul.", "_nul"),
    ("nul. .", "_nul"),
    ("COM1.tar.gz", "_COM1.tar.gz"),
    ("COM11", "COM11"),
    ("xCON", "xCON"),
  ] {
    assert_eq!(sanitize_file_name(raw), expected, "{raw:?}");
  }
}

#[test]
fn collision_suffixes_still_fit_the_sidecar_checkpoint_name_budget() {
  let name = super::sanitize::sanitize_file_name(&format!("{}.pdf", "ت".repeat(180)));
  let numbered = super::state::alternative(&name, 1000);
  assert!(numbered.len() <= 200);
  assert!(numbered.ends_with(" (1000).pdf"));
}

#[tokio::test]
async fn encoded_pages_stay_pages_but_encoded_files_are_refused() {
  use super::{engine::Engine, Error};
  use std::sync::Arc;
  for (mime, expected) in [("text/html", true), ("application/pdf", false)] {
    let engine = Engine::scripted(Arc::new(move |_, _| {
      Ok(reqwest::Response::from(
        http::Response::builder()
          .status(200)
          .header("content-type", mime)
          .header("content-encoding", "gzip")
          .body(Vec::<u8>::new())
          .unwrap(),
      ))
    }));
    let (_tx, rx) = tokio::sync::watch::channel(true);
    assert!(
      super::runner::probe_metadata(&engine, "https://files.example/index", rx.clone())
        .await
        .is_none()
    );
    match engine.probe("https://files.example/index", rx).await {
      Ok(p) => assert!(expected && p.kind == Kind::Page),
      Err(e) => assert!(!expected && matches!(e, Error::Encoding)),
    }
  }
}

#[test]
fn public_url_hash_does_not_authenticate_downloaded_state() {
  use super::{
    plan::{Segment, Validator},
    state,
  };
  use sha2::{Digest, Sha256};
  let dir = std::env::temp_dir().join(format!("nezzel-state-forgery-{}", uuid::Uuid::new_v4()));
  std::fs::create_dir(&dir).unwrap();
  let key = super::auth::StateKey::ephemeral();
  let saved = state::State::new(
    state::identity("https://files.example/target.zip"),
    "target.zip".into(),
    Some(10),
    true,
    Validator::default(),
    vec![Segment {
      start: 0,
      end: Some(10),
      done: 10,
    }],
  );
  let bytes = serde_json::to_vec(&saved).unwrap();
  let mut json = serde_json::to_value(&saved).unwrap();
  json["checksum"] = hex::encode(Sha256::digest(bytes)).into();
  let path = dir.join("target.zip.nezzel-state");
  std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
  assert!(
    state::State::load(&path, &key).is_err(),
    "a public SHA-256 checksum must not authenticate resume state"
  );
  std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_or_short_part_does_not_permanently_block_a_url() {
  use super::{
    plan::{Segment, Validator},
    state,
  };
  for missing in [true, false] {
    let dir = std::env::temp_dir().join(format!("nezzel-stale-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let key = super::auth::StateKey::ephemeral();
    let identity = state::identity("https://files.example/a.zip");
    let target = state::target(&dir, &identity, "a.zip", &key).unwrap();
    let saved = state::State::new(
      identity.clone(),
      "a.zip".into(),
      Some(128),
      true,
      Validator::default(),
      vec![Segment {
        start: 0,
        end: Some(128),
        done: 64,
      }],
    );
    saved.save(&target.state_path, &target.part, &key).unwrap();
    assert!(
      matches!(
        state::resume(&dir, &identity, &key),
        Err(super::Error::Busy)
      ),
      "only a live lock should remain Busy"
    );
    let path = target.part_path.clone();
    drop(target);
    if missing {
      std::fs::remove_file(path).unwrap();
    }
    assert!(state::resume(&dir, &identity, &key).unwrap().is_none());
    let fresh = state::target(&dir, &identity, "a.zip", &key).unwrap();
    assert_eq!(fresh.final_path.file_name().unwrap(), "a.zip");
    let reservation = fresh.part_path.clone();
    drop(fresh);
    assert!(
      !reservation.exists(),
      "a reservation cancelled before its first checkpoint must be cleaned"
    );
    std::fs::remove_dir_all(dir).unwrap();
  }
}

#[test]
fn publication_falls_back_without_replacing_or_publishing_reserved_files() {
  use super::state;
  let dir = std::env::temp_dir().join(format!("nezzel-publish-{}", uuid::Uuid::new_v4()));
  std::fs::create_dir(&dir).unwrap();
  let source = dir.join("source.nezzel-part");
  let target = dir.join("target.zip");
  for copy in [false, true] {
    std::fs::write(&source, b"whole file").unwrap();
    state::publish_with(
      &source,
      &target,
      |_, _| Err(std::io::ErrorKind::Unsupported.into()),
      |from, to| {
        if copy {
          Err(std::io::ErrorKind::Unsupported.into())
        } else {
          std::fs::hard_link(from, to)
        }
      },
    )
    .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"whole file");
    assert!(!source.exists());
    std::fs::write(&source, b"do not replace").unwrap();
    assert!(state::publish_with(
      &source,
      &target,
      |_, _| Err(std::io::ErrorKind::Unsupported.into()),
      |_, _| Err(std::io::ErrorKind::Unsupported.into())
    )
    .is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"whole file");
    std::fs::remove_file(&target).unwrap();
  }
  assert!(state::publish(&source, &dir.join("x.nezzel-state")).is_err());
  assert!(source.exists());
  std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn authentication_key_is_private_persistent_and_verification_rejects_another_key() {
  use super::auth::StateKey;
  let dir = std::env::temp_dir().join(format!("nezzel-auth-{}", uuid::Uuid::new_v4()));
  let first = StateKey::load(&dir).unwrap();
  let tag = first.sign(b"checkpoint").unwrap();
  let second = StateKey::load(&dir).unwrap();
  assert!(second.verify(b"checkpoint", &tag).is_ok());
  assert!(StateKey::ephemeral().verify(b"checkpoint", &tag).is_err());
  assert!(second.verify(b"modified", &tag).is_err());
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
      std::fs::metadata(dir.join("resume.key"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777,
      0o600
    );
  }
  std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn private_address_probe_is_inconclusive_for_existing_media_routing() {
  let engine = super::engine::Engine::new().unwrap();
  let (_tx, rx) = tokio::sync::watch::channel(true);
  for url in [
    "https://198.18.0.1/video",
    "https://100.64.0.1/video",
    "http://192.168.1.1/video",
  ] {
    assert!(super::runner::probe_metadata(&engine, url, rx.clone())
      .await
      .is_none());
    assert!(
      matches!(
        engine.probe(url, rx.clone()).await,
        Err(super::Error::Address)
      ),
      "the download boundary must still refuse private addresses"
    );
  }
}

#[test]
fn shared_urls_preserve_access_keys_and_export_formats() {
  use super::hosts::{plan, Export};
  for (input, format, want) in [
    (
      "http://dropbox.com/scl/fi/abc/report.pdf?rlkey=secret&e=1&dl=0",
      Export::Pdf,
      "https://www.dropbox.com/scl/fi/abc/report.pdf?rlkey=secret&e=1&dl=1",
    ),
    (
      "http://drive.google.com/file/d/abc/view",
      Export::Pdf,
      "https://drive.usercontent.google.com/download?id=abc&export=download",
    ),
    (
      "dropbox.com/s/abc/report.pdf?dl=0",
      Export::Pdf,
      "https://www.dropbox.com/s/abc/report.pdf?dl=1",
    ),
    (
      "https://drive.google.com/file/d/abc_123/view?resourcekey=secret",
      Export::Pdf,
      "https://drive.usercontent.google.com/download?id=abc_123&export=download&resourcekey=secret",
    ),
    (
      "https://drive.google.com/file/u/2/d/abc/view",
      Export::Pdf,
      "https://drive.usercontent.google.com/download?id=abc&export=download",
    ),
    (
      "https://drive.google.com/open?id=abc&resourcekey=a%2Bb",
      Export::Pdf,
      "https://drive.usercontent.google.com/download?id=abc&export=download&resourcekey=a%2Bb",
    ),
    (
      "https://docs.google.com/document/d/abc/edit",
      Export::Pdf,
      "https://docs.google.com/document/d/abc/export?format=pdf",
    ),
    (
      "https://docs.google.com/document/d/abc/edit",
      Export::Office,
      "https://docs.google.com/document/d/abc/export?format=docx",
    ),
    (
      "https://docs.google.com/spreadsheets/d/abc/edit#gid=2",
      Export::Office,
      "https://docs.google.com/spreadsheets/d/abc/export?format=xlsx",
    ),
    (
      "https://docs.google.com/presentation/d/abc/edit",
      Export::Office,
      "https://docs.google.com/presentation/d/abc/export/pptx",
    ),
    (
      "https://docs.google.com/presentation/d/abc/edit",
      Export::Pdf,
      "https://docs.google.com/presentation/d/abc/export/pdf",
    ),
    (
      "https://www.dropbox.com/scl/fi/abc/a.zip?rlkey=secret&dl=0&st=xyz",
      Export::Pdf,
      "https://www.dropbox.com/scl/fi/abc/a.zip?rlkey=secret&st=xyz&dl=1",
    ),
    (
      "https://dropbox.com/scl/fo/abc/hash?rlkey=key&dl=0",
      Export::Pdf,
      "https://www.dropbox.com/scl/fo/abc/hash?rlkey=key&dl=1",
    ),
  ] {
    assert_eq!(plan(input, format).unwrap().url.as_str(), want);
  }
  for input in [
    "https://drive.google.com/drive/folders/abc",
    "https://drive.google.com.evil.test/file/d/abc",
    "https://docs.google.com/document/d/e/published/pub",
    "https://example.com/a.zip",
    "https://user:pass@drive.google.com/file/d/abc/view",
    "http://user@dropbox.com/scl/fi/abc/report.pdf?dl=0",
    "http://www.dropbox.com:80/scl/fi/abc/report.pdf?dl=0",
  ] {
    assert!(plan(input, Export::Pdf).is_none(), "{input}");
  }
}

#[test]
fn drive_confirmation_submits_all_hidden_fields_only_from_the_download_form() {
  use super::hosts::{plan, Export};
  let plan = plan(
    "https://drive.google.com/file/d/abc/view?resourcekey=secret",
    Export::Pdf,
  )
  .unwrap();
  let next = plan
    .next(&plan.url, include_str!("fixtures/drive-confirm.html"))
    .unwrap();
  let fields: Vec<_> = next
    .query_pairs()
    .map(|(k, v)| (k.into_owned(), v.into_owned()))
    .collect();
  assert_eq!(
    fields,
    vec![
      ("id".into(), "abc".into()),
      ("export".into(), "download".into()),
      ("confirm".into(), "t".into()),
      ("uuid".into(), "a&b".into()),
      ("at".into(), "token+value".into()),
      ("future".into(), "keep".into()),
      ("resourcekey".into(), "secret".into())
    ]
  );
  for html in ["<form action='https://evil.test/download'><input type=hidden name=id value=abc><input type=hidden name=confirm value=t></form>", "<form action='/download'><input type=hidden name=id value=other><input type=hidden name=confirm value=t></form>", "<form action='/download' method=post><input type=hidden name=id value=abc><input type=hidden name=confirm value=t></form>"] {
    assert!(plan.next(&plan.url, html).is_err());
  }
}

#[test]
fn host_html_rejects_permission_pages_and_only_accepts_dropbox_download_origins() {
  use super::{
    hosts::{plan, Export},
    Error,
  };
  let drive = plan("https://drive.google.com/uc?id=abc", Export::Pdf).unwrap();
  assert!(matches!(
    drive.next(&drive.url, include_str!("fixtures/google-private.html")),
    Err(Error::NotPublic)
  ));
  assert!(matches!(
    drive.next(
      &drive.url,
      "Too many users have viewed or downloaded this file recently"
    ),
    Err(Error::RateLimited)
  ));
  let dropbox = plan(
    "https://www.dropbox.com/scl/fi/abc/test.zip?rlkey=key",
    Export::Pdf,
  )
  .unwrap();
  assert_eq!(
    dropbox
      .next(&dropbox.url, include_str!("fixtures/dropbox-download.html"))
      .unwrap()
      .as_str(),
    "https://uc123.dl.dropboxusercontent.com/cd/0/get/file?token=a&key=b"
  );
  assert!(matches!(
    dropbox.next(&dropbox.url, include_str!("fixtures/dropbox-private.html")),
    Err(Error::NotPublic)
  ));
  assert!(matches!(
    dropbox.next(
      &dropbox.url,
      "<a href='https://evil.dl.dropboxusercontent.com.evil.test/file'>Download</a>"
    ),
    Err(Error::NotFile)
  ));
}

#[tokio::test]
async fn dropbox_folder_is_not_a_single_file() {
  use reqwest::ResponseBuilderExt;
  let engine = super::engine::Engine::scripted(std::sync::Arc::new(|url, _| {
    Ok(
      http::Response::builder()
        .url(Url::parse(url).unwrap())
        .header("content-type", "application/zip")
        .header("content-disposition", "attachment; filename=folder.zip")
        .body(vec![1u8; 2048])
        .unwrap()
        .into(),
    )
  }));
  let (_active, cancel) = tokio::sync::watch::channel(true);
  let result = engine
    .probe(
      "https://dropbox.com/scl/fo/abc/folder?rlkey=secret&dl=0",
      cancel,
    )
    .await;
  assert_eq!(result.err().map(|error| error.code()), Some("fileNotFile"));
}
