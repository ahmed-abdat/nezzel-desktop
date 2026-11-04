//! Select the cause rather than the last engine's symptom.
use super::outcome::*;

// Plan §3 numbers are preserved; the second component places rank 3b after 3.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rank(u8, u8);

impl Rank {
  const CONTENT_PROTECTED: Self = Self(1, 0);
  const CONTENT_GONE: Self = Self(2, 0);
  const REGION: Self = Self(3, 0);
  const NOT_YET_AVAILABLE: Self = Self(3, 1);
  const ACCOUNT_GATED: Self = Self(4, 0);
  const LOGIN: Self = Self(5, 0);
  const THROTTLED: Self = Self(6, 0);
  const FORBIDDEN: Self = Self(7, 0);
  const NETWORK: Self = Self(8, 0);
  const KIND_MISMATCH: Self = Self(9, 0);
  const SITE_CHANGED: Self = Self(10, 0);
  const SAVED_NOTHING: Self = Self(11, 0);
  const UNSUPPORTED: Self = Self(12, 0);
  const TIMEOUT: Self = Self(13, 0);
  const FALLBACK: Self = Self(14, 0);
}

/// Choose one final error; success and silent cancellation have no error code.
pub fn final_code(attempts: &[Attempt]) -> Option<&'static str> {
  let last = attempts.last()?;
  if attempts
    .iter()
    .any(|attempt| attempt.outcome == Outcome::Verified)
    || matches!(last.outcome, Outcome::Cancelled | Outcome::LoginPending)
  {
    return None;
  }
  let all_network = attempts
    .iter()
    .filter(|attempt| attempt.outcome != Outcome::NoMatch && !is_timeout(attempt))
    .all(|attempt| matches!(attempt.outcome, Outcome::Network(_)));
  let all_unsupported = attempts
    .iter()
    .filter(|attempt| !is_timeout(attempt))
    .all(|attempt| matches!(attempt.outcome, Outcome::NoMatch | Outcome::Unsupported));
  let mut best: Option<(Rank, Confidence, &'static str)> = None;
  let authenticated_terminal = attempts.iter().any(|attempt| {
    attempt.auth != AuthUsed::Anonymous
      && (attempt.outcome == Outcome::NoMedia
        || (attempt.outcome == Outcome::Removed && scoped_content_rank(attempt).is_some()))
  });
  for attempt in attempts {
    if authenticated_terminal
      && matches!(
        attempt.outcome,
        Outcome::Login | Outcome::AgeRestricted | Outcome::Paid | Outcome::Private
      )
    {
      continue;
    }
    let site_changed = attempt.confidence == Confidence::High
      && matches!(
        (attempt.stage, attempt.outcome),
        (StageId::Reader(_), Outcome::Unknown)
          | (StageId::Ytdlp | StageId::Gallery, Outcome::ExtractFailed)
      );
    let rank = if is_timeout(attempt) {
      Some(Rank::TIMEOUT)
    } else {
      match attempt.outcome {
        Outcome::Drm => Some(Rank::CONTENT_PROTECTED),
        Outcome::Removed | Outcome::Private => scoped_content_rank(attempt),
        Outcome::Geo => Some(Rank::REGION),
        Outcome::LiveNotStarted => Some(Rank::NOT_YET_AVAILABLE),
        Outcome::Paid => Some(Rank::ACCOUNT_GATED),
        Outcome::Login | Outcome::AgeRestricted => Some(Rank::LOGIN),
        Outcome::RateLimited => Some(Rank::THROTTLED),
        Outcome::Blocked | Outcome::Forbidden => Some(Rank::FORBIDDEN),
        Outcome::Network(_) => all_network.then_some(Rank::NETWORK),
        Outcome::KindMismatch => Some(Rank::KIND_MISMATCH),
        Outcome::ExtractFailed => Some(Rank::SITE_CHANGED),
        Outcome::NoMedia => Some(Rank::SAVED_NOTHING),
        Outcome::Unsupported => all_unsupported.then_some(Rank::UNSUPPORTED),
        Outcome::Unknown => Some(if site_changed {
          Rank::SITE_CHANGED
        } else {
          Rank::FALLBACK
        }),
        Outcome::Verified | Outcome::NoMatch | Outcome::Cancelled | Outcome::LoginPending => None,
      }
    };
    let code = if site_changed {
      Some("siteChanged")
    } else {
      attempt.code.map(DiagnosticCode::as_str)
    };
    if let (Some(rank), Some(code)) = (rank, code) {
      let replace = match best {
        None => true,
        Some((best_rank, confidence, _)) => {
          rank < best_rank || (rank == best_rank && attempt.confidence > confidence)
        }
      };
      if replace {
        best = Some((rank, attempt.confidence, code));
      }
    }
  }
  Some(best.map_or(
    if all_unsupported {
      "urlNotSupported"
    } else {
      "unknown"
    },
    |(_, _, code)| code,
  ))
}

