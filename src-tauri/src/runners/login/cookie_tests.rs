//! Real-engine contract tests. Nightly only: Python/sqlite3 and both CLIs are required.
use super::*;
use crate::runners::retry_policy::RetryAction;
use crate::runners::{gallery, gallery_files::Staging};
use crate::state::config_models::AuthSettings;
use std::{path::PathBuf, process::Stdio, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
  url: String,
  accepted: Arc<std::sync::atomic::AtomicUsize>,
  task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
  fn drop(&mut self) {
    self.task.abort();
  }
}
impl Server {
  async fn start() -> Self {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = accepted.clone();
    let task = tokio::spawn(async move {
      loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while request.len() < 16384 && !request.ends_with(b"\r\n\r\n") {
          let mut byte = [0];
          if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            break;
          }
          request.push(byte[0]);
        }
        let authenticated = String::from_utf8_lossy(&request).lines().any(|line| {
          line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("cookie")
              && value
                .split(';')
                .any(|v| v.trim() == "session=hermetic-session")
          })
        });
        let (status, body): (&str, &[u8]) = if authenticated {
          count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
          (
            "200 OK",
            include_bytes!("../../../../src/assets/placeholder.png"),
          )
        } else {
          ("403 login required", b"login required")
        };
        let content_type = if request.starts_with(b"GET /asset.mp4 ") {
          "video/mp4"
        } else {
          "image/png"
        };
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.write_all(body).await;
      }
    });
    Self {
      url,
      accepted,
      task,
    }
  }
}

async fn run(
  binary: &std::path::Path,
  args: &[String],
  home: &std::path::Path,
) -> std::process::Output {
  let mut child = tokio::process::Command::new(binary);
  child
    .args(args)
    .env("HOME", home)
    .env("USERPROFILE", home)
    .env("XDG_CONFIG_HOME", home)
    .env("NO_PROXY", "127.0.0.1")
    .env_remove("HTTP_PROXY")
    .env_remove("HTTPS_PROXY")
    .env_remove("ALL_PROXY")
    .env_remove("http_proxy")
    .env_remove("https_proxy")
    .env_remove("all_proxy")
    .stdin(Stdio::null())
    .kill_on_drop(true);
  tokio::time::timeout(Duration::from_secs(20), child.output())
    .await
    .expect("fixture CLI exceeded nightly budget")
    .expect("install the real engine CLI")
}

