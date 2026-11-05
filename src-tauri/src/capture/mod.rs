mod browser;
pub mod clipboard;
mod policy;
mod preview;

use crate::models::parsed::FetchEngine;
use crate::SharedConfig;
use policy::{supported_url, CapturePolicy};

#[cfg(feature = "link-matrix")]
pub(crate) fn matrix_normalize(text: &str) -> Option<String> {
  supported_url(text).map(|(url, _)| url).or_else(|| {
    let url = reqwest::Url::parse(text.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| text.trim().to_string())
  })
}
use serde::{Deserialize, Serialize};
use std::{
  collections::HashMap,
  sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
  },
  time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
  pub id: u64,
  pub url: String,
  pub site: String,
  pub title: String,
  pub details: preview::PreviewDetails,
  pub failed: bool,
  /// Why the read failed (`preview::reason_for_code` vocabulary); None while reading or after success.
  #[serde(skip_deserializing)]
  pub reason: Option<&'static str>,
  pub resolving: bool,
  /// Home/feed page with no specific post: no metadata fetch.
  pub feed: bool,
  #[serde(skip)]
  adopt_on_focus: bool,
}

pub struct CaptureState {
  policy: Mutex<CapturePolicy>,
  offers: Mutex<HashMap<String, Offer>>,
  sequence: AtomicU64,
  quick_request: AtomicU64,
  ready: AtomicBool,
  started: Instant,
  shortcut: Mutex<Option<String>>,
  metadata: Mutex<HashMap<u64, Option<serde_json::Value>>>,
  /// Offer id → the action that claimed it.
  claimed: Mutex<HashMap<u64, &'static str>>,
  jobs: Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>,
  changed: tokio::sync::Notify,
  pub shortcut_failed: AtomicBool,
  /// Why the global shortcut cannot work in this session ("wayland"), when it can't.
  shortcut_reason: Mutex<Option<&'static str>>,
  /// Whether the Wayland compositor offers data-control (background clipboard reads); probed once.
  data_control: tokio::sync::OnceCell<bool>,
  /// Whether the main window had focus when the quick bar opened; when it didn't, closing the
  /// bar gives focus back to the app the user came from instead of raising the main window.
  main_was_focused: AtomicBool,
  main_is_focused: AtomicBool,
  /// Everything sent to the main window, until it pulls it (`capture_take_pending`). Linux can hold
  /// an event for a hidden window until the next user input (tauri#3654); the pull never waits on that.
  pending: Mutex<Outbox>,
}

/// How long, and how many, unpulled messages the main window may have waiting: it pulls every 1.5 s,
/// so anything older belongs to a main window that is gone or stuck.
const OUTBOX_TTL: Duration = Duration::from_secs(120);
const OUTBOX_CAP: usize = 50;

#[derive(Default)]
struct Outbox {
  actions: Vec<(Instant, CaptureAction)>,
  items: Vec<(Instant, CaptureItem)>,
}

#[derive(Default, Serialize)]
pub struct PendingForMain {
  actions: Vec<CaptureAction>,
  items: Vec<CaptureItem>,
}

/// Adds `value`, dropping entries past the TTL, then the oldest past the cap.
fn push_bounded<T>(queue: &mut Vec<(Instant, T)>, now: Instant, value: T) {
  queue.retain(|(at, _)| now.duration_since(*at) < OUTBOX_TTL);
  queue.push((now, value));
  if queue.len() > OUTBOX_CAP {
    let excess = queue.len() - OUTBOX_CAP;
    queue.drain(..excess);
  }
}

fn drain_fresh<T>(queue: &mut Vec<(Instant, T)>, now: Instant) -> Vec<T> {
  std::mem::take(queue)
    .into_iter()
    .filter(|(at, _)| now.duration_since(*at) < OUTBOX_TTL)
    .map(|(_, value)| value)
    .collect()
}

impl Default for CaptureState {
  fn default() -> Self {
    Self {
      policy: Mutex::default(),
      offers: Mutex::default(),
      sequence: AtomicU64::new(0),
      quick_request: AtomicU64::new(0),
      ready: AtomicBool::new(false),
      started: Instant::now(),
      shortcut: Mutex::default(),
      metadata: Mutex::default(),
      claimed: Mutex::default(),
      jobs: Mutex::default(),
      changed: tokio::sync::Notify::new(),
      shortcut_failed: AtomicBool::new(false),
      shortcut_reason: Mutex::default(),
      data_control: tokio::sync::OnceCell::new(),
      main_was_focused: AtomicBool::new(false),
      main_is_focused: AtomicBool::new(false),
      pending: Mutex::default(),
    }
  }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
  app.manage(CaptureState::default());
  for (label, width, height) in [("capture-card", 360., 180.), ("capture-bar", 620., 360.)] {
    tauri::WebviewWindowBuilder::new(
      app,
      label,
      tauri::WebviewUrl::App(format!("index.html?capture={label}").into()),
    )
    .title("Nezzel")
    .inner_size(width, height)
    .visible(false)
    .decorations(false)
    // Transparent so the panel's rounded corners show (macOS needs macOSPrivateApi in tauri.conf).
    .transparent(true)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .accept_first_mouse(true)
    .focusable(label == "capture-bar")
    .build()?;
  }
  // The user may click into the main window while the bar is still open; closing the bar must
  // then leave Nezzel in front instead of hiding it.
  if let Some(main) = app.get_webview_window("main") {
    app
      .state::<CaptureState>()
      .main_is_focused
      .store(main.is_focused().unwrap_or(true), Ordering::Relaxed);
    let handle = app.clone();
    main.on_window_event(move |event| {
      if let tauri::WindowEvent::Focused(focused) = event {
        handle
          .state::<CaptureState>()
          .main_is_focused
          .store(*focused, Ordering::Relaxed);
      }
      if matches!(event, tauri::WindowEvent::Focused(true)) {
        handle
          .state::<CaptureState>()
          .main_was_focused
          .store(true, Ordering::Relaxed);
        let action = adopt_on_focus(&handle.state::<CaptureState>());
        if let Some(action) = action {
          tracing::info!("Copied-link card adopted: the main window got focus");
          if let Some(card) = handle.get_webview_window("capture-card") {
            let _ = card.hide();
          }
          let _ = emit_action(&handle, action);
        }
      }
    });
  }
  let app = app.clone();
  tauri::async_runtime::spawn(watch_clipboard(app));
  Ok(())
}

