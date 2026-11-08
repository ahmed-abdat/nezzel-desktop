//! Clipboard reads and writes (by `crate::platform` session), and
//! rate-limited diagnostics. Logs carry kinds only, never clipboard text or URLs.
//!
//! Linux: arboard (behind tauri-plugin-clipboard-manager) talks X11 or the Wayland data-control
//! protocol on its own connection. GNOME/Mutter has no data-control, so arboard falls back to X11
//! through Xwayland, which is unreliable for a native Wayland window. GTK's own clipboard (the one
//! the webview pastes from) works on X11 always and on Wayland while a Nezzel window has focus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use super::state::CaptureState;
use crate::platform::Session;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Writer {
  Plugin,
  Gtk,
}

/// GTK owns the native Wayland selection without requiring data-control. Keep other platforms'
/// existing plugin writer, including X11. Data-control availability does not affect this choice.
fn writer_for(session: Session, linux: bool) -> Writer {
  if linux && session == Session::Wayland {
    Writer::Gtk
  } else {
    Writer::Plugin
  }
}

fn write_locked(
  state: &CaptureState,
  text: &str,
  suppress_offer: bool,
  write: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
  // Lock where the write actually runs, not while awaiting main-thread dispatch. A watcher that
  // reads the new selection must wait until its self-write suppression has been recorded.
  let mut policy = state.policy.lock().unwrap();
  write()?;
  if suppress_offer {
    policy.own_write(text, state.started.elapsed());
  }
  Ok(())
}

#[cfg(target_os = "linux")]
fn write_gtk(text: &str) -> Result<(), String> {
  let display = gtk::gdk::Display::default().ok_or("clipboard unavailable")?;
  let clipboard = gtk::Clipboard::default(&display).ok_or("clipboard unavailable")?;
  let targets: Vec<_> = [
    "UTF8_STRING",
    "COMPOUND_TEXT",
    "TEXT",
    "STRING",
    "text/plain;charset=utf-8",
    "text/plain",
  ]
  .iter()
  .map(|target| gtk::TargetEntry::new(target, gtk::TargetFlags::empty(), 0))
  .collect();
  let text = text.to_owned();
  // Unlike set_text, set_with_data reports failure to acquire the selection. GTK retains the
  // closure until ownership changes, serving text to other apps even after our window loses focus.
  if clipboard.set_with_data(&targets, move |_, selection, _| {
    selection.set_text(&text);
  }) {
    Ok(())
  } else {
    Err("clipboard unavailable".into())
  }
}

#[cfg(not(target_os = "linux"))]
fn write_gtk(_: &str) -> Result<(), String> {
  Err("clipboard unavailable".into())
}

/// All app text writes share the native writer. Only welcome's intentional "Try it" copy should
/// pass false: its purpose is to offer the sample through the watcher.
pub async fn write_text(app: &AppHandle, text: String, suppress_offer: bool) -> Result<(), String> {
  let writer = writer_for(
    crate::platform::current_session(),
    cfg!(target_os = "linux"),
  );
  let handle = app.clone();
  let write = move || {
    let state = handle.state::<CaptureState>();
    write_locked(&state, &text, suppress_offer, || match writer {
      Writer::Gtk => write_gtk(&text),
      Writer::Plugin => handle
        .clipboard()
        .write_text(&text)
        .map_err(|_| String::from("clipboard unavailable")),
    })
  };
  match writer {
    Writer::Plugin => write(),
    Writer::Gtk => {
      let (tx, rx) = tokio::sync::oneshot::channel();
      app
        .run_on_main_thread(move || {
          let _ = tx.send(write());
        })
        .map_err(|_| "clipboard unavailable")?;
      rx.await.map_err(|_| "clipboard unavailable")?
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
  /// arboard through the clipboard plugin.
  Plugin,
  /// GTK on the main thread (Linux only).
  Gtk,
}

/// Which readers to try, in order. Wayland with data-control: arboard sees copies in the background,
/// GTK only while focused (and may hold a stale offer), so arboard first. Everywhere else on Linux GTK
/// first: it is the path native paste uses.
pub fn read_order(session: Session, data_control: bool) -> &'static [Reader] {
  if !cfg!(target_os = "linux") {
    return &[Reader::Plugin];
  }
  match (session, data_control) {
    (Session::Wayland, true) => &[Reader::Plugin, Reader::Gtk],
    _ => &[Reader::Gtk, Reader::Plugin],
  }
}

/// Whether a copy in another app can be seen while no Nezzel window has focus.
pub fn background_clipboard(session: Session, data_control: bool) -> bool {
  session != Session::Wayland || data_control
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
  /// Empty or not text (arboard ContentNotAvailable, GTK no text).
  Empty,
  NotSupported,
  Occupied,
  /// No answer within `READ_LIMIT`, or the previous read is still stuck.
  Timeout,
  /// Clipboard init failed or an unexpected backend error.
  Other,
}

impl ReadError {
  pub fn kind(self) -> &'static str {
    match self {
      Self::Empty => "empty",
      Self::NotSupported => "not-supported",
      Self::Occupied => "occupied",
      Self::Timeout => "timeout",
      Self::Other => "other",
    }
  }
}

