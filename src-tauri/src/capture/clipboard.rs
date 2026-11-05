//! Clipboard reads for the capture watcher and paste, the desktop session they depend on, and
//! rate-limited diagnostics. Logs carry kinds only, never clipboard text or URLs.
//!
//! Linux: arboard (behind tauri-plugin-clipboard-manager) talks X11 or the Wayland data-control
//! protocol on its own connection. GNOME/Mutter has no data-control, so arboard falls back to X11
//! through Xwayland, which is unreliable for a native Wayland window. GTK's own clipboard (the one
//! the webview pastes from) works on X11 always and on Wayland while a Nezzel window has focus.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Session {
  X11,
  Wayland,
  /// macOS, Windows, or a Linux session we could not identify.
  Other,
}

/// Session from `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY` (Linux only; pass `None` elsewhere).
pub fn session_from(
  session_type: Option<&str>,
  wayland_display: Option<&str>,
  display: Option<&str>,
) -> Session {
  let set = |v: Option<&str>| v.is_some_and(|v| !v.trim().is_empty());
  match session_type
    .map(|s| s.trim().to_ascii_lowercase())
    .as_deref()
  {
    Some("wayland") => Session::Wayland,
    Some("x11") => Session::X11,
    _ if set(wayland_display) => Session::Wayland,
    _ if set(display) => Session::X11,
    _ => Session::Other,
  }
}

pub fn current_session() -> Session {
  if cfg!(target_os = "linux") {
    let var = |name| std::env::var(name).ok();
    session_from(
      var("XDG_SESSION_TYPE").as_deref(),
      var("WAYLAND_DISPLAY").as_deref(),
      var("DISPLAY").as_deref(),
    )
  } else {
    Session::Other
  }
}

/// Which desktop's words the tray copy uses: GNOME says top bar/Activities, KDE system tray,
/// the rest (XFCE, Cinnamon, MATE, Budgie, ...) notification area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Desktop {
  Gnome,
  Kde,
  Other,
}

impl Desktop {
  pub fn as_str(self) -> &'static str {
    match self {
      Desktop::Gnome => "gnome",
      Desktop::Kde => "kde",
      Desktop::Other => "other",
    }
  }
}

/// Desktop from `XDG_CURRENT_DESKTOP` (colon list, e.g. `ubuntu:GNOME`, `Budgie:GNOME`).
pub fn desktop_from(current_desktop: Option<&str>) -> Desktop {
  let names: Vec<String> = current_desktop
    .unwrap_or_default()
    .split(':')
    .map(|name| name.trim().to_ascii_lowercase())
    .collect();
  let has = |wanted: &str| names.iter().any(|name| name == wanted);
  if has("kde") {
    Desktop::Kde
  } else if has("gnome")
    && !["budgie", "unity", "x-cinnamon", "cinnamon", "pantheon"]
      .iter()
      .any(|name| has(name))
  {
    Desktop::Gnome
  } else {
    Desktop::Other
  }
}

