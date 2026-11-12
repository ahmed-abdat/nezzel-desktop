use crate::models::error::{AppliesTo, DiagnosticEvent, DiagnosticLevel, PatternKind, RulesFile};
use crate::sites::{site_by_domain, SiteId};
use regex::Regex;
use std::borrow::Cow;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

const INPUT_FILTER_SKIPPED_CODE: &str = "input_filter_skipped";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieFetchFailure {
  Transient,
  Refused,
  Other,
}
impl CookieFetchFailure {
  pub fn combine(self, other: Self) -> Self {
    if self == Self::Refused || other == Self::Refused {
      Self::Refused
    } else if self == Self::Transient || other == Self::Transient {
      Self::Transient
    } else {
      Self::Other
    }
  }
}

/// Local cookie access failures, shared by the exporter and both engine parsers.
/// Only a fixed code crosses the cookie export boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CookieReadError {
  KeychainDenied,
  DpapiFailed,
  AppBound,
  SafariAccessDenied,
  DatabaseLocked,
  ProfileMissing,
}
impl CookieReadError {
  pub(crate) fn code(self) -> &'static str {
    match self {
      Self::KeychainDenied => "browserKeychainDenied",
      Self::DpapiFailed => "browserDpapiFailed",
      Self::AppBound => "browserAppBound",
      Self::SafariAccessDenied => "browserSafariAccessDenied",
      Self::DatabaseLocked => "browserDatabaseLocked",
      Self::ProfileMissing => "browserProfileMissing",
    }
  }

  pub(crate) fn classify(message: &str, cookie_context: bool, safari: bool) -> Option<Self> {
    let lower = redact_urls(message).to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    let cookie_context = cookie_context
      || has(&[
        "[cookies]",
        "cookie database",
        "cookies database",
        "decrypt cookie",
        "decrypt v10 cookies",
        "decrypt v11 cookies",
        "cookies.binarycookies",
      ]);
    if (safari || has(&["cookies.binarycookies"]) || (cookie_context && has(&["safari"])))
      && has(&[
        "operation not permitted",
        "permission denied",
        "full disk access",
      ])
    {
      Some(Self::SafariAccessDenied)
    } else if has(&["app-bound", "app bound", "app_bound"]) && has(&["cookie", "encrypt"]) {
      Some(Self::AppBound)
    } else if has(&[
      "failed to decrypt with dpapi",
      "failed to decrypt cookie (dpapi)",
    ]) {
      // A DPAPI failure alone does not prove App-Bound encryption (#10927).
      Some(Self::DpapiFailed)
    } else if has(&[
      "find-generic-password failed",
      "exception running find-generic-password",
      "error when using find-generic-password",
      "cannot decrypt v10 cookies",
      "cannot decrypt v11 cookies",
      "unable to decrypt v10 cookies",
      "unable to decrypt v11 cookies",
      "failed to read from keyring",
      "secretstorage not available",
      "kwallet-query command not found",
      "kwallet-query failed",
      "exception running kwallet-query",
      "failed to read networkwallet",
      "exception while obtaining networkwallet",
    ]) || (cookie_context && has(&["failed to decrypt"]))
    {
      Some(Self::KeychainDenied)
    } else if cookie_context
      && has(&[
        "database is locked",
        "database is busy",
        "could not copy chrome cookie database",
        "being used by another process",
      ])
    {
      Some(Self::DatabaseLocked)
    } else if has(&[
      "cookies database",
      "cookie database",
      "firefox container",
      "containers.json",
    ]) && has(&[
      "could not find",
      "unable to find",
      "not found",
      "no such file",
      "could not read",
      "unable to read",
    ]) {
      Some(Self::ProfileMissing)
    } else {
      None
    }
  }
}

/// Transport outages cannot be repaired by changing a browser session.
pub fn network_down(code: &str, message: &str) -> bool {
  if matches!(
    code,
    "networkNameResolutionFailed" | "unableToConnectToProxy"
  ) {
    return true;
  }
  let lower = message.to_ascii_lowercase();
  [
    "network is unreachable",
    "network unreachable",
    "connection refused",
    "name or service not known",
    "nodename nor servname",
    "temporary failure in name resolution",
    "getaddrinfo failed",
  ]
  .iter()
  .any(|needle| lower.contains(needle))
}

/// yt-dlp wording the fetch chain uses to refine an inconclusive yt-dlp stage diagnostic.
pub(crate) mod stage_hint {
  /// The extractor ran and found nothing to download.
  pub(crate) fn no_media_found(message: &str) -> bool {
    message.to_ascii_lowercase().contains("no media found")
  }

  /// The item has no video representation (photo posts, audio-only slideshows).
  pub(crate) fn no_formats(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
      "no video formats",
      "no formats",
      "no available formats",
      "not a video",
      "no video could be found in this tweet",
    ]
    .iter()
    .any(|hint| message.contains(hint))
  }

  /// The requested selection failed; other formats may still exist.
  pub(crate) fn requested_format_unavailable(message: &str) -> bool {
    message
      .to_ascii_lowercase()
      .contains("requested format is not available")
  }
}

pub fn redact_urls(text: &str) -> String {
  static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:https?|ftp)://[^\s<>"']+"#).expect("valid URL redaction regex")
  });
  static HOST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?\.[a-z]{2,}(?::[0-9]+)?/[^\s<>"']*"#)
      .expect("valid scheme-less URL regex")
  });
  let urls = URL.replace_all(text, "[redacted URL]");
  HOST_URL.replace_all(&urls, "[redacted URL]").into_owned()
}

/// Engine errors can echo Cookie/Authorization headers and local browser database paths.
pub fn redact_diagnostic(text: &str) -> String {
  static CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:cookies?|set-cookie|authorization|proxy-authorization|sessionid|ds_user_id|mid|csrftoken|c_user|xs|auth_token|access_token|refresh_token|sapisid|__secure-3psid|login_info|reddit_session|li_at|_pinterest_sess|vimeo|sc-a-session)["']?\s*[:=,][^\r\n]*"#)
      .expect("valid credential redaction regex")
  });
  static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?P<prefix>^|[\s\"'=:(\[,])(?:[A-Za-z]:[\\/]|\\\\|~?/)[^\r\n<>\"'\]]+"#)
      .expect("valid path redaction regex")
  });
  static POST_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<extractor>\[[^\]\r\n]+\]\s+)[A-Za-z0-9_-]+:\s")
      .expect("valid extractor id regex")
  });
  let urls = redact_urls(text);
  let credentials = CREDENTIAL.replace_all(&urls, "[redacted credentials]");
  let paths = PATH.replace_all(&credentials, "${prefix}[redacted path]");
  POST_ID
    .replace_all(&paths, "${extractor}[redacted ID]: ")
    .into_owned()
}

