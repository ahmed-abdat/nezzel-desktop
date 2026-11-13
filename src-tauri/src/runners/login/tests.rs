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
fn fixed_browser_fetch_uses_the_saved_profile_without_another_prompt() {
  use crate::runners::browsers::{BrowserSource, CookieSource};
  for (fixed, accepted, saved, expected) in [
    (
      "chrome",
      None,
      "chrome:/saved/Default",
      "chrome:/saved/Default",
    ),
    ("chrome", None, "firefox:/different", "chrome"),
    (
      "chrome:/explicit",
      None,
      "chrome:/saved/Default",
      "chrome:/explicit",
    ),
    (
      "chrome",
      Some("chrome:/row"),
      "chrome:/saved/Default",
      "chrome:/row",
    ),
  ] {
    let auth = AuthSettings {
      cookie_browser: fixed.into(),
      browser_login_consent: Some(true),
      always_use_browser_logins: true,
      site_browsers: std::collections::HashMap::from([("instagram.com".into(), saved.into())]),
      ..Default::default()
    };
    let mut session = LoginSession::from_auth(
      auth.clone(),
      "https://www.instagram.com/p/BCISVm-GVr7/",
      accepted.map(str::to_owned),
    );
    assert_eq!(
      CookieSource::for_site(&auth, session.browser.as_deref(), "instagram.com"),
      Ok(CookieSource::Browser(
        BrowserSource::parse(expected).unwrap()
      ))
    );
    assert_eq!(session.begin_fetch(Phase::Anonymous), Phase::Anonymous);
    assert!(!session.uses_cookies());
    assert_eq!(
      CookieSource::for_site(&auth, session.fetch_browser.as_deref(), "instagram.com"),
      Ok(CookieSource::Browser(
        BrowserSource::parse(expected).unwrap()
      ))
    );
  }
}

#[test]
fn fresh_automatic_fetch_is_anonymous_even_with_an_accepted_browser() {
  use crate::runners::browsers::{CookieSource, AUTOMATIC};
  for host in [
    "fb.watch",
    "pin.it",
    "www.snapchat.com",
    "t.snapchat.com",
    "facebook.com",
    "unknown.example",
  ] {
    let auth = AuthSettings::default();
    assert_eq!(auth.cookie_browser, AUTOMATIC);
    let mut session = LoginSession::from_auth(
      auth.clone(),
      &format!("https://{host}/public"),
      Some("chrome:Default".into()),
    );
    session.begin_fetch(Phase::Login);
    assert_eq!(
      CookieSource::for_site(&auth, session.browser.as_deref(), host),
      Ok(CookieSource::Anonymous),
      "{host}: the public attempt must reach extraction without a cookie read"
    );
    assert!(!session.uses_cookies(), "{host}");
    assert_eq!(
      session
        .decide(None, false, false, |_| panic!(
          "discovery before a login wall"
        ))
        .0,
      RetryAction::Stop,
      "{host}: no login pending on a public success or ordinary refusal"
    );
  }
}

