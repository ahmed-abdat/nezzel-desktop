//! Stable, privacy-safe vocabulary shared by extraction stages.

pub use crate::routing::{Confidence, ReaderId, StageId};

/// Auth provenance records retries without retaining browser or profile identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthUsed {
  Anonymous,
  Browser,
}

/// Network scope is supplied by the caller that knows which endpoint failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkScope {
  PageHost,
  StageLocal,
}

/// Typed causes allow exhaustive stop rules independent of diagnostic wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
  Verified,
  NoMatch,
  Unknown,
  Unsupported,
  ExtractFailed,
  Login,
  AgeRestricted,
  Paid,
  Private,
  Removed,
  Geo,
  Drm,
  LiveNotStarted,
  RateLimited,
  Blocked,
  Forbidden,
  Network(NetworkScope),
  KindMismatch,
  NoMedia,
  Cancelled,
  LoginPending,
}

/// A validated code prevents attempts from accidentally storing arbitrary engine text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticCode(&'static str);

impl DiagnosticCode {
  /// Exposes only the fixed runner vocabulary for eventual event serialization.
  pub fn as_str(self) -> &'static str {
    self.0
  }
}

/// Classification retains the precise code when multiple codes share a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic {
  pub outcome: Outcome,
  pub code: DiagnosticCode,
}

/// Normalize existing diagnostics; age wording must outrank the old generic login rule.
/// The scope is explicit because a diagnostic code alone cannot identify the failed host.
/// Pass the parser's redacted message so URLs cannot masquerade as age or WAF wording.
pub fn classify(code: &str, message: &str, scope: NetworkScope) -> Diagnostic {
  let lower = message.to_ascii_lowercase();
  if matches!(code, "signInRequired" | "ageRestricted")
    && (lower.contains("sign in to confirm your age") || lower.contains("age-restricted"))
  {
    return Diagnostic {
      outcome: Outcome::AgeRestricted,
      code: DiagnosticCode("ageRestricted"),
    };
  }
  let (outcome, code) = match code {
    "networkTimeout" => (Outcome::Network(scope), "networkTimeout"),
    "fetchTimeout" => (Outcome::Network(scope), "fetchTimeout"),
    "fileNetwork" => (Outcome::Network(scope), "fileNetwork"),
    "fileProxy" => (Outcome::Network(scope), "fileProxy"),
    "fileNotPublic" => (Outcome::Forbidden, "fileNotPublic"),
    "fileRateLimited" => (Outcome::RateLimited, "fileRateLimited"),
    "fileNotFile" => (Outcome::Unsupported, "fileNotFile"),
    "browserDpapiFailed" => (Outcome::Unknown, "browserDpapiFailed"),
    "browserAppBound" => (Outcome::Unknown, "browserAppBound"),
    "browserCookiesUnreadable" => (Outcome::Unknown, "browserCookiesUnreadable"),
    "browserKeychainDenied" => (Outcome::Unknown, "browserKeychainDenied"),
    "browserSafariAccessDenied" => (Outcome::Unknown, "browserSafariAccessDenied"),
    "browserDatabaseLocked" => (Outcome::Unknown, "browserDatabaseLocked"),
    "browserProfileMissing" => (Outcome::Unknown, "browserProfileMissing"),
    "diskFull" => (Outcome::Unknown, "diskFull"),
    "outputNameTooLong" => (Outcome::Unknown, "outputNameTooLong"),
    "fileState" => (Outcome::Unknown, "fileState"),
    "fileAddressRefused" => (Outcome::Forbidden, "fileAddressRefused"),
    "fileEncoding" => (Outcome::Unknown, "fileEncoding"),
    "fileEmpty" => (Outcome::NoMedia, "fileEmpty"),
    "drmProtected" => (Outcome::Drm, "drmProtected"),
    "notFound404" => (Outcome::Removed, "notFound404"),
    "copyrightClaimed" => (Outcome::Removed, "copyrightClaimed"),
    "channelUnavailable" => (Outcome::Removed, "channelUnavailable"),
    "videoNotFoundOrPrivate" => (Outcome::Private, "videoNotFoundOrPrivate"),
    "playlistPrivateOrMissing" => (Outcome::Private, "playlistPrivateOrMissing"),
    "geoBlocked" => (Outcome::Geo, "geoBlocked"),
    "livestreamNotStarted" => (Outcome::LiveNotStarted, "livestreamNotStarted"),
    "notPremieredYet" => (Outcome::LiveNotStarted, "notPremieredYet"),
    "membersOnly" => (Outcome::Paid, "membersOnly"),
    "accountAccessDenied" => (Outcome::Private, "accountAccessDenied"),
    "signInRequired" => (Outcome::Login, "signInRequired"),
    "signInRequiredForBotDetection" => (Outcome::Login, "signInRequiredForBotDetection"),
    "ageRestricted" => (Outcome::AgeRestricted, "ageRestricted"),
    "rateLimited429" => (Outcome::RateLimited, "rateLimited429"),
    "accessForbidden403" => {
      let waf = ["cloudflare", "blocked by network security", "waf challenge"]
        .iter()
        .any(|marker| lower.contains(marker));
      (
        if waf {
          Outcome::Blocked
        } else {
          Outcome::Forbidden
        },
        "accessForbidden403",
      )
    }
    // Plan rank 7: a generic block belongs with forbidden/WAF responses.
    "blockedGeneric" => (Outcome::Blocked, "blockedGeneric"),
    "photosOnly" => (Outcome::KindMismatch, "photosOnly"),
    "unableToExtractInitialData" => (Outcome::ExtractFailed, "unableToExtractInitialData"),
    "unableToExtractIdOrInfo" => (Outcome::ExtractFailed, "unableToExtractIdOrInfo"),
    "nsigExtractionFailed" => (Outcome::ExtractFailed, "nsigExtractionFailed"),
    "server5xx" => (Outcome::ExtractFailed, "server5xx"),
    "siteChanged" => (Outcome::ExtractFailed, "siteChanged"),
    // Plan rank 10: missing formats are an extraction failure, not unsupported input.
    "requestedFormatUnavailable" => (Outcome::ExtractFailed, "requestedFormatUnavailable"),
    // Plan rank 10: a missing embedding context makes extraction inconclusive.
    "embedOnly" => (Outcome::ExtractFailed, "embedOnly"),
    "videoPasswordRequired" => (Outcome::Forbidden, "videoPasswordRequired"),
    // Processing media needs the same wait-and-retry policy as a scheduled live event.
    "mediaNotReady" => (Outcome::LiveNotStarted, "mediaNotReady"),
    "mediaUnavailable" => (Outcome::Unknown, "mediaUnavailable"),
    "facebookVideoUnavailable" => (Outcome::Forbidden, "facebookVideoUnavailable"),
    "imageEngineUnavailable" => (Outcome::ExtractFailed, "imageEngineUnavailable"),
    "imageSourceUnavailable" => (Outcome::NoMedia, "imageSourceUnavailable"),
    "noMediaFound" => (Outcome::NoMedia, "noMediaFound"),
    "urlNotSupported" => (Outcome::Unsupported, "urlNotSupported"),
    "urlUnsupported" => (Outcome::Unsupported, "urlUnsupported"),
    "unknownUrlType" => (Outcome::Unsupported, "unknownUrlType"),
    "networkNameResolutionFailed" => (Outcome::Network(scope), "networkNameResolutionFailed"),
    "sslVerifyFailed" => (Outcome::Network(scope), "sslVerifyFailed"),
    "unableToConnectToProxy" => (Outcome::Network(scope), "unableToConnectToProxy"),
    // Plan rank 8: this API failure is stage-local and counts only in an all-network row.
    "sponsorBlockUnreachable" => (
      Outcome::Network(NetworkScope::StageLocal),
      "sponsorBlockUnreachable",
    ),
    _ => (Outcome::Unknown, "unknown"),
  };
  Diagnostic {
    outcome,
    code: DiagnosticCode(code),
  }
}