fn is_timeout(attempt: &Attempt) -> bool {
  attempt
    .code
    .is_some_and(|code| code.as_str() == "fetchTimeout")
}

fn scoped_content_rank(attempt: &Attempt) -> Option<Rank> {
  (attempt.confidence >= Confidence::Medium && attempt.stage != StageId::PageScan)
    .then_some(Rank::CONTENT_GONE)
}

#[cfg(test)]
mod tests {
  use super::*;
  fn failed(code: &str) -> Attempt {
    Attempt::failed(
      StageId::Ytdlp,
      AuthUsed::Anonymous,
      Confidence::High,
      classify(code, "", NetworkScope::PageHost),
      0,
    )
  }

  #[test]
  fn every_colliding_precedence_rank_preserves_the_more_informative_cause() {
    // All rank pairs, in both execution orders: order cannot overwrite a better cause.
    let ranked = [
      "drmProtected",
      "notFound404",
      "geoBlocked",
      "livestreamNotStarted",
      "membersOnly",
      "signInRequired",
      "rateLimited429",
      "accessForbidden403",
      "photosOnly",
      "siteChanged",
      "noMediaFound",
      "fetchTimeout",
      "unknown",
    ];
    for (i, better) in ranked.iter().enumerate() {
      for worse in &ranked[i + 1..] {
        for pair in [
          [failed(better), failed(worse)],
          [failed(worse), failed(better)],
        ] {
          assert_eq!(final_code(&pair), Some(*better), "{better} vs {worse}");
        }
      }
    }
  }

  #[test]
  fn logged_in_terminal_evidence_replaces_anonymous_account_gates() {
    for terminal in ["noMediaFound", "notFound404"] {
      let mut browser = failed(terminal);
      browser.auth = AuthUsed::Browser;
      for gate in [
        "signInRequired",
        "ageRestricted",
        "membersOnly",
        "videoNotFoundOrPrivate",
      ] {
        assert_eq!(final_code(&[failed(gate), browser]), Some(terminal));
      }
    }
  }

  #[test]
  fn reddit_login_survives_a_later_unsupported_browser_attempt() {
    let mut browser = failed("urlNotSupported");
    browser.auth = AuthUsed::Browser;
    assert_eq!(
      final_code(&[failed("signInRequired"), browser]),
      Some("signInRequired")
    );
  }

  #[test]
  fn network_is_selected_when_every_matching_attempt_is_network() {
    for (codes, expected) in [
      (
        vec!["networkNameResolutionFailed", "sslVerifyFailed"],
        "networkNameResolutionFailed",
      ),
      (
        vec!["networkNameResolutionFailed", "photosOnly"],
        "photosOnly",
      ),
      (
        vec!["networkNameResolutionFailed", "urlNotSupported"],
        "unknown",
      ),
      (vec!["networkNameResolutionFailed", "unknown"], "unknown"),
      (vec!["fetchTimeout"], "fetchTimeout"),
      (vec!["fetchTimeout", "unknown"], "fetchTimeout"),
      (vec!["fetchTimeout", "sslVerifyFailed"], "sslVerifyFailed"),
      (vec!["fetchTimeout", "urlNotSupported"], "urlNotSupported"),
    ] {
      let attempts: Vec<_> = codes.iter().map(|code| failed(code)).collect();
      assert_eq!(final_code(&attempts), Some(expected));
    }
    let mut no_match = failed("unknown");
    no_match.outcome = Outcome::NoMatch;
    no_match.code = None;
    assert_eq!(
      final_code(&[failed("sslVerifyFailed"), no_match]),
      Some("sslVerifyFailed")
    );
  }

