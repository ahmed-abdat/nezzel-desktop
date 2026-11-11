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
  gallery_files::{finalize, read_image, Staging},
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
  if asset.file_index.is_none() {
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
  if asset.file_index.is_none() {
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
    let resolution = crate::resolve::resolve(&entry.url).await;
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
  let staging = Staging::new()?;
  let mut images = Vec::new();
  let image_assets: Vec<_> = selected
    .iter()
    .copied()
    .filter(|asset| matches!(asset.item.kind, AttachmentKind::Image))
    .collect();
  let sources = runner
    .images(&post, &image_assets, &entry.group_id, &staging.path)
    .await?;
  for (asset, source) in image_assets.iter().zip(sources) {
    if !is_group_running(&entry.group_id) {
      return Err(GalleryError::Cancelled);
    }
    let bytes = read_image(&source)?;
    let destination = filename(app, entry, asset, &bytes, post.assets.len() > 1, &post).await?;
    images.push((source, destination));
  }
  // A video login retry resumes the original selected manifest, not a detached child download.
  for asset in &selected {
    if matches!(asset.item.kind, AttachmentKind::Video) {
      let video = video_request(entry, &post, asset, login.browser.clone())?;
      match run_ytdlp_download_with_cookies(app.clone(), video, runner._cookies.clone()).await {
        Ok(()) => {}
        Err(YtdlpDownloadError::LoginPending) => {
          return Err(GalleryError::Refused("imageLoginPending"))
        }
        Err(YtdlpDownloadError::ExpiredMedia) => return Err(GalleryError::Expired),
        Err(_) => return Err(GalleryError::Refused("imageSourceUnavailable")),
      }
    }
  }
  for (index, (source, destination)) in images.iter().enumerate() {
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
  if !is_group_running(&entry.group_id) {
    return Err(GalleryError::Cancelled);
  }
  if let Some(records) = app.try_state::<crate::download_records::DownloadRecords>() {
    records.seal(&entry.group_id);
  }
  login.succeeded(app, &entry.group_id);
  Ok(())
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
  loop {
    login.announce(&app, &entry.id, &entry.group_id);
    match download_attempt(&app, &entry, &mut login).await {
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
          RetryAction::AskLogin => return Err(YtdlpDownloadError::LoginPending),
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
  use serde_json::json;

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
      cp "$(dirname "$0")/fixture.png" "$destination/asset.png"
      printf '{"nezzel_url":"%s","extension":"png"}\n' "$url" > "$destination/receipt.json"
      ;;
    *) exit 1 ;;
  esac
done < "$input"
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
      .images(&post, &selected, &group, &stage)
      .await
      .unwrap();
    let destination = root.path.join("saved/b.png");
    finalize(&downloaded[0], &destination, || false).unwrap();
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
    let paths = runner.images(&post, &all, &group, &stage).await.unwrap();
    assert_eq!(paths.len(), 5);
    for (index, path) in paths.iter().enumerate() {
      let destination = root.path.join(format!("saved/{}.png", index + 1));
      finalize(path, &destination, || false).unwrap();
      // Resume recognizes our completed bytes rather than overwriting a different file.
      finalize(path, &destination, || false).unwrap();
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
    crate::scheduling::group_state::remove_group(&group);
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
    let input = child
      .resolved
      .as_ref()
      .expect("selected resolver video must skip another post resolution");
    assert_eq!(input.url, "https://cdn.example/v.mp4");
    assert_eq!(input.source_page.as_deref(), Some(resolved.source.as_str()));
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
