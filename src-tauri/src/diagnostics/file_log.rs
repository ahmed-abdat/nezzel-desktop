//! App log on disk, scrubbed at write time: `<app dir>/logs/nezzel.log`, rotated to
//! `nezzel.1.log` at 2 MB (≤ 4 MB total). The newest lines also stay in memory for reports.
//! A tracing `fmt` writer, so every `tracing::` event lands here without a log-crate bridge.

use super::redact::{local_names, scrub};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tracing_subscriber::fmt::MakeWriter;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const RING_LINES: usize = 200;
pub const FILE_NAME: &str = "nezzel.log";
pub const OLD_FILE_NAME: &str = "nezzel.1.log";

struct Sink {
  ring: VecDeque<String>,
  file: Option<File>,
  path: Option<PathBuf>,
  bytes: u64,
}

static SINK: LazyLock<Mutex<Sink>> = LazyLock::new(|| {
  Mutex::new(Sink {
    ring: VecDeque::with_capacity(RING_LINES),
    file: None,
    path: None,
    bytes: 0,
  })
});
static NAMES: LazyLock<Vec<String>> = LazyLock::new(local_names);

/// Start writing to `<dir>/nezzel.log`. Earlier lines stay in memory only.
pub fn attach_dir(dir: &Path) {
  if std::fs::create_dir_all(dir).is_err() {
    return;
  }
  let path = dir.join(FILE_NAME);
  let Ok(mut sink) = SINK.lock() else { return };
  if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
    sink.bytes = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    sink.file = Some(file);
    sink.path = Some(path);
  }
}

/// The newest `count` scrubbed lines, oldest first.
pub fn recent_lines(count: usize) -> Vec<String> {
  let Ok(sink) = SINK.lock() else {
    return Vec::new();
  };
  let skip = sink.ring.len().saturating_sub(count);
  sink.ring.iter().skip(skip).cloned().collect()
}

/// Scrub and record one formatted event (may span lines).
pub fn record(raw: &str) {
  let clean = scrub(raw, &NAMES);
  let Ok(mut sink) = SINK.lock() else { return };
  for line in clean.lines().filter(|line| !line.trim().is_empty()) {
    if sink.ring.len() == RING_LINES {
      sink.ring.pop_front();
    }
    sink.ring.push_back(line.to_string());
  }
  sink.append(&clean);
}

/// Like [`record`] for an already-scrubbed line, but gives up instead of waiting on the lock.
pub fn try_record(clean: &str) {
  if let Ok(mut sink) = SINK.try_lock() {
    if sink.ring.len() == RING_LINES {
      sink.ring.pop_front();
    }
    sink.ring.push_back(clean.to_string());
    sink.append(clean);
  }
}

impl Sink {
  fn append(&mut self, text: &str) {
    if self.file.is_none() {
      return;
    }
    if self.bytes + text.len() as u64 > MAX_FILE_BYTES {
      self.rotate();
    }
    let Some(file) = self.file.as_mut() else {
      return;
    };
    let mut text = text.to_string();
    if !text.ends_with('\n') {
      text.push('\n');
    }
    if file.write_all(text.as_bytes()).is_ok() {
      self.bytes += text.len() as u64;
    }
  }

  fn rotate(&mut self) {
    let Some(path) = self.path.clone() else {
      return;
    };
    self.file = None;
    let _ = std::fs::rename(&path, path.with_file_name(OLD_FILE_NAME));
    self.file = OpenOptions::new()
      .create(true)
      .write(true)
      .truncate(true)
      .open(&path)
      .ok();
    self.bytes = 0;
  }
}

/// `fmt::layer().with_writer(ReportLog)`: one buffer per event, scrubbed on drop.
#[derive(Clone, Copy, Default)]
pub struct ReportLog;

pub struct EventBuffer(Vec<u8>);

impl Write for EventBuffer {
  fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    self.0.extend_from_slice(buf);
    Ok(buf.len())
  }
  fn flush(&mut self) -> io::Result<()> {
    Ok(())
  }
}

impl Drop for EventBuffer {
  fn drop(&mut self) {
    if !self.0.is_empty() {
      record(&String::from_utf8_lossy(&self.0));
    }
  }
}

impl<'a> MakeWriter<'a> for ReportLog {
  type Writer = EventBuffer;
  fn make_writer(&'a self) -> Self::Writer {
    EventBuffer(Vec::with_capacity(256))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn records_scrubbed_lines_to_file_and_ring_and_rotates() {
    let dir = std::env::temp_dir().join(format!("nezzel-log-{}", uuid::Uuid::new_v4()));
    attach_dir(&dir);
    {
      let mut writer = ReportLog.make_writer();
      writer
        .write_all(
          b"WARN failed https://www.youtube.com/watch?v=secret for /Users/sara/Movies/a.mp4\n",
        )
        .unwrap();
    }
    let file = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
    assert!(
      !file.contains("youtube.com") && !file.contains("sara"),
      "{file}"
    );
    assert!(file.contains("WARN failed [redacted URL]"), "{file}");
    assert!(recent_lines(500)
      .iter()
      .any(|line| line.contains("WARN failed [redacted URL]")));

    // Rotation keeps one old file and starts fresh.
    {
      let mut sink = SINK.lock().unwrap();
      sink.bytes = MAX_FILE_BYTES;
    }
    record("after rotation");
    assert!(dir.join(OLD_FILE_NAME).exists());
    let fresh = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
    assert_eq!(fresh, "after rotation\n");
    let _ = std::fs::remove_dir_all(dir);
  }
}