#[tokio::test]
async fn automatic_instagram_resume_runs_the_stage_with_cookies_once() {
  use crate::extract::{
    chain::StageResult,
    outcome::{AuthUsed, Confidence, Outcome, StageId},
  };
  use crate::runners::browser_cookies::{CookieAccess, CookieContext};
  let root = crate::runners::gallery_files::Staging::new().unwrap();
  let profile = root.path.join("profile");
  std::fs::create_dir(&profile).unwrap();
  let db = rusqlite::Connection::open(profile.join("cookies.sqlite")).unwrap();
  db.execute_batch("CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, originAttributes TEXT);
    INSERT INTO moz_cookies VALUES ('.instagram.com', 'sessionid', 'fixture-session', '/', 4102444800, 1, 1, '');").unwrap();
  drop(db);
  let browser = format!("firefox:{}", profile.display());
  let group = uuid::Uuid::new_v4().to_string();
  crate::scheduling::group_state::ensure_group_running(&group);
  let app = tauri::test::mock_app();
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  app.manage(FetchSender(tx));
  let mut entry = match pending(&group).request {
    LoginRequest::Fetch(entry) => entry,
    _ => unreachable!(),
  };
  entry.url = "https://www.instagram.com/p/BCISVm-GVr7/".into();
  let auth = AuthSettings::default();
  let mut session = LoginSession::from_auth(auth.clone(), &entry.url, None);
  assert_eq!(session.begin_fetch(Phase::Anonymous), Phase::Anonymous);
  let context = CookieContext {
    exporter: root.path.join("must-not-run"),
    directory: root.path.join("cookies"),
  };
  let (_tx, cancel) = watch::channel(true);
  let cookies = CookieAccess::prepare(
    &auth,
    session.browser.as_deref(),
    &entry.url,
    &context,
    cancel.clone(),
  )
  .await
  .unwrap();
  assert!(!cookies.has_session);
  let mut first = StageResult::<()>::new(StageId::Gallery, Confidence::High);
  first.record_code("signInRequired", "", cookies.has_session, Instant::now());
  let mut resumes = 0;
  {
    let mut schedule = |request: LoginRequest| {
      resumes += 1;
      request.enqueue(app.handle(), browser.clone()).unwrap();
      RetryAction::Pending
    };
    // Gallery can report a login failure before the coordinator has recorded its history.
    let action = session.automatic_retry(LoginRequest::Fetch(entry.clone()), &mut schedule);
    if action == RetryAction::Stop {
      session.set_fetch_history(first.attempts, Some(entry.url.clone()));
      assert_eq!(
        session.automatic_retry(LoginRequest::Fetch(entry), &mut schedule),
        RetryAction::Pending
      );
    }
  }
  let DispatchRequest::Pipeline(FetchRequest::Resume(resumed)) = rx.try_recv().unwrap() else {
    panic!("automatic fetch was not queued");
  };
  let mut retry =
    LoginSession::from_auth(auth.clone(), &resumed.url, resumed.login_browser.clone());
  retry.set_fetch_history(
    resumed.prior_attempts.clone(),
    resumed.prior_resolved_url.clone(),
  );
  retry.begin_fetch(Phase::Login);
  let cookies = CookieAccess::prepare(
    &auth,
    retry.browser.as_deref(),
    &resumed.url,
    &context,
    cancel,
  )
  .await
  .unwrap();
  let stage = StageResult::<()>::new(StageId::Gallery, Confidence::High).finish(
    Outcome::Verified,
    cookies.has_session,
    Instant::now(),
  );
  assert_eq!(
    stage.attempts[0].auth,
    AuthUsed::Browser,
    "resumed stage must send the site session"
  );
  assert!(cookies.has_proof("instagram.com"));
  assert_eq!(resumes, 1);
  assert_eq!(
    retry.automatic_retry(LoginRequest::Fetch(*resumed), |_| panic!(
      "second automatic fetch resume"
    )),
    RetryAction::Stop
  );
  assert!(rx.try_recv().is_err());
  crate::scheduling::group_state::cancel_group(&group);
  crate::scheduling::group_state::remove_group(&group);
}

#[test]
fn short_links_use_the_parent_site_session_and_consent() {
  use crate::runners::browser_cookies::{known_login_host, proof_cookies, site_key};
  for (host, canonical, names, supported) in [
    ("fb.watch", "facebook.com", vec!["c_user", "xs"], true),
    ("pin.it", "pinterest.com", vec!["_pinterest_sess"], true),
    (
      "www.snapchat.com",
      "snapchat.com",
      vec!["sc-a-session"],
      false,
    ),
    (
      "t.snapchat.com",
      "snapchat.com",
      vec!["sc-a-session"],
      false,
    ),
  ] {
    assert_eq!(site_key(host).as_deref(), Some(canonical), "{host}");
    assert_eq!(known_login_host(host), supported, "{host}");
    assert_eq!(
      proof_cookies(
        host,
        names.iter().map(|name| (canonical, *name, "session", 0)),
        1,
      ),
      supported,
      "{host}: only supported sites may prove a session on retry"
    );
    assert!(
      !proof_cookies(
        host,
        names
          .iter()
          .map(|name| ("other.example", *name, "session", 0)),
        1,
      ),
      "{host}: unrelated cookies must stay excluded"
    );
  }
}

#[test]
fn unsupported_login_hosts_never_park_downloads_in_automatic_mode() {
  use crate::runners::browser_cookies::known_login_host;
  use crate::runners::browsers::CookieSource;
  for (host, browser) in [
    ("snapchat.com", None),
    ("www.snapchat.com", Some(ANONYMOUS_BROWSER)),
    ("t.snapchat.com", Some("chrome:Default")),
    ("unknown.example", None),
  ] {
    assert!(!known_login_host(host), "{host}");
    let auth = AuthSettings::default();
    let mut session = LoginSession::from_auth(
      auth.clone(),
      &format!("https://{host}/spotlight/public"),
      browser.map(str::to_owned),
    );
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, |_| panic!(
          "unsupported login discovery"
        ))
        .0,
      RetryAction::Stop,
      "{host} {browser:?}: downloads must never wait for automatic login"
    );
    if browser.is_none() {
      assert_eq!(
        CookieSource::for_site(&auth, None, host),
        Ok(CookieSource::Anonymous)
      );
    }
  }
}

