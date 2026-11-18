//! Media download flow: the video request, attempts, manifest refresh and the entry point.
use super::{
  images::filename,
  info::{diagnostic, error_diagnostic, image_post},
  probe::probe_ytdlp_post,
  routing::{download_engine, probe_with_gallery, retry_gallery_login, DownloadEngine},
  runner::GalleryRunner,
};
use crate::models::parsed::{AttachmentKind, FetchEngine};
use crate::models::progress::{
  MediaDestination, MediaDestinationPath, MediaProgress, ProgressCategory,
};
use crate::models::TrackType;
use crate::parsers::gallery::{GalleryAsset, GalleryError, GalleryPost};
use crate::runners::{
  gallery_files::{finalize, read_image},
  login::{CookieRetry, LoginRequest, LoginSession},
  retry_policy::RetryAction,
  ytdlp_download::{
    run_ytdlp_download, run_ytdlp_download_with_cookies, run_ytdlp_download_with_login,
    YtdlpDownloadError,
  },
  ytdlp_runner::YtdlpRunner,
};
use crate::scheduling::{
  download_pipeline::DownloadEntry,
  group_state::{is_group_running, subscribe_group},
};
use crate::sites::{site_by_domain, SiteId};
use tauri::{AppHandle, Manager};

pub(super) fn video_request(
  entry: &DownloadEntry,
  post: &GalleryPost,
  asset: &GalleryAsset,
  browser: Option<String>,
) -> Result<DownloadEntry, GalleryError> {
  let position = post
    .assets
    .iter()
    .position(|a| a.item.id == asset.item.id)
    .ok_or(GalleryError::Invalid)?;
  let mut video = entry.clone();
  video.attachment_ids = None;
  video.fetch_engine = Some(FetchEngine::Ytdlp);
  video.media_kind = None;
  video.emit_complete = false;
  video.login_browser = browser;
  video.resume_parent = Some(Box::new(entry.clone()));
  video.resolved = None;
  let listing = !crate::parsers::post::single_post_url(&post.source);
  if listing {
    let host = reqwest::Url::parse(&post.source).ok();
    let site = host
      .as_ref()
      .and_then(|url| url.host_str())
      .and_then(site_by_domain);
    let target = match site {
      Some(SiteId::X) => asset.metadata["tweet_id"].as_u64().map(|id| {
        format!(
          "https://x.com/i/status/{id}/video/{}",
          asset.metadata["num"].as_u64().unwrap_or(1)
        )
      }),
      Some(SiteId::Bluesky) => asset.metadata["post_id"].as_str().and_then(|id| {
        let handle = asset.metadata["author"]["handle"].as_str()?;
        Some(format!("https://bsky.app/profile/{handle}/post/{id}"))
      }),
      _ => None,
    };
    if let Some(target) = target {
      video.url = target;
      video.video_index = None;
      return Ok(video);
    }
  }
  // Mixed Instagram posts can expose CDN videos that its yt-dlp post extractor cannot see.
  // Delegated ytdl: entries still point at Instagram and retain b014's carousel position.
  let direct_instagram_video = post.has_images()
    && reqwest::Url::parse(&post.source)
      .is_ok_and(|url| url.host_str().and_then(site_by_domain) == Some(SiteId::Instagram))
    && reqwest::Url::parse(&asset.item.url).is_ok_and(|url| {
      url.path().ends_with(".mp4")
        && url.host_str().and_then(site_by_domain) != Some(SiteId::Instagram)
    });
  if asset.file_index.is_none() || direct_instagram_video || listing {
    if let Some(title) = &post.title {
      let title = if post.assets.len() > 1 {
        format!("{title} ({}/{})", asset.index, post.assets.len())
      } else {
        title.clone()
      };
      video
        .template_context
        .values
        .insert("title".into(), title.clone());
      video
        .template_context
        .values
        .insert("fulltitle".into(), title);
    }
  }
  if asset.file_index.is_none() || direct_instagram_video || listing {
    // Direct reader videos (e.g. Threads) may have no dimensions. The empty height is
    // authoritative; yt-dlp's empty-placeholder setting would otherwise leave "-(p)".
    if asset.item.height.is_none() {
      video.template_context.insert("height", "");
    }
    // Expired selected CDN URLs fail or resume the parent manifest. Retrying the
    // whole post inside yt-dlp could save an attachment the user did not select.
    let mut input = crate::resolve::ResolvedInput::new(&asset.item.url);
    input.source_page = Some(post.source.clone());
    video.resolved = Some(Box::new(input));
    video.video_index = None;
    return Ok(video);
  }
  video.url = post.extract_url.clone();
  video.video_index = Some(
    post.assets[..=position]
      .iter()
      .filter(|a| matches!(a.item.kind, AttachmentKind::Video))
      .count(),
  );
  let mut url = reqwest::Url::parse(&post.extract_url).map_err(|_| GalleryError::Invalid)?;
  if url.host_str().and_then(site_by_domain) == Some(SiteId::Instagram) {
    // Instagram playlists retain photo entries. Counting videos selects the wrong child.
    video.video_index = asset.file_index;
  }
  if url.host_str().and_then(site_by_domain) == Some(SiteId::X) {
    let base = url
      .path()
      .split("/photo/")
      .next()
      .unwrap_or(url.path())
      .split("/video/")
      .next()
      .unwrap_or(url.path());
    let index = asset.metadata["num"].as_u64().unwrap_or(asset.index as u64);
    url.set_path(&format!("{base}/video/{index}"));
    video.url = url.to_string();
    video.video_index = None;
  }
  Ok(video)
}

