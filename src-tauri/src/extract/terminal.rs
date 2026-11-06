//! Terminal row events: the attempt trail, and one diagnostic plus fatal pair per failed row.
use super::{outcome::*, precedence::final_code};
use crate::models::{error::DiagnosticLevel, MediaDiagnosticPayload, MediaFatalPayload};
use serde::Serialize;
use tauri::AppHandle;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AttemptPayload {
  stage: &'static str,
  login: bool,
  outcome: &'static str,
  code: Option<&'static str>,
  ms: u32,
}
impl From<&Attempt> for AttemptPayload {
  fn from(a: &Attempt) -> Self {
    Self {
      stage: match a.stage {
        StageId::Files => "files",
        StageId::Ytdlp => "ytdlp",
        StageId::Gallery => "gallery",
        StageId::Reader(_) => "reader",
        StageId::PageScan => "pageScan",
      },
      login: a.auth != AuthUsed::Anonymous,
      outcome: match a.outcome {
        Outcome::Verified => "verified",
        Outcome::NoMatch => "noMatch",
        Outcome::Unknown => "unknown",
        Outcome::Unsupported => "unsupported",
        Outcome::ExtractFailed => "extractFailed",
        Outcome::Login => "login",
        Outcome::AgeRestricted => "ageRestricted",
        Outcome::Paid => "paid",
        Outcome::Private => "private",
        Outcome::Removed => "removed",
        Outcome::Geo => "geo",
        Outcome::Drm => "drm",
        Outcome::LiveNotStarted => "liveNotStarted",
        Outcome::RateLimited => "rateLimited",
        Outcome::Blocked => "blocked",
        Outcome::Forbidden => "forbidden",
        Outcome::Network(_) => "network",
        Outcome::KindMismatch => "kindMismatch",
        Outcome::NoMedia => "noMedia",
        Outcome::Cancelled => "cancelled",
        Outcome::LoginPending => "loginPending",
      },
      code: a.code.map(DiagnosticCode::as_str),
      ms: a.ms,
    }
  }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AttemptsPayload<'a> {
  id: &'a str,
  group_id: &'a str,
  attempts: Vec<AttemptPayload>,
  winner: Option<usize>,
  final_code: Option<&'static str>,
}

pub(super) fn emit_attempts(app: &AppHandle, id: &str, group_id: &str, attempts: &[Attempt]) {
  crate::events::emit(
    app,
    crate::events::MEDIA_ATTEMPTS,
    AttemptsPayload {
      id,
      group_id,
      attempts: attempts.iter().map(AttemptPayload::from).collect(),
      winner: attempts.iter().position(|a| a.outcome == Outcome::Verified),
      final_code: final_code(attempts),
    },
  );
}

pub(super) fn publish_terminal(
  attempts: &[Attempt],
  group: &str,
  id: &str,
  message: impl FnOnce(&str) -> String,
  mut emit: impl FnMut(MediaDiagnosticPayload, MediaFatalPayload),
) -> bool {
  let Some(code) = final_code(attempts) else {
    return false;
  };
  let message = message(code);
  // Stage trail for bug reports: enums, codes and timings only (no URLs, names or engine stderr).
  let trail = attempts
    .iter()
    .map(|a| {
      format!(
        "{:?} {} {:?} {} {}ms",
        a.stage,
        if a.auth == AuthUsed::Anonymous {
          "anonymous"
        } else {
          "login"
        },
        a.outcome,
        a.code.map_or("-", |c| c.as_str()),
        a.ms
      )
    })
    .collect::<Vec<_>>()
    .join("\n");
  let mut fatal = MediaFatalPayload::with_exit(group.into(), id.into(), 1, message.clone());
  fatal.details = Some(trail.clone());
  emit(
    MediaDiagnosticPayload {
      id: id.into(),
      group_id: group.into(),
      level: DiagnosticLevel::Error,
      code: code.into(),
      component: Some("extract".into()),
      message,
      raw: trail,
      timestamp: std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis()),
    },
    fatal,
  );
  true
}

// Download paths retain their existing behavior until the download coordinator lands.
pub(crate) fn emit_terminal(
  app: &AppHandle,
  diagnostic: MediaDiagnosticPayload,
  fatal: MediaFatalPayload,
) {
  crate::events::emit(app, crate::events::MEDIA_DIAGNOSTIC, diagnostic);
  crate::events::emit(app, crate::events::MEDIA_FATAL, fatal);
}