/// Codes a login can fix: the queue row offers to sign in and the capture card says `login`.
pub const LOGIN_CODES: [&str; 4] = [
  "signInRequired",
  "signInRequiredForBotDetection",
  "membersOnly",
  "videoPasswordRequired",
];

/// Codes whose failure the site decides, so retrying the same link fails the same way (the webview's
/// `isPermanentError`, rendered by `crate::contract`): the codes `classify` puts under Unsupported, Removed,
/// Private, Geo, Drm, Paid, KindMismatch or NoMedia, except `imageSourceUnavailable`, whose message tells the
/// user to try again.
#[cfg_attr(not(test), allow(dead_code))]
pub const PERMANENT_CODES: [&str; 15] = [
  // Unsupported
  "urlNotSupported",
  "urlUnsupported",
  "unknownUrlType",
  "fileNotFile",
  // Removed / Private
  "notFound404",
  "copyrightClaimed",
  "channelUnavailable",
  "videoNotFoundOrPrivate",
  "playlistPrivateOrMissing",
  // Geo, Drm, Paid
  "geoBlocked",
  "drmProtected",
  "membersOnly",
  // KindMismatch, NoMedia
  "photosOnly",
  "noMediaFound",
  "fileEmpty",
];

/// An attempt contains only enum values, a validated code and elapsed milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
  pub stage: StageId,
  pub auth: AuthUsed,
  pub confidence: Confidence,
  pub outcome: Outcome,
  pub code: Option<DiagnosticCode>,
  pub ms: u32,
}