/// Maps the plugin's error text (arboard's Display strings) to a kind.
pub fn classify(message: &str) -> ReadError {
  let m = message.to_ascii_lowercase();
  if m.contains("not available in the requested format") || m.contains("clipboard is empty") {
    ReadError::Empty
  } else if m.contains("not supported") {
    ReadError::NotSupported
  } else if m.contains("held by another party") {
    ReadError::Occupied
  } else {
    ReadError::Other
  }
}

/// Probe once whether the compositor offers a data-control protocol (wlr or ext). Reads MIME types only,
/// never content. Blocking: call off the main thread.
#[cfg(target_os = "linux")]
pub fn probe_data_control() -> bool {
  use wl_clipboard_rs::paste::{get_mime_types, ClipboardType, Error, Seat};
  // Empty or non-text clipboards still prove the protocol is there; anything else means no background reads.
  matches!(
    get_mime_types(ClipboardType::Regular, Seat::Unspecified),
    Ok(_) | Err(Error::ClipboardEmpty | Error::NoMimeType)
  )
}

#[cfg(not(target_os = "linux"))]
pub fn probe_data_control() -> bool {
  false
}

/// How long one clipboard read (or the data-control probe) may take before it counts as `timeout`.
pub const READ_LIMIT: Duration = Duration::from_millis(750);

/// A plugin read or probe still running: a stalled owner or compositor can block arboard indefinitely.
static PLUGIN_BUSY: AtomicBool = AtomicBool::new(false);
static PROBE_BUSY: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "linux")]
static GTK_PENDING: AtomicBool = AtomicBool::new(false);

struct Release(&'static AtomicBool);
impl Drop for Release {
  fn drop(&mut self) {
    self.0.store(false, Ordering::Release);
  }
}

/// Runs a blocking read on a worker, at most one per `busy` flag at a time, and gives up after `limit`.
/// A timed-out read keeps its worker until it returns; until then later calls fail fast with `Timeout`
/// instead of stacking more stuck threads.
pub async fn bounded<T: Send + 'static>(
  busy: &'static AtomicBool,
  limit: Duration,
  read: impl FnOnce() -> Result<T, ReadError> + Send + 'static,
) -> Result<T, ReadError> {
  if busy.swap(true, Ordering::AcqRel) {
    return Err(ReadError::Timeout);
  }
  let worker = tauri::async_runtime::spawn_blocking(move || {
    let _release = Release(busy);
    read()
  });
  match tokio::time::timeout(limit, worker).await {
    Ok(Ok(result)) => result,
    Ok(Err(_)) => Err(ReadError::Other),
    Err(_) => Err(ReadError::Timeout),
  }
}

/// Data-control probe with the read limit; a stalled compositor counts as "no background reads".
pub async fn probe_data_control_bounded() -> bool {
  bounded(&PROBE_BUSY, READ_LIMIT, || Ok(probe_data_control()))
    .await
    .unwrap_or(false)
}

fn read_plugin(app: &AppHandle) -> Result<String, ReadError> {
  app
    .clipboard()
    .read_text()
    .map_err(|e| classify(&e.to_string()))
}

