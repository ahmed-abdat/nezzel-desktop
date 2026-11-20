//! Reading a link for its card: the engine chain (file shares, gallery, yt-dlp) and the read's result.

use super::preview::{self, FailReason};
use super::state::{CaptureState, Offer};
use super::window::{wake_window, CaptureWindow};
use crate::models::parsed::FetchEngine;
use crate::SharedConfig;
use std::time::Duration;
use tauri::{AppHandle, Manager};

type PreviewItem = (serde_json::Value, Option<preview::PreviewDetails>);

pub(super) async fn preview(app: AppHandle, window: CaptureWindow, value: Offer) {
  let started = std::time::Instant::now();
  let result = preview_item(&app, &value).await;
  tracing::info!(
    window = window.label(),
    ms = started.elapsed().as_millis() as u64,
    ok = result.as_ref().is_ok_and(|item| item.is_some()),
    reason = result
      .as_ref()
      .err()
      .map(|reason| reason.as_str())
      .unwrap_or(""),
    "Capture preview read"
  );
  let state = app.state::<CaptureState>();
  let recorded = record_preview(&mut state.offers.lock().unwrap(), window, value.id, result);
  if recorded {
    wake_window(&app, window);
  }
  state.changed.notify_waiters();
}

fn record_preview(
  offers: &mut super::state::Offers,
  window: CaptureWindow,
  id: u64,
  result: Result<Option<PreviewItem>, FailReason>,
) -> bool {
  match result {
    Ok(Some((item, details))) => offers.record_read(window, id, Some(item), details, None),
    Ok(None) => {
      let held = offers.holds(window, id);
      offers.dismiss(window, id);
      held
    }
    Err(reason) => offers.record_read(window, id, None, None, Some(reason)),
  }
}

/// The link's media item (plus the card facts when they come from a richer source than the item),
/// or why it could not be read (a fixed reason word, never engine text).
/// One engine's read of a copied link (files, reader, yt-dlp and gallery each get this long). The card stays up
/// untouched for at most one read plus 5 s to show its result; a Download clicked while the card
/// reads waits two reads plus 6 s for the row's item (a second engine after a failed first), then
/// the main window reads the link itself.
const READ_SECS: u64 = 20;
pub(super) const READ_LIMIT: Duration = Duration::from_secs(READ_SECS);
const COLD_READ_SECS: u64 = 60;
pub(super) fn preview_read_limit(
  url: &str,
  cold_spent: &std::sync::atomic::AtomicBool,
) -> Duration {
  // Reserve the first actual read before spawning it: a hang/cancel cannot renew the allowance.
  if !url.is_empty() && !cold_spent.swap(true, std::sync::atomic::Ordering::Relaxed) {
    Duration::from_secs(COLD_READ_SECS)
  } else {
    READ_LIMIT
  }
}

/// An engine a preview can read a link with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
  Files,
  /// A site reader (`crate::resolve`): finds the post's media or the link yt-dlp should read.
  Reader,
  Ytdlp,
  Gallery,
}

/// Capture admission, then engines in registry order. Previews are anonymous and silent:
/// no login session, group cancellation, attempt reports or terminal events.
fn preview_plan(url: &str, proxy: bool) -> Vec<Engine> {
  // A resolved short link is still clipboard input: re-apply capture's site/path policy.
  let Some((_, host)) = super::policy::supported_url(url) else {
    return Vec::new();
  };
  // A proxy (or a non-redirecting shortener) can leave the destination unknown. Only
  // site-owned shorteners may be handed to a background engine unresolved.
  if crate::sites::site_of(&host).is_none()
    && reqwest::Url::parse(url).is_ok_and(|url| crate::links::normalize::needs_redirect(&url))
  {
    return Vec::new();
  }
  if crate::files::hosts::is_share(url) {
    // File-share failures stay with Files; never yt-dlp for their preview.
    return vec![Engine::Files];
  }
  let gallery_first = crate::extract::registry::gallery_first(url);
  let readers = crate::extract::registry::readers(url);
  let facebook = readers.contains(&crate::routing::ReaderId::Facebook);
  let mut plan = Vec::new();
  if gallery_first && !facebook {
    plan.push(Engine::Gallery);
  }
  // Readers fetch public pages without the proxy: never bypass it.
  if !proxy && !readers.is_empty() {
    plan.push(Engine::Reader);
  }
  if gallery_first && facebook {
    plan.push(Engine::Gallery);
  }
  plan.push(Engine::Ytdlp);
  if !gallery_first {
    plan.push(Engine::Gallery);
  }
  plan
}

