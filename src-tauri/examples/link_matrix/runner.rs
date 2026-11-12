//! Developer harness. See README.md beside this file for scope and stand-ins.
mod options;
mod support;

use crate::{
  models::{download::*, parsed::MediaKind, ParsedMedia},
  runners::{gallery, template_context::TemplateContext},
  scheduling::{download_pipeline::DownloadEntry, fetch_pipeline::FetchEntry, group_state},
  state::{config_models::Config, preferences_models::Preferences},
  SharedConfig, SharedPreferences,
};
use futures::{stream, StreamExt};
use options::{prepare_evidence, AttemptsColumns, Options};
use serde_json::{json, Value};
use std::{
  cell::RefCell,
  collections::{BTreeMap, HashMap, HashSet},
  error::Error,
  fs,
  path::{Path, PathBuf},
  sync::{Arc, Mutex},
  time::{Duration, Instant},
};
use support::{
  classify, last_engine_error, matches_error, matches_expected, matches_selection, parse_links,
  verify_file, Link,
};
use tauri::{Listener, Manager};

type Events = Arc<Mutex<HashMap<String, Vec<(String, Value)>>>>;
pub(crate) struct GalleryResources(pub PathBuf);
tokio::task_local! { static GALLERY_ERRORS: RefCell<Vec<String>>; }

pub(crate) fn observe_gallery_stderr(bytes: &[u8]) {
  let _ = GALLERY_ERRORS.try_with(|lines| {
    lines.borrow_mut().extend(
      String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(safe_line),
    );
  });
}

fn safe_line(line: &str) -> String {
  // Suppress any raw engine line mentioning cookie material.
  if line.to_ascii_lowercase().contains("cookie") {
    "[engine cookie-related diagnostic omitted]".into()
  } else {
    line.chars().take(8192).collect()
  }
}

#[derive(Clone)]
struct ResultRow {
  index: usize,
  link: Link,
  got: String,
  verdict: String,
  reason: String,
  ui: String,
  engine: String,
  cause: String,
  files: Vec<String>,
  seconds: f64,
  attempts: AttemptsColumns,
}

/// Uses a real, windowless Wry runtime because production AppHandle types are Wry-specific.
pub fn main() {
  if let Err(error) = start() {
    eprintln!("link matrix: {error}");
    std::process::exit(2);
  }
}