/// Whether the compositor offers data-control; only probed in Wayland sessions.
async fn data_control(app: &AppHandle) -> bool {
  *app
    .state::<CaptureState>()
    .data_control
    .get_or_init(|| async {
      clipboard::current_session() == clipboard::Session::Wayland
        && clipboard::probe_data_control_bounded().await
    })
    .await
}

async fn watch_clipboard(app: AppHandle) {
  let session = clipboard::current_session();
  let data_control = data_control(&app).await;
  let order = clipboard::read_order(session, data_control);
  tracing::info!(
    session = ?session,
    data_control,
    background = clipboard::background_clipboard(session, data_control),
    readers = ?order,
    "Clipboard watch started"
  );
  let mut diag = clipboard::Diagnostics::default();
  let mut last_prefill = String::new();
  let mut tick = tokio::time::interval(Duration::from_millis(500));
  // A slow read (bounded by READ_LIMIT) skips the ticks it overran instead of firing them in a burst.
  tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  loop {
    tick.tick().await;
    let state = app.state::<CaptureState>();
    let ready = state.ready.load(Ordering::Relaxed);
    diag.note("ready", if ready { "true" } else { "false" });
    if !ready {
      continue;
    }
    let cfg = app.state::<SharedConfig>().load();
    let enabled = cfg.input.watch_clipboard || cfg.input.auto_fill_clipboard;
    diag.note("enabled", if enabled { "true" } else { "false" });
    if !enabled {
      continue;
    }
    // Clipboard failures are expected (empty, non-text, locked by another process).
    let (read, reader) = clipboard::read_text(&app, order).await;
    diag.note(
      "read",
      match (&read, reader) {
        (Ok(_), Some(clipboard::Reader::Gtk)) => "ok-gtk",
        (Ok(_), _) => "ok-plugin",
        (Err(e), _) => e.kind(),
      },
    );
    let (candidate, prefill) = match read {
      Ok(text) => {
        // Lock after reading: capture_write_clipboard records its own write under this lock first.
        let mut policy = state.policy.lock().unwrap();
        let prefill = supported_url(&text).map(|(url, _)| url).unwrap_or_default();
        let decision = policy.decide(&text, state.started.elapsed());
        diag.note("policy", decision.kind());
        (decision.offer(), prefill)
      }
      Err(_) => (None, String::new()),
    };
    if prefill != last_prefill {
      if cfg.input.auto_fill_clipboard {
        let _ = app.emit_to("main", "capture_clipboard", &prefill);
      }
      last_prefill = prefill;
    }
    if let Some((url, _)) = candidate {
      if cfg.input.watch_clipboard {
        // Do not replace a quick-bar interaction with a background offer.
        if app
          .get_webview_window("capture-bar")
          .is_some_and(|w| w.is_visible().unwrap_or(false))
        {
          diag.note("card", "skipped-bar-open");
        } else {
          let shown = offer(&app, "capture-card", &url, false);
          diag.note(
            "card",
            if shown.is_ok() {
              "shown"
            } else {
              "show-failed"
            },
          );
        }
      }
    }
  }
}

