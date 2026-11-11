//! Local crash catcher: a panic hook writes a scrubbed crash file to the app dir; the next
//! launch reads it, and deletes it once it is sent or staged for the user's click. Nothing is sent from here.

use super::redact::{local_names, scrub};
use std::backtrace::Backtrace;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// This run's panic hook wrote the crash file (it replaces any earlier one).
static WRITTEN_THIS_RUN: AtomicBool = AtomicBool::new(false);

pub const CRASH_FILE: &str = "crash-report.txt";
pub const NONFATAL_FILE: &str = "background-panic.txt";
const MAX_FRAMES: usize = 30;

pub fn install(dir: PathBuf, version: String, os: String) {
  let names = local_names();
  let previous = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    let payload = info.payload();
    let message = payload
      .downcast_ref::<&str>()
      .map(|text| (*text).to_string())
      .or_else(|| payload.downcast_ref::<String>().cloned())
      .unwrap_or_else(|| "unknown panic payload".into());
    let location = info
      .location()
      .map(|at| format!("{}:{}:{}", short_path(at.file()), at.line(), at.column()));
    let thread = std::thread::current().name().map(str::to_string);
    let backtrace = Backtrace::force_capture().to_string();
    let text = format_crash(
      &message,
      location.as_deref(),
      thread.as_deref(),
      &backtrace,
      &version,
      &os,
    );
    let clean = scrub(&text, names);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(CRASH_FILE), clean.as_str());
    WRITTEN_THIS_RUN.store(true, Ordering::SeqCst);
    // Non-blocking: the panic may have happened while the log lock was held.
    if let Some(line) = clean.lines().nth(2) {
      super::file_log::try_record(line.with_prefix("ERROR panic: "));
    }
    previous(info);
  }));
}

/// The previous run's crash file, if any. The caller deletes it ([`clear`]) once the crash is handled.
pub fn read(dir: &Path) -> Option<String> {
  std::fs::read_to_string(dir.join(CRASH_FILE))
    .ok()
    .filter(|text| !text.trim().is_empty())
}

/// Remove an unexpected-close report after it was sent or staged.
pub fn clear(dir: &Path) {
  let _ = std::fs::remove_file(dir.join(CRASH_FILE));
}

/// Keep this run's background panic separately, so it can be reported without an unexpected-close prompt.
/// A file from an earlier run stays until it is sent or staged.
pub fn clear_on_clean_exit(dir: &Path) {
  clear_if_written(dir, &WRITTEN_THIS_RUN);
}

fn clear_if_written(dir: &Path, written: &AtomicBool) {
  if written.load(Ordering::SeqCst) {
    let _ = std::fs::rename(dir.join(CRASH_FILE), dir.join(NONFATAL_FILE));
  }
}

/// Background panics are consumed once, including when reports are off.
pub fn take_nonfatal(dir: &Path) -> Option<String> {
  let path = dir.join(NONFATAL_FILE);
  let text = std::fs::read_to_string(&path).ok();
  let _ = std::fs::remove_file(path);
  text.filter(|text| !text.trim().is_empty())
}

pub fn format_crash(
  message: &str,
  location: Option<&str>,
  thread: Option<&str>,
  backtrace: &str,
  version: &str,
  os: &str,
) -> String {
  let mut out = format!("Nezzel {version} on {os}\n");
  out.push_str(&format!(
    "thread {} panicked at {}\n",
    thread.unwrap_or("unnamed"),
    location.unwrap_or("unknown location")
  ));
  out.push_str(message.trim());
  out.push('\n');
  let frames = our_frames(backtrace);
  if !frames.is_empty() {
    out.push_str("backtrace (Nezzel frames):\n");
    for frame in frames {
      out.push_str("  ");
      out.push_str(&frame);
      out.push('\n');
    }
  }
  out
}

/// Keep only this crate's frames, with source paths cut to their last three parts.
fn our_frames(backtrace: &str) -> Vec<String> {
  let mut frames = Vec::new();
  let mut lines = backtrace.lines().peekable();
  while let Some(line) = lines.next() {
    let trimmed = line.trim_start();
    let Some((index, symbol)) = trimmed.split_once(": ") else {
      continue;
    };
    if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
      continue;
    }
    let at = lines
      .peek()
      .and_then(|next| next.trim_start().strip_prefix("at "))
      .map(str::to_string);
    if at.is_some() {
      lines.next();
    }
    if !symbol.contains("nezzel") {
      continue;
    }
    let symbol = strip_hash(symbol.trim());
    frames.push(match at {
      Some(at) => format!("{symbol} ({})", short_path(at.trim())),
      None => symbol.to_string(),
    });
    if frames.len() == MAX_FRAMES {
      break;
    }
  }
  frames
}