fn start() -> Result<(), Box<dyn Error>> {
  let args: Vec<_> = std::env::args().skip(1).collect();
  if args.iter().any(|arg| arg == "--help") {
    println!("cargo run --features link-matrix --example link_matrix -- --links A.tsv [--links B.tsv] --out DIR [--jobs 1|2] [--only site[,site]] [--login browser[:profile] | --cookies PATH] (both auth modes are local-only; refused with CI=1)");
    return Ok(());
  }
  let ci = std::env::var("CI").ok();
  let mut options = Options::parse(args, ci.as_deref())?;
  fs::create_dir_all(&options.out)?;
  options.out = fs::canonicalize(&options.out)?;
  let mut seen = HashSet::new();
  let mut links = Vec::new();
  for input in &options.inputs {
    links.extend(
      parse_links(&fs::read_to_string(input)?, &mut seen)
        .map_err(|e| format!("{}: {e}", input.display()))?,
    );
  }
  links.retain(|link| options.includes(&link.site));
  if links.is_empty() {
    return Err("no matching rows".into());
  }
  // Fresh directory per run prevents old files from producing a false PASS.
  let run_dir = options.out.join(format!("run-{}", uuid::Uuid::new_v4()));
  fs::create_dir(&run_dir)?;
  let bin_dir = match std::env::var_os("LINK_MATRIX_BIN_DIR") {
    Some(path) => fs::canonicalize(path)?,
    None => PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?)
      .join("Library/Application Support/dev.ahmed.nezzel/bin"),
  };
  for name in ["yt-dlp", "ffmpeg", "ffprobe", "deno"] {
    if !bin_dir.join(name).is_file() {
      return Err(format!("missing app binary {name}").into());
    }
  }
  // macOS ships gallery-dl as a onedir resource (tools/gallery-dl/gallery-dl); link the whole tools folder.
  let binaries = fs::canonicalize(
    std::env::var_os("LINK_MATRIX_RESOURCES_DIR")
      .map(PathBuf::from)
      .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries")),
  )?;
  let resources = run_dir.join("resources");
  fs::create_dir(&resources)?;
  if binaries.join("tools").is_dir() {
    std::os::unix::fs::symlink(binaries.join("tools"), resources.join("tools"))?;
  }
  // Linux single-file sidecars can be staged with a stable, unsuffixed name.
  if binaries.join("gallery-dl").is_file() {
    link_file(&binaries.join("gallery-dl"), &resources.join("gallery-dl"))?;
  }
  link_file(
    &binaries.join("gallery-dl.sha256"),
    &resources.join("gallery-dl.sha256"),
  )?;
  // Keep the production checksum gate. A missing or mismatched sidecar fails setup.
  crate::runners::gallery_process::find_bundled(&resources.join("harness"), &resources)
    .map_err(|e| format!("gallery-dl verification: {e:?}"))?;

  let events: Events = Arc::default();
  // The library already embeds the macOS plist for the real app entry point.
  let mut context = tauri::generate_context!(test = true);
  context.config_mut().app.windows.clear();
  let app = tauri::Builder::default()
    .plugin(tauri_plugin_store::Builder::new().build())
    .build(context)?;
  let handle = app.handle();
  handle.manage(crate::paths::PathsManager::isolated(
    run_dir.clone(),
    bin_dir.clone(),
  ));
  let mut config = Config::default();
  // Same manual-file setting as Settings > Logins > Advanced; production selects the source.
  config.auth.cookie_file = options.cookies.clone();
  // Discovery remains disabled unless a local browser was explicitly consented to.
  config.auth.cookie_browser = if options.login.is_some() {
    "automatic"
  } else {
    "none"
  }
  .into();
  // Production defaults, with the requested run folder replacing the user's Downloads folder.
  config.output.download_dir = Some(run_dir.to_string_lossy().into_owned());
  handle.manage::<SharedConfig>(Arc::new(crate::state::config::ConfigHandle::isolated(
    handle, config,
  )?));
  handle.manage::<SharedPreferences>(Arc::new(
    crate::state::preferences::PreferencesHandle::isolated(handle, Preferences::default())?,
  ));
  handle.manage(crate::i18n::I18nManager::new(handle));
  handle.manage(crate::logging::LogStoreState::new());
  handle.manage(crate::download_records::DownloadRecords::default());
  handle.manage(crate::library::Library::default());
  handle.manage(GalleryResources(resources));
  // LoginSession needs cleanup channels even when consent was supplied on the command line.
  let (fetch_tx, _fetch_rx) = tokio::sync::mpsc::unbounded_channel();
  let (download_tx, _download_rx) = tokio::sync::mpsc::unbounded_channel();
  handle.manage(crate::scheduling::fetch_pipeline::FetchSender(fetch_tx));
  handle.manage(crate::scheduling::download_pipeline::DownloadSender(
    download_tx,
  ));
  for event_name in [
    "media_attempts",
    "media_diagnostic",
    "media_fatal",
    "media_login_needed",
    "media_complete",
  ] {
    let collected = events.clone();
    handle.listen(event_name, move |event| {
      if let Ok(mut value) = serde_json::from_str::<Value>(event.payload()) {
        // Only fields displayed by the app; never retain browser/profile candidates.
        let group = value["groupId"].as_str().unwrap_or_default().to_string();
        if let Some(object) = value.as_object_mut() {
          object.remove("candidates");
        }
        collected
          .lock()
          .unwrap()
          .entry(group)
          .or_default()
          .push((event_name.into(), value));
      }
    });
  }
  let handle = handle.clone();
  tauri::async_runtime::spawn(async move {
    let outcome = run_all(&handle, options, links, run_dir, bin_dir, events).await;
    if let Err(error) = &outcome {
      eprintln!("link matrix: {error}");
    }
    // No GUI shutdown work exists in this host. Wry's NSApplication exit request
    // can stall without windows under macOS sandboxing; all jobs and reports have
    // settled here, so terminate the developer host explicitly.
    std::process::exit(if outcome.is_ok() { 0 } else { 2 });
  });
  app.run(|_, _| {});
  Ok(())
}

