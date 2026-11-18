use crate::binaries::bundled_tools::{self, exe_name, sidecar_candidates};
use crate::extract::deadline::{wait_until, Interrupted};
use crate::parsers::gallery::GalleryError;
use crate::runners::tool_process::{Spawned, ToolProcess};
use sha2::{Digest, Sha256};
use std::{
  collections::HashMap,
  io::Read,
  path::{Path, PathBuf},
  process::{Command, Stdio},
  sync::{LazyLock, Mutex},
  time::{Duration, SystemTime},
};
use tokio::sync::watch;

pub struct GalleryOutput {
  pub code: i32,
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
}

#[derive(PartialEq, Eq)]
struct FileStamp {
  length: u64,
  modified: Option<SystemTime>,
  created: Option<SystemTime>,
  #[cfg(unix)]
  identity: (u64, u64, i64, i64),
}
impl FileStamp {
  fn new(metadata: &std::fs::Metadata) -> Self {
    Self {
      length: metadata.len(),
      modified: metadata.modified().ok(),
      created: metadata.created().ok(),
      #[cfg(unix)]
      identity: {
        use std::os::unix::fs::MetadataExt;
        (
          metadata.dev(),
          metadata.ino(),
          metadata.ctime(),
          metadata.ctime_nsec(),
        )
      },
    }
  }
}
static VERIFIED: LazyLock<Mutex<HashMap<PathBuf, (String, FileStamp)>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn find_bundled(executable: &Path, resources: &Path) -> Result<PathBuf, GalleryError> {
  let expected = std::fs::read_to_string(resources.join("gallery-dl.sha256"))
    .map_err(|_| GalleryError::Unavailable)?;
  let expected = expected
    .split_whitespace()
    .next()
    .ok_or(GalleryError::Unavailable)?;
  let name = exe_name("gallery-dl");
  let name = name.as_str();
  // macOS ships a PyInstaller onedir resource (tools/gallery-dl/gallery-dl): a onefile build
  // extracts Python to $TMPDIR, which the hardened-runtime signature refuses to load.
  let onedir = resources.join("tools").join("gallery-dl").join(name);
  let candidates =
    std::iter::once(onedir).chain(sidecar_candidates(Some(executable), Some(resources), name));
  for path in candidates {
    let Ok(metadata) = path.metadata() else {
      continue;
    };
    if metadata.len() > 128 * 1024 * 1024 || !bundled_tools::executable(&path) {
      continue;
    }
    let stamp = FileStamp::new(&metadata);
    let mut verified = VERIFIED.lock().map_err(|_| GalleryError::Unavailable)?;
    if verified
      .get(&path)
      .is_some_and(|(digest, previous)| digest == expected && previous == &stamp)
    {
      return Ok(path);
    }
    let bytes = std::fs::read(&path).map_err(|_| GalleryError::Unavailable)?;
    if bytes.starts_with(b"NEZZEL_GALLERY_UNAVAILABLE") {
      continue;
    }
    if hex::encode(Sha256::digest(&bytes)) == expected {
      if verified.len() >= 32 {
        verified.clear();
      }
      verified.insert(path.clone(), (expected.into(), stamp));
      return Ok(path);
    }
  }
  Err(GalleryError::Unavailable)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, GalleryError> {
  let mut bytes = Vec::new();
  reader
    .take(limit as u64 + 1)
    .read_to_end(&mut bytes)
    .map_err(|_| GalleryError::Invalid)?;
  if bytes.len() > limit {
    return Err(GalleryError::Invalid);
  }
  Ok(bytes)
}

fn read_diagnostics(mut reader: impl Read, limit: usize) -> Result<Vec<u8>, GalleryError> {
  let mut kept = Vec::new();
  let mut buffer = [0; 8192];
  loop {
    let length = reader
      .read(&mut buffer)
      .map_err(|_| GalleryError::Invalid)?;
    if length == 0 {
      return Ok(kept);
    }
    kept.extend_from_slice(&buffer[..length]);
    if kept.len() > limit {
      kept.drain(..kept.len() - limit);
    }
  }
}

#[cfg(test)]
pub async fn execute(
  path: &Path,
  args: &[String],
  cancel: watch::Receiver<bool>,
  deadline: Duration,
) -> Result<GalleryOutput, GalleryError> {
  execute_with_started(path, args, cancel, deadline, || {}).await
}

pub(crate) async fn execute_with_started(
  path: &Path,
  args: &[String],
  cancel: watch::Receiver<bool>,
  deadline: Duration,
  started: impl FnOnce() + Send,
) -> Result<GalleryOutput, GalleryError> {
  execute_monitored(path, args, cancel, deadline, None, started, || {}).await
}

pub(crate) async fn execute_monitored(
  path: &Path,
  args: &[String],
  mut cancel: watch::Receiver<bool>,
  deadline: Duration,
  staging: Option<&Path>,
  started: impl FnOnce() + Send,
  mut progressed: impl FnMut() + Send,
) -> Result<GalleryOutput, GalleryError> {
  if !*cancel.borrow() {
    return Err(GalleryError::Cancelled);
  }
  let start = tokio::time::Instant::now();
  let mut watchdog = crate::runners::reliability::StallWatch::new(Duration::ZERO);
  if let Some(bytes) = staging.and_then(crate::runners::reliability::partial_bytes) {
    watchdog.observe_file_bytes(bytes, Duration::ZERO);
  }
  let mut command = Command::new(path);
  command
    .args(args)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
  let Spawned {
    mut process,
    stdout,
    stderr,
  } = ToolProcess::spawn(&mut command, ()).map_err(|_| GalleryError::Unavailable)?;
  started();
  let stdout = stdout.ok_or(GalleryError::Invalid)?;
  let stderr = stderr.ok_or(GalleryError::Invalid)?;
  let out_task = tokio::task::spawn_blocking(move || read_bounded(stdout, 8 * 1024 * 1024));
  let err_task = tokio::task::spawn_blocking(move || read_diagnostics(stderr, 128 * 1024));
  let run = async {
    tokio::try_join!(
      async {
        let status = process.wait().await.map_err(|_| GalleryError::Invalid)?;
        Ok(status.code().unwrap_or(1))
      },
      async { out_task.await.map_err(|_| GalleryError::Invalid)? },
      async { err_task.await.map_err(|_| GalleryError::Invalid)? },
    )
  };
  let deadline = tokio::time::Instant::now()
    + if staging.is_some() {
      deadline.max(Duration::from_secs(300))
    } else {
      deadline
    };
  let guarded = wait_until(run, deadline, &mut cancel);
  tokio::pin!(guarded);
  let mut tick = tokio::time::interval(Duration::from_secs(1));
  let result = loop {
    tokio::select! {
      result = &mut guarded => break result,
      _ = tick.tick(), if staging.is_some() => {
        if let Some(bytes) = staging.and_then(crate::runners::reliability::partial_bytes) { if watchdog.observe_file_bytes(bytes, start.elapsed()) { progressed(); } }
        if watchdog.extraction_expired(start.elapsed()) { return Err(GalleryError::Refused("imageSourceUnavailable")); }
        if watchdog.stalled(start.elapsed()) { return Err(GalleryError::Refused("downloadStalled")); }
      }
    }
  };
  let (code, stdout, stderr) = match result {
    Ok(output) => output?,
    Err(Interrupted::Cancelled) => return Err(GalleryError::Cancelled),
    Err(Interrupted::Timeout) => {
      return Err(GalleryError::Refused(if staging.is_none() {
        "fetchTimeout"
      } else {
        "imageSourceUnavailable"
      }))
    }
  };
  #[cfg(feature = "link-matrix")]
  crate::link_matrix::observe_gallery_stderr(&stderr);
  Ok(GalleryOutput {
    code,
    stdout,
    stderr,
  })
}

/// gallery-dl's output.jsonl mode flushes each record while enumeration is running.
pub(crate) async fn execute_listing(
  path: &Path,
  args: &[String],
  mut cancel: watch::Receiver<bool>,
  budget: Duration,
  started: impl FnOnce() + Send,
) -> Result<(GalleryOutput, bool), GalleryError> {
  use std::io::BufRead;
  if !*cancel.borrow() {
    return Err(GalleryError::Cancelled);
  }
  let mut command = Command::new(path);
  command
    .args(args)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
  let Spawned {
    mut process,
    stdout,
    stderr,
  } = ToolProcess::spawn(&mut command, ()).map_err(|_| GalleryError::Unavailable)?;
  started();
  let stdout = stdout.ok_or(GalleryError::Invalid)?;
  let stderr = stderr.ok_or(GalleryError::Invalid)?;
  let (tx, mut rx) = tokio::sync::mpsc::channel(8);
  let reader = tokio::task::spawn_blocking(move || {
    let mut reader = std::io::BufReader::new(stdout.take(8 * 1024 * 1024 + 1));
    loop {
      let mut line = Vec::new();
      match reader.read_until(b'\n', &mut line) {
        Ok(0) => break,
        Ok(_) => {
          if tx.blocking_send(Ok(line)).is_err() {
            break;
          }
        }
        Err(_) => {
          let _ = tx.blocking_send(Err(GalleryError::Invalid));
          break;
        }
      }
    }
  });
  let errors = tokio::task::spawn_blocking(move || read_diagnostics(stderr, 128 * 1024));
  let deadline = tokio::time::Instant::now() + budget;
  let mut records = Vec::new();
  let mut count = 0;
  let mut size = 0;
  let mut truncated = false;
  let mut budget_exhausted = false;
  // A failed enumeration ends in one pretty-printed array (`[`, `  [`, ...), not JSONL lines.
  let mut error_array: Option<Vec<u8>> = None;
  let mut code = 0;
  loop {
    let line = match wait_until(rx.recv(), deadline, &mut cancel).await {
      Ok(Some(line)) => line?,
      Ok(None) => {
        code = match wait_until(process.wait(), deadline, &mut cancel).await {
          Ok(status) => status
            .map_err(|_| GalleryError::Invalid)?
            .code()
            .unwrap_or(1),
          Err(Interrupted::Cancelled) => return Err(GalleryError::Cancelled),
          Err(Interrupted::Timeout) => {
            truncated = true;
            budget_exhausted = true;
            0
          }
        };
        break;
      }
      Err(Interrupted::Cancelled) => return Err(GalleryError::Cancelled),
      Err(Interrupted::Timeout) => {
        truncated = true;
        budget_exhausted = true;
        break;
      }
    };
    size += line.len();
    if size > 8 * 1024 * 1024 {
      return Err(GalleryError::Invalid);
    }
    if line.iter().all(u8::is_ascii_whitespace) {
      continue;
    }
    if let Some(array) = &mut error_array {
      array.extend_from_slice(&line);
      continue;
    }
    let record: serde_json::Value = match serde_json::from_slice(&line) {
      Ok(record) => record,
      Err(_) if records.is_empty() => {
        error_array = Some(line);
        continue;
      }
      Err(_) => return Err(GalleryError::Invalid),
    };
    if record[0] == 3
      || (record[0] == 6
        && record[1]
          .as_str()
          .is_some_and(|url| url.starts_with("ytdl:")))
    {
      count += 1;
    }
    records.push(record);
    if count >= 100 {
      truncated = true;
      break;
    }
  }
  // Close the receiver before joining: the bounded reader may be waiting to send.
  drop(rx);
  drop(process); // kills descendants and reaps the process at the cap or budget
  reader.await.map_err(|_| GalleryError::Invalid)?;
  let mut stderr = errors.await.map_err(|_| GalleryError::Invalid)??;
  if budget_exhausted {
    // code=0 admits the verified partial entries; it is not a child exit status.
    stderr
      .extend_from_slice(b"\n[nezzel] listing budget ran out; process stopped before completion\n");
    if stderr.len() > 128 * 1024 {
      stderr.drain(..stderr.len() - 128 * 1024);
    }
  }
  if let Some(array) = error_array {
    records = serde_json::from_slice(&array).map_err(|_| GalleryError::Invalid)?;
  }
  #[cfg(feature = "link-matrix")]
  crate::link_matrix::observe_gallery_stderr(&stderr);
  if truncated && count == 0 {
    return Err(GalleryError::Refused("fetchTimeout"));
  }
  Ok((
    GalleryOutput {
      code,
      stdout: serde_json::to_vec(&records).map_err(|_| GalleryError::Invalid)?,
      stderr,
    },
    truncated,
  ))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Recorded gallery-dl 1.31 output for a deleted Facebook album listing (jsonl mode):
  /// the failure arrives as one pretty-printed array, not as JSONL lines.
  #[tokio::test]
  #[cfg(unix)]
  async fn recorded_facebook_album_404_is_not_found_not_unknown() {
    let staging = crate::runners::gallery_files::Staging::new().unwrap();
    let recorded = staging.path.join("recorded.json");
    std::fs::write(
      &recorded,
      include_bytes!("../parsers/fixtures/gallery-recorded-facebook-album-404.json"),
    )
    .unwrap();
    let (_tx, cancel) = watch::channel(true);
    let (output, truncated) = execute_listing(
      Path::new("/bin/cat"),
      &[recorded.to_string_lossy().into_owned()],
      cancel,
      Duration::from_secs(30),
      || {},
    )
    .await
    .unwrap();
    assert!(!truncated);
    assert_eq!(
      crate::parsers::gallery::parse_output(
        output.code,
        &output.stdout,
        &output.stderr,
        "https://www.facebook.com/media/set/?set=a.10150146071831729",
      )
      .unwrap_err(),
      GalleryError::Refused("notFound404")
    );
  }

  #[tokio::test(start_paused = true)]
  #[cfg(unix)]
  async fn recorded_partial_listing_keeps_budget_exhaustion_evidence() {
    let staging = crate::runners::gallery_files::Staging::new().unwrap();
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
      "../parsers/fixtures/gallery-recorded-bluesky-listing.json"
    ))
    .unwrap();
    let records = staging.path.join("records");
    // Fill beyond both the pipe and the eight-record channel before signalling ready:
    // seeing the marker proves the collector consumed the first asset.
    let directory = serde_json::json!([2, {"description": "x".repeat(128 * 1024)}]);
    let mut lines = format!("{}\n", fixture[0]);
    for _ in 0..16 {
      lines.push_str(&format!("{directory}\n"));
    }
    std::fs::write(&records, lines).unwrap();
    let marker = staging.path.join("ready");
    let child_marker = marker.clone();
    let (_tx, cancel) = watch::channel(true);
    let running = tokio::spawn(async move {
      execute_listing(
        Path::new("/bin/sh"),
        &[
          "-c".into(),
          "cat \"$1\"; echo ready > \"$2\"; exec 1>&-; exec /usr/bin/tail -f /dev/null".into(),
          "gallery".into(),
          records.to_string_lossy().into_owned(),
          child_marker.to_string_lossy().into_owned(),
        ],
        cancel,
        Duration::from_secs(30),
        || {},
      )
      .await
    });
    while !marker.exists() {
      assert!(
        !running.is_finished(),
        "process ended before emitting its fixture"
      );
      tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(30)).await;
    let (output, truncated) = running.await.unwrap().unwrap();
    assert!(truncated);
    let post = crate::parsers::gallery::parse_output(
      output.code,
      &output.stdout,
      &output.stderr,
      "https://bsky.app/profile/nytimes.com/media",
    )
    .unwrap();
    assert_eq!(post.assets.len(), 1);
    assert!(
      String::from_utf8_lossy(&output.stderr).contains("budget ran out"),
      "partial success must explain why enumeration stopped: {:?}",
      output.stderr
    );
  }

  #[test]
  fn only_checksum_verified_bundled_sidecars_are_accepted() {
    let root = std::env::temp_dir().join(format!("gallery-resolve-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let binary = root.join(exe_name("gallery-dl"));
    std::fs::write(&binary, b"verified executable").unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
      root.join("gallery-dl.sha256"),
      hex::encode(Sha256::digest(b"verified executable")),
    )
    .unwrap();
    assert_eq!(find_bundled(&root.join("app"), &root).unwrap(), binary);
    std::fs::write(&binary, b"tampered").unwrap();
    assert!(find_bundled(&root.join("app"), &root).is_err());
    // macOS onedir resource layout: <resources>/tools/gallery-dl/gallery-dl.
    let onedir = root
      .join("tools/gallery-dl")
      .join(binary.file_name().unwrap());
    std::fs::create_dir_all(onedir.parent().unwrap()).unwrap();
    std::fs::write(&onedir, b"verified executable").unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(&onedir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(find_bundled(&root.join("app"), &root).unwrap(), onedir);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[tokio::test]
  #[cfg(unix)]
  async fn subprocess_output_is_bounded_and_pre_cancelled_requests_do_not_spawn() {
    let (tx, cancel) = watch::channel(true);
    let output = execute(
      Path::new("/bin/sh"),
      &["-c".into(), "printf '[[-1,{}]]'".into()],
      cancel.clone(),
      Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(output.code, 0);
    assert_eq!(output.stdout, b"[[-1,{}]]");
    let noisy = execute(
      Path::new("/bin/sh"),
      &[
        "-c".into(),
        "dd if=/dev/zero bs=1024 count=256 1>&2 2>/dev/null; printf '[]'".into(),
      ],
      cancel.clone(),
      Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(noisy.code, 0);
    assert_eq!(noisy.stdout, b"[]");
    assert_eq!(noisy.stderr.len(), 128 * 1024);
    assert!(matches!(
      execute(
        Path::new("/bin/sh"),
        &[
          "-c".into(),
          "dd if=/dev/zero bs=1048576 count=9 2>/dev/null".into()
        ],
        cancel.clone(),
        Duration::from_secs(5)
      )
      .await,
      Err(GalleryError::Invalid)
    ));
    let started = std::sync::atomic::AtomicUsize::new(0);
    assert!(matches!(
      execute_with_started(
        Path::new("/does/not/exist"),
        &[],
        cancel.clone(),
        Duration::from_secs(5),
        || {
          started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
      )
      .await,
      Err(GalleryError::Unavailable)
    ));
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 0);
    tx.send(false).unwrap();
    assert!(matches!(
      execute_with_started(
        Path::new("/does/not/exist"),
        &[],
        cancel,
        Duration::from_secs(5),
        || {
          started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
      )
      .await,
      Err(GalleryError::Cancelled)
    ));
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 0);
  }

  #[tokio::test]
  #[cfg(unix)]
  async fn cancellation_reaps_the_running_process() {
    let stage = crate::runners::gallery_files::Staging::new().unwrap();
    let marker = stage.path.join("pid");
    let args = vec![
      "-c".into(),
      "echo $$ > \"$1\"; exec /usr/bin/tail -f /dev/null".into(),
      "sh".into(),
      marker.to_string_lossy().into_owned(),
    ];
    let (tx, cancel) = watch::channel(true);
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let announced = started.clone();
    let running = tokio::spawn(async move {
      execute_with_started(
        Path::new("/bin/sh"),
        &args,
        cancel,
        Duration::from_secs(5),
        move || {
          announced.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        },
      )
      .await
    });
    let pid = loop {
      if let Some(pid) = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
      {
        break pid;
      }
      assert!(
        !running.is_finished(),
        "child exited before publishing its pid"
      );
      tokio::task::yield_now().await;
    };
    tx.send(false).unwrap();
    assert!(matches!(
      running.await.unwrap(),
      Err(GalleryError::Cancelled)
    ));
    assert_eq!(
      started.load(std::sync::atomic::Ordering::SeqCst),
      1,
      "a later cancellation cannot erase actual use"
    );
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
      std::io::Error::last_os_error().raw_os_error(),
      Some(libc::ESRCH)
    );
  }
}
