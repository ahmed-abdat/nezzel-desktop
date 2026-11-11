use super::consent::*;
use super::registry::*;
use super::session::*;
use super::*;
use crate::extract::policy::Phase;
use crate::models::error::DiagnosticLevel;
use crate::parsers::ytdlp_error::CookieFetchFailure;
use crate::runners::browser_cookies::migrate_site_browsers;
use crate::runners::retry_policy::RetryAction;
use crate::scheduling::{
  dispatcher::DispatchRequest,
  download_pipeline::{DownloadEntry, DownloadSender},
  fetch_pipeline::{FetchEntry, FetchRequest, FetchSender},
  group_state::subscribe_group,
};
use crate::state::config_models::AuthSettings;
use tauri::Manager;
use tokio::sync::watch;
#[test]
fn unknown_hosts_ignore_legacy_remembered_and_generic_cookie_evidence() {
  let host = format!("unsafe-{}.example", uuid::Uuid::new_v4());
  let mut auth = AuthSettings::default();
  record_host_browser(&mut auth, &host, Some("chrome:/legacy".into()));
  let mut session = LoginSession::from_auth(auth.clone(), &format!("https://{host}/login"), None);
  assert!(session.browser.is_none());
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, false, |_| vec![
        LoginCandidate {
          id: "chrome:/legacy".into(),
          browser: BrowserKind::Chrome,
          profile: "Legacy".into(),
          signed_in: true,
          app_bound: false,
        }
      ])
      .0,
    RetryAction::AskLogin
  );
  EXPLICIT_SITES
    .lock()
    .unwrap()
    .insert(site_key(&host).unwrap_or_else(|| host.clone()));
  let consented = LoginSession::from_auth(auth, &format!("https://{host}/login"), None);
  assert_eq!(consented.browser.as_deref(), Some("chrome:/legacy"));
  EXPLICIT_SITES
    .lock()
    .unwrap()
    .remove(&site_key(&host).unwrap_or_else(|| host.clone()));
}

#[test]
fn reddit_aliases_reuse_a_legacy_remembered_profile_and_write_one_canonical_key() {
  for legacy in [
    "www.reddit.com",
    "old.reddit.com",
    "v.redd.it",
    "redd.it",
    "reddit.com",
  ] {
    for host in ["redd.it", "v.redd.it", "old.reddit.com", "www.reddit.com"] {
      let mut auth = AuthSettings::default();
      auth
        .site_browsers
        .insert(legacy.into(), "chrome:/remembered".into());
      let session = LoginSession::from_auth(auth, &format!("https://{host}/item"), None);
      assert_eq!(
        session.browser.as_deref(),
        Some("chrome:/remembered"),
        "{legacy} → {host}"
      );
      assert_eq!(session.auth.site_browsers.len(), 1);
      assert_eq!(
        session
          .auth
          .site_browsers
          .get("reddit.com")
          .map(String::as_str),
        Some("chrome:/remembered")
      );
      assert_eq!(
        session.success_patch().unwrap(),
        serde_json::json!({"auth":{"siteBrowsers":{"reddit.com":"chrome:/remembered"}}})
      );
    }
  }
}

#[test]
fn legacy_site_key_migration_preserves_values_and_is_idempotent() {
  for (legacy, canonical) in [
    ("www.reddit.com", "reddit.com"),
    ("redd.it", "reddit.com"),
    ("www.youtube.com", "youtube.com"),
    ("youtu.be", "youtube.com"),
    ("threads.com", "threads.net"),
    ("twitter.com", "x.com"),
    ("www.instagram.com", "instagram.com"),
  ] {
    let mut browsers = HashMap::from([(legacy.to_owned(), "chrome:/private/Profile 1".to_owned())]);
    migrate_site_browsers(&mut browsers);
    assert_eq!(
      browsers,
      HashMap::from([(canonical.to_owned(), "chrome:/private/Profile 1".to_owned())])
    );
    let once = browsers.clone();
    migrate_site_browsers(&mut browsers);
    assert_eq!(browsers, once);
    browsers.insert(legacy.into(), "firefox:/stale".into());
    migrate_site_browsers(&mut browsers);
    assert_eq!(
      browsers, once,
      "canonical selection must survive a conflicting legacy key"
    );
  }
  // A host only `sites` (not earlier versions) ties to a site keeps its own key: never promoted into that
  // site's consent, so it still needs an explicit pick.
  let mut browsers = HashMap::from([(
    "youtube-nocookie.com".to_owned(),
    "chrome:/private/Profile 1".to_owned(),
  )]);
  migrate_site_browsers(&mut browsers);
  assert_eq!(
    browsers,
    HashMap::from([(
      "youtube-nocookie.com".to_owned(),
      "chrome:/private/Profile 1".to_owned()
    )])
  );
}

#[test]
fn candidate_payload_ids_do_not_expose_engine_profile_paths() {
  let candidate = LoginCandidate {
    id: "chrome:/Users/private-user/Chrome/Default".into(),
    browser: BrowserKind::Chrome,
    profile: "Ahmed".into(),
    signed_in: true,
    app_bound: false,
  };
  let wire = serde_json::to_value(&candidate).unwrap();
  assert!(uuid::Uuid::parse_str(wire["id"].as_str().unwrap()).is_ok());
  assert!(!wire.to_string().contains("private-user"));
  let id = wire["id"].as_str().unwrap();
  assert_eq!(resolve_candidate_id(id).unwrap(), candidate.id);
  assert!(resolve_candidate_id(&candidate.id).is_err());
  assert_eq!(serde_json::to_value(&candidate).unwrap()["id"], wire["id"]);
  let group = uuid::Uuid::new_v4().to_string();
  let mut offered = pending(&group);
  offered.candidates = vec![candidate];
  let mut map = HashMap::from([("request".into(), offered)]);
  let other_token = opaque_candidate_id("firefox:/not-offered");
  assert!(claim_pending(
    &mut map,
    "request",
    &resolve_candidate_id(&other_token).unwrap()
  )
  .is_err());
  let claimed = claim_pending(&mut map, "request", &resolve_candidate_id(id).unwrap()).unwrap();
  assert_eq!(claimed.request.ids().1, group);
  crate::scheduling::group_state::remove_group(&group);
}