#[derive(Clone)]
struct LoadedPattern {
  kind: PatternKind,
  value_lower: Option<String>,
  regex: Option<Regex>,
}

#[derive(Clone)]
struct LoadedRule {
  code: String,
  component_lower: Option<String>,
  applies_to: AppliesTo,
  patterns: Vec<LoadedPattern>,
}

#[derive(Clone)]
pub struct DiagnosticMatcher {
  rules: Vec<LoadedRule>,
  fallback_code: String,
}

impl DiagnosticMatcher {
  /// The app's `diagnostic_rules.json`, parsed once per run (a copy shares the compiled regexes).
  pub fn shared() -> Result<Self, String> {
    static SHARED: std::sync::OnceLock<Result<DiagnosticMatcher, String>> =
      std::sync::OnceLock::new();
    SHARED
      .get_or_init(|| Self::from_json(include_str!("../diagnostic_rules.json")))
      .clone()
  }

  pub fn from_json(json: &str) -> Result<Self, String> {
    let parsed: RulesFile =
      serde_json::from_str(json).map_err(|e| format!("Failed to parse rules: {e}"))?;

    let mut rules = Vec::with_capacity(parsed.rules.len());
    for rs in parsed.rules {
      let component_lower = rs.component.as_ref().map(|s| s.to_ascii_lowercase());
      let mut patterns = Vec::with_capacity(rs.patterns.len());
      for ps in rs.patterns {
        match ps.kind {
          PatternKind::Substr => patterns.push(LoadedPattern {
            kind: PatternKind::Substr,
            value_lower: Some(ps.value.to_ascii_lowercase()),
            regex: None,
          }),
          PatternKind::Regex => {
            let re = Regex::new(&ps.value)
              .map_err(|e| format!("Invalid regex in rule {}: {e}", rs.code))?;
            patterns.push(LoadedPattern {
              kind: PatternKind::Regex,
              value_lower: None,
              regex: Some(re),
            });
          }
        }
      }
      rules.push(LoadedRule {
        code: rs.code,
        component_lower,
        applies_to: rs.applies_to,
        patterns,
      });
    }

    Ok(Self {
      rules,
      fallback_code: parsed.fallback_code,
    })
  }

  fn match_code(&self, level: &DiagnosticLevel, component: Option<&str>, msg: &str) -> &str {
    let msg = redact_urls(msg);
    let msg_lower = msg.to_ascii_lowercase();
    let comp_lower = component.map(str::to_ascii_lowercase);

    for r in &self.rules {
      match r.applies_to {
        AppliesTo::Error if !matches!(level, DiagnosticLevel::Error) => continue,
        AppliesTo::Warning if !matches!(level, DiagnosticLevel::Warning) => continue,
        _ => {}
      }

      if let Some(rule_comp) = &r.component_lower {
        match &comp_lower {
          Some(c) if c != rule_comp => continue,
          None => continue,
          _ => {}
        }
      }

      for p in &r.patterns {
        match p.kind {
          PatternKind::Substr => {
            if let Some(needle) = &p.value_lower {
              if msg_lower.contains(needle) {
                return &r.code;
              }
            }
          }
          PatternKind::Regex => {
            if let Some(re) = &p.regex {
              if re.is_match(&msg) {
                return &r.code;
              }
            }
          }
        }
      }
    }

    &self.fallback_code
  }
}

pub struct YtdlpErrorParser {
  id: String,
  group_id: String,
  matcher: DiagnosticMatcher,
  source_url: Option<reqwest::Url>,
}

impl YtdlpErrorParser {
  pub fn new(id: &str, group_id: &str, matcher: DiagnosticMatcher) -> Self {
    Self {
      id: id.into(),
      group_id: group_id.into(),
      matcher,
      source_url: None,
    }
  }

  pub fn with_source_url(mut self, url: &str) -> Self {
    self.source_url = reqwest::Url::parse(url)
      .ok()
      .filter(|url| matches!(url.scheme(), "http" | "https"));
    self
  }

  pub fn login_reason(&self, line: &str) -> Option<String> {
    self.login_reason_with_cookies(line, false)
  }

  pub fn login_reason_with_cookies(&self, line: &str, used_cookies: bool) -> Option<String> {
    self
      .parse_line(line)
      .filter(|e| {
        matches!(e.level, DiagnosticLevel::Error)
          && (matches!(
            e.code.as_str(),
            "signInRequired" | "signInRequiredForBotDetection" | "membersOnly"
          ) || !used_cookies
            && e.code == "unableToExtractInitialData"
            && self.source_url.as_ref().is_some_and(|url| {
              let host = url.host_str().unwrap_or_default();
              let component = e.component.as_deref().unwrap_or_default();
              (component.eq_ignore_ascii_case("facebook")
                && e.message.to_ascii_lowercase().contains("cannot parse data")
                && crate::sites::host_is(host, "facebook.com"))
                || (component.eq_ignore_ascii_case("instagram:user")
                  && e
                    .message
                    .to_ascii_lowercase()
                    .contains("unable to extract data")
                  && crate::sites::host_is(host, "instagram.com")
                  && {
                    let path = url.path().trim_matches('/');
                    !path.is_empty()
                      && path
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.'))
                      && !matches!(
                        path,
                        "accounts"
                          | "explore"
                          | "direct"
                          | "stories"
                          | "reels"
                          | "reel"
                          | "p"
                          | "tv"
                          | "share"
                      )
                  })
            }))
      })
      .map(|e| e.code)
  }