fn offer(app: &AppHandle, label: &str, text: &str, resolving: bool) -> Result<Offer, String> {
  let parsed = supported_url(text);
  let (url, site) = parsed.unwrap_or_default();
  let state = app.state::<CaptureState>();
  // Serialize creation with adoption; never make native window calls while holding this lock.
  let mut offers = state.offers.lock().unwrap();
  let feed = !url.is_empty() && policy::is_feed_page(&url);
  let value = Offer {
    id: state.sequence.fetch_add(1, Ordering::Relaxed) + 1,
    url,
    site,
    resolving,
    feed,
    adopt_on_focus: label == "capture-card" && !state.main_is_focused.load(Ordering::Relaxed),
    ..Offer::default()
  };
  if let Some(previous) = offers.insert(label.into(), value.clone()) {
    state.metadata.lock().unwrap().remove(&previous.id);
    state.claimed.lock().unwrap().remove(&previous.id);
  }
  drop(offers);
  if let Some(job) = state.jobs.lock().unwrap().remove(label) {
    job.abort();
  }
  state.changed.notify_waiters();
  let window = app.get_webview_window(label).ok_or("capture unavailable")?;
  if label == "capture-card" {
    if let Some(monitor) = window
      .current_monitor()
      .ok()
      .flatten()
      .or_else(|| window.primary_monitor().ok().flatten())
    {
      let area = monitor.work_area();
      let scale = monitor.scale_factor();
      let width = (360. * scale) as i32;
      let height = (180. * scale) as i32;
      let gap = (16. * scale) as i32;
      let lang = app
        .state::<SharedConfig>()
        .load()
        .appearance
        .language
        .clone();
      let rtl = lang == "ar"
        || (lang == "system" && sys_locale::get_locale().is_some_and(|s| s.starts_with("ar")));
      let x = area.position.x
        + if rtl {
          gap
        } else {
          area.size.width as i32 - width - gap
        };
      let y = area.position.y
        + if cfg!(target_os = "macos") {
          gap
        } else {
          area.size.height as i32 - height - gap
        };
      let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
  } else {
    let _ = window.center();
  }
  if label == "capture-bar" && !window.is_visible().unwrap_or(false) {
    let focused = app
      .get_webview_window("main")
      .and_then(|main| main.is_focused().ok())
      .unwrap_or(false);
    state.main_was_focused.store(focused, Ordering::Relaxed);
  }
  window.show().map_err(|_| "capture unavailable")?;
  tracing::debug!(window = label, host = %value.site, "Capture window shown");
  if label == "capture-bar" && !resolving {
    let _ = window.set_focus();
  }
  let _ = app.emit_to(label, "capture_update", &value);
  if !resolving && !value.url.is_empty() && !value.feed {
    let app = app.clone();
    let value = value.clone();
    let label = label.to_string();
    let job_label = label.clone();
    let job = tauri::async_runtime::spawn(async move {
      preview(app, job_label, value).await;
    });
    state.jobs.lock().unwrap().insert(label, job);
  }
  if label == "capture-card" {
    let app = app.clone();
    let id = value.id;
    tauri::async_runtime::spawn(async move {
      tokio::time::sleep(Duration::from_secs(8)).await;
      let state = app.state::<CaptureState>();
      let offers = state.offers.lock().unwrap();
      if offers.get("capture-card").is_some_and(|v| v.id == id) {
        if let Some(window) = app.get_webview_window("capture-card") {
          let _ = window.hide();
        }
      }
    });
  }
  Ok(value)
}

pub fn open_quick_bar(app: &AppHandle, browser_first: bool) {
  let request_id = app
    .state::<CaptureState>()
    .quick_request
    .fetch_add(1, Ordering::Relaxed)
    + 1;
  let app = app.clone();
  tauri::async_runtime::spawn(async move {
    // Resolve the foreground identity BEFORE focusing Nezzel. The second command may
    // request Automation permission, so show the explanation before executing it.
    let request = if browser_first {
      browser::request().await
    } else {
      None
    };
    let clipboard = read_current(&app).await.unwrap_or_default();
    if app
      .state::<CaptureState>()
      .quick_request
      .load(Ordering::Relaxed)
      != request_id
    {
      return;
    }
    if let Some(card) = app.get_webview_window("capture-card") {
      let _ = card.hide();
    }
    let pending_id = if request.is_some() {
      offer(&app, "capture-bar", &clipboard, true)
        .ok()
        .map(|offer| offer.id)
    } else {
      None
    };
    let browser_url = if let Some(command) = request {
      browser::resolve(command).await
    } else {
      None
    };
    let state = app.state::<CaptureState>();
    if state.quick_request.load(Ordering::Relaxed) != request_id {
      return;
    }
    if let Some(id) = pending_id {
      // Dismissal or typing while the OS prompt is open supersedes the browser result.
      if !state
        .offers
        .lock()
        .unwrap()
        .get("capture-bar")
        .is_some_and(|offer| offer.id == id)
      {
        return;
      }
    }
    let mut text = browser_url
      .filter(|s| supported_url(s).is_some())
      .unwrap_or_else(|| clipboard.clone());
    // On a feed page, prefer a clipboard post URL from the same site.
    if let (Some((url, site)), Some((_, clip_site))) =
      (supported_url(&text), supported_url(&clipboard))
    {
      if policy::is_feed_page(&url)
        && same_site(&site, &clip_site)
        && !supported_url(&clipboard).is_some_and(|(u, _)| policy::is_feed_page(&u))
      {
        text = clipboard;
      }
    }
    let _ = offer(&app, "capture-bar", &text, false);
  });
}

/// `www.` and mobile `m.` hosts are the same site as the bare domain.
fn same_site(a: &str, b: &str) -> bool {
  let key = |s: &str| {
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.strip_prefix("m.").unwrap_or(s).to_string()
  };
  key(a) == key(b)
}

async fn preview(app: AppHandle, label: String, value: Offer) {
  let (item, reason) = match preview_item(&app, &value).await {
    Ok(item) => (Some(item), None),
    Err(reason) => (None, Some(reason)),
  };
  let state = app.state::<CaptureState>();
  let mut offers = state.offers.lock().unwrap();
  if let Some(current) = offers.get_mut(&label).filter(|v| v.id == value.id) {
    current.failed = item.is_none();
    current.reason = reason;
    if let Some(item) = &item {
      current.title = item
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(300)
        .collect();
      current.details = preview::summarize(item);
    }
    state.metadata.lock().unwrap().insert(value.id, item);
    let status = preview::PreviewStatus {
      id: current.id,
      title: current.title.clone(),
      details: current.details.clone(),
      failed: current.failed,
      reason: current.reason,
    };
    let _ = app.emit_to(&label, "capture_metadata", &status);
  }
  state.changed.notify_waiters();
}

/// The link's media item, or why it could not be read (a fixed reason word, never engine text).
async fn preview_item(app: &AppHandle, value: &Offer) -> Result<serde_json::Value, &'static str> {
  let network = app.state::<SharedConfig>().load().network.clone();
  // Check before either engine: a configured but empty proxy must never go direct.
  if network.enable_proxy == Some(true)
    && network.proxy.as_deref().is_none_or(|p| p.trim().is_empty())
  {
    return Err("network");
  }
  if crate::files::hosts::plan(&value.url, crate::files::hosts::Export::Pdf).is_some() {
    // File-share failures stay with Files; never invoke yt-dlp for their preview.
    if network.enable_proxy == Some(true) {
      return Err("unknown");
    }
    return tokio::time::timeout(
      Duration::from_secs(20),
      crate::files::runner::preview(&value.url, format!("capture-{}", value.id)),
    )
    .await
    .map_err(|_| "slow")?
    .map_err(|_| "unknown")
    .map(|item| with_engine(item, FetchEngine::Files));
  }
  let gallery_first = crate::parsers::gallery::gallery_first(&value.url);
  if gallery_first {
    match crate::runners::gallery::preview_media(
      app,
      &value.url,
      format!("capture-{}", value.id),
      &network,
    )
    .await
    {
      Ok(Some(item)) => return Ok(with_engine(item, FetchEngine::Gallery)),
      // A login wall leaves the media type unknown. The foreground fetch must
      // resolve it with consented cookies instead of caching a yt-dlp preview.
      Err(crate::parsers::gallery::GalleryError::Login) => return Err("login"),
      _ => {}
    }
  }
  let item = preview_ytdlp(app, value, &network)
    .await
    .map(|item| with_engine(item, FetchEngine::Ytdlp));
  match item {
    Ok(item) => Ok(item),
    Err(reason) if gallery_first => Err(reason),
    Err(reason) => crate::runners::gallery::preview_media(
      app,
      &value.url,
      format!("capture-{}", value.id),
      &network,
    )
    .await
    .ok()
    .flatten()
    .map(|item| with_engine(item, FetchEngine::Gallery))
    .ok_or(reason),
  }
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
  network: &crate::state::config_models::NetworkSettings,
) -> Result<serde_json::Value, &'static str> {
  let process_guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
  let manager = crate::binaries::binaries_manager::BinariesManager::for_app(app);
  let path = manager.tool_environment().map_err(|_| "unknown")?;
  let executable = manager.tool_path("yt-dlp").map_err(|_| "unknown")?;
  let mut command = preview_command(&executable, network, &value.url).ok_or("unknown")?;
  command.env("PATH", path);
  let bytes = preview::read_output_reason(&mut command, process_guard).await?;
  let json = String::from_utf8(bytes).map_err(|_| "unknown")?;
  if serde_json::from_str::<serde_json::Value>(&json)
    .is_ok_and(|info| crate::parsers::gallery::tiktok_photo_candidate(&value.url, &info))
  {
    return Err("unsupported");
  }
  let parsed = crate::parsers::ytdlp_info::parse_ytdlp_info(&json, format!("capture-{}", value.id))
    .map_err(|_| "unknown")?;
  match parsed {
    crate::models::ParsedMedia::Single(item) => serde_json::to_value(item).map_err(|_| "unknown"),
    crate::models::ParsedMedia::Playlist(item) => serde_json::to_value(item).map_err(|_| "unknown"),
    _ => Err("unsupported"),
  }
}