#[test]
fn discovery_is_cached_per_host_until_login_ttl() {
  let now = Instant::now();
  let mut cache = HashMap::new();
  let first = cached_discovery(&mut cache, "instagram.com", now, || candidates(None));
  let second = cached_discovery(&mut cache, "instagram.com", now, || {
    panic!("profile rescan for a concurrent row")
  });
  assert_eq!(second[0].id, first[0].id);
  let reddit = cached_discovery(&mut cache, "www.reddit.com", now, || candidates(None));
  let short = cached_discovery(&mut cache, "redd.it", now, || {
    panic!("alias repeated browser discovery")
  });
  assert_eq!(short[0].id, reddit[0].id);
  let other = cached_discovery(&mut cache, "facebook.com", now, || {
    let mut c = candidates(None);
    c[0].profile = "Other site".into();
    c
  });
  assert_eq!(other[0].profile, "Other site");
  let refreshed = cached_discovery(&mut cache, "instagram.com", now + LOGIN_TTL, || {
    let mut c = candidates(None);
    c[0].profile = "Refreshed".into();
    c
  });
  assert_eq!(refreshed[0].profile, "Refreshed");
}

#[test]
fn safari_without_evidence_requires_a_picker_unless_globally_allowed() {
  let host = format!("safari-{}.instagram.com", uuid::Uuid::new_v4());
  let url = format!("https://{host}/reel/private");
  let mut auth = AuthSettings::default();
  record_host_browser(&mut auth, &host, Some("safari".into()));
  let mut legacy = LoginSession::from_auth(auth.clone(), &url, None);
  assert!(
    legacy.browser.is_none(),
    "legacy Safari entries are not proof of consent"
  );
  assert_eq!(
    legacy
      .decide(Some("signInRequired"), false, false, |_| vec![
        LoginCandidate {
          id: "safari".into(),
          browser: BrowserKind::Safari,
          profile: "Default".into(),
          signed_in: false,
          app_bound: false,
        }
      ])
      .0,
    RetryAction::AskLogin
  );
  EXPLICIT_SITES
    .lock()
    .unwrap()
    .insert(site_key(&host).unwrap_or_else(|| host.clone()));
  assert_eq!(
    LoginSession::from_auth(auth, &url, None).browser.as_deref(),
    Some("safari")
  );
  EXPLICIT_SITES
    .lock()
    .unwrap()
    .remove(&site_key(&host).unwrap_or_else(|| host.clone()));
  for (allowed, expected) in [(false, RetryAction::AskLogin), (true, RetryAction::Browser)] {
    let mut session = LoginSession::from_auth(
      AuthSettings {
        always_use_browser_logins: allowed,
        ..Default::default()
      },
      "https://instagram.com/reel/private",
      None,
    );
    let (action, _) = session.decide(Some("signInRequired"), false, false, |_| {
      vec![LoginCandidate {
        id: "safari".into(),
        browser: BrowserKind::Safari,
        profile: "Default".into(),
        signed_in: false,
        app_bound: false,
      }]
    });
    assert_eq!(action, expected);
  }
}

#[test]
fn app_bound_reason_requires_a_blocked_session_without_a_usable_login() {
  for (blocked, signed_in, expected) in [
    (false, false, "signInRequired"),
    (true, false, APP_BOUND_REASON),
    (true, true, "signInRequired"),
  ] {
    let mut session = LoginSession::from_auth(
      AuthSettings::default(),
      "https://reddit.com/r/videos/",
      None,
    );
    let (_, candidates) = session.decide(Some("signInRequired"), false, false, |_| {
      let mut candidates = vec![];
      if blocked {
        candidates.push(LoginCandidate {
          id: "edge:/signed".into(),
          browser: BrowserKind::Edge,
          profile: "Default".into(),
          signed_in: true,
          app_bound: true,
        });
      }
      if signed_in {
        candidates.push(LoginCandidate {
          id: "firefox:/signed".into(),
          browser: BrowserKind::Firefox,
          profile: "Default".into(),
          signed_in: true,
          app_bound: false,
        });
      }
      candidates
    });
    assert_eq!(
      session.failure_reason(Some("signInRequired"), &candidates),
      expected
    );
    assert_eq!(
      session.blocked_browser,
      blocked.then_some(BrowserKind::Edge)
    );
    assert!(candidates.iter().all(|c| !c.app_bound));
  }
}

#[test]
fn automatic_signed_profiles_need_no_prompt_and_skip_app_bound_chromium() {
  for (blocked, expected) in [(false, "chrome:/signed"), (true, "firefox:/signed")] {
    let mut session = LoginSession::from_auth(
      AuthSettings::default(),
      "https://instagram.com/reel/private",
      None,
    );
    let (action, _) = session.decide(Some("signInRequired"), false, false, |_| {
      vec![
        LoginCandidate {
          id: "chrome:/signed".into(),
          browser: BrowserKind::Chrome,
          profile: "Ahmed".into(),
          signed_in: true,
          app_bound: blocked,
        },
        LoginCandidate {
          id: "firefox:/signed".into(),
          browser: BrowserKind::Firefox,
          profile: "Ahmed.work".into(),
          signed_in: true,
          app_bound: false,
        },
      ]
    });
    assert_eq!(action, RetryAction::Browser);
    assert_eq!(session.browser.as_deref(), Some(expected));
    assert!(!session.auth.always_use_browser_logins);
  }
}