pub(super) async fn download_attempt(
  app: &AppHandle,
  entry: &DownloadEntry,
  login: &mut LoginSession,
  recovery: &mut crate::runners::reliability::Recovery,
) -> Result<(), GalleryError> {
  login.has_session = false;
  let runner = GalleryRunner::new(
    app,
    entry.overrides.as_ref(),
    login.browser.as_deref(),
    &entry.url,
    &entry.group_id,
  )
  .await?;
  login.has_session = runner
    ._cookies
    .as_ref()
    .is_some_and(|cookies| cookies.has_session);
  let post = if !probe_with_gallery(entry) {
    if !crate::resolve::admit_post_attachments(&entry.attachments).await {
      return Err(GalleryError::Invalid);
    }
    GalleryPost::from_attachments(
      entry.attachments.clone(),
      &entry.url,
      entry.template_context.values.get("title").cloned(),
      entry.template_context.values.get("uploader").cloned(),
    )
    .map(|mut post| {
      if let Some(input) = entry.resolved.as_deref() {
        post.extract_url.clone_from(&input.url);
      }
      post
    })
  } else {
    let resolution =
      if crate::resolve::readers(&entry.url).contains(&crate::routing::ReaderId::Threads) {
        if let Some(cookies) = runner._cookies.as_ref().filter(|c| c.has_session) {
          cookies.resolve_threads(&entry.url).await
        } else {
          crate::resolve::resolve(&entry.url).await
        }
      } else {
        crate::resolve::resolve(&entry.url).await
      };
    match resolution {
      crate::resolve::Resolution::Failed { failure } => {
        return Err(GalleryError::Refused(failure.code()))
      }
      crate::resolve::Resolution::Attachments { metadata } => {
        GalleryPost::from_resolved(&metadata, &entry.url)
      }
      resolution => {
        let (target, title) = match resolution {
          crate::resolve::Resolution::Passthrough(url) => (url, None),
          crate::resolve::Resolution::PostMedia { url, title, .. } => (url, title),
          _ => (entry.url.clone(), None),
        };
        let gallery = if probe_with_gallery(entry) {
          let probe_target = if crate::resolve::facebook_photo_target(&target) {
            &target
          } else {
            &entry.url
          };
          runner.probe(probe_target, &entry.group_id).await
        } else {
          Err(GalleryError::Unsupported)
        };
        if entry.attachment_ids.is_some()
          && crate::parsers::post::single_post_url(&entry.url)
          && !matches!(&gallery, Err(GalleryError::Cancelled))
          && gallery.as_ref().map_or(true, |post| {
            post.selected(entry.attachment_ids.as_deref()).is_err()
          })
        {
          probe_ytdlp_post(
            app,
            entry,
            login.browser.as_deref(),
            &target,
            runner._cookies.as_ref(),
          )
          .await
          .map(|mut post| {
            post.title = title.or(post.title);
            post
          })
        } else {
          gallery
        }
      }
    }
  };
  let post = if entry.fetch_engine == Some(FetchEngine::Gallery) || entry.attachment_ids.is_some() {
    post?
  } else {
    image_post(post)?.ok_or(GalleryError::Unsupported)?
  };
  let selected = post.selected(entry.attachment_ids.as_deref())?;
  if let Some(records) = app.try_state::<crate::download_records::DownloadRecords>() {
    records.allow_root(
      &entry.group_id,
      YtdlpRunner::new(app).output_root(&TrackType::Video, entry.overrides.as_ref()),
    );
  }
  let root = YtdlpRunner::new(app).output_root(&TrackType::Video, entry.overrides.as_ref());
  // Images are finalized within this attempt; unlike video partials they are not resumable.
  let staging = crate::runners::gallery_files::Staging::in_root(&root)?;
  let mut images = Vec::new();
  let mut skipped = 0;
  let mut last_failure = GalleryError::Refused("imageSourceUnavailable");
  let image_assets: Vec<_> = selected
    .iter()
    .copied()
    .filter(|asset| matches!(asset.item.kind, AttachmentKind::Image))
    .collect();
  let sources = runner
    .images(&post, &image_assets, &entry.group_id, &staging.path, || {
      recovery.progressed()
    })
    .await?;
  for (asset, source) in image_assets.iter().zip(sources) {
    if !is_group_running(&entry.group_id) {
      return Err(GalleryError::Cancelled);
    }
    let source = match source {
      Ok(source) => source,
      Err(error) => {
        skipped += 1;
        last_failure = error;
        continue;
      }
    };
    let bytes = match read_image(&source) {
      Ok(bytes) => bytes,
      Err(error) => {
        skipped += 1;
        last_failure = error;
        continue;
      }
    };
    let destination = filename(app, entry, asset, &bytes, post.assets.len() > 1, &post).await?;
    images.push((source, destination));
  }
  for (index, (source, destination)) in images.iter().enumerate() {
    crate::scheduling::disk_space::check(
      destination,
      std::fs::metadata(source).ok().map(|m| m.len()),
    )
    .map_err(GalleryError::Refused)?;
    finalize(source, destination, || !is_group_running(&entry.group_id))?;
    if let Some(records) = app.try_state::<crate::download_records::DownloadRecords>() {
      records.record_item(&entry.group_id, &entry.id, &destination.to_string_lossy());
    }
    crate::events::emit(
      app,
      crate::events::MEDIA_DESTINATION,
      MediaDestination {
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        is_merged: false,
        destination: MediaDestinationPath {
          confidence: 100,
          path: destination.to_string_lossy().into_owned(),
          already_downloaded: false,
        },
      },
    );
    crate::events::emit(
      app,
      crate::events::MEDIA_PROGRESS,
      MediaProgress {
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        category: ProgressCategory::Other,
        percentage: Some((index + 1) as f64 / images.len() as f64 * 100.0),
        speed_bps: None,
        eta_secs: None,
      },
    );
  }
  let mut saved = images.len();
  // Photos are committed before a video can request login or fail.
  for asset in &selected {
    if matches!(asset.item.kind, AttachmentKind::Video) {
      let video = video_request(entry, &post, asset, login.browser.clone())?;
      match crate::events::ATTACHMENT_ATTEMPT
        .scope(
          (),
          run_ytdlp_download_with_cookies(app.clone(), video, runner._cookies.clone()),
        )
        .await
      {
        Ok(()) => {
          saved += 1;
        }
        Err(YtdlpDownloadError::LoginPending) => {
          return Err(GalleryError::Refused("imageLoginPending"))
        }
        Err(error) => {
          skipped += 1;
          last_failure = if matches!(error, YtdlpDownloadError::ExpiredMedia) {
            GalleryError::Expired
          } else {
            GalleryError::Refused("imageSourceUnavailable")
          };
        }
      }
    }
  }
  if saved == 0 {
    return Err(last_failure);
  }
  if skipped > 0 {
    let mut event = error_diagnostic(
      &entry.group_id,
      &entry.id,
      &GalleryError::Refused(skipped_code(saved, skipped, &last_failure)),
      login.has_session,
    );
    event.level = crate::models::error::DiagnosticLevel::Warning;
    crate::events::emit(
      app,
      crate::events::MEDIA_DIAGNOSTIC,
      crate::models::MediaDiagnosticPayload::from_diagnostic_event(event),
    );
  }
  if !is_group_running(&entry.group_id) {
    return Err(GalleryError::Cancelled);
  }
  if let Some(records) = app.try_state::<crate::download_records::DownloadRecords>() {
    records.seal(&entry.group_id);
  }
  login.succeeded(app, &entry.group_id);
  Ok(())
}

