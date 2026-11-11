//! Automatic reports to our own Sentry project (DSN from `NEZZEL_SENTRY_DSN` at build time;
//! unset means the client is disabled and nothing leaves the app). On by default, switch in
//! Settings and on the welcome screen. Only events built here are sent, and they carry only [`EventText`]:
//! scrubbed text, tokens, the reduced link or a built [`Report`] (a compile error otherwise), and `before_send`
//! drops anything else (no breadcrumbs, host name or user
//! fields; the user carries only a placeholder IP so Sentry can't infer a city, #39).

use super::link::SiteToken;
use super::redact::ScrubbedText;
use super::report::{CodeToken, DownloadContext, ExtractorToken, Report};
use crate::SharedConfig;
use sentry::protocol::{Attachment, Event, Level};
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const SOURCE_TAG: &str = "nezzel.source";
/// One session can't flood the project: a loop of failures sends a handful of events.
const MAX_AUTO_EVENTS: usize = 20;

// A timeout alone cannot distinguish an unreachable network from a slow site. Reuse a
// recent HTTP response or successful remote metadata fetch; unknown connectivity suppresses it.
const ONLINE_EVIDENCE_TTL: Duration = Duration::from_secs(60);
static LAST_NETWORK_RESPONSE: Mutex<Option<Instant>> = Mutex::new(None);

pub(crate) fn network_response_received() {
  if let Ok(mut last) = LAST_NETWORK_RESPONSE.lock() {
    *last = Some(Instant::now());
  }
}

pub(crate) fn network_response_failed() {
  if let Ok(mut last) = LAST_NETWORK_RESPONSE.lock() {
    *last = None;
  }
}

fn worth_reporting_with_network(code: &str, response_age: Option<Duration>) -> bool {
  worth_reporting(code)
    && (code != "networkTimeout" || response_age.is_some_and(|age| age < ONLINE_EVIDENCE_TTL))
}

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
  // Not `None`: with no user IP, Sentry looks up the connection's IP and stores the city
  // (`user.geo`, #39). An unroutable placeholder stops that lookup; IP storage is off server side.
  event.user = Some(sentry::User {
    ip_address: Some(sentry::protocol::IpAddress::Exact(
      std::net::Ipv4Addr::UNSPECIFIED.into(),
    )),
    ..Default::default()
  });
  event.request = None;
  event.breadcrumbs = Default::default();
  event.modules = Default::default();
  Some(event)
}