fn link_file(source: &Path, destination: &Path) -> std::io::Result<()> {
  #[cfg(unix)]
  {
    std::os::unix::fs::symlink(source, destination)
  }
  #[cfg(not(unix))]
  {
    fs::copy(source, destination).map(|_| ())
  }
}

async fn run_all(
  app: &tauri::AppHandle,
  options: Options,
  links: Vec<Link>,
  run_dir: PathBuf,
  bins: PathBuf,
  events: Events,
) -> Result<(), String> {
  let total = links.len();
  let mut pending = stream::iter(
    links
      .into_iter()
      .enumerate()
      .map(|(index, link)| run_one(app, index, link, &run_dir, &bins, &events, &options)),
  )
  .buffer_unordered(options.jobs);
  let mut results = Vec::new();
  while let Some(row) = pending.next().await {
    eprintln!(
      "[{}/{}] {} {}: {} ({})",
      results.len() + 1,
      total,
      row.link.site,
      row.link.label,
      row.verdict,
      row.got
    );
    results.push(row);
    results.sort_by_key(|r| r.index);
    write_reports(&options, &run_dir, &results, total)?;
  }
  Ok(())
}

async fn run_one(
  app: &tauri::AppHandle,
  index: usize,
  link: Link,
  run_dir: &Path,
  bins: &Path,
  events: &Events,
  options: &Options,
) -> ResultRow {
  let started = Instant::now();
  let group = format!("matrix-{}", uuid::Uuid::new_v4());
  let folder = run_dir.join(format!("{:03}", index + 1));
  let mut row = ResultRow {
    index,
    link,
    got: "none".into(),
    verdict: "FAIL".into(),
    reason: String::new(),
    ui: String::new(),
    engine: String::new(),
    cause: String::new(),
    files: Vec::new(),
    seconds: 0.0,
    attempts: AttemptsColumns::default(),
  };
  if row.link.url.is_empty() {
    row.reason = "Input coverage gap: no URL; no app invocation".into();
    row.cause = "input: missing fixture URL".into();
    return row;
  }
  if let Err(error) = fs::create_dir(&folder) {
    row.reason = error.to_string();
    row.cause = "harness: output directory".into();
    return row;
  }
  group_state::ensure_group_running(&group);
  let outcome = GALLERY_ERRORS
    .scope(RefCell::new(Vec::new()), async {
      let (item_count, mut discovered) = tokio::sync::watch::channel(1usize);
      let work = pipeline(
        app,
        &row.link,
        &group,
        &folder,
        bins,
        &item_count,
        options.login.as_deref(),
      );
      tokio::pin!(work);
      let deadline = tokio::time::sleep(Duration::from_secs(180));
      tokio::pin!(deadline);
      let began = tokio::time::Instant::now();
      let mut watching = true;
      let result = loop { tokio::select! {
        result = &mut work => break result,
        changed = discovered.changed(), if watching => {
          if changed.is_ok() {
            deadline.as_mut().reset(began + crate::parsers::gallery::gallery_deadline(*discovered.borrow()).max(Duration::from_secs(180)));
          } else { watching = false; }
        }
        _ = &mut deadline => {
          group_state::cancel_group(&group);
          // Let production cancellation kill yt-dlp's process tree before dropping its future.
          let _ = tokio::time::timeout(Duration::from_secs(25), &mut work).await;
          break Err(format!("Harness per-link deadline: {} seconds (production group cancellation)",
            crate::parsers::gallery::gallery_deadline(*discovered.borrow()).max(Duration::from_secs(180)).as_secs()))
        }
      }};
      let gallery_errors = GALLERY_ERRORS.with(|v| v.borrow().clone());
      (result, gallery_errors)
    })
    .await;
  let log = app
    .state::<crate::logging::LogStoreState>()
    .write()
    .get_log(&group)
    .unwrap_or_default();
  let mut captured = events.lock().unwrap().remove(&group).unwrap_or_default();
  row.attempts = AttemptsColumns::from_events(&captured);
  let codes: Vec<_> = captured
    .iter()
    .filter_map(|(kind, v)| {
      if kind == "media_diagnostic" {
        v["code"].as_str().map(str::to_string)
      } else if kind == "media_login_needed" {
        v["reason"].as_str().map(str::to_string)
      } else {
        None
      }
    })
    .collect();
  row.ui = captured
    .iter()
    .filter_map(|(kind, v)| match kind.as_str() {
      "media_diagnostic" => v["code"].as_str().map(|code| {
        format!(
          "{code} / errors.runner.{code}.message / {}",
          crate::parsers::ytdlp_error::redact_diagnostic(v["message"].as_str().unwrap_or_default())
        )
      }),
      "media_login_needed" => Some(format!(
        "media_login_needed / {}",
        v["reason"].as_str().unwrap_or_default()
      )),
      "media_fatal" => Some(format!(
        "media_fatal(exit={}) / {}",
        v["exitCode"],
        crate::parsers::ytdlp_error::redact_diagnostic(v["message"].as_str().unwrap_or_default())
      )),
      "media_complete" => Some("media_complete (app reports success)".into()),
      _ => None,
    })
    .collect::<Vec<_>>()
    .join("; ");
  row.engine = safe_line(&last_engine_error(&log));
  let gallery_error = last_engine_error(&outcome.1.join("\n"));
  if !gallery_error.is_empty() {
    // Retain both engines on fallbacks rather than hiding a useful gallery refusal.
    if row.engine.is_empty() {
      row.engine.clone_from(&gallery_error);
    } else {
      row.engine = format!("gallery-dl: {gallery_error}; yt-dlp: {}", row.engine);
    }
  }
  match outcome.0 {
    Ok((images, videos, audio, files)) => {
      row.got = if row.link.expected == "file" {
        format!("{} verified file(s)", files.len())
      } else {
        format!("{images} image, {videos} video, {audio} audio")
      };
      row.files = files;
      if !captured.iter().any(|(kind, _)| kind == "media_fatal")
        && ((row.link.expected == "file" && !row.files.is_empty())
          || matches_expected(
            &row.link.expected,
            row.link.count,
            row.link.count_is_total,
            images,
            videos,
            audio,
          ))
      {
        row.verdict = "PASS".into();
        row.reason = if row.link.expected == "unknown" {
          "Files verified; input has no kind assertion".into()
        } else {
          "Files verified".into()
        };
      } else {
        row.reason = format!(
          "Saved files do not match expected {} (photo count {:?})",
          row.link.expected, row.link.count
        );
        row.cause = if row.link.url.contains("tiktok.com/")
          && row.link.url.contains("/video/")
          && videos + audio > 0
          && matches!(row.link.expected.as_str(), "photo" | "photos")
        {
          "expected photos but /video URL saved AV; gallery-first bypass (src/parsers/gallery.rs:47); fixture type needs confirmation".into()
        } else {
          "validation: expected kind/count mismatch".into()
        };
      }
    }
    Err(error) => {
      row.reason = error;
      // Only the final failing engine determines LOGIN/GONE. An earlier gallery
      // unsupported/404 on a yt-dlp fallback is diagnostic context, not the verdict.
      // A successful yt-dlp probe can leave a log before a later gallery image
      // failure. The last production diagnostic identifies that gallery path.
      let gallery_failed = captured
        .iter()
        .rev()
        .find(|(kind, _)| kind == "media_diagnostic")
        .is_some_and(|(_, value)| value["component"] == "gallery-dl");
      let final_engine = if gallery_failed || log.trim().is_empty() {
        gallery_error.as_str()
      } else {
        &last_engine_error(&log)
      };
      let evidence = format!("{} {}", row.reason, final_engine);
      row.verdict = classify(&codes, &evidence).into();
      row.cause = root_cause(&codes, &evidence);
      // Include incomplete artifacts for inspection, never count them toward PASS.
      let listed = list_files(&folder, &mut row.files).is_ok();
      // Captured events are appended in production emission order. A previous
      // fallback's refusal is insufficient if the terminal attempt failed otherwise.
      let terminal_codes: Vec<_> = captured
        .iter()
        .filter_map(|(kind, value)| {
          match kind.as_str() {
            "media_diagnostic" => Some(value["code"].as_str().unwrap_or_default()),
            // An empty code is a successful fetch: it must hide earlier refusals.
            "media_attempts" => Some(value["finalCode"].as_str().unwrap_or_default()),
            "media_login_needed" => Some(value["reason"].as_str().unwrap_or_default()),
            _ => None,
          }
        })
        .collect();
      if listed
        && row.files.is_empty()
        && matches_error(&row.link.expected, &terminal_codes, &row.reason)
      {
        row.verdict = "PASS".into();
        row.cause.clear();
        row.got = format!(
          "Expected refusal: {}",
          terminal_codes.last().copied().unwrap_or_default()
        );
      }
    }
  }
  if row.ui.is_empty() {
    row.ui = if row.verdict == "PASS" {
      "media_complete"
    } else {
      "No app error event; harness validation/deadline"
    }
    .into();
  }
  if options.authenticated() {
    row.engine.clear();
    row.reason = crate::parsers::ytdlp_error::redact_diagnostic(&row.reason);
  }
  row.seconds = started.elapsed().as_secs_f64();
  for (_, value) in &mut captured {
    scrub(value);
  }
  let mut evidence = json!({"login":if options.login.is_some() { "yes" } else { "no" },"auth_mode":options.auth_mode(),"url":row.link.url,"expected":row.link.expected,"note":row.link.note,"explicit_photo_or_slide_count":row.link.count,"count_includes_video":row.link.count_is_total,"events":captured,"gallery_stderr":outcome.1,"ytdlp_log":log.lines().map(safe_line).collect::<Vec<_>>(),"reason":row.reason,"seconds":row.seconds});
  prepare_evidence(&mut evidence, options.authenticated());
  if let Err(error) = fs::write(
    folder.join("evidence.json"),
    serde_json::to_vec_pretty(&evidence).unwrap(),
  ) {
    row.verdict = "FAIL".into();
    row.reason = error.to_string();
    row.cause = "harness: evidence write".into();
  }
  group_state::cancel_group(&group);
  group_state::remove_group(&group);
  app
    .state::<crate::logging::LogStoreState>()
    .write()
    .remove_group(&group);
  row
}

