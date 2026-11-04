//! Crash and error reports: a scrubbed diagnostic report, a local crash catcher and
//! automatic delivery to our own Sentry ([`send`]), on by default with a switch in Settings.

pub mod crash;
pub mod file_log;
pub mod link;
pub mod redact;
pub mod report;
pub mod send;

use crate::binaries::binaries_manager::BinariesManager;
use crate::paths::PathsManager;
use crate::SharedConfig;
use report::{DownloadContext, OsInfo, Report, ReportInput};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

pub fn logs_dir(app_dir: &std::path::Path) -> PathBuf {
  app_dir.join("logs")
}

/// Start the file log and the panic hook once the app dir is known.
pub fn setup(app: &AppHandle, paths: &PathsManager) {
  let app_dir = paths.app_dir().clone();
  file_log::attach_dir(&logs_dir(&app_dir));
  crash::install(
    app_dir,
    app.package_info().version.to_string(),
    os_line(&os_info()),
  );
}

/// On a clean exit the app did not close unexpectedly.
pub fn on_clean_exit(app: &AppHandle) {
  if let Some(paths) = app.try_state::<PathsManager>() {
    crash::clear(paths.app_dir());
  }
}

fn os_line(os: &OsInfo) -> String {
  format!("{} {} ({})", os.name, os.version, os.arch)
}

pub fn os_info() -> OsInfo {
  let mut info = OsInfo {
    arch: std::env::consts::ARCH.into(),
    ..OsInfo::default()
  };
  #[cfg(target_os = "macos")]
  {
    info.name = "macOS".into();
    info.version = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
      .ok()
      .and_then(|text| report::parse_macos_plist(&text))
      .unwrap_or_else(|| "unknown".into());
  }
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    info.name = "Windows".into();
    info.version = std::process::Command::new("cmd")
      .args(["/C", "ver"])
      .creation_flags(0x08000000) // CREATE_NO_WINDOW
      .output()
      .ok()
      .and_then(|out| report::parse_windows_ver(&String::from_utf8_lossy(&out.stdout)))
      .unwrap_or_else(|| "unknown".into());
  }
  #[cfg(target_os = "linux")]
  {
    info.name = "Linux".into();
    info.version = std::fs::read_to_string("/proc/sys/kernel/osrelease")
      .map(|text| report::token(text.trim()))
      .unwrap_or_else(|_| "unknown".into());
    info.distro = std::fs::read_to_string("/etc/os-release")
      .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
      .ok()
      .and_then(|text| report::parse_os_release(&text))
      .map(|distro| redact::scrub(&distro, &[]));
    let env = |key: &str| {
      std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| report::token(&v))
    };
    info.desktop = env("XDG_CURRENT_DESKTOP");
    info.session = env("XDG_SESSION_TYPE");
  }
  if info.name.is_empty() {
    info.name = std::env::consts::OS.into();
    info.version = "unknown".into();
  }
  info
}

fn collect(
  app: &AppHandle,
  download: Option<DownloadContext>,
  crash: Option<String>,
  note: Option<String>,
  with_log: bool,
) -> ReportInput {
  let config = app.state::<SharedConfig>().load();
  let mut tools = app
    .try_state::<BinariesManager>()
    .map(|manager| manager.tool_versions())
    .unwrap_or_default();
  let pinned = |manifest: &str| {
    serde_json::from_str::<serde_json::Value>(manifest)
      .ok()
      .and_then(|value| value.get("version")?.as_str().map(report::token))
      .unwrap_or_else(|| "unknown".into())
  };
  tools.push((
    "gallery-dl".into(),
    if crate::runners::gallery::available(app) {
      pinned(include_str!("../../../scripts/gallery-dl-manifest.json"))
    } else {
      "missing".into()
    },
  ));
  tools.push((
    "aria2".into(),
    if crate::runners::transport::resolve_aria2(app).is_some() {
      pinned(include_str!("../../../scripts/aria2-manifest.json"))
    } else {
      "missing".into()
    },
  ));
  ReportInput {
    note,
    app_version: app.package_info().version.to_string(),
    os: os_info(),
    tools,
    locale: (
      report::token(&config.appearance.language),
      report::token(&sys_locale::get_locale().unwrap_or_else(|| "unknown".into())),
    ),
    settings: report::settings_summary(&config),
    download,
    crash,
    // Automatic events carry structured tokens only; the log tail goes with manual sends.
    log_lines: if with_log {
      file_log::recent_lines(report::LOG_LINES)
    } else {
      Vec::new()
    },
  }
}

