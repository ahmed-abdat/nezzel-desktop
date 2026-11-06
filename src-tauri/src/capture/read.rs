//! Reading a link for its card: the engine chain (file shares, gallery, yt-dlp) and the read's result.

use super::preview::{self, FailReason};
use super::state::{CaptureState, Offer};
use super::window::{wake_window, CaptureWindow};
use crate::models::parsed::FetchEngine;
use crate::SharedConfig;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub(super) async fn preview(app: AppHandle, window: CaptureWindow, value: Offer) {
  let started = std::time::Instant::now();
  let (item, details, reason) = match preview_item(&app, &value).await {
    Ok((item, details)) => (Some(item), details, None),
    Err(reason) => (None, None, Some(reason)),
  };
  tracing::info!(
    window = window.label(),
    ms = started.elapsed().as_millis() as u64,
    ok = item.is_some(),
    reason = reason.map(FailReason::as_str).unwrap_or(""),
    "Capture preview read"
  );
  let state = app.state::<CaptureState>();
  let recorded = state
    .offers
    .lock()
    .unwrap()
    .record_read(window, value.id, item, details, reason);
  if recorded {
    wake_window(&app, window);
  }
  state.changed.notify_waiters();
}

/// The link's media item (plus the card facts when they come from a richer source than the item),
/// or why it could not be read (a fixed reason word, never engine text).
/// One engine's read of a copied link (files, reader, yt-dlp and gallery each get this long). The card stays up
/// untouched for at most one read plus 5 s to show its result (`CARD_LIMIT`); a Download clicked while the card
/// reads waits two reads plus 6 s for the row's item (`ITEM_WAIT`: a second engine after a failed first), then
/// the main window reads the link itself.
const READ_SECS: u64 = 20;
pub(super) const READ_LIMIT: Duration = Duration::from_secs(READ_SECS);
pub(super) const CARD_LIMIT: Duration = Duration::from_secs(READ_SECS + 5);
pub(super) const ITEM_WAIT: Duration = Duration::from_secs(2 * READ_SECS + 6);

/// An engine a preview can read a link with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
  Files,
  /// A site reader (`crate::resolve`): finds the post's media or the link yt-dlp should read.
  Reader,
  Ytdlp,
  Gallery,
}

/// Which engines a preview tries, in order. Routing only, from the extract registry: a preview is anonymous and
/// silent (no login session, no group cancellation, no attempt reports or terminal events).
fn preview_plan(url: &str, proxy: bool) -> Vec<Engine> {
  if crate::files::hosts::is_share(url) {
    // File-share failures stay with Files; never yt-dlp for their preview.
    return vec![Engine::Files];
  }
  let gallery_first = crate::extract::registry::gallery_first(url);
  let mut plan = Vec::new();
  if gallery_first {
    plan.push(Engine::Gallery);
  }
  // Readers fetch public pages without the proxy: with one switched on they are skipped, never bypassed.
  if !proxy && !crate::extract::registry::readers(url).is_empty() {
    plan.push(Engine::Reader);
  }
  plan.push(Engine::Ytdlp);
  if !gallery_first {
    plan.push(Engine::Gallery);
  }
  plan
}