#[cfg(target_os = "linux")]
async fn read_gtk(app: &AppHandle) -> Result<String, ReadError> {
  // One request in flight: a stalled owner must not pile up GTK callbacks.
  if GTK_PENDING.swap(true, Ordering::AcqRel) {
    return Err(ReadError::Timeout);
  }
  let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
  let dispatched = app.run_on_main_thread(move || {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    // Async request: never block the GTK loop, which may be the owner that has to answer.
    clipboard.request_text(move |_, text| {
      GTK_PENDING.store(false, Ordering::Release);
      let _ = tx.send(text.map(String::from));
    });
  });
  if dispatched.is_err() {
    GTK_PENDING.store(false, Ordering::Release);
    return Err(ReadError::Other);
  }
  match tokio::time::timeout(READ_LIMIT, rx).await {
    Ok(Ok(Some(text))) => Ok(text),
    Ok(Ok(None)) => Err(ReadError::Empty),
    Ok(Err(_)) => Err(ReadError::Other),
    Err(_) => Err(ReadError::Timeout),
  }
}

#[cfg(not(target_os = "linux"))]
async fn read_gtk(_: &AppHandle) -> Result<String, ReadError> {
  Err(ReadError::NotSupported)
}

/// Reads clipboard text with the readers in `order`; returns the first text and which reader gave it,
/// or the last error. Never call from the main thread (the plugin reader can deadlock there on Linux).
pub async fn read_text(
  app: &AppHandle,
  order: &[Reader],
) -> (Result<String, ReadError>, Option<Reader>) {
  let mut last = Err(ReadError::Other);
  for &reader in order {
    let result = match reader {
      Reader::Plugin => {
        let app = app.clone();
        bounded(&PLUGIN_BUSY, READ_LIMIT, move || read_plugin(&app)).await
      }
      Reader::Gtk => read_gtk(app).await,
    };
    match result {
      Ok(text) if !text.is_empty() => return (Ok(text), Some(reader)),
      Ok(_) => last = Err(ReadError::Empty),
      Err(e) => last = Err(e),
    }
  }
  (last, None)
}

/// One diagnostic line per state change, at most `BUDGET` lines per `WINDOW`.
#[derive(Default)]
pub struct Diagnostics {
  last: std::collections::HashMap<&'static str, String>,
  window_start: Option<Instant>,
  used: u32,
}

const BUDGET: u32 = 20;
const WINDOW: Duration = Duration::from_secs(600);

impl Diagnostics {
  /// True when `value` differs from the last value of `topic` and the budget allows a line.
  pub fn should_log(&mut self, topic: &'static str, value: &str, now: Instant) -> bool {
    if self.last.get(topic).is_some_and(|v| v == value) {
      return false;
    }
    self.last.insert(topic, value.to_string());
    if self
      .window_start
      .is_none_or(|start| now.duration_since(start) >= WINDOW)
    {
      self.window_start = Some(now);
      self.used = 0;
    }
    if self.used >= BUDGET {
      return false;
    }
    self.used += 1;
    true
  }