#[test]
fn remembered_refusal_uses_next_signed_profile_then_offers_the_picker() {
  let mut auth = AuthSettings::default();
  record_host_browser(&mut auth, "www.instagram.com", Some("chrome:/old".into()));
  let mut session = LoginSession::from_auth(auth, "https://www.instagram.com/reel/private", None);
  let choices = || {
    vec![
      LoginCandidate {
        id: "chrome:/old".into(),
        browser: BrowserKind::Chrome,
        profile: "Old".into(),
        signed_in: true,
        app_bound: false,
      },
      LoginCandidate {
        id: "firefox:/new".into(),
        browser: BrowserKind::Firefox,
        profile: "New".into(),
        signed_in: true,
        app_bound: false,
      },
    ]
  };
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, false, |_| choices())
      .0,
    RetryAction::Browser
  );
  assert_eq!(session.browser.as_deref(), Some("firefox:/new"));
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, false, |_| choices())
      .0,
    RetryAction::AskLogin
  );
  assert!(session.browser.is_none());
}

#[test]
fn success_replaces_the_refused_starting_browser_but_not_a_newer_choice() {
  let mut auth = AuthSettings::default();
  record_host_browser(&mut auth, "site.test", Some("chrome:/old".into()));
  remember_success(
    &mut auth,
    "site.test",
    Some("chrome:/old"),
    "firefox:/working",
  );
  assert_eq!(auth.site_browsers["site.test"], "firefox:/working");
  remember_success(&mut auth, "site.test", Some("chrome:/old"), "chrome:/late");
  assert_eq!(auth.site_browsers["site.test"], "firefox:/working");
}

#[test]
fn concurrent_rows_share_one_host_retry_until_the_window_expires() {
  let now = Instant::now();
  let mut budget = HashMap::new();
  assert!(claim_host_retry(&mut budget, "www.instagram.com", now));
  for _ in 0..5 {
    assert!(!claim_host_retry(&mut budget, "m.instagram.com", now));
  }
  assert!(claim_host_retry(&mut budget, "facebook.com", now));
  assert!(claim_host_retry(
    &mut budget,
    "instagram.com",
    now + Duration::from_secs(31)
  ));
}

#[test]
fn only_a_login_refusal_exhausts_a_browser_session() {
  let session = LoginSession::from_auth(
    AuthSettings::default(),
    "https://www.instagram.com/reel/test/",
    Some("chrome".into()),
  );
  for (reason, cancelled, expected) in [
    (None, false, None),
    (Some("signInRequired"), false, Some(true)),
    (Some("signInRequired"), true, None),
  ] {
    assert_eq!(
      session
        .failure_status(&RetryAction::Stop, reason, cancelled, false)
        .map(|status| matches!(status, LoginStatus::Exhausted)),
      expected
    );
  }
}
#[test]
fn cookie_failure_retries_anonymously_without_forgetting_the_remembered_browser() {
  use crate::runners::browsers::CookieSource;
  for (mode, file, accepted) in [
    ("automatic", None, None),
    ("automatic", None, Some("firefox:/accepted")),
    ("firefox", None, None),
    ("automatic", Some("/manual.txt"), None),
  ] {
    let mut auth = AuthSettings {
      cookie_browser: mode.into(),
      cookie_file: file.map(str::to_owned),
      ..Default::default()
    };
    record_host_browser(&mut auth, "m.facebook.com", Some("chrome:/saved".into()));
    let mut session = LoginSession::from_auth(
      auth.clone(),
      "https://m.facebook.com/watch/",
      accepted.map(str::to_owned),
    );
    let (action, _) = session.decide(None, false, false, |_| panic!("no discovery"));
    assert_eq!(action, RetryAction::Anonymous, "{mode}, {file:?}");
    assert_eq!(
      CookieSource::select(&auth, session.browser.as_deref()),
      Ok(CookieSource::Anonymous)
    );
    assert!(!session.uses_cookies());
    assert!(session.success_patch().is_none());
    assert_eq!(session.auth.site_browsers["facebook.com"], "chrome:/saved");
    assert_eq!(
      session
        .decide(None, false, false, |_| panic!("no discovery"))
        .0,
      RetryAction::Stop
    );
  }
}

#[test]
fn failed_anonymous_retry_reports_the_more_informative_attempt() {
  use crate::parsers::ytdlp_error::{DiagnosticMatcher, YtdlpErrorParser};
  let parser = YtdlpErrorParser::new(
    "item",
    "group",
    DiagnosticMatcher::from_json(include_str!("../../diagnostic_rules.json")).unwrap(),
  );
  for (cookie_error, anonymous_error, expected) in [
    (
      "ERROR: Unsupported URL",
      "ERROR: Unsupported URL",
      "urlNotSupported",
    ),
    (
      "ERROR: Unsupported URL",
      "ERROR: Login required",
      "signInRequired",
    ),
    (
      "ERROR: Video does not exist",
      "ERROR: Unsupported URL",
      "videoNotFoundOrPrivate",
    ),
    (
      "ERROR: HTTP Error 404",
      "ERROR: Unable to extract info section",
      "notFound404",
    ),
    (
      "ERROR: Unsupported URL",
      "ERROR: private video",
      "signInRequired",
    ),
    (
      "ERROR: [Reddit] Account authentication is required. Use --cookies",
      "ERROR: Unsupported URL",
      "accountAccessDenied",
    ),
  ] {
    let mut session = LoginSession::from_auth(
      AuthSettings::default(),
      "https://facebook.com/watch/",
      Some("firefox".into()),
    );
    session.record_failure(&parser.parse_lines(vec![cookie_error], true), false);
    assert_eq!(
      session
        .decide(None, false, false, |_| panic!("no discovery"))
        .0,
      RetryAction::Anonymous
    );
    let diagnostics = parser.parse_lines(vec![anonymous_error], false);
    session.record_failure(&diagnostics, false);
    let reason = parser.login_reason(anonymous_error);
    let action = session
      .decide(reason.as_deref(), false, false, |_| vec![])
      .0;
    assert!(matches!(action, RetryAction::Stop | RetryAction::AskLogin));
    if cookie_error.contains("Account authentication is required") {
      assert_eq!(session.best_failure.as_ref().unwrap().raw, cookie_error);
    }
    assert_eq!(
      session
        .failure_diagnostics(diagnostics)
        .iter()
        .filter(|event| matches!(event.level, DiagnosticLevel::Error))
        .map(|event| event.code.as_str())
        .collect::<Vec<_>>(),
      vec![expected],
      "{cookie_error}; {anonymous_error}"
    );
  }
}