fn preview_command(
  executable: &std::path::Path,
  network: &crate::state::config_models::NetworkSettings,
  url: &str,
) -> Option<tokio::process::Command> {
  // Background previews never load cookies, user yt-dlp config, or engine diagnostics.
  let mut command = tokio::process::Command::new(executable);
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
  // Same network route as real downloads: never bypass the user's proxy.
  if network.enable_proxy == Some(true) {
    match network.proxy.as_deref() {
      Some(proxy) if !proxy.is_empty() => {
        command.args(["--proxy", proxy]);
      }
      // Proxy switched on but empty: skip the background fetch rather than go direct.
      _ => return None,
    }
  }
  match network.impersonate.as_str() {
    "none" => {}
    "any" => {
      command.args(["--impersonate", ""]);
    }
    other => {
      command.args(["--impersonate", other]);
    }
  }
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
mod action_tests {
  use super::*;

  #[test]
  fn only_known_quick_picks_reach_the_main_window() {
    for (input, out) in [
      (Some("best"), Some("best")),
      (Some("audio"), Some("audio")),
      (Some("small"), Some("small")),
      (Some("4k"), None),
      (Some("audio; rm"), None),
      (None, None),
    ] {
      assert_eq!(quick_choice(input), out, "{input:?}");
    }
  }

  #[test]
  fn capture_previews_record_their_engine_but_playlists_keep_kind_routing() {
    let single = with_engine(serde_json::json!({ "id": "a" }), FetchEngine::Gallery);
    assert_eq!(single["fetchEngine"], "gallery");
    let playlist = with_engine(serde_json::json!({ "entries": [] }), FetchEngine::Ytdlp);
    assert!(playlist.get("fetchEngine").is_none());
  }

  fn pending_offer(state: &CaptureState, label: &str, adoptable: bool) {
    state.offers.lock().unwrap().insert(
      label.into(),
      Offer {
        id: 1,
        url: "https://www.youtube.com/watch?v=capture".into(),
        adopt_on_focus: adoptable,
        ..Offer::default()
      },
    );
  }

  #[test]
  fn focus_adopts_card_once_and_a_racing_click_still_acts_once() {
    let state = CaptureState::default();
    pending_offer(&state, "capture-card", true);
    let payload = adopt_on_focus(&state).unwrap();
    assert_eq!(payload.action, "adopt");
    assert_eq!(payload.id, 1);
    assert!(payload.item.is_none());
    assert!(adopt_on_focus(&state).is_none());
    // A click racing the focus upgrades the adopted row once; nothing claims it after that.
    assert_eq!(
      claim_action(&state, "capture-card", 1, "download")
        .unwrap()
        .action,
      "download"
    );
    for action in ["download", "options", "adopt"] {
      assert_eq!(
        claim_action(&state, "capture-card", 1, action)
          .err()
          .as_deref(),
        Some("offer already handled")
      );
    }
  }

  #[test]
  fn focus_ignores_bar_foreground_created_and_unsupported_offers() {
    let state = CaptureState::default();
    pending_offer(&state, "capture-bar", true);
    assert!(adopt_on_focus(&state).is_none());
    pending_offer(&state, "capture-card", false);
    assert!(adopt_on_focus(&state).is_none());
    state
      .offers
      .lock()
      .unwrap()
      .get_mut("capture-card")
      .unwrap()
      .adopt_on_focus = true;
    state
      .offers
      .lock()
      .unwrap()
      .get_mut("capture-card")
      .unwrap()
      .url = "https://example.com".into();
    assert!(adopt_on_focus(&state).is_none());
    assert!(state.claimed.lock().unwrap().is_empty());
  }

  /// The card (id 1) and the bar (id 2) both hold the same link.
  fn same_link_in_both_windows(state: &CaptureState) {
    pending_offer(state, "capture-card", true);
    let mut bar = state.offers.lock().unwrap()["capture-card"].clone();
    bar.id = 2;
    state
      .offers
      .lock()
      .unwrap()
      .insert("capture-bar".into(), bar);
  }

  #[test]
  fn one_link_is_claimed_once_across_both_capture_windows() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    let first = claim_action(&state, "capture-bar", 2, "download").unwrap();
    assert_eq!(first.retired.as_deref(), Some("capture-card"));
    // The racing click in the other window finds its offer settled: one queued download, not two.
    assert_eq!(
      claim_action(&state, "capture-card", 1, "download")
        .err()
        .as_deref(),
      Some("offer expired")
    );
    assert!(adopt_on_focus(&state).is_none());
    // The retired offer leaves no claim or metadata behind.
    assert!(!state.claimed.lock().unwrap().contains_key(&1));
    assert!(state.offers.lock().unwrap().contains_key("capture-bar"));
  }

  #[test]
  fn a_link_focus_already_adopted_is_upgraded_not_queued_again() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    assert_eq!(adopt_on_focus(&state).unwrap().id, 1);
    let upgrade = claim_action(&state, "capture-bar", 2, "download").unwrap();
    // Same row id as the adopted one: the main window upgrades it instead of adding a second row.
    assert_eq!((upgrade.id, upgrade.action.as_str()), (1, "download"));
    assert_eq!(
      claim_action(&state, "capture-bar", 2, "download")
        .err()
        .as_deref(),
      Some("offer already handled")
    );
  }

  #[test]
  fn a_different_link_in_the_other_window_stays() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    state
      .offers
      .lock()
      .unwrap()
      .get_mut("capture-card")
      .unwrap()
      .url = "https://www.youtube.com/watch?v=other".into();
    assert!(claim_action(&state, "capture-bar", 2, "download")
      .unwrap()
      .retired
      .is_none());
    assert!(state.offers.lock().unwrap().contains_key("capture-card"));
  }

  #[test]
  fn the_outbox_forgets_old_entries_and_keeps_at_most_the_cap() {
    let start = Instant::now();
    let mut queue = Vec::new();
    push_bounded(&mut queue, start, 0);
    push_bounded(&mut queue, start + OUTBOX_TTL, 1);
    assert_eq!(queue.iter().map(|(_, v)| *v).collect::<Vec<_>>(), [1]);
    for n in 2..(OUTBOX_CAP as i32 + 10) {
      push_bounded(&mut queue, start + OUTBOX_TTL, n);
    }
    assert_eq!(queue.len(), OUTBOX_CAP);
    assert_eq!(queue[0].1, 10);
    assert!(drain_fresh(&mut queue, start + OUTBOX_TTL * 3).is_empty());
    assert!(queue.is_empty());
  }

  #[test]
  fn main_pulls_every_sent_action_once() {
    let state = CaptureState::default();
    pending_offer(&state, "capture-card", true);
    let action = claim_action(&state, "capture-card", 1, "download").unwrap();
    queue_for_main(&state, &action);
    let pending = take_pending(&state);
    assert_eq!(pending.actions.len(), 1);
    assert_eq!(pending.actions[0].id, 1);
    assert!(take_pending(&state).actions.is_empty());
  }

  #[test]
  fn download_and_options_claim_immediately_without_preview_metadata() {
    for action in ["download", "options"] {
      let state = CaptureState::default();
      pending_offer(&state, "capture-card", true);
      let payload = claim_action(&state, "capture-card", 1, action).unwrap();
      assert_eq!(payload.action, action);
      assert_eq!(payload.url, "https://www.youtube.com/watch?v=capture");
      assert!(payload.item.is_none());
    }
  }

  #[test]
  fn action_includes_an_already_finished_preview() {
    let state = CaptureState::default();
    pending_offer(&state, "capture-card", true);
    let item = serde_json::json!({ "title": "Ready" });
    state.metadata.lock().unwrap().insert(1, Some(item.clone()));
    assert_eq!(
      claim_action(&state, "capture-card", 1, "options")
        .unwrap()
        .item,
      Some(item)
    );
  }

  #[tokio::test(start_paused = true)]
  async fn preview_completion_covers_success_failure_replacement_and_timeout() {
    let state = CaptureState::default();
    pending_offer(&state, "capture-card", true);
    // Tokio's paused clock advances through the timeout without waiting in real time.
    assert!(wait_for_item(&state, 1).await.is_none());
    let item = serde_json::json!({ "title": "Finished" });
    state.metadata.lock().unwrap().insert(1, Some(item.clone()));
    assert_eq!(wait_for_item(&state, 1).await, Some(item));
    state.metadata.lock().unwrap().insert(1, None);
    assert!(wait_for_item(&state, 1).await.is_none());
    state.metadata.lock().unwrap().clear();
    state.offers.lock().unwrap().clear();
    assert!(wait_for_item(&state, 1).await.is_none());
  }
}