fn scrub(value: &mut Value) {
  match value {
    Value::String(text) => *text = crate::parsers::ytdlp_error::redact_diagnostic(text),
    Value::Object(fields) => fields.values_mut().for_each(scrub),
    Value::Array(items) => items.iter_mut().for_each(scrub),
    _ => {}
  }
}

async fn pipeline(
  app: &tauri::AppHandle,
  link: &Link,
  group: &str,
  folder: &Path,
  bins: &Path,
  item_count: &tokio::sync::watch::Sender<usize>,
  login: Option<&str>,
) -> Result<(usize, usize, usize, Vec<String>), String> {
  // Clipboard normalization is production code; manual-paste validation is represented
  // by the URL parser fallback for public domains outside the clipboard allowlist.
  let url = crate::capture::matrix_normalize(&link.url).ok_or("Invalid HTTP(S) input URL")?;
  let entry = FetchEntry {
    group_id: group.into(),
    id: group.into(),
    url,
    total: 1,
    format: None,
    overrides: None,
    login_browser: login.map(str::to_owned),
    login_was_accepted: login.is_some(),
    prior_attempts: Vec::new(),
    prior_resolved_url: None,
  };
  let media = gallery::run_media_info(app, &entry)
    .await
    .map_err(|e| format!("fetch: {e:?}: {e}"))?
    .ok_or("fetch cancelled/no metadata")?;
  let mut media_items = Vec::new();
  match media {
    ParsedMedia::Single(item) => media_items.push((item, entry.url.clone())),
    ParsedMedia::Playlist(playlist) => {
      item_count.send_replace(playlist.entries.len().max(1));
      // The UI asks for a playlist selection. Exercise every returned entry, within
      // the same per-link time budget (no silent first-item-only success).
      for item in playlist.entries {
        let child = FetchEntry {
          url: item.video_url,
          total: 2,
          ..entry.clone()
        };
        match gallery::run_media_info(app, &child)
          .await
          .map_err(|e| format!("playlist fetch: {e:?}: {e}"))?
        {
          Some(ParsedMedia::Single(item)) => media_items.push((item, child.url.clone())),
          _ => return Err("Playlist child has no downloadable metadata".into()),
        }
      }
    }
    ParsedMedia::Livestream(_) => {
      return Err("Livestreams unsupported (production fetch_pipeline.rs)".into())
    }
  }
  if media_items.is_empty() {
    return Err("Empty playlist".into());
  }
  item_count.send_replace(
    media_items
      .iter()
      .map(|(item, _)| item.attachments.len().max(1))
      .sum(),
  );
  let mut totals = (0, 0, 0, Vec::new());
  let mut observed = HashSet::new();
  for (number, (item, fallback)) in media_items.into_iter().enumerate() {
    let expected_images = item
      .attachments
      .iter()
      .filter(|a| matches!(a.kind, crate::models::parsed::AttachmentKind::Image))
      .count();
    let expected_videos = item.attachments.len() - expected_images;
    let has_attachments = !item.attachments.is_empty();
    let format = FormatOptions {
      // src/stores/media/media.ts::getDownloadOptions uses Both at zero setup,
      // including audio-only sites. Do not silently repair default format failures.
      track_type: TrackType::Both,
      abr: None,
      height: None,
      fps: None,
      audio_encoding: None,
      video_encoding: None,
      audio_track: None,
      video_track: None,
    };
    let mut values = HashMap::from([("autonumber".into(), (number + 1).to_string())]);
    if let Some(title) = item.title {
      values.insert("fulltitle".into(), title.clone());
      values.insert("title".into(), title);
    }
    if let Some(uploader) = item.uploader {
      values.insert("uploader".into(), uploader);
    }
    let download = DownloadItem {
      fetch_engine: item.fetch_engine,
      resolved_url: item.resolved_url,
      id: format!("{group}-{number}"),
      url: item.url.unwrap_or(fallback),
      format,
      attachment_ids: (!item.attachments.is_empty())
        .then(|| item.attachments.iter().map(|a| a.id.clone()).collect()),
      attachments: item.attachments,
      // Production sends the row's kind; File must reach the file engine on download.
      media_kind: if has_attachments || matches!(item.media_kind, Some(MediaKind::File)) {
        item.media_kind
      } else {
        None
      },
      subtitle_inventory: Some(item.subtitle_inventory),
      overrides: Some(DownloadOverrides {
        output: Some(OutputOverrides {
          download_dir: Some(folder.to_string_lossy().into_owned()),
          partial_download: Some(PartialDownloadOverride {
            section: Some(DownloadSection {
              id: "matrix-budget".into(),
              start: "0".into(),
              end: "15".into(),
            }),
          }),
          ..Default::default()
        }),
        ..Default::default()
      }),
      template_context: TemplateContext { values },
    };
    // This is the production scheduler's DownloadItem -> DownloadEntry conversion,
    // followed by the identical worker function, including mixed/photo routing.
    let mut download = DownloadEntry::from((download, group.into()));
    download.login_browser = login.map(str::to_owned);
    download.login_was_accepted = login.is_some();
    gallery::run_media_download(app.clone(), download)
      .await
      .map_err(|e| format!("download: {e:?}: {e}"))?;
    let (images, videos, audio, files) = verify_files(
      folder,
      bins,
      &observed,
      link.expected == "file",
      link
        .note
        .split_whitespace()
        .find_map(|word| word.strip_prefix("sha256=")),
    )
    .await?;
    if link.expected != "file"
      && !matches_selection(expected_images, expected_videos, images, videos, audio)
    {
      return Err(format!("Request {number} saved {images} images/{videos} videos, but selected {expected_images} images/{expected_videos} videos"));
    }
    totals.0 += images;
    totals.1 += videos;
    totals.2 += audio;
    observed.extend(files.iter().cloned());
    totals.3.extend(files);
  }
  Ok(totals)
}