#[test]
fn network_outages_and_cancellation_do_not_spend_the_anonymous_budget() {
  use crate::parsers::ytdlp_error::{DiagnosticMatcher, YtdlpErrorParser};
  let parser = YtdlpErrorParser::new(
    "item",
    "group",
    DiagnosticMatcher::from_json(include_str!("../../diagnostic_rules.json")).unwrap(),
  );
  for (error, cancelled) in [
    ("ERROR: getaddrinfo failed", false),
    ("ERROR: Network is unreachable", false),
    ("ERROR: Connection refused", false),
    ("ERROR: Connection timed out", false),
    ("ERROR: Unsupported URL", true),
  ] {
    let mut session = LoginSession::from_auth(
      AuthSettings::default(),
      "https://facebook.com/watch/",
      Some("firefox".into()),
    );
    session.record_failure(&parser.parse_lines(vec![error], true), false);
    assert_eq!(
      session
        .decide(None, false, cancelled, |_| panic!("no discovery"))
        .0,
      RetryAction::Stop,
      "{error}"
    );
    assert!(!session.state.anonymous_attempted);
    if !cancelled && !error.contains("timed out") {
      assert_eq!(
        crate::parsers::gallery::failure(error),
        crate::parsers::gallery::GalleryError::NetworkDown
      );
    }
  }
}

#[test]
fn terminal_network_failure_keeps_its_diagnostic_after_a_cookie_failure() {
  use crate::parsers::ytdlp_error::{DiagnosticMatcher, YtdlpErrorParser};
  let parser = YtdlpErrorParser::new(
    "item",
    "group",
    DiagnosticMatcher::from_json(include_str!("../../diagnostic_rules.json")).unwrap(),
  );
  let mut session = LoginSession::from_auth(
    AuthSettings::default(),
    "https://facebook.com/watch/",
    Some("firefox".into()),
  );
  session.record_failure(
    &parser.parse_lines(vec!["ERROR: Unsupported URL"], true),
    false,
  );
  assert_eq!(
    session
      .decide(None, false, false, |_| panic!("no discovery"))
      .0,
    RetryAction::Anonymous
  );
  // Gallery can classify a transport outage even when it exposes only a generic diagnostic.
  let diagnostics = parser.parse_lines(vec!["ERROR: Request failed"], false);
  session.record_failure(&diagnostics, true);
  assert_eq!(
    session
      .decide(None, false, false, |_| panic!("no discovery"))
      .0,
    RetryAction::Stop
  );
  assert_eq!(
    session
      .failure_diagnostics(diagnostics)
      .last()
      .unwrap()
      .code,
    "unknown"
  );
}

#[test]
fn anonymous_mode_survives_a_gallery_to_video_session_handoff() {
  use crate::runners::browsers::CookieSource;
  let auth = AuthSettings {
    cookie_browser: "firefox".into(),
    cookie_file: Some("/manual.txt".into()),
    ..Default::default()
  };
  let mut session = LoginSession::from_auth(
    auth.clone(),
    "https://facebook.com/watch/",
    Some(ANONYMOUS_BROWSER.into()),
  );
  assert!(!session.uses_cookies());
  // `CookieAccess::prepare` gives an anonymous source no engine arguments.
  assert_eq!(
    CookieSource::select(&auth, session.browser.as_deref()),
    Ok(CookieSource::Anonymous)
  );
  assert_eq!(
    session
      .decide(None, false, false, |_| panic!("no discovery"))
      .0,
    RetryAction::Stop
  );
  assert!(session.success_patch().is_none());
}

#[test]
fn anonymous_fetch_ignores_every_cookie_setting_and_does_not_erase_host_memory() {
  use crate::runners::browsers::CookieSource;
  for (mode, file) in [
    ("automatic", None),
    ("firefox", None),
    ("automatic", Some("/private/manual.txt")),
  ] {
    let mut auth = AuthSettings {
      cookie_browser: mode.into(),
      cookie_file: file.map(str::to_owned),
      ..Default::default()
    };
    record_host_browser(
      &mut auth,
      "www.instagram.com",
      Some("chrome:/remembered".into()),
    );
    let mut session =
      LoginSession::from_auth(auth.clone(), "https://www.instagram.com/p/123/", None);
    session.begin_fetch(Phase::Anonymous);
    assert!(!session.uses_cookies());
    assert_eq!(
      CookieSource::select(&auth, session.browser.as_deref()),
      Ok(CookieSource::Anonymous)
    );
    assert!(session.success_patch().is_none());
    assert_eq!(
      session.auth.site_browsers["instagram.com"],
      "chrome:/remembered"
    );
  }
}