  pub fn cookie_fetch_failure(&self, stderr: &str, has_cookies: bool) -> CookieFetchFailure {
    if !has_cookies {
      return CookieFetchFailure::Other;
    }
    let mut failure = CookieFetchFailure::Other;
    for event in stderr.lines().filter_map(|line| self.parse_line(line)) {
      if !matches!(event.level, DiagnosticLevel::Error) {
        continue;
      }
      let lower = event.message.to_ascii_lowercase();
      if [
        "session has expired",
        "session expired",
        "invalid session",
        "cookies are no longer valid",
      ]
      .iter()
      .any(|needle| lower.contains(needle))
      {
        return CookieFetchFailure::Refused;
      }
      // Instagram appends generic login/cookie advice to empty and throttled responses.
      // That advice does not establish whether a supplied session was accepted.
      if lower.contains("empty media response")
        || lower.contains("rate-limit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("http error 429")
      {
        failure = CookieFetchFailure::Transient;
      } else if matches!(
        self.matcher.match_code(
          &event.level,
          event.component.as_deref(),
          lower.split("use --cookies").next().unwrap_or_default(),
        ),
        "signInRequired" | "signInRequiredForBotDetection" | "membersOnly"
      ) {
        return CookieFetchFailure::Refused;
      }
    }
    failure
  }

  pub fn parse_fetch_line(&self, line: &str, has_cookies: bool) -> Option<DiagnosticEvent> {
    let mut event = self.parse_line(line)?;
    if !has_cookies
      && event.code == "unableToExtractInitialData"
      && self.login_reason_with_cookies(line, false).is_some()
    {
      // Anonymous parse walls must reach the chain's login gate before the generic error.
      event.code = "signInRequired".into();
    }
    if has_cookies && self.login_reason_with_cookies(line, true).is_some() {
      event.code = match self.cookie_fetch_failure(line, has_cookies) {
        CookieFetchFailure::Transient
          if event
            .message
            .to_ascii_lowercase()
            .contains("empty media response") =>
        {
          "unableToExtractIdOrInfo".into()
        }
        CookieFetchFailure::Transient => "rateLimited429".into(),
        CookieFetchFailure::Other => "unknown".into(),
        CookieFetchFailure::Refused => "accountAccessDenied".into(),
      };
    }
    event.message = redact_diagnostic(&event.message);
    event.raw = redact_diagnostic(&event.raw);
    Some(event)
  }

  pub fn parse_lines(&self, lines: Vec<&str>, has_cookies: bool) -> Vec<DiagnosticEvent> {
    let mut events = Vec::with_capacity(lines.len());
    for line in lines {
      if let Some(event) = self.parse_fetch_line(line, has_cookies) {
        events.push(event);
      }
    }
    events
  }

  pub fn parse_line(&self, line: &str) -> Option<DiagnosticEvent> {
    let trimmed = line.trim_end();

    if let Some(event) = self.parse_skip_line(trimmed) {
      return Some(event);
    }

    let (level, rest) = if let Some(msg) = trimmed.strip_prefix("ERROR:") {
      (DiagnosticLevel::Error, msg.trim())
    } else {
      if let Some(msg) = trimmed.strip_prefix("WARNING:") {
        (DiagnosticLevel::Warning, msg.trim())
      } else if ["[in#", "[hls @", "[http @", "[https @"]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
        && matches!(
          self
            .matcher
            .match_code(&DiagnosticLevel::Error, None, trimmed),
          "drmProtected" | "accessForbidden403"
        )
      {
        // ffmpeg reports DRM and CDN refusals before yt-dlp's generic exit-code error.
        (DiagnosticLevel::Error, trimmed)
      } else {
        return None;
      }
    };

    let (component, rest) = split_component(rest);
    let (_video_id, message) = split_video_id(rest.as_ref(), component.is_some());

    let cookie_error = CookieReadError::classify(
      &message,
      component
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case("cookies")),
      false,
    );
    let matched_code = cookie_error.map(CookieReadError::code).unwrap_or_else(|| {
      self
        .matcher
        .match_code(&level, component.as_deref(), &message)
    });
    static LINKEDIN_NO_VIDEO: LazyLock<Regex> = LazyLock::new(|| {
      Regex::new(r"(?i)^(?:[0-9]+:\s*)?Unable to extract video(?:\s*;|\s*$)").unwrap()
    });
    let code = if matched_code == "unknown"
      && matches!(level, DiagnosticLevel::Error)
      && component
        .as_deref()
        .is_some_and(|c| c.eq_ignore_ascii_case("LinkedIn"))
      && LINKEDIN_NO_VIDEO.is_match(&message)
    {
      // An inconclusive post reader can leave this to yt-dlp. Missing video does not prove
      // a photo post, and it does not mean photo downloads are unsupported.
      "imageSourceUnavailable"
    } else if matched_code == "urlNotSupported" && self.is_photo_refusal(&message) {
      "photosOnly"
    } else if matched_code == "urlUnsupported" && self.source_url.is_some() {
      // A malformed target extracted from a valid source does not invalidate the user's link.
      "urlNotSupported"
    } else {
      matched_code
    };

    Some(DiagnosticEvent {
      id: self.id.clone(),
      group_id: self.group_id.clone(),
      level,
      code: code.to_string(),
      component,
      message: redact_urls(&message),
      raw: redact_urls(trimmed),
      timestamp: SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0),
    })
  }

  fn is_photo_refusal(&self, message: &str) -> bool {
    let Some(source) = &self.source_url else {
      return false;
    };
    let host = SiteOrHost::of(source);
    let path = source.path();
    let photo_path = match host {
      SiteOrHost::Site(SiteId::X) => path.contains("/status/") && path.contains("/photo/"),
      SiteOrHost::Site(SiteId::TikTok) => path.contains("/photo/"),
      SiteOrHost::Site(SiteId::Reddit) => path.starts_with("/gallery/"),
      SiteOrHost::Site(SiteId::Facebook) => path.contains("/photos/") || path == "/photo.php",
      _ => false,
    };
    let image_file = path.rsplit_once('.').is_some_and(|(_, ext)| {
      matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "avif" | "heic" | "gif"
      )
    });
    if photo_path || image_file {
      return true;
    }
    let SiteOrHost::Site(site) = host else {
      return false;
    };
    let social_post = match site {
      SiteId::X => path.contains("/status/"),
      SiteId::Instagram => path.starts_with("/p/"),
      SiteId::Threads => path.contains("/post/"),
      SiteId::LinkedIn => path.starts_with("/posts/") || path.starts_with("/feed/update/"),
      SiteId::Facebook => path.contains("/posts/"),
      SiteId::Reddit => path.contains("/comments/"),
      SiteId::Pinterest => path.starts_with("/pin/"),
      SiteId::Bluesky => path.starts_with("/profile/") && path.contains("/post/"),
      SiteId::Telegram => path
        .rsplit('/')
        .next()
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit())),
      _ => false,
    };
    if !social_post {
      return false;
    }
    // Photo tweets can make yt-dlp follow a link card instead of the post's attachments.
    // Only reclassify an unsupported external target; login/network errors keep their codes.
    let lower = message.to_ascii_lowercase();
    let Some(offset) = lower.find("unsupported url:") else {
      return false;
    };
    message[offset + "unsupported url:".len()..]
      .split_whitespace()
      .next()
      .and_then(|target| reqwest::Url::parse(target).ok())
      .is_some_and(|target| {
        matches!(target.scheme(), "http" | "https") && SiteOrHost::of(&target) != host
      })
  }

  fn parse_skip_line(&self, line: &str) -> Option<DiagnosticEvent> {
    let message = normalize_skip_message(line)?;

    Some(DiagnosticEvent {
      id: self.id.clone(),
      group_id: self.group_id.clone(),
      level: DiagnosticLevel::Warning,
      code: INPUT_FILTER_SKIPPED_CODE.to_string(),
      component: Some("download".to_string()),
      message,
      raw: redact_urls(line),
      timestamp: SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0),
    })
  }
}