  pub fn note(&mut self, topic: &'static str, value: &str) {
    if self.should_log(topic, value, Instant::now()) {
      tracing::info!(topic, state = value, "Clipboard watch");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn write_path_follows_the_session() {
    for (linux, session, expected) in [
      (true, Session::X11, Writer::Plugin),
      (true, Session::Other, Writer::Plugin),
      (true, Session::Wayland, Writer::Gtk),
      (false, Session::X11, Writer::Plugin),
      (false, Session::Other, Writer::Plugin),
      (false, Session::Wayland, Writer::Plugin),
    ] {
      assert_eq!(
        writer_for(session, linux),
        expected,
        "{session:?} linux={linux}"
      );
    }
  }

  #[test]
  fn a_copy_holds_the_policy_lock_and_records_the_successful_write() {
    let state = CaptureState::default();
    let text = super::super::commands::TRY_LINK;
    let mut written = None;
    write_locked(&state, text, true, || {
      assert!(state.policy.try_lock().is_err());
      written = Some(text.to_owned());
      Ok(())
    })
    .unwrap();
    assert_eq!(written.as_deref(), Some(text));
    assert_eq!(
      state
        .policy
        .lock()
        .unwrap()
        .decide(text, state.started.elapsed())
        .kind(),
      "own-write"
    );
  }

  #[test]
  fn a_failed_write_surfaces_its_error_and_does_not_suppress_a_later_copy() {
    let state = CaptureState::default();
    let text = super::super::commands::TRY_LINK;
    assert_eq!(
      write_locked(&state, text, true, || Err("clipboard unavailable".into())),
      Err("clipboard unavailable".into())
    );
    assert!(state
      .policy
      .lock()
      .unwrap()
      .decide(text, state.started.elapsed())
      .offer()
      .is_some());
  }

  #[test]
  fn the_try_it_sample_is_written_but_still_offered() {
    let state = CaptureState::default();
    let text = super::super::commands::TRY_LINK;
    let mut written = false;
    write_locked(&state, text, false, || {
      written = true;
      Ok(())
    })
    .unwrap();
    assert!(written);
    assert!(state
      .policy
      .lock()
      .unwrap()
      .decide(text, state.started.elapsed())
      .offer()
      .is_some());
  }

  #[test]
  fn background_and_shortcut_follow_the_session() {
    assert!(background_clipboard(Session::X11, false));
    assert!(background_clipboard(Session::Wayland, true));
    assert!(!background_clipboard(Session::Wayland, false));
    assert!(background_clipboard(Session::Other, false));
  }

  #[test]
  fn read_order_table() {
    if cfg!(target_os = "linux") {
      assert_eq!(
        read_order(Session::Wayland, true),
        [Reader::Plugin, Reader::Gtk]
      );
      assert_eq!(
        read_order(Session::Wayland, false),
        [Reader::Gtk, Reader::Plugin]
      );
      assert_eq!(
        read_order(Session::X11, false),
        [Reader::Gtk, Reader::Plugin]
      );
    } else {
      assert_eq!(read_order(Session::Other, false), [Reader::Plugin]);
    }
  }

  #[test]
  fn classify_arboard_messages() {
    let cases = [
      ("The clipboard contents were not available in the requested format or the clipboard is empty.", ReadError::Empty),
      ("The selected clipboard is not supported with the current system configuration.", ReadError::NotSupported),
      ("The native clipboard is not accessible due to being held by another party.", ReadError::Occupied),
      ("Unknown error while interacting with the clipboard: Connection refused", ReadError::Other),
    ];
    for (message, expected) in cases {
      assert_eq!(classify(message), expected, "{message}");
    }
  }

  #[tokio::test]
  async fn a_stuck_read_times_out_and_never_stacks() {
    static BUSY: AtomicBool = AtomicBool::new(false);
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let limit = Duration::from_millis(50);
    let stuck = bounded(&BUSY, limit, move || {
      let _ = wait.recv();
      Ok("late")
    });
    assert_eq!(stuck.await, Err(ReadError::Timeout));
    // The stuck worker still holds the slot: the next read fails fast without starting another worker.
    let started = std::sync::Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let next = bounded(&BUSY, limit, move || {
      flag.store(true, Ordering::SeqCst);
      Ok("next")
    });
    assert_eq!(next.await, Err(ReadError::Timeout));
    assert!(!started.load(Ordering::SeqCst));
    // Once it returns, reads work again.
    release.send(()).unwrap();
    for _ in 0..2000 {
      if !BUSY.load(Ordering::Acquire) {
        break;
      }
      tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(bounded(&BUSY, limit, || Ok("ok")).await, Ok("ok"));
  }

  #[test]
  fn diagnostics_log_changes_only_within_budget() {
    let mut d = Diagnostics::default();
    let t0 = Instant::now();
    assert!(d.should_log("read", "ok", t0));
    assert!(!d.should_log("read", "ok", t0), "same state twice");
    assert!(d.should_log("read", "empty", t0));
    assert!(d.should_log("ready", "true", t0), "topics are independent");
    // Flapping states stop logging once the budget is spent, then resume in the next window.
    let mut logged = 3;
    for i in 0..100 {
      if d.should_log("read", if i % 2 == 0 { "ok" } else { "empty" }, t0) {
        logged += 1;
      }
    }
    assert_eq!(logged, BUDGET);
    assert!(d.should_log("read", "timeout", t0 + WINDOW));
  }
}