#[test]
fn consent_migration_preserves_explicit_answers_and_never_policy() {
  for (answer, browser, legacy, always, expected) in [
    (None, "automatic", "", false, Some(true)),
    (None, "automatic", "firefox:/profile", false, Some(true)),
    (None, "automatic", "safari", false, Some(true)),
    (None, "automatic", "not-a-browser", false, Some(true)),
    (None, "automatic", "", true, Some(true)),
    (
      Some(false),
      "automatic",
      "chrome:/profile",
      true,
      Some(false),
    ),
    (Some(true), "automatic", "", false, Some(true)),
    (Some(true), "none", "chrome:/profile", true, Some(false)),
  ] {
    let mut auth = AuthSettings {
      browser_login_consent: answer,
      cookie_browser: browser.into(),
      always_use_browser_logins: always,
      ..Default::default()
    };
    if !legacy.is_empty() {
      auth
        .site_browsers
        .insert("facebook.com".into(), legacy.into());
    }
    migrate_global_consent(&mut auth);
    assert_eq!(global_consent(&auth), expected);
    let before = auth.browser_login_consent;
    migrate_global_consent(&mut auth);
    assert_eq!(auth.browser_login_consent, before);
  }
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
      let session = LoginSession::from_auth(
        auth,
        &format!("https://{host}/item"),
        Some("chrome:/remembered".into()),
      );
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
fn remembered_sources_authorize_global_retry_without_pre_reading_cookies() {
  for source in ["safari", "firefox:/profile", "chrome:/profile"] {
    let mut auth = AuthSettings::default();
    auth
      .site_browsers
      .insert("instagram.com".into(), source.into());
    let mut session = LoginSession::from_auth(auth, "https://facebook.com/watch?v=1", None);
    assert!(session.browser.is_none());
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, |_| panic!(
          "inline discovery"
        ))
        .0,
      RetryAction::Browser
    );
  }
}

#[test]
fn default_retry_does_not_offer_a_blocking_consent_prompt() {
  let mut session = LoginSession::from_auth(
    AuthSettings::default(),
    "https://instagram.com/reel/private",
    None,
  );
  let (action, profiles) = session.decide(Some("signInRequired"), false, false, |_| {
    panic!("inline discovery before scheduled task")
  });
  assert_eq!(action, RetryAction::Browser);
  assert!(profiles.is_empty());
}

#[test]
fn automatic_mode_keeps_the_native_fallback_on_public_sites() {
  for host in ["snapchat.com", "unknown.example"] {
    let mut session = LoginSession::from_auth(
      AuthSettings::default(),
      &format!("https://{host}/clip"),
      None,
    );
    let (action, _) = session.decide(None, true, false, |_| panic!("inline discovery"));
    assert_eq!(action, RetryAction::Native, "{host}");
  }
}

#[test]
fn global_consent_retries_only_known_login_sites_without_another_question() {
  for (host, expected) in [
    ("facebook.com", RetryAction::Browser),
    ("instagram.com", RetryAction::Browser),
    ("fb.watch", RetryAction::Browser),
    ("pin.it", RetryAction::Browser),
    ("t.snapchat.com", RetryAction::Stop),
    ("unknown.example", RetryAction::Stop),
  ] {
    let auth = AuthSettings {
      browser_login_consent: Some(true),
      ..Default::default()
    };
    let mut session = LoginSession::from_auth(auth, &format!("https://{host}/private"), None);
    assert_eq!(
      session
        .decide(Some("signInRequired"), false, false, |_| panic!(
          "inline discovery"
        ))
        .0,
      expected,
      "{host}"
    );
  }
}