/// A link's site (any of its domains, `twitter.com` is X), else its host without `www.`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SiteOrHost<'a> {
  Site(SiteId),
  Host(&'a str),
}

impl<'a> SiteOrHost<'a> {
  fn of(url: &'a reqwest::Url) -> Self {
    let host = url.host_str().unwrap_or_default();
    match site_by_domain(host) {
      Some(site) => Self::Site(site),
      None => Self::Host(host.strip_prefix("www.").unwrap_or(host)),
    }
  }
}

fn normalize_skip_message(line: &str) -> Option<String> {
  let trimmed = line.trim();
  let lower = trimmed.to_ascii_lowercase();

  if lower.contains("upload date is not in range") {
    return Some("Skipped by upload date filter".to_string());
  }

  if lower.contains("file is larger than max-filesize") {
    return Some("Skipped by maximum filesize filter".to_string());
  }

  if lower.contains("file is smaller than min-filesize") {
    return Some("Skipped by minimum filesize filter".to_string());
  }

  if lower.contains("does not pass filter") {
    if lower.contains("break-match") {
      return Some("Skipped by break-match filter".to_string());
    }
    return Some("Skipped by advanced input filter".to_string());
  }

  if lower.contains("maximum number of downloads reached")
    || lower.contains("max downloads reached")
  {
    return Some("Skipped by maximum downloads filter".to_string());
  }

  if lower.contains("skipping") && lower.contains("age limit") || lower.contains("age restricted") {
    return Some("Skipped by age limit filter".to_string());
  }

  if lower.contains("skipping") && lower.contains("filter") {
    return Some("Skipped by input filter".to_string());
  }

  if lower.contains(" is not in range ") {
    return Some("Skipped by input filter".to_string());
  }

  if !lower.starts_with("[download] ") && !lower.contains("skipping") {
    return None;
  }

  None
}

fn split_component(s: &'_ str) -> (Option<String>, Cow<'_, str>) {
  if let Some(rest) = s.strip_prefix('[') {
    // OS error prefixes are part of the message, not extractor component names.
    if rest.starts_with("Errno ") || rest.starts_with("WinError ") {
      return (None, Cow::Borrowed(s));
    }
    if let Some(end) = rest.find(']') {
      let comp = rest[..end].to_string();
      let msg = rest[end + 1..].trim();
      return (Some(comp), Cow::Borrowed(msg));
    }
  }
  (None, Cow::Borrowed(s))
}