/// "The other items were saved" is only true when most were; a CDN burst limit is named instead.
fn skipped_code(saved: usize, skipped: usize, last_failure: &GalleryError) -> &'static str {
  if skipped > saved && *last_failure == GalleryError::Refused("rateLimited429") {
    "rateLimited429"
  } else {
    "galleryItemsSkipped"
  }
}

pub(super) fn refresh_expired_manifest(
  entry: &mut DownloadEntry,
  error: &GalleryError,
  refreshed: &mut bool,
) -> bool {
  if *refreshed || *error != GalleryError::Expired {
    return false;
  }
  *refreshed = true;
  // Refresh may reorder assets or add new ones. Freeze the original all-selected
  // manifest to stable IDs before discarding its expiring transfer URLs.
  if entry.attachment_ids.is_none() && !entry.attachments.is_empty() {
    entry.attachment_ids = Some(
      entry
        .attachments
        .iter()
        .map(|asset| asset.id.clone())
        .collect(),
    );
  }
  entry.attachments.clear();
  entry.resolved = None;
  true
}

/// An expiry that survives the one refresh is a plain access refusal again:
/// keep the 403 reason and the sign-in offer the pre-refresh path gave.
pub(super) fn after_refresh(error: GalleryError) -> GalleryError {
  if error == GalleryError::Expired {
    GalleryError::Refused("accessForbidden403")
  } else {
    error
  }
}