fn main_only(window: &WebviewWindow) -> Result<(), String> {
  if window.label() == "main" {
    Ok(())
  } else {
    Err("unavailable".into())
  }
}

#[tauri::command]
pub fn capture_ready(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
  main_only(&window)?;
  app
    .state::<CaptureState>()
    .ready
    .store(true, Ordering::Relaxed);
  Ok(())
}

/// Raw clipboard text through the session's readers. Async on purpose: a sync command runs on the main
/// thread, where arboard can deadlock on Linux and GTK could not answer its own selection.
async fn read_current(app: &AppHandle) -> Result<String, clipboard::ReadError> {
  let order = clipboard::read_order(clipboard::current_session(), data_control(app).await);
  clipboard::read_text(app, order).await.0
}

#[tauri::command]
pub async fn capture_clipboard_current(
  app: AppHandle,
  window: WebviewWindow,
) -> Result<String, String> {
  main_only(&window)?;
  Ok(
    read_current(&app)
      .await
      .ok()
      .and_then(|text| supported_url(&text))
      .map(|(url, _)| url)
      .unwrap_or_default(),
  )
}

/// Paste into the link field (Ctrl/⌘+V outside the field, the Paste button): same readers as the watcher.
#[tauri::command]
pub async fn capture_read_clipboard(
  app: AppHandle,
  window: WebviewWindow,
) -> Result<String, String> {
  main_only(&window)?;
  read_current(&app)
    .await
    .map_err(|e| format!("clipboard {}", e.kind()))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePlatform {
  pub session: clipboard::Session,
  /// A copy in another app can raise the card while Nezzel is in the background.
  pub background_clipboard: bool,
  /// Whose words the tray copy uses (Linux; `other` elsewhere).
  pub desktop: clipboard::Desktop,
}

#[tauri::command]
pub async fn capture_platform(app: AppHandle) -> CapturePlatform {
  let session = clipboard::current_session();
  CapturePlatform {
    session,
    background_clipboard: clipboard::background_clipboard(session, data_control(&app).await),
    desktop: clipboard::current_desktop(),
  }
}

#[tauri::command]
pub fn capture_snapshot(app: AppHandle, window: WebviewWindow) -> Offer {
  app
    .state::<CaptureState>()
    .offers
    .lock()
    .unwrap()
    .get(window.label())
    .cloned()
    .unwrap_or_default()
}

#[tauri::command]
pub fn capture_edit(app: AppHandle, window: WebviewWindow, text: String) -> Result<(), String> {
  if window.label() != "capture-bar" {
    return Err("unavailable".into());
  }
  offer(&app, "capture-bar", &text, false)?;
  Ok(())
}

/// Failed read → "Retry": read the same link again in the same bar or card (a fresh offer id).
#[tauri::command]
pub fn capture_retry(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
  let label = window.label();
  if !matches!(label, "capture-card" | "capture-bar") {
    return Err("unavailable".into());
  }
  let state = app.state::<CaptureState>();
  let url = state
    .offers
    .lock()
    .unwrap()
    .get(label)
    .filter(|offer| offer.failed)
    .map(|offer| offer.url.clone())
    .ok_or("nothing to retry")?;
  offer(&app, label, &url, false)?;
  Ok(())
}

/// Failed read → "Report": the main window opens Report a problem with the reason word only;
/// the link goes along only if the user ticks "Include the link" there.
#[derive(Clone, Serialize)]
struct CaptureReport {
  url: String,
  reason: &'static str,
}

#[derive(Clone, Serialize)]
pub struct CaptureAction {
  pub id: u64,
  pub url: String,
  pub action: String,
  pub item: Option<serde_json::Value>,
  /// Quick pick from the bar or card ("best", "audio", "small"); None = the user's default choice.
  pub choice: Option<&'static str>,
  /// The other capture window whose offer for the same link this claim settled (to hide).
  #[serde(skip)]
  pub retired: Option<String>,
}

/// Only the known quick picks cross into the main window; anything else means "the default choice".
pub fn quick_choice(choice: Option<&str>) -> Option<&'static str> {
  match choice? {
    "best" => Some("best"),
    "audio" => Some("audio"),
    "small" => Some("small"),
    _ => None,
  }
}

