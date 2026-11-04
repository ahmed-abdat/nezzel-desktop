//! macOS MP4 compatibility, after yt-dlp has merged and moved the final output.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

#[cfg(target_os = "macos")]
pub(super) fn read_final_paths(path: &Path) -> Result<Vec<PathBuf>, String> {
  let contents = match std::fs::read_to_string(path) {
    Ok(contents) => contents,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
    Err(_) => return Err("Unable to read QuickTime output list".into()),
  };
  let mut paths = Vec::new();
  for line in contents.lines() {
    let path = serde_json::from_str::<String>(line).map_err(|_| "Invalid QuickTime output list")?;
    if !path.is_empty() {
      let path = PathBuf::from(path);
      if !paths.contains(&path) {
        paths.push(path);
      }
    }
  }
  Ok(paths)
}

async fn wait_for_cancel(cancel: &mut watch::Receiver<bool>) {
  loop {
    if !*cancel.borrow_and_update() {
      return;
    }
    if cancel.changed().await.is_err() {
      // A closed channel whose final value is still active is not a cancellation.
      std::future::pending::<()>().await;
    }
  }
}

fn should_convert(codec: &str, os: &str, container: &str, keep_original: bool) -> bool {
  os == "macos"
    && container.eq_ignore_ascii_case("mp4")
    && !keep_original
    && !matches!(codec, "" | "h264" | "hevc")
}

struct TemporaryOutput(PathBuf);

impl TemporaryOutput {
  fn beside(path: &Path) -> Result<Self, String> {
    let parent = path
      .parent()
      .ok_or("QuickTime output has no parent folder")?;
    let path = parent.join(format!(".nezzel-qt-{}.mp4", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(&path)
      .map_err(|_| "Unable to create QuickTime temporary output")?;
    Ok(Self(path))
  }
}

impl Drop for TemporaryOutput {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
  }
}

async fn probe(
  ffprobe: &Path,
  path: &Path,
  cancel: &mut watch::Receiver<bool>,
) -> Result<Option<serde_json::Value>, String> {
  if !*cancel.borrow() {
    return Ok(None);
  }
  let mut command = Command::new(ffprobe);
  command
    .args([
      "-v",
      "error",
      "-show_entries",
      "stream=index,codec_name,codec_type:stream_disposition=attached_pic:format=duration",
      "-of",
      "json",
    ])
    .arg(path)
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .kill_on_drop(true);
  let output = tokio::select! {
    biased;
    _ = wait_for_cancel(cancel) => return Ok(None),
    output = command.output() => output.map_err(|_| "Unable to start QuickTime codec probe")?,
  };
  if !output.status.success() {
    return Err("QuickTime codec probe failed".into());
  }
  serde_json::from_slice(&output.stdout)
    .map(Some)
    .map_err(|_| "Invalid QuickTime codec probe output".into())
}

fn main_video(info: &serde_json::Value) -> Option<&serde_json::Value> {
  info["streams"].as_array()?.iter().find(|stream| {
    stream["codec_type"] == "video" && stream["disposition"]["attached_pic"].as_u64() != Some(1)
  })
}

async fn encode(
  ffmpeg: &Path,
  input: &Path,
  output: &Path,
  info: &serde_json::Value,
  encoder: &str,
  cancel: &mut watch::Receiver<bool>,
  progress: &mut impl FnMut(Option<f64>),
) -> Result<bool, String> {
  if !*cancel.borrow() {
    return Ok(false);
  }
  let video = main_video(info).ok_or("QuickTime video stream is missing")?;
  let video_index = info["streams"]
    .as_array()
    .into_iter()
    .flatten()
    .filter(|stream| stream["codec_type"] == "video")
    .position(|stream| stream == video)
    .ok_or("QuickTime video stream is missing")?;
  let mut command = Command::new(ffmpeg);
  command
    .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
    .arg(input)
    .args([
      "-map",
      "0:v",
      "-map",
      "0:a?",
      "-map",
      "0:s?",
      "-map",
      "0:t?",
      "-map_metadata",
      "0",
      "-map_chapters",
      "0",
      "-c",
      "copy",
    ])
    .arg(format!("-c:v:{video_index}"))
    .arg(encoder)
    .arg(format!("-pix_fmt:v:{video_index}"))
    .arg("yuv420p");
  if encoder == "h264_videotoolbox" {
    command.args(["-q:v", "65"]);
  } else {
    command.args(["-crf", "18", "-preset", "medium", "-threads", "2"]);
  }
  if let Some(streams) = info["streams"].as_array() {
    for (index, stream) in streams
      .iter()
      .filter(|s| s["codec_type"] == "audio")
      .enumerate()
    {
      if !matches!(stream["codec_name"].as_str(), Some("aac" | "mp3" | "alac")) {
        command.arg(format!("-c:a:{index}")).arg("aac");
      }
    }
  }
  command
    .args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"])
    .arg(output)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true);
  let mut child = command
    .spawn()
    .map_err(|_| "Unable to start QuickTime encoder")?;
  let stdout = child
    .stdout
    .take()
    .ok_or("QuickTime progress pipe is missing")?;
  let mut lines = BufReader::new(stdout).lines();
  let duration = info["format"]["duration"]
    .as_str()
    .and_then(|s| s.parse::<f64>().ok())
    .filter(|d| d.is_finite() && *d > 0.0);
  loop {
    let line = tokio::select! {
      biased;
      _ = wait_for_cancel(cancel) => {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Ok(false);
      }
      line = lines.next_line() => line.map_err(|_| "Unable to read QuickTime progress")?,
    };
    let Some(line) = line else {
      break;
    };
    if let Some(elapsed) = line
      .strip_prefix("out_time_us=")
      .and_then(|v| v.parse::<f64>().ok())
    {
      if let Some(duration) = duration {
        progress(Some(
          (elapsed / 1_000_000.0 / duration * 100.0).clamp(0.0, 99.9),
        ));
      }
    }
  }
  let status = tokio::select! {
    biased;
    _ = wait_for_cancel(cancel) => {
      let _ = child.kill().await;
      let _ = child.wait().await;
      return Ok(false);
    }
    status = child.wait() => status.map_err(|_| "Unable to wait for QuickTime encoder")?,
  };
  if status.success() {
    Ok(true)
  } else {
    Err("QuickTime video encoding failed".into())
  }
}