async fn preview_item(app: &AppHandle, value: &Offer) -> Result<Option<PreviewItem>, FailReason> {
  let network = app.state::<SharedConfig>().load().network.clone();
  let proxy = network.enable_proxy == Some(true);
  // Check before any engine: a configured but empty proxy must never go direct.
  if proxy && network.proxy.as_deref().is_none_or(|p| p.trim().is_empty()) {
    return Err(FailReason::Network);
  }
  let mut normalized = value.clone();
  normalized.url = crate::links::normalize::resolve(&value.url, proxy)
    .await
    .map_err(|_| FailReason::Network)?
    .into();
  let value = &normalized;
  let plan = preview_plan(&value.url, proxy);
  if plan.is_empty() {
    return Ok(None);
  }
  let row = format!("capture-{}", value.id);
  // What yt-dlp reads: the copied link, or the one a reader found for it.
  let mut target = crate::resolve::ResolvedInput::new(&value.url);
  // The first failure is the one the card explains; later engines are only fallbacks.
  let mut failed: Option<FailReason> = None;
  // The reader's post title; the media file name it hands yt-dlp is not one.
  let mut post_title: Option<String> = None;
  for engine in plan {
    match engine {
      Engine::Files => {
        if proxy {
          return Err(FailReason::Unknown);
        }
        return tokio::time::timeout(
          value.read_limit(),
          crate::files::runner::preview(&value.url, row.clone()),
        )
        .await
        .map_err(|_| FailReason::Slow)?
        .map_err(|_| FailReason::Unknown)
        .map(|item| Some((with_engine(item, FetchEngine::Files), None)));
      }
      Engine::Gallery => {
        match crate::runners::gallery::preview_media(
          app,
          &target.url,
          row.clone(),
          &network,
          value.read_limit(),
        )
        .await
        {
          Ok(Some(item)) => return Ok(Some((with_engine(item, FetchEngine::Gallery), None))),
          // A login wall leaves the media type unknown. The foreground fetch must resolve it with
          // consented cookies instead of caching a yt-dlp preview.
          Err(crate::parsers::gallery::GalleryError::Login) if failed.is_none() => {
            return Err(FailReason::Login)
          }
          _ => {}
        }
      }
      Engine::Reader => {
        match crate::resolve::resolve_with_limit(&value.url, value.read_limit()).await {
          crate::resolve::Resolution::Passthrough(url) => target.rewrite(url),
          crate::resolve::Resolution::PostMedia { url, title, .. } => {
            post_title = title;
            target.rewrite(url);
          }
          crate::resolve::Resolution::Attachments { metadata } => {
            return reader_item(&metadata, &row)
              .map(|item| Some((item, None)))
              .ok_or(FailReason::Unknown);
          }
          crate::resolve::Resolution::Failed { .. } => return Err(FailReason::Network),
          crate::resolve::Resolution::Unsupported {
            reason: crate::resolve::Reason::Login,
          } => return Err(FailReason::Login),
          crate::resolve::Resolution::Unsupported {
            reason: crate::resolve::Reason::Unavailable,
          } => return Err(FailReason::Unknown),
          crate::resolve::Resolution::Unsupported { .. } => {
            failed.get_or_insert(FailReason::Unsupported);
          }
        }
      }
      // A reader that found no video (photos only) leaves yt-dlp nothing to read.
      Engine::Ytdlp if failed.is_some() || crate::resolve::facebook_photo_target(&target.url) => {}
      Engine::Ytdlp => match preview_ytdlp(app, value, &target, &network).await {
        Ok((item, details)) => {
          let (item, details) = with_post_title(item, details, post_title);
          return Ok(Some((with_engine(item, FetchEngine::Ytdlp), details)));
        }
        Err(reason) => {
          failed.get_or_insert(reason);
        }
      },
    }
  }
  Err(failed.unwrap_or(FailReason::Unknown))
}

/// A post's attachments read by its site reader: the app's item, recorded as a reader fetch.
fn reader_item(metadata: &str, row: &str) -> Option<serde_json::Value> {
  let mut media = serde_json::from_str::<crate::models::ParsedSingleVideo>(metadata).ok()?;
  media.id = row.to_string();
  media.fetch_engine = Some(FetchEngine::Reader);
  serde_json::to_value(media).ok()
}

/// A post title the reader found replaces the one yt-dlp took from the media file name, in the
/// cached item and in the card details (which the offer title prefers).
fn with_post_title(
  mut item: serde_json::Value,
  mut details: Option<preview::PreviewDetails>,
  title: Option<String>,
) -> (serde_json::Value, Option<preview::PreviewDetails>) {
  if let (Some(title), Some(fields)) = (title, item.as_object_mut()) {
    if !fields.contains_key("entries") {
      fields.insert("title".into(), title.clone().into());
      if let Some(details) = details.as_mut() {
        details.title = Some(title);
      }
    }
  }
  (item, details)
}