#[test]
fn fetch_consent_reaches_download_and_retry_without_a_second_prompt() {
  let group = uuid::Uuid::new_v4().to_string();
  let mut request = pending(&group).request;
  request.consent_to_browser("chrome:/selected");
  // Fetch Cleanup removes scheduler state, but consent must reach the download.
  crate::scheduling::group_state::remove_group(&group);
  let item = serde_json::from_value::<crate::models::DownloadItem>(serde_json::json!({
    "id":"login-item", "url":"https://example.com/private", "format":{"trackType":"both"}, "templateContext":{"values":{}}
  })).unwrap();
  let mut foreign = item.clone();
  foreign.url = "https://different.example/private".into();
  let foreign_download = DownloadEntry::from((foreign, group.clone()));
  assert!(
    foreign_download.login_browser.is_none(),
    "same row id cannot transfer consent to another host"
  );
  let download = DownloadEntry::from((item.clone(), group.clone()));
  assert_eq!(download.login_browser.as_deref(), Some("chrome:/selected"));
  assert!(download.login_was_accepted);
  for retry in [download.clone(), download.login_request_entry()] {
    let mut login =
      LoginSession::from_auth(AuthSettings::default(), &retry.url, retry.login_browser);
    assert_eq!(
      login
        .decide(Some("signInRequired"), false, false, |_| panic!(
          "download asked again"
        ))
        .0,
      RetryAction::Stop
    );
  }
  let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
  let app = tauri::test::mock_app();
  app.manage(DownloadSender(tx.clone()));
  remember_row_browser(&group, "waiting-sibling", "example.com", "firefox");
  LoginRequest::Download(download).release(app.handle());
  let retry = DownloadEntry::from((item, group.clone()));
  assert_eq!(
    retry.login_browser.as_deref(),
    Some("chrome:/selected"),
    "failed finish must preserve consent for the retried row"
  );
  crate::scheduling::download_pipeline::finish_download(&tx, &group, "login-item", true);
  assert!(row_browser(&group, "login-item", "https://example.com/private").is_none());
  assert_eq!(
    row_browser(&group, "waiting-sibling", "https://example.com/private").as_deref(),
    Some("firefox")
  );
  // Removing a fetched row from the list cancels its group without downloading it.
  request.consent_to_browser("chrome:/selected");
  crate::scheduling::group_state::cancel_group(&group);
  assert!(row_browser(&group, "login-item", "https://example.com/private").is_none());
  crate::scheduling::group_state::remove_group(&group);
}

#[test]
fn remembered_host_browser_is_used_on_first_fetch_and_download_attempts() {
  use crate::runners::browsers::CookieSource;
  for (mode, file, host, accepted, expected) in [
    (
      "automatic",
      None,
      "www.instagram.com",
      None,
      Some("chrome:/selected"),
    ),
    ("automatic", None, "other.test", None, None),
    (
      "automatic",
      None,
      "www.instagram.com",
      Some("firefox:/accepted"),
      Some("firefox:/accepted"),
    ),
    ("none", None, "www.instagram.com", None, None),
    ("firefox", None, "www.instagram.com", None, Some("firefox")),
    (
      "automatic",
      Some("/manual.txt"),
      "www.instagram.com",
      None,
      Some("/manual.txt"),
    ),
  ] {
    let mut auth = AuthSettings {
      cookie_browser: mode.into(),
      cookie_file: file.map(str::to_owned),
      ..Default::default()
    };
    record_host_browser(
      &mut auth,
      "www.instagram.com",
      Some("chrome:/selected".into()),
    );
    // Both runners construct their session at this boundary, including gallery fallbacks.
    let session = LoginSession::from_auth(
      auth.clone(),
      &format!("https://{host}/reel/test/"),
      accepted.map(str::to_owned),
    );
    let chosen = match CookieSource::select(&auth, session.browser.as_deref()).unwrap() {
      CookieSource::Anonymous => None,
      CookieSource::File(file) => Some(file),
      CookieSource::Browser(browser) => Some(browser.spec()),
    };
    assert_eq!(chosen.as_deref(), expected, "{mode}, {file:?}, {host}");
    assert_eq!(session.state.browser_attempted, session.browser.is_some());
  }
}

#[tokio::test(start_paused = true)]
async fn transient_cookie_fetch_backs_off_once_unless_the_group_is_cancelled() {
  let (running, _) = watch::channel(true);
  let consented = |browser: Option<&str>| {
    // A fresh registrable host per case: the host retry window is process-wide.
    let url = format!("https://cookie-{}.com/reel/test/", uuid::Uuid::new_v4());
    LoginSession::from_auth(AuthSettings::default(), &url, browser.map(str::to_owned))
  };
  for (browser, failure, expected) in [
    (
      Some("chrome"),
      CookieFetchFailure::Transient,
      CookieRetry::Retry,
    ),
    (None, CookieFetchFailure::Transient, CookieRetry::GiveUp),
    (
      Some("chrome"),
      CookieFetchFailure::Refused,
      CookieRetry::GiveUp,
    ),
    (
      Some("chrome"),
      CookieFetchFailure::Other,
      CookieRetry::GiveUp,
    ),
  ] {
    let mut session = consented(browser);
    let mut cancel = running.subscribe();
    let start = tokio::time::Instant::now();
    assert_eq!(
      session.retry_cookie_fetch(failure, &mut cancel).await,
      expected
    );
    let waited = if expected == CookieRetry::Retry {
      COOKIE_RETRY_DELAY
    } else {
      Duration::ZERO
    };
    assert_eq!(start.elapsed(), waited);
    assert_eq!(
      session.retry_cookie_fetch(failure, &mut cancel).await,
      CookieRetry::GiveUp,
      "one backoff per session"
    );
    assert_eq!(start.elapsed(), waited);
  }

  let mut session = consented(Some("chrome"));
  let mut cancel = running.subscribe();
  let start = tokio::time::Instant::now();
  let cancel_midway = async {
    tokio::time::sleep(Duration::from_millis(500)).await;
    running.send(false).unwrap();
    std::future::pending::<()>().await
  };
  let outcome = tokio::select! {
    outcome = session.retry_cookie_fetch(CookieFetchFailure::Transient, &mut cancel) => outcome,
    _ = cancel_midway => unreachable!(),
  };
  assert_eq!(outcome, CookieRetry::Cancelled);
  assert_eq!(start.elapsed(), Duration::from_millis(500));
}

