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
use redact::ScrubbedText;
use report::{DownloadContext, ExtractorToken, OsInfo, Report, ReportInput, Token};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    crash::clear_on_clean_exit(paths.app_dir());
  }
}

fn os_line(os: &OsInfo) -> String {
  format!("{} {} ({})", os.name, os.version, os.arch)
}

pub fn os_info() -> OsInfo {
  let mut info = OsInfo {
    arch: std::env::consts::ARCH,
    ..OsInfo::default()
  };
  #[cfg(target_os = "macos")]
  {
    info.name = "macOS";
    info.version = Token::new(
      &std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
        .ok()
        .and_then(|text| report::parse_macos_plist(&text))
        .unwrap_or_else(|| "unknown".into()),
    );
  }
  #[cfg(windows)]
  {
    info.name = "Windows";
    info.version = Token::new(
      &crate::runners::tool_process::command("cmd")
        .and_then(|mut command| command.args(["/C", "ver"]).output())
        .ok()
        .and_then(|out| report::parse_windows_ver(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_else(|| "unknown".into()),
    );
  }
  #[cfg(target_os = "linux")]
  {
    info.name = "Linux";
    info.version = std::fs::read_to_string("/proc/sys/kernel/osrelease")
      .map(|text| Token::new(text.trim()))
      .unwrap_or_else(|_| Token::new("unknown"));
    info.distro = std::fs::read_to_string("/etc/os-release")
      .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
      .ok()
      .and_then(|text| report::parse_os_release(&text))
      .map(|distro| redact::scrub(&distro, &[]));
    let env = |key: &str| {
      std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| Token::new(&v))
    };
    info.desktop = env("XDG_CURRENT_DESKTOP");
    info.session = env("XDG_SESSION_TYPE");
  }
  if info.name.is_empty() {
    info.name = std::env::consts::OS;
    info.version = Token::new("unknown");
  }
  info
}

/// What a report says about this install: app, OS, tools, locale, settings and, for reports the user sends
/// or reads, the newest log lines. The parts that came with the request are added by [`Draft::into_input`].
fn environment(app: &AppHandle, with_log: bool) -> ReportInput {
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
    app_version: app.package_info().version.to_string(),
    os: os_info(),
    tools,
    locale: (
      Token::new(&config.appearance.language),
      Token::new(&sys_locale::get_locale().unwrap_or_else(|| "unknown".into())),
    ),
    settings: report::settings_summary(&config),
    // Automatic events carry structured tokens only; the log tail goes with manual sends.
    log_lines: if with_log {
      file_log::recent_lines(report::LOG_LINES)
    } else {
      Vec::new()
    },
    ..ReportInput::default()
  }
}

/// A report request as it arrives: the raw parts, before scrubbing and reduction.
#[derive(Default)]
struct Draft {
  /// What the user typed in "Report a problem".
  note: Option<String>,
  /// The failed row; `url` and `extractor` fill its site, link and extractor.
  download: Option<DownloadContext>,
  url: Option<String>,
  extractor: Option<String>,
  /// The user left "Include the link" checked.
  include_full_link: bool,
  /// The previous run's crash file.
  crash: Option<String>,
  /// Include the log tail (reports the user sends or reads, never automatic ones).
  with_log: bool,
}

impl Draft {
  /// The one place a request's raw parts become report fields: the note is capped then scrubbed, the link
  /// reduced ([`link_for_report`]), the extractor tokenised, the crash scrubbed. `names`: [`redact::local_names`].
  fn into_input(self, environment: ReportInput, names: &[String]) -> ReportInput {
    let download = self.download.map(|mut download| {
      let link = self
        .url
        .as_deref()
        .and_then(|url| link_for_report(url, self.include_full_link));
      download.site = link.as_ref().map(|link| link.site.clone());
      download.link = link.map(|link| link.link);
      download.extractor = self.extractor.as_deref().map(ExtractorToken::new);
      download
    });
    ReportInput {
      note: self.note.map(|note| {
        // Cap first: a link cut in half at the cap is still removed.
        let note: String = note.chars().take(report::MAX_NOTE_CHARS).collect();
        redact::scrub(&note, names)
      }),
      download,
      crash: self.crash.map(|crash| redact::scrub(&crash, names)),
      ..environment
    }
  }
}

/// Every report is built here: the request's parts on top of [`environment`].
fn report_input(app: &AppHandle, draft: Draft) -> ReportInput {
  let environment = environment(app, draft.with_log);
  draft.into_input(environment, redact::local_names())
}

fn build_report(app: &AppHandle, draft: Draft) -> Report {
  report::build(&report_input(app, draft))
}

