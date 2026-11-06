//! What kind of desktop Nezzel runs in: the OS, the Linux display session and desktop family, and what
//! the session allows (global shortcuts). Window, tray, background and capture code read it from here.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Session {
  X11,
  Wayland,
  /// macOS, Windows, or a Linux session we could not identify.
  Other,
}

impl Session {
  /// Every value, as the webview's `CaptureSession` (`crate::contract`).
  #[cfg_attr(not(test), allow(dead_code))]
  pub const ALL: [Self; 3] = [Self::X11, Self::Wayland, Self::Other];

  #[cfg_attr(not(test), allow(dead_code))]
  pub fn as_str(self) -> &'static str {
    match self {
      Session::X11 => "x11",
      Session::Wayland => "wayland",
      Session::Other => "other",
    }
  }
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
  /// Every value, as the webview's `LinuxDesktop` (`crate::contract`).
  #[cfg_attr(not(test), allow(dead_code))]
  pub const ALL: [Self; 3] = [Self::Gnome, Self::Kde, Self::Other];

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

/// The running shell when the environment names no desktop at all (launched from a terminal outside
/// the session, `sudo -u`, a remote shell, some autostart paths): Plasma's or GNOME's own processes.
pub fn desktop_from_processes<S: AsRef<str>>(names: impl IntoIterator<Item = S>) -> Desktop {
  let mut found = Desktop::Other;
  for name in names {
    match name.as_ref().trim() {
      "plasmashell" | "kwin_wayland" | "kwin_x11" | "ksmserver" => return Desktop::Kde,
      "gnome-shell" => found = Desktop::Gnome,
      _ => {}
    }
  }
  found
}

/// No session variable says which desktop this is.
fn desktop_env_unset(var: &impl Fn(&str) -> Option<String>) -> bool {
  [
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
    "KDE_FULL_SESSION",
  ]
  .iter()
  .all(|name| var(name).is_none_or(|value| value.trim().is_empty()))
}

/// Environment first; the process list only when the environment is silent.
pub fn detect_desktop<S: AsRef<str>>(
  var: impl Fn(&str) -> Option<String>,
  processes: impl FnOnce() -> Vec<S>,
) -> Desktop {
  let found = desktop_from_env(&var);
  if found == Desktop::Other && desktop_env_unset(&var) {
    desktop_from_processes(processes())
  } else {
    found
  }
}

/// Short process names (`/proc/<pid>/comm`), bounded.
fn process_names() -> Vec<String> {
  let Ok(entries) = std::fs::read_dir("/proc") else {
    return Vec::new();
  };
  entries
    .flatten()
    .filter(|entry| {
      entry
        .file_name()
        .to_string_lossy()
        .bytes()
        .all(|b| b.is_ascii_digit())
    })
    .take(8192)
    .filter_map(|entry| std::fs::read_to_string(entry.path().join("comm")).ok())
    .collect()
}

/// `current_desktop` for async callers: the first call may scan /proc, so it runs off the runtime.
pub async fn current_desktop_blocking() -> Desktop {
  tauri::async_runtime::spawn_blocking(current_desktop)
    .await
    .unwrap_or(Desktop::Other)
}

pub fn current_desktop() -> Desktop {
  static DESKTOP: std::sync::OnceLock<Desktop> = std::sync::OnceLock::new();
  if cfg!(target_os = "linux") {
    *DESKTOP.get_or_init(|| detect_desktop(|name| std::env::var(name).ok(), process_names))
  } else {
    Desktop::Other
  }
}

/// Whether tauri-plugin-global-shortcut (X11 key grabs on Linux) can receive presses. Under Wayland the grab
/// lands on Xwayland and only fires while an X11 window has focus, so registering "succeeds" but never fires.
pub fn global_shortcut_reason(session: Session) -> Option<&'static str> {
  (session == Session::Wayland).then_some("wayland")
}

/// The OS word the webview and usage counts use.
pub fn os_name() -> &'static str {
  if cfg!(target_os = "macos") {
    "mac"
  } else if cfg!(target_os = "windows") {
    "windows"
  } else {
    "linux"
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

  /// Fedora KDE re-test (#36): the app saw no desktop variables and said "notification area".
  #[test]
  fn silent_environment_falls_back_to_the_running_shell() {
    let none = |_: &str| None::<String>;
    let plasma = || vec!["systemd\n", "kwin_wayland\n", "plasmashell\n"];
    assert_eq!(detect_desktop(none, plasma), Desktop::Kde);
    assert_eq!(
      detect_desktop(none, || vec!["gnome-shell\n"]),
      Desktop::Gnome
    );
    assert_eq!(
      detect_desktop(none, || vec!["xfce4-panel\n"]),
      Desktop::Other
    );
    assert_eq!(detect_desktop(none, Vec::<String>::new), Desktop::Other);
    // A named desktop wins over the process list (XFCE with a stray gnome-shell stays XFCE).
    let xfce = |name: &str| (name == "XDG_CURRENT_DESKTOP").then(|| "XFCE".to_string());
    assert_eq!(
      detect_desktop(xfce, || vec!["gnome-shell\n"]),
      Desktop::Other
    );
    let kde = |name: &str| (name == "XDG_CURRENT_DESKTOP").then(|| "KDE".to_string());
    assert_eq!(
      detect_desktop(kde, || -> Vec<String> { panic!("env is enough") }),
      Desktop::Kde
    );
  }
}