#[test]
fn older_success_cannot_overwrite_a_newer_same_host_browser_choice() {
  let mut auth = AuthSettings::default();
  record_host_browser(&mut auth, "www.instagram.com", Some("firefox:/new".into()));
  remember_success(
    &mut auth,
    "www.instagram.com",
    Some("chrome:/old"),
    "chrome:/old",
  );
  remember_success(&mut auth, "other.test", None, "chrome:/other");
  assert_eq!(auth.site_browsers["instagram.com"], "firefox:/new");
  assert_eq!(auth.site_browsers["other.test"], "chrome:/other");
  assert!(!auth.always_use_browser_logins);
}

#[test]
fn same_host_waiters_share_one_prompt_and_acceptance_reaches_every_row() {
  let groups: Vec<_> = (0..70).map(|_| uuid::Uuid::new_v4().to_string()).collect();
  let mut map = HashMap::new();
  let mut prompts = 0;
  for (i, group) in groups.iter().enumerate() {
    prompts += usize::from(queue_pending(
      &mut map,
      format!("token-{i}"),
      pending(group),
    ));
  }
  assert_eq!((prompts, map.len()), (1, 1));
  let claimed = claim_pending(&mut map, "token-0", "chrome:/fake/Profile 1").unwrap();
  let mut resumed = Vec::new();
  for mut row in claimed.into_rows() {
    row.request.consent_to_browser("chrome:/fake/Profile 1");
    let LoginRequest::Fetch(entry) = row.request else {
      panic!("fetch waiter");
    };
    assert!(entry.login_was_accepted);
    assert_eq!(
      entry.login_browser.as_deref(),
      Some("chrome:/fake/Profile 1")
    );
    resumed.push(entry.group_id);
  }
  assert_eq!(resumed, groups);
  assert!(claim_pending(&mut map, "token-0", "chrome:/fake/Profile 1").is_err());
  for group in groups {
    forget_group(&group);
    crate::scheduling::group_state::remove_group(&group);
  }
}

#[test]
fn cancelling_the_prompt_owner_preserves_other_rows_and_the_single_use_token() {
  let first = uuid::Uuid::new_v4().to_string();
  let second = uuid::Uuid::new_v4().to_string();
  let mut map = HashMap::new();
  assert!(queue_pending(&mut map, "gate".into(), pending(&first)));
  assert!(!queue_pending(
    &mut map,
    "follower".into(),
    pending(&second)
  ));
  remove_pending_group(&mut map, &first);
  assert_eq!(map.len(), 1);
  let claimed = claim_pending(&mut map, "gate", "chrome:/fake/Profile 1").unwrap();
  assert_eq!(claimed.request.ids().1, second);
  assert!(claimed.followers.is_empty());
  crate::scheduling::group_state::remove_group(&first);
  crate::scheduling::group_state::remove_group(&second);
}

#[tokio::test]
async fn unanswered_login_emits_terminal_failure_and_releases_its_request() {
  let group = uuid::Uuid::new_v4().to_string();
  let token = uuid::Uuid::new_v4().to_string();
  let second = uuid::Uuid::new_v4().to_string();
  {
    let mut pending_map = PENDING.lock().unwrap();
    let same_host = format!("https://gate-{group}.example/private");
    let mut first_row = pending(&group);
    let mut second_row = pending(&second);
    if let LoginRequest::Fetch(entry) = &mut first_row.request {
      entry.url = same_host.clone();
    }
    if let LoginRequest::Fetch(entry) = &mut second_row.request {
      entry.url = same_host;
    }
    assert!(queue_pending(&mut pending_map, token.clone(), first_row));
    assert!(!queue_pending(
      &mut pending_map,
      "unused-follower".into(),
      second_row
    ));
  }
  let mut fatals = Vec::new();
  let mut released = Vec::new();
  expire_pending(
    &token,
    std::future::ready(()),
    |diagnostic, event| {
      assert_eq!(diagnostic.code, "loginConsentTimeout");
      fatals.push(event);
    },
    |request| released.push(request.ids().1.to_owned()),
  )
  .await;
  assert_eq!(fatals.len(), 2);
  let fatal = &fatals[0];
  assert_eq!(fatal.group_id, group);
  assert_eq!(fatal.id, "login-item");
  assert!(!fatal.internal);
  assert!(fatal.message.contains("timed out"));
  assert_eq!(fatal.details.as_deref(), Some("loginConsentTimeout"));
  assert_eq!(released, vec![group.clone(), second.clone()]);
  assert!(!PENDING.lock().unwrap().contains_key(&token));
  expire_pending(
    &token,
    std::future::ready(()),
    |_, _| panic!("duplicate fatal"),
    |_| panic!("duplicate release"),
  )
  .await;
  crate::scheduling::group_state::remove_group(&group);
  crate::scheduling::group_state::remove_group(&second);
}
#[tokio::test]
async fn accept_and_change_leave_shared_config_unchanged_after_a_failed_attempt() {
  use crate::state::{config::TestConfigHandle, config_models::Config};
  use std::sync::Arc;
  let root = crate::runners::gallery_files::Staging::new().unwrap();
  let app = tauri::test::mock_builder()
    .plugin(tauri_plugin_store::Builder::default().build())
    .build(tauri::test::mock_context(tauri::test::noop_assets()))
    .unwrap();
  let mut initial = Config::default();
  initial
    .auth
    .site_browsers
    .insert("example.com".into(), "firefox:/old".into());
  let config =
    Arc::new(TestConfigHandle::isolated_for_test(app.handle(), &root.path, initial).unwrap());
  app.manage(config.clone());
  let (fetch_tx, mut fetch_rx) = tokio::sync::mpsc::unbounded_channel();
  let (download_tx, _download_rx) = tokio::sync::mpsc::unbounded_channel();
  app.manage(FetchSender(fetch_tx));
  app.manage(DownloadSender(download_tx));
  let before = serde_json::to_value(config.load().as_ref()).unwrap();
  for change_only in [false, true] {
    let group = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let offered = candidates(None);
    let browser = offered[0].id.clone();
    let mut entry = match pending(&group).request {
      LoginRequest::Fetch(entry) => entry,
      _ => unreachable!(),
    };
    if change_only {
      CHOICES.lock().unwrap().insert(
        token.clone(),
        ProfileChoice {
          used: LoginUsing {
            id: entry.id.clone(),
            group_id: group.clone(),
            host: "example.com".into(),
            candidate: offered[0].clone(),
            selected_only: false,
          },
          candidates: offered,
          created: Instant::now(),
        },
      );
    } else {
      PENDING
        .lock()
        .unwrap()
        .insert(token.clone(), pending(&group));
    }
    accept_login(app.handle().clone(), token, opaque_candidate_id(&browser)).unwrap();
    if change_only {
      entry.login_browser = row_browser(&group, &entry.id, &entry.url);
      entry.login_was_accepted = true;
    } else {
      let DispatchRequest::Pipeline(FetchRequest::Resume(queued)) = fetch_rx.try_recv().unwrap()
      else {
        panic!("resume not queued");
      };
      entry = *queued;
    }
    assert_eq!(entry.login_browser.as_deref(), Some(browser.as_str()));
    let mut session =
      LoginSession::from_auth(config.load().auth.clone(), &entry.url, entry.login_browser);
    session.begin_fetch(Phase::Login);
    let outcome = crate::extract::chain::failed_login_for_test(&entry.url, session).await;
    assert_eq!(outcome, Some("signInRequired"));
    assert_eq!(
      serde_json::to_value(app.state::<Arc<TestConfigHandle>>().load().as_ref()).unwrap(),
      before,
      "{change_only}: acceptance and the failed worker must not alter SharedConfig"
    );
    forget_group(&group);
    USED.lock().unwrap().remove(&group);
    crate::scheduling::group_state::remove_group(&group);
  }
  assert!(!root.path.join("config.store.json").exists());
}

