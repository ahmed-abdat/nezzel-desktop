//! gallery-dl is a separate executable. No gallery Python code is linked into the app.
use crate::extract::{
  chain::{remaining_budget, wait_until, Interrupted, StageResult, FETCH_TIMEOUT},
  outcome::{Confidence, Outcome, StageId},
};
use crate::models::error::{DiagnosticEvent, DiagnosticLevel};
use crate::models::parsed::{AttachmentKind, FetchEngine, MediaKind};
use crate::models::progress::{
  MediaDestination, MediaDestinationPath, MediaProgress, ProgressCategory,
};
use crate::models::{MediaDiagnosticPayload, MediaFatalPayload, ParsedMedia, TrackType};
use crate::parsers::gallery::{
  cdn_failure, failure, gallery_deadline, gallery_first, parse_output, verify_receipt,
  GalleryAsset, GalleryError, GalleryPost,
};
use crate::runners::{
  browser_cookies::{CookieAccess, CookieContext},
  gallery_files::{finalize, image_type, read_image, Staging},
  gallery_process::{execute, find_bundled},
  login::{LoginRequest, LoginSession},
  override_resolver::resolve_with_patch,
  retry_policy::RetryAction,
  ytdlp_download::{
    run_ytdlp_download, run_ytdlp_download_with_cookies, run_ytdlp_download_with_login,
    YtdlpDownloadError,
  },
  ytdlp_runner::YtdlpRunner,
};
use crate::scheduling::{
  download_pipeline::DownloadEntry,
  fetch_pipeline::FetchEntry,
  group_state::{is_group_running, subscribe_group},
};
#[cfg(test)]
use crate::state::config_models::AuthSettings;
use crate::state::config_models::NetworkSettings;
use crate::stronghold::stronghold_state::{AuthSecrets, StrongholdState};
use crate::SharedConfig;
use serde_json::{json, Value};
use std::time::Instant;
use std::{
  path::{Path, PathBuf},
  time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

fn configuration(
  secrets: &AuthSecrets,
  network: &NetworkSettings,
  source: &str,
) -> Result<Value, GalleryError> {
  // Extractors reuse their requests session for CDNs and redirects. Do not let
  // user-defined headers (including nonstandard API keys) follow those requests.
  if secrets.bearer_token.is_some() || !secrets.headers.is_empty() {
    return Err(GalleryError::Refused("imageHeaderAuthUnavailable"));
  }
  if !matches!(network.impersonate.as_str(), "none" | "") {
    return Err(GalleryError::Refused("imageImpersonationUnavailable"));
  }
  let mut config = json!({"extractor": {}});
  match network.enable_proxy {
    Some(true) => {
      let proxy = network
        .proxy
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .ok_or(GalleryError::Refused("unableToConnectToProxy"))?;
      config["extractor"]["proxy"] = json!(proxy);
    }
    Some(false) => config["extractor"]["proxy-env"] = json!(false),
    None => {}
  }
  let host = reqwest::Url::parse(source).ok().and_then(|u| {
    u.host_str()
      .map(|h| h.trim_start_matches("www.").to_string())
  });
  let category = match host.as_deref() {
    Some("x.com" | "twitter.com" | "mobile.twitter.com") => Some("twitter"),
    Some("instagram.com") => Some("instagram"),
    Some("reddit.com" | "old.reddit.com" | "redd.it") => Some("reddit"),
    Some("bsky.app") => Some("bluesky"),
    Some("pinterest.com" | "pin.it") => Some("pinterest"),
    Some("facebook.com" | "m.facebook.com") => Some("facebook"),
    _ => None,
  };
  if let Some(category) = category {
    let mut credentials = serde_json::Map::new();
    if let Some(username) = &secrets.username {
      credentials.insert("username".into(), json!(username));
    }
    if let Some(password) = &secrets.password {
      credentials.insert("password".into(), json!(password));
    }
    config["extractor"][category] = Value::Object(credentials);
  }
  Ok(config)
}

pub(super) fn arguments(cookies: &[String], config: &Path) -> Vec<String> {
  let mut args = vec![
    "--config-ignore".into(),
    "-c".into(),
    config.to_string_lossy().into_owned(),
    "--no-colors".into(),
    "--http-timeout".into(),
    "20".into(),
    "--retries".into(),
    "0".into(),
  ];
  args.extend_from_slice(cookies);
  args
}

struct GalleryRunner {
  path: PathBuf,
  args: Vec<String>,
  _config: Staging,
  _cookies: Option<CookieAccess>,
}

/// Whether a verified gallery-dl copy ships with this build (problem reports).
pub(crate) fn available(app: &AppHandle) -> bool {
  bundled_path(app).is_ok()
}

fn bundled_path(app: &AppHandle) -> Result<PathBuf, GalleryError> {
  #[cfg(feature = "link-matrix")]
  if let Some(resources) = app.try_state::<crate::link_matrix::GalleryResources>() {
    return find_bundled(&resources.0.join("harness"), &resources.0);
  }
  find_bundled(
    &std::env::current_exe().map_err(|_| GalleryError::Unavailable)?,
    &app
      .path()
      .resource_dir()
      .map_err(|_| GalleryError::Unavailable)?,
  )
}

impl GalleryRunner {
  async fn new(
    app: &AppHandle,
    overrides: Option<&crate::models::download::DownloadOverrides>,
    browser: Option<&str>,
    source: &str,
    group: &str,
  ) -> Result<Self, GalleryError> {
    let path = bundled_path(app)?;
    let config = app.state::<SharedConfig>().load();
    let patch = overrides.and_then(|o| o.auth.as_ref());
    let auth = resolve_with_patch(&config.auth, patch);
    let secrets = if browser == Some(super::login::ANONYMOUS_BROWSER) {
      AuthSecrets::default()
    } else {
      let secrets = app
        .try_state::<StrongholdState>()
        .and_then(|s| s.load_auth_secrets().ok())
        .unwrap_or_default();
      resolve_with_patch(&secrets, patch)
    };
    let network = resolve_with_patch(&config.network, overrides.and_then(|o| o.network.as_ref()));
    let context = CookieContext::for_app(app).map_err(|_| GalleryError::Invalid)?;
    let cookies = CookieAccess::prepare(&auth, browser, source, &context, subscribe_group(group))
      .await
      .map_err(|error| match error {
        "cancelled" => GalleryError::Cancelled,
        FETCH_TIMEOUT => GalleryError::Refused(FETCH_TIMEOUT),
        _ => GalleryError::Invalid,
      })?;
    let mut runner = Self::configured(path, &secrets, &network, source)?;
    runner.args.extend(cookies.args.iter().cloned());
    runner._cookies = Some(cookies);
    Ok(runner)
  }
  fn anonymous(
    path: PathBuf,
    network: &NetworkSettings,
    source: &str,
  ) -> Result<Self, GalleryError> {
    // Background previews must never inherit saved cookies, browser choices or credentials.
    Self::configured(path, &AuthSecrets::default(), network, source)
  }
  fn configured(
    path: PathBuf,
    secrets: &AuthSecrets,
    network: &NetworkSettings,
    source: &str,
  ) -> Result<Self, GalleryError> {
    let settings = configuration(secrets, network, source)?;
    let stage = Staging::new()?;
    let config = stage.json("gallery.json", &settings)?;
    Ok(Self {
      path,
      args: arguments(&[], &config),
      _config: stage,
      _cookies: None,
    })
  }
  async fn probe(&self, source: &str, group: &str) -> Result<GalleryPost, GalleryError> {
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

  async fn probe_hops(&self, source: &str, group: &str) -> Result<GalleryPost, GalleryError> {
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
      let output = execute(&self.path, &args, subscribe_group(group), deadline).await?;
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
      let output = execute(&self.path, &args, subscribe_group(group), deadline).await?;
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
  async fn images(
    &self,
    post: &GalleryPost,
    assets: &[&GalleryAsset],
    group: &str,
    stage: &Path,
  ) -> Result<Vec<PathBuf>, GalleryError> {
    if assets.is_empty() {
      return Ok(Vec::new());
    }
    let mut input = String::new();
    for asset in assets {
      let directory = stage.join(asset.index.to_string());
      std::fs::create_dir(&directory).map_err(|_| GalleryError::Invalid)?;
      // Per-URL input options isolate filenames and receipts in a single process.
      // JSON encoding prevents a source URL or directory from injecting input directives.
      input.push_str(&format!(
        "-base-directory={}\n-extractor.directlink.transform={}\nhttps://nezzel.invalid/attachment.jpg\n",
        json!(directory),
        json!([[
          r"^https://nezzel\.invalid/attachment\.jpg$",
          asset.item.url.replace('\\', r"\\")
        ]])
      ));
    }
    let input_path = stage.join("images.txt");
    std::fs::write(&input_path, input).map_err(|_| GalleryError::Invalid)?;
    let mut args = self.args.clone();
    args.extend([
      "-o".into(),
      format!(
        "downloader.http.headers={}",
        json!({"Referer": post.source})
      ),
      "-o".into(),
      "downloader.http.filesize-max=134217728".into(),
      "--quiet".into(),
      "--range".into(),
      "1".into(),
      "--child-filter".into(),
      "False".into(),
      "--directory".into(),
      stage.to_string_lossy().into_owned(),
      "--filename".into(),
      "asset.{extension}".into(),
      "-o".into(),
      "extractor.metadata-url=nezzel_url".into(),
      "--write-metadata".into(),
      "--postprocessor-option".into(),
      "filename=receipt.json".into(),
      "--input-file".into(),
      input_path.to_string_lossy().into_owned(),
    ]);
    let output = execute(
      &self.path,
      &args,
      subscribe_group(group),
      gallery_deadline(assets.len()),
    )
    .await?;
    if output.code != 0 {
      return Err(cdn_failure(&String::from_utf8_lossy(&output.stderr)));
    }
    assets
      .iter()
      .map(|asset| verified_image(post, asset, group, &stage.join(asset.index.to_string())))
      .collect()
  }
}

fn verified_image(
  post: &GalleryPost,
  asset: &GalleryAsset,
  group: &str,
  stage: &Path,
) -> Result<PathBuf, GalleryError> {
  let receipt = stage.join("receipt.json");
  let metadata = std::fs::symlink_metadata(&receipt).map_err(|_| GalleryError::Invalid)?;
  if !metadata.is_file() || metadata.len() > 1024 * 1024 {
    return Err(GalleryError::Invalid);
  }
  verify_receipt(
    &std::fs::read(receipt).map_err(|_| GalleryError::Invalid)?,
    post,
    asset,
  )?;
  let mut files = std::fs::read_dir(stage)
    .map_err(|_| GalleryError::Invalid)?
    .filter_map(Result::ok)
    .filter(|entry| {
      entry.file_name().to_string_lossy().starts_with("asset.")
        && entry.file_type().is_ok_and(|kind| kind.is_file())
        && !entry.file_name().to_string_lossy().ends_with(".part")
    });
  let path = files
    .next()
    .ok_or(GalleryError::Refused("imageSourceUnavailable"))?
    .path();
  if files.next().is_some() || !is_group_running(group) {
    return Err(GalleryError::Invalid);
  }
  Ok(path)
}

fn image_post(
  result: Result<GalleryPost, GalleryError>,
) -> Result<Option<GalleryPost>, GalleryError> {
  match result {
    Ok(post) if post.has_images() => Ok(Some(post)),
    Ok(_) | Err(GalleryError::Unsupported) => Ok(None),
    Err(error) => Err(error),
  }
}

/// One-way fallback: a failed/no-photo probe never invokes yt-dlp again.
pub(crate) async fn photo_info(
  app: &AppHandle,
  entry: &FetchEntry,
  browser: Option<&str>,
) -> Result<Option<ParsedMedia>, GalleryError> {
  let runner = GalleryRunner::new(
    app,
    entry.overrides.as_ref(),
    browser,
    &entry.url,
    &entry.group_id,
  )
  .await?;
  let result = runner.probe(&entry.url, &entry.group_id).await;
  Ok(image_post(result)?.map(|post| ParsedMedia::Single(post.into_media(entry.id.clone()))))
}

fn preview_from_post(
  result: Result<GalleryPost, GalleryError>,
  id: String,
) -> Result<Option<Value>, GalleryError> {
  let Some(post) = image_post(result)? else {
    return Ok(None);
  };
  Ok(serde_json::to_value(post.into_media(id)).ok())
}

/// Silent, anonymous metadata only: no LoginSession, diagnostics or user gallery-dl config.
pub async fn preview_media(
  app: &AppHandle,
  source: &str,
  id: String,
  network: &NetworkSettings,
) -> Result<Option<Value>, GalleryError> {
  let runner = GalleryRunner::anonymous(bundled_path(app)?, network, source)?;
  let group = format!("capture-preview-{id}");
  // Match the yt-dlp preview budget, including all gallery redirect attempts.
  let result = tokio::time::timeout(Duration::from_secs(20), runner.probe(source, &group)).await;
  crate::scheduling::group_state::remove_group(&group);
  preview_from_post(result.map_err(|_| GalleryError::Unavailable)?, id)
}

pub(crate) fn error_diagnostic(group: &str, id: &str, error: &GalleryError) -> DiagnosticEvent {
  let code = match error {
    GalleryError::Login => "signInRequired",
    GalleryError::Unavailable => "imageEngineUnavailable",
    GalleryError::Refused(FETCH_TIMEOUT) => "unknown",
    GalleryError::Refused(code) => code,
    GalleryError::Unsupported => "urlNotSupported",
    _ => "imageSourceUnavailable",
  };
  DiagnosticEvent {
    id: id.into(),
    group_id: group.into(),
    level: DiagnosticLevel::Error,
    code: code.into(),
    component: Some("gallery-dl".into()),
    message: String::new(),
    raw: String::new(),
    timestamp: std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map_or(0, |d| d.as_millis()),
  }
}

pub(crate) fn diagnostic(app: &AppHandle, group: &str, id: &str, event: DiagnosticEvent) {
  crate::extract::chain::emit_terminal(
    app,
    MediaDiagnosticPayload::from_diagnostic_event(event),
    MediaFatalPayload::with_exit(group.into(), id.into(), 1, String::new()),
  );
}

// Compatibility entry point for link_matrix; remove after the download coordinator migrates callers.

#[cfg(feature = "link-matrix")]
pub async fn run_media_info(
  app: &AppHandle,
  entry: &FetchEntry,
) -> Result<Option<ParsedMedia>, crate::runners::ytdlp_info::YtdlpInfoFetchError> {
  crate::extract::chain::run(app, entry).await
}

/// Shared URL-only capability claim; downloads and fetch routing use the same table.
pub(crate) fn claim(source: &str) -> Confidence {
  crate::extract::registry::claim(StageId::Gallery, source)
}

pub(crate) async fn run_gallery_info(
  app: &AppHandle,
  entry: &FetchEntry,
  login: Option<LoginSession>,
  confidence: Confidence,
) -> StageResult {
  let mut login = login.unwrap_or_else(|| {
    LoginSession::new(
      app,
      &entry.url,
      entry.overrides.as_ref(),
      entry.login_browser.clone(),
      entry.login_was_accepted,
    )
  });
  let start = Instant::now();
  let mut stage = StageResult::new(StageId::Gallery, confidence);
  // Keep evidence outside the timed future: a late timeout must not erase an earlier login cause.
  // Dropping a probe kills its ChildGuard, bounding redirect hops as well as the current subprocess.
  let deadline = tokio::time::Instant::now() + remaining_budget(Duration::from_secs(45));
  let mut cancel = subscribe_group(&entry.group_id);
  let outcome = match wait_until(
    gallery_info_attempts(app, entry, &mut login, &mut stage),
    deadline,
    &mut cancel,
  )
  .await
  {
    Ok(outcome) => outcome,
    Err(Interrupted::Cancelled) => Some(Outcome::Cancelled),
    Err(Interrupted::Timeout) => {
      stage.record_timeout(login.uses_cookies(), start);
      None
    }
  };
  if let Some(outcome) = outcome {
    stage = stage.finish(outcome, login.uses_cookies(), start);
  }
  stage.login = Some(login);
  stage
}

async fn gallery_info_attempts(
  app: &AppHandle,
  entry: &FetchEntry,
  login: &mut LoginSession,
  stage: &mut StageResult,
) -> Option<Outcome> {
  loop {
    let attempt_start = Instant::now();
    login.announce(app, &entry.id, &entry.group_id);
    let (result, cookies) = match GalleryRunner::new(
      app,
      entry.overrides.as_ref(),
      login.browser.as_deref(),
      &entry.url,
      &entry.group_id,
    )
    .await
    {
      Ok(runner) => {
        let result = runner.probe(&entry.url, &entry.group_id).await;
        (result, runner._cookies.clone())
      }
      Err(error) => (Err(error), None),
    };
    if matches!(result, Err(GalleryError::Unsupported)) {
      stage.record_code("urlNotSupported", "", login.uses_cookies(), attempt_start);
      return None;
    }
    match image_post(result) {
      Ok(None) => return Some(Outcome::NoMatch),
      Ok(Some(post)) => {
        let mixed = post
          .assets
          .iter()
          .any(|a| matches!(a.item.kind, AttachmentKind::Video));
        let video_source = post
          .extract_url
          .split("/photo/")
          .next()
          .unwrap_or(&post.extract_url)
          .to_string();
        let mut media = post.into_media(entry.id.clone());
        if mixed {
          let probe = YtdlpRunner::new(app)
            .with_auth_args(
              entry.overrides.as_ref(),
              login.browser.as_deref(),
              &entry.url,
            )
            .with_cookie_access(cookies.as_ref())
            .with_network_args(entry.overrides.as_ref())
            .with_args(["-J", "--yes-playlist", "--playlist-items", "1"])
            .with_args(["--", video_source.as_str()])
            .output(&entry.group_id)
            .await;
          if let Ok(Some(output)) = probe {
            if output.code == 0 && output.stdout.len() <= 8 * 1024 * 1024 {
              if let Ok(mut value) = serde_json::from_slice::<Value>(&output.stdout) {
                if let Some(first) = value["entries"].as_array().and_then(|v| v.first()).cloned() {
                  value = first;
                }
                if let Ok(ParsedMedia::Single(video)) =
                  crate::parsers::ytdlp_info::parse_ytdlp_info(&value.to_string(), entry.id.clone())
                {
                  media.formats = video.formats;
                  media.video_tracks = video.video_tracks;
                  media.audio_tracks = video.audio_tracks;
                  media.video_codecs = video.video_codecs;
                  media.audio_codecs = video.audio_codecs;
                  media.subtitle_inventory = video.subtitle_inventory;
                }
              }
            }
          }
        }
        login.succeeded(app, &entry.group_id);
        stage.media = Some(ParsedMedia::Single(media));
        return Some(Outcome::Verified);
      }
      Err(GalleryError::Cancelled) => return Some(Outcome::Cancelled),
      Err(GalleryError::Refused(FETCH_TIMEOUT)) => {
        stage.record_timeout(login.uses_cookies(), attempt_start);
        return None;
      }
      Err(error) => {
        let mut cancel = subscribe_group(&entry.group_id);
        if !*cancel.borrow() {
          return Some(Outcome::Cancelled);
        }
        let diagnostics = vec![error_diagnostic(&entry.group_id, &entry.id, &error)];
        if error == GalleryError::NetworkDown {
          stage.record_code(
            "networkNameResolutionFailed",
            "",
            login.uses_cookies(),
            attempt_start,
          );
        } else {
          stage.record_diagnostics(&diagnostics, login.uses_cookies(), attempt_start);
        }
        if tokio::select! {
          biased;
          _ = cancel.changed() => return Some(Outcome::Cancelled),
          retry = login.retry_cookie_fetch(error.cookie_fetch_failure(), tokio::time::sleep) => retry,
        } {
          continue;
        }
        let reason = (error == GalleryError::Login
          || (!login.uses_cookies() && error == GalleryError::Refused("unableToExtractIdOrInfo")))
        .then_some("signInRequired");
        login.record_failure(&diagnostics, error == GalleryError::NetworkDown);
        if login.fetch_managed() || !login.uses_cookies() {
          return None;
        }
        match if error == GalleryError::NetworkDown {
          RetryAction::Stop
        } else {
          login.failed(
            app,
            LoginRequest::Fetch(entry.clone()),
            reason,
            false,
            !is_group_running(&entry.group_id),
          )
        } {
          RetryAction::Browser | RetryAction::Anonymous => continue,
          RetryAction::AskLogin => return Some(Outcome::LoginPending),
          _ => {}
        }
        return None;
      }
    }
  }
}

async fn filename(
  app: &AppHandle,
  entry: &DownloadEntry,
  asset: &GalleryAsset,
  bytes: &[u8],
  multiple: bool,
  post: &GalleryPost,
) -> Result<PathBuf, GalleryError> {
  let extension = image_type(bytes)?;
  let stage = Staging::new()?;
  let info = stage.json("image.json", &json!({"id":asset.item.id,"title":post.title.as_deref().unwrap_or(&asset.item.id),"uploader":post.uploader,"webpage_url":post.source,"url":asset.item.url,"ext":extension,"width":asset.item.width,"height":asset.item.height,
    "formats":[{"format_id":"image","url":asset.item.url,"ext":extension,"protocol":"https","vcodec":"unknown","acodec":"none"}]}))?;
  let output = YtdlpRunner::new(app)
    .with_location_args(
      &TrackType::Video,
      &entry.template_context,
      entry.overrides.as_ref(),
    )
    .with_args([
      "--skip-download".into(),
      "--simulate".into(),
      "--print".into(),
      "filename".into(),
      "--load-info-json".into(),
      info.to_string_lossy().into_owned(),
    ])
    .output(&entry.group_id)
    .await
    .map_err(|_| GalleryError::Invalid)?
    .ok_or(GalleryError::Cancelled)?;
  if output.code != 0 || output.stdout.len() > 32768 {
    return Err(GalleryError::Invalid);
  }
  let rendered = std::str::from_utf8(&output.stdout)
    .map_err(|_| GalleryError::Invalid)?
    .trim_end_matches(['\r', '\n']);
  if rendered.is_empty() || rendered.contains(['\r', '\n']) {
    return Err(GalleryError::Invalid);
  }
  image_destination(PathBuf::from(rendered), asset.index, multiple, extension)
}

fn image_destination(
  path: PathBuf,
  index: usize,
  multiple: bool,
  extension: &str,
) -> Result<PathBuf, GalleryError> {
  if !path.is_absolute() || path.file_name().is_none() {
    return Err(GalleryError::Invalid);
  }
  let path = if multiple {
    path.with_file_name(format!(
      "{}-{:03}.{}",
      path.file_stem().unwrap().to_string_lossy(),
      index,
      extension
    ))
  } else {
    path.with_extension(extension)
  };
  let name = path
    .file_name()
    .ok_or(GalleryError::Invalid)?
    .to_string_lossy();
  let base = name
    .split('.')
    .next()
    .unwrap_or_default()
    .trim_end_matches([' ', '.'])
    .to_ascii_uppercase();
  let numbered = base
    .strip_prefix("COM")
    .or_else(|| base.strip_prefix("LPT"));
  if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
    || numbered.is_some_and(|v| v.len() == 1 && matches!(v.as_bytes()[0], b'1'..=b'9'))
  {
    Ok(path.with_file_name(format!("_{name}")))
  } else {
    Ok(path)
  }
}

fn video_request(
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
  if matches!(
    url.host_str(),
    Some("x.com" | "twitter.com" | "www.x.com" | "www.twitter.com")
  ) {
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

/// Refresh once per parent, then match all selections by stable attachment ID.
async fn probe_ytdlp_post(
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
    let matcher = crate::parsers::ytdlp_error::DiagnosticMatcher::from_json(include_str!(
      "../diagnostic_rules.json"
    ))
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

async fn resolved_ytdlp_post(
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

async fn download_attempt(
  app: &AppHandle,
  entry: &DownloadEntry,
  login: &mut LoginSession,
) -> Result<(), GalleryError> {
  let runner = GalleryRunner::new(
    app,
    entry.overrides.as_ref(),
    login.browser.as_deref(),
    &entry.url,
    &entry.group_id,
  )
  .await?;
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
          runner.probe(&entry.url, &entry.group_id).await
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
  if let Some(records) = app.try_state::<crate::commands::DownloadRecords>() {
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
    if let Some(records) = app.try_state::<crate::commands::DownloadRecords>() {
      records.record_item(&entry.group_id, &entry.id, &destination.to_string_lossy());
    }
    let _ = app.emit(
      "media_destination",
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
    let _ = app.emit(
      "media_progress",
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
  if let Some(records) = app.try_state::<crate::commands::DownloadRecords>() {
    records.seal(&entry.group_id);
  }
  login.succeeded(app, &entry.group_id);
  Ok(())
}

fn gallery_download(entry: &DownloadEntry) -> bool {
  entry.attachment_ids.is_some()
    || matches!(
      entry.media_kind,
      Some(MediaKind::Image | MediaKind::Carousel | MediaKind::Mixed)
    )
    // A login-walled anonymous preview leaves the kind unknown: probe with gallery-dl first.
    // Known videos and selected video children keep yt-dlp and must not re-probe the parent.
    || (entry.media_kind.is_none() && entry.resume_parent.is_none() && gallery_first(&entry.url))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadEngine {
  Gallery,
  Files,
  Ytdlp,
}

fn download_engine(entry: &DownloadEntry) -> DownloadEngine {
  match entry.fetch_engine {
    Some(FetchEngine::Gallery) => DownloadEngine::Gallery,
    Some(FetchEngine::Files) => DownloadEngine::Files,
    Some(FetchEngine::Ytdlp) => DownloadEngine::Ytdlp,
    Some(FetchEngine::Reader) | None => {
      if matches!(entry.media_kind, Some(MediaKind::File)) {
        DownloadEngine::Files
      } else if gallery_download(entry) {
        DownloadEngine::Gallery
      } else {
        DownloadEngine::Ytdlp
      }
    }
  }
}

fn probe_with_gallery(entry: &DownloadEntry) -> bool {
  entry.fetch_engine.is_none()
    || (entry.attachments.is_empty()
      && matches!(
        entry.fetch_engine,
        Some(FetchEngine::Gallery | FetchEngine::Reader)
      ))
}

fn refresh_expired_manifest(
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
fn after_refresh(error: GalleryError) -> GalleryError {
  if error == GalleryError::Expired {
    GalleryError::Refused("accessForbidden403")
  } else {
    error
  }
}

fn retry_gallery_login(error: &GalleryError) -> bool {
  !matches!(error, GalleryError::Unsupported | GalleryError::NetworkDown)
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
        if retry_gallery_login(&error)
          && tokio::select! {
            biased;
            _ = cancel.changed() => return Ok(()),
            retry = login.retry_cookie_fetch(error.cookie_fetch_failure(), tokio::time::sleep) => retry,
          }
        {
          continue;
        }
        let reason = (error == GalleryError::Login
          || (!login.uses_cookies() && error == GalleryError::Refused("unableToExtractIdOrInfo")))
        .then_some("signInRequired");
        let diagnostics = vec![error_diagnostic(&entry.group_id, &entry.id, &error)];
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

  #[test]
  fn unsupported_gallery_errors_never_enter_the_login_retry_loop() {
    for (error, retry) in [
      (GalleryError::Unsupported, false),
      (GalleryError::NetworkDown, false),
      (GalleryError::Login, true),
      (GalleryError::Refused("rateLimited429"), true),
    ] {
      assert_eq!(retry_gallery_login(&error), retry, "{error:?}");
    }
  }

  #[test]
  fn imgur_albums_are_a_high_gallery_claim_but_single_pages_are_not() {
    for (url, expected) in [
      ("https://imgur.com/a/VemXvh0", Confidence::High),
      ("https://imgur.com/gallery/abc", Confidence::High),
      ("https://imgur.com/abc123", Confidence::Low),
    ] {
      assert_eq!(claim(url), expected, "{url}");
    }
    assert_eq!(
      crate::extract::chain::stages_for("https://imgur.com/a/VemXvh0")[..3],
      [StageId::Files, StageId::Gallery, StageId::Ytdlp]
    );
  }

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

  #[test]
  fn imgur_selected_videos_keep_distinct_cdn_targets_instead_of_repeating_the_album() {
    let post = crate::parsers::gallery::parse_gallery(
      include_bytes!("../parsers/fixtures/gallery-imgur-album.json"),
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

  #[test]
  fn routing_keeps_preview_login_failures_for_the_full_gallery_fetch() {
    let source = "https://www.instagram.com/p/DeAYvsnn4la/";
    for (records, expected_kind) in [
      (
        json!([[3,"https://cdn.example/a.jpg",{"extension":"jpg"}]]),
        Some("image"),
      ),
      (
        json!([[3,"https://cdn.example/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example/b.jpg",{"extension":"jpg"}]]),
        Some("carousel"),
      ),
      (
        json!([[3,"https://cdn.example/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example/v.mp4",{"extension":"mp4"}]]),
        Some("mixed"),
      ),
      (
        json!([[3,"https://cdn.example/v.mp4",{"extension":"mp4"}]]),
        None,
      ),
      (json!([]), None),
    ] {
      let post = parse_output(0, records.to_string().as_bytes(), b"", source);
      let preview = preview_from_post(post, "capture".into()).unwrap();
      assert_eq!(
        preview.as_ref().and_then(|item| item["mediaKind"].as_str()),
        expected_kind
      );
    }
    // No extractor is a video handoff, not a failed attempt that spends login retries.
    assert!(image_post(Err(GalleryError::Unsupported))
      .unwrap()
      .is_none());
    let login = parse_output(
      0,
      br#"[[-1,{"error":"AbortExtraction","message":"HTTP redirect to login page"}]]"#,
      b"",
      source,
    );
    assert_eq!(
      preview_from_post(login, "capture".into()),
      Err(GalleryError::Login)
    );
  }

  #[test]
  fn routing_gallery_fetch_is_independent_of_batch_size_and_selected_format() {
    let mut entry = FetchEntry {
      group_id: "group".into(),
      id: "photo".into(),
      url: "https://www.instagram.com/p/DeAYvsnn4la/".into(),
      total: 1,
      format: None,
      overrides: None,
      login_browser: None,
      login_was_accepted: false,
      prior_attempts: Vec::new(),
      prior_resolved_url: None,
    };
    for (url, total, format, expected) in [
      ("https://www.instagram.com/p/DeAYvsnn4la/", 1, false, true),
      ("https://www.instagram.com/p/DeAYvsnn4la/", 3, false, true),
      ("https://www.instagram.com/p/DeAYvsnn4la/", 1, true, true),
      ("https://x.com/user/status/123", 2, false, true),
      ("https://www.instagram.com/reel/123/", 1, false, false),
      ("https://www.instagram.com/", 1, false, false),
    ] {
      entry.url = url.into();
      entry.total = total;
      entry.format = format.then(|| serde_json::from_value(json!({"trackType":"both"})).unwrap());
      assert_eq!(
        gallery_first(&entry.url),
        expected,
        "{url}, total={total}, format={format}"
      );
    }
  }

  #[test]
  fn routing_gallery_download_reprobes_unknown_posts_and_browser_retries() {
    for (url, kind, expected) in [
      ("https://www.instagram.com/p/DeAYvsnn4la/", None, true),
      (
        "https://www.instagram.com/p/DeAYvsnn4la/",
        Some("video"),
        false,
      ),
      ("https://x.com/user/status/123", Some("video"), false),
      ("https://x.com/user/status/123", None, true),
      ("https://www.instagram.com/reel/123/", Some("video"), false),
      ("https://www.instagram.com/", None, false),
      ("https://www.youtube.com/watch?v=123", Some("video"), false),
      ("https://cdn.example/post", Some("image"), true),
    ] {
      let item = serde_json::from_value(json!({
        "id":"post", "url":url, "format":{"trackType":"both"},
        "mediaKind":kind, "templateContext":{"values":{}}
      }))
      .unwrap();
      let mut entry = DownloadEntry::from((item, "group".into()));
      assert_eq!(gallery_download(&entry), expected, "{url}, kind={kind:?}");
      assert_eq!(
        download_engine(&entry),
        if expected {
          DownloadEngine::Gallery
        } else {
          DownloadEngine::Ytdlp
        }
      );
      entry.login_browser = Some("chrome".into());
      assert_eq!(gallery_download(&entry), expected, "browser retry: {url}");
    }
  }

  #[test]
  fn verified_fetch_engine_overrides_gallery_heuristics() {
    for (engine, kind, expected) in [
      ("gallery", Some("audio"), DownloadEngine::Gallery),
      ("gallery", Some("video"), DownloadEngine::Gallery),
      ("gallery", Some("file"), DownloadEngine::Gallery),
      ("ytdlp", None, DownloadEngine::Ytdlp),
      ("ytdlp", Some("carousel"), DownloadEngine::Ytdlp),
      ("ytdlp", Some("file"), DownloadEngine::Ytdlp),
      ("files", Some("image"), DownloadEngine::Files),
      ("files", None, DownloadEngine::Files),
      ("reader", Some("image"), DownloadEngine::Gallery),
      ("reader", Some("video"), DownloadEngine::Ytdlp),
      ("reader", Some("file"), DownloadEngine::Files),
    ] {
      let item = serde_json::from_value(json!({
        "id":"post", "url":"https://www.instagram.com/p/_FVse5H9Pe/",
        "format":{"trackType":"both"}, "fetchEngine":engine, "mediaKind":kind,
        "templateContext":{"values":{}}
      }))
      .unwrap();
      let entry = DownloadEntry::from((item, "winner-routing".into()));
      assert_eq!(download_engine(&entry), expected, "{engine}, {kind:?}");
      assert_eq!(
        probe_with_gallery(&entry),
        matches!(engine, "gallery" | "reader"),
        "empty manifest: {engine}"
      );
    }
  }

  #[test]
  fn tiktok_video_path_photos_use_the_verified_gallery_without_a_selection() {
    let source = "https://www.tiktok.com/@x/video/7703997545197915412";
    let post = crate::parsers::gallery::parse_gallery(
      include_bytes!("../parsers/fixtures/gallery-tiktok-photos.json"),
      source,
    )
    .unwrap();
    let mut media = ParsedMedia::Single(post.into_media("photos".into()));
    let attempts = StageResult::<()>::new(StageId::Gallery, Confidence::Medium)
      .finish(Outcome::Verified, false, Instant::now())
      .attempts;
    crate::extract::chain::record_fetch_engine(&mut media, &attempts);
    let mut payload = serde_json::to_value(media).unwrap();
    payload["format"] = json!({"trackType":"both"});
    payload["templateContext"] = json!({"values":{}});
    let entry = DownloadEntry::from((serde_json::from_value(payload).unwrap(), "photos".into()));
    assert!(entry.attachment_ids.is_none());
    assert_eq!(download_engine(&entry), DownloadEngine::Gallery);
    assert!(
      !probe_with_gallery(&entry),
      "the gallery winner skips the download-time probe"
    );
    assert!(matches!(entry.media_kind, Some(MediaKind::Carousel)));
  }

  #[test]
  fn credentials_never_appear_in_sidecar_arguments() {
    let secrets = AuthSecrets {
      username: Some("private-user".into()),
      password: Some("private-password".into()),
      ..Default::default()
    };
    let network = NetworkSettings {
      enable_proxy: Some(true),
      proxy: Some("http://proxy-user:proxy-password@proxy.invalid:8080".into()),
      ..Default::default()
    };
    let runner = GalleryRunner::configured(
      PathBuf::from("gallery-dl"),
      &secrets,
      &network,
      "https://x.com/user/status/1",
    )
    .unwrap();
    assert!(!runner.args.iter().any(|arg| [
      "private-user",
      "private-password",
      "proxy-user",
      "proxy-password"
    ]
    .iter()
    .any(|secret| arg.contains(secret))));
    let config =
      PathBuf::from(&runner.args[runner.args.iter().position(|a| a == "-c").unwrap() + 1]);
    let value: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["extractor"]["twitter"]["username"], "private-user");
    assert_eq!(
      value["extractor"]["twitter"]["password"],
      "private-password"
    );
    assert!(value["extractor"].get("username").is_none());
    assert!(value["extractor"].get("headers").is_none());
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
      );
    }
    drop(runner);
    assert!(!config.exists());
    let unknown = configuration(
      &secrets,
      &NetworkSettings::default(),
      "https://cdn.example/photo.jpg",
    )
    .unwrap();
    assert!(!unknown.to_string().contains("private-password"));
  }

  #[test]
  fn custom_authorization_headers_are_rejected_before_gallery_can_forward_them() {
    for secrets in [
      AuthSecrets {
        bearer_token: Some("private-token".into()),
        ..Default::default()
      },
      AuthSecrets {
        headers: vec!["X-API-Key: private-key".into()],
        ..Default::default()
      },
    ] {
      assert_eq!(
        configuration(
          &secrets,
          &NetworkSettings::default(),
          "https://x.com/user/status/1"
        )
        .unwrap_err(),
        GalleryError::Refused("imageHeaderAuthUnavailable")
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
      include_bytes!("../../../src/assets/placeholder.png"),
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
      include_bytes!("../../../src/assets/placeholder.png")
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
        include_bytes!("../../../src/assets/placeholder.png")
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

  #[test]
  fn image_naming_preserves_resolved_folders_and_original_carousel_position() {
    let folder = std::env::temp_dir().join("Nezzel/صور");
    let path = image_destination(folder.join("photo.custom"), 3, true, "jpg").unwrap();
    assert_eq!(path, folder.join("photo-003.jpg"));
    assert!(image_destination(PathBuf::from("relative.jpg"), 1, false, "jpg").is_err());
    for stem in [
      "CON",
      "PrN",
      "AUX",
      "nul",
      "COM1",
      "com9",
      "LPT1",
      "lpt9",
      "CON.photo",
    ] {
      assert_eq!(
        image_destination(folder.join(format!("{stem}.old")), 1, false, "jpg").unwrap(),
        folder.join(format!("_{stem}.jpg"))
      );
    }
    for stem in ["COM0", "COM10", "LPT0", "CONSOLE"] {
      assert_eq!(
        image_destination(folder.join(format!("{stem}.old")), 1, false, "jpg").unwrap(),
        folder.join(format!("{stem}.jpg"))
      );
    }
  }

  #[test]
  fn browser_cookie_access_requires_existing_consent_and_proxy_overrides_are_explicit() {
    let auth = AuthSettings::default();
    let secrets = AuthSecrets::default();
    let mut network = NetworkSettings::default();
    let anonymous = crate::runners::ytdlp_runner::cookie_source_args(&auth, None);
    assert!(!anonymous.iter().any(|arg| arg == "--cookies-from-browser"));
    let accepted =
      crate::runners::ytdlp_runner::cookie_source_args(&auth, Some("chrome:/tmp/Profile 1"));
    assert!(accepted
      .windows(2)
      .any(|args| args == ["--cookies-from-browser", "chrome:/tmp/Profile 1"]));
    let configured = AuthSettings {
      cookie_browser: "chrome".into(),
      ..Default::default()
    };
    assert!(configured.site_browsers.is_empty());
    assert!(
      crate::runners::ytdlp_runner::cookie_source_args(&configured, None)
        .windows(2)
        .any(|args| args == ["--cookies-from-browser", "chrome"])
    );
    network.enable_proxy = Some(false);
    assert_eq!(
      configuration(&secrets, &network, "https://x.com/user/status/1").unwrap()["extractor"]
        ["proxy-env"],
      false
    );
    network.enable_proxy = Some(true);
    network.proxy = Some("http://proxy.invalid:8080".into());
    assert_eq!(
      configuration(&secrets, &network, "https://x.com/user/status/1").unwrap()["extractor"]
        ["proxy"],
      "http://proxy.invalid:8080"
    );
    let anonymous = GalleryRunner::anonymous(
      PathBuf::from("gallery-dl"),
      &network,
      "https://x.com/user/status/1",
    )
    .unwrap();
    assert!(anonymous.args.iter().any(|arg| arg == "--config-ignore"));
    assert!(!anonymous
      .args
      .iter()
      .any(|arg| arg.starts_with("--cookies")));
    let config = &anonymous.args[anonymous.args.iter().position(|arg| arg == "-c").unwrap() + 1];
    let settings: Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    assert_eq!(
      settings,
      json!({"extractor": {"proxy": "http://proxy.invalid:8080", "twitter": {}}})
    );
    network.proxy = Some("  ".into());
    assert_eq!(
      GalleryRunner::anonymous(
        PathBuf::from("must-not-run"),
        &network,
        "https://x.com/user/status/1",
      )
      .err(),
      Some(GalleryError::Refused("unableToConnectToProxy"))
    );
  }
}