  #[test]
  fn unsupported_requires_only_no_match_or_unsupported_evidence() {
    let mut no_match = failed("unknown");
    no_match.outcome = Outcome::NoMatch;
    no_match.code = None;
    assert_eq!(
      final_code(&[no_match, failed("urlNotSupported")]),
      Some("urlNotSupported")
    );
    assert_eq!(
      final_code(&[failed("urlNotSupported"), failed("unknown")]),
      Some("unknown")
    );
    assert_eq!(final_code(&[no_match]), Some("urlNotSupported"));
  }

  #[test]
  fn high_confidence_reader_or_engine_failures_report_site_changed() {
    for (stage, confidence, code, expected) in [
      (
        StageId::Reader(ReaderId::Reddit),
        Confidence::High,
        "unknown",
        "siteChanged",
      ),
      (
        StageId::Reader(ReaderId::Reddit),
        Confidence::Medium,
        "unknown",
        "unknown",
      ),
      (StageId::PageScan, Confidence::High, "unknown", "unknown"),
      (
        StageId::Ytdlp,
        Confidence::High,
        "unableToExtractInitialData",
        "siteChanged",
      ),
      (
        StageId::Gallery,
        Confidence::High,
        "unableToExtractIdOrInfo",
        "siteChanged",
      ),
      (
        StageId::Ytdlp,
        Confidence::Medium,
        "unableToExtractInitialData",
        "unableToExtractInitialData",
      ),
    ] {
      let mut attempt = failed(code);
      attempt.stage = stage;
      attempt.confidence = confidence;
      assert_eq!(
        final_code(&[attempt, failed("urlNotSupported")]),
        Some(expected),
        "{stage:?} {confidence:?} {code}"
      );
    }
    let mut reader = failed("unknown");
    reader.stage = StageId::Reader(ReaderId::Reddit);
    assert_eq!(
      final_code(&[failed("noMediaFound"), reader]),
      Some("siteChanged")
    );
    assert_eq!(
      final_code(&[reader, failed("photosOnly")]),
      Some("photosOnly")
    );
  }

  #[test]
  fn content_gone_requires_a_medium_or_high_non_generic_claim() {
    for code in [
      "notFound404",
      "videoNotFoundOrPrivate",
      "playlistPrivateOrMissing",
      "copyrightClaimed",
      "channelUnavailable",
    ] {
      for confidence in [
        Confidence::None,
        Confidence::Low,
        Confidence::Medium,
        Confidence::High,
      ] {
        for stage in [StageId::Ytdlp, StageId::PageScan] {
          let mut attempt = failed(code);
          attempt.confidence = confidence;
          attempt.stage = stage;
          let expected = if confidence >= Confidence::Medium && stage != StageId::PageScan {
            code
          } else {
            "signInRequired"
          };
          assert_eq!(
            final_code(&[attempt, failed("signInRequired")]),
            Some(expected),
            "{code} {confidence:?} {stage:?}"
          );
        }
      }
    }
  }

  #[test]
  fn ties_prefer_confidence_then_earlier_attempt_including_age_and_live_codes() {
    for (earlier, later) in [
      ("signInRequired", "ageRestricted"),
      ("livestreamNotStarted", "notPremieredYet"),
    ] {
      let mut low = failed(earlier);
      low.confidence = Confidence::Low;
      assert_eq!(final_code(&[low, failed(later)]), Some(later));
      assert_eq!(final_code(&[failed(earlier), failed(later)]), Some(earlier));
    }
  }

  #[test]
  fn verified_gallery_suppresses_photos_only_and_cancelled_is_silent() {
    for outcome in [Outcome::Verified, Outcome::Cancelled, Outcome::LoginPending] {
      let mut attempt = failed("unknown");
      attempt.stage = StageId::Gallery;
      attempt.outcome = outcome;
      attempt.code = None;
      assert_eq!(final_code(&[failed("photosOnly"), attempt]), None);
    }
    assert_eq!(final_code(&[]), None);
  }
}