#[derive(Clone, Serialize)]
pub struct CaptureItem {
  id: u64,
  item: Option<serde_json::Value>,
}

// Claim and snapshot only: metadata must never delay the initial queue event.
fn claim_action(
  state: &CaptureState,
  label: &str,
  id: u64,
  action: &str,
) -> Result<CaptureAction, String> {
  let offers = state.offers.lock().unwrap();
  let current = offers
    .get(label)
    .filter(|offer| offer.id == id)
    .ok_or("offer expired")?;
  if action != "dismiss" && current.url.is_empty() {
    return Err("unsupported URL".into());
  }
  let action: &'static str = match action {
    "download" => "download",
    "options" => "options",
    "adopt" => "adopt",
    _ => "dismiss",
  };
  let url = current.url.clone();
  let mut claimed = state.claimed.lock().unwrap();
  let upgrades = |claim: Option<&&str>| match claim {
    None => Ok(()),
    // X11 can focus the main window as the card is clicked: focus adopts the row first, and the
    // click must still start it (or open its options) instead of being dropped.
    Some(&"adopt") if matches!(action, "download" | "options") => Ok(()),
    Some(_) => Err("offer already handled".to_string()),
  };
  upgrades(claimed.get(&id))?;
  // One claim per link across both capture windows, under the offers lock: two windows (or a double
  // click racing the focus) can't both queue the same link.
  // Adopting never settles the other window: the bar may be the user's live interaction.
  let other = matches!(action, "download" | "options")
    .then(|| {
      offers
        .iter()
        .find(|(other, offer)| other.as_str() != label && offer.url == url)
        .map(|(other, offer)| (other.clone(), offer.id))
    })
    .flatten();
  // The other window's offer was already adopted as a row: upgrade that row instead of a second one.
  let row_id = match &other {
    Some((_, other_id)) if claimed.contains_key(other_id) => {
      upgrades(claimed.get(other_id))?;
      *other_id
    }
    _ => id,
  };
  claimed.insert(id, action);
  claimed.insert(row_id, action);
  drop(claimed);
  let item = state
    .metadata
    .lock()
    .unwrap()
    .get(&row_id)
    .cloned()
    .flatten();
  let mut offers = offers;
  let retired = other.map(|(other, other_id)| {
    offers.remove(&other);
    if other_id != row_id {
      state.claimed.lock().unwrap().remove(&other_id);
    }
    state.metadata.lock().unwrap().remove(&other_id);
    if let Some(job) = state.jobs.lock().unwrap().remove(&other) {
      job.abort();
    }
    other
  });
  drop(offers);
  if retired.is_some() {
    state.changed.notify_waiters();
  }
  Ok(CaptureAction {
    id: row_id,
    url,
    action: action.into(),
    item,
    choice: None,
    retired,
  })
}