/// Codes that are the user's choice or their network, not a bug to patch.
pub fn worth_reporting(code: &str) -> bool {
  !matches!(
    code,
    "rejectedByFilter" | "input_filter_skipped" | "cancelled"
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

/// Text an event may carry. Built only from the types below (private field), so an unscrubbed `String`
/// can't become a message, tag, fingerprint or attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EventText(String);

impl EventText {
  fn title(report: &Report) -> Self {
    Self(report.title().to_string())
  }

  fn body(report: &Report) -> Self {
    Self(report.body().to_string())
  }

  fn join<const N: usize>(parts: [EventText; N]) -> Self {
    Self(parts.into_iter().map(|part| part.0).collect())
  }

  /// The first line, at most `max` characters.
  fn first_line(self, max: usize) -> Self {
    Self(
      self
        .0
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(max)
        .collect(),
    )
  }
}

impl From<&'static str> for EventText {
  fn from(label: &'static str) -> Self {
    Self(label.into())
  }
}

macro_rules! event_text_from {
  ($($safe:ty),*) => {$(
    impl From<&$safe> for EventText {
      fn from(value: &$safe) -> Self {
        Self(value.as_str().to_string())
      }
    }
  )*};
}

event_text_from!(ScrubbedText, CodeToken, ExtractorToken, SiteToken);

struct Outgoing {
  source: &'static str,
  level: Level,
  /// Cut to its first line, 200 characters.
  message: EventText,
  tags: Vec<(&'static str, EventText)>,
  fingerprint: Vec<EventText>,
  body: EventText,
}

/// The event and its `report.txt`, exactly as they leave the app.
fn event(out: Outgoing) -> (Event<'static>, Attachment) {
  let mut event = Event {
    level: out.level,
    message: Some(out.message.first_line(200).0),
    ..Event::default()
  };
  event.tags.insert(SOURCE_TAG.into(), out.source.into());
  for (key, value) in out.tags {
    event.tags.insert(key.into(), value.0);
  }
  if !out.fingerprint.is_empty() {
    event.fingerprint = Cow::Owned(
      out
        .fingerprint
        .into_iter()
        .map(|part| Cow::Owned(part.0))
        .collect(),
    );
  }
  let attachment = Attachment {
    buffer: out.body.0.into_bytes(),
    filename: "report.txt".into(),
    content_type: Some("text/plain".into()),
    ty: None,
  };
  (event, attachment)
}

fn capture(out: Outgoing) -> sentry::types::Uuid {
  let (event, attachment) = event(out);
  sentry::with_scope(
    |scope| scope.add_attachment(attachment),
    || sentry::capture_event(event),
  )
}

fn crash_event(report: &Report, crash: &ScrubbedText, fatal: bool) -> Outgoing {
  // Line 3 of a crash file is the panic message (see `crash::format_crash`).
  let message = crash
    .lines()
    .nth(2)
    .map_or_else(|| EventText::from("crash"), |line| EventText::from(&line));
  Outgoing {
    source: "crash",
    level: if fatal { Level::Fatal } else { Level::Error },
    message,
    tags: vec![],
    fingerprint: vec![],
    body: EventText::body(report),
  }
}

/// The error code a failed row is reported under; a row without one is `unknown`.
fn failure_code(download: &DownloadContext) -> CodeToken {
  download
    .error_code
    .clone()
    .unwrap_or_else(|| CodeToken::new("unknown"))
}

fn failure_site(download: &DownloadContext) -> EventText {
  download
    .site
    .as_ref()
    .map_or_else(|| EventText::from("unknown"), EventText::from)
}

fn failure_event(report: &Report, download: &DownloadContext) -> Outgoing {
  let code = failure_code(download);
  let site = failure_site(download);
  let mut tags = vec![
    ("error_code", EventText::from(&code)),
    ("site", site.clone()),
  ];
  if let Some(extractor) = &download.extractor {
    tags.push(("extractor", EventText::from(extractor)));
  }
  Outgoing {
    source: "download",
    level: Level::Error,
    message: EventText::join([
      "Download failed: ".into(),
      EventText::from(&code),
      " on ".into(),
      site.clone(),
    ]),
    tags,
    fingerprint: vec!["download-failed".into(), EventText::from(&code), site],
    body: EventText::body(report),
  }
}

fn frontend_event(clean: &ScrubbedText) -> Outgoing {
  Outgoing {
    source: "frontend",
    level: Level::Error,
    message: EventText::from(clean),
    tags: vec![],
    fingerprint: vec![],
    body: EventText::from(clean),
  }
}

fn manual_event(report: &Report) -> Outgoing {
  Outgoing {
    source: "manual",
    level: Level::Info,
    message: EventText::title(report),
    tags: vec![],
    fingerprint: vec![],
    body: EventText::body(report),
  }
}

/// Previous run's panic; fatal only after an unexpected close. Returns whether it was sent.
pub fn send_crash(app: &AppHandle, report: &Report, crash: &ScrubbedText, fatal: bool) -> bool {
  if !enabled(app) {
    return false;
  }
  capture(crash_event(report, crash, fatal));
  true
}

/// A failed download, sent automatically. Site and extractor come from the row (`link::link_for_report`).
pub fn send_failure(app: &AppHandle, report: &Report, download: &DownloadContext) -> bool {
  let code = failure_code(download);
  let response_age = LAST_NETWORK_RESPONSE
    .lock()
    .ok()
    .and_then(|last| last.map(|when| when.elapsed()));
  if !enabled(app) || !worth_reporting_with_network(code.as_str(), response_age) {
    return false;
  }
  let site = download.site.as_ref().map_or("unknown", SiteToken::as_str);
  if !admit(format!("failure:{code}:{site}")) {
    return false;
  }
  capture(failure_event(report, download));
  true
}

/// Frontend errors: scrubbed text, de-duplicated by message.
pub fn send_frontend_error(app: &AppHandle, clean: &ScrubbedText) {
  let first = clean.as_str().lines().next().unwrap_or_default();
  if !enabled(app) || !admit(format!("frontend:{first}")) {
    return;
  }
  capture(frontend_event(clean));
}

/// Why a report the user sent did not go. Only `notDelivered` is worth trying again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
  /// This build has no reports DSN.
  NotConfigured,
  /// Sentry did not confirm delivery (offline, timeout).
  NotDelivered,
  /// No staged crash to send.
  NothingToSend,
}

impl SendError {
  /// Read by the TS contract (`contract.rs`).
  #[cfg_attr(not(test), allow(dead_code))]
  pub const ALL: [SendError; 3] = [
    SendError::NotConfigured,
    SendError::NotDelivered,
    SendError::NothingToSend,
  ];

  pub fn as_str(self) -> &'static str {
    match self {
      SendError::NotConfigured => "notConfigured",
      SendError::NotDelivered => "notDelivered",
      SendError::NothingToSend => "nothingToSend",
    }
  }
}