/// A crash the user chose not to send automatically, kept for one "Send report" click.
#[derive(Default)]
pub struct PendingCrash(Mutex<Option<Report>>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashNotice {
  /// Sent automatically; false means automatic reports are off (or no DSN in this build).
  pub sent: bool,
}

pub use link::link_for_report;

/// Fill the row's site and link (one reduction rule: [`link_for_report`]).
fn with_link(
  download: Option<DownloadContext>,
  url: Option<&str>,
  extractor: Option<&str>,
  include_full: bool,
) -> Option<DownloadContext> {
  download.map(|mut download| {
    let link = url.and_then(|url| link_for_report(url, include_full));
    download.site = link.as_ref().map(|link| link.site.clone());
    download.link = link.map(|link| link.link);
    download.extractor = extractor.map(report::token);
    download
  })
}

fn clean_note(note: Option<String>) -> Option<String> {
  note.map(|note| {
    let note: String = note.chars().take(report::MAX_NOTE_CHARS).collect();
    redact::scrub(&note, &redact::local_names())
  })
}

/// The report text, for "Copy details" when sending is not possible.
#[tauri::command]
pub fn diagnostics_report(
  app: AppHandle,
  note: Option<String>,
  download: Option<DownloadContext>,
  url: Option<String>,
  extractor: Option<String>,
  include_link: Option<bool>,
) -> Report {
  let download = with_link(
    download,
    url.as_deref(),
    extractor.as_deref(),
    include_link.unwrap_or(false),
  );
  report::build(&collect(&app, download, None, clean_note(note), true))
}

/// "Report a problem": sends now, even with automatic reports off (the user asked).
#[tauri::command]
pub async fn diagnostics_send(
  app: AppHandle,
  note: Option<String>,
  download: Option<DownloadContext>,
  url: Option<String>,
  extractor: Option<String>,
  include_link: Option<bool>,
) -> Result<(), String> {
  let download = with_link(
    download,
    url.as_deref(),
    extractor.as_deref(),
    include_link.unwrap_or(false),
  );
  let report = report::build(&collect(&app, download, None, clean_note(note), true));
  tauri::async_runtime::spawn_blocking(move || send::send_manual(&report.body, &report.title))
    .await
    .map_err(|error| error.to_string())?
}

/// A download failed: sent automatically when reports are on. Returns whether it was sent.
#[tauri::command]
pub fn diagnostics_download_failed(
  app: AppHandle,
  download: DownloadContext,
  url: Option<String>,
  extractor: Option<String>,
) -> bool {
  let Some(download) = with_link(Some(download), url.as_deref(), extractor.as_deref(), false)
  else {
    return false;
  };
  let report = report::build(&collect(&app, Some(download.clone()), None, None, false));
  send::send_failure(
    &app,
    &report.body,
    &download,
    download.site.as_deref(),
    download.extractor.as_deref(),
  )
}

/// The previous run's crash, handled once: the file is deleted as it is read, then sent
/// automatically when reports are on, or kept for one "Send report" click.
#[tauri::command]
pub fn diagnostics_crash_check(app: AppHandle, auto: Option<bool>) -> Option<CrashNotice> {
  let paths = app.try_state::<PathsManager>()?;
  let crash = crash::take(paths.app_dir())?;
  // Already scrubbed at write time; scrub again in case the file came from an older build.
  let crash = redact::scrub(&crash, &redact::local_names());
  let message = crash.lines().nth(2).unwrap_or("crash").to_string();
  let report = report::build(&collect(&app, None, Some(crash.clone()), None, false));
  // `auto: false`: the user has not seen that reports are on yet; offer instead of sending.
  let sent = auto.unwrap_or(true) && send::send_crash(&app, &report.body, &message);
  if !sent {
    // Sent only on the user's click: include the log tail like any manual report.
    let report = report::build(&collect(&app, None, Some(crash), None, true));
    if let Some(pending) = app.try_state::<PendingCrash>() {
      if let Ok(mut slot) = pending.0.lock() {
        *slot = Some(report);
      }
    }
  }
  Some(CrashNotice { sent })
}

#[tauri::command]
pub async fn diagnostics_send_crash(app: AppHandle) -> Result<(), String> {
  let report = app
    .try_state::<PendingCrash>()
    .and_then(|pending| pending.0.lock().ok()?.take())
    .ok_or("no crash to send")?;
  tauri::async_runtime::spawn_blocking(move || send::send_manual(&report.body, &report.title))
    .await
    .map_err(|error| error.to_string())?
}

/// Frontend errors (window.onerror, unhandledrejection, Vue errorHandler): app log, and a
/// report when reports are on.
#[tauri::command]
pub fn diagnostics_log(app: AppHandle, level: String, message: String) {
  let message: String = message.chars().take(2_000).collect();
  let clean = redact::scrub(&message, &redact::local_names());
  if level == "warn" {
    tracing::warn!(target: "frontend", "{clean}");
  } else {
    tracing::error!(target: "frontend", "{clean}");
    send::send_frontend_error(&app, &clean);
  }
}