fn adopt_on_focus(state: &CaptureState) -> Option<CaptureAction> {
  let current = state.offers.lock().unwrap().get("capture-card").cloned()?;
  if !current.adopt_on_focus || supported_url(&current.url).is_none() {
    return None;
  }
  claim_action(state, "capture-card", current.id, "adopt").ok()
}

async fn wait_for_item(state: &CaptureState, id: u64) -> Option<serde_json::Value> {
  tokio::time::timeout(Duration::from_secs(46), async {
    loop {
      let notified = state.changed.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();
      if let Some(item) = state.metadata.lock().unwrap().get(&id).cloned() {
        return item;
      }
      if !state.offers.lock().unwrap().values().any(|v| v.id == id) {
        return None;
      }
      notified.await;
    }
  })
  .await
  .ok()
  .flatten()
}

fn emit_action(app: &AppHandle, action: CaptureAction) -> Result<(), String> {
  let pending = action.item.is_none();
  let id = action.id;
  let state = app.state::<CaptureState>();
  queue_for_main(&state, &action);
  tracing::info!(id, action = %action.action, "Capture action sent to the main window");
  // The pull above already holds it: an event that fails or stalls is not a lost download.
  let _ = app.emit_to("main", "capture_action", action);
  if pending {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
      let state = app.state::<CaptureState>();
      let item = wait_for_item(&state, id).await;
      let item = CaptureItem { id, item };
      push_bounded(
        &mut state.pending.lock().unwrap().items,
        Instant::now(),
        item.clone(),
      );
      let _ = app.emit_to("main", "capture_item", item);
    });
  }
  Ok(())
}

/// What the main window has not pulled yet. It still listens for the events (the fast path) and
/// treats both the same, by capture id.
#[tauri::command]
pub fn capture_take_pending(
  app: AppHandle,
  window: WebviewWindow,
) -> Result<PendingForMain, String> {
  main_only(&window)?;
  Ok(take_pending(&app.state::<CaptureState>()))
}

fn queue_for_main(state: &CaptureState, action: &CaptureAction) {
  push_bounded(
    &mut state.pending.lock().unwrap().actions,
    Instant::now(),
    action.clone(),
  );
}

/// The pull is the acknowledgement: what it returns is gone from the outbox.
fn take_pending(state: &CaptureState) -> PendingForMain {
  let now = Instant::now();
  let mut outbox = state.pending.lock().unwrap();
  PendingForMain {
    actions: drain_fresh(&mut outbox.actions, now),
    items: drain_fresh(&mut outbox.items, now),
  }
}