fn split_video_id(s: &str, extractor: bool) -> (Option<String>, Cow<'_, str>) {
  let mut parts = s.splitn(2, ':');
  let first = parts.next().unwrap_or("").trim();
  let rest = parts.next();

  if let Some(msg) = rest {
    if is_probable_id(first)
      || (extractor
        && !first.is_empty()
        && first
          .bytes()
          .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')))
    {
      return (Some(first.to_string()), Cow::Borrowed(msg.trim()));
    }
  }

  (None, Cow::Borrowed(s.trim()))
}

fn is_probable_id(s: &str) -> bool {
  let len = s.len();
  (10..=15).contains(&len)
    && s
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parser() -> YtdlpErrorParser {
    YtdlpErrorParser::new(
      "item-id",
      "group-id",
      DiagnosticMatcher::from_json(r#"{"rules":[],"fallbackCode":"unknown"}"#).unwrap(),
    )
  }

  #[test]
  fn urls_and_extractor_ids_cannot_trigger_login_or_transient_classification() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for line in [
      "ERROR: Unsupported URL: https://evil.example/logged-in",
      "ERROR: [generic] abc: Failed to read https://evil.example/login_required",
      "ERROR: [generic] logged-in: Unable to extract data",
      "ERROR: [generic] Failed to read evil.example/login_required",
      "ERROR: [generic] Failed to read https://evil.example/rate-limit",
    ] {
      assert!(parser.login_reason(line).is_none(), "{line}");
      assert_eq!(
        parser.cookie_fetch_failure(line, true),
        CookieFetchFailure::Other,
        "{line}"
      );
    }
    assert!(parser
      .login_reason("ERROR: [generic] abc: Login required for https://example.com/public")
      .is_some());
  }

  #[test]
  fn fetch_diagnostic_redacts_urls_credentials_and_cookie_paths() {
    for (line, secret) in [
      ("ERROR: request failed at https://instagram.com/reel/private?token=secret", "instagram.com"),
      ("ERROR: Cookie: sessionid=secret; csrftoken=private", "sessionid=secret"),
      ("ERROR: Authorization: Bearer private", "Bearer private"),
      ("ERROR: sessionid=private", "sessionid=private"),
      ("ERROR: header {'Cookie': 'secret-value'}", "secret-value"),
      ("ERROR: [('ds_user_id', 'private-user'), ('mid', 'private-device')]", "private-device"),
      ("ERROR: Cookies: [ds_user_id=private-user, mid=private-device]", "private-device"),
      ("ERROR: ds_user_id=private-user; mid=private-device; csrftoken=private-token", "private-device"),
      ("ERROR: unavailable at instagram.com/reel/DfQIowKutL3/?token=private", "DfQIowKutL3"),
      ("ERROR: [Instagram] DfQIowKutL3: empty media response", "DfQIowKutL3"),
      ("ERROR: [Youtube] dQw4w9WgXcQ: Sign in required", "dQw4w9WgXcQ"),
      ("ERROR: copy failed [/Users/private-user/Cookies]", "private-user"),
      ("ERROR: copy failed, /private/user/Cookies", "/private/user"),
      (r"ERROR: copy failed \\server\private-user\Cookies", "private-user"),
      ("ERROR: could not copy cookie database /Users/person/Library/Application Support/Chrome/Default/Cookies", "/Users/person"),
      (r"ERROR: could not open C:\Users\person\Chrome\Cookies", r"C:\Users\person"),
    ] {
      let redacted = redact_diagnostic(line);
      assert!(redacted.starts_with("ERROR:"));
      assert!(!redacted.contains(secret), "{redacted}");
    }
  }

  #[test]
  fn cookie_fetch_classification_distinguishes_transient_responses_from_session_refusals() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for (stderr, expected) in [
      ("ERROR: [Instagram] Instagram sent an empty media response. Check if you are logged in. Use --cookies-from-browser", CookieFetchFailure::Transient),
      ("ERROR: [Instagram] Requested content is not available, rate-limit reached or login required. Use --cookies", CookieFetchFailure::Transient),
      ("ERROR: HTTP Error 429: Too Many Requests. Use --cookies", CookieFetchFailure::Transient),
      ("ERROR: [Instagram] login required. Use --cookies", CookieFetchFailure::Refused),
      ("ERROR: [Instagram] Session has expired. Use --cookies", CookieFetchFailure::Refused),
      ("ERROR: HTTP Error 503: Service Unavailable", CookieFetchFailure::Other),
      ("ERROR: Unable to extract data. Use --cookies", CookieFetchFailure::Other),
      ("WARNING: rate limit reached\nERROR: login required", CookieFetchFailure::Refused),
      ("ERROR: empty media response\nERROR: login required", CookieFetchFailure::Refused),
    ] {
      assert_eq!(parser.cookie_fetch_failure(stderr, true), expected, "{stderr}");
      assert_eq!(parser.cookie_fetch_failure(stderr, false), CookieFetchFailure::Other, "anonymous: {stderr}");
      let streamed = stderr.lines().fold(CookieFetchFailure::Other, |previous, line| previous.combine(parser.cookie_fetch_failure(line, true)));
      assert_eq!(streamed, expected, "streamed: {stderr}");
      if expected != CookieFetchFailure::Refused {
        for line in stderr.lines() {
          if let Some(event) = parser.parse_fetch_line(line, true) {
            assert_ne!(event.code, "signInRequired", "{stderr}");
          }
        }
      }
    }
    let refusal = "ERROR: [Instagram] This content isn't available to everyone: It can't be seen by certain audiences.";
    for (cookies, code) in [(false, "signInRequired"), (true, "accountAccessDenied")] {
      assert_eq!(
        parser.parse_fetch_line(refusal, cookies).unwrap().code,
        code
      );
    }
    // An empty response plus generic cookie advice does not prove a login requirement.
    assert!(parser
      .login_reason("ERROR: [Instagram] Instagram sent an empty media response. Use --cookies")
      .is_none());
  }

  #[test]
  fn photo_refusals_and_unsupported_links_are_not_invalid_links() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for (line, expected) in [
      (
        "ERROR: [twitter] 2116957135661945283: Media #1 is not a video",
        "photosOnly",
      ),
      (
        "ERROR: [twitter] 2116957135661945283: No video could be found in this tweet",
        "photosOnly",
      ),
      (
        "ERROR: Unsupported URL: https://example.com/post/1",
        "urlNotSupported",
      ),
      (
        "ERROR: [LinkedIn] 1234567890123456789: Unable to extract video; please report this issue",
        "imageSourceUnavailable",
      ),
      (
        "ERROR: [LinkedIn] Unable to extract video",
        "imageSourceUnavailable",
      ),
      (
        "ERROR: [LinkedIn] Unable to extract video ID",
        "unableToExtractIdOrInfo",
      ),
      (
        "ERROR: [OtherSite] 1234567890123456789: Unable to extract video",
        "unknown",
      ),
      ("ERROR: 'not-a-link' is not a valid URL", "urlUnsupported"),
    ] {
      assert_eq!(parser.parse_line(line).unwrap().code, expected, "{line}");
    }
  }

  #[test]
  fn recognizes_photo_urls_and_social_link_card_failures_from_source_context() {
    // Captured with yt-dlp 2026.08.19 on the owner's canonical X post, 2026-11-02.
    let stderr = "WARNING: [generic] Falling back on generic information extractor\n\
ERROR: Unsupported URL: https://hackertube.org/";
    for (source, expected) in [
      (
        "https://x.com/karpathy/status/2116957135661945283",
        "photosOnly",
      ),
      (
        "https://x.com/karpathy/status/2116957135661945283/photo/1",
        "photosOnly",
      ),
      ("https://www.instagram.com/p/BCISVm-GVr7/", "photosOnly"),
      (
        "https://pbs.twimg.com/media/photo.jpg?name=orig",
        "photosOnly",
      ),
      ("https://example.com/photo/1", "urlNotSupported"),
      (
        "https://x.com.example.org/karpathy/status/2116957135661945283",
        "urlNotSupported",
      ),
    ] {
      let parser = YtdlpErrorParser::new(
        "item",
        "group",
        DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
      )
      .with_source_url(source);
      let events = parser.parse_lines(stderr.lines().collect(), false);
      let event = events.last().unwrap();
      assert_eq!(event.code, expected, "{source}");
      assert_eq!(event.raw, "ERROR: Unsupported URL: [redacted URL]");
    }
  }

  #[test]
  fn site_refusals_map_to_honest_codes() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for (line, expected) in [
      ("ERROR: [vimeo] 1084537: Unable to download webpage: HTTP Error 401: Unauthorized (caused by <HTTPError 401: Unauthorized>)", "accessForbidden403"),
      ("ERROR: [generic] 1084537: Unable to download webpage: HTTP Error 401: Unauthorized", "unknown"),
      ("ERROR: [facebook] 123: Cannot parse data; please report this issue", "unableToExtractInitialData"),
      ("ERROR: [instagram:user] porsche: Unable to extract data; please report this issue", "unableToExtractInitialData"),
      ("ERROR: [likee] 123: Unable to extract video info; please report this issue", "unableToExtractIdOrInfo"),
      ("ERROR: Unable to extract data", "unknown"),
      ("ERROR: [generic] 123: Unable to extract data", "unknown"),
      ("ERROR: Unable to extract video info", "unknown"),
      ("ERROR: [generic] 123: Unable to extract video info", "unknown"),
      ("ERROR: No suitable extractor (BiliBili) found for URL http://www.bilibili.tv/video/av92494333/", "urlNotSupported"),
      ("ERROR: [generic] 123: Cannot parse data", "unknown"),
      (
        "ERROR: [TikTok] 7704010367441161524: Your IP address is blocked from accessing this post",
        "geoBlocked",
      ),
      (
        "ERROR: [Instagram] DfQUr1dMhUQ: This content isn't available to everyone: It can't be seen by certain audiences.",
        "signInRequired",
      ),
      ("ERROR: [youtube] Login details are needed to download this content", "signInRequired"),
      ("ERROR: [Instagram] This content is only available for registered users who follow this account", "signInRequired"),
      ("ERROR: [TikTok] TikTok is requiring login for access to this content", "signInRequired"),
      ("ERROR: [TikTok] This post may not be comfortable for some audiences. Log in for access", "signInRequired"),
      ("ERROR: [TikTok] You do not have permission to view this post. Log into an account that has access", "signInRequired"),
      ("ERROR: [twitter] You are not authorized to view this protected tweet", "signInRequired"),
      ("ERROR: [twitter] 1851261049: Error(s) while querying API: You are not authorized to view this content", "signInRequired"),
      ("ERROR: [Reddit] Quarantined subreddit; an account that has opted in is required", "signInRequired"),
      ("ERROR: [Reddit] Private subreddit; an account that has been approved is required", "signInRequired"),
      ("ERROR: [Reddit] Account authentication is required. Use --cookies", "signInRequired"),
      ("ERROR: HTTP Error 429: Too Many Requests. Use --cookies", "rateLimited429"),
      ("ERROR: [twitch] Your account does not have access to this subscriber-only content. Use --cookies", "membersOnly"),
      ("ERROR: [youtube] The uploader has not made this video available in your country", "geoBlocked"),
      ("ERROR: [Reddit] Your IP address is unable to access the Reddit API", "accessForbidden403"),
      ("ERROR: [vimeo] This request has been blocked due to its TLS fingerprint. Try compromising your security/cookies", "accessForbidden403"),
      ("ERROR: [Instagram] The webpage request was redirected to the login page. You have exceeded the rate-limit for accessing posts anonymously. Use --cookies", "rateLimited429"),
      ("ERROR: [Instagram] Instagram sent an empty media response. Check if you are logged-in. Use --cookies", "unableToExtractIdOrInfo"),
      ("ERROR: [Instagram] Requested content is not available, rate-limit reached or login required. Use --cookies", "unableToExtractIdOrInfo"),
      ("ERROR: [youtube] Incomplete data received", "unableToExtractIdOrInfo"),
      ("ERROR: [TikTok] TikTok API keeps sending the same page. Taking measures to avoid an infinite loop", "unableToExtractIdOrInfo"),
      ("ERROR: [TikTok] No working app info is available", "unableToExtractIdOrInfo"),
      ("ERROR: [TikTok] Unable to extract secondary user ID", "unableToExtractIdOrInfo"),
      ("ERROR: [youtube] All player responses are invalid. Your IP is likely being blocked by Youtube", "blockedGeneric"),
      ("ERROR: [youtube] Video unavailable. YouTube is requiring a captcha challenge before playback", "blockedGeneric"),
      ("ERROR: [Instagram] There is no video in this post", "photosOnly"),
      ("ERROR: [Bluesky] No video could be found in this post", "photosOnly"),
      ("ERROR: [twitter] Requested tweet is unavailable", "mediaUnavailable"),
      ("ERROR: [twitter] Video #2 is unavailable", "mediaUnavailable"),
      ("ERROR: [twitter] Broadcast no longer exists", "videoNotFoundOrPrivate"),
      ("ERROR: [twitter:spaces] Twitter Space not found", "videoNotFoundOrPrivate"),
      ("ERROR: [facebook] The video is not available, Facebook said: \"This content is no longer available\"", "mediaUnavailable"),
      ("ERROR: [twitch:clips] This clip is no longer available", "videoNotFoundOrPrivate"),
      ("ERROR: [vimeo] This event video is unavailable", "mediaUnavailable"),
      ("ERROR: [vimeo] This event has not been made available to anyone", "accessForbidden403"),
      ("ERROR: [twitch] Channel \"example\" not found", "channelUnavailable"),
      ("ERROR: [twitter:broadcast] This live broadcast has not yet started", "livestreamNotStarted"),
      ("ERROR: [twitter:spaces] Twitter Space not started yet", "livestreamNotStarted"),
      ("ERROR: [vimeo] This live event is scheduled for 2030-01-01T00:00:00Z", "livestreamNotStarted"),
      ("ERROR: [LinkedIn] This live event has not yet started", "livestreamNotStarted"),
      ("ERROR: [Reddit] This video is processing", "mediaNotReady"),
      ("ERROR: [twitter:spaces] Twitter Space ended but not downloadable yet", "mediaNotReady"),
      ("ERROR: [Reddit] No media found", "noMediaFound"),
      ("ERROR: [TikTok] This account does not have any videos posted", "noMediaFound"),
      ("ERROR: [TikTokLive] This livestream has ended", "noMediaFound"),
      ("ERROR: [twitter:spaces] Twitter Space ended and replay is disabled", "noMediaFound"),
      ("ERROR: [vimeo] This video is protected by a password, use the --video-password option", "videoPasswordRequired"),
      ("ERROR: [vimeo] Wrong video password", "videoPasswordRequired"),
      ("ERROR: [vimeo] Wrong password", "videoPasswordRequired"),
      ("ERROR: [vimeo:album] Wrong password", "videoPasswordRequired"),
      ("ERROR: [vimeo:ondemand] Wrong password", "videoPasswordRequired"),
      ("ERROR: [vimeo:review] Wrong password", "videoPasswordRequired"),
      ("ERROR: [vimeo:event] Wrong password", "videoPasswordRequired"),
      ("ERROR: [TikTok] Wrong password", "unknown"),
      ("ERROR: [Instagram] This content is unreachable", "signInRequired"),
      ("ERROR: [Instagram] Restricted Video", "signInRequired"),
      ("ERROR: HTTP Error 403: Forbidden. Use --cookies", "accessForbidden403"),
      ("ERROR: This video does not exist; login required", "signInRequired"),
    ] {
      assert_eq!(parser.parse_line(line).unwrap().code, expected, "{line}");
    }
  }

  #[test]
  fn photo_context_preserves_other_failures_and_does_not_guess_from_a_social_host_alone() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    )
    .with_source_url("https://x.com/karpathy/status/2116957135661945283/photo/1");
    for (line, expected) in [
      ("ERROR: [twitter] Sign in required", "signInRequired"),
      ("ERROR: HTTP Error 403: Forbidden", "accessForbidden403"),
      ("ERROR: getaddrinfo failed", "networkNameResolutionFailed"),
      ("ERROR: 'not-a-link' is not a valid URL", "urlNotSupported"),
      ("WARNING: Unsupported URL", "unknown"),
    ] {
      assert_eq!(parser.parse_line(line).unwrap().code, expected, "{line}");
    }
    let parser = parser.with_source_url("https://x.com/karpathy/status/2116957135661945283");
    assert_eq!(
      parser
        .parse_line(
          "ERROR: Unsupported URL: https://twitter.com/karpathy/status/2116957135661945283"
        )
        .unwrap()
        .code,
      "urlNotSupported"
    );
    let parser = parser.with_source_url("not-a-link");
    assert_eq!(
      parser
        .parse_line("ERROR: 'not-a-link' is not a valid URL")
        .unwrap()
        .code,
      "urlUnsupported"
    );
    // Pins can contain videos, so the URL alone cannot establish a photo post.
    let parser = parser.with_source_url("https://www.pinterest.com/pin/1084663891475263837/");
    assert_eq!(
      parser
        .parse_line("ERROR: Unsupported URL: https://www.pinterest.com/pin/1084663891475263837/")
        .unwrap()
        .code,
      "urlNotSupported"
    );
  }

  #[test]
  fn engine_refusal_rules_recognize_sites_without_treating_http_errors_as_logins() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for line in [
      "ERROR: [Reddit] 1wtjy4r: Account authentication is required. Use --cookies-from-browser",
      "ERROR: [vimeo] 13739299: The web client only works when logged-in",
      "ERROR: [vimeo] 76979871: The web client only works when logged-in. Use --cookies, --cookies-from-browser, --username and --password to provide account credentials.",
      "ERROR: [vimeo] This content is available to logged-in users",
      "ERROR: [Instagram] login_required",
      "ERROR: This video is age-restricted",
      "ERROR: [Instagram] DfQUr1dMhUQ: This content isn't available to everyone: It can't be seen by certain audiences.",
    ] {
      assert_eq!(
        parser.login_reason(line).as_deref(),
        Some("signInRequired"),
        "{line}"
      );
    }
    for line in [
      "ERROR: HTTP Error 403: Forbidden",
      "ERROR: HTTP Error 403: Forbidden. Use --cookies, --cookies-from-browser to provide account cookies",
      "ERROR: HTTP Error 429: Too Many Requests. Use --cookies-from-browser to provide account cookies",
      "ERROR: Use --cookies, --cookies-from-browser to provide account cookies",
      "ERROR: Use --cookies-from-browser to provide account cookies",
    ] {
      assert_eq!(parser.login_reason(line), None, "{line}");
    }
    assert_eq!(parser.login_reason("WARNING: login required"), None);
    assert_eq!(parser.login_reason("ERROR: Unsupported URL"), None);
  }

  #[test]
  fn timeouts_are_reportable_failures_but_offline_errors_are_excluded() {
    let parser = YtdlpErrorParser::new("item", "group", DiagnosticMatcher::shared().unwrap());
    for (line, code, offline) in [
      ("ERROR: Connection timed out", "networkTimeout", false),
      (
        "ERROR: The read operation timed out",
        "networkTimeout",
        false,
      ),
      ("ERROR: request timeout", "unknown", false),
      ("ERROR: HTTP Error 504: Gateway Timeout", "server5xx", false),
      (
        "ERROR: HTTP Error 504: connection timed out",
        "server5xx",
        false,
      ),
      (
        "ERROR: Network is unreachable",
        "networkNameResolutionFailed",
        true,
      ),
      (
        "ERROR: getaddrinfo failed",
        "networkNameResolutionFailed",
        true,
      ),
    ] {
      let event = parser.parse_fetch_line(line, false).unwrap();
      assert_eq!(event.code, code);
      assert_eq!(network_down(&event.code, &event.message), offline);
      assert_eq!(
        crate::diagnostics::send::worth_reporting(&event.code),
        !offline
      );
    }
  }

  #[test]
  fn storage_failures_have_actionable_codes() {
    let parser = YtdlpErrorParser::new("item", "group", DiagnosticMatcher::shared().unwrap());
    for (line, code) in [
      (
        "ERROR: unable to open for writing: [Errno 28] No space left on device",
        "diskFull",
      ),
      ("ERROR: [Errno 36] File name too long", "outputNameTooLong"),
      ("ERROR: [Errno 63] File name too long", "outputNameTooLong"),
      ("ERROR: No space left on device", "diskFull"),
      (
        "ERROR: [WinError 206] The filename or extension is too long",
        "outputNameTooLong",
      ),
      ("ERROR: [Errno 22] Invalid argument", "unknown"),
      (
        "ERROR: [Errno 22] The filename or extension is too long",
        "outputNameTooLong",
      ),
      (
        "ERROR: [WinError 206] Der Dateiname ist zu lang",
        "outputNameTooLong",
      ),
    ] {
      assert_eq!(parser.parse_line(line).unwrap().code, code, "{line}");
    }
  }

  #[test]
  fn audio_extraction_without_a_track_has_an_actionable_code() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    for line in [
      "ERROR: Postprocessing: unable to obtain file audio codec",
      "ERROR: [ExtractAudio] unable to obtain file audio codec with ffprobe",
    ] {
      assert_eq!(parser.parse_line(line).unwrap().code, "noAudioTrack");
    }
  }

  #[test]
  fn drm_protected_formats_say_so() {
    let parser = YtdlpErrorParser::new(
      "item",
      "group",
      DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
    );
    let event = parser
      .parse_line("ERROR: This format is DRM protected; Try selecting another format with --format or add --check-formats to automatically fallback to the next best format")
      .expect("diagnostic");
    assert_eq!(event.code, "drmProtected");
    let events = parser.parse_lines(
      vec![
        "[in#0] Unable to open key file skd://drm",
        "ERROR: ffmpeg exited with code 183",
      ],
      false,
    );
    let mut stage = crate::extract::chain::StageResult::<()>::new(
      crate::extract::outcome::StageId::Ytdlp,
      crate::extract::outcome::Confidence::High,
    );
    stage.record_diagnostics(&events, false, std::time::Instant::now());
    assert_eq!(
      crate::extract::precedence::final_code(&stage.attempts),
      Some("drmProtected")
    );
  }

  #[test]
  fn unknown_errors_keep_engine_details_without_urls() {
    let event = parser()
      .parse_line("ERROR: [generic] Unexpected response from https://user:secret@example.com/video?token=private (see https://example.com/help)")
      .unwrap();
    assert_eq!(event.code, "unknown");
    assert_eq!(
      event.message,
      "Unexpected response from [redacted URL] (see [redacted URL]"
    );
    assert_eq!(
      event.raw,
      "ERROR: [generic] Unexpected response from [redacted URL] (see [redacted URL]"
    );
  }

  #[test]
  fn parses_upload_date_skip_lines_as_warnings() {
    let event = parser()
      .parse_line("[download] 2022-01-05 upload date is not in range 2026-06-18 to 9999-12-31")
      .unwrap();

    assert!(matches!(event.level, DiagnosticLevel::Warning));
    assert_eq!(event.code, INPUT_FILTER_SKIPPED_CODE);
    assert_eq!(event.message, "Skipped by upload date filter");
    assert_eq!(event.component.as_deref(), Some("download"));
  }

  #[test]
  fn normalizes_advanced_filter_skip_lines() {
    let event = parser()
      .parse_line("[download] Video does not pass filter (!is_live), skipping ..")
      .unwrap();

    assert_eq!(event.message, "Skipped by advanced input filter");
  }

  #[test]
  fn normalizes_filesize_skip_lines() {
    let event = parser()
      .parse_line("[download] File is larger than max-filesize (123 > 100), skipping")
      .unwrap();

    assert_eq!(event.message, "Skipped by maximum filesize filter");
  }

  #[test]
  fn ignores_non_skip_download_lines() {
    for line in [
      "[download] Destination: /tmp/video.mp4",
      "[download] Destination: This video is DRM protected.mp4",
      "[info] This video is DRM protected",
    ] {
      assert!(parser().parse_line(line).is_none());
    }
  }

  #[test]
  fn normalizes_plain_break_match_filter_rejection_lines() {
    let event = parser()
      .parse_line("Some title does not pass filter (duration <? 1), skipping ..")
      .unwrap();

    assert_eq!(event.code, INPUT_FILTER_SKIPPED_CODE);
    assert_eq!(event.message, "Skipped by advanced input filter");
  }
  #[test]
  fn facebook_parse_wall_enters_login_only_for_anonymous_facebook_sources() {
    for (source, cookies, expected) in [
      ("https://facebook.com/watch?v=1", false, true),
      ("https://web.facebook.com/watch?v=1", false, true),
      ("https://facebook.com/share/p/abc/", false, true),
      (
        "https://facebook.com/story.php?story_fbid=123&id=42",
        false,
        true,
      ),
      (
        "https://facebook.com/permalink.php?story_fbid=123&id=42",
        false,
        true,
      ),
      ("https://m.facebook.com/watch?v=1", true, false),
      ("https://facebook.com.evil.example/watch?v=1", false, false),
      ("https://instagram.com/p/example/", false, false),
    ] {
      let parser = YtdlpErrorParser::new(
        "id",
        "group",
        DiagnosticMatcher::from_json(include_str!("../diagnostic_rules.json")).unwrap(),
      )
      .with_source_url(source);
      let line = "ERROR: [facebook] 123: Cannot parse data";
      assert_eq!(
        parser.login_reason_with_cookies(line, cookies).is_some(),
        expected
      );
      if expected {
        assert_eq!(
          parser.parse_fetch_line(line, cookies).unwrap().code,
          "signInRequired"
        );
      } else {
        assert_eq!(
          parser.parse_fetch_line(line, cookies).unwrap().code,
          "unableToExtractInitialData"
        );
      }
      for other in [
        "ERROR: [generic] Unable to extract data",
        "ERROR: [instagram:user] nasa: Unable to extract data",
        "ERROR: [facebook] 123: Unable to extract data",
      ] {
        assert!(parser.login_reason_with_cookies(other, cookies).is_none());
      }
    }
  }

  #[test]
  fn instagram_profile_parse_failure_suggests_login_only_for_anonymous_profiles() {
    let line = "ERROR: [instagram:user] porsche: Unable to extract data";
    for (url, cookies, expected) in [
      (
        "https://www.instagram.com/porsche/",
        false,
        "signInRequired",
      ),
      (
        "https://www.instagram.com/porsche/",
        true,
        "unableToExtractInitialData",
      ),
      (
        "https://www.instagram.com/p/example/",
        false,
        "unableToExtractInitialData",
      ),
      (
        "https://instagram.com.evil.test/porsche/",
        false,
        "unableToExtractInitialData",
      ),
    ] {
      let parser = YtdlpErrorParser::new("id", "group", DiagnosticMatcher::shared().unwrap())
        .with_source_url(url);
      assert_eq!(
        parser.parse_fetch_line(line, cookies).unwrap().code,
        expected
      );
      assert_eq!(
        parser.login_reason_with_cookies(line, cookies).is_some(),
        expected == "signInRequired"
      );
      assert_eq!(
        parser
          .parse_fetch_line(
            "ERROR: [instagram:user] HTTP Error 429: Too Many Requests",
            cookies
          )
          .unwrap()
          .code,
        "rateLimited429"
      );
    }
  }
}
