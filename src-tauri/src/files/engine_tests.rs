use super::{
  engine::{Engine, Event},
  state,
};
use sha2::{Digest, Sha256};
use std::sync::{
  atomic::{AtomicBool, Ordering},
  Arc, Mutex,
};
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::TcpListener,
  sync::watch,
};

struct Server {
  url: String,
  data: Arc<Vec<u8>>,
  version: Arc<AtomicBool>,
  requests: Arc<Mutex<Vec<String>>>,
  task: tokio::task::JoinHandle<()>,
  engine: Engine,
  fail_next: Option<Arc<AtomicBool>>,
}
impl Drop for Server {
  fn drop(&mut self) {
    self.task.abort();
  }
}
impl Server {
  async fn start(ranges: bool, ignore: bool, malformed: bool) -> Self {
    use reqwest::ResponseBuilderExt;
    let url = "https://files.example/fixture.bin".to_owned();
    let data = Arc::new(
      (0..10 * 1024 * 1024)
        .map(|i| ((i * 31 + i / 17) % 251) as u8)
        .collect::<Vec<_>>(),
    );
    let version = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let fail_next = Arc::new(AtomicBool::new(false));
    let fail = fail_next.clone();
    let (d, v, r) = (data.clone(), version.clone(), requests.clone());
    let engine = Engine::scripted(Arc::new(move |url, range| {
      if fail.swap(false, Ordering::SeqCst) {
        return Err(super::Error::Network);
      }
      let changed = v.load(Ordering::SeqCst);
      let tag = if changed { "\"v2\"" } else { "\"v1\"" };
      let mut request = format!("get {url} http/1.1\r\naccept-encoding: identity\r\n");
      if let Some((start, end, validator)) = range {
        request.push_str(&format!(
          "range: bytes={start}-{}\r\nif-range: {validator}\r\n",
          end.map(|n| n.to_string()).unwrap_or_default()
        ));
      }
      r.lock().unwrap().push(request);
      let valid = range.filter(|(_, _, validator)| ranges && !ignore && *validator == tag);
      let (start, end, status) = valid
        .map(|(start, end, _)| {
          (
            start as usize,
            end.unwrap_or(d.len() as u64 - 1) as usize,
            206,
          )
        })
        .unwrap_or((0, d.len() - 1, 200));
      let mut response = http::Response::builder()
        .status(status)
        .url(reqwest::Url::parse(url).unwrap())
        .header("content-type", "application/octet-stream")
        .header("content-length", end - start + 1)
        .header("etag", tag);
      if ranges {
        response = response.header("accept-ranges", "bytes");
      }
      if status == 206 {
        response = response.header(
          "content-range",
          format!("bytes {}-{end}/{}", start + usize::from(malformed), d.len()),
        );
      }
      let (data, mut offset) = (d.clone(), start);
      let stream = futures::stream::poll_fn(move |cx| {
        // Yield between network frames; no sockets or wall-clock waits.
        if offset > end {
          return std::task::Poll::Ready(None);
        }
        let limit = (offset + 32768).min(end + 1);
        let bytes: Vec<_> = data[offset..limit]
          .iter()
          .map(|b| if changed { b ^ 0x5a } else { *b })
          .collect();
        offset = limit;
        cx.waker().wake_by_ref();
        std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(bytes)))
      });
      Ok(reqwest::Response::from(
        response.body(reqwest::Body::wrap_stream(stream)).unwrap(),
      ))
    }));
    let task = tokio::spawn(async {});
    Self {
      url,
      data,
      version,
      requests,
      task,
      engine,
      fail_next: Some(fail_next),
    }
  }

  async fn start_tcp(ranges: bool, ignore: bool, malformed: bool) -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture.bin", listener.local_addr().unwrap());
    let data = Arc::new(
      (0..10 * 1024 * 1024)
        .map(|i| ((i * 31 + i / 17) % 251) as u8)
        .collect::<Vec<_>>(),
    );
    let version = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (d, v, r) = (data.clone(), version.clone(), requests.clone());
    let task = tokio::spawn(async move {
      let mut clients = tokio::task::JoinSet::new();
      loop {
        tokio::select! {
          accepted = listener.accept() => {
            let (mut socket,_) = accepted.unwrap();
            let (data, version, requests) = (d.clone(),v.clone(),r.clone());
            clients.spawn(async move {
              let mut request = Vec::new();
              while !request.ends_with(b"\r\n\r\n") {
                let mut c = [0];
                if socket.read_exact(&mut c).await.is_err() { return; }
                request.push(c[0]);
                if request.len() > 16384 { return; }
              }
              let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
              requests.lock().unwrap().push(request.clone());
              let changed = version.load(Ordering::SeqCst);
              let tag = if changed { "\"v2\"" } else { "\"v1\"" };
              let range = request.lines().find_map(|l| l.strip_prefix("range: bytes="));
              let valid = !request.contains("if-range:") || request.contains(&format!("if-range: {tag}"));
              let (start,end,status) = if let Some(range) = range.filter(|_| ranges && !ignore && valid) {
                let (a,b) = range.split_once('-').unwrap();
                (a.parse::<usize>().unwrap(), b.parse().unwrap_or(data.len()-1), "206 Partial Content")
              } else { (0,data.len()-1,"200 OK") };
              let extra = if status.starts_with("206") { format!("Content-Range: bytes {}-{end}/{}\r\n", start + usize::from(malformed), data.len()) } else { String::new() };
              let h = format!("HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nETag: {tag}\r\n{}{}Connection: close\r\n\r\n",end-start+1, if ranges { "Accept-Ranges: bytes\r\n" } else { "" },extra);
              if socket.write_all(h.as_bytes()).await.is_err() { return; }
              for chunk in data[start..=end].chunks(32768) {
                let changed_bytes;
                let body = if changed { changed_bytes = chunk.iter().map(|b| b^0x5a).collect::<Vec<_>>(); &changed_bytes[..] } else { chunk };
                if socket.write_all(body).await.is_err() { return; }
                tokio::task::yield_now().await;
              }
            });
          },
          _ = clients.join_next(), if !clients.is_empty() => {},
        }
      }
    });
    Self {
      url,
      data,
      version,
      requests,
      task,
      engine: Engine::local(),
      fail_next: None,
    }
  }
}
struct Dir(std::path::PathBuf);
impl Dir {
  fn new() -> Self {
    let p = std::env::temp_dir().join(format!("nezzel-engine-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&p).unwrap();
    Self(p)
  }
}
impl Drop for Dir {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

async fn range_cases(tcp: bool) {
  for (ranges, ignore, malformed) in [
    (true, false, false),
    (false, false, false),
    (true, true, false),
    (true, false, true),
  ] {
    let s = if tcp {
      Server::start_tcp(ranges, ignore, malformed).await
    } else {
      Server::start(ranges, ignore, malformed).await
    };
    let dir = Dir::new();
    let (_tx, rx) = watch::channel(true);
    let engine = s.engine.clone();
    let probe = engine.probe(&s.url, rx.clone()).await.unwrap();
    let result = engine
      .download(&s.url, &dir.0, "one", Some(probe), rx, &mut |_| {})
      .await
      .unwrap();
    assert_eq!(
      Sha256::digest(std::fs::read(&result).unwrap()),
      Sha256::digest(&*s.data)
    );
    let requests = s.requests.lock().unwrap();
    assert!(requests
      .iter()
      .all(|r| r.starts_with("get ") && r.contains("accept-encoding: identity")));
    if !ranges {
      assert_eq!(requests.len(), 1);
    }
    if ranges && !ignore && !malformed {
      assert_eq!(requests.len(), 8);
      assert_eq!(requests.iter().filter(|r| !r.contains("range:")).count(), 1);
      assert!(requests
        .iter()
        .skip(1)
        .all(|r| r.contains("if-range: \"v1\"")));
    }
  }
}

async fn resume_cases(tcp: bool) {
  for changed in [false, true] {
    let server = if tcp {
      Server::start_tcp(true, false, false).await
    } else {
      Server::start(true, false, false).await
    };
    let dir = Dir::new();
    let engine = server.engine.clone();
    let (_tx, rx) = watch::channel(true);
    let (checkpoint_tx, mut checkpoint_rx) = tokio::sync::mpsc::unbounded_channel();
    let (url, path, engine2, rx2) = (
      server.url.clone(),
      dir.0.clone(),
      engine.clone(),
      rx.clone(),
    );
    let task = tokio::spawn(async move {
      engine2
        .download(&url, &path, "kill", None, rx2, &mut |event| {
          if matches!(event,Event::Checkpoint { bytes } if bytes>0) {
            let _ = checkpoint_tx.send(());
          }
        })
        .await
    });
    checkpoint_rx
      .recv()
      .await
      .expect("download never checkpointed");
    task.abort();
    let _ = task.await;
    let saved = std::fs::read_dir(&dir.0)
      .unwrap()
      .filter_map(Result::ok)
      .find(|e| e.path().to_string_lossy().ends_with(state::STATE_SUFFIX))
      .unwrap()
      .path();
    let before = std::fs::read(&saved).unwrap();
    assert!(!String::from_utf8_lossy(&before).contains(&server.url));
    server.version.store(changed, Ordering::SeqCst);
    if let Some(fail) = &server.fail_next {
      fail.store(true, Ordering::SeqCst);
    }
    server.requests.lock().unwrap().clear();
    let result = engine
      .download(&server.url, &dir.0, "kill", None, rx, &mut |_| {})
      .await
      .unwrap();
    let expected = if changed {
      server.data.iter().map(|b| b ^ 0x5a).collect()
    } else {
      server.data.to_vec()
    };
    assert_eq!(
      Sha256::digest(std::fs::read(&result).unwrap()),
      Sha256::digest(expected)
    );
    assert!(server
      .requests
      .lock()
      .unwrap()
      .iter()
      .any(|r| r.contains("if-range: \"v1\"")));
    assert!(!saved.exists());
  }
}

#[tokio::test]
async fn cancellation_checkpoints_and_final_collision_never_overwrites() {
  let s = Server::start(true, false, false).await;
  let dir = Dir::new();
  std::fs::write(dir.0.join("fixture.bin"), b"existing").unwrap();
  let engine = s.engine.clone();
  let (tx, rx) = watch::channel(true);
  let stopped = engine
    .download(&s.url, &dir.0, "cancel", None, rx.clone(), &mut |e| {
      if matches!(e,Event::Progress { bytes,.. } if bytes>0) {
        let _ = tx.send(false);
      }
    })
    .await;
  assert!(matches!(stopped, Err(super::Error::Cancelled)));
  let sidecar = std::fs::read_dir(&dir.0)
    .unwrap()
    .filter_map(Result::ok)
    .find(|e| e.path().to_string_lossy().ends_with(state::STATE_SUFFIX))
    .unwrap()
    .path();
  assert!(sidecar.exists(), "pause must retain resume state");
  engine
    .discard(
      dir.0.clone(),
      state::identity(&s.url),
      state::identity("different-row"),
    )
    .await
    .unwrap();
  assert!(
    sidecar.exists(),
    "removing another row must not delete this checkpoint"
  );
  engine
    .discard(
      dir.0.clone(),
      state::identity(&s.url),
      state::identity("cancel"),
    )
    .await
    .unwrap();
  assert!(!sidecar.exists());
  assert!(std::fs::read_dir(&dir.0)
    .unwrap()
    .filter_map(Result::ok)
    .all(|e| !e.path().to_string_lossy().ends_with(state::PART_SUFFIX)));
  tx.send(true).unwrap();
  let path = engine
    .download(&s.url, &dir.0, "cancel", None, rx, &mut |_| {})
    .await
    .unwrap();
  assert_eq!(path.file_name().unwrap(), "fixture (1).bin");
  assert_eq!(
    std::fs::read(dir.0.join("fixture.bin")).unwrap(),
    b"existing"
  );
  #[cfg(target_os = "macos")]
  {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut value = [0u8; 128];
    let n = unsafe {
      libc::getxattr(
        path.as_ptr(),
        c"com.apple.quarantine".as_ptr(),
        value.as_mut_ptr().cast(),
        value.len(),
        0,
        libc::XATTR_NOFOLLOW,
      )
    };
    assert!(n > 0);
    assert!(value.starts_with(b"0081;"));
  }
}

#[tokio::test]
async fn range_download_reuses_probe_and_falls_back_without_ranges() {
  range_cases(false).await;
}
#[tokio::test(start_paused = true)]
async fn abort_after_durable_checkpoint_resumes_or_restarts_changed_source() {
  resume_cases(false).await;
}
#[tokio::test]
async fn local_http_server_ranges_fallback_kill_resume_and_if_range_change() {
  range_cases(true).await;
  resume_cases(true).await;
}

#[tokio::test]
async fn unknown_length_stream_finishes_once_and_error_pages_are_not_published() {
  use reqwest::ResponseBuilderExt;
  for (name, mime, data, expected) in [
    (
      "notes.txt",
      "application/octet-stream",
      vec![b'a'; 2048],
      true,
    ),
    (
      "archive.zip",
      "application/octet-stream",
      b"<html>sign in</html>".to_vec(),
      false,
    ),
    (
      "missing.zip",
      "application/octet-stream",
      b"File not found".to_vec(),
      false,
    ),
    (
      "small.pdf",
      "application/octet-stream",
      b"%PDF-1.7\n%%EOF".to_vec(),
      true,
    ),
    (
      "large.pdf",
      "application/octet-stream",
      vec![b' '; 4096],
      false,
    ),
    (
      "login.zip",
      "application/octet-stream",
      b"{\"login\":true}".repeat(100),
      false,
    ),
    (
      "wrong.exe",
      "application/octet-stream",
      b"%PDF-1.7".repeat(100),
      false,
    ),
    (
      "valid.7z",
      "application/octet-stream",
      b"7z\xbc\xaf\x27\x1c".repeat(100),
      true,
    ),
    (
      "valid.rar",
      "application/octet-stream",
      b"Rar!\x1a\x07\x01\x00".repeat(100),
      true,
    ),
    (
      "valid.gz",
      "application/octet-stream",
      b"\x1f\x8b\x08\x00".repeat(100),
      true,
    ),
    ("claimed.txt", "application/pdf", vec![b' '; 2048], false),
    (
      "setup.msi",
      "application/x-msdownload",
      b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".repeat(100),
      true,
    ),
    (
      "setup.bat",
      "application/x-msdownload",
      b"@echo off\r\necho hello\r\n".repeat(100),
      true,
    ),
  ] {
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = requests.clone();
    let source = data.clone();
    let engine = Engine::scripted(Arc::new(move |url, range| {
      assert!(range.is_none());
      assert_eq!(
        count.fetch_add(1, Ordering::SeqCst),
        0,
        "an unknown-size stream must not be fetched twice"
      );
      let body = reqwest::Body::wrap_stream(futures::stream::iter(
        source
          .chunks(64)
          .map(|c| Ok::<_, std::io::Error>(c.to_vec()))
          .collect::<Vec<_>>(),
      ));
      Ok(reqwest::Response::from(
        http::Response::builder()
          .status(200)
          .url(reqwest::Url::parse(url).unwrap())
          .header("content-type", mime)
          .body(body)
          .unwrap(),
      ))
    }));
    let dir = Dir::new();
    let (_tx, rx) = watch::channel(true);
    let result = engine
      .download(
        &format!("https://files.example/{name}"),
        &dir.0,
        "unknown",
        None,
        rx,
        &mut |_| {},
      )
      .await;
    assert_eq!(result.is_ok(), expected);
    if let Ok(path) = result {
      assert_eq!(std::fs::read(path).unwrap(), data);
    } else {
      assert!(!dir.0.join(name).exists());
    }
  }
}

#[tokio::test(start_paused = true)]
async fn a_single_use_get_survives_a_server_that_forbids_range_requests() {
  use reqwest::ResponseBuilderExt;
  for status in [403, 429, 503] {
    let data = Arc::new(vec![42u8; 3 * 1024 * 1024]);
    let source = data.clone();
    let full_gets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let requests = full_gets.clone();
    let range_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ranges = range_requests.clone();
    let engine = Engine::scripted(Arc::new(move |url, range| {
      if range.is_some() {
        assert_eq!(
          ranges.fetch_add(1, Ordering::SeqCst),
          0,
          "preflight rejection must immediately reuse the original stream"
        );
        return Ok(reqwest::Response::from(
          http::Response::builder()
            .status(status)
            .header("retry-after", "30")
            .body(Vec::<u8>::new())
            .unwrap(),
        ));
      }
      assert_eq!(
        requests.fetch_add(1, Ordering::SeqCst),
        0,
        "a one-use GET must never be repeated"
      );
      Ok(reqwest::Response::from(
        http::Response::builder()
          .status(200)
          .url(reqwest::Url::parse(url).unwrap())
          .header("content-type", "application/octet-stream")
          .header("accept-ranges", "bytes")
          .header("etag", "\"once\"")
          .header("content-length", source.len())
          .body(source.to_vec())
          .unwrap(),
      ))
    }));
    let dir = Dir::new();
    let (_tx, rx) = watch::channel(true);
    let path = engine
      .download(
        "https://files.example/once.bin",
        &dir.0,
        "once",
        None,
        rx,
        &mut |_| {},
      )
      .await
      .unwrap();
    assert_eq!(
      Sha256::digest(std::fs::read(path).unwrap()),
      Sha256::digest(&*data)
    );
  }
}

#[tokio::test(start_paused = true)]
async fn a_busy_server_reduces_connections_and_finishes_unassigned_segments() {
  use reqwest::ResponseBuilderExt;
  for status in [0, 403, 408, 429, 503] {
    let data = Arc::new(vec![17u8; 3 * 1024 * 1024]);
    let source = data.clone();
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = requests.clone();
    let engine = Engine::scripted(Arc::new(move |url, range| {
      let attempt = count.fetch_add(1, Ordering::SeqCst);
      if attempt == 2 {
        if status == 0 {
          return Err(super::Error::Network);
        }
        return Ok(reqwest::Response::from(
          http::Response::builder()
            .status(status)
            .header("retry-after", "0")
            .body(Vec::<u8>::new())
            .unwrap(),
        ));
      }
      let mut response = http::Response::builder()
        .url(reqwest::Url::parse(url).unwrap())
        .header("content-type", "application/octet-stream")
        .header("accept-ranges", "bytes")
        .header("etag", "\"v1\"");
      let (start, end) = if let Some((start, end, validator)) = range {
        assert_eq!(validator, "\"v1\"");
        let end = end.unwrap() as usize;
        response = response.status(206).header(
          "content-range",
          format!("bytes {start}-{end}/{}", source.len()),
        );
        (start as usize, end)
      } else {
        response = response.status(200);
        (0, source.len() - 1)
      };
      Ok(reqwest::Response::from(
        response
          .header("content-length", end - start + 1)
          .body(source[start..=end].to_vec())
          .unwrap(),
      ))
    }));
    let dir = Dir::new();
    let (_tx, rx) = watch::channel(true);
    let path = engine
      .download(
        "https://files.example/busy.bin",
        &dir.0,
        "busy",
        None,
        rx,
        &mut |_| {},
      )
      .await
      .unwrap();
    assert_eq!(
      Sha256::digest(std::fs::read(path).unwrap()),
      Sha256::digest(&*data)
    );
  }
}

#[tokio::test(start_paused = true)]
async fn transient_probe_failures_retry_and_persistent_failures_stop() {
  use reqwest::ResponseBuilderExt;
  for (status, persistent, expected_attempts) in [
    (0, false, 3),
    (0, true, 5),
    (503, false, 3),
    (503, true, 5),
    (501, true, 1),
  ] {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = attempts.clone();
    let engine = Engine::scripted(Arc::new(move |url, _| {
      if count.fetch_add(1, Ordering::SeqCst) < 2 || persistent {
        if status == 0 {
          return Err(super::Error::Network);
        }
        return Ok(reqwest::Response::from(
          http::Response::builder()
            .status(status)
            .url(reqwest::Url::parse(url).unwrap())
            .header("retry-after", "0")
            .body(Vec::<u8>::new())
            .unwrap(),
        ));
      }
      Ok(reqwest::Response::from(
        http::Response::builder()
          .status(200)
          .url(reqwest::Url::parse(url).unwrap())
          .header("content-type", "application/pdf")
          .body(b"%PDF-1.7".to_vec())
          .unwrap(),
      ))
    }));
    let (_tx, rx) = watch::channel(true);
    let result = engine.probe("https://files.example/retry.pdf", rx).await;
    assert_eq!(result.is_ok(), !persistent);
    assert_eq!(attempts.load(Ordering::SeqCst), expected_attempts);
  }
}

#[tokio::test]
async fn unsupported_file_fallback_survives_an_expired_probe_cache() {
  use reqwest::ResponseBuilderExt;
  let engine = Engine::scripted(Arc::new(|url, _| {
    Ok(reqwest::Response::from(
      http::Response::builder()
        .status(200)
        .url(reqwest::Url::parse(url).unwrap())
        .header("content-type", "application/octet-stream")
        .body(vec![17u8; 2048])
        .unwrap(),
    ))
  }));
  let (_tx, rx) = watch::channel(true);
  let dir = Dir::new();
  assert!(matches!(
    engine
      .download(
        "https://files.example/watch?id=1",
        &dir.0,
        "fallback",
        None,
        rx.clone(),
        &mut |_| {}
      )
      .await,
    Err(super::Error::NotFile)
  ));
  let path = engine
    .for_file_row()
    .download(
      "https://files.example/watch?id=1",
      &dir.0,
      "fallback",
      None,
      rx,
      &mut |_| {},
    )
    .await
    .unwrap();
  assert_eq!(std::fs::read(path).unwrap(), vec![17u8; 2048]);
}

#[tokio::test]
async fn shared_file_confirmation_reuses_final_stream_and_private_pages_fail() {
  use super::Error;
  use reqwest::ResponseBuilderExt;
  let engine = Engine::scripted(Arc::new(|url, range| {
    assert!(range.is_none());
    let body = match url {
      "https://drive.usercontent.google.com/download?id=abc&export=download&resourcekey=secret" => http::Response::builder().url(url.parse().unwrap()).header("content-type", "text/html").body(include_str!("fixtures/drive-confirm.html").as_bytes().to_vec()).unwrap(),
      "https://drive.usercontent.google.com/download?id=abc&export=download&confirm=t&uuid=a%26b&at=token%2Bvalue&future=keep&resourcekey=secret" => http::Response::builder().url(url.parse().unwrap()).header("content-type", "application/pdf").header("content-disposition", "attachment; filename=sample.pdf").header("content-length", "12").body(b"%PDF-1.7\nend".to_vec()).unwrap(),
      _ => http::Response::builder().url(url.parse().unwrap()).status(403).header("content-type", "text/html").body(include_str!("fixtures/google-private.html").as_bytes().to_vec()).unwrap(),
    };
    Ok(body.into())
  }));
  let (_tx, rx) = watch::channel(true);
  let dir = std::env::temp_dir().join(format!("nezzel-host-{}", uuid::Uuid::new_v4()));
  let result = engine
    .download(
      "https://drive.google.com/file/d/abc/view?resourcekey=secret",
      &dir,
      "owner",
      None,
      rx.clone(),
      &mut |_| {},
    )
    .await;
  let path = result.unwrap();
  assert_eq!(path.file_name().unwrap(), "sample.pdf");
  assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7\nend");
  assert!(matches!(
    engine
      .probe("https://docs.google.com/document/d/private/edit", rx)
      .await,
    Err(Error::NotPublic)
  ));
  std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn shared_ranges_and_resume_use_resolved_endpoint_with_original_identity() {
  use reqwest::ResponseBuilderExt;
  const SHARE: &str = "https://www.dropbox.com/scl/fi/abc/test.bin?rlkey=key&dl=0";
  const DIRECT: &str = "https://uc123.dl.dropboxusercontent.com/payload.bin";
  let data = Arc::new(vec![42_u8; 10 * 1024 * 1024]);
  let payload = data.clone();
  let ranges = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let count = ranges.clone();
  let resolves = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let generation = resolves.clone();
  let changed = Arc::new(AtomicBool::new(false));
  let version = changed.clone();
  let engine = Engine::scripted(Arc::new(move |url, range| {
    let response = http::Response::builder().url(url.parse().unwrap());
    if url == "https://www.dropbox.com/scl/fi/abc/test.bin?rlkey=key&dl=1" {
      assert!(range.is_none());
      let generation = generation.fetch_add(1, Ordering::SeqCst) + 1;
      return Ok(
        response
          .header("content-type", "text/html")
          .body(format!(
            "<a href='{DIRECT}?generation={generation}'>Download</a>"
          ))
          .unwrap()
          .into(),
      );
    }
    assert_eq!(
      url,
      format!("{DIRECT}?generation={}", generation.load(Ordering::SeqCst))
    );
    let changed = version.load(Ordering::SeqCst);
    let tag = if changed { "\"v2\"" } else { "\"v1\"" };
    let response = response.url(
      "https://uc999.dl.dropboxusercontent.com/expiring.bin"
        .parse()
        .unwrap(),
    );
    let (start, end, status) = match range {
      Some((start, end, validator)) => {
        count.fetch_add(1, Ordering::SeqCst);
        if validator != tag {
          (0, payload.len() - 1, 200)
        } else {
          (
            start as usize,
            end.unwrap_or(payload.len() as u64 - 1) as usize,
            206,
          )
        }
      }
      None => (0, payload.len() - 1, 200),
    };
    let mut response = response
      .status(status)
      .header("content-type", "application/octet-stream")
      .header("content-length", end - start + 1)
      .header("accept-ranges", "bytes")
      .header("etag", tag);
    if status == 206 {
      response = response.header(
        "content-range",
        format!("bytes {start}-{end}/{}", payload.len()),
      );
    }
    let data = payload.clone();
    let mut offset = start;
    let stream = futures::stream::poll_fn(move |cx| {
      if offset > end {
        return std::task::Poll::Ready(None);
      }
      let stop = (offset + 16384).min(end + 1);
      let bytes = data[offset..stop].to_vec();
      offset = stop;
      cx.waker().wake_by_ref();
      std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(bytes)))
    });
    Ok(
      response
        .body(reqwest::Body::wrap_stream(stream))
        .unwrap()
        .into(),
    )
  }));
  let dir = std::env::temp_dir().join(format!("nezzel-host-resume-{}", uuid::Uuid::new_v4()));
  let (tx, rx) = watch::channel(true);
  let first = engine
    .download(SHARE, &dir, "owner", None, rx, &mut |event| {
      if matches!(event, Event::Progress { bytes, .. } if bytes > 512 * 1024) {
        let _ = tx.send(false);
      }
    })
    .await;
  assert!(matches!(first, Err(super::Error::Cancelled)));
  changed.store(true, Ordering::SeqCst);
  let (_tx, rx) = watch::channel(true);
  let path = engine
    .download(SHARE, &dir, "owner", None, rx, &mut |_| {})
    .await
    .unwrap();
  assert_eq!(std::fs::read(path).unwrap(), *data);
  assert!(ranges.load(Ordering::SeqCst) >= 2);
  assert_eq!(resolves.load(Ordering::SeqCst), 2);
  assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
  std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn changing_google_export_format_does_not_reuse_the_other_formats_partial() {
  use reqwest::ResponseBuilderExt;
  let pdf_gets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let requests = pdf_gets.clone();
  let engine = Engine::scripted(Arc::new(move |url, _| {
    let office = url.ends_with("format=docx");
    if !office {
      requests.fetch_add(1, Ordering::SeqCst);
    }
    let mut data = vec![0_u8; 2 * 1024 * 1024];
    data[..8].copy_from_slice(if office {
      b"PK\x03\x04test"
    } else {
      b"%PDF-1.7"
    });
    let mime = if office {
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    } else {
      "application/pdf"
    };
    let disposition = if office {
      "attachment; filename=sample.docx"
    } else {
      "attachment; filename=sample.pdf"
    };
    let stream = futures::stream::iter(
      data
        .chunks(16384)
        .map(|b| Ok::<_, std::io::Error>(b.to_vec()))
        .collect::<Vec<_>>(),
    );
    Ok(
      http::Response::builder()
        .url(url.parse().unwrap())
        .header("content-type", mime)
        .header("content-length", data.len())
        .header("content-disposition", disposition)
        .body(reqwest::Body::wrap_stream(stream))
        .unwrap()
        .into(),
    )
  }));
  let dir = Dir::new();
  let url = "https://docs.google.com/document/d/abc/edit";
  let (tx, rx) = watch::channel(true);
  let result = engine
    .download(url, &dir.0, "owner", None, rx, &mut |event| {
      if matches!(event, Event::Progress { bytes, .. } if bytes > 32768) {
        let _ = tx.send(false);
      }
    })
    .await;
  assert!(matches!(result, Err(super::Error::Cancelled)));
  let office = engine.clone().with_export(super::hosts::Export::Office);
  let (tx, rx) = watch::channel(true);
  let result = office
    .download(url, &dir.0, "owner", None, rx, &mut |event| {
      if matches!(event, Event::Progress { bytes, .. } if bytes > 32768) {
        let _ = tx.send(false);
      }
    })
    .await;
  assert!(matches!(result, Err(super::Error::Cancelled)));
  // This is the same record constructor and cleanup used by the runner's registry.
  let partial = super::runner::Partial::new(office.clone(), dir.0.clone(), url, "owner");
  partial.discard().await.unwrap();
  assert!(!dir.0.join("sample.docx.nezzel-part").exists());
  assert!(dir.0.join("sample.pdf.nezzel-part").exists());
  let (_tx, rx) = watch::channel(true);
  let path = engine
    .clone()
    .with_export(super::hosts::Export::Office)
    .download(url, &dir.0, "owner", None, rx, &mut |_| {})
    .await
    .unwrap();
  assert_eq!(path.file_name().unwrap(), "sample.docx");
  assert_eq!(&std::fs::read(path).unwrap()[..8], b"PK\x03\x04test");
  let (_tx, rx) = watch::channel(true);
  let path = engine
    .download(url, &dir.0, "owner", None, rx, &mut |_| {})
    .await
    .unwrap();
  assert_eq!(path.file_name().unwrap(), "sample.pdf");
  assert_eq!(pdf_gets.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn anonymous_host_api_fixtures_resolve_metadata_and_keep_final_html_and_json() {
  use reqwest::ResponseBuilderExt;
  for (share, name, expected) in [
    (
      "https://pixeldrain.com/u/Ab123xyz",
      "report.html",
      b"<html>test file</html>".as_slice(),
    ),
    (
      "https://yadi.sk/d/public",
      "report.json",
      b"{\"a\":1}".as_slice(),
    ),
    (
      "https://www.mediafire.com/file/abc123/report.html/file",
      "report.html",
      b"<html>test file</html>".as_slice(),
    ),
    (
      "https://archive.org/details/test-item",
      "report.json",
      b"{\"a\":1}".as_slice(),
    ),
  ] {
    let engine = Engine::scripted(Arc::new(|url, range| {
      assert!(range.is_none());
      let (mime, body) = if url.ends_with("/info") {
        (
          "application/json",
          include_str!("fixtures/pixeldrain-info.json").to_owned(),
        )
      } else if url.starts_with("https://cloud-api.yandex.net/v1/disk/public/resources/download?") {
        (
          "application/json",
          include_str!("fixtures/yandex-download.json").to_owned(),
        )
      } else if url.starts_with("https://cloud-api.yandex.net/") {
        (
          "application/json",
          include_str!("fixtures/yandex-resource.json").to_owned(),
        )
      } else if url.starts_with("https://www.mediafire.com/") {
        (
          "text/html",
          include_str!("fixtures/mediafire-download.html").to_owned(),
        )
      } else if url == "https://archive.org/metadata/test-item" {
        ("application/json", r#"{"files":[{"name":"test-item_meta.xml","source":"metadata"},{"name":"report.json","source":"original","size":"7"}]}"#.to_owned())
      } else if url.contains("report.json") {
        ("application/json", "{\"a\":1}".to_owned())
      } else {
        assert!(
          url == "https://pixeldrain.com/api/file/Ab123xyz?download"
            || url.starts_with("https://download123.mediafire.com/"),
          "{url}"
        );
        ("text/html", "<html>test file</html>".to_owned())
      };
      Ok(
        http::Response::builder()
          .url(url.parse().unwrap())
          .header("content-type", mime)
          .body(body)
          .unwrap()
          .into(),
      )
    }));
    let (_tx, rx) = watch::channel(true);
    let probe = engine.probe(share, rx.clone()).await.unwrap();
    assert_eq!(probe.name, name, "{share}");
    assert_eq!(probe.size, Some(expected.len() as u64), "{share}");
    let dir = Dir::new();
    let path = engine
      .download(share, &dir.0, "owner", Some(probe), rx, &mut |_| {})
      .await
      .unwrap();
    assert_eq!(path.file_name().unwrap(), name);
    assert_eq!(std::fs::read(path).unwrap(), expected);
  }
}

#[tokio::test]
async fn github_file_downloads_reject_html_pages_without_saving_them() {
  use reqwest::ResponseBuilderExt;
  for (source, status, mime, expected_error) in [
    (
      "https://github.com/o/r/blob/main/a.html",
      200,
      "text/html; charset=utf-8",
      Some("fileNotFile"),
    ),
    (
      "https://github.com/o/r/raw/main/a.txt",
      200,
      "TEXT/HTML",
      Some("fileNotFile"),
    ),
    (
      "https://raw.githubusercontent.com/o/r/main/a.html",
      200,
      "text/html",
      Some("fileNotFile"),
    ),
    (
      "https://raw.githubusercontent.com/o/r/main/missing.txt",
      404,
      "text/html",
      Some("notFound404"),
    ),
    (
      "https://github.com/o/r/blob/main/a.html",
      200,
      "text/plain",
      None,
    ),
    (
      "https://github.com/o/r/releases/download/v1/a.html",
      200,
      "text/html",
      None,
    ),
  ] {
    let engine = Engine::scripted(Arc::new(move |url, _| {
      Ok(
        http::Response::builder()
          .url(url.parse().unwrap())
          .status(status)
          .header("content-type", mime)
          .body("<html>test page</html>")
          .unwrap()
          .into(),
      )
    }));
    let dir = Dir::new();
    let (_tx, rx) = watch::channel(true);
    let result = engine
      .download(source, &dir.0, "owner", None, rx, &mut |_| {})
      .await;
    match (result, expected_error) {
      (Err(error), Some(code)) => {
        assert_eq!(error.code(), code, "{source}");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0, "{source}");
      }
      (Ok(path), None) => assert_eq!(
        std::fs::read(path).unwrap(),
        b"<html>test page</html>",
        "{source}"
      ),
      (actual, expected) => panic!("{source}: got {actual:?}, expected {expected:?}"),
    }
  }
}

#[tokio::test]
async fn anonymous_hosts_report_removed_private_and_quota_responses() {
  use reqwest::ResponseBuilderExt;
  for (share, status, mime, body, code) in [
    (
      "https://pixeldrain.com/u/missing",
      404,
      "application/json",
      r#"{"success":false,"value":"not_found"}"#,
      "notFound404",
    ),
    (
      "https://pixeldrain.com/u/quota",
      403,
      "application/json",
      r#"{"success":false,"value":"file_rate_limited_captcha_required"}"#,
      "fileRateLimited",
    ),
    (
      "https://pixeldrain.com/u/private",
      403,
      "application/json",
      r#"{"success":false,"value":"access_denied"}"#,
      "fileNotPublic",
    ),
    (
      "https://yadi.sk/d/missing",
      404,
      "application/json",
      r#"{"error":"DiskNotFoundError"}"#,
      "notFound404",
    ),
    (
      "https://yadi.sk/d/quota",
      403,
      "application/json",
      r#"{"error":"DiskDownloadLimitExceededError"}"#,
      "fileRateLimited",
    ),
    (
      "https://yadi.sk/d/private",
      403,
      "application/json",
      r#"{"error":"DiskForbiddenError"}"#,
      "fileNotPublic",
    ),
    (
      "https://yadi.sk/d/folder",
      200,
      "application/json",
      r#"{"type":"dir","name":"folder"}"#,
      "fileNotFile",
    ),
    (
      "https://yadi.sk/d/rate",
      429,
      "application/json",
      "{}",
      "fileRateLimited",
    ),
    (
      "https://www.mediafire.com/file/missing",
      200,
      "text/html",
      "<html>Invalid or Deleted File.</html>",
      "notFound404",
    ),
    (
      "https://www.mediafire.com/file/private",
      200,
      "text/html",
      "<html><input type=password></html>",
      "fileNotPublic",
    ),
    (
      "https://www.mediafire.com/file/quota",
      509,
      "text/html",
      "",
      "fileRateLimited",
    ),
    (
      "https://archive.org/details/missing",
      200,
      "application/json",
      "{}",
      "notFound404",
    ),
    (
      "https://archive.org/details/private",
      200,
      "application/json",
      r#"{"is_dark":true}"#,
      "fileNotPublic",
    ),
    (
      "https://archive.org/details/album",
      200,
      "application/json",
      r#"{"files":[{"name":"a.mp3","source":"original"},{"name":"b.mp3","source":"original"}]}"#,
      "fileNotFile",
    ),
    (
      "https://archive.org/download/item/folder",
      200,
      "text/html",
      "<html>directory listing</html>",
      "fileNotFile",
    ),
    (
      "https://archive.org/download/missing/a.txt",
      404,
      "text/html",
      "",
      "notFound404",
    ),
    (
      "https://github.com/o/r/releases/download/missing/a.zip",
      404,
      "text/plain",
      "Not Found",
      "notFound404",
    ),
    (
      "https://github.com/o/r/blob/main/private.txt",
      403,
      "text/html",
      "",
      "fileNotPublic",
    ),
  ] {
    let engine = Engine::scripted(Arc::new(move |url, _| {
      Ok(
        http::Response::builder()
          .url(
            if url.ends_with("/folder") && url.contains("archive.org/download/") {
              format!("{url}/").parse().unwrap()
            } else {
              url.parse().unwrap()
            },
          )
          .status(status)
          .header("content-type", mime)
          .body(body)
          .unwrap()
          .into(),
      )
    }));
    let (_tx, rx) = watch::channel(true);
    match engine.probe(share, rx).await {
      Err(error) => assert_eq!(error.code(), code, "{share}"),
      Ok(_) => panic!("accepted an error response: {share}"),
    }
  }
}

#[tokio::test(start_paused = true)]
async fn archive_ambiguous_or_oversized_metadata_keeps_media_fallback() {
  use reqwest::ResponseBuilderExt;
  for (status, body) in [
    (
      200,
      r#"{"files":[{"name":"a.mp3","source":"original"},{"name":"b.mp3","source":"original"}]}"#
        .to_owned(),
    ),
    (200, " ".repeat(2 * 1024 * 1024 + 1)),
    (503, "API unavailable".to_owned()),
  ] {
    let engine = Engine::scripted(Arc::new(move |url, _| {
      Ok(
        http::Response::builder()
          .status(status)
          .url(url.parse().unwrap())
          .header("content-type", "application/json")
          .body(body.clone())
          .unwrap()
          .into(),
      )
    }));
    let (_tx, rx) = watch::channel(true);
    assert!(
      super::runner::probe_metadata(&engine, "https://archive.org/details/item", rx)
        .await
        .is_none()
    );
  }
}

#[tokio::test]
async fn yandex_resume_renews_signed_endpoint_and_keeps_name_and_bytes() {
  use reqwest::ResponseBuilderExt;
  use std::sync::atomic::AtomicUsize;
  let resolutions = Arc::new(AtomicUsize::new(0));
  let calls = resolutions.clone();
  let data = Arc::new(vec![42_u8; 1024 * 1024]);
  let content = data.clone();
  let engine = Engine::scripted(Arc::new(move |url, range| {
    let mut response = http::Response::builder().url(url.parse().unwrap());
    if url.starts_with("https://cloud-api.yandex.net/") {
      let body = if url.contains("resources/download?") {
        let version = calls.fetch_add(1, Ordering::SeqCst);
        format!(r#"{{"href":"https://downloader.disk.yandex.ru/disk/{version}/payload.bin"}}"#)
      } else {
        format!(
          r#"{{"type":"file","name":"original.bin","size":{}}}"#,
          content.len()
        )
      };
      return Ok(
        response
          .header("content-type", "application/json")
          .body(body)
          .unwrap()
          .into(),
      );
    }
    assert!(url.starts_with("https://downloader.disk.yandex.ru/disk/"));
    let (start, end) = range.map_or((0, content.len() - 1), |(start, end, validator)| {
      assert!(url.contains("/disk/1/"), "resume must use the renewed link");
      assert_eq!(validator, "\"version1\"");
      (
        start as usize,
        end.unwrap_or(content.len() as u64 - 1) as usize,
      )
    });
    if range.is_some() {
      response = response.status(206).header(
        "content-range",
        format!("bytes {start}-{end}/{}", content.len()),
      );
    }
    let frames = content[start..=end]
      .chunks(16384)
      .map(|b| Ok::<_, std::io::Error>(b.to_vec()))
      .collect::<Vec<_>>();
    Ok(
      response
        .header("content-type", "application/octet-stream")
        .header("content-length", end - start + 1)
        .header("accept-ranges", "bytes")
        .header("etag", "\"version1\"")
        .body(reqwest::Body::wrap_stream(futures::stream::iter(frames)))
        .unwrap()
        .into(),
    )
  }))
  .with_connections(1);
  let dir = Dir::new();
  let url = "https://yadi.sk/d/public";
  let (tx, rx) = watch::channel(true);
  assert!(matches!(
    engine
      .download(url, &dir.0, "owner", None, rx, &mut |event| {
        if matches!(event, Event::Progress { bytes, .. } if bytes > 32768) {
          let _ = tx.send(false);
        }
      })
      .await,
    Err(super::Error::Cancelled)
  ));
  let (_tx, rx) = watch::channel(true);
  let path = engine
    .download(url, &dir.0, "owner", None, rx, &mut |_| {})
    .await
    .unwrap();
  assert_eq!(path.file_name().unwrap(), "original.bin");
  assert_eq!(std::fs::read(path).unwrap(), *data);
  assert_eq!(resolutions.load(Ordering::SeqCst), 2);
}