/// Desktop from the session environment: `XDG_CURRENT_DESKTOP` first; when that names neither
/// GNOME nor KDE (unset in some launch paths), KDE's own `KDE_FULL_SESSION`, then the session
/// names (`XDG_SESSION_DESKTOP=KDE`, `DESKTOP_SESSION=plasma`/`plasmawayland`/`gnome`).
pub fn desktop_from_env(var: impl Fn(&str) -> Option<String>) -> Desktop {
  let current = var("XDG_CURRENT_DESKTOP").filter(|value| !value.trim().is_empty());
  let from_current = desktop_from(current.as_deref());
  if from_current != Desktop::Other {
    return from_current;
  }
  let sessions: Vec<String> = ["XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
    .iter()
    .filter_map(|name| var(name))
    .flat_map(|value| {
      value
        .split(':')
        // DESKTOP_SESSION may be a path to the session file.
        .map(|part| {
          part
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
        })
        .collect::<Vec<_>>()
    })
    .collect();
  let kde_session = sessions
    .iter()
    .any(|name| name == "kde" || name.starts_with("plasma"));
  if var("KDE_FULL_SESSION").is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
    || kde_session
  {
    return Desktop::Kde;
  }
  // GNOME only when nothing named another desktop (Budgie, Cinnamon, ... say so in XDG_CURRENT_DESKTOP).
  let gnome_session = sessions.iter().any(|name| {
    matches!(
      name.as_str(),
      "gnome" | "gnome-wayland" | "gnome-xorg" | "ubuntu" | "ubuntu-wayland" | "ubuntu-xorg"
    )
  });
  if current.is_none() && gnome_session {
    Desktop::Gnome
  } else {
    Desktop::Other
  }
}

pub fn current_desktop() -> Desktop {
  if cfg!(target_os = "linux") {
    desktop_from_env(|name| std::env::var(name).ok())
  } else {
    Desktop::Other
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

/// Whether tauri-plugin-global-shortcut (X11 key grabs on Linux) can receive presses. Under Wayland the grab
/// lands on Xwayland and only fires while an X11 window has focus, so registering "succeeds" but never fires.
pub fn global_shortcut_reason(session: Session) -> Option<&'static str> {
  (session == Session::Wayland).then_some("wayland")
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
  fn session_table() {
    let cases = [
      (Some("wayland"), None, Some(":0"), Session::Wayland),
      (Some("x11"), Some("wayland-0"), Some(":10"), Session::X11),
      (Some("tty"), Some("wayland-0"), None, Session::Wayland),
      (None, None, Some(":10"), Session::X11),
      (None, Some(" "), Some(""), Session::Other),
      (Some("Wayland "), None, None, Session::Wayland),
    ];
    for (kind, wl, x, expected) in cases {
      assert_eq!(session_from(kind, wl, x), expected, "{kind:?} {wl:?} {x:?}");
    }
  }

  #[test]
  fn background_and_shortcut_follow_the_session() {
    assert!(background_clipboard(Session::X11, false));
    assert!(background_clipboard(Session::Wayland, true));
    assert!(!background_clipboard(Session::Wayland, false));
    assert!(background_clipboard(Session::Other, false));
  }

  #[test]
  fn desktop_picks_the_tray_words() {
    for (value, expected) in [
      (Some("GNOME"), Desktop::Gnome),
      (Some("ubuntu:GNOME"), Desktop::Gnome),
      (Some("pop:GNOME"), Desktop::Gnome),
      (Some("KDE"), Desktop::Kde),
      (Some("kde"), Desktop::Kde),
      (Some("Budgie:GNOME"), Desktop::Other),
      (Some("X-Cinnamon"), Desktop::Other),
      (Some("XFCE"), Desktop::Other),
      (Some("MATE"), Desktop::Other),
      (Some("Unity:Unity7:ubuntu"), Desktop::Other),
      (Some(""), Desktop::Other),
      (None, Desktop::Other),
    ] {
      assert_eq!(desktop_from(value), expected, "{value:?}");
    }
    // Fallbacks when XDG_CURRENT_DESKTOP is missing or names neither (Fedora KDE report, #36).
    for (vars, expected) in [
      (&[("XDG_CURRENT_DESKTOP", "KDE")][..], Desktop::Kde),
      (&[("KDE_FULL_SESSION", "true")][..], Desktop::Kde),
      (
        &[("XDG_CURRENT_DESKTOP", ""), ("XDG_SESSION_DESKTOP", "KDE")][..],
        Desktop::Kde,
      ),
      (&[("DESKTOP_SESSION", "plasma")][..], Desktop::Kde),
      (&[("DESKTOP_SESSION", "plasmawayland")][..], Desktop::Kde),
      (
        &[("DESKTOP_SESSION", "/usr/share/xsessions/plasmax11")][..],
        Desktop::Kde,
      ),
      (&[("DESKTOP_SESSION", "gnome")][..], Desktop::Gnome),
      (&[("XDG_SESSION_DESKTOP", "ubuntu")][..], Desktop::Gnome),
      // An explicit other desktop wins over a GNOME session name.
      (
        &[
          ("XDG_CURRENT_DESKTOP", "XFCE"),
          ("DESKTOP_SESSION", "gnome"),
        ][..],
        Desktop::Other,
      ),
      (
        &[
          ("XDG_CURRENT_DESKTOP", "Budgie:GNOME"),
          ("DESKTOP_SESSION", "budgie-desktop"),
        ][..],
        Desktop::Other,
      ),
      (
        &[
          ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
          ("KDE_FULL_SESSION", "true"),
        ][..],
        Desktop::Gnome,
      ),
      (&[("KDE_FULL_SESSION", "false")][..], Desktop::Other),
      (&[][..], Desktop::Other),
    ] {
      let found = desktop_from_env(|name| {
        vars
          .iter()
          .find(|(key, _)| *key == name)
          .map(|(_, value)| value.to_string())
      });
      assert_eq!(found, expected, "{vars:?}");
    }
    assert_eq!(global_shortcut_reason(Session::Wayland), Some("wayland"));
    assert_eq!(global_shortcut_reason(Session::X11), None);
    assert_eq!(global_shortcut_reason(Session::Other), None);
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