impl serde::Serialize for SendError {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(self.as_str())
  }
}

/// "Report a problem": explicit, so it is sent even with automatic reports off.
/// Waits for delivery so the UI can say it was sent.
pub fn send_manual(report: &Report) -> Result<(), SendError> {
  if !configured() {
    return Err(SendError::NotConfigured);
  }
  capture(manual_event(report));
  let delivered = sentry::Hub::current()
    .client()
    .is_some_and(|client| client.flush(Some(Duration::from_secs(10))));
  if delivered {
    Ok(())
  } else {
    Err(SendError::NotDelivered)
  }
}

#[cfg(test)]
mod tests {
  use super::super::redact::scrub;
  use super::super::report::{self, ReportInput};
  use super::super::Draft;
  use super::*;

  fn names() -> Vec<String> {
    vec!["samira".into()]
  }

  /// What any of our events must never carry, whatever the user, the webview or the engine handed in.
  const LEAKS: &[&str] = &[
    "samira",
    "/Users",
    "Movies",
    "فيلم",
    "Cx1secret",
    "igsh",
    "bob",
    "hunter2",
    "31337abc",
    "zzz",
  ];

  /// The event and its attachment exactly as they would leave the app.
  fn wire(out: Outgoing) -> (serde_json::Value, String) {
    let (event, attachment) = event(out);
    let json = serde_json::to_value(&event).expect("event serialises");
    (
      json,
      String::from_utf8(attachment.buffer).expect("utf-8 body"),
    )
  }

  fn assert_no_leaks(json: &serde_json::Value, body: &str) {
    let all = format!("{json}\n{body}");
    for leak in LEAKS {
      assert!(!all.contains(leak), "{leak} leaked in\n{all}");
    }
  }

  /// A report from raw parts, the way `diagnostics::build_report` does it (environment is test data).
  fn built(draft: Draft) -> (Report, ReportInput) {
    let environment = ReportInput {
      app_version: "1.2.0".into(),
      log_lines: if draft.with_log {
        vec![scrub(
          "WARN failed https://www.youtube.com/watch?v=Cx1secret for /Users/samira/Movies/a.mp4 Cookie: sessionid=31337abc",
          &names(),
        )]
      } else {
        Vec::new()
      },
      ..ReportInput::default()
    };
    let input = draft.into_input(environment, &names());
    (report::build(&input), input)
  }

  fn failed_row() -> DownloadContext {
    serde_json::from_value(serde_json::json!({
      "errorCode": "http403",
      "attempts": [{ "stage": "https://x.com/a b", "outcome": "failed", "code": "http403" }],
    }))
    .expect("a row from the webview")
  }

  #[test]
  fn automatic_failure_event_carries_tokens_and_the_link_shape_only() {
    let (report, input) = built(Draft {
      download: Some(failed_row()),
      url: Some("https://bob:hunter2@www.instagram.com/reel/Cx1secret/?igsh=zzz".into()),
      extractor: Some("Instagram; rm -rf".into()),
      ..Draft::default()
    });
    let download = input.download.as_ref().expect("the row");
    let (json, body) = wire(failure_event(&report, download));
    assert_no_leaks(&json, &body);
    assert_eq!(json["message"], "Download failed: http403 on instagram.com");
    assert_eq!(json["tags"]["error_code"], "http403");
    assert_eq!(json["tags"]["site"], "instagram.com");
    assert_eq!(json["tags"]["extractor"], "Instagramrm-rf");
    assert_eq!(json["tags"][SOURCE_TAG], "download");
    assert_eq!(
      json["fingerprint"],
      serde_json::json!(["download-failed", "http403", "instagram.com"])
    );
    assert!(body.contains("- Link: instagram.com/reel/{id}\n"), "{body}");
    // Automatic events carry no log tail.
    assert!(!body.contains("Recent log"), "{body}");
  }