/// "Report a problem" and "Copy details": the user's note, the failed row, the log tail.
fn manual_draft(
  note: Option<String>,
  download: Option<DownloadContext>,
  url: Option<String>,
  extractor: Option<String>,
  include_link: Option<bool>,
) -> Draft {
  Draft {
    note,
    download,
    url,
    extractor,
    include_full_link: include_link.unwrap_or(false),
    with_log: true,
    ..Draft::default()
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

use link::link_for_report;

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
  build_report(
    &app,
    manual_draft(note, download, url, extractor, include_link),
  )
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
) -> Result<(), send::SendError> {
  let report = build_report(
    &app,
    manual_draft(note, download, url, extractor, include_link),
  );
  // The frontend says "could not send" and offers "Copy details" for any `SendError`.
  tauri::async_runtime::spawn_blocking(move || send::send_manual(&report))
    .await
    .map_err(|_| send::SendError::NotDelivered)?
}

/// A download failed: sent automatically when reports are on. Returns whether it was sent.
#[tauri::command]
pub fn diagnostics_download_failed(
  app: AppHandle,
  download: DownloadContext,
  url: Option<String>,
  extractor: Option<String>,
) -> bool {
  let input = report_input(
    &app,
    Draft {
      download: Some(download),
      url,
      extractor,
      ..Draft::default()
    },
  );
  let Some(download) = &input.download else {
    return false;
  };
  send::send_failure(&app, &report::build(&input), download)
}

/// The crash file goes only once the crash was sent or staged for the user's click; otherwise it stays, to be
/// offered on the next launch, and there is nothing to show now. A build without reports never offers a send
/// that cannot work.
fn settle_crash(
  dir: &std::path::Path,
  configured: bool,
  send: impl FnOnce() -> bool,
  stage: impl FnOnce() -> bool,
) -> Option<CrashNotice> {
  if !configured {
    return None;
  }
  let sent = send();
  if !sent && !stage() {
    return None;
  }
  crash::clear(dir);
  Some(CrashNotice { sent })
}

/// The previous run's crash, handled once: sent automatically when reports are on, or staged for one
/// "Send report" click. The file is deleted only after one of those worked ([`settle_crash`]).
#[tauri::command]
pub fn diagnostics_crash_check(app: AppHandle, auto: Option<bool>) -> Option<CrashNotice> {
  let paths = app.try_state::<PathsManager>()?;
  // Already scrubbed at write time; scrubbed again (in `Draft::into_input`) in case the file came from an older build.
  let crash = crash::read(paths.app_dir())?;
  let draft = |with_log| Draft {
    crash: Some(crash.clone()),
    with_log,
    ..Draft::default()
  };
  // `auto: false`: the user has not seen that reports are on yet; offer instead of sending.
  let send = || {
    let input = report_input(&app, draft(false));
    let report = report::build(&input);
    auto.unwrap_or(true)
      && input
        .crash
        .as_ref()
        .is_some_and(|crash| send::send_crash(&app, &report, crash))
  };
  settle_crash(paths.app_dir(), send::configured(), send, || {
    // Sent only on the user's click: include the log tail like any manual report.
    let report = build_report(&app, draft(true));
    app
      .try_state::<PendingCrash>()
      .and_then(|pending| pending.0.lock().ok().map(|mut slot| *slot = Some(report)))
      .is_some()
  })
}

#[tauri::command]
pub async fn diagnostics_send_crash(app: AppHandle) -> Result<(), send::SendError> {
  // A copy: the staged crash stays until it was delivered, so "Send report" can be tried again.
  let report = app
    .try_state::<PendingCrash>()
    .and_then(|pending| pending.0.lock().ok()?.clone())
    .ok_or(send::SendError::NothingToSend)?;
  tauri::async_runtime::spawn_blocking(move || send::send_manual(&report))
    .await
    .map_err(|_| send::SendError::NotDelivered)??;
  if let Some(pending) = app.try_state::<PendingCrash>() {
    if let Ok(mut slot) = pending.0.lock() {
      *slot = None;
    }
  }
  Ok(())
}

/// A render loop must not flood the log: one run takes this many frontend errors (every window together).
const MAX_FRONTEND_ERRORS: usize = 50;

fn admit_frontend_error(count: &AtomicUsize) -> bool {
  count.fetch_add(1, Ordering::Relaxed) < MAX_FRONTEND_ERRORS
}

/// Frontend errors (window.onerror, unhandledrejection, Vue errorHandler): app log, and a
/// report when reports are on.
#[tauri::command]
pub fn diagnostics_log(app: AppHandle, level: String, message: String) {
  static LOGGED: AtomicUsize = AtomicUsize::new(0);
  if !admit_frontend_error(&LOGGED) {
    return;
  }
  let message: String = message.chars().take(2_000).collect();
  let clean: ScrubbedText = redact::scrub(&message, redact::local_names());
  if level == "warn" {
    tracing::warn!(target: "frontend", "{clean}");
  } else {
    tracing::error!(target: "frontend", "{clean}");
    send::send_frontend_error(&app, &clean);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn names() -> Vec<String> {
    vec!["samira".into()]
  }

  fn note(text: String) -> Option<String> {
    let draft = Draft {
      note: Some(text),
      ..Draft::default()
    };
    draft
      .into_input(ReportInput::default(), &names())
      .note
      .map(|note| note.to_string())
  }

  fn row(url: &str, extractor: Option<&str>, include_full_link: bool) -> DownloadContext {
    let draft = Draft {
      download: Some(DownloadContext::default()),
      url: Some(url.into()),
      extractor: extractor.map(str::to_string),
      include_full_link,
      ..Draft::default()
    };
    draft
      .into_input(ReportInput::default(), &names())
      .download
      .expect("a row stays a row")
  }

  #[test]
  fn the_crash_file_goes_only_once_sent_or_staged() {
    let dir = std::env::temp_dir().join(format!("nezzel-settle-{}", uuid::Uuid::new_v4()));
    let write = || {
      std::fs::create_dir_all(&dir).unwrap();
      std::fs::write(dir.join(crash::CRASH_FILE), "boom").unwrap();
    };
    // Staging failed: nothing to offer now, and the file stays for the next launch.
    write();
    assert!(settle_crash(&dir, true, || false, || false).is_none());
    assert!(crash::read(&dir).is_some());
    // Staged for the user's click.
    let notice = settle_crash(&dir, true, || false, || true).expect("offered");
    assert!(!notice.sent);
    assert!(crash::read(&dir).is_none());
    // Sent: never staged as well.
    write();
    let notice =
      settle_crash(&dir, true, || true, || panic!("a sent crash is not staged")).expect("sent");
    assert!(notice.sent);
    assert!(crash::read(&dir).is_none());
    // No reports in this build: neither sent nor staged (no "Send report" that cannot work); the file stays.
    write();
    assert!(settle_crash(&dir, false, || panic!("not sent"), || panic!("not staged")).is_none());
    assert!(crash::read(&dir).is_some());
    let notice = settle_crash(&dir, true, || true, || false).expect("sent");
    assert!(notice.sent);
    assert!(crash::read(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn frontend_errors_stop_after_the_cap() {
    let count = AtomicUsize::new(0);
    let admitted = (0..MAX_FRONTEND_ERRORS + 5)
      .filter(|_| admit_frontend_error(&count))
      .count();
    assert_eq!(admitted, MAX_FRONTEND_ERRORS);
  }

  #[test]
  fn typed_notes_are_capped_then_scrubbed() {
    let draft = Draft::default();
    assert_eq!(
      draft.into_input(ReportInput::default(), &names()).note,
      None
    );
    let text = note(format!(
      "it fails on https://www.instagram.com/reel/Cx1/ for samira {}",
      "x".repeat(5_000)
    ))
    .unwrap();
    assert!(!text.contains("instagram.com"), "{text}");
    assert!(!text.contains("samira"), "{text}");
    assert!(text.chars().count() <= report::MAX_NOTE_CHARS);
    // The cap applies before scrubbing: a link cut in half is still removed.
    let cut = "a".repeat(report::MAX_NOTE_CHARS - 12) + " https://vimeo.com/123456789";
    assert!(!note(cut).unwrap().contains("vimeo"));
  }

  #[test]
  fn the_row_carries_a_reduced_link_and_a_token_extractor() {
    let auto = row(
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      Some("youtube:tab; rm -rf"),
      false,
    );
    assert_eq!(
      auto.site.as_ref().map(|site| site.as_str()),
      Some("youtube.com")
    );
    assert_eq!(
      auto.link.as_ref().map(|link| link.as_str()),
      Some("youtube.com/watch?v={id}")
    );
    assert_eq!(
      auto.extractor.as_ref().map(ExtractorToken::as_str),
      Some("youtube:tabrm-rf")
    );
    let full = row("https://vimeo.com/123?token=x", None, true);
    assert_eq!(
      full.link.as_ref().map(|link| link.as_str()),
      Some("https://vimeo.com/123")
    );
    // A private link leaves site and link empty rather than failing the report.
    let private = row("http://192.168.1.4/a.mp4", None, true);
    assert_eq!((private.site, private.link), (None, None));
    let no_row = Draft {
      url: Some("https://vimeo.com/1".into()),
      ..Draft::default()
    };
    assert!(no_row
      .into_input(ReportInput::default(), &names())
      .download
      .is_none());
  }
}