/// Capture previews skip the fetch chain, so they record their engine here; the download
/// then runs through the engine that read the link (#28). Playlists keep kind routing.
fn with_engine(mut item: serde_json::Value, engine: FetchEngine) -> serde_json::Value {
  if let Some(fields) = item.as_object_mut() {
    if !fields.contains_key("entries") {
      if let Ok(value) = serde_json::to_value(engine) {
        fields.entry("fetchEngine").or_insert(value);
      }
    }
  }
  item
}

async fn preview_ytdlp(
  app: &AppHandle,
  value: &Offer,
  target: &crate::resolve::ResolvedInput,
  network: &crate::state::config_models::NetworkSettings,
) -> Result<(serde_json::Value, Option<preview::PreviewDetails>), FailReason> {
  let manager = crate::binaries::binaries_manager::BinariesManager::for_app(app);
  let path = manager
    .tool_environment()
    .map_err(|_| FailReason::Unknown)?;
  let executable = manager
    .tool_path("yt-dlp")
    .map_err(|_| FailReason::Unknown)?;
  let ffmpeg_args = manager.ffmpeg_location_args();
  let mut input = target.clone();
  input.rewrite(target.url.clone());
  let mut first_vimeo_reason = None;
  let started = std::time::Instant::now();
  let limit = value.read_limit();
  let deadline = tokio::time::Instant::now() + limit;
  let bytes = loop {
    // URL retries and the fast/full pair all consume the same preview budget.
    match tokio::time::timeout_at(deadline, async {
      let read = |fast| {
        let input = &input;
        let executable = &executable;
        let path = &path;
        let ffmpeg_args = &ffmpeg_args;
        async move {
          let process_guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
          let mut command = preview_command(executable, network, input, fast, ffmpeg_args)
            .ok_or((FailReason::Unknown, String::new()))?;
          command.env("PATH", path);
          preview::read_output_stderr_with_limit(&mut command, process_guard, limit).await
        }
      };
      if reqwest::Url::parse(&input.url)
        .ok()
        .and_then(|url| url.host_str().and_then(crate::sites::site_of))
        == Some(crate::sites::SiteId::YouTube)
      {
        probe_with_fallback(read(true), || read(false)).await
      } else {
        read(false).await
      }
    })
    .await
    .map_err(|_| FailReason::Slow)?
    {
      Ok(bytes) => break bytes,
      Err((reason, stderr)) => {
        if tokio::time::timeout_at(deadline, input.retry_facebook_share(&stderr))
          .await
          .map_err(|_| FailReason::Slow)?
        {
          continue;
        }
        if input.impersonation_unavailable(&stderr) {
          return Err(first_vimeo_reason.unwrap_or(reason));
        }
        if input.retry_vimeo_401(&stderr) {
          first_vimeo_reason = Some(reason);
        } else if !input.retry_original(&stderr) {
          return Err(reason);
        }
      }
    }
  };
  tracing::info!(
    ms = started.elapsed().as_millis() as u64,
    "Capture yt-dlp probe"
  );
  let json = String::from_utf8(bytes).map_err(|_| FailReason::Unknown)?;
  ytdlp_preview(&value.url, &json, value.id)
}

async fn probe_with_fallback<
  T,
  F: std::future::Future<Output = Result<T, (FailReason, String)>>,
>(
  fast: impl std::future::Future<Output = Result<T, (FailReason, String)>>,
  full: impl FnOnce() -> F,
) -> Result<T, (FailReason, String)> {
  use crate::parsers::ytdlp_error::{stage_hint, DiagnosticMatcher, YtdlpErrorParser};

  match fast.await {
    Ok(value) => Ok(value),
    Err(error) => {
      let Ok(matcher) = DiagnosticMatcher::shared() else {
        return Err(error);
      };
      let parser = YtdlpErrorParser::new("capture", "capture", matcher);
      let diagnostics: Vec<_> = error
        .1
        .lines()
        .filter_map(|line| parser.parse_fetch_line(line, false))
        .collect();
      // Missing formats are terminal for photo fallback, but an abbreviated capture probe
      // can still find video by restoring manifests. Explicit site refusals remain terminal.
      let missing_formats_only = error.0 == FailReason::Unavailable
        && diagnostics
          .iter()
          .filter(|event| matches!(event.level, crate::models::error::DiagnosticLevel::Error))
          .all(|event| {
            let message = event.message.to_ascii_lowercase();
            event.code == "mediaUnavailable"
              && ["no video formats", "no formats", "no available formats"]
                .iter()
                .any(|prefix| message.starts_with(prefix))
          });
      if error.0 != FailReason::Unknown && !missing_formats_only {
        return Err(error);
      }
      let retry = diagnostics.iter().any(|event| {
        matches!(
          event.code.as_str(),
          "requestedFormatUnavailable"
            | "unableToExtractIdOrInfo"
            | "unableToExtractInitialData"
            | "nsigExtractionFailed"
        ) || stage_hint::no_formats(&event.message)
          || stage_hint::requested_format_unavailable(&event.message)
      });
      if retry {
        full().await
      } else {
        Err(error)
      }
    }
  }
}

