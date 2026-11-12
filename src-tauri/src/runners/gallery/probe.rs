//! Probing a post: the gallery-dl child with its deadline and the yt-dlp fallback probe.
use super::runner::GalleryRunner;
use crate::extract::deadline::{remaining_budget, wait_until, Interrupted, FETCH_TIMEOUT};
use crate::models::ParsedMedia;
use crate::parsers::gallery::{failure, gallery_deadline, parse_output, GalleryError, GalleryPost};
use crate::runners::{
  browser_cookies::CookieAccess, gallery_process::execute_with_started, ytdlp_runner::YtdlpRunner,
};
use crate::scheduling::{download_pipeline::DownloadEntry, group_state::subscribe_group};
use std::time::Duration;
use tauri::AppHandle;

impl GalleryRunner {
  pub(super) async fn probe(&self, source: &str, group: &str) -> Result<GalleryPost, GalleryError> {
    let mut cancel = subscribe_group(group);
    if !*cancel.borrow() {
      return Err(GalleryError::Cancelled);
    }
    let budget = remaining_budget(Duration::from_secs(45));
    if budget.is_zero() {
      return Err(GalleryError::Refused(FETCH_TIMEOUT));
    }
    let deadline = tokio::time::Instant::now() + budget;
    match wait_until(self.probe_hops(source, group), deadline, &mut cancel).await {
      Ok(result) => result,
      Err(Interrupted::Cancelled) => Err(GalleryError::Cancelled),
      Err(Interrupted::Timeout) => Err(GalleryError::Refused(FETCH_TIMEOUT)),
    }
  }

  pub(super) async fn probe_hops(
    &self,
    source: &str,
    group: &str,
  ) -> Result<GalleryPost, GalleryError> {
    let mut target = source.to_string();
    let mut deadline = Duration::from_secs(90);
    if reqwest::Url::parse(source).is_ok_and(|url| {
      matches!(url.host_str(), Some("imgur.com" | "www.imgur.com")) && url.path().starts_with("/a/")
    }) {
      let mut args = self.args.clone();
      args.extend([
        "-j".into(),
        "--range".into(),
        "1".into(),
        "--".into(),
        target.clone(),
      ]);
      if let Some(cookies) = &self._cookies {
        cookies
          .check_live_consent()
          .map_err(GalleryError::Refused)?;
      }
      let output =
        execute_with_started(&self.path, &args, subscribe_group(group), deadline, || {
          if let Some(cookies) = &self._cookies {
            cookies.announce_used();
          }
        })
        .await?;
      match parse_output(output.code, &output.stdout, &output.stderr, source) {
        Ok(first) => {
          let count = first
            .assets
            .iter()
            .filter_map(|asset| asset.metadata["count"].as_u64())
            .max()
            .unwrap_or(1)
            .min(256) as usize;
          deadline = gallery_deadline(count);
        }
        Err(GalleryError::Redirect(next)) => target = next,
        Err(error) => return Err(error),
      }
    }
    for _ in 0..3 {
      let mut args = self.args.clone();
      args.extend(["-j".into(), "--".into(), target.clone()]);
      if let Some(cookies) = &self._cookies {
        cookies
          .check_live_consent()
          .map_err(GalleryError::Refused)?;
      }
      let output =
        execute_with_started(&self.path, &args, subscribe_group(group), deadline, || {
          if let Some(cookies) = &self._cookies {
            cookies.announce_used();
          }
        })
        .await?;
      match parse_output(output.code, &output.stdout, &output.stderr, &target) {
        Err(GalleryError::Redirect(next)) => target = next,
        Ok(mut post) => {
          post.source = source.into();
          return Ok(post);
        }
        result => return result,
      }
    }
    Err(GalleryError::Invalid)
  }
}