#[test]
fn acceptance_of_a_cancelled_owner_still_resumes_live_followers() {
  let app = tauri::test::mock_app();
  let (fetch_tx, mut fetch_rx) = tokio::sync::mpsc::unbounded_channel();
  let (download_tx, _download_rx) = tokio::sync::mpsc::unbounded_channel();
  app.manage(FetchSender(fetch_tx));
  app.manage(DownloadSender(download_tx));
  let cancelled = uuid::Uuid::new_v4().to_string();
  let live = uuid::Uuid::new_v4().to_string();
  let token = uuid::Uuid::new_v4().to_string();
  let mut owner = pending(&cancelled);
  let (_tx, rx) = watch::channel(false);
  owner.cancel = rx;
  owner.followers.push(pending(&live));
  PENDING.lock().unwrap().insert(token.clone(), owner);
  accept_login(
    app.handle().clone(),
    token,
    opaque_candidate_id("chrome:/fake/Profile 1"),
  )
  .unwrap();
  let mut resumed = Vec::new();
  while let Ok(message) = fetch_rx.try_recv() {
    if let DispatchRequest::Pipeline(FetchRequest::Resume(entry)) = message {
      resumed.push(entry.group_id);
    }
  }
  assert_eq!(resumed, vec![live.clone()]);
  forget_group(&cancelled);
  forget_group(&live);
  crate::scheduling::group_state::remove_group(&cancelled);
  crate::scheduling::group_state::remove_group(&live);
}

#[test]
fn anonymous_fetch_success_carries_no_cookies_into_the_download() {
  let group = uuid::Uuid::new_v4().to_string();
  let mut previous = pending(&group).request;
  if let LoginRequest::Fetch(entry) = &mut previous {
    entry.id = "item".into();
    entry.url = "https://redd.it/item".into();
  }
  previous.consent_to_browser("chrome:/remembered");
  let LoginRequest::Fetch(previous) = previous else {
    unreachable!()
  };
  let fresh = crate::scheduling::fetch_pipeline::fresh_fetch_for_test(&previous);
  assert!(
    fresh.login_browser.is_none() && !fresh.login_was_accepted,
    "re-fetches start anonymous even when this row has an accepted browser"
  );
  let mut auth = AuthSettings::default();
  record_host_browser(&mut auth, "reddit.com", Some("chrome:/remembered".into()));
  let mut session = LoginSession::from_auth(auth, "https://redd.it/item", None);
  session.begin_fetch(Phase::Anonymous);
  session.carry_fetch_browser("item", &group);
  let item = serde_json::from_value::<crate::models::DownloadItem>(serde_json::json!({
    "id":"item", "url":"https://old.reddit.com/item", "format":{"trackType":"both"}, "templateContext":{"values":{}}
  })).unwrap();
  let download = DownloadEntry::from((item, group.clone()));
  assert_eq!(download.login_browser.as_deref(), Some("none"));
  forget_group(&group);
}

