//! Automatic reports to our own Sentry project (DSN from `NEZZEL_SENTRY_DSN` at build time;
//! unset means the client is disabled and nothing leaves the app). On by default, switch in
//! Settings and on the welcome screen. Only events built here are sent: every free-form text
//! is scrubbed first, and `before_send` drops anything else (no breadcrumbs, host name or user).

use super::report::{self, DownloadContext};
use crate::SharedConfig;
use sentry::protocol::{Attachment, Event, Level};
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};

const SOURCE_TAG: &str = "nezzel.source";
/// One session can't flood the project: a loop of failures sends a handful of events.
const MAX_AUTO_EVENTS: usize = 20;

static SENT: LazyLock<Mutex<(usize, HashSet<String>)>> =
  LazyLock::new(|| Mutex::new((0, HashSet::new())));

/// The build-time DSN, parsed here: empty or malformed means `None` (never a panic).
pub fn build_dsn(raw: Option<&str>) -> Option<sentry::types::Dsn> {
  let raw = raw?.trim();
  if raw.is_empty() {
    return None;
  }
  raw.parse().ok()
}

/// Start the client only with our build-time DSN. Without one, `sentry::init` is never called,
/// so sentry's own fallback to the runtime `SENTRY_DSN` variable can't kick in.
pub fn init(raw: Option<&str>) -> Option<sentry::ClientInitGuard> {
  let dsn = build_dsn(raw)?;
  Some(sentry::init(sentry::ClientOptions {
    dsn: Some(dsn),
    ..client_options()
  }))
}

pub fn client_options() -> sentry::ClientOptions {
  sentry::ClientOptions {
    // Every crash counts; no performance tracing.
    sample_rate: 1.0,
    traces_sample_rate: 0.0,
    release: sentry::release_name!(),
    // No panic/contexts/backtrace integrations: they read the host name and unscrubbed panics.
    default_integrations: false,
    send_default_pii: false,
    attach_stacktrace: false,
    server_name: None,
    before_send: Some(Arc::new(strip_event)),
    ..sentry::ClientOptions::default()
  }
}

