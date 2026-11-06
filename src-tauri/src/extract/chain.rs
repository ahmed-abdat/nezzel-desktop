//! Fetch stages return evidence; only this coordinator terminates a row.
use super::{
  deadline::{stage_budget, within_deadline, Interrupted, FETCH_TIMEOUT},
  outcome::*,
  page_scan::page_scan,
  policy::{self, Claim, Decision, Phase, PolicyContext},
  precedence::final_code,
  registry,
  terminal::{emit_attempts, emit_terminal, publish_terminal},
};
use crate::resolve::vimeo::drm_may_be_login;
use crate::{
  models::{
    error::{DiagnosticEvent, DiagnosticLevel},
    ParsedMedia,
  },
  parsers::ytdlp_error::stage_hint,
  runners::{login::LoginSession, ytdlp_info::YtdlpInfoFetchError},
  scheduling::{fetch_pipeline::FetchEntry, group_state::is_group_running},
};
use std::{
  future::Future,
  time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::{sync::watch, time::Instant as Deadline};

fn classify_stage(stage: StageId, code: &str, message: &str, scope: NetworkScope) -> Diagnostic {
  let diagnostic = classify(code, message, scope);
  if stage == StageId::Ytdlp
    && diagnostic.outcome == Outcome::Unknown
    && stage_hint::no_media_found(message)
  {
    return classify("noMediaFound", "", scope);
  }
  if stage != StageId::Ytdlp
    || !matches!(
      diagnostic.outcome,
      Outcome::Unknown | Outcome::ExtractFailed | Outcome::NoMedia
    )
  {
    return diagnostic;
  }
  if stage_hint::no_formats(message) {
    // No video representation is inconclusive for photo-capable engines, regardless of the host.
    classify("photosOnly", "", scope)
  } else if diagnostic.outcome == Outcome::Unknown
    && stage_hint::requested_format_unavailable(message)
  {
    // A requested selection can fail even when alternatives exist: retain the format diagnostic.
    classify("requestedFormatUnavailable", "", scope)
  } else {
    diagnostic
  }
}

/// One invocation can contain several cookie/URL retries. Keep all their causes.
pub struct StageResult<T = ParsedMedia> {
  pub media: Option<T>,
  pub attempts: Vec<Attempt>,
  pub login: Option<LoginSession>,
  pub error: Option<YtdlpInfoFetchError>,
  pub resolved_url: Option<String>,
  stage: StageId,
  confidence: Confidence,
}
impl<T> StageResult<T> {
  pub fn new(stage: StageId, confidence: Confidence) -> Self {
    Self {
      media: None,
      attempts: Vec::new(),
      login: None,
      error: None,
      resolved_url: None,
      stage,
      confidence,
    }
  }
  pub fn record_code(&mut self, code: &str, message: &str, login: bool, start: Instant) {
    self.attempts.push(Attempt::failed(
      self.stage,
      if login {
        AuthUsed::Browser
      } else {
        AuthUsed::Anonymous
      },
      self.confidence,
      classify_stage(
        self.stage,
        code,
        message,
        if self.stage == StageId::Files
          || matches!(
            code,
            "networkNameResolutionFailed" | "sslVerifyFailed" | "unableToConnectToProxy"
          )
        {
          NetworkScope::PageHost
        } else {
          NetworkScope::StageLocal
        },
      ),
      elapsed(start),
    ));
  }
  pub(crate) fn record_timeout(&mut self, login: bool, start: Instant) {
    self.record_code(FETCH_TIMEOUT, "", login, start);
  }
  pub fn record_diagnostics(
    &mut self,
    diagnostics: &[DiagnosticEvent],
    login: bool,
    start: Instant,
  ) {
    let candidates: Vec<_> = diagnostics
      .iter()
      .filter(|d| matches!(d.level, DiagnosticLevel::Error))
      .map(|d| {
        Attempt::failed(
          self.stage,
          if login {
            AuthUsed::Browser
          } else {
            AuthUsed::Anonymous
          },
          self.confidence,
          classify_stage(
            self.stage,
            &d.code,
            &d.message,
            if matches!(
              d.code.as_str(),
              "networkNameResolutionFailed" | "sslVerifyFailed" | "unableToConnectToProxy"
            ) {
              NetworkScope::PageHost
            } else {
              NetworkScope::StageLocal
            },
          ),
          elapsed(start),
        )
      })
      .collect();
    let code = final_code(&candidates).unwrap_or("unknown");
    if let Some(attempt) = candidates
      .iter()
      .find(|a| a.code.is_some_and(|c| c.as_str() == code))
    {
      self.attempts.push(*attempt);
    } else {
      self.record_code(code, "", login, start);
    }
  }
  pub fn finish(mut self, outcome: Outcome, login: bool, start: Instant) -> Self {
    self.attempts.push(Attempt {
      stage: self.stage,
      auth: if login {
        AuthUsed::Browser
      } else {
        AuthUsed::Anonymous
      },
      confidence: self.confidence,
      outcome,
      code: None,
      ms: elapsed(start),
    });
    self
  }
}
fn elapsed(start: Instant) -> u32 {
  start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}

struct ChainResult<T> {
  media: Option<T>,
  attempts: Vec<Attempt>,
  error: Option<YtdlpInfoFetchError>,
  login: Option<LoginSession>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainStep {
  Stage(StageId),
  Login {
    stage: StageId,
    reason: &'static str,
  },
}

fn gallery_claim(original: &str, resolved: Option<&str>) -> Confidence {
  registry::claim(StageId::Gallery, resolved.unwrap_or(original))
}

pub(super) fn stops(attempts: &[Attempt], exclusive: bool) -> bool {
  attempts.iter().any(|attempt| {
    matches!(
      policy::next(
        attempt.outcome,
        PolicyContext {
          phase: if attempt.auth == AuthUsed::Anonymous {
            Phase::Anonymous
          } else {
            Phase::Login
          },
          stage: attempt.stage,
          claim: Claim {
            confidence: attempt.confidence,
            exclusive: exclusive
              && !matches!(
                attempt.outcome,
                Outcome::Login | Outcome::AgeRestricted | Outcome::Paid | Outcome::Private
              ),
          },
          retried: true,
          known_login_host: false,
        }
      ),
      Decision::Stop | Decision::Suspend
    )
  })
}

struct RowState {
  phase: Phase,
  attempts: Vec<Attempt>,
  login: Option<LoginSession>,
  resolved_url: Option<String>,
}

#[cfg(test)]
async fn coordinate_budgeted<T, F: Future<Output = StageResult<T>>>(
  original: &str,
  ytdlp_budget: Duration,
  phase: Phase,
  cancel: watch::Receiver<bool>,
  run: impl FnMut(ChainStep, Option<LoginSession>, Option<String>) -> F,
) -> ChainResult<T> {
  coordinate_row(
    original,
    ytdlp_budget,
    RowState {
      phase,
      attempts: Vec::new(),
      login: None,
      resolved_url: None,
    },
    cancel,
    run,
  )
  .await
}

fn known_login_host(url: &str) -> bool {
  reqwest::Url::parse(url)
    .ok()
    .and_then(|url| {
      url
        .host_str()
        .map(crate::runners::browser_cookies::known_login_host)
    })
    .unwrap_or(false)
}

fn login_cause(attempts: &[Attempt], url: &str) -> Option<Attempt> {
  let code = final_code(attempts)?;
  attempts.iter().copied().find(|attempt| {
    attempt
      .code
      .is_some_and(|candidate| candidate.as_str() == code)
      && attempt.auth == AuthUsed::Anonymous
      && (matches!(
        attempt.outcome,
        Outcome::Login | Outcome::AgeRestricted | Outcome::Paid
      ) || (attempt.outcome == Outcome::Private && known_login_host(url))
        || (attempt.stage == StageId::Ytdlp
          && attempt.outcome == Outcome::Drm
          && drm_may_be_login(url)))
  })
}

fn login_capable(stage: StageId, attempts: &[Attempt]) -> bool {
  if stage == StageId::Files {
    return false;
  }
  let evidence: Vec<_> = attempts
    .iter()
    .filter(|a| a.stage == stage && a.auth == AuthUsed::Anonymous)
    .collect();
  evidence.is_empty()
    || evidence
      .iter()
      .any(|a| !matches!(a.outcome, Outcome::Unsupported | Outcome::NoMatch))
}

async fn coordinate_row<T, F: Future<Output = StageResult<T>>>(
  original: &str,
  ytdlp_budget: Duration,
  state: RowState,
  mut cancel: watch::Receiver<bool>,
  mut run: impl FnMut(ChainStep, Option<LoginSession>, Option<String>) -> F,
) -> ChainResult<T> {
  let RowState {
    mut phase,
    mut attempts,
    mut login,
    mut resolved_url,
  } = state;
  let row_budget = if phase == Phase::Anonymous {
    Duration::from_secs(90).max(ytdlp_budget)
  } else {
    Duration::from_secs(90)
  };
  let mut row_deadline = Deadline::now() + row_budget;
  let mut exclusive =
    registry::exclusive(original) || resolved_url.as_deref().is_some_and(registry::exclusive);
  let mut media = None;
  let mut error = None;
  let mut login_started = phase == Phase::Login;
  let mut degraded = false;
  loop {
    let phase_start = attempts.len();
    let mut stopped = false;
    let mut gallery_tried = false;
    let candidates = registry::stages_for(
      resolved_url
        .as_deref()
        .filter(|_| exclusive)
        .unwrap_or(original),
    );
    for stage in candidates {
      if Deadline::now() >= row_deadline {
        stopped = true;
        break;
      }
      if phase == Phase::Login {
        if !login_capable(stage, &attempts) {
          continue;
        }
      } else {
        let ytdlp_tried = attempts.iter().any(|a| a.stage == StageId::Ytdlp);
        if stage == StageId::Gallery {
          if gallery_claim(original, resolved_url.as_deref()) < Confidence::Low
            || gallery_tried
            || (ytdlp_tried
              && !attempts.iter().any(|a| {
                a.stage == StageId::Ytdlp
                  && matches!(
                    a.outcome,
                    Outcome::Unsupported
                      | Outcome::ExtractFailed
                      | Outcome::KindMismatch
                      | Outcome::Network(NetworkScope::StageLocal)
                      | Outcome::Login
                      | Outcome::AgeRestricted
                      | Outcome::Paid
                      | Outcome::Private
                  )
              }))
          {
            continue;
          }
          gallery_tried = true;
        }
        if stage == StageId::PageScan
          && (resolved_url
            .as_deref()
            .is_some_and(crate::resolve::reader_bound)
            || !attempts.iter().any(|a| {
              matches!(a.stage, StageId::Ytdlp | StageId::Gallery)
                && matches!(
                  a.outcome,
                  Outcome::Unsupported
                    | Outcome::ExtractFailed
                    | Outcome::Network(NetworkScope::StageLocal)
                )
            }))
        {
          continue;
        }
      }
      let deadline = row_deadline.min(Deadline::now() + stage_budget(stage, ytdlp_budget));
      let start = Instant::now();
      // Keep the session if an outer timer drops the stage future.
      let result = within_deadline(
        run(ChainStep::Stage(stage), login.clone(), resolved_url.clone()),
        deadline,
        &mut cancel,
      )
      .await;
      let result = match result {
        Ok(result) => result,
        Err(Interrupted::Cancelled) => StageResult::new(stage, Confidence::Medium).finish(
          Outcome::Cancelled,
          phase == Phase::Login,
          start,
        ),
        Err(Interrupted::Timeout) => {
          let mut result = StageResult::new(stage, Confidence::Medium);
          result.record_timeout(phase == Phase::Login, start);
          result
        }
      };
      login = result.login.or(login);
      media = result.media;
      error = result.error.or(error);
      if let Some(url) = result.resolved_url {
        exclusive |= registry::exclusive(&url);
        resolved_url = Some(url);
      }
      let evidence: Vec<_> = result
        .attempts
        .iter()
        .copied()
        .filter(|a| {
          // Some sites mislabel an account gate as DRM (resolve::vimeo). Real page DRM still stops.
          !(phase == Phase::Anonymous
            && stage == StageId::Ytdlp
            && a.outcome == Outcome::Drm
            && (drm_may_be_login(original)
              || resolved_url.as_deref().is_some_and(drm_may_be_login)))
        })
        .collect();
      stopped = stops(&evidence, exclusive) || Deadline::now() >= row_deadline;
      gallery_tried |= result.attempts.iter().any(|a| a.stage == StageId::Gallery);
      attempts.extend(result.attempts);
      if media.is_some() || stopped {
        break;
      }
    }
    if media.is_some() {
      break;
    }
    if stopped && (phase != Phase::Login || stops(&attempts[phase_start..], false)) {
      break;
    }
    if phase == Phase::Login {
      // NoMedia is final only after other login-capable stages have had their chance.
      // It is content evidence, not a cookie failure that permits an anonymous degrade.
      if attempts[phase_start..]
        .iter()
        .any(|attempt| attempt.auth != AuthUsed::Anonymous && attempt.outcome == Outcome::NoMedia)
      {
        break;
      }
      // U0: a cookie failure gets one anonymous degrade, without another consent or clock reset.
      let cookie_failure = attempts[phase_start..].iter().any(|attempt| {
        attempt.auth != AuthUsed::Anonymous
          && matches!(
            attempt.outcome,
            Outcome::Unknown
              | Outcome::Unsupported
              | Outcome::ExtractFailed
              | Outcome::Forbidden
              | Outcome::Blocked
              | Outcome::RateLimited
              | Outcome::KindMismatch
              | Outcome::Network(NetworkScope::StageLocal)
          )
      });
      if !degraded && cookie_failure && login.as_ref().is_some_and(LoginSession::uses_cookies) {
        if let Some(session) = &mut login {
          session.degrade_fetch();
        }
        degraded = true;
        phase = Phase::Anonymous;
        row_deadline = row_deadline.max(Deadline::now() + Duration::from_secs(20));
        continue;
      }
      break;
    }
    if login_started {
      break;
    }
    let Some(cause) = login_cause(&attempts, resolved_url.as_deref().unwrap_or(original)) else {
      break;
    };
    let reason = cause.code.map_or("signInRequired", DiagnosticCode::as_str);
    if let Some(session) = &mut login {
      session.set_fetch_history(attempts.clone(), resolved_url.clone());
    }
    let deadline = Deadline::now() + Duration::from_secs(90);
    let start = Instant::now();
    let result = within_deadline(
      run(
        ChainStep::Login {
          stage: cause.stage,
          reason,
        },
        login.clone(),
        resolved_url.clone(),
      ),
      deadline,
      &mut cancel,
    )
    .await;
    let result = match result {
      Ok(result) => result,
      Err(Interrupted::Cancelled) => {
        StageResult::new(cause.stage, cause.confidence).finish(Outcome::Cancelled, false, start)
      }
      Err(Interrupted::Timeout) => {
        let mut result = StageResult::new(cause.stage, cause.confidence);
        result.record_timeout(false, start);
        result
      }
    };
    let pending = result
      .attempts
      .iter()
      .any(|a| a.outcome == Outcome::LoginPending);
    let enter_login = result
      .login
      .as_ref()
      .is_some_and(LoginSession::uses_cookies);
    if cause.outcome == Outcome::Drm && (pending || enter_login) {
      for attempt in &mut attempts {
        if attempt.auth == AuthUsed::Anonymous
          && attempt.stage == StageId::Ytdlp
          && attempt.outcome == Outcome::Drm
        {
          let diagnostic = classify("signInRequired", "", NetworkScope::StageLocal);
          attempt.outcome = diagnostic.outcome;
          attempt.code = Some(diagnostic.code);
        }
      }
    }
    login = result.login.or(login);
    media = result.media;
    error = result.error.or(error);
    stopped = stops(&result.attempts, false);
    attempts.extend(result.attempts);
    if media.is_some() || stopped || !enter_login {
      break;
    }
    phase = Phase::Login;
    login_started = true;
    row_deadline = Deadline::now() + Duration::from_secs(90);
  }
  ChainResult {
    media,
    attempts,
    error,
    login,
  }
}

#[cfg(test)]
async fn coordinate<T, F: Future<Output = StageResult<T>>>(
  original: &str,
  run: impl FnMut(ChainStep, Option<LoginSession>, Option<String>) -> F,
) -> ChainResult<T> {
  let (_tx, cancel) = watch::channel(true);
  coordinate_budgeted(
    original,
    Duration::from_secs(60),
    Phase::Anonymous,
    cancel,
    run,
  )
  .await
}

#[cfg(test)]
pub(crate) async fn failed_login_for_test(url: &str, login: LoginSession) -> Option<&'static str> {
  let (_tx, cancel) = watch::channel(true);
  let result = coordinate_row(
    url,
    Duration::from_secs(60),
    RowState {
      phase: Phase::Login,
      attempts: Vec::new(),
      login: Some(login),
      resolved_url: None,
    },
    cancel,
    |step, login, _| async move {
      let ChainStep::Stage(stage) = step else {
        panic!("accepted row asked again");
      };
      assert!(login.as_ref().unwrap().uses_cookies());
      let mut result = StageResult::<()>::new(stage, Confidence::Medium);
      result.record_code("signInRequired", "", true, Instant::now());
      result
    },
  )
  .await;
  final_code(&result.attempts)
}

pub(crate) fn record_fetch_engine(media: &mut ParsedMedia, attempts: &[Attempt]) {
  use crate::models::parsed::FetchEngine;
  let ParsedMedia::Single(item) = media else {
    return;
  };
  if let Some(winner) = attempts.iter().find(|a| a.outcome == Outcome::Verified) {
    // Attachment readers can finish inside the yt-dlp stage without invoking yt-dlp.
    item.fetch_engine.get_or_insert(match winner.stage {
      StageId::Gallery => FetchEngine::Gallery,
      StageId::Files => FetchEngine::Files,
      StageId::Reader(_) => FetchEngine::Reader,
      StageId::Ytdlp | StageId::PageScan => FetchEngine::Ytdlp,
    });
  }
}

pub async fn run(
  app: &AppHandle,
  entry: &FetchEntry,
) -> Result<Option<ParsedMedia>, YtdlpInfoFetchError> {
  let ytdlp_budget = crate::runners::ytdlp_runner::YtdlpRunner::new(app)
    .with_input_args(entry.overrides.as_ref())
    .with_args(["-J", "--flat-playlist", "--", &entry.url])
    .info_timeout();
  let cancel = crate::scheduling::group_state::subscribe_group(&entry.group_id);
  let phase = if entry.login_browser.is_some() || entry.login_was_accepted {
    Phase::Login
  } else {
    Phase::Anonymous
  };
  let page_probe = super::probe::PageProbe::default();
  let page_probe = &page_probe;
  let mut session = LoginSession::new(
    app,
    &entry.url,
    entry.overrides.as_ref(),
    entry.login_browser.clone(),
    entry.login_was_accepted,
  );
  session.begin_fetch(phase);
  let mut report = coordinate_row(
    &entry.url,
    ytdlp_budget,
    RowState {
      phase,
      attempts: entry.prior_attempts.clone(),
      login: Some(session),
      resolved_url: entry.prior_resolved_url.clone(),
    },
    cancel,
    |step, login, resolved_url| async move {
      let stage = match step {
        ChainStep::Stage(stage) => stage,
        ChainStep::Login { stage, .. } => stage,
      };
      if !is_group_running(&entry.group_id) {
        return StageResult::new(stage, Confidence::Medium).finish(
          Outcome::Cancelled,
          false,
          Instant::now(),
        );
      }
      match step {
        ChainStep::Stage(StageId::Files) => {
          let mut result = crate::files::runner::info(app, entry).await;
          result.login = login;
          result
        }
        ChainStep::Stage(StageId::Gallery) => {
          crate::runners::gallery::run_gallery_info(
            app,
            entry,
            login,
            gallery_claim(&entry.url, resolved_url.as_deref()),
          )
          .await
        }
        ChainStep::Stage(StageId::PageScan) => {
          page_scan(
            app,
            entry,
            login,
            resolved_url.as_deref().unwrap_or(&entry.url),
            page_probe,
          )
          .await
        }
        ChainStep::Login { stage, reason } => {
          let Some(mut login) = login else {
            return StageResult::new(stage, Confidence::Medium);
          };
          let mut result = StageResult::new(stage, Confidence::Medium);
          match login.request_fetch_login(app, entry, reason) {
            crate::runners::retry_policy::RetryAction::Browser => {}
            crate::runners::retry_policy::RetryAction::AskLogin => {
              result = result.finish(Outcome::LoginPending, false, Instant::now());
            }
            _ => {}
          }
          result.login = Some(login);
          result
        }

        _ => match login {
          Some(login) => {
            crate::runners::ytdlp_info::run_ytdlp_info_with_login(app, entry, login).await
          }
          None => crate::runners::ytdlp_info::run_ytdlp_info_fetch(app, entry).await,
        },
      }
    },
  )
  .await;
  if !is_group_running(&entry.group_id) {
    report.media = None;
    report.attempts.push(Attempt {
      stage: StageId::Ytdlp,
      auth: AuthUsed::Anonymous,
      confidence: Confidence::Medium,
      outcome: Outcome::Cancelled,
      code: None,
      ms: 0,
    });
  }
  if let Some(media) = report.media.as_mut() {
    record_fetch_engine(media, &report.attempts);
  }
  if let Some(login) = report.login.as_mut() {
    if report.media.is_some() {
      login.fetch_succeeded(app, &entry.id, &entry.group_id);
    } else if final_code(&report.attempts).is_some_and(|code| {
      matches!(
        code,
        "signInRequired"
          | "signInRequiredForBotDetection"
          | "ageRestricted"
          | "membersOnly"
          | "videoNotFoundOrPrivate"
          | "playlistPrivateOrMissing"
      )
    }) && report
      .attempts
      .iter()
      .any(|attempt| attempt.auth != AuthUsed::Anonymous)
    {
      login.fetch_exhausted(app, entry);
    }
  }
  emit_attempts(app, &entry.id, &entry.group_id, &report.attempts);
  if publish_terminal(
    &report.attempts,
    &entry.group_id,
    &entry.id,
    // useDiagnostic localizes by code; this text only fills the problem list and bug reports.
    |code| {
      use tauri::Manager;
      if code.starts_with("file") {
        app
          .state::<crate::i18n::I18nManager>()
          .t(&format!("files.errors.{code}"))
      } else {
        format!("Couldn't get media from this link ({code})")
      }
    },
    |diagnostic, fatal| emit_terminal(app, diagnostic, fatal),
  ) {
    return Err(report.error.unwrap_or(YtdlpInfoFetchError::NonZeroExit(1)));
  }
  if report
    .attempts
    .last()
    .is_some_and(|a| a.outcome == Outcome::LoginPending)
  {
    return Err(YtdlpInfoFetchError::LoginPending);
  }
  Ok(report.media)
}

#[cfg(test)]
mod tests {
  use super::super::{
    page_scan::page_scan_plan,
    registry::stages_for,
    terminal::{publish_terminal, AttemptPayload},
  };
  use super::*;
  use std::collections::VecDeque;

  // Wrong URL priorities can silently save one video instead of a complete album.
  #[test]
  fn matrix_url_shapes_have_stable_routing() {
    for row in include_str!("fixtures/routing.tsv")
      .lines()
      .filter(|line| !line.trim_start().starts_with('#'))
    {
      let columns: Vec<_> = row.split('\t').collect();
      let [site, url, expected_order, expected_capabilities] = columns.as_slice() else {
        continue;
      };
      if columns.iter().any(|column| column.trim().is_empty()) || reqwest::Url::parse(url).is_err()
      {
        continue;
      }
      let order = stages_for(url)
        .into_iter()
        .map(|stage| match stage {
          StageId::Files => "files",
          StageId::Ytdlp => "ytdlp",
          StageId::Gallery => "gallery",
          StageId::PageScan => "pageScan",
          _ => panic!("unexpected executor: {stage:?}"),
        })
        .collect::<Vec<_>>()
        .join(",");
      assert_eq!(order, *expected_order, "{site}: {url}");
      let capabilities = super::super::registry::routes(url)
        .into_iter()
        .map(|matcher| match matcher.stage {
          StageId::Reader(reader) => format!("reader:{reader:?}").to_ascii_lowercase(),
          StageId::PageScan => "pageScan".into(),
          stage => format!("{stage:?}").to_ascii_lowercase(),
        })
        .collect::<Vec<_>>()
        .join(",");
      assert_eq!(
        capabilities, *expected_capabilities,
        "{site}: {url} capabilities"
      );
    }
  }

  #[tokio::test]
  async fn youtube_fast_path_never_probes_or_runs_fallbacks() {
    for url in [
      "https://www.youtube.com/watch?v=age-gated",
      "https://youtu.be/id",
      "https://www.youtube.com/playlist?list=123",
      "https://www.youtube.com/live/premiere",
      "https://www.youtube-nocookie.com/embed/id",
    ] {
      let report = coordinate(url, |step, _, _| async move {
        assert_eq!(
          step,
          ChainStep::Stage(StageId::Ytdlp),
          "unexpected probe/fallback"
        );
        let mut result = StageResult::new(StageId::Ytdlp, Confidence::High);
        result.media = Some(());
        result.finish(Outcome::Verified, false, Instant::now())
      })
      .await;
      assert!(report.media.is_some());
      assert_eq!(report.attempts.len(), 1);
    }
  }

  #[tokio::test]
  async fn album_claim_prevents_a_partial_ytdlp_success_from_winning() {
    let report = coordinate("https://imgur.com/a/VemXvh0", |step, _, _| async move {
      let ChainStep::Stage(stage) = step else {
        panic!("unexpected login")
      };
      let mut result = StageResult::new(stage, Confidence::High);
      match stage {
        StageId::Files => return result.finish(Outcome::NoMatch, false, Instant::now()),
        StageId::Gallery => result.media = Some(vec![1, 2, 3, 4]),
        StageId::Ytdlp => result.media = Some(vec![1]),
        _ => panic!("unexpected fallback"),
      }
      result.finish(Outcome::Verified, false, Instant::now())
    })
    .await;
    assert_eq!(report.media, Some(vec![1, 2, 3, 4]));
  }

  fn failed(stage: StageId, code: &str, login: bool) -> StageResult<()> {
    let mut result = StageResult::new(stage, Confidence::Medium);
    result.record_code(code, "", login, Instant::now());
    result
  }

  #[tokio::test(start_paused = true)]
  async fn file_share_stage_can_take_more_than_five_seconds_and_reports_timeout() {
    for delay in [6, 100] {
      let report = coordinate(
        "https://drive.google.com/file/d/abc/view",
        |step, _, _| async move {
          assert_eq!(step, ChainStep::Stage(StageId::Files));
          tokio::time::sleep(Duration::from_secs(delay)).await;
          let mut result = StageResult::new(StageId::Files, Confidence::High);
          result.media = Some(());
          result.finish(Outcome::Verified, false, Instant::now())
        },
      )
      .await;
      assert_eq!(report.media.is_some(), delay == 6);
      assert_eq!(
        final_code(&report.attempts),
        if delay == 6 {
          None
        } else {
          Some(FETCH_TIMEOUT)
        }
      );
      assert_eq!(report.attempts.len(), 1);
    }
  }

  // Without coordinator deadlines, the delayed stage verifies instead of yielding to fallback.
  #[tokio::test(start_paused = true)]
  async fn over_budget_stages_yield_to_the_next_engine() {
    for (url, slow_stage, expected) in [
      (
        "https://www.instagram.com/p/123/",
        StageId::Gallery,
        &[StageId::Files, StageId::Gallery, StageId::Ytdlp][..],
      ),
      (
        "https://example.com/post",
        StageId::Ytdlp,
        &[StageId::Files, StageId::Ytdlp, StageId::Gallery][..],
      ),
    ] {
      let mut visited = Vec::new();
      let report = coordinate(url, |step, _, _| {
        let ChainStep::Stage(stage) = step else {
          panic!("unexpected login")
        };
        visited.push(stage);
        async move {
          if stage == slow_stage {
            tokio::time::sleep(std::time::Duration::from_secs(85)).await;
          }
          let mut result = StageResult::new(stage, Confidence::Medium);
          if stage == StageId::Files {
            return result.finish(Outcome::NoMatch, false, Instant::now());
          }
          result.media = Some(());
          result.finish(Outcome::Verified, false, Instant::now())
        }
      })
      .await;
      assert!(report.media.is_some());
      assert_eq!(
        visited, expected,
        "{slow_stage:?} must time out then try the next engine"
      );
      assert_eq!(
        report
          .attempts
          .iter()
          .find(|attempt| attempt.stage == slow_stage)
          .unwrap()
          .outcome,
        Outcome::Network(NetworkScope::StageLocal)
      );
      assert_eq!(
        report
          .attempts
          .iter()
          .find(|a| a.stage == slow_stage)
          .and_then(|a| a.code)
          .map(DiagnosticCode::as_str),
        Some(FETCH_TIMEOUT)
      );
    }
  }

  // A deadline must retain the earlier login evidence and never start PageScan/login.
  #[tokio::test(start_paused = true)]
  async fn row_deadline_stops_with_the_best_completed_cause() {
    let start = tokio::time::Instant::now();
    let mut visited = Vec::new();
    let report = coordinate("https://www.instagram.com/p/123/", |step, _, _| {
      visited.push(step);
      async move {
        let ChainStep::Stage(stage) = step else {
          panic!("row expired before login")
        };
        match stage {
          StageId::Files => {
            StageResult::new(stage, Confidence::Low).finish(Outcome::NoMatch, false, Instant::now())
          }
          StageId::Gallery => {
            tokio::time::sleep(std::time::Duration::from_secs(40)).await;
            failed(stage, "signInRequired", false)
          }
          StageId::Ytdlp => {
            tokio::time::sleep(std::time::Duration::from_secs(85)).await;
            failed(stage, "urlNotSupported", false)
          }
          _ => panic!("row expired before scan"),
        }
      }
    })
    .await;
    assert_eq!(
      tokio::time::Instant::now() - start,
      std::time::Duration::from_secs(90)
    );
    assert_eq!(
      visited,
      [
        ChainStep::Stage(StageId::Files),
        ChainStep::Stage(StageId::Gallery),
        ChainStep::Stage(StageId::Ytdlp)
      ]
    );
    assert_eq!(final_code(&report.attempts), Some("signInRequired"));
  }

  #[tokio::test(start_paused = true)]
  async fn login_and_degrade_budgets_follow_the_anonymous_phase() {
    let start = Deadline::now();
    let report = coordinate(
      "https://www.instagram.com/p/123/",
      |step, login, _| async move {
        match step {
          ChainStep::Login { stage, .. } => {
            let mut result = StageResult::new(stage, Confidence::Medium);
            result.login = Some(crate::runners::login::test_consented_session());
            result
          }
          ChainStep::Stage(stage) if login.is_some() => {
            tokio::time::sleep(Duration::from_secs(if stage == StageId::Gallery {
              40
            } else {
              100
            }))
            .await;
            failed(stage, "signInRequired", true)
          }
          ChainStep::Stage(stage @ (StageId::Gallery | StageId::Ytdlp)) => {
            if stage == StageId::Gallery {
              tokio::time::sleep(Duration::from_secs(10)).await;
            }
            failed(stage, "signInRequired", false)
          }
          ChainStep::Stage(stage) => {
            StageResult::new(stage, Confidence::Low).finish(Outcome::NoMatch, false, Instant::now())
          }
        }
      },
    )
    .await;
    assert_eq!(Deadline::now() - start, Duration::from_secs(120));
    assert_eq!(final_code(&report.attempts), Some("signInRequired"));
    assert_eq!(report.attempts.last().unwrap().auth, AuthUsed::Anonymous);
  }

  #[tokio::test(start_paused = true)]
  async fn cancellation_during_a_stage_is_silent_including_at_the_deadline() {
    for seconds in [30, 45] {
      let (tx, cancel) = watch::channel(true);
      let cancellation = async move {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        tx.send(false).unwrap();
      };
      let fetch = coordinate_budgeted(
        "https://www.instagram.com/p/123/",
        Duration::from_secs(60),
        Phase::Anonymous,
        cancel,
        |step, _, _| async move {
          let ChainStep::Stage(stage) = step else {
            panic!("cancelled row entered login")
          };
          if stage == StageId::Files {
            return StageResult::<()>::new(stage, Confidence::Low).finish(
              Outcome::NoMatch,
              false,
              Instant::now(),
            );
          }
          std::future::pending().await
        },
      );
      let ((), report) = tokio::join!(cancellation, fetch);
      assert!(report.media.is_none());
      assert_eq!(report.attempts.last().unwrap().outcome, Outcome::Cancelled);
      assert_eq!(final_code(&report.attempts), None);
      assert!(!publish_terminal(
        &report.attempts,
        "group",
        "row",
        str::to_string,
        |_, _| panic!("cancellation published a terminal event")
      ));
    }
  }

  #[tokio::test(start_paused = true)]
  async fn playlist_top_stage_lifts_the_row_cap_without_resetting_it() {
    for (gallery_time, ytdlp_time, expected_time, success) in
      [(0, 110, 110, true), (40, 100, 120, false)]
    {
      let (_tx, cancel) = watch::channel(true);
      let start = Deadline::now();
      let report = coordinate_budgeted(
        "https://www.instagram.com/p/123/",
        Duration::from_secs(120),
        Phase::Anonymous,
        cancel,
        |step, _, _| async move {
          let ChainStep::Stage(stage) = step else {
            panic!("expired row entered login")
          };
          match stage {
            StageId::Files => StageResult::new(stage, Confidence::Low).finish(
              Outcome::NoMatch,
              false,
              Instant::now(),
            ),
            StageId::Gallery => {
              tokio::time::sleep(Duration::from_secs(gallery_time)).await;
              failed(stage, "signInRequired", false)
            }
            StageId::Ytdlp => {
              tokio::time::sleep(Duration::from_secs(ytdlp_time)).await;
              let mut result = StageResult::new(stage, Confidence::Medium);
              result.media = Some(());
              result.finish(Outcome::Verified, false, Instant::now())
            }
            _ => panic!("unexpected stage"),
          }
        },
      )
      .await;
      assert_eq!(Deadline::now() - start, Duration::from_secs(expected_time));
      assert_eq!(report.media.is_some(), success);
      assert_eq!(
        final_code(&report.attempts),
        if success {
          None
        } else {
          Some("signInRequired")
        }
      );
    }
  }

  #[tokio::test(start_paused = true)]
  async fn resumed_login_uses_ninety_seconds_even_for_a_playlist() {
    let (_tx, cancel) = watch::channel(true);
    let start = Deadline::now();
    let report = coordinate_budgeted(
      "https://youtube.com/playlist?list=123",
      Duration::from_secs(120),
      Phase::Login,
      cancel,
      |step, _, _| async move {
        let ChainStep::Stage(stage) = step else {
          panic!("login phase restarted")
        };
        tokio::time::sleep(Duration::from_secs(100)).await;
        failed(stage, "signInRequired", true)
      },
    )
    .await;
    assert_eq!(Deadline::now() - start, Duration::from_secs(90));
    assert_eq!(report.attempts.last().unwrap().auth, AuthUsed::Browser);
    assert!(report.media.is_none());
  }

  #[tokio::test(start_paused = true)]
  async fn fake_stages_follow_claims_stop_rules_and_publish_one_terminal_cause() {
    let high = "https://www.instagram.com/p/123/";
    let low = "https://example.com/gallery/123";
    for (name, url, results, expected_code, expected_media, expected_login) in [
      (
        "HTTP Dropbox share network never tries yt-dlp",
        "http://dropbox.com/scl/fi/abc/report.pdf?rlkey=secret&e=1&dl=0",
        vec![(StageId::Files, "fileNetwork")],
        Some("fileNetwork"),
        false,
        None,
      ),
      (
        "private file stays with Files",
        "https://docs.google.com/document/d/abc/edit",
        vec![(StageId::Files, "fileNotPublic")],
        Some("fileNotPublic"),
        false,
        None,
      ),
      (
        "rate limited file stays with Files",
        "https://drive.google.com/file/d/abc/view",
        vec![(StageId::Files, "fileRateLimited")],
        Some("fileRateLimited"),
        false,
        None,
      ),
      (
        "gallery unsupported hands off",
        high,
        vec![
          (StageId::Gallery, "urlNotSupported"),
          (StageId::Ytdlp, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "low claim follows yt-dlp unsupported",
        low,
        vec![
          (StageId::Ytdlp, "urlNotSupported"),
          (StageId::Gallery, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "Pinterest photo without video formats hands off",
        "https://pin.it/10RjZzoMd",
        vec![
          (
            StageId::Ytdlp,
            "ERROR: [Pinterest] 123: No video formats found!",
          ),
          (StageId::Gallery, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "generic no formats hands off",
        low,
        vec![
          (StageId::Ytdlp, "ERROR: [generic] No formats found"),
          (StageId::Gallery, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "short requested-format refusal hands off",
        low,
        vec![
          (
            StageId::Ytdlp,
            "ERROR: [generic] Requested format is not available",
          ),
          (StageId::Gallery, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "resolved short URL has gallery High claim",
        "https://pin.it/10RjZzoMd",
        vec![
          (StageId::Ytdlp, "resolvedPinNoFormats"),
          (StageId::Gallery, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "resolved gallery High removal stops",
        "https://pin.it/10RjZzoMd",
        vec![
          (StageId::Ytdlp, "resolvedPinNoFormats"),
          (StageId::Gallery, "notFound404"),
        ],
        Some("notFound404"),
        false,
        None,
      ),
      (
        "no-formats wording preserves DRM stop",
        low,
        vec![(
          StageId::Ytdlp,
          "ERROR: This video is DRM protected. No video formats found!",
        )],
        Some("drmProtected"),
        false,
        None,
      ),
      (
        "TikTok extraction failure tries gallery",
        "https://www.tiktok.com/@user/video/123",
        vec![
          (StageId::Ytdlp, "unableToExtractIdOrInfo"),
          (StageId::Gallery, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "both engines fail then scan once",
        low,
        vec![
          (StageId::Ytdlp, "urlNotSupported"),
          (StageId::Gallery, "unableToExtractIdOrInfo"),
          (StageId::PageScan, "noMatch"),
        ],
        Some("unableToExtractIdOrInfo"),
        false,
        None,
      ),
      (
        "gallery-first failures scan once",
        high,
        vec![
          (StageId::Gallery, "urlNotSupported"),
          (StageId::Ytdlp, "urlNotSupported"),
          (StageId::PageScan, "noMatch"),
        ],
        Some("urlNotSupported"),
        false,
        None,
      ),
      (
        "scan receives resolved URL",
        low,
        vec![
          (StageId::Ytdlp, "resolvedUnsupported"),
          (StageId::Gallery, "urlNotSupported"),
          (StageId::PageScan, "resolvedNoMatch"),
        ],
        Some("urlNotSupported"),
        false,
        None,
      ),
      (
        "bound reader host never scans",
        "https://www.threads.com/@user/post/123",
        vec![
          (StageId::Ytdlp, "urlNotSupported"),
          (StageId::Gallery, "urlNotSupported"),
        ],
        Some("urlNotSupported"),
        false,
        None,
      ),
      (
        "YouTube remains exclusive",
        "https://www.youtube.com/watch?v=123",
        vec![(StageId::Ytdlp, "urlNotSupported")],
        Some("urlNotSupported"),
        false,
        None,
      ),
      (
        "rewritten YouTube remains exclusive",
        low,
        vec![(StageId::Ytdlp, "rewrittenUnsupported")],
        Some("urlNotSupported"),
        false,
        None,
      ),
      (
        "high claim removed stops",
        high,
        vec![(StageId::Gallery, "notFound404")],
        Some("notFound404"),
        false,
        None,
      ),
      (
        "low claim removed continues to scan",
        low,
        vec![
          (StageId::Ytdlp, "urlNotSupported"),
          (StageId::Gallery, "notFound404"),
          (StageId::PageScan, "noMatch"),
        ],
        Some("unknown"),
        false,
        None,
      ),
      (
        "global network stops",
        high,
        vec![(StageId::Gallery, "networkNameResolutionFailed")],
        Some("networkNameResolutionFailed"),
        false,
        None,
      ),
      (
        "stage-local network continues",
        high,
        vec![
          (StageId::Gallery, "sponsorBlockUnreachable"),
          (StageId::Ytdlp, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "photosOnly tries gallery",
        low,
        vec![
          (StageId::Ytdlp, "photosOnly"),
          (StageId::Gallery, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "anonymous gallery login can succeed in yt-dlp",
        high,
        vec![
          (StageId::Gallery, "signInRequired"),
          (StageId::Ytdlp, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "deferred login reaches consent once after fallback fails",
        high,
        vec![
          (StageId::Gallery, "signInRequired"),
          (StageId::Ytdlp, "urlNotSupported"),
          (StageId::PageScan, "noMatch"),
        ],
        None,
        false,
        Some((StageId::Gallery, "signInRequired")),
      ),
      (
        "first age-restricted stage keeps its reason",
        high,
        vec![
          (StageId::Gallery, "ageRestricted"),
          (StageId::Ytdlp, "signInRequired"),
        ],
        None,
        false,
        Some((StageId::Gallery, "ageRestricted")),
      ),
      (
        "paid stage keeps its reason",
        high,
        vec![
          (StageId::Gallery, "membersOnly"),
          (StageId::Ytdlp, "urlNotSupported"),
          (StageId::PageScan, "noMatch"),
        ],
        None,
        false,
        Some((StageId::Gallery, "membersOnly")),
      ),
      (
        "gallery extraction failure hands off",
        high,
        vec![
          (StageId::Gallery, "unableToExtractIdOrInfo"),
          (StageId::Ytdlp, "verified"),
        ],
        None,
        true,
        None,
      ),
      (
        "gallery unknown hands off",
        high,
        vec![(StageId::Gallery, "unknown"), (StageId::Ytdlp, "verified")],
        None,
        true,
        None,
      ),
      (
        "yt-dlp success stops before low gallery",
        low,
        vec![(StageId::Ytdlp, "verified")],
        None,
        true,
        None,
      ),
      (
        "geo stops",
        high,
        vec![(StageId::Gallery, "geoBlocked")],
        Some("geoBlocked"),
        false,
        None,
      ),
      (
        "DRM stops",
        high,
        vec![(StageId::Gallery, "drmProtected")],
        Some("drmProtected"),
        false,
        None,
      ),
      (
        "live not started stops",
        low,
        vec![(StageId::Ytdlp, "livestreamNotStarted")],
        Some("livestreamNotStarted"),
        false,
        None,
      ),
      (
        "cancel stops silently",
        high,
        vec![(StageId::Gallery, "cancelled")],
        None,
        false,
        None,
      ),
      (
        "pending consent suspends silently",
        high,
        vec![(StageId::Gallery, "loginPending")],
        None,
        false,
        None,
      ),
      (
        "no scan without extraction or unsupported evidence",
        low,
        vec![(StageId::Ytdlp, "unknown")],
        Some("unknown"),
        false,
        None,
      ),
    ] {
      if crate::files::hosts::is_share(url) {
        assert_eq!(
          stages_for(url),
          vec![StageId::Files],
          "{name}: Files owns the row"
        );
      }
      let started = Deadline::now();
      let mut results = VecDeque::from(results);
      let mut consents = 0;
      let report = coordinate(url, |step, _, resolved_url| {
        let (stage, code) = match step {
          ChainStep::Login { stage, reason } => {
            assert_eq!(
              Some((stage, reason)),
              expected_login,
              "{name}: deferred target/reason"
            );
            consents += 1;
            (stage, "loginPending")
          }
          ChainStep::Stage(StageId::Files)
            if results
              .front()
              .is_none_or(|(stage, _)| *stage != StageId::Files) =>
          {
            (StageId::Files, "noMatch")
          }
          ChainStep::Stage(stage) => {
            let (expected_stage, code) = results.pop_front().expect(name);
            assert_eq!(stage, expected_stage, "{name}: stage order");
            (stage, code)
          }
        };
        let mut result = StageResult::new(
          stage,
          if stage == StageId::Gallery {
            gallery_claim(url, resolved_url.as_deref())
          } else {
            Confidence::Medium
          },
        );
        let stderr = if code == "resolvedPinNoFormats" {
          result.resolved_url = Some("https://www.pinterest.com/pin/123/".into());
          Some("ERROR: [Pinterest] 123: No video formats found!")
        } else {
          code.starts_with("ERROR:").then_some(code)
        };
        if let Some(stderr) = stderr {
          let matcher = crate::parsers::ytdlp_error::DiagnosticMatcher::shared().unwrap();
          let parser = crate::parsers::ytdlp_error::YtdlpErrorParser::new("row", "group", matcher)
            .with_source_url(url);
          result.record_diagnostics(
            &parser.parse_lines(vec![stderr], false),
            false,
            Instant::now(),
          );
          return std::future::ready(result);
        }
        match code {
          "verified" => {
            result.media = Some(());
            result = result.finish(Outcome::Verified, false, Instant::now());
          }
          "resolvedNoMatch" => {
            assert_eq!(
              resolved_url.as_deref(),
              Some("https://example.com/resolved-post"),
              "{name}"
            );
            result = result.finish(Outcome::NoMatch, false, Instant::now());
          }
          "resolvedUnsupported" => {
            result.record_code("urlNotSupported", "", false, Instant::now());
            result.resolved_url = Some("https://example.com/resolved-post".into());
          }
          "noMatch" => result = result.finish(Outcome::NoMatch, false, Instant::now()),
          "cancelled" => result = result.finish(Outcome::Cancelled, false, Instant::now()),
          "loginPending" => result = result.finish(Outcome::LoginPending, false, Instant::now()),
          "rewrittenUnsupported" => {
            result.record_code("urlNotSupported", "", false, Instant::now());
            result.resolved_url = Some("https://www.youtube.com/watch?v=123".into());
          }
          code => result.record_code(code, "", false, Instant::now()),
        }
        std::future::ready(result)
      })
      .await;
      if name == "resolved short URL has gallery High claim" {
        assert_eq!(
          report
            .attempts
            .iter()
            .find(|a| a.stage == StageId::Gallery)
            .map(|a| a.confidence),
          Some(Confidence::High),
          "{name}"
        );
      }
      if name == "pending consent suspends silently" {
        assert_eq!(
          Deadline::now() - started,
          Duration::ZERO,
          "LoginPending must suspend without exhausting the row budget"
        );
        assert_eq!(
          report.attempts.last().unwrap().outcome,
          Outcome::LoginPending
        );
      }
      assert!(results.is_empty(), "{name}: missing fallback");
      assert_eq!(
        (report.media.is_some(), consents),
        (expected_media, usize::from(expected_login.is_some())),
        "{name}"
      );
      let mut errors = Vec::new();
      let mut fatals = Vec::new();
      publish_terminal(
        &report.attempts,
        "group",
        "item",
        str::to_owned,
        |diagnostic, fatal| {
          assert!(
            !fatal.message.is_empty() && fatal.details.as_deref().is_some_and(|d| !d.is_empty()),
            "{name}"
          );
          assert!(!diagnostic.raw.is_empty(), "{name}: retain stage trail");
          errors.push(diagnostic.code);
          fatals.push(fatal);
        },
      );
      assert_eq!(
        (errors, fatals.len()),
        (
          expected_code.into_iter().map(str::to_owned).collect(),
          usize::from(expected_code.is_some())
        ),
        "{name}"
      );
    }
  }

  #[test]
  fn page_scan_plan_keeps_bound_readers_and_probes_only_changed_urls() {
    for (original, resolved, expected) in [
      (
        "https://example.com/post",
        "https://example.com/post",
        Some(false),
      ),
      (
        "https://example.com/post",
        "https://example.com/redirected",
        Some(true),
      ),
      (
        "https://www.threads.com/@user/post/123",
        "https://example.com/redirected",
        None,
      ),
      (
        "https://example.com/post",
        "https://www.facebook.com/watch?v=123",
        None,
      ),
    ] {
      assert_eq!(
        page_scan_plan(original, resolved),
        expected,
        "{original} -> {resolved}"
      );
    }
  }

  #[test]
  fn cookie_no_media_is_content_evidence_instead_of_a_login_failure() {
    let mut result = failed(StageId::Ytdlp, "signInRequired", false);
    result.record_code("unknown", "ERROR: No media found", true, Instant::now());
    assert_eq!(final_code(&result.attempts), Some("noMediaFound"));
    assert!(!stops(&result.attempts[1..], false));
  }

  #[tokio::test]
  async fn known_private_and_vimeo_drm_reach_one_consent_after_anonymous_stages() {
    for (url, code) in [
      ("https://www.instagram.com/p/123/", "videoNotFoundOrPrivate"),
      ("https://vimeo.com/123", "drmProtected"),
    ] {
      let mut stages = Vec::new();
      let mut consents = 0;
      let report = coordinate(url, |step, _, _| {
        let result = match step {
          ChainStep::Login { stage, .. } => {
            consents += 1;
            StageResult::<()>::new(stage, Confidence::Medium).finish(
              Outcome::LoginPending,
              false,
              Instant::now(),
            )
          }
          ChainStep::Stage(stage) => {
            stages.push(stage);
            if stage == StageId::Ytdlp {
              failed(stage, code, false)
            } else {
              StageResult::new(stage, Confidence::Low).finish(
                Outcome::NoMatch,
                false,
                Instant::now(),
              )
            }
          }
        };
        std::future::ready(result)
      })
      .await;
      assert_eq!(consents, 1, "{code}");
      assert!(stages.contains(&StageId::Ytdlp));
      assert_eq!(
        report.attempts.last().unwrap().outcome,
        Outcome::LoginPending
      );
    }
  }

  #[tokio::test]
  async fn vimeo_without_a_consented_or_offered_browser_keeps_real_drm() {
    let mut consents = 0;
    let result = coordinate("https://vimeo.com/123", |step, _, _| {
      let result = match step {
        ChainStep::Login { stage, reason } => {
          assert_eq!(reason, "drmProtected");
          consents += 1;
          StageResult::<()>::new(stage, Confidence::Medium)
        }
        ChainStep::Stage(StageId::Ytdlp) => failed(StageId::Ytdlp, "drmProtected", false),
        ChainStep::Stage(stage) => {
          StageResult::new(stage, Confidence::Low).finish(Outcome::NoMatch, false, Instant::now())
        }
      };
      std::future::ready(result)
    })
    .await;
    assert_eq!(consents, 1);
    assert_eq!(final_code(&result.attempts), Some("drmProtected"));
  }

  #[tokio::test]
  async fn declined_consent_never_replays_a_cookie_stage_and_unsupported_is_skipped() {
    for accepted in [false, true] {
      let mut consented = false;
      let mut login_stages = Vec::new();
      let report = coordinate("https://www.instagram.com/p/123/", |step, _, _| {
        let result = match step {
          ChainStep::Login { stage, .. } => {
            assert!(!consented);
            consented = true;
            let mut result = StageResult::new(stage, Confidence::High);
            if accepted {
              result.login = Some(crate::runners::login::test_consented_session());
            }
            result
          }
          ChainStep::Stage(stage) if consented => {
            assert!(accepted, "cookies after refusal");
            login_stages.push(stage);
            failed(stage, "signInRequired", true)
          }
          ChainStep::Stage(StageId::Ytdlp) => failed(StageId::Ytdlp, "signInRequired", false),
          ChainStep::Stage(StageId::Gallery) => failed(StageId::Gallery, "urlNotSupported", false),
          ChainStep::Stage(stage) => StageResult::<()>::new(stage, Confidence::Low).finish(
            Outcome::NoMatch,
            false,
            Instant::now(),
          ),
        };
        std::future::ready(result)
      })
      .await;
      assert_eq!(
        login_stages,
        if accepted {
          vec![StageId::Ytdlp]
        } else {
          vec![]
        }
      );
      assert_eq!(final_code(&report.attempts), Some("signInRequired"));
    }
  }

  // Replaying only the first login-producing engine loses capable fallback engines.
  #[tokio::test]
  async fn row_login_replays_claim_order_once_and_terminal_media_evidence_wins() {
    for terminal in ["noMediaFound", "notFound404", "verified"] {
      let mut visited = Vec::new();
      let mut consented = false;
      let report = coordinate("https://www.instagram.com/p/123/", |step, login, _| {
        visited.push(step);
        let result = match step {
          ChainStep::Login { stage, .. } => {
            assert!(!consented, "second row consent");
            consented = true;
            let mut result = StageResult::new(stage, Confidence::High);
            result.login = Some(crate::runners::login::test_consented_session());
            result
          }
          ChainStep::Stage(stage) if consented => {
            assert_eq!(login.as_ref().unwrap().browser.as_deref(), Some("firefox"));
            if stage == StageId::Gallery {
              failed(stage, "signInRequired", true)
            } else if terminal == "verified" {
              let mut result = StageResult::new(stage, Confidence::High);
              result.media = Some(());
              result.finish(Outcome::Verified, true, Instant::now())
            } else {
              failed(stage, terminal, true)
            }
          }
          ChainStep::Stage(StageId::Gallery) => failed(StageId::Gallery, "signInRequired", false),
          ChainStep::Stage(StageId::Ytdlp) => failed(StageId::Ytdlp, "signInRequired", false),
          ChainStep::Stage(stage) => {
            StageResult::new(stage, Confidence::Low).finish(Outcome::NoMatch, false, Instant::now())
          }
        };
        std::future::ready(result)
      })
      .await;
      let mut expected = vec![
        ChainStep::Stage(StageId::Files),
        ChainStep::Stage(StageId::Gallery),
        ChainStep::Stage(StageId::Ytdlp),
        ChainStep::Login {
          stage: StageId::Gallery,
          reason: "signInRequired",
        },
        ChainStep::Stage(StageId::Gallery),
        ChainStep::Stage(StageId::Ytdlp),
      ];
      if terminal == "noMediaFound" {
        expected.push(ChainStep::Stage(StageId::PageScan));
      }
      assert_eq!(visited, expected);
      assert_eq!(
        final_code(&report.attempts),
        (terminal != "verified").then_some(terminal)
      );
    }
  }

  #[tokio::test]
  async fn logged_in_ytdlp_no_media_still_allows_a_capable_gallery_stage() {
    let (_tx, cancel) = watch::channel(true);
    let mut visited = Vec::new();
    let result = coordinate_row(
      "https://pin.it/123",
      Duration::from_secs(60),
      RowState {
        phase: Phase::Login,
        attempts: Vec::new(),
        login: Some(crate::runners::login::test_consented_session()),
        resolved_url: None,
      },
      cancel,
      |step, _, _| {
        visited.push(step);
        let ChainStep::Stage(stage) = step else {
          panic!("second consent");
        };
        let result = if stage == StageId::Ytdlp {
          let mut result = failed(stage, "noMediaFound", true);
          result.resolved_url = Some("https://www.pinterest.com/pin/123/".into());
          result
        } else {
          assert_eq!(stage, StageId::Gallery);
          assert_eq!(
            gallery_claim(
              "https://pin.it/123",
              Some("https://www.pinterest.com/pin/123/")
            ),
            Confidence::High
          );
          let mut result = StageResult::new(stage, Confidence::High);
          result.media = Some(());
          result.finish(Outcome::Verified, true, Instant::now())
        };
        std::future::ready(result)
      },
    )
    .await;
    assert_eq!(
      visited,
      vec![
        ChainStep::Stage(StageId::Ytdlp),
        ChainStep::Stage(StageId::Gallery)
      ]
    );
    assert!(result.media.is_some());
  }

  #[tokio::test(start_paused = true)]
  async fn cookie_degrade_has_a_twenty_second_floor_after_login_deadline() {
    let (_tx, cancel) = watch::channel(true);
    let start = Deadline::now();
    let result = coordinate_row(
      "https://youtube.com/watch?v=123",
      Duration::from_secs(120),
      RowState {
        phase: Phase::Login,
        attempts: Vec::new(),
        login: Some(crate::runners::login::test_consented_session()),
        resolved_url: None,
      },
      cancel,
      |step, login, _| async move {
        let ChainStep::Stage(stage) = step else {
          panic!("second consent");
        };
        if login.unwrap().uses_cookies() {
          tokio::time::sleep(Duration::from_secs(100)).await;
          failed(stage, "unableToExtractIdOrInfo", true)
        } else {
          tokio::time::sleep(Duration::from_secs(10)).await;
          let mut result = StageResult::new(stage, Confidence::Medium);
          result.media = Some(());
          result.finish(Outcome::Verified, false, Instant::now())
        }
      },
    )
    .await;
    assert_eq!(Deadline::now() - start, Duration::from_secs(100));
    assert!(result.media.is_some());
  }

  #[tokio::test]
  async fn cookie_failure_degrades_once_and_anonymous_success_keeps_browser_memory() {
    let (_tx, cancel) = watch::channel(true);
    let mut session = crate::runners::login::test_consented_session();
    session.begin_fetch(Phase::Login);
    let mut cookie_stages = 0;
    let mut anonymous_stages = 0;
    let result = coordinate_row(
      "https://youtube.com/watch?v=123",
      Duration::from_secs(60),
      RowState {
        phase: Phase::Login,
        attempts: Vec::new(),
        login: Some(session),
        resolved_url: None,
      },
      cancel,
      |step, login, _| {
        let ChainStep::Stage(stage) = step else {
          panic!("degrade asked consent");
        };
        let login = login.unwrap();
        let result = if login.uses_cookies() {
          cookie_stages += 1;
          failed(stage, "unableToExtractIdOrInfo", true)
        } else {
          anonymous_stages += 1;
          let mut result = StageResult::new(stage, Confidence::Medium);
          result.media = Some(());
          result.finish(Outcome::Verified, false, Instant::now())
        };
        std::future::ready(result)
      },
    )
    .await;
    assert_eq!((cookie_stages, anonymous_stages), (1, 1));
    assert!(result.media.is_some());
    assert!(!result.login.unwrap().uses_cookies());
  }

  #[tokio::test]
  async fn resumed_consent_preserves_history_and_enters_login_without_restarting_files() {
    let prior = vec![
      failed(StageId::Gallery, "urlNotSupported", false).attempts[0],
      failed(StageId::Ytdlp, "signInRequired", false).attempts[0],
    ];
    let (_tx, cancel) = watch::channel(true);
    let mut visited = Vec::new();
    let report = coordinate_row(
      "https://www.instagram.com/p/123/",
      Duration::from_secs(60),
      RowState {
        phase: Phase::Login,
        attempts: prior.clone(),
        login: Some(crate::runners::login::test_consented_session()),
        resolved_url: None,
      },
      cancel,
      |step, login, _| {
        visited.push(step);
        assert!(login.as_ref().unwrap().uses_cookies());
        let ChainStep::Stage(stage) = step else {
          panic!("resume asked consent again");
        };
        std::future::ready(failed(stage, "noMediaFound", true))
      },
    )
    .await;
    assert_eq!(
      visited,
      vec![
        ChainStep::Stage(StageId::Ytdlp),
        ChainStep::Stage(StageId::PageScan)
      ]
    );
    assert_eq!(&report.attempts[..2], &prior);
    assert_eq!(final_code(&report.attempts), Some("noMediaFound"));
  }

  #[test]
  fn cookie_retry_history_keeps_the_anonymous_login_cause() {
    let mut result = failed(StageId::Ytdlp, "signInRequired", false);
    result.record_code("urlNotSupported", "", true, Instant::now());
    assert_eq!(final_code(&result.attempts), Some("signInRequired"));
  }

  #[test]
  fn attempts_event_contains_only_stage_login_outcome_code_and_duration() {
    let result = failed(
      StageId::Gallery,
      "https://secret.example/private-name",
      true,
    );
    let mut attempt = result.attempts[0];
    attempt.ms = 7;
    let wire = serde_json::to_value(AttemptPayload::from(&attempt)).unwrap();
    assert!(!wire.to_string().contains("secret.example"));
    assert!(wire.get("browser").is_none() && wire.get("profile").is_none());
    assert_eq!(
      wire,
      serde_json::json!({"stage":"gallery", "login":true,"outcome":"unknown","code":"unknown","ms":7})
    );
  }
}