  #[test]
  fn manual_report_has_the_full_link_only_when_the_user_includes_it() {
    let draft = |include_link| {
      super::super::manual_draft(
        Some("stops on https://vimeo.com/123 for samira in /Users/samira/Movies/فيلم.mp4".into()),
        Some(failed_row()),
        Some("https://bob:hunter2@www.youtube.com/watch?v=dQw4w9WgXcQ&token=zzz".into()),
        Some("youtube".into()),
        Some(include_link),
      )
    };
    let (report, _) = built(draft(true));
    let (json, body) = wire(manual_event(&report));
    assert_no_leaks(&json, &body);
    assert_eq!(json["message"], "Download failed: http403");
    assert_eq!(json["tags"][SOURCE_TAG], "manual");
    assert!(
      body.contains("- Link: https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
      "{body}"
    );
    assert!(body.contains("### Recent log"), "{body}");

    let (report, _) = built(draft(false));
    let (json, body) = wire(manual_event(&report));
    assert_no_leaks(&json, &body);
    assert!(!body.contains("dQw4w9WgXcQ"), "{body}");
    assert!(
      body.contains("- Link: youtube.com/watch?v={id}&token={id}"),
      "{body}"
    );
  }

  #[test]
  fn crash_event_is_scrubbed_even_from_an_old_crash_file() {
    // An older build may have written the file unscrubbed.
    let raw = "Nezzel 1.2.0 on macOS 15.6 (aarch64)\nthread main panicked at src/lib.rs:1:1\nno such file /Users/samira/Movies/فيلم.mp4\n";
    let (report, input) = built(Draft {
      crash: Some(raw.into()),
      ..Draft::default()
    });
    let crash = input.crash.as_ref().expect("the crash");
    for (fatal, level) in [(true, Level::Fatal), (false, Level::Error)] {
      let event = crash_event(&report, crash, fatal);
      assert_eq!(event.level, level);
      let (json, body) = wire(event);
      assert_no_leaks(&json, &body);
      assert!(
        json["message"]
          .as_str()
          .is_some_and(|message| message.starts_with("no such file ")),
        "{json}"
      );
      assert!(body.contains("panicked at src/lib.rs:1:1"), "{body}");
    }
  }

  #[test]
  fn frontend_error_event_is_the_scrubbed_first_line() {
    let clean = scrub(
      "TypeError: failed https://www.instagram.com/reel/Cx1secret/ for samira\n  at /Users/samira/app/main.js:1:2",
      &names(),
    );
    let (json, body) = wire(frontend_event(&clean));
    assert_no_leaks(&json, &body);
    assert!(
      json["message"]
        .as_str()
        .is_some_and(|message| message.starts_with("TypeError: failed ") && !message.contains('\n')),
      "{json}"
    );
    assert_eq!(body, clean.as_str());
  }

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
      user: Some(sentry::User {
        username: Some("sara".into()),
        ..Default::default()
      }),
      ..Event::default()
    };
    ours.tags.insert(SOURCE_TAG.into(), "crash".into());
    let kept = strip_event(ours).expect("ours is kept");
    assert!(kept.server_name.is_none());
    // Only the placeholder IP is left: no name, and nothing for Sentry to turn into a city (#39).
    let user = kept.user.expect("a placeholder user blocks IP inference");
    assert_eq!(
      user.ip_address,
      Some(sentry::protocol::IpAddress::Exact(
        std::net::Ipv4Addr::UNSPECIFIED.into()
      ))
    );
    assert_eq!(
      serde_json::to_value(&user).expect("user serialises"),
      serde_json::json!({ "ip_address": "0.0.0.0" })
    );
  }

  #[test]
  fn user_choices_and_outages_are_not_reported() {
    for code in [
      "rejectedByFilter",
      "cancelled",
      "networkNameResolutionFailed",
    ] {
      assert!(!worth_reporting(code), "{code}");
    }
    assert!(worth_reporting("http403"));
    assert!(worth_reporting("unknown"));
    assert!(worth_reporting("networkTimeout"));
    for (age, report) in [
      (None, false),
      (Some(Duration::ZERO), true),
      (Some(ONLINE_EVIDENCE_TTL), false),
    ] {
      assert_eq!(worth_reporting_with_network("networkTimeout", age), report);
      assert!(worth_reporting_with_network("unknown", age));
      assert!(!worth_reporting_with_network(
        "networkNameResolutionFailed",
        age
      ));
    }
    // Login walls and site decisions are still worth a report (only the user's own choices and outages are not).
    for code in [
      "signInRequired",
      "membersOnly",
      "geoBlocked",
      "unableToExtractInitialData",
    ] {
      assert!(worth_reporting(code), "{code}");
    }
  }
}