pub async fn run_media_download(
  app: AppHandle,
  mut entry: DownloadEntry,
) -> Result<(), YtdlpDownloadError> {
  match download_engine(&entry) {
    DownloadEngine::Files => {
      return crate::files::runner::download(app, entry)
        .await
        .map_err(|e| YtdlpDownloadError::RunnerError(e.code().into()));
    }
    DownloadEngine::Ytdlp => return run_ytdlp_download(app, entry).await,
    DownloadEngine::Gallery => {}
  }
  let mut login = LoginSession::new(
    &app,
    &entry.url,
    entry.overrides.as_ref(),
    entry.login_browser.clone(),
    entry.login_was_accepted,
  );
  let mut refreshed = false;
  let start = tokio::time::Instant::now();
  let mut recovery = crate::runners::reliability::Recovery::default();
  let mut cancel = subscribe_group(&entry.group_id);
  loop {
    if !*cancel.borrow() {
      return Ok(());
    }
    loop {
      let delay = crate::runners::reliability::host_delay(&entry.url);
      if delay.is_zero() {
        break;
      }
      if !crate::runners::reliability::wait(&app, &entry, &mut cancel, delay, true).await {
        return Ok(());
      }
    }
    login.announce(&app, &entry.id, &entry.group_id);
    match download_attempt(&app, &entry, &mut login, &mut recovery).await {
      Ok(()) => {
        crate::library::complete(&app, &entry).await;
        return Ok(());
      }
      Err(GalleryError::Cancelled) => return Ok(()),
      Err(GalleryError::Unsupported)
        if entry.fetch_engine.is_none() && entry.attachment_ids.is_none() =>
      {
        let mut video = entry;
        video.login_browser = login.browser.clone();
        return run_ytdlp_download_with_login(app, video, login).await;
      }
      Err(GalleryError::Refused("imageLoginPending")) => {
        return Err(YtdlpDownloadError::LoginPending)
      }
      Err(error) => {
        if error == GalleryError::Refused("rateLimited429") {
          if let Some(delay) = recovery.busy_delay() {
            crate::runners::reliability::cool_host(&entry.url, delay);
            continue;
          }
          crate::runners::reliability::fatal(&app, &entry, "rateLimited429");
          return Err(YtdlpDownloadError::RunnerError("rateLimited429".into()));
        }
        if error == GalleryError::NetworkDown
          || matches!(
            error,
            GalleryError::Refused("networkTimeout" | "networkNameResolutionFailed")
          )
        {
          if let Some(delay) = recovery.network_delay(start.elapsed()) {
            if !crate::runners::reliability::wait(&app, &entry, &mut cancel, delay, false).await {
              return Ok(());
            }
            continue;
          }
        }
        if error == GalleryError::Refused("downloadStalled") && recovery.restart_stall() {
          continue;
        }
        if refresh_expired_manifest(&mut entry, &error, &mut refreshed) {
          continue;
        }
        let error = after_refresh(error);
        let mut cancel = subscribe_group(&entry.group_id);
        if !*cancel.borrow() {
          return Ok(());
        }
        if retry_gallery_login(&error) {
          match login
            .retry_cookie_fetch(error.cookie_fetch_failure(login.has_session), &mut cancel)
            .await
          {
            CookieRetry::Cancelled => return Ok(()),
            CookieRetry::Retry => continue,
            CookieRetry::GiveUp => {}
          }
        }
        let reason = (error == GalleryError::Login
          || matches!(error, GalleryError::Refused(code) if crate::runners::browser_cookies::read_error_code(code).is_some())
          || (!login.uses_cookies() && error == GalleryError::Refused("unableToExtractIdOrInfo")))
        .then_some("signInRequired");
        let diagnostics = vec![error_diagnostic(
          &entry.group_id,
          &entry.id,
          &error,
          login.has_session,
        )];
        login.record_failure(&diagnostics, error == GalleryError::NetworkDown);
        match if !retry_gallery_login(&error) {
          RetryAction::Stop
        } else {
          login.failed(
            &app,
            LoginRequest::Download(entry.clone()),
            reason,
            false,
            !is_group_running(&entry.group_id),
          )
        } {
          RetryAction::Browser | RetryAction::Anonymous => continue,
          RetryAction::AskLogin | RetryAction::Pending => {
            return Err(YtdlpDownloadError::LoginPending)
          }
          _ => {}
        }
        for event in login.failure_diagnostics(diagnostics) {
          diagnostic(&app, &entry.group_id, &entry.id, event);
        }
        return Err(YtdlpDownloadError::NonZeroExit(1));
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::ParsedMedia;
  use crate::runners::gallery::images::verified_image;
  use crate::runners::gallery_files::Staging;
  use serde_json::json;

  #[test]
  fn mostly_rate_limited_gallery_names_the_limit_instead_of_saying_the_rest_were_saved() {
    let limited = crate::parsers::gallery::stderr_failure(
      "[flickr][error] HttpError: '429 Too Many Requests' for 'https://live.staticflickr.com/1/2_o.jpg'",
      false,
    );
    assert_eq!(skipped_code(1, 99, &limited), "rateLimited429");
    assert_eq!(skipped_code(99, 1, &limited), "galleryItemsSkipped");
    assert_eq!(
      skipped_code(1, 99, &GalleryError::Refused("imageSourceUnavailable")),
      "galleryItemsSkipped"
    );
  }

  #[test]
  fn imgur_selected_videos_keep_distinct_cdn_targets_instead_of_repeating_the_album() {
    let post = crate::parsers::gallery::parse_gallery(
      include_bytes!("../../parsers/fixtures/gallery-imgur-album.json"),
      "https://imgur.com/a/VemXvh0",
    )
    .unwrap();
    let item = serde_json::from_value(json!({"id":"id","url":post.source,"format":{"trackType":"both"},"mediaKind":"mixed","templateContext":{"values":{}}})).unwrap();
    let entry = DownloadEntry::from((item, "imgur-test".into()));
    let videos = post
      .assets
      .iter()
      .filter(|asset| matches!(asset.item.kind, AttachmentKind::Video));
    for asset in videos {
      let child = video_request(&entry, &post, asset, None).unwrap();
      assert_eq!(
        (
          child
            .resolved
            .as_ref()
            .map(|input| (input.url.as_str(), input.source_page.as_deref())),
          crate::runners::ytdlp_download::selected_video_args(&child)
        ),
        (
          Some((asset.item.url.as_str(), Some(post.source.as_str()))),
          vec![
            "--no-playlist".to_string(),
            "--force-generic-extractor".to_string()
          ]
        )
      );
    }
  }

  #[tokio::test]
  #[cfg(unix)]
  async fn resolver_subset_transfers_only_selected_cdn_image_and_finalizes_its_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let root = Staging::new().unwrap();
    let binary = root.path.join("gallery-fixture");
    std::fs::write(
      root.path.join("fixture.png"),
      include_bytes!("../../../../src/assets/placeholder.png"),
    )
    .unwrap();
    std::fs::write(
      &binary,
      r#"#!/bin/sh
set -eu
input=''
range=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --input-file) input="$2"; shift 2 ;;
    --range) range="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ "$range" = '1' ]
printf 'run\n' >> "$(dirname "$0")/invocations"
while IFS= read -r line; do
  case "$line" in
    -base-directory=*) destination=$(printf '%s' "$line" | cut -d '"' -f 2) ;;
    -extractor.directlink.transform=*) url=$(printf '%s' "$line" | cut -d '"' -f 4) ;;
    https://nezzel.invalid/attachment.jpg)
      printf '%s\n' "$url" >> "$(dirname "$0")/transferred"
      if [ -f "$(dirname "$0")/skip" ] && [ "${destination##*/}" = 2 ]; then continue; fi
      cp "$(dirname "$0")/fixture.png" "$destination/asset.png"
      printf '{"nezzel_url":"%s","extension":"png"}\n' "$url" > "$destination/receipt.json"
      ;;
    *) exit 1 ;;
  esac