impl Attempt {
  /// Build a failed attempt without copying raw text into the chain report.
  pub fn failed(
    stage: StageId,
    auth: AuthUsed,
    confidence: Confidence,
    diagnostic: Diagnostic,
    ms: u32,
  ) -> Self {
    Self {
      stage,
      auth,
      confidence,
      outcome: diagnostic.outcome,
      code: Some(diagnostic.code),
      ms,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn permanent_codes_are_exactly_the_site_decided_outcomes() {
    let permanent = |code: &str| {
      matches!(
        classify(code, "", NetworkScope::PageHost).outcome,
        Outcome::Unsupported
          | Outcome::Removed
          | Outcome::Private
          | Outcome::Geo
          | Outcome::Drm
          | Outcome::Paid
          | Outcome::KindMismatch
          | Outcome::NoMedia
      ) && code != "imageSourceUnavailable"
    };
    let rules: serde_json::Value =
      serde_json::from_str(include_str!("../diagnostic_rules.json")).unwrap();
    let mut known: Vec<&str> = rules["rules"]
      .as_array()
      .unwrap()
      .iter()
      .filter_map(|rule| rule["code"].as_str())
      .collect();
    known.extend(PERMANENT_CODES);
    known.extend(LOGIN_CODES);
    for code in known {
      assert_eq!(permanent(code), PERMANENT_CODES.contains(&code), "{code}");
    }
  }

  #[test]
  fn age_gate_wording_is_classified_before_generic_login() {
    for text in [
      "Sign in to confirm your age. Use --cookies",
      "This video is age-restricted",
    ] {
      let actual = classify("signInRequired", text, NetworkScope::PageHost);
      assert_eq!(
        (actual.outcome, actual.code.as_str()),
        (Outcome::AgeRestricted, "ageRestricted"),
        "{text}"
      );
    }
  }

  #[test]
  fn diagnostic_codes_map_to_their_extraction_causes() {
    for (code, expected) in [
      ("drmProtected", Outcome::Drm),
      ("notFound404", Outcome::Removed),
      ("copyrightClaimed", Outcome::Removed),
      ("channelUnavailable", Outcome::Removed),
      ("videoNotFoundOrPrivate", Outcome::Private),
      ("playlistPrivateOrMissing", Outcome::Private),
      ("geoBlocked", Outcome::Geo),
      ("livestreamNotStarted", Outcome::LiveNotStarted),
      ("notPremieredYet", Outcome::LiveNotStarted),
      ("membersOnly", Outcome::Paid),
      ("signInRequired", Outcome::Login),
      ("signInRequiredForBotDetection", Outcome::Login),
      ("ageRestricted", Outcome::AgeRestricted),
      ("rateLimited429", Outcome::RateLimited),
      ("accessForbidden403", Outcome::Forbidden),
      ("blockedGeneric", Outcome::Blocked),
      ("photosOnly", Outcome::KindMismatch),
      ("unableToExtractInitialData", Outcome::ExtractFailed),
      ("unableToExtractIdOrInfo", Outcome::ExtractFailed),
      ("nsigExtractionFailed", Outcome::ExtractFailed),
      ("server5xx", Outcome::ExtractFailed),
      ("siteChanged", Outcome::ExtractFailed),
      ("requestedFormatUnavailable", Outcome::ExtractFailed),
      ("embedOnly", Outcome::ExtractFailed),
      ("videoPasswordRequired", Outcome::Forbidden),
      ("mediaNotReady", Outcome::LiveNotStarted),
      ("mediaUnavailable", Outcome::Unknown),
      ("imageEngineUnavailable", Outcome::ExtractFailed),
      ("imageSourceUnavailable", Outcome::NoMedia),
      ("noMediaFound", Outcome::NoMedia),
      ("urlNotSupported", Outcome::Unsupported),
      ("urlUnsupported", Outcome::Unsupported),
      ("unknownUrlType", Outcome::Unsupported),
      (
        "networkNameResolutionFailed",
        Outcome::Network(NetworkScope::PageHost),
      ),
      ("sslVerifyFailed", Outcome::Network(NetworkScope::PageHost)),
      (
        "unableToConnectToProxy",
        Outcome::Network(NetworkScope::PageHost),
      ),
      (
        "sponsorBlockUnreachable",
        Outcome::Network(NetworkScope::StageLocal),
      ),
      ("unknown", Outcome::Unknown),
    ] {
      let actual = classify(code, "", NetworkScope::PageHost);
      assert_eq!(
        (actual.outcome, actual.code.as_str()),
        (expected, code),
        "{code}"
      );
    }
  }

  #[test]
  fn waf_challenges_use_blocked_policy_without_inventing_a_diagnostic_code() {
    for message in [
      "Cloudflare challenge: HTTP Error 403",
      "blocked by network security",
      "WAF challenge",
    ] {
      let diagnostic = classify("accessForbidden403", message, NetworkScope::StageLocal);
      assert_eq!(
        (diagnostic.outcome, diagnostic.code.as_str()),
        (Outcome::Blocked, "accessForbidden403"),
        "{message}"
      );
    }
  }

  #[test]
  fn unrecognized_and_download_only_codes_cannot_leak_into_attempts() {
    for code in [
      "https://private.example/token",
      "ffmpegFailed",
      "permissionDenied",
      "downloadRetryingFragment",
    ] {
      assert_eq!(
        classify(code, "", NetworkScope::PageHost).code.as_str(),
        "unknown"
      );
    }
  }

  #[test]
  fn endpoint_context_distinguishes_page_network_from_api_network() {
    assert_eq!(
      classify("networkNameResolutionFailed", "", NetworkScope::StageLocal).outcome,
      Outcome::Network(NetworkScope::StageLocal)
    );
  }
}