/// Returns false when cancelled. The source is replaced only after a successful encode and probe.
pub(super) async fn convert_file(
  ffmpeg: &Path,
  ffprobe: &Path,
  path: &Path,
  cancel: &mut watch::Receiver<bool>,
  mut progress: impl FnMut(Option<f64>),
) -> Result<bool, String> {
  let container = path
    .extension()
    .and_then(|ext| ext.to_str())
    .unwrap_or_default();
  if !container.eq_ignore_ascii_case("mp4") {
    return Ok(true);
  }
  let Some(info) = probe(ffprobe, path, cancel).await? else {
    return Ok(false);
  };
  let codec = main_video(&info)
    .and_then(|video| video["codec_name"].as_str())
    .unwrap_or_default();
  if !should_convert(codec, "macos", container, false) {
    return Ok(true);
  }
  let temp = TemporaryOutput::beside(path)?;
  progress(None);
  let encoded = match encode(
    ffmpeg,
    path,
    &temp.0,
    &info,
    "h264_videotoolbox",
    cancel,
    &mut progress,
  )
  .await
  {
    Ok(completed) => completed,
    Err(_) => {
      encode(
        ffmpeg,
        path,
        &temp.0,
        &info,
        "libx264",
        cancel,
        &mut progress,
      )
      .await?
    }
  };
  if !encoded {
    return Ok(false);
  }
  let Some(encoded_info) = probe(ffprobe, &temp.0, cancel).await? else {
    return Ok(false);
  };
  if main_video(&encoded_info).and_then(|v| v["codec_name"].as_str()) != Some("h264") {
    return Err("QuickTime output is not H.264".into());
  }
  if !*cancel.borrow() {
    return Ok(false);
  }
  tokio::fs::rename(&temp.0, path)
    .await
    .map_err(|_| "Unable to replace QuickTime output")?;
  progress(Some(100.0));
  Ok(true)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::process::Command;

  #[test]
  fn quicktime_conversion_decision_table() {
    for (codec, os, container, keep, expected) in [
      ("vp9", "macos", "mp4", false, true),
      ("av1", "macos", "mp4", false, true),
      ("mpeg4", "macos", "mp4", false, true),
      ("h264", "macos", "mp4", false, false),
      ("hevc", "macos", "mp4", false, false),
      ("vp9", "macos", "mp4", true, false),
      ("vp9", "linux", "mp4", false, false),
      ("vp9", "windows", "mp4", false, false),
      ("vp9", "macos", "webm", false, false),
      ("vp9", "macos", "mkv", false, false),
      ("", "macos", "mp4", false, false),
    ] {
      assert_eq!(
        should_convert(codec, os, container, keep),
        expected,
        "{codec}/{os}/{container}/{keep}"
      );
    }
  }

  #[test]
  fn main_video_ignores_cover_art_and_non_video_streams() {
    for (info, expected) in [
      (
        serde_json::json!({"streams":[
          {"index":0,"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},
          {"index":1,"codec_type":"audio","codec_name":"aac"},
          {"index":2,"codec_type":"video","codec_name":"vp9","disposition":{"attached_pic":0}}
        ]}),
        Some("vp9"),
      ),
      (
        serde_json::json!({"streams":[
          {"index":0,"codec_type":"data"},
          {"index":1,"codec_type":"video","codec_name":"hevc"}
        ]}),
        Some("hevc"),
      ),
      (
        serde_json::json!({"streams":[
          {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},
          {"codec_type":"audio","codec_name":"aac"}
        ]}),
        None,
      ),
      (serde_json::json!({}), None),
    ] {
      assert_eq!(
        main_video(&info).and_then(|stream| stream["codec_name"].as_str()),
        expected,
        "{info}"
      );
    }
  }

  #[tokio::test]
  async fn vp9_mp4_becomes_h264_without_changing_resolution() {
    if !Command::new("ffmpeg")
      .arg("-version")
      .output()
      .is_ok_and(|output| output.status.success())
      || !Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
      eprintln!("skipped: ffmpeg/ffprobe absent");
      return;
    }
    let root = std::env::temp_dir().join(format!("nezzel-quicktime-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("video.mp4");
    let generated = Command::new("ffmpeg")
      .args([
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=blue:s=128x96:r=10:d=0.2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=0.2",
        "-c:v",
        "libvpx-vp9",
        "-threads",
        "2",
        "-c:a",
        "aac",
        "-metadata",
        "title=fixture",
        "-shortest",
      ])
      .arg(&path)
      .output();
    if !generated.is_ok_and(|output| output.status.success()) {
      eprintln!("skipped: ffmpeg could not generate the VP9 fixture (libvpx-vp9/lavfi required)");
      std::fs::remove_dir_all(root).unwrap();
      return;
    }
    let original = std::fs::read(&path).unwrap();
    let (tx, mut cancel) = watch::channel(true);
    assert!(convert_file(
      &root.join("missing-ffmpeg"),
      Path::new("ffprobe"),
      &path,
      &mut cancel,
      |_| {}
    )
    .await
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    tx.send(false).unwrap();
    assert!(!convert_file(
      Path::new("ffmpeg"),
      Path::new("ffprobe"),
      &path,
      &mut cancel,
      |_| {}
    )
    .await
    .unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    tx.send(true).unwrap();
    // An unseen active update, even with the sender closed, must not cancel conversion.
    drop(tx);
    let mut progress = Vec::new();
    #[cfg(unix)]
    let encoder = {
      use std::os::unix::fs::PermissionsExt;
      let wrapper = root.join("encoder");
      // Exercise real software fallback even on Macs where VideoToolbox succeeds.
      std::fs::write(&wrapper, "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = h264_videotoolbox ]; then exit 1; fi\ndone\nexec ffmpeg \"$@\"\n").unwrap();
      std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
      wrapper
    };
    #[cfg(not(unix))]
    let encoder = PathBuf::from("ffmpeg");
    assert!(
      convert_file(&encoder, Path::new("ffprobe"), &path, &mut cancel, |p| {
        progress.push(p)
      })
      .await
      .unwrap()
    );
    #[cfg(unix)]
    std::fs::remove_file(encoder).unwrap();
    let probe = Command::new("ffprobe")
      .args([
        "-v",
        "error",
        "-show_streams",
        "-show_format",
        "-of",
        "json",
      ])
      .arg(&path)
      .output()
      .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(
      (
        info["streams"][0]["codec_name"].as_str(),
        info["streams"][0]["width"].as_u64(),
        info["streams"][0]["height"].as_u64(),
        info["streams"][1]["codec_name"].as_str()
      ),
      (Some("h264"), Some(128), Some(96), Some("aac"))
    );
    assert_eq!(info["format"]["tags"]["title"], "fixture");
    assert!(!progress.is_empty());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
  }
}