#[tokio::test]
#[ignore = "nightly real-engine contract; needs Python sqlite3, yt-dlp, gallery-dl, loopback"]
async fn browser_cookie_contract() {
  let root = Staging::new().unwrap();
  let profile = root.path.join("Firefox profile");
  std::fs::create_dir(&profile).unwrap();
  let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
  let created = tokio::process::Command::new(python).args(["-c", r#"
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute('PRAGMA user_version=15')
    db.execute('CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, originAttributes TEXT)')
    db.execute('INSERT INTO moz_cookies VALUES (?, ?, ?, ?, ?, ?, ?)', ('127.0.0.1', 'session', 'hermetic-session', '/', 4102444800, 0, ''))
"#]).arg(profile.join("cookies.sqlite")).output().await.unwrap();
  assert!(
    created.status.success(),
    "Python sqlite3 fixture creation failed"
  );
  let browser = format!("firefox:{}", profile.display());
  let server = Server::start().await;
  let config = root
    .json(
      "gallery.json",
      &serde_json::json!({"extractor": {"proxy-env": false}}),
    )
    .unwrap();

  for (engine, binary) in [
    (
      "yt-dlp",
      std::env::var_os("YTDLP_TEST_BIN").unwrap_or_else(|| "yt-dlp".into()),
    ),
    (
      "gallery-dl",
      std::env::var_os("GALLERY_TEST_BIN").unwrap_or_else(|| "gallery-dl".into()),
    ),
  ] {
    let binary = PathBuf::from(binary);
    let target = format!(
      "{}/asset.{}",
      server.url,
      if engine == "yt-dlp" { "mp4" } else { "png" }
    );
    let mut session = LoginSession::from_auth(AuthSettings::default(), &target, None);
    let context = crate::runners::browser_cookies::CookieContext {
      exporter: PathBuf::from(
        std::env::var_os("YTDLP_TEST_BIN").unwrap_or_else(|| "yt-dlp".into()),
      ),
      directory: root.path.join("scoped"),
    };
    let build = |cookies: &[String]| {
      let mut args = if engine == "gallery-dl" {
        gallery::arguments(cookies, &config)
      } else {
        let mut args = vec![
          "--ignore-config".into(),
          "--no-cache-dir".into(),
          "--proxy".into(),
          "".into(),
          "--retries".into(),
          "0".into(),
        ];
        args.extend_from_slice(cookies);
        args
      };
      if engine == "yt-dlp" {
        args.extend(["--simulate".into(), "--get-url".into()]);
      } else {
        // -g never requests a directlink: transfer a tiny fixture to assert the actual Cookie header.
        args.extend([
          "--directory".into(),
          root.path.join("download").display().to_string(),
          "--filename".into(),
          "fixture.png".into(),
        ]);
      }
      args.extend(["--".into(), target.clone()]);
      args
    };
    let before = server.accepted.load(std::sync::atomic::Ordering::SeqCst);
    let refused = run(&binary, &build(&[]), &root.path).await;
    assert!(
      !refused.status.success(),
      "anonymous {engine} unexpectedly succeeded"
    );
    assert_eq!(
      server.accepted.load(std::sync::atomic::Ordering::SeqCst),
      before
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    let reason = if engine == "gallery-dl" {
      assert_eq!(
        crate::parsers::gallery::failure(&stderr),
        crate::parsers::gallery::GalleryError::Login
      );
      "signInRequired".to_string()
    } else {
      let matcher = crate::parsers::ytdlp_error::DiagnosticMatcher::from_json(include_str!(
        "../../diagnostic_rules.json"
      ))
      .unwrap();
      let parser =
        crate::parsers::ytdlp_error::YtdlpErrorParser::new("fixture", "fixture", matcher);
      stderr
        .lines()
        .find_map(|line| parser.login_reason(line))
        .expect("real engine refusal must reach login classification")
    };
    let (action, offered) = session.decide(Some(&reason), false, false, |_| {
      vec![LoginCandidate {
        id: browser.clone(),
        browser: crate::runners::browsers::BrowserKind::Firefox,
        profile: "Firefox profile".into(),
        signed_in: false,
        app_bound: false,
      }]
    });
    assert_eq!(action, RetryAction::AskLogin);
    assert!(session.browser.is_none());
    assert!(
      !context.directory.exists() || std::fs::read_dir(&context.directory).unwrap().count() == 0
    );
    session = LoginSession::from_auth(
      AuthSettings::default(),
      &target,
      Some(offered[0].id.clone()),
    );
    let (_cancel_tx, cancel) = tokio::sync::watch::channel(true);
    let scoped = crate::runners::browser_cookies::CookieAccess::prepare(
      &AuthSettings::default(),
      session.browser.as_deref(),
      &target,
      &context,
      cancel,
    )
    .await
    .unwrap();
    let retried = run(&binary, &build(&scoped.args), &root.path).await;
    assert!(
      retried.status.success(),
      "authenticated {engine} failed (output deliberately withheld)"
    );
    assert!(
      server.accepted.load(std::sync::atomic::Ordering::SeqCst) > before,
      "{engine} did not send the fixture cookie"
    );
    if engine == "gallery-dl" {
      assert_eq!(
        std::fs::read(root.path.join("download/fixture.png")).unwrap(),
        include_bytes!("../../../../src/assets/placeholder.png")
      );
    }
    assert_eq!(
      session
        .decide(Some(&reason), false, false, |_| panic!(
          "second browser retry"
        ))
        .0,
      RetryAction::Stop
    );
  }
}
