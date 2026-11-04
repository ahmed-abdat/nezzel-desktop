use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

/// The only preview-completion event. It deliberately cannot carry an input URL or stderr.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewStatus {
  pub id: u64,
  pub title: String,
  pub details: PreviewDetails,
  pub failed: bool,
}

/// What the quick bar shows about a link: plain facts, never raw format ids.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDetails {
  /// https only; shown as an <img>, never fetched by the backend.
  pub thumbnail: Option<String>,
  pub media_kind: Option<String>,
  pub photos: Option<u64>,
  pub duration: Option<u64>,
  pub uploader: String,
  pub views: Option<u64>,
  /// Height of the best video format, or None for audio-only media.
  pub best_height: Option<u64>,
  pub best_ext: String,
  /// Bytes of the best video + best audio when the site reports them.
  pub size: Option<u64>,
}

fn text(item: &Value, key: &str, max: usize) -> String {
  item
    .get(key)
    .and_then(Value::as_str)
    .unwrap_or("")
    .trim()
    .chars()
    .take(max)
    .collect()
}

fn bytes(format: &Value) -> Option<u64> {
  ["filesize", "filesize_approx"]
    .iter()
    .find_map(|key| format.get(*key).and_then(Value::as_f64))
    // Site metadata is untrusted: anything past 1 TiB is noise, and the sum below saturates.
    .filter(|v| v.is_finite() && *v > 0.0 && *v < (1u64 << 40) as f64)
    .map(|v| v as u64)
}

pub fn summarize(item: &Value) -> PreviewDetails {
  let media_kind = item
    .get("mediaKind")
    .and_then(Value::as_str)
    .map(str::to_owned);
  let photos_only = matches!(media_kind.as_deref(), Some("image" | "carousel"));
  let formats: Vec<&Value> = item
    .get("formats")
    .and_then(Value::as_array)
    .filter(|_| !photos_only)
    .map(|f| f.iter().take(400).collect())
    .unwrap_or_default();
  let codec = |f: &Value, key: &str| {
    f.get(key)
      .and_then(Value::as_str)
      .is_some_and(|c| c != "none")
  };
  let height = |f: &Value| f.get("height").and_then(Value::as_u64).filter(|h| *h > 0);
  let best_video = formats
    .iter()
    .filter(|f| height(f).is_some() && (codec(f, "vcodec") || f.get("vcodec").is_none()))
    .max_by_key(|f| (height(f), bytes(f)));
  let best_audio = formats
    .iter()
    .filter(|f| codec(f, "acodec") && !codec(f, "vcodec"))
    .max_by_key(|f| f.get("abr").and_then(Value::as_f64).map(|v| v as u64));
  let video_has_audio = best_video.is_some_and(|f| codec(f, "acodec"));
  let size = if media_kind.as_deref() == Some("file") {
    bytes(item)
  } else {
    match (
      best_video.and_then(|f| bytes(f)),
      best_audio.and_then(|f| bytes(f)),
    ) {
      (Some(v), Some(a)) if !video_has_audio => Some(v.saturating_add(a)),
      (Some(v), _) => Some(v),
      (None, Some(a)) if best_video.is_none() => Some(a),
      _ => None,
    }
  };
  let best_ext = match best_video {
    Some(f) if video_has_audio => text(f, "ext", 8),
    Some(_) => "mp4".into(),
    None => best_audio.map(|f| text(f, "ext", 8)).unwrap_or_default(),
  };
  PreviewDetails {
    media_kind,
    photos: item
      .get("attachments")
      .and_then(Value::as_array)
      .map(|attachments| {
        attachments
          .iter()
          .filter(|attachment| attachment.get("kind").and_then(Value::as_str) == Some("image"))
          .count() as u64
      }),
    thumbnail: item
      .get("thumbnail")
      .and_then(Value::as_str)
      .filter(|u| u.starts_with("https://") && u.len() <= 2048)
      .map(str::to_owned),
    duration: item
      .get("duration")
      .and_then(Value::as_f64)
      .filter(|d| d.is_finite() && *d > 0.0)
      .map(|d| d.round() as u64),
    uploader: [text(item, "uploader", 120), text(item, "channel", 120)]
      .into_iter()
      .find(|s| !s.is_empty())
      .unwrap_or_default(),
    views: item.get("view_count").and_then(Value::as_u64),
    best_height: best_video.and_then(|f| height(f)),
    best_ext,
    size,
  }
}