async fn preview_item(
  app: &AppHandle,
  value: &Offer,
) -> Result<(serde_json::Value, Option<preview::PreviewDetails>), FailReason> {
  let network = app.state::<SharedConfig>().load().network.clone();
  let proxy = network.enable_proxy == Some(true);
  // Check before any engine: a configured but empty proxy must never go direct.
  if proxy && network.proxy.as_deref().is_none_or(|p| p.trim().is_empty()) {
    return Err(FailReason::Network);
  }
  let row = format!("capture-{}", value.id);
  // What yt-dlp reads: the copied link, or the one a reader found for it.
  let mut target = value.url.clone();
  // The first failure is the one the card explains; later engines are only fallbacks.
  let mut failed: Option<FailReason> = None;
  for engine in preview_plan(&value.url, proxy) {
    match engine {
      Engine::Files => {
        if proxy {
          return Err(FailReason::Unknown);
        }
        return tokio::time::timeout(
          READ_LIMIT,
          crate::files::runner::preview(&value.url, row.clone()),
        )
        .await
        .map_err(|_| FailReason::Slow)?
        .map_err(|_| FailReason::Unknown)
        .map(|item| (with_engine(item, FetchEngine::Files), None));
      }
      Engine::Gallery => {
        match crate::runners::gallery::preview_media(app, &value.url, row.clone(), &network).await {
          Ok(Some(item)) => return Ok((with_engine(item, FetchEngine::Gallery), None)),
          // A login wall leaves the media type unknown. The foreground fetch must resolve it with
          // consented cookies instead of caching a yt-dlp preview.
          Err(crate::parsers::gallery::GalleryError::Login) if failed.is_none() => {
            return Err(FailReason::Login)
          }
          _ => {}
        }
      }
      Engine::Reader => match crate::resolve::resolve(&value.url).await {
        crate::resolve::Resolution::Passthrough(url)
        | crate::resolve::Resolution::PostMedia { url, .. } => target = url,
        crate::resolve::Resolution::Attachments { metadata } => {
          return reader_item(&metadata, &row)
            .map(|item| (item, None))
            .ok_or(FailReason::Unknown);
        }
        crate::resolve::Resolution::Unsupported { .. } => {
          failed.get_or_insert(FailReason::Unsupported);
        }
      },
      // A reader that found no video (photos only) leaves yt-dlp nothing to read.
      Engine::Ytdlp if failed.is_some() => {}
      Engine::Ytdlp => match preview_ytdlp(app, value, &target, &network).await {
        Ok((item, details)) => return Ok((with_engine(item, FetchEngine::Ytdlp), details)),
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
  target: &str,
  network: &crate::state::config_models::NetworkSettings,
) -> Result<(serde_json::Value, Option<preview::PreviewDetails>), FailReason> {
  let process_guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
  let manager = crate::binaries::binaries_manager::BinariesManager::for_app(app);
  let path = manager
    .tool_environment()
    .map_err(|_| FailReason::Unknown)?;
  let executable = manager
    .tool_path("yt-dlp")
    .map_err(|_| FailReason::Unknown)?;
  let mut command = preview_command(&executable, network, target).ok_or(FailReason::Unknown)?;
  command.env("PATH", path);
  let bytes = preview::read_output_reason(&mut command, process_guard).await?;
  let json = String::from_utf8(bytes).map_err(|_| FailReason::Unknown)?;
  ytdlp_preview(&value.url, &json, value.id)
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
  let parsed = crate::parsers::ytdlp_info::parse_ytdlp_info(json, format!("capture-{id}"))
    .map_err(|_| FailReason::Unknown)?;
  match parsed {
    crate::models::ParsedMedia::Single(item) => {
      let item = serde_json::to_value(item).map_err(|_| FailReason::Unknown)?;
      let from_item = preview::summarize(&item);
      let details = raw.map(|raw| {
        let mut details = preview::summarize(&raw);
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
  url: &str,
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
  command.args(["--", url]);
  Some(command)
}

#[cfg(test)]
mod login_privacy_tests {
  #[test]
  fn capture_command_never_requests_browser_or_file_cookies() {
    let command = super::preview_command(
      std::path::Path::new("yt-dlp"),
      &crate::state::config_models::NetworkSettings::default(),
      "https://example.com/post",
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
    assert_eq!(
      item["title"], "Me at the zoo",
      "the stored item stays the parsed model"
    );
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
        vec![Gallery, Reader, Ytdlp],
      ),
      ("https://drive.google.com/file/d/abc123/view", vec![Files]),
    ] {
      assert_eq!(preview_plan(url, false), plan, "{url}");
    }
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
        "https://example.com/v",
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
  }

  #[test]
  fn capture_previews_record_their_engine_but_playlists_keep_kind_routing() {
    let single = with_engine(serde_json::json!({ "id": "a" }), FetchEngine::Gallery);
    assert_eq!(single["fetchEngine"], "gallery");
    let playlist = with_engine(serde_json::json!({ "entries": [] }), FetchEngine::Ytdlp);
    assert!(playlist.get("fetchEngine").is_none());
  }
}