fn strip_hash(symbol: &str) -> &str {
  match symbol.rsplit_once("::h") {
    Some((head, hash)) if hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()) => head,
    _ => symbol,
  }
}

/// `/Users/me/dev/app/src-tauri/src/runners/x.rs:1:2` → `src/runners/x.rs:1:2`.
pub fn short_path(path: &str) -> String {
  let parts: Vec<&str> = path
    .split(['/', '\\'])
    .filter(|part| !part.is_empty() && *part != ".")
    .collect();
  let absolute = path.starts_with('/') || path.starts_with('\\') || path.get(1..2) == Some(":");
  if !absolute && parts.len() <= 3 {
    return parts.join("/");
  }
  parts[parts.len().saturating_sub(3)..].join("/")
}

#[cfg(test)]
mod tests {
  use super::*;

  const BACKTRACE: &str = "   0: std::backtrace_rs::backtrace::libunwind::trace
             at /rustc/abc/library/std/src/../../backtrace/src/backtrace/libunwind.rs:116:5
   1: nezzel_lib::runners::ytdlp_download::spawn::h0123456789abcdef
             at /Users/samira/dev/nezzel/app/src-tauri/src/runners/ytdlp_download.rs:42:9
   2: tokio::runtime::task::raw::poll
             at /Users/samira/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.54.1/src/runtime/task/raw.rs:255:9
   3: <nezzel_lib::state::Config as core::default::Default>::default
   4: main
";

  #[test]
  fn crash_text_keeps_our_frames_and_drops_private_paths() {
    let text = format_crash(
      "index out of bounds for /Users/samira/Movies/فيلم.mp4",
      Some("src/runners/ytdlp_download.rs:42:9"),
      Some("tokio-runtime-worker"),
      BACKTRACE,
      "1.2.0",
      "macOS 15.6 (aarch64)",
    );
    let clean = scrub(&text, &["samira".to_string()]).to_string();
    assert!(
      clean.starts_with("Nezzel 1.2.0 on macOS 15.6 (aarch64)\n"),
      "{clean}"
    );
    assert!(
      clean.contains("thread tokio-runtime-worker panicked at src/runners/ytdlp_download.rs:42:9"),
      "{clean}"
    );
    assert!(
      clean.contains(
        "nezzel_lib::runners::ytdlp_download::spawn (src/runners/ytdlp_download.rs:42:9)"
      ),
      "{clean}"
    );
    assert!(
      clean.contains("<nezzel_lib::state::Config as core::default::Default>::default"),
      "{clean}"
    );
    for gone in [
      "tokio::runtime",
      "libunwind",
      "samira",
      "فيلم",
      "Movies",
      "h0123456789abcdef",
    ] {
      assert!(!clean.contains(gone), "{gone} in {clean}");
    }
  }

  #[test]
  fn a_clean_exit_preserves_only_this_runs_crash_as_nonfatal() {
    let dir = std::env::temp_dir().join(format!("nezzel-exit-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(CRASH_FILE), "earlier run").unwrap();
    let written = AtomicBool::new(false);
    clear_if_written(&dir, &written);
    assert_eq!(read(&dir).as_deref(), Some("earlier run"));
    std::fs::write(dir.join(CRASH_FILE), "background panic").unwrap();
    written.store(true, Ordering::SeqCst);
    clear_if_written(&dir, &written);
    assert_eq!(read(&dir), None);
    assert_eq!(
      std::fs::read_to_string(dir.join("background-panic.txt")).unwrap(),
      "background panic"
    );
    let _ = std::fs::remove_dir_all(dir);
  }

  #[test]
  fn read_keeps_the_file_until_cleared() {
    let dir = std::env::temp_dir().join(format!("nezzel-crash-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(CRASH_FILE), "boom").unwrap();
    assert_eq!(read(&dir).as_deref(), Some("boom"));
    assert_eq!(read(&dir).as_deref(), Some("boom"));
    clear(&dir);
    assert_eq!(read(&dir), None);
    let _ = std::fs::remove_dir_all(dir);
  }

  #[test]
  fn short_path_cases() {
    assert_eq!(short_path("src/lib.rs"), "src/lib.rs");
    assert_eq!(
      short_path(r"C:\Users\Sara\app\src-tauri\src\lib.rs:3:1"),
      "src-tauri/src/lib.rs:3:1"
    );
    assert_eq!(short_path("/home/sara/x/src/a/b.rs:1:1"), "src/a/b.rs:1:1");
  }
}