pub async fn read_output(
  command: &mut Command,
  process_guard: tokio::sync::OwnedRwLockReadGuard<()>,
) -> Option<Vec<u8>> {
  command
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true);
  #[cfg(windows)]
  command.creation_flags(0x08000000); // CREATE_NO_WINDOW
  let mut child = command.spawn().ok()?;
  // 20 s leaves room for a second engine within capture_action's 46-second metadata wait.
  // Cancellation signals the owner to kill + reap before releasing its process guard.
  let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
  let owner = tokio::spawn(async move {
    let _process_guard = process_guard;
    let read = tokio::time::timeout(Duration::from_secs(20), async {
      let stdout = child.stdout.take()?;
      let mut bytes = Vec::new();
      stdout
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
      if bytes.len() > 8 * 1024 * 1024 || !child.wait().await.ok()?.success() {
        return None;
      }
      Some(bytes)
    });
    let result = tokio::select! {
      result = read => result.ok().flatten(),
      _ = cancelled => None,
    };
    // Also reap on oversized output and timeout; kill_on_drop alone does not wait.
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
  });
  let result = owner.await.ok().flatten();
  drop(cancel);
  result
}

#[cfg(test)]
mod summary_tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn summarizes_file_size_without_inventing_video_or_audio_quality() {
    let details =
      summarize(&json!({"mediaKind":"file", "title":"report.pdf", "filesize":4096, "formats":[]}));
    assert_eq!(
      (
        details.media_kind.as_deref(),
        details.size,
        details.best_height,
        details.best_ext.as_str()
      ),
      (Some("file"), Some(4096), None, "")
    );
  }

  #[test]
  fn summarizes_facts_and_never_raw_format_ids() {
    let item = json!({
      "thumbnail": "https://cdn.example/t.jpg", "duration": 718.4, "uploader": "Ahmed", "view_count": 42400,
      "formats": [
        {"format_id": "903814006058504v", "height": 1280, "vcodec": "h264", "acodec": "none", "ext": "mp4", "filesize": 100},
        {"format_id": "903813999391838v", "height": 1920, "vcodec": "h264", "acodec": "none", "ext": "mp4", "filesize_approx": 300.0},
        {"format_id": "902828846157020a", "abr": 88.0, "vcodec": "none", "acodec": "aac", "ext": "m4a", "filesize": 20}
      ]
    });
    let d = summarize(&item);
    assert_eq!(d.thumbnail.as_deref(), Some("https://cdn.example/t.jpg"));
    assert_eq!(
      (d.duration, d.uploader.as_str(), d.views),
      (Some(718), "Ahmed", Some(42400))
    );
    assert_eq!(
      (d.best_height, d.best_ext.as_str(), d.size),
      (Some(1920), "mp4", Some(320))
    );
  }

  #[test]
  fn summarizes_gallery_photos_from_the_serialized_media_item() {
    let post = crate::parsers::gallery::parse_gallery(
      br#"[[3,"https://cdn.example/first.jpg",{"extension":"jpg"}],
          [3,"https://cdn.example/clip.mp4",{"extension":"mp4"}],
          [3,"https://cdn.example/second.png",{"extension":"png"}],
          [3,"https://cdn.example/third.webp",{"extension":"webp"}]]"#,
      "https://x.com/user/status/123",
    )
    .unwrap();
    for (post, kind) in [
      (post.clone(), "mixed"),
      (
        {
          let mut images = post;
          images.assets.remove(1);
          images
        },
        "carousel",
      ),
    ] {
      let mut item = serde_json::to_value(post.into_media("capture-1".into())).unwrap();
      if kind == "carousel" {
        item["formats"] = json!([{"height": 1080, "vcodec": "h264", "ext": "mp4"}]);
      }
      let details = serde_json::to_value(summarize(&item)).unwrap();
      assert_eq!(details["mediaKind"], kind);
      assert_eq!(details["photos"], 3);
      assert_eq!(details["thumbnail"], "https://cdn.example/first.jpg");
      assert!(details["bestHeight"].is_null());
      assert_eq!(details["bestExt"], "");
      item["thumbnail"] = json!("http://cdn.example/first.jpg");
      assert!(summarize(&item).thumbnail.is_none());
    }
  }

  #[test]
  fn audio_only_and_unsafe_thumbnails() {
    let item = json!({
      "thumbnail": "http://insecure/t.jpg", "channel": "Label",
      "formats": [{"abr": 128.0, "vcodec": "none", "acodec": "opus", "ext": "webm", "filesize": 5}]
    });
    let d = summarize(&item);
    assert_eq!(d.thumbnail, None);
    assert_eq!(
      (
        d.best_height,
        d.best_ext.as_str(),
        d.size,
        d.uploader.as_str()
      ),
      (None, "webm", Some(5), "Label")
    );
  }

  #[test]
  fn absurd_sizes_are_ignored() {
    let item = json!({"formats": [
      {"height": 720, "vcodec": "avc1", "acodec": "none", "ext": "mp4", "filesize": 1.0e30},
      {"abr": 128.0, "vcodec": "none", "acodec": "opus", "ext": "webm", "filesize": 7}
    ]});
    assert_eq!(summarize(&item).size, None);
  }

  #[test]
  fn progressive_format_keeps_its_own_container_and_size() {
    let item = json!({"formats": [{"height": 720, "vcodec": "avc1", "acodec": "mp4a", "ext": "mp4", "filesize": 9}]});
    let d = summarize(&item);
    assert_eq!(
      (d.best_height, d.best_ext.as_str(), d.size),
      (Some(720), "mp4", Some(9))
    );
  }
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;
  use std::{
    io::Write,
    sync::{Arc, Mutex},
  };

  struct LogWriter(Arc<Mutex<Vec<u8>>>);
  impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
      self.0.lock().unwrap().extend_from_slice(bytes);
      Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
      Ok(())
    }
  }

  #[tokio::test]
  async fn cancelled_preview_keeps_update_lock_until_process_is_reaped() {
    use std::os::unix::ffi::OsStrExt;
    let root = std::env::temp_dir().join(format!("nezzel-preview-lock-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let fifo = root.join("input");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let input = fifo.clone();
    let pid_path = root.join("pid");
    let child_pid_path = pid_path.clone();
    let caller = tokio::spawn(async move {
      let mut command = Command::new("/bin/sh");
      command.args(["-c", "echo $$ > \"$1\"; exec /bin/cat \"$2\"", "sh"]);
      command.arg(child_pid_path).arg(input);
      let guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
      read_output(&mut command, guard).await
    });
    // Opening the writer completes only once the child has opened the FIFO.
    let writer = tokio::task::spawn_blocking(move || {
      std::fs::OpenOptions::new().write(true).open(fifo).unwrap()
    })
    .await
    .unwrap();
    let pid: i32 = std::fs::read_to_string(pid_path)
      .unwrap()
      .trim()
      .parse()
      .unwrap();
    assert!(crate::binaries::YTDLP_PROCESSES.try_write().is_err());
    caller.abort();
    let _ = caller.await;
    let _reaped = crate::binaries::YTDLP_PROCESSES.write().await;
    assert_eq!(
      unsafe { libc::kill(pid, 0) },
      -1,
      "write lock acquired before child was reaped"
    );
    assert_eq!(
      std::io::Error::last_os_error().raw_os_error(),
      Some(libc::ESRCH)
    );
    drop(writer);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[tokio::test]
  async fn failed_capture_fetch_discards_url_from_output_logs_and_completion_event() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
      .with_max_level(tracing::Level::TRACE)
      .without_time()
      .with_ansi(false)
      .with_writer(move || LogWriter(sink.clone()))
      .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let secret = "https://youtu.be/private?token=capture-secret";
    let mut command = Command::new("/bin/sh");
    command.args([
      "-c",
      "printf '%s' \"$1\"; printf '%s' \"$1\" >&2; exit 2",
      "sh",
      secret,
    ]);
    let guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
    let result = read_output(&mut command, guard).await;
    assert!(
      result.is_none(),
      "failed preview must not expose raw stdout/stderr"
    );
    let event = serde_json::to_string(&PreviewStatus {
      id: 1,
      failed: true,
      ..PreviewStatus::default()
    })
    .unwrap();
    assert!(event.contains("\"failed\":true"));
    assert!(!event.contains("youtu.be") && !event.contains("capture-secret"));
    assert!(
      logs.lock().unwrap().is_empty(),
      "preview failures must be silent in logs"
    );
  }
}
