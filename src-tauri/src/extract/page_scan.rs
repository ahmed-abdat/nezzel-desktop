//! PageScan stage: read an unknown page for a media candidate and hand it to yt-dlp.
use super::{
  chain::{stops, StageResult},
  outcome::*,
};
use crate::{
  models::ParsedMedia, runners::login::LoginSession, scheduling::fetch_pipeline::FetchEntry,
};
use std::time::Instant;
use tauri::AppHandle;

/// None leaves identity-bound pages with their reader; true probes a newly resolved file target.
pub(super) fn page_scan_plan(original: &str, resolved: &str) -> Option<bool> {
  if crate::resolve::reader_bound(original) || crate::resolve::reader_bound(resolved) {
    None
  } else {
    Some(original != resolved)
  }
}

pub(super) async fn page_scan(
  app: &AppHandle,
  entry: &FetchEntry,
  login: Option<LoginSession>,
  resolved_url: &str,
  probe: &super::probe::PageProbe,
) -> StageResult {
  let start = Instant::now();
  let mut stage = StageResult::new(StageId::PageScan, Confidence::Low);
  let Some(probe_file) = page_scan_plan(&entry.url, resolved_url) else {
    stage.login = login;
    return stage.finish(Outcome::NoMatch, false, start);
  };
  if probe_file {
    let mut target = entry.clone();
    target.url = resolved_url.into();
    stage = crate::files::runner::after_unsupported(app, &target).await;
    if stage.media.is_some() || stops(&stage.attempts, false) {
      stage.login = login;
      return stage;
    }
  }
  match scan_page(probe, resolved_url, &crate::resolve::http::PublicHttp).await {
    Err(code) => {
      stage.record_code(code, "", false, start);
      stage.login = login;
      stage
    }
    Ok(Some(resolution)) => {
      // A candidate is evidence to try yt-dlp, not a verified media result itself.
      stage.attempts.extend(
        StageResult::<ParsedMedia>::new(StageId::PageScan, Confidence::Low)
          .finish(Outcome::Unknown, false, start)
          .attempts,
      );
      let mut result =
        crate::runners::ytdlp_info::run_ytdlp_info_resolved(app, entry, login, resolution).await;
      stage.attempts.append(&mut result.attempts);
      result.attempts = stage.attempts;
      result
    }
    Ok(None) => {
      stage.attempts.extend(
        StageResult::<ParsedMedia>::new(StageId::PageScan, Confidence::Low)
          .finish(Outcome::NoMatch, false, start)
          .attempts,
      );
      stage.login = login;
      stage
    }
  }
}

/// Shared PageScan decision boundary; verified media outranks even a strong wall title.
pub(super) async fn scan_page<T: crate::resolve::http::Transport>(
  probe: &super::probe::PageProbe,
  url: &str,
  transport: &T,
) -> Result<Option<crate::resolve::Resolution>, &'static str> {
  // PublicHttp is anonymous; U10 can supply a scoped transport and distinct auth context.
  let Ok(signals) = probe
    .page_signals(url, super::probe::ProbeAuth::Anonymous, transport)
    .await
  else {
    return Ok(None);
  };
  if signals.drm_protected {
    return Err("drmProtected");
  }
  let resolution = signals.resolution();
  if resolution.is_none() && signals.login_wall {
    return Err("signInRequired");
  }
  Ok(resolution)
}