/// yt-dlp's dump → the item the download uses (the app's parsed model) and the card facts. The facts
/// come from the raw dump: the parsed formats drop per-format codecs, sizes and extensions, so
/// summarizing them lost Audio and the sizes once the card had read the link (#36).
fn ytdlp_preview(
  url: &str,
  json: &str,
  id: u64,
) -> Result<(serde_json::Value, Option<preview::PreviewDetails>), FailReason> {
  let raw = serde_json::from_str::<serde_json::Value>(json).ok();
  if raw
    .as_ref()
    .is_some_and(|info| crate::parsers::gallery::tiktok_photo_candidate(url, info))
  {
    return Err(FailReason::Unsupported);
  }
  let parsed =
    crate::parsers::ytdlp_info::parse_ytdlp_info_for_url(json, format!("capture-{id}"), Some(url))
      .map_err(|_| FailReason::Unknown)?;
  match parsed {
    crate::models::ParsedMedia::Single(item) => {
      let item = serde_json::to_value(item).map_err(|_| FailReason::Unknown)?;
      let from_item = preview::summarize(&item);
      let details = raw.map(|raw| {
        let mut details = preview::summarize(&raw);
        details.title = item["title"]
          .as_str()
          .filter(|title| raw["title"].as_str() != Some(*title))
          .map(str::to_owned)
          .or_else(|| Some(crate::parsers::ytdlp_info::caption_title(&raw)));
        details.media_kind = from_item.media_kind.clone();
        details.photos = from_item.photos;
        details.thumbnail = details.thumbnail.or(from_item.thumbnail.clone());
        if details.uploader.is_empty() {
          details.uploader = from_item.uploader.clone();
        }
        details
      });
      Ok((item, details))
    }
    crate::models::ParsedMedia::Playlist(item) => serde_json::to_value(item)
      .map(|item| (item, None))
      .map_err(|_| FailReason::Unknown),
    _ => Err(FailReason::Unsupported),
  }
}

fn preview_command(
  executable: &std::path::Path,
  network: &crate::state::config_models::NetworkSettings,
  input: &crate::resolve::ResolvedInput,
  fast: bool,
  ffmpeg_args: &[std::ffi::OsString],
) -> Option<tokio::process::Command> {
  // Background previews never load cookies, user yt-dlp config, or engine diagnostics.
  let mut command = crate::runners::tool_process::async_command(executable).ok()?;
  command.args([
    "--ignore-config",
    "--no-cache-dir",
    "--quiet",
    "--no-warnings",
    "--socket-timeout",
    "15",
    "--retries",
    "0",
    "--extractor-retries",
    "0",
    "--no-playlist",
    "--flat-playlist",
    "--skip-download",
    "--dump-single-json",
  ]);
  // Proxy switched on but empty: skip the background fetch rather than go direct.
  if network.enable_proxy == Some(true) && network.proxy.as_deref().is_none_or(str::is_empty) {
    return None;
  }
  // Same network route as real downloads: never bypass the user's proxy.
  command.args(crate::runners::ytdlp_runner::route_args(network));
  if fast {
    command.args(["--extractor-args", "youtube:skip=hls,dash,translated_subs"]);
  }
  command.args(ffmpeg_args).args(input.args());
  Some(command)
}