fn pending(group_id: &str) -> PendingLogin {
  PendingLogin {
    followers: Vec::new(),
    announcement: None,
    request: LoginRequest::Fetch(FetchEntry {
      group_id: group_id.into(),
      id: "login-item".into(),
      url: "https://example.com/private".into(),
      total: 1,
      format: None,
      overrides: None,
      login_browser: None,
      login_was_accepted: false,
      prior_attempts: Vec::new(),
      prior_resolved_url: None,
    }),
    candidates: candidates(None),
    cancel: subscribe_group(group_id),
    created: Instant::now(),
    cleanup: CleanupChannels {
      fetch: FetchSender(tokio::sync::mpsc::unbounded_channel().0),
      download: DownloadSender(tokio::sync::mpsc::unbounded_channel().0),
    },
  }
}
#[test]
fn consent_is_single_use_and_cannot_substitute_an_unoffered_profile() {
  let group = uuid::Uuid::new_v4().to_string();
  let mut map = HashMap::new();
  map.insert("token".into(), pending(&group));
  assert!(claim_pending(&mut map, "token", "chrome:/other/private-profile").is_err());
  assert_eq!(map.len(), 1);
  let selected = claim_pending(&mut map, "token", "chrome:/fake/Profile 1").unwrap();
  assert_eq!(selected.request.ids().0, "login-item");
  assert!(map.is_empty());
  assert!(claim_pending(&mut map, "token", "chrome:/fake/Profile 1").is_err());
  let mut choices = HashMap::from([(
    "choice".into(),
    ProfileChoice {
      used: LoginUsing {
        id: "login-item".into(),
        group_id: group.clone(),
        host: "example.com".into(),
        candidate: candidates(None).remove(0),
        selected_only: false,
      },
      candidates: candidates(None),
      created: Instant::now(),
    },
  )]);
  assert!(claim_choice(&mut choices, "choice", "chrome:/other/private-profile").is_err());
  assert_eq!(choices.len(), 1);
  let selected = claim_choice(&mut choices, "choice", "chrome:/fake/Profile 1").unwrap();
  assert_eq!(selected.used.group_id, group);
  assert!(choices.is_empty());
  assert!(claim_choice(&mut choices, "choice", "chrome:/fake/Profile 1").is_err());

  crate::scheduling::group_state::remove_group(&group);
}
#[tokio::test]
async fn cancelling_a_group_invalidates_its_pending_login() {
  let group = uuid::Uuid::new_v4().to_string();
  let token = uuid::Uuid::new_v4().to_string();
  let (fetch, mut fetch_rx) = tokio::sync::mpsc::unbounded_channel();
  let (download, mut download_rx) = tokio::sync::mpsc::unbounded_channel();
  let mut request = pending(&group);
  request.cleanup = CleanupChannels {
    fetch: FetchSender(fetch),
    download: DownloadSender(download),
  };
  PENDING.lock().unwrap().insert(token.clone(), request);
  let cancel = subscribe_group(&group);
  crate::scheduling::group_state::cancel_group(&group);
  assert!(!*cancel.borrow());
  assert!(!PENDING.lock().unwrap().contains_key(&token));
  assert!(
    matches!(fetch_rx.try_recv().unwrap(),DispatchRequest::Cleanup {group_id} if group_id == group)
  );
  assert!(
    matches!(download_rx.try_recv().unwrap(),DispatchRequest::Cleanup {group_id} if group_id == group)
  );
  expire_pending(
    &token,
    std::future::ready(()),
    |_, _| panic!("fatal after cancel"),
    |_| panic!("duplicate cleanup"),
  )
  .await;
  crate::scheduling::group_state::remove_group(&group);
}
fn candidates(_: Option<&str>) -> Vec<LoginCandidate> {
  vec![LoginCandidate {
    id: "chrome:/fake/Profile 1".into(),
    browser: BrowserKind::Chrome,
    profile: "Profile 1".into(),
    signed_in: false,
    app_bound: false,
  }]
}
#[test]
fn no_cookie_probes_before_refusal_or_after_cancel_and_first_refusal_stops() {
  let mut session = LoginSession::from_auth(
    AuthSettings {
      cookie_browser: "automatic".into(),
      ..Default::default()
    },
    "https://www.instagram.com/p/test",
    None,
  );
  assert_eq!(
    session
      .decide(None, false, false, |_| panic!("unexpected profile probe"))
      .0,
    RetryAction::Stop
  );
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, true, |_| panic!(
        "probe after cancel"
      ))
      .0,
    RetryAction::Stop
  );
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, false, candidates)
      .0,
    RetryAction::AskLogin
  );
  assert!(session.browser.is_none());
  assert!(session.success_patch().is_none());
}
#[test]
fn opted_in_refusal_tries_each_profile_once_and_success_remembers_the_host() {
  let auth = AuthSettings {
    cookie_browser: "automatic".into(),
    always_use_browser_logins: true,
    ..Default::default()
  };
  let mut session = LoginSession::from_auth(
    auth,
    "https://www.instagram.com/p/private?token=secret",
    None,
  );
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, false, candidates)
      .0,
    RetryAction::Browser
  );
  assert_eq!(session.browser.as_deref(), Some("chrome:/fake/Profile 1"));
  let patch = session.success_patch().unwrap();
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, false, candidates)
      .0,
    RetryAction::AskLogin
  );
  let mut config = serde_json::to_value(crate::state::config_models::Config::default()).unwrap();
  crate::state::json_merge(&mut config, &patch);
  let config: crate::state::config_models::Config = serde_json::from_value(config).unwrap();
  assert_eq!(config.auth.site_browsers.len(), 1);
  assert_eq!(
    config.auth.site_browsers["instagram.com"],
    "chrome:/fake/Profile 1"
  );
}
#[test]
fn explicit_browser_and_file_overrides_do_not_trigger_automatic_probes() {
  for auth in [
    AuthSettings {
      cookie_browser: "firefox".into(),
      ..Default::default()
    },
    AuthSettings {
      cookie_file: Some("/manual.txt".into()),
      ..Default::default()
    },
  ] {
    let mut session = LoginSession::from_auth(auth, "https://example.com/", None);
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, |_| panic!(
          "manual override probe"
        ))
        .0,
      RetryAction::Stop
    );
  }
}
