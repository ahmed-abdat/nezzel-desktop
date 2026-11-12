//! Row events the engine sends to the webview: their names (the TS listeners render the same spelling
//! from `contract.rs`) and one emit helper that logs a failed send instead of dropping it.
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

pub(crate) const MEDIA_ATTEMPTS: &str = "media_attempts";
pub(crate) const MEDIA_COMPLETE: &str = "media_complete";
pub(crate) const MEDIA_DESTINATION: &str = "media_destination";
pub(crate) const MEDIA_DIAGNOSTIC: &str = "media_diagnostic";
pub(crate) const MEDIA_FATAL: &str = "media_fatal";
pub(crate) const MEDIA_LOGIN_NEEDED: &str = "media_login_needed";
pub(crate) const MEDIA_LOGIN_NOTICE: &str = "media_login_notice";
pub(crate) const MEDIA_LOGIN_USING: &str = "media_login_using";
pub(crate) const MEDIA_PROGRESS: &str = "media_progress";
pub(crate) const MEDIA_PROGRESS_STAGE: &str = "media_progress_stage";
pub(crate) const MEDIA_TRANSPORT: &str = "media_transport";

/// Every name above, as (TS key, wire name), for `contract.rs`.
#[cfg(test)]
pub(crate) const ALL: [(&str, &str); 11] = [
  ("mediaAttempts", MEDIA_ATTEMPTS),
  ("mediaComplete", MEDIA_COMPLETE),
  ("mediaDestination", MEDIA_DESTINATION),
  ("mediaDiagnostic", MEDIA_DIAGNOSTIC),
  ("mediaFatal", MEDIA_FATAL),
  ("mediaLoginNeeded", MEDIA_LOGIN_NEEDED),
  ("mediaLoginUsing", MEDIA_LOGIN_USING),
  ("mediaLoginNotice", MEDIA_LOGIN_NOTICE),
  ("mediaProgress", MEDIA_PROGRESS),
  ("mediaProgressStage", MEDIA_PROGRESS_STAGE),
  ("mediaTransport", MEDIA_TRANSPORT),
];

/// Sends `event` to every window. A failed send is logged (event name and error only, never the payload).
pub(crate) fn emit<R: Runtime, S: Serialize + Clone>(app: &AppHandle<R>, event: &str, payload: S) {
  if let Err(err) = app.emit(event, payload) {
    tracing::warn!("Failed to emit {event}: {err}");
  }
}