#[cfg(test)]
mod login_privacy_tests {
  #[test]
  fn capture_command_never_requests_browser_or_file_cookies() {
    let command = super::preview_command(
      std::path::Path::new("yt-dlp"),
      &crate::state::config_models::NetworkSettings::default(),
      &crate::resolve::ResolvedInput::new("https://example.com/post"),
      false,
      &[],
    )
    .unwrap();
    let args: Vec<_> = command
      .as_std()
      .get_args()
      .map(|v| v.to_str().unwrap())
      .collect();
    assert!(args.contains(&"--ignore-config"));
    assert!(!args
      .iter()
      .any(|v| v.starts_with("--cookies")
        || matches!(*v, "--username" | "--password" | "--add-header")));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::capture::state::Offers;

  #[test]
  fn reader_post_title_reaches_the_offer_card_not_the_media_file_name() {
    let dump = serde_json::json!({
      "id": "x", "title": "DASHPlaylist", "extractor": "generic",
      "webpage_url": "https://v.redd.it/abcdefgh1234/DASHPlaylist.mpd",
      "formats": [{"format_id": "1", "ext": "mp4", "acodec": "none", "vcodec": "avc1", "height": 720}]
    });
    let (item, details) = ytdlp_preview(
      "https://v.redd.it/abcdefgh1234/DASHPlaylist.mpd",
      &dump.to_string(),
      1,
    )
    .unwrap();
    let (item, details) = with_post_title(item, details, Some("NASA clip".into()));
    let mut offers = Offers::default();
    offers.replace(
      CaptureWindow::Bar,
      Offer {
        id: 1,
        ..Offer::default()
      },
    );
    assert!(offers.record_read(CaptureWindow::Bar, 1, Some(item), details, None));
    let offer = offers.current(CaptureWindow::Bar).unwrap();
    assert_eq!(offer.title, "NASA clip");
    assert_eq!(offer.details.title.as_deref(), Some("NASA clip"));
  }

  #[test]
  fn first_read_gets_cold_budget_for_any_host() {
    for (url, engine) in [
      ("https://www.youtube.com/watch?v=abc", Engine::Ytdlp),
      ("https://www.instagram.com/p/abc/", Engine::Gallery),
      ("https://facebook.com/nasa/posts/123", Engine::Reader),
      ("https://drive.google.com/file/d/abc123/view", Engine::Files),
    ] {
      assert_eq!(preview_plan(url, false).first(), Some(&engine));
      let state = CaptureState::default();
      assert_eq!(
        preview_read_limit(url, &state.cold_read_spent),
        Duration::from_secs(60)
      );
    }
  }

  #[test]
  fn second_read_gets_normal_budget_even_on_another_host() {
    let state = CaptureState::default();
    preview_read_limit("https://vimeo.com/123", &state.cold_read_spent);
    assert_eq!(
      preview_read_limit(
        "https://www.youtube.com/watch?v=abc",
        &state.cold_read_spent
      ),
      READ_LIMIT
    );
  }

  #[test]
  fn empty_offer_does_not_spend_first_read_budget() {
    let state = CaptureState::default();
    preview_read_limit("", &state.cold_read_spent);
    assert_eq!(
      preview_read_limit("https://vimeo.com/123", &state.cold_read_spent),
      Duration::from_secs(60)
    );
  }

  #[test]
  fn tool_replacement_rearms_one_cold_read() {
    let state = CaptureState::default();
    let url = "https://vimeo.com/123";
    preview_read_limit(url, &state.cold_read_spent);
    state.tools_changed();
    assert_eq!(
      [
        preview_read_limit(url, &state.cold_read_spent),
        preview_read_limit(url, &state.cold_read_spent)
      ],
      [Duration::from_secs(60), READ_LIMIT]
    );
  }

  #[tokio::test(start_paused = true)]
  async fn hanging_first_read_for_any_engine_times_out_at_sixty_seconds() {
    for url in [
      "https://www.youtube.com/watch?v=abc",
      "https://www.instagram.com/p/abc/",
      "https://facebook.com/nasa/posts/123",
      "https://drive.google.com/file/d/abc123/view",
    ] {
      let state = CaptureState::default();
      let offer = Offer {
        read_limit: preview_read_limit(url, &state.cold_read_spent),
        ..Offer::default()
      };
      let read = tokio::time::timeout(offer.read_limit(), std::future::pending::<()>());
      tokio::pin!(read);
      assert!(futures::poll!(&mut read).is_pending());
      tokio::time::advance(Duration::from_secs(26)).await;
      assert!(futures::poll!(&mut read).is_pending(), "{url}");
      tokio::time::advance(Duration::from_secs(34)).await;
      assert!(
        matches!(futures::poll!(&mut read), std::task::Poll::Ready(Err(_))),
        "{url}"
      );
    }
  }

  #[test]
  fn card_title_prefers_captions_only_over_generated_titles() {
    for (title, caption, expected) in [
      (
        "Video by nasa",
        "  The weekend ends. The mission keeps going.  \n\nMore",
        "The weekend ends. The mission keeps going.",
      ),
      (
        "Video by nasa",
        "\n  رحلة إلى القمر 🚀  \nتفاصيل",
        "رحلة إلى القمر 🚀",
      ),
      ("Post by nasa", "   ", "Post by nasa"),
      ("Video 1", "First caption", "First caption"),
      ("Me at the zoo", "Unrelated description", "Me at the zoo"),
      (
        "Video by Wes Anderson analysis",
        "Description",
        "Video by Wes Anderson analysis",
      ),
      ("", "Caption without title", "Caption without title"),
      (
        "Video by nasa",
        "#space @nasa\n  A real caption  ",
        "A real caption",
      ),
      (
        "Video by nasa",
        "\u{200e}\u{200f}\u{061c}\u{200b} \nرحلة إلى القمر",
        "رحلة إلى القمر",
      ),
      (
        "Video by nasa",
        "\u{200f} #فضاء \u{061c} @nasa\n\u{200e} A real caption \u{200f}",
        "A real caption",
      ),
      (
        "Video by nasa",
        "#space @nasa\n\u{200e}\u{200f}\u{061c}\u{200b} ",
        "Video by nasa",
      ),
      (
        "Video by nasa",
        "Launch day #space @nasa",
        "Launch day #space @nasa",
      ),
    ] {
      assert_eq!(
        crate::parsers::ytdlp_info::caption_title(
          &serde_json::json!({"title":title,"description":caption,"channel":"nasa"})
        ),
        expected
      );
    }
    assert_eq!(crate::parsers::ytdlp_info::caption_title(&serde_json::json!({"title":"Video by nasa","channel":"nasa","description":"ع🚀".repeat(200)})).chars().count(), 300);
  }

  #[tokio::test]
  async fn manifest_fallback_keeps_success_or_returns_the_full_probe_failure() {
    use std::cell::Cell;
    use std::future::ready;
    let full_called = Cell::new(false);
    assert_eq!(
      probe_with_fallback(ready(Ok::<_, (FailReason, String)>(1)), || {
        full_called.set(true);
        ready(Ok(2))
      })
      .await,
      Ok(1)
    );
    assert!(!full_called.get());
    for (stderr, retry) in [
      ("ERROR: [youtube] Requested format is not available. Use --list-formats for a list of available formats", true),
      ("ERROR: [youtube] No video formats found!", true),
      ("ERROR: [youtube] Unable to extract yt initial data", true),
      ("ERROR: [youtube] Unable to extract video ID", true),
      ("ERROR: [youtube] Sign in to confirm your age", false),
      ("ERROR: [youtube] Private video. Sign in if you've been granted access to this video", false),
      ("ERROR: [youtube] This video is not available in your country", false),
      ("ERROR: [youtube] This clip is no longer available", false),
      ("ERROR: [youtube] HTTP Error 429: Too Many Requests", false),
      ("ERROR: [youtube] DRM protected", false),
      ("ERROR: Unable to connect to proxy", false),
      ("ERROR: unrecognised failure", false),
    ] {
      full_called.set(false);
      let error = (preview::fail_reason(stderr), stderr.to_string());
      let result = probe_with_fallback(ready(Err(error.clone())), || {
        full_called.set(true);
        ready(Ok(2))
      }).await;
      assert_eq!(result, if retry { Ok(2) } else { Err(error) }, "{stderr}");
      assert_eq!(full_called.get(), retry, "{stderr}");
    }
    let full_error = (
      FailReason::Login,
      "HTTP Error 401: Unauthorized".to_string(),
    );
    assert_eq!(
      probe_with_fallback(
        ready(Err::<(), _>((
          FailReason::Unknown,
          "ERROR: No video formats found".into()
        ))),
        || ready(Err(full_error.clone()))
      )
      .await,
      Err(full_error)
    );
  }

  /// The card's facts after a yt-dlp read keep Audio, Smaller and sizes (#36: the row lost Audio).
  #[test]
  fn ytdlp_preview_keeps_audio_and_sizes_after_the_read() {
    let dump = serde_json::json!({
      "id": "jNQXAC9IVRw", "title": "Me at the zoo", "extractor": "youtube",
      "webpage_url": "https://www.youtube.com/watch?v=jNQXAC9IVRw",
      "uploader": "jawed", "view_count": 300, "duration": 19,
      "thumbnail": "https://i.ytimg.com/vi/jNQXAC9IVRw/hq.jpg",
      "formats": [
        {"format_id": "139", "ext": "m4a", "acodec": "mp4a.40.5", "vcodec": "none", "abr": 48.0, "filesize": 100},
        {"format_id": "140", "ext": "m4a", "acodec": "mp4a.40.2", "vcodec": "none", "abr": 129.0, "filesize": 300},
        {"format_id": "160", "ext": "mp4", "acodec": "none", "vcodec": "avc1.4d400b", "height": 144, "filesize": 200},
        {"format_id": "133", "ext": "mp4", "acodec": "none", "vcodec": "avc1.4d400c", "height": 240, "filesize": 400},
        {"format_id": "18", "ext": "mp4", "acodec": "mp4a.40.2", "vcodec": "avc1.42001E", "height": 240, "filesize": 500}
      ]
    });
    let (item, details) = ytdlp_preview(
      "https://www.youtube.com/watch?v=jNQXAC9IVRw",
      &dump.to_string(),
      7,
    )
    .unwrap();
    let details = details.unwrap_or_else(|| preview::summarize(&item));
    assert!(details.has_audio, "Audio stays after the read");
    assert_eq!(details.audio_size, Some(300));
    assert_eq!(details.best_height, Some(240));
    assert_eq!(details.small_height, Some(144));
    assert_eq!(details.size, Some(500));
    assert_eq!(details.views, Some(300));
    assert_eq!(details.duration, Some(19));
    assert!(details.thumbnail.is_some());
    assert_eq!(
      item["title"], "Me at the zoo",
      "the stored item stays the parsed model"
    );
    let mut captioned = dump;
    captioned["title"] = "Video by nasa".into();
    captioned["channel"] = "nasa".into();
    captioned["description"] = "  Caption first line  \nMore".into();
    let (cached, details) = ytdlp_preview(
      "https://www.instagram.com/reel/abc/",
      &captioned.to_string(),
      8,
    )
    .unwrap();
    assert_eq!(
      details.unwrap().title.as_deref(),
      Some("Caption first line")
    );
    assert_eq!(
      cached["title"], "Caption first line",
      "caption titles also name the cached download"
    );
    captioned["description"] = serde_json::Value::Null;
    let (cached, details) = ytdlp_preview(
      "https://www.instagram.com/nasa/reel/abc/",
      &captioned.to_string(),
      9,
    )
    .unwrap();
    assert_eq!(details.unwrap().title.as_deref(), cached["title"].as_str());
  }

  #[test]
  fn dropped_preview_retires_only_its_offer_without_a_failed_card() {
    use super::super::state::{ItemState, Offers};
    let window = CaptureWindow::Card;
    let mut offers = Offers::default();
    offers.replace(
      window,
      Offer {
        id: 1,
        url: "https://bit.ly/a".into(),
        ..Offer::default()
      },
    );
    assert!(record_preview(&mut offers, window, 1, Ok(None)));
    assert_eq!(offers.item(1), ItemState::Gone);
    offers.replace(
      window,
      Offer {
        id: 2,
        ..Offer::default()
      },
    );
    assert!(!record_preview(&mut offers, window, 1, Ok(None)));
    assert_eq!(offers.current(window).unwrap().id, 2);
  }

  #[test]
  fn unsupported_shortener_destinations_have_no_background_engine() {
    for target in [
      "https://example.com/video.mp4",
      "https://t.me/+secret",
      "https://t.me/joinchat/secret",
      "https://t.me/c/123/4",
      "https://drive.google.com/drive/folders/abc",
      "https://docs.google.com/forms/d/abc/viewform",
      "https://dropbox.com/home",
    ] {
      assert!(preview_plan(target, false).is_empty(), "{target}");
    }
    for proxy in [false, true] {
      for unresolved in ["https://bit.ly/a", "https://t.co/a"] {
        assert!(preview_plan(unresolved, proxy).is_empty(), "{unresolved}");
      }
    }
    assert!(!preview_plan("https://youtu.be/abc", true).is_empty());
  }

  /// Routing comes from the extract registry: files alone, gallery before or after yt-dlp, site readers first.
  #[test]
  fn previews_try_the_engines_routing_names() {
    use Engine::*;
    for (url, plan) in [
      (
        "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        vec![Ytdlp, Gallery],
      ),
      ("https://www.instagram.com/p/abc/", vec![Gallery, Ytdlp]),
      ("https://www.instagram.com/reel/abc/", vec![Ytdlp, Gallery]),
      (
        "https://facebook.com/nasa/posts/123",
        vec![Reader, Gallery, Ytdlp],
      ),
      (
        "https://www.facebook.com/share/p/1EiNJaQ8Yr/",
        vec![Reader, Gallery, Ytdlp],
      ),
      (
        "https://www.facebook.com/share/p/18nsn8sxfB/",
        vec![Reader, Gallery, Ytdlp],
      ),
      (
        "https://www.facebook.com/photo/?fbid=1683838639778172",
        vec![Reader, Gallery, Ytdlp],
      ),
      ("https://drive.google.com/file/d/abc123/view", vec![Files]),
    ] {
      assert_eq!(preview_plan(url, false), plan, "{url}");
    }
    let mut input = crate::resolve::ResolvedInput::new("https://player.vimeo.com/video/1084537");
    input.rewrite(input.url.clone());
    assert!(input.retry_vimeo_401(
      "ERROR: [vimeo] 1084537: Unable to download webpage: HTTP Error 401: Unauthorized"
    ));
    let command = preview_command(
      std::path::Path::new("yt-dlp"),
      &Default::default(),
      &input,
      false,
      &[],
    )
    .unwrap();
    let args: Vec<_> = command
      .as_std()
      .get_args()
      .map(|v| v.to_str().unwrap())
      .collect();
    assert!(args
      .windows(2)
      .any(|pair| pair == ["--referer", "https://vimeo.com/"]));
    assert!(args
      .windows(2)
      .any(|pair| pair == ["--impersonate", "chrome"]));
    assert_eq!(
      &args[args.len() - 2..],
      ["--", "https://player.vimeo.com/video/1084537"]
    );
    // Readers fetch without the proxy: skipped when it is on, never bypassed.
    assert_eq!(
      preview_plan("https://facebook.com/nasa/posts/123", true),
      [Gallery, Ytdlp]
    );
  }

  #[test]
  fn reader_attachments_become_a_reader_item() {
    // A reader hands over the app's parsed model as JSON (`Resolution::Attachments`).
    let dump =
      serde_json::json!({"id": "post", "title": "Post", "extractor": "threads", "formats": []});
    let crate::models::ParsedMedia::Single(media) =
      crate::parsers::ytdlp_info::parse_ytdlp_info(&dump.to_string(), "reader".into()).unwrap()
    else {
      panic!("a single post");
    };
    let item = reader_item(&serde_json::to_string(&media).unwrap(), "capture-3").unwrap();
    assert_eq!(item["id"], "capture-3");
    assert_eq!(item["fetchEngine"], "reader");
    assert!(reader_item("not json", "capture-3").is_none());
  }

  /// Previews take the same proxy and impersonation route as downloads (`ytdlp_runner::route_args`).
  #[test]
  fn preview_command_takes_the_download_route() {
    let args = |network: &crate::state::config_models::NetworkSettings| {
      super::preview_command(
        std::path::Path::new("yt-dlp"),
        network,
        &crate::resolve::ResolvedInput::new("https://example.com/v"),
        false,
        &[],
      )
      .map(|command| {
        command
          .as_std()
          .get_args()
          .map(|v| v.to_string_lossy().into_owned())
          .collect::<Vec<_>>()
      })
    };
    let mut network = crate::state::config_models::NetworkSettings {
      enable_proxy: Some(true),
      proxy: Some("socks5://127.0.0.1:9050".into()),
      impersonate: "chrome".into(),
      ..Default::default()
    };
    let with_proxy = args(&network).unwrap();
    let route = crate::runners::ytdlp_runner::route_args(&network);
    assert!(with_proxy
      .windows(route.len())
      .any(|w| w == route.as_slice()));
    assert!(with_proxy.ends_with(&["--".into(), "https://example.com/v".into()]));
    network.proxy = Some(String::new());
    assert!(args(&network).is_none(), "an empty proxy never goes direct");

    let mut input = crate::resolve::ResolvedInput::new("https://example.com/post");
    input.rewrite("https://www.youtube.com/watch?v=jNQXAC9IVRw".into());
    let command = preview_command(
      std::path::Path::new("yt-dlp"),
      &Default::default(),
      &input,
      true,
      &crate::binaries::bundled_tools::ffmpeg_location_args(
        Some(std::path::Path::new(
          "install with spaces/tools/ffmpeg/ffmpeg.exe",
        )),
        true,
      ),
    )
    .unwrap();
    let args: Vec<_> = command
      .as_std()
      .get_args()
      .map(|v| v.to_str().unwrap())
      .collect();
    let flags = args
      .iter()
      .position(|arg| *arg == "--extractor-args")
      .unwrap();
    assert_eq!(args[flags + 1], "youtube:skip=hls,dash,translated_subs");
    assert!(
      args.ends_with(&input.args()),
      "fast probes retain the source referer and resolved target"
    );
    assert!(flags < args.iter().position(|arg| *arg == "--").unwrap());
    let location = args
      .iter()
      .position(|arg| *arg == "--ffmpeg-location")
      .unwrap();
    assert_eq!(args[location + 1], "install with spaces/tools/ffmpeg");
    assert!(location < args.iter().position(|arg| *arg == "--").unwrap());
  }

  #[test]
  fn capture_previews_record_their_engine_but_playlists_keep_kind_routing() {
    let single = with_engine(serde_json::json!({ "id": "a" }), FetchEngine::Gallery);
    assert_eq!(single["fetchEngine"], "gallery");
    let playlist = with_engine(serde_json::json!({ "entries": [] }), FetchEngine::Ytdlp);
    assert!(playlist.get("fetchEngine").is_none());
  }
}