/// Refresh once per parent, then match all selections by stable attachment ID.
pub(super) async fn probe_ytdlp_post(
  app: &AppHandle,
  entry: &DownloadEntry,
  browser: Option<&str>,
  target: &str,
  cookies: Option<&CookieAccess>,
) -> Result<GalleryPost, GalleryError> {
  let mut input = crate::resolve::ResolvedInput::new(&entry.url);
  input.rewrite(target.into());
  let output = YtdlpRunner::new(app)
    .with_input_args(entry.overrides.as_ref())
    .with_auth_args(entry.overrides.as_ref(), browser, &entry.url)
    .with_cookie_access(cookies)
    .with_network_args(entry.overrides.as_ref())
    .with_args(["-J", "--flat-playlist"])
    .with_resolved_input(&input)
    .output(&entry.group_id)
    .await
    .map_err(|_| GalleryError::Invalid)?
    .ok_or(GalleryError::Cancelled)?;
  if output.code != 0 {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let matcher = crate::parsers::ytdlp_error::DiagnosticMatcher::shared()
      .map_err(|_| GalleryError::Invalid)?;
    let parser =
      crate::parsers::ytdlp_error::YtdlpErrorParser::new(&entry.id, &entry.group_id, matcher)
        .with_source_url(&entry.url);
    return Err(
      if stderr
        .lines()
        .any(|line| parser.login_reason(line).is_some())
      {
        GalleryError::Login
      } else {
        failure(&stderr)
      },
    );
  }
  if output.stdout.len() > 8 * 1024 * 1024 {
    return Err(GalleryError::Invalid);
  }
  let json = std::str::from_utf8(&output.stdout).map_err(|_| GalleryError::Invalid)?;
  resolved_ytdlp_post(json, entry.id.clone(), &entry.url).await
}

pub(super) async fn resolved_ytdlp_post(
  json: &str,
  id: String,
  source: &str,
) -> Result<GalleryPost, GalleryError> {
  let ParsedMedia::Single(media) =
    crate::parsers::ytdlp_info::parse_ytdlp_info_for_url(json, id, Some(source))
      .map_err(|_| GalleryError::Invalid)?
  else {
    return Err(GalleryError::Invalid);
  };
  if !crate::resolve::admit_post_attachments(&media.attachments).await {
    return Err(GalleryError::Invalid);
  }
  let metadata = serde_json::to_string(&media).map_err(|_| GalleryError::Invalid)?;
  GalleryPost::from_resolved(&metadata, source)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::runners::gallery_files::Staging;
  use serde_json::json;
  use std::path::PathBuf;

  #[tokio::test(start_paused = true)]
  #[cfg(unix)]
  async fn probe_deadline_kills_a_stalled_gallery_child() {
    let staging = Staging::new().unwrap();
    let marker = staging.path.join("pid");
    let group = format!("gallery-deadline-{}", uuid::Uuid::new_v4());
    let runner = GalleryRunner {
      path: PathBuf::from("/bin/sh"),
      args: vec![
        "-c".into(),
        "trap '' TERM; echo $$ > \"$1\"; exec /usr/bin/tail -f /dev/null".into(),
        "sh".into(),
        marker.to_string_lossy().into_owned(),
      ],
      _config: staging,
      _cookies: None,
    };
    let task_group = group.clone();
    let start = tokio::time::Instant::now();
    let running =
      tokio::spawn(async move { runner.probe("https://example.com/post", &task_group).await });
    let pid = loop {
      if let Some(pid) = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
      {
        break pid;
      }
      assert!(
        !running.is_finished(),
        "child exited before publishing its pid"
      );
      tokio::task::yield_now().await;
    };
    tokio::time::advance(Duration::from_secs(45)).await;
    let result = running.await.unwrap();
    crate::scheduling::group_state::remove_group(&group);
    assert_eq!(
      unsafe { libc::kill(pid, 0) },
      -1,
      "deadline must reap child"
    );
    assert_eq!(result.unwrap_err(), GalleryError::Refused("fetchTimeout"));
    assert_eq!(tokio::time::Instant::now() - start, Duration::from_secs(45));
  }

  #[tokio::test]
  async fn ytdlp_post_refresh_rejects_private_media_or_thumbnails() {
    // Literal public/private IPs exercise production admission without DNS or network requests.
    for (url, thumbnail, admitted) in [
      (
        "https://127.0.0.1/video.mp4",
        "https://8.8.8.8/cover.jpg",
        false,
      ),
      (
        "https://8.8.8.8/video.mp4",
        "https://127.0.0.1/cover.jpg",
        false,
      ),
      (
        "https://8.8.8.8/video.mp4",
        "https://8.8.8.8/cover.jpg",
        true,
      ),
    ] {
      let source = "https://www.reddit.com/comments/abc123/";
      let info = json!({"_type":"playlist", "entries":[{"id":"clip", "url":url, "ext":"mp4", "thumbnail":thumbnail}]});
      assert_eq!(
        resolved_ytdlp_post(&info.to_string(), "row".into(), source)
          .await
          .is_ok(),
        admitted,
        "{url}, {thumbnail}"
      );
    }
  }
}
