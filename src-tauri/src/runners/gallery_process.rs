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
    let retain = length.min(limit.saturating_sub(kept.len()));
    kept.extend_from_slice(&buffer[..retain]);
  }
}

pub async fn execute(
  path: &Path,
  args: &[String],
  mut cancel: watch::Receiver<bool>,
  deadline: Duration,
) -> Result<GalleryOutput, GalleryError> {
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
  let deadline = tokio::time::Instant::now() + deadline;
  let (code, stdout, stderr) = match wait_until(run, deadline, &mut cancel).await {
    Ok(output) => output?,
    Err(Interrupted::Cancelled) => return Err(GalleryError::Cancelled),
    Err(Interrupted::Timeout) => return Err(GalleryError::Refused("imageSourceUnavailable")),
  };
  #[cfg(feature = "link-matrix")]
  crate::link_matrix::observe_gallery_stderr(&stderr);
  Ok(GalleryOutput {
    code,
    stdout,
    stderr,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
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
    tx.send(false).unwrap();
    assert!(matches!(
      execute(
        Path::new("/does/not/exist"),
        &[],
        cancel,
        Duration::from_secs(5)
      )
      .await,
      Err(GalleryError::Cancelled)
    ));
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
    let running = tokio::spawn(async move {
      execute(Path::new("/bin/sh"), &args, cancel, Duration::from_secs(5)).await
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
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
      std::io::Error::last_os_error().raw_os_error(),
      Some(libc::ESRCH)
    );
  }
}