done < "$input"
if [ -f "$(dirname "$0")/skip" ]; then echo 'ERROR: HTTP Error 404: Not Found' >&2; exit 1; fi
"#,
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let source = "https://www.threads.com/@user/post/abc";
    let mut post = crate::parsers::gallery::parse_gallery(br#"[[3,"https://cdn.example/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example/b?token=old",{"extension":"jpg"}]]"#, source).unwrap();
    post.assets[1].item.id = "threads-second-image".into();
    let metadata = serde_json::to_string(&post.into_media("post".into())).unwrap();
    let post = GalleryPost::from_resolved(&metadata, source).unwrap();
    let selected = post
      .selected(Some(&["threads-second-image".into()]))
      .unwrap();
    let runner = GalleryRunner {
      path: binary,
      args: vec![],
      _config: Staging::new().unwrap(),
      _cookies: None,
    };
    let group = format!("gallery-subset-{}", uuid::Uuid::new_v4());
    let stage = root.path.join("transfer");
    std::fs::create_dir(&stage).unwrap();
    let downloaded = runner
      .images(&post, &selected, &group, &stage, || {})
      .await
      .unwrap();
    let destination = root.path.join("saved/b.png");
    finalize(downloaded[0].as_ref().unwrap(), &destination, || false).unwrap();
    assert_eq!(
      std::fs::read(destination).unwrap(),
      include_bytes!("../../../../src/assets/placeholder.png")
    );
    assert_eq!(
      std::fs::read_dir(root.path.join("saved")).unwrap().count(),
      1
    );
    assert_eq!(
      std::fs::read_to_string(root.path.join("transferred")).unwrap(),
      "https://cdn.example/b?token=old\n"
    );
    let post = crate::parsers::gallery::parse_gallery(br#"[[3,"https://cdn.example/1.jpg",{"extension":"jpg"}],[3,"https://cdn.example/2.jpg",{"extension":"jpg"}],[3,"https://cdn.example/3.jpg",{"extension":"jpg"}],[3,"https://cdn.example/4.jpg",{"extension":"jpg"}],[3,"https://cdn.example/5.jpg",{"extension":"jpg"}]]"#, source).unwrap();
    let all = post.selected(None).unwrap();
    let stage = root.path.join("slideshow");
    std::fs::create_dir(&stage).unwrap();
    let paths = runner
      .images(&post, &all, &group, &stage, || {})
      .await
      .unwrap();
    assert_eq!(paths.len(), 5);
    for (index, path) in paths.iter().enumerate() {
      let destination = root.path.join(format!("saved/{}.png", index + 1));
      finalize(path.as_ref().unwrap(), &destination, || false).unwrap();
      // Resume recognizes our completed bytes rather than overwriting a different file.
      finalize(path.as_ref().unwrap(), &destination, || false).unwrap();
      assert_eq!(
        std::fs::read(destination).unwrap(),
        include_bytes!("../../../../src/assets/placeholder.png")
      );
    }
    assert_eq!(
      std::fs::read_to_string(root.path.join("invocations")).unwrap(),
      "run\nrun\n"
    );
    // Every selected image keeps its own identity receipt, including the last in the batch.
    std::fs::write(
      stage.join("5/receipt.json"),
      br#"{"nezzel_url":"https://cdn.example/wrong.jpg","extension":"png"}"#,
    )
    .unwrap();
    assert_eq!(
      verified_image(&post, all[4], &group, &stage.join("5")),
      Err(GalleryError::Refused("imageSelectionChanged"))
    );
    // Recorded Pinterest board: a nonzero process exit and a missing receipt must
    // preserve every other asset in its original slot.
    std::fs::write(root.path.join("skip"), b"").unwrap();
    let board = crate::parsers::gallery::parse_output(
      0,
      include_bytes!("../../parsers/fixtures/gallery-recorded-pinterest-board.json"),
      b"",
      "https://www.pinterest.com/nasa/board/",
    )
    .unwrap();
    let assets = board.selected(None).unwrap();
    let transfer = root.path.join("board");
    std::fs::create_dir(&transfer).unwrap();
    let results = runner
      .images(&board, &assets, &group, &transfer, || {})
      .await
      .unwrap();
    assert_eq!(results.len(), assets.len());
    assert!(results[1].is_err());
    for (index, result) in results.iter().enumerate().filter(|(index, _)| *index != 1) {
      let path = result.as_ref().unwrap();
      assert_eq!(
        path
          .parent()
          .unwrap()
          .file_name()
          .unwrap()
          .to_str()
          .unwrap(),
        (index + 1).to_string()
      );
      assert_eq!(
        std::fs::read(path).unwrap(),
        include_bytes!("../../../../src/assets/placeholder.png")
      );
    }
    crate::scheduling::group_state::remove_group(&group);
  }

  #[test]
  fn recorded_gallery_videos_use_the_asset_not_the_listing() {
    for (source, fixture) in [
      (
        "https://www.instagram.com/p/DfxRiPZIXJo/",
        include_bytes!("../../parsers/fixtures/gallery-recorded-instagram-mixed.json").as_slice(),
      ),
      (
        "https://www.instagram.com/reel/DfxRiPZIXJo/",
        include_bytes!("../../parsers/fixtures/gallery-recorded-instagram-mixed.json").as_slice(),
      ),
      (
        "https://www.instagram.com/nasa/p/DfxRiPZIXJo/",
        include_bytes!("../../parsers/fixtures/gallery-recorded-instagram-mixed.json").as_slice(),
      ),
      (
        "https://www.instagram.com/p/DfxRiPZIXJo/embed/",
        include_bytes!("../../parsers/fixtures/gallery-recorded-instagram-mixed.json").as_slice(),
      ),
      (
        "https://x.com/NASA/media",
        include_bytes!("../../parsers/fixtures/gallery-recorded-x-listing.json").as_slice(),
      ),
      (
        "https://bsky.app/profile/nytimes.com/media",
        include_bytes!("../../parsers/fixtures/gallery-recorded-bluesky-listing.json").as_slice(),
      ),
    ] {
      let post = crate::parsers::gallery::parse_output(0, fixture, b"", source).unwrap();
      let item = serde_json::from_value(json!({"id":"id","url":source,"format":{"trackType":"both"},"fetchEngine":"gallery","templateContext":{"values":{}}})).unwrap();
      let entry = DownloadEntry::from((item, "group".into()));
      for asset in post
        .assets
        .iter()
        .filter(|asset| matches!(asset.item.kind, AttachmentKind::Video))
      {
        let child = video_request(&entry, &post, asset, None).unwrap();
        assert_eq!(child.video_index, None);
        if source.contains("instagram") {
          assert_eq!(
            child.resolved.as_ref().unwrap().url,
            asset.metadata["video_url"].as_str().unwrap()
          );
          let media = serde_json::to_string(&post.clone().into_media("post".into())).unwrap();
          let restored = GalleryPost::from_resolved(&media, source).unwrap();
          let restored_asset = restored
            .assets
            .iter()
            .find(|a| matches!(a.item.kind, AttachmentKind::Video))
            .unwrap();
          assert_eq!(
            video_request(&entry, &restored, restored_asset, None)
              .unwrap()
              .resolved
              .unwrap()
              .url,
            child.resolved.unwrap().url
          );
        } else if source.contains("x.com") {
          assert!(child
            .url
            .contains(&format!("/i/status/{}/video/", asset.metadata["tweet_id"])));
        } else {
          assert!(child.url.contains(&format!(
            "/post/{}",
            asset.metadata["post_id"].as_str().unwrap()
          )));
        }
      }
    }
  }

  #[test]
  fn instagram_mixed_video_uses_gallery_cdn_and_pure_video_keeps_positions() {
    let source = "https://www.instagram.com/p/DfxRiPZIXJo/";
    let post = crate::parsers::gallery::parse_gallery(br#"[[3,"https://cdn.example/a.jpg",{"extension":"jpg","num":1}],[3,"https://cdn.example/v.mp4?sig=example",{"extension":"mp4","num":2}]]"#, source).unwrap();
    let item = serde_json::from_value(json!({"id":"id","url":source,"format":{"trackType":"both"},"fetchEngine":"gallery","mediaKind":"mixed","templateContext":{"values":{}}})).unwrap();
    let entry = DownloadEntry::from((item, "group".into()));
    let child = video_request(&entry, &post, &post.assets[1], None).unwrap();
    assert_eq!(
      child.resolved.as_ref().map(|input| input.url.as_str()),
      Some("https://cdn.example/v.mp4?sig=example")
    );
    assert_eq!(
      child.resolved.as_ref().unwrap().source_page.as_deref(),
      Some(source)
    );
    assert_eq!(child.video_index, None);
    let mut videos = post.clone();
    videos.assets.remove(0);
    let child = video_request(&entry, &videos, &videos.assets[0], None).unwrap();
    assert!(child.resolved.is_none());
    assert_eq!(child.video_index, Some(2));
  }

  #[test]
  fn instagram_delegated_video_selects_its_carousel_position_not_video_count() {
    let source = "https://www.instagram.com/p/DfxRiPZIXJo/";
    let post = crate::parsers::gallery::parse_gallery(br#"[[3,"https://cdn.example/a.jpg",{"extension":"jpg","num":1}],[6,"ytdl:https://www.instagram.com/p/DfxRiPZIXJo/2.mp4",{"extension":"mp4","num":2}],[3,"https://cdn.example/b.jpg",{"extension":"jpg","num":3}]]"#, source).unwrap();
    let metadata = serde_json::to_string(&post.clone().into_media("post".into())).unwrap();
    let post = GalleryPost::from_resolved(&metadata, source).unwrap();
    let item = serde_json::from_value(json!({"id":"id","url":source,"format":{"trackType":"both"},"attachmentIds":[post.assets[1].item.id],"fetchEngine":"gallery","mediaKind":"mixed","templateContext":{"values":{}}})).unwrap();
    let entry = DownloadEntry::from((item, "group".into()));
    let child = video_request(&entry, &post, &post.assets[1], None).unwrap();
    assert_eq!(child.url, source);
    assert!(child.resolved.is_none());
    assert_eq!(
      crate::runners::ytdlp_download::selected_video_args(&child),
      ["--yes-playlist", "--playlist-items", "2"]
    );
    assert!(matches!(post.assets[0].item.kind, AttachmentKind::Image));
    assert_eq!(download_engine(&entry), DownloadEngine::Gallery);
  }

  #[test]
  fn selected_video_retry_keeps_the_original_manifest_and_uses_its_source_position() {
    let source = "https://x.com/user/status/123/photo/1";
    let post = crate::parsers::gallery::parse_gallery(br#"[[3,"https://cdn.example/a.jpg",{"extension":"jpg","num":1}],[3,"https://cdn.example/v.mp4",{"extension":"mp4","num":2}]]"#, source).unwrap();
    let metadata = serde_json::to_string(&post.clone().into_media("post".into())).unwrap();
    let post = GalleryPost::from_resolved(&metadata, source).unwrap();
    let item = serde_json::from_value(json!({"id":"id","url":source,"format":{"trackType":"both"},"attachmentIds":[post.assets[1].item.id],"fetchEngine":"gallery","mediaKind":"mixed","templateContext":{"values":{}}})).unwrap();
    let entry = DownloadEntry::from((item, "group".into()));
    let child = video_request(
      &entry,
      &post,
      &post.assets[1],
      Some("chrome:/tmp/Profile 1".into()),
    )
    .unwrap();
    assert_eq!(child.url, "https://x.com/user/status/123/video/2");
    assert!(!child.emit_complete);
    assert_eq!(
      crate::runners::ytdlp_download::selected_video_args(&child),
      ["--no-playlist"]
    );
    assert_eq!(
      child.login_browser.as_deref(),
      Some("chrome:/tmp/Profile 1")
    );
    let resumed = child.login_request_entry();
    assert_eq!(download_engine(&child), DownloadEngine::Ytdlp);
    assert_eq!(download_engine(&resumed), DownloadEngine::Gallery);
    assert_eq!(resumed.url, source);
    assert_eq!(resumed.attachment_ids, entry.attachment_ids);
    assert!(resumed.emit_complete);
    let mut resolved = post.clone();
    resolved.source = "https://www.threads.com/@user/post/abc".into();
    resolved.extract_url.clone_from(&resolved.source);
    for asset in &mut resolved.assets {
      asset.file_index = None;
    }
    let child = video_request(&entry, &resolved, &resolved.assets[1], None).unwrap();
    assert_eq!(
      child
        .template_context
        .values
        .get("height")
        .map(String::as_str),
      Some("")
    );
    let input = child
      .resolved
      .as_ref()
      .expect("selected resolver video must skip another post resolution");
    assert_eq!(input.url, "https://cdn.example/v.mp4");
    assert_eq!(input.source_page.as_deref(), Some(resolved.source.as_str()));
    // Known dimensions leave yt-dlp/template metadata intact, including 60 fps.
    // Missing Threads dimensions suppress only the empty resolution suffix.
    for (asset_height, expected) in [(Some(720), "Title-(1080p60).mp4"), (None, "Title.mp4")] {
      resolved.assets[1].item.height = asset_height;
      let mut entry = entry.clone();
      for (key, value) in [
        ("title", "Title"),
        ("height", "1080"),
        ("fps", "60"),
        ("ext", "mp4"),
      ] {
        entry.template_context.insert(key, value);
      }
      let child = video_request(&entry, &resolved, &resolved.assets[1], None).unwrap();
      assert_eq!(child.template_context.values["fps"], "60");
      let args = crate::runners::ytdlp_args::build_location_args(
        &TrackType::Both,
        &child.template_context,
        &crate::state::config_models::OutputSettings::default(),
        &crate::state::preferences_models::PathPreferences::default(),
        "/downloads".into(),
      );
      assert_eq!(
        std::path::Path::new(&args[1]).file_name().unwrap(),
        expected
      );
      if asset_height.is_some() {
        entry.template_context.values.remove("height");
        entry.template_context.values.remove("fps");
        let probed = video_request(&entry, &resolved, &resolved.assets[1], None).unwrap();
        assert!(!probed.template_context.values.contains_key("height"));
        assert!(!probed.template_context.values.contains_key("fps"));
      }
    }
    let mut expired = input.clone();
    assert!(
      !expired.retry_original("HTTP Error 403"),
      "a selected CDN video must never retry the whole post and download another attachment"
    );
    assert_eq!(
      crate::runners::ytdlp_download::selected_video_args(&child),
      ["--no-playlist"]
    );
  }

  #[test]
  fn expired_manifest_refreshes_once_and_keeps_the_original_selection() {
    let item = serde_json::from_value(json!({"id":"id","url":"https://www.instagram.com/p/123/","format":{"trackType":"both"},"fetchEngine":"gallery","mediaKind":"carousel","attachments":[{"id":"a","kind":"image","url":"https://cdn.example/a.jpg"},{"id":"b","kind":"image","url":"https://cdn.example/b.jpg"}],"templateContext":{"values":{}}})).unwrap();
    let mut entry = DownloadEntry::from((item, "expiry-refresh".into()));
    let mut refreshed = false;
    assert!(refresh_expired_manifest(
      &mut entry,
      &GalleryError::Expired,
      &mut refreshed
    ));
    assert!(entry.attachments.is_empty());
    assert_eq!(entry.attachment_ids, Some(vec!["a".into(), "b".into()]));
    assert!(probe_with_gallery(&entry));
    assert!(!refresh_expired_manifest(
      &mut entry,
      &GalleryError::Expired,
      &mut refreshed
    ));
    let error = after_refresh(GalleryError::Expired);
    assert_eq!(error, GalleryError::Refused("accessForbidden403"));
    assert!(retry_gallery_login(&error));
  }

  #[test]
  fn selected_post_videos_keep_numbered_parent_titles_and_stable_ids_after_reordering() {
    let source = "https://www.reddit.com/comments/abc123/";
    let info = json!({"_type":"playlist", "webpage_url":source, "title":"Post title", "entries":[
      {"id":"first","url":"https://cdn.example/first.mp4","ext":"mp4"},
      {"id":"second","url":"https://cdn.example/second.mp4","ext":"mp4"}
    ]});
    let ParsedMedia::Single(media) =
      crate::parsers::ytdlp_info::parse_ytdlp_info(&info.to_string(), "post".into()).unwrap()
    else {
      panic!("post expected");
    };
    let mut post =
      GalleryPost::from_resolved(&serde_json::to_string(&media).unwrap(), source).unwrap();
    post.assets.reverse();
    let selected = post.selected(Some(&["second".into()])).unwrap();
    let item = serde_json::from_value(json!({"id":"row", "url":source, "format":{"trackType":"both"}, "attachmentIds":["second"], "mediaKind":"carousel", "templateContext":{"values":{}}})).unwrap();
    let request = video_request(
      &DownloadEntry::from((item, "group".into())),
      &post,
      selected[0],
      None,
    )
    .unwrap();
    assert_eq!(
      request
        .template_context
        .values
        .get("title")
        .map(String::as_str),
      Some("Post title (2/2)")
    );
    assert_eq!(
      request.resolved.as_ref().unwrap().url,
      "https://cdn.example/second.mp4"
    );
  }
}