#[test]
fn remembered_source_never_overrides_explicit_global_refusal() {
  let mut auth = AuthSettings {
    browser_login_consent: Some(false),
    ..Default::default()
  };
  auth
    .site_browsers
    .insert("instagram.com".into(), "chrome:/old".into());
  let mut session = LoginSession::from_auth(auth, "https://instagram.com/reel/private", None);
  assert!(session.browser.is_none());
  assert_eq!(
    session
      .decide(Some("signInRequired"), false, false, |_| panic!(
        "discovery after refusal"
      ))
      .0,
    RetryAction::Stop
  );
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
    ("automatic", None, Some("chrome:/saved")),
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
      "ERROR: Failed to decrypt with DPAPI",
      "ERROR: Login required",
      "browserDpapiFailed",
    ),
    (
      "ERROR: cookies use App-Bound encryption",
      "ERROR: Unsupported URL",
      "browserAppBound",
    ),
    (
      "ERROR: [Errno 1] Operation not permitted: Safari/Cookies.binarycookies",
      "ERROR: Login required",
      "browserSafariAccessDenied",
    ),
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
fn remembered_source_is_deferred_until_a_login_failure() {
  use crate::runners::browsers::CookieSource;
  for (mode, file, host, accepted, expected) in [
    ("automatic", None, "www.instagram.com", None, None),
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
  assert!(auth.always_use_browser_logins);
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
    session.set_fetch_history(
      vec![crate::extract::outcome::Attempt::failed(
        crate::extract::outcome::StageId::Ytdlp,
        crate::extract::outcome::AuthUsed::Anonymous,
        crate::extract::outcome::Confidence::Medium,
        crate::extract::outcome::classify(
          "signInRequired",
          "",
          crate::extract::outcome::NetworkScope::StageLocal,
        ),
        0,
      )],
      None,
    );
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
    automatic: false,
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
    RetryAction::Browser
  );
  assert!(session.browser.is_none());
  assert!(session.success_patch().is_none());
}
#[test]
fn successful_source_is_remembered_without_changing_global_consent() {
  let auth = AuthSettings {
    browser_login_consent: Some(true),
    ..Default::default()
  };
  let session = LoginSession::from_auth(
    auth.clone(),
    "https://www.instagram.com/p/private",
    Some("firefox:/working".into()),
  );
  let mut saved = auth;
  remember_success(&mut saved, "instagram.com", None, "firefox:/working");
  assert_eq!(saved.site_browsers["instagram.com"], "firefox:/working");
  assert_eq!(saved.browser_login_consent, Some(true));
  assert!(session.success_patch().is_some());
}
#[test]
fn fixed_browser_exports_only_for_known_sites_or_a_login_retry() {
  use RetryAction::{Browser, Stop};
  for (mode, host, retry, export) in [
    ("firefox", "instagram.com", None, true),
    ("firefox", "instagram.com", Some(Stop), true),
    ("firefox", "example.com", None, false),
    ("firefox", "example.com", Some(Browser), true),
    ("none", "instagram.com", None, false),
    ("none", "instagram.com", Some(Stop), false),
    ("none", "example.com", None, false),
    ("none", "example.com", Some(Stop), false),
  ] {
    let mut session = LoginSession::from_auth(
      AuthSettings {
        cookie_browser: mode.into(),
        ..Default::default()
      },
      &format!("https://{host}/video"),
      None,
    );
    if let Some(action) = retry {
      assert_eq!(
        session
          .decide(Some("signInRequired"), false, false, |_| panic!(
            "fixed mode must not discover browsers"
          ))
          .0,
        action
      );
      if action == Browser {
        assert_eq!(
          session
            .decide(Some("signInRequired"), false, false, |_| panic!(
              "no discovery"
            ))
            .0,
          Stop
        );
      }
    }
    assert_eq!(session.uses_cookies(), export, "{mode} {host}");
  }
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
    let mut session = LoginSession::from_auth(auth, "https://instagram.com/", None);
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

#[test]
fn default_retry_is_automatic_without_inline_discovery() {
  let mut session = LoginSession::from_auth(
    AuthSettings::default(),
    "https://facebook.com/watch?v=1",
    None,
  );
  let (action, _) = session.decide(Some("signInRequired"), false, false, |_| {
    panic!("inline discovery before scheduled task")
  });
  assert_eq!(action, RetryAction::Browser);
}

#[test]
fn unknown_legacy_browser_login_policy_migrates_on_without_a_prompt() {
  for raw in [
    r#"{}"#,
    r#"{"browserLoginConsent":null,"alwaysUseBrowserLogins":false}"#,
    r#"{"browserLoginConsent":true}"#,
  ] {
    let mut auth: AuthSettings = serde_json::from_str(raw).unwrap();
    migrate_global_consent(&mut auth);
    assert_eq!(global_consent(&auth), Some(true));
    assert_eq!(auth.browser_login_consent, Some(true));
  }
}