/// Anything not built by this module is dropped; ours lose fields that could identify the person.
fn strip_event(mut event: Event<'static>) -> Option<Event<'static>> {
  if !event.tags.contains_key(SOURCE_TAG) {
    return None;
  }
  event.server_name = None;
  event.user = None;
  event.request = None;
  event.breadcrumbs = Default::default();
  event.modules = Default::default();
  Some(event)
}

/// Codes that are the user's choice or their network, not a bug to patch.
pub fn worth_reporting(code: &str) -> bool {
  !matches!(
    code,
    "rejectedByFilter" | "input_filter_skipped" | "cancelled" | "unknown"
  ) && !crate::parsers::ytdlp_error::network_down(code, "")
}

fn enabled(app: &AppHandle) -> bool {
  app.state::<SharedConfig>().load().privacy.send_reports && configured()
}

pub fn configured() -> bool {
  sentry::Hub::current()
    .client()
    .is_some_and(|client| client.is_enabled())
}

/// Count and de-duplicate automatic events; manual reports skip this.
fn admit(key: String) -> bool {
  let Ok(mut sent) = SENT.lock() else {
    return false;
  };
  if sent.0 >= MAX_AUTO_EVENTS || !sent.1.insert(key) {
    return false;
  }
  sent.0 += 1;
  true
}

struct Outgoing<'a> {
  source: &'static str,
  level: Level,
  message: String,
  tags: Vec<(&'static str, String)>,
  fingerprint: Vec<String>,
  body: &'a str,
}

fn capture(out: Outgoing<'_>) -> sentry::types::Uuid {
  let mut event = Event {
    level: out.level,
    message: Some(out.message),
    ..Event::default()
  };
  event.tags.insert(SOURCE_TAG.into(), out.source.into());
  for (key, value) in out.tags {
    event.tags.insert(key.into(), value);
  }
  if !out.fingerprint.is_empty() {
    event.fingerprint = Cow::Owned(out.fingerprint.into_iter().map(Cow::Owned).collect());
  }
  let attachment = Attachment {
    buffer: out.body.as_bytes().to_vec(),
    filename: "report.txt".into(),
    content_type: Some("text/plain".into()),
    ty: None,
  };
  sentry::with_scope(
    |scope| scope.add_attachment(attachment),
    || sentry::capture_event(event),
  )
}

/// Previous run's crash. Returns whether it was sent.
pub fn send_crash(app: &AppHandle, body: &str, first_line: &str) -> bool {
  if !enabled(app) {
    return false;
  }
  capture(Outgoing {
    source: "crash",
    level: Level::Fatal,
    message: first_line.chars().take(200).collect(),
    tags: vec![],
    fingerprint: vec![],
    body,
  });
  true
}

/// A failed download, sent automatically. `site` comes from [`super::link::link_for_report`].
pub fn send_failure(
  app: &AppHandle,
  body: &str,
  download: &DownloadContext,
  site: Option<&str>,
  extractor: Option<&str>,
) -> bool {
  let code = download
    .error_code
    .as_deref()
    .map(report::token)
    .unwrap_or_else(|| "unknown".into());
  if !enabled(app) || !worth_reporting(&code) {
    return false;
  }
  let site = site.unwrap_or("unknown").to_string();
  if !admit(format!("failure:{code}:{site}")) {
    return false;
  }
  let mut tags = vec![("error_code", code.clone()), ("site", site.clone())];
  if let Some(extractor) = extractor {
    tags.push(("extractor", report::token(extractor)));
  }
  capture(Outgoing {
    source: "download",
    level: Level::Error,
    message: format!("Download failed: {code} on {site}"),
    tags,
    fingerprint: vec!["download-failed".into(), code, site],
    body,
  });
  true
}

/// Frontend errors: scrubbed text, de-duplicated by message.
pub fn send_frontend_error(app: &AppHandle, clean: &str) {
  let first = clean.lines().next().unwrap_or_default().to_string();
  if !enabled(app) || !admit(format!("frontend:{first}")) {
    return;
  }
  capture(Outgoing {
    source: "frontend",
    level: Level::Error,
    message: first.chars().take(200).collect(),
    tags: vec![],
    fingerprint: vec![],
    body: clean,
  });
}

/// "Report a problem": explicit, so it is sent even with automatic reports off.
/// Waits for delivery so the UI can say it was sent.
pub fn send_manual(body: &str, title: &str) -> Result<(), String> {
  if !configured() {
    return Err("reports are not configured in this build".into());
  }
  capture(Outgoing {
    source: "manual",
    level: Level::Info,
    message: title.chars().take(200).collect(),
    tags: vec![],
    fingerprint: vec![],
    body,
  });
  let delivered = sentry::Hub::current()
    .client()
    .is_some_and(|client| client.flush(Some(Duration::from_secs(10))));
  if delivered {
    Ok(())
  } else {
    Err("report could not be delivered".into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_a_valid_build_dsn_starts_a_client() {
    // A user's own SENTRY_DSN must never be picked up by a keyless build.
    std::env::set_var("SENTRY_DSN", "https://public@o0.ingest.sentry.io/1");
    assert!(build_dsn(None).is_none());
    assert!(build_dsn(Some("  ")).is_none());
    assert!(build_dsn(Some("not a dsn")).is_none());
    assert!(init(None).is_none());
    assert!(init(Some("garbage")).is_none());
    assert!(!configured());
    std::env::remove_var("SENTRY_DSN");
    assert!(build_dsn(Some("https://public@o0.ingest.sentry.io/1")).is_some());
  }

  #[test]
  fn foreign_events_are_dropped_and_ours_lose_identifying_fields() {
    let foreign = Event {
      message: Some("tracing error /Users/sara".into()),
      ..Event::default()
    };
    assert!(strip_event(foreign).is_none());

    let mut ours = Event {
      server_name: Some("Saras-MacBook".into()),
      ..Event::default()
    };
    ours.tags.insert(SOURCE_TAG.into(), "crash".into());
    let kept = strip_event(ours).expect("ours is kept");
    assert!(kept.server_name.is_none());
  }

  #[test]
  fn user_choices_and_outages_are_not_reported() {
    for code in [
      "rejectedByFilter",
      "cancelled",
      "networkNameResolutionFailed",
      "unknown",
    ] {
      assert!(!worth_reporting(code), "{code}");
    }
    assert!(worth_reporting("http403"));
  }
}