fn list_files(folder: &Path, files: &mut Vec<String>) -> Result<(), String> {
  for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
    let entry = entry.map_err(|e| e.to_string())?;
    let kind = entry.file_type().map_err(|e| e.to_string())?;
    if kind.is_dir() {
      list_files(&entry.path(), files)?;
    } else if kind.is_file() && entry.file_name() != "evidence.json" {
      files.push(entry.path().to_string_lossy().into_owned());
    }
  }
  files.sort();
  Ok(())
}

async fn command_output(binary: &Path, args: &[&str]) -> Result<std::process::Output, String> {
  tokio::process::Command::new(binary)
    .args(args)
    .kill_on_drop(true)
    .output()
    .await
    .map_err(|e| e.to_string())
}

async fn verify_files(
  folder: &Path,
  bins: &Path,
  observed: &HashSet<String>,
  generic_file: bool,
  checksum: Option<&str>,
) -> Result<(usize, usize, usize, Vec<String>), String> {
  let mut paths = Vec::new();
  list_files(folder, &mut paths)?;
  let mut files = Vec::new();
  let (mut images, mut videos, mut audio) = (0, 0, 0);
  for path in paths {
    if observed.contains(&path) {
      continue;
    }
    let candidate = Path::new(&path);
    let ext = candidate
      .extension()
      .and_then(|v| v.to_str())
      .unwrap_or_default();
    if matches!(
      ext,
      "part"
        | "ytdl"
        | "json"
        | "vtt"
        | "srt"
        | "ass"
        | "lrc"
        | "ttml"
        | "srv3"
        | "json3"
        | "description"
        | "temp"
    ) {
      continue;
    }
    if fs::metadata(candidate).map_err(|e| e.to_string())?.len() == 0 {
      return Err(format!("Empty output: {path}"));
    }
    if generic_file {
      verify_file(candidate, checksum)?;
      files.push(path);
      continue;
    }
    let probe = command_output(
      &bins.join("ffprobe"),
      &[
        "-v",
        "error",
        "-show_streams",
        "-show_format",
        "-of",
        "json",
        &path,
      ],
    )
    .await?;
    if !probe.status.success() {
      return Err(format!(
        "ffprobe failed for {path}: {}",
        safe_line(&String::from_utf8_lossy(&probe.stderr))
      ));
    }
    let metadata: Value = serde_json::from_slice(&probe.stdout).map_err(|e| e.to_string())?;
    let streams = metadata["streams"]
      .as_array()
      .ok_or("ffprobe: no streams")?;
    // Signature sniffing plus actual decoder prevents HTML/error bodies named .jpg passing.
    let image = crate::runners::gallery_files::read_image(candidate)
      .ok()
      .and_then(|bytes| crate::runners::gallery_files::image_type(&bytes).ok())
      .is_some();
    if image {
      let decoded = command_output(
        &bins.join("ffmpeg"),
        &[
          "-v",
          "error",
          "-nostdin",
          "-i",
          &path,
          "-frames:v",
          "1",
          "-f",
          "null",
          "-",
        ],
      )
      .await?;
      if !decoded.status.success() {
        return Err(format!("Image decode failed: {path}"));
      }
      images += 1;
    } else if streams
      .iter()
      .any(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
    {
      videos += 1;
    } else if streams.iter().any(|s| s["codec_type"] == "audio") {
      audio += 1;
    } else {
      return Err(format!("No playable stream: {path}"));
    }
    files.push(path);
  }
  if files.is_empty() {
    return Err("Pipeline finished without any verified media files".into());
  }
  Ok((images, videos, audio, files))
}

fn root_cause(codes: &[String], evidence: &str) -> String {
  let lower = evidence.to_ascii_lowercase();
  if lower.contains("sync semaphore") || lower.contains("semctl: operation not permitted") {
    return "sandbox: bundled gallery-dl PyInstaller semaphore denied before extraction (src/runners/gallery_process.rs:164)".into();
  }
  if lower.contains("deadline") {
    return "budget: bounded item-count link deadline".into();
  }
  if lower.contains("unsupported url") || codes.iter().any(|c| c == "urlNotSupported") {
    return "extractor coverage / resolve fallback (src/runners/ytdlp_info.rs)".into();
  }
  if lower.contains("403") || codes.iter().any(|c| c == "accessForbidden403") {
    return "anonymous access/CDN forbidden (src/runners/gallery.rs; src/runners/ytdlp_info.rs)"
      .into();
  }
  if lower.contains("429")
    || lower.contains("rate limit")
    || codes.iter().any(|c| c == "rateLimited429")
  {
    return "remote rate limit (src/runners/gallery.rs; src/runners/ytdlp_info.rs)".into();
  }
  if lower.contains("imageengineunavailable") {
    return "gallery sidecar unavailable (src/runners/gallery.rs::bundled_path)".into();
  }
  if lower.contains("requested format") || codes.iter().any(|c| c == "requestedFormatUnavailable") {
    return "default format selection / engine formats (src/runners/ytdlp_args/format_args.rs)"
      .into();
  }
  if let Some(code) = codes.last() {
    return format!("{code} (src/runners/gallery.rs; src/runners/ytdlp_info.rs)");
  }
  "engine/reader failure: inspect evidence.json (src/runners/gallery.rs; src/runners/ytdlp_info.rs)"
    .into()
}

fn cell(value: &str, markdown: bool) -> String {
  let value = value.replace(['\t', '\n', '\r'], " ");
  if markdown {
    value.replace('|', "\\|")
  } else {
    value
  }
}

fn write_reports(
  options: &Options,
  run_dir: &Path,
  rows: &[ResultRow],
  total: usize,
) -> Result<(), String> {
  let headers = [
    "site",
    "label",
    "expected",
    "got",
    "verdict",
    "reason",
    "files",
    "app_code_message_key",
    "engine_last_error",
    "root_cause",
    "url",
    "seconds",
    "login",
    "auth_mode",
    "stages",
    "winner",
    "final_code",
  ];
  let mut tsv = format!("{}\n", headers.join("\t"));
  let mut md = format!("# Link matrix\n\nCompleted {}/{} rows. Run files: `{}`.\n\nProduction: gallery::run_media_info (including resolve readers), DownloadItem→DownloadEntry, gallery::run_media_download→gallery/yt-dlp, default argument builders, error parser and login consent. Stand-ins: windowless real Tauri host; isolated default state; direct scheduler worker calls (queue/UI selection omitted); clipboard normalization plus HTTP URL validation; all playlist entries selected; first 15s AV section and output folder override. {} No saved credentials/vault loaded. Images decode with app ffmpeg, streams checked with app ffprobe.\n\nInputs: {}\n\n", rows.len(), total, run_dir.display(), if options.authenticated() { "Explicit local authentication supplied; raw engine output omitted." } else { "No login consent accepted." }, options.inputs.iter().map(|p| format!("`{}`", p.display())).collect::<Vec<_>>().join(", "));
  let mut counts = BTreeMap::<&str, usize>::new();
  let mut groups = BTreeMap::<&str, usize>::new();
  for row in rows {
    *counts.entry(&row.verdict).or_default() += 1;
    if row.verdict == "FAIL" {
      *groups.entry(&row.cause).or_default() += 1;
    }
  }
  md.push_str(&format!(
    "Totals: {}\n\nFAIL groups (engine evidence classification; not proof of a local bug):\n\n",
    counts
      .iter()
      .map(|(k, v)| format!("{k}={v}"))
      .collect::<Vec<_>>()
      .join(", ")
  ));
  for (group, count) in groups {
    md.push_str(&format!("- {count}: {group}\n"));
  }
  md.push_str(&format!(
    "\n| {} |\n| {} |\n",
    headers.join(" | "),
    vec!["---"; headers.len()].join(" | ")
  ));
  for row in rows {
    let fields = [
      &row.link.site,
      &row.link.label,
      &row.link.expected,
      &row.got,
      &row.verdict,
      &row.reason,
      &row.files.join("; "),
      &row.ui,
      &row.engine,
      &row.cause,
      &row.link.url,
      &format!("{:.1}", row.seconds),
      &if options.login.is_some() { "yes" } else { "no" }.into(),
      &options.auth_mode(),
      &row.attempts.stages,
      &row.attempts.winner,
      &row.attempts.final_code,
    ]
    .map(|v| v.to_string());
    tsv.push_str(&format!(
      "{}\n",
      fields
        .iter()
        .map(|v| cell(v, false))
        .collect::<Vec<_>>()
        .join("\t")
    ));
    md.push_str(&format!(
      "| {} |\n",
      fields
        .iter()
        .map(|v| cell(v, true))
        .collect::<Vec<_>>()
        .join(" | ")
    ));
  }
  for (name, body) in [("results.tsv", tsv), ("results.md", md)] {
    let temporary = options.out.join(format!("{name}.tmp"));
    fs::write(&temporary, body).map_err(|e| e.to_string())?;
    fs::rename(temporary, options.out.join(name)).map_err(|e| e.to_string())?;
  }
  Ok(())
}