#[tauri::command]
pub async fn capture_action(
  app: AppHandle,
  window: WebviewWindow,
  id: u64,
  action: String,
  choice: Option<String>,
) -> Result<(), String> {
  if !matches!(window.label(), "capture-card" | "capture-bar") {
    return Err("unavailable".into());
  }
  if !matches!(
    action.as_str(),
    "download" | "options" | "dismiss" | "report"
  ) {
    return Err("unavailable".into());
  }
  let state = app.state::<CaptureState>();
  if action == "report" {
    let report = {
      let mut offers = state.offers.lock().unwrap();
      if !offers
        .get(window.label())
        .is_some_and(|offer| offer.id == id && offer.failed)
      {
        return Err("nothing to report".into());
      }
      let offer = offers.remove(window.label()).ok_or("nothing to report")?;
      CaptureReport {
        url: offer.url,
        reason: offer.reason.unwrap_or("unknown"),
      }
    };
    state.metadata.lock().unwrap().remove(&id);
    state.changed.notify_waiters();
    window.hide().map_err(|_| "capture unavailable")?;
    if let Some(main) = app.get_webview_window("main") {
      let _ = main.show();
      let _ = main.set_focus();
    }
    return app
      .emit_to("main", "capture_report", report)
      .map_err(|_| "capture unavailable".into());
  }
  let mut payload = claim_action(&state, window.label(), id, &action)?;
  if action == "download" {
    payload.choice = quick_choice(choice.as_deref());
  }
  if let Some(other) = payload.retired.as_deref() {
    if let Some(other) = app.get_webview_window(other) {
      let _ = other.hide();
    }
  }
  // macOS makes the main window key when the focused bar hides, pulling Nezzel over the browser.
  // "Options" shows the main window on purpose; otherwise go back to where the user was.
  // Read before hiding: the main window's focus event from the hide would flip the flag.
  let return_to_previous_app = window.label() == "capture-bar"
    && action != "options"
    && !state.main_was_focused.load(Ordering::Relaxed);
  // Claimed already: a failed hide must not lose the download.
  let _ = window.hide();
  #[cfg(target_os = "macos")]
  if return_to_previous_app {
    let _ = app.hide();
  }
  #[cfg(not(target_os = "macos"))]
  let _ = return_to_previous_app;
  if action == "dismiss" {
    if let Some(job) = state.jobs.lock().unwrap().remove(window.label()) {
      job.abort();
    }
    state.metadata.lock().unwrap().remove(&id);
    state.offers.lock().unwrap().remove(window.label());
    state.changed.notify_waiters();
    return Ok(());
  }
  emit_action(&app, payload)
}

#[tauri::command]
pub fn capture_write_clipboard(
  app: AppHandle,
  window: WebviewWindow,
  text: String,
) -> Result<(), String> {
  main_only(&window)?;
  let state = app.state::<CaptureState>();
  let mut policy = state.policy.lock().unwrap();
  app
    .clipboard()
    .write_text(&text)
    .map_err(|_| "clipboard unavailable")?;
  policy.own_write(&text, state.started.elapsed());
  Ok(())
}

/// Welcome "Try it" sample: a short, long-lived public video ("Me at the zoo").
pub const TRY_LINK: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

/// Welcome "Try it": copies the fixed sample link, and the clipboard watcher brings up the real card
/// for it like any copied link (dedupe included). Only with the card setting on; never reads the clipboard.
#[tauri::command]
pub fn capture_try_link(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
  main_only(&window)?;
  if !app.state::<SharedConfig>().load().input.watch_clipboard {
    return Err("card off".into());
  }
  app
    .clipboard()
    .write_text(TRY_LINK)
    .map_err(|_| String::from("clipboard unavailable"))
}

/// Quick download shortcut as the OS took it: `active` is what the user presses (the fallback when the default
/// was taken), `failed` means nothing could be registered.
#[derive(Clone, Serialize)]
pub struct ShortcutStatus {
  pub failed: bool,
  pub active: Option<String>,
  /// Why it cannot work in this session ("wayland"); absent for an ordinary refusal.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reason: Option<&'static str>,
}

#[tauri::command]
pub fn capture_shortcut_status(app: AppHandle) -> ShortcutStatus {
  let state = app.state::<CaptureState>();
  let active = state.shortcut.lock().unwrap().clone();
  let reason = *state.shortcut_reason.lock().unwrap();
  ShortcutStatus {
    failed: state.shortcut_failed.load(Ordering::Relaxed),
    active,
    reason,
  }
}

pub fn current_shortcut(app: &AppHandle) -> Option<String> {
  app.state::<CaptureState>().shortcut.lock().unwrap().clone()
}
pub fn take_shortcut(app: &AppHandle) -> Option<String> {
  app.state::<CaptureState>().shortcut.lock().unwrap().take()
}
pub fn set_shortcut(app: &AppHandle, shortcut: Option<String>, failed: bool) {
  set_shortcut_status(app, shortcut, failed, None);
}

/// Nothing registered because the session cannot deliver global presses (see `clipboard::global_shortcut_reason`).
pub fn set_shortcut_unsupported(app: &AppHandle, reason: &'static str) {
  set_shortcut_status(app, None, true, Some(reason));
}

fn set_shortcut_status(
  app: &AppHandle,
  shortcut: Option<String>,
  failed: bool,
  reason: Option<&'static str>,
) {
  let state = app.state::<CaptureState>();
  *state.shortcut.lock().unwrap() = shortcut.clone();
  state.shortcut_failed.store(failed, Ordering::Relaxed);
  *state.shortcut_reason.lock().unwrap() = reason;
  let status = ShortcutStatus {
    failed,
    active: shortcut,
    reason,
  };
  let _ = app.emit_to("main", "capture_shortcut_status", status);
}
