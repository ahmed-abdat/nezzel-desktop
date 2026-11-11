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
  let mut target = crate::resolve::ResolvedInput::new(&value.url);
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
        match crate::runners::gallery::preview_media(app, &target.url, row.clone(), &network).await
        {
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
        | crate::resolve::Resolution::PostMedia { url, .. } => target.rewrite(url),
        crate::resolve::Resolution::Attachments { metadata } => {
          return reader_item(&metadata, &row)
            .map(|item| (item, None))
            .ok_or(FailReason::Unknown);
        }
        crate::resolve::Resolution::Failed { .. } => return Err(FailReason::Network),
        crate::resolve::Resolution::Unsupported { .. } => {
          failed.get_or_insert(FailReason::Unsupported);
        }
      },
      // A reader that found no video (photos only) leaves yt-dlp nothing to read.
      Engine::Ytdlp if failed.is_some() || crate::resolve::facebook_photo_target(&target.url) => {}
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
  let mut input = target.clone();
  input.rewrite(target.url.clone());
  let mut first_vimeo_reason = None;
  let started = std::time::Instant::now();
  let deadline = tokio::time::Instant::now() + READ_LIMIT;
  let bytes = loop {
    // URL retries and the fast/full pair all consume the same preview budget.
    match tokio::time::timeout_at(deadline, async {
      let read = |fast| {
        let input = &input;
        let executable = &executable;
        let path = &path;
        async move {
          let process_guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
          let mut command = preview_command(executable, network, input, fast)
            .ok_or((FailReason::Unknown, String::new()))?;
          command.env("PATH", path);
          preview::read_output_stderr(&mut command, process_guard).await
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
      // Restoring manifests cannot repair login, availability, transport or access refusals.
      if error.0 != FailReason::Unknown {
        return Err(error);
      }
      let Ok(matcher) = DiagnosticMatcher::shared() else {
        return Err(error);
      };
      let parser = YtdlpErrorParser::new("capture", "capture", matcher);
      let retry = error
        .1
        .lines()
        .filter_map(|line| parser.parse_fetch_line(line, false))
        .any(|event| {
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
  let parsed = crate::parsers::ytdlp_info::parse_ytdlp_info(json, format!("capture-{id}"))
    .map_err(|_| FailReason::Unknown)?;
  match parsed {
    crate::models::ParsedMedia::Single(item) => {
      let item = serde_json::to_value(item).map_err(|_| FailReason::Unknown)?;
      let from_item = preview::summarize(&item);
      let details = raw.map(|raw| {
        let mut details = preview::summarize(&raw);
        details.title = Some(card_title(&raw));
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

/// yt-dlp's generated social title is a fallback; a post's caption is more useful on the card.
fn card_title(raw: &serde_json::Value) -> String {
  let title = raw["title"].as_str().unwrap_or("").trim();
  let generic = title.is_empty()
    || ["uploader", "uploader_id", "channel"].iter().any(|key| {
      raw[*key].as_str().is_some_and(|user| {
        title == format!("Video by {user}") || title == format!("Post by {user}")
      })
    })
    || title
      .strip_prefix("Video ")
      .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
  let noise =
    |c: char| c.is_whitespace() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{200b}');
  let caption = ["description", "caption"].iter().find_map(|key| {
    raw[*key]
      .as_str()?
      .lines()
      .map(|line| line.trim_matches(noise))
      .find(|line| {
        line.split_whitespace().any(|word| {
          let word = word.trim_matches(noise);
          !word.is_empty() && !word.starts_with('#') && !word.starts_with('@')
        })
      })
  });
  (if generic {
    caption.unwrap_or(title)
  } else {
    title
  })
  .chars()
  .take(300)
  .collect()
}

fn preview_command(
  executable: &std::path::Path,
  network: &crate::state::config_models::NetworkSettings,
  input: &crate::resolve::ResolvedInput,
  fast: bool,
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
  command.args(input.args());
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
        card_title(&serde_json::json!({"title":title,"description":caption,"channel":"nasa"})),
        expected
      );
    }
    assert_eq!(card_title(&serde_json::json!({"title":"Video by nasa","channel":"nasa","description":"ع🚀".repeat(200)})).chars().count(), 300);
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
      cached["title"], "Video by nasa",
      "card text never changes the cached download title"
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
  }

  #[test]
  fn capture_previews_record_their_engine_but_playlists_keep_kind_routing() {
    let single = with_engine(serde_json::json!({ "id": "a" }), FetchEngine::Gallery);
    assert_eq!(single["fetchEngine"], "gallery");
    let playlist = with_engine(serde_json::json!({ "entries": [] }), FetchEngine::Ytdlp);
    assert!(playlist.get("fetchEngine").is_none());
  }
}
