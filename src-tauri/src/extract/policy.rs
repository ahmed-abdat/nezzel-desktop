//! Pure stop rules; the coordinator owns timers, transport and consent I/O.
use super::outcome::*;

/// A resumed consent request starts in Login, preserving the anonymous history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
  Anonymous,
  Login,
}

/// Claims carry exclusivity so YouTube cannot fall through to generic extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claim {
  pub confidence: Confidence,
  pub exclusive: bool,
}

/// Retry state bounds stage-local throttling to one retry per stage invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyContext {
  pub phase: Phase,
  pub stage: StageId,
  pub claim: Claim,
  pub retried: bool,
  pub known_login_host: bool,
}

/// Continue records whether login may be useful after anonymous stages are exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
  Continue { login_wanted: bool },
  Stop,
  Backoff,
  Suspend,
}

/// Decide without I/O; the coordinator keeps exclusive authentication on the same stage.
pub fn next(outcome: Outcome, ctx: PolicyContext) -> Decision {
  let login_wanted = ctx.phase == Phase::Anonymous;
  let decision = match outcome {
    Outcome::Verified
    | Outcome::Geo
    | Outcome::Drm
    | Outcome::LiveNotStarted
    | Outcome::Cancelled
    | Outcome::Network(NetworkScope::PageHost) => Decision::Stop,
    Outcome::LoginPending => Decision::Suspend,
    Outcome::NoMatch
    | Outcome::Unknown
    | Outcome::Unsupported
    | Outcome::ExtractFailed
    | Outcome::Forbidden
    | Outcome::Network(NetworkScope::StageLocal)
    | Outcome::KindMismatch
    | Outcome::NoMedia => Decision::Continue {
      login_wanted: false,
    },
    Outcome::Login | Outcome::AgeRestricted | Outcome::Paid => Decision::Continue { login_wanted },
    Outcome::Private => Decision::Continue {
      login_wanted: login_wanted && ctx.known_login_host,
    },
    Outcome::Removed => {
      if ctx.claim.confidence >= Confidence::Medium && ctx.stage != StageId::PageScan {
        Decision::Stop
      } else {
        Decision::Continue {
          login_wanted: false,
        }
      }
    }
    Outcome::RateLimited | Outcome::Blocked => {
      if ctx.retried {
        Decision::Continue {
          login_wanted: false,
        }
      } else {
        Decision::Backoff
      }
    }
  };
  if ctx.claim.exclusive && matches!(decision, Decision::Continue { .. }) {
    Decision::Stop
  } else {
    decision
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  fn context(stage: StageId, phase: Phase, confidence: Confidence) -> PolicyContext {
    PolicyContext {
      phase,
      stage,
      claim: Claim {
        confidence,
        exclusive: false,
      },
      retried: false,
      known_login_host: true,
    }
  }

  #[test]
  fn stop_table_applies_to_each_outcome_phase_and_stage_kind() {
    let stages = [
      StageId::Files,
      StageId::Reader(ReaderId::Reddit),
      StageId::Ytdlp,
      StageId::Gallery,
      StageId::PageScan,
    ];
    let cont = Decision::Continue {
      login_wanted: false,
    };
    for stage in stages {
      for phase in [Phase::Anonymous, Phase::Login] {
        for (outcome, expected) in [
          (Outcome::Verified, Decision::Stop),
          (Outcome::NoMatch, cont),
          (Outcome::Unknown, cont),
          (Outcome::Unsupported, cont),
          (Outcome::ExtractFailed, cont),
          (Outcome::KindMismatch, cont),
          (Outcome::NoMedia, cont),
          (Outcome::Geo, Decision::Stop),
          (Outcome::Drm, Decision::Stop),
          (Outcome::LiveNotStarted, Decision::Stop),
          (Outcome::Cancelled, Decision::Stop),
          (Outcome::LoginPending, Decision::Suspend),
          (Outcome::RateLimited, Decision::Backoff),
          (Outcome::Blocked, Decision::Backoff),
          (Outcome::Forbidden, cont),
          (Outcome::Network(NetworkScope::PageHost), Decision::Stop),
          (Outcome::Network(NetworkScope::StageLocal), cont),
          (
            Outcome::Removed,
            if stage == StageId::PageScan {
              cont
            } else {
              Decision::Stop
            },
          ),
          (
            Outcome::Login,
            Decision::Continue {
              login_wanted: phase == Phase::Anonymous,
            },
          ),
          (
            Outcome::AgeRestricted,
            Decision::Continue {
              login_wanted: phase == Phase::Anonymous,
            },
          ),
          (
            Outcome::Paid,
            Decision::Continue {
              login_wanted: phase == Phase::Anonymous,
            },
          ),
          (
            Outcome::Private,
            Decision::Continue {
              login_wanted: phase == Phase::Anonymous,
            },
          ),
        ] {
          assert_eq!(
            next(outcome, context(stage, phase, Confidence::High)),
            expected,
            "{stage:?} {phase:?} {outcome:?}"
          );
        }
      }
    }
  }

  #[test]
  fn weak_or_generic_removed_evidence_cannot_stop_other_engines() {
    for confidence in [
      Confidence::None,
      Confidence::Low,
      Confidence::Medium,
      Confidence::High,
    ] {
      let expected = if confidence >= Confidence::Medium {
        Decision::Stop
      } else {
        Decision::Continue {
          login_wanted: false,
        }
      };
      assert_eq!(
        next(
          Outcome::Removed,
          context(StageId::Gallery, Phase::Anonymous, confidence)
        ),
        expected
      );
    }
    let mut ctx = context(StageId::Ytdlp, Phase::Anonymous, Confidence::High);
    ctx.known_login_host = false;
    assert_eq!(
      next(Outcome::Private, ctx),
      Decision::Continue {
        login_wanted: false
      }
    );
  }

  #[test]
  fn stage_local_failures_retry_once_then_continue_but_youtube_is_exclusive() {
    for phase in [Phase::Anonymous, Phase::Login] {
      for exclusive in [false, true] {
        let mut ctx = context(StageId::Ytdlp, phase, Confidence::High);
        ctx.claim.exclusive = exclusive;
        for outcome in [Outcome::RateLimited, Outcome::Blocked] {
          assert_eq!(next(outcome, ctx), Decision::Backoff);
          ctx.retried = true;
          assert_eq!(
            next(outcome, ctx),
            if exclusive {
              Decision::Stop
            } else {
              Decision::Continue {
                login_wanted: false,
              }
            }
          );
          ctx.retried = false;
        }
        if exclusive {
          for outcome in [
            Outcome::NoMatch,
            Outcome::Unknown,
            Outcome::Unsupported,
            Outcome::ExtractFailed,
            Outcome::Login,
            Outcome::AgeRestricted,
            Outcome::Paid,
            Outcome::Private,
            Outcome::Forbidden,
            Outcome::KindMismatch,
            Outcome::NoMedia,
            Outcome::Network(NetworkScope::StageLocal),
          ] {
            assert_eq!(next(outcome, ctx), Decision::Stop, "{outcome:?}");
          }
        }
      }
    }
  }
}
