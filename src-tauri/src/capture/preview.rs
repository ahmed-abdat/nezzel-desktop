use super::choice::{Choice, MAX_FPS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Stdio;
use tokio::{io::AsyncReadExt, process::Command};

/// Plain reasons the bar and card can explain: `login`, `unavailable` (removed or private), `unsupported`,
/// `region`, `protected`, `busy` (rate limited), `network`, `slow` (timed out), `unknown`.
/// The webview words each as `capture.fail.<reason>.title/.hint` (vocabulary rendered by `crate::contract`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailReason {
  Login,
  Unavailable,
  Unsupported,
  Region,
  Protected,
  Busy,
  Network,
  Slow,
  Unknown,
}

impl FailReason {
  #[cfg_attr(not(test), allow(dead_code))]
  pub const ALL: [Self; 9] = [
    Self::Login,
    Self::Unavailable,
    Self::Unsupported,
    Self::Region,
    Self::Protected,
    Self::Busy,
    Self::Network,
    Self::Slow,
    Self::Unknown,
  ];

  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Login => "login",
      Self::Unavailable => "unavailable",
      Self::Unsupported => "unsupported",
      Self::Region => "region",
      Self::Protected => "protected",
      Self::Busy => "busy",
      Self::Network => "network",
      Self::Slow => "slow",
      Self::Unknown => "unknown",
    }
  }
}

pub fn reason_for_code(code: &str) -> FailReason {
  match code {
    code if crate::extract::outcome::LOGIN_CODES.contains(&code) => FailReason::Login,
    "videoNotFoundOrPrivate"
    | "playlistPrivateOrMissing"
    | "notFound404"
    | "mediaUnavailable"
    | "channelUnavailable"
    | "copyrightClaimed"
    | "accessForbidden403" => FailReason::Unavailable,
    "urlNotSupported" | "urlUnsupported" | "unknownUrlType" | "noMediaFound" | "embedOnly" => {
      FailReason::Unsupported
    }
    "geoBlocked" => FailReason::Region,
    "drmProtected" => FailReason::Protected,
    "rateLimited429" | "blockedGeneric" => FailReason::Busy,
    "networkNameResolutionFailed" | "unableToConnectToProxy" | "sslVerifyFailed" | "server5xx" => {
      FailReason::Network
    }
    _ => FailReason::Unknown,
  }
}

/// The first recognised yt-dlp error line decides the reason; anything unrecognised is `unknown`.
pub fn fail_reason(stderr: &str) -> FailReason {
  use crate::parsers::ytdlp_error::{DiagnosticMatcher, YtdlpErrorParser};
  static PARSER: std::sync::OnceLock<Option<YtdlpErrorParser>> = std::sync::OnceLock::new();
  let Some(parser) = PARSER
    .get_or_init(|| {
      DiagnosticMatcher::shared()
        .ok()
        .map(|matcher| YtdlpErrorParser::new("capture", "capture", matcher))
    })
    .as_ref()
  else {
    return FailReason::Unknown;
  };
  stderr
    .lines()
    .filter_map(|line| parser.parse_fetch_line(line, false))
    .map(|event| reason_for_code(&event.code))
    .find(|reason| *reason != FailReason::Unknown)
    .unwrap_or(FailReason::Unknown)
}

/// What the quick bar shows about a link: plain facts, never raw format ids.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDetails {
  /// https only; shown as an <img>, never fetched by the backend.
  pub thumbnail: Option<String>,
  pub media_kind: Option<String>,
  pub photos: Option<u64>,
  pub duration: Option<u64>,
  pub uploader: String,
  pub views: Option<u64>,
  /// Height of the best video format, or None for audio-only media.
  pub best_height: Option<u64>,
  pub best_ext: String,
  /// Bytes of the best video + best audio when the site reports them.
  pub size: Option<u64>,
  /// The link has a sound track (separate or muxed): the quick "Audio" pick applies.
  pub has_audio: bool,
  /// Bytes of the best audio-only format, when the site reports it.
  pub audio_size: Option<u64>,
  /// The quick "Smaller" pick: the highest height at or under 720 below the best, else the next lower one.
  /// None when there is only one quality (no smaller pick to offer).
  pub small_height: Option<u64>,
  /// Bytes of that smaller video (+ best audio when it has none of its own).
  pub small_size: Option<u64>,
  /// The copied-link card's format menu: one row per height (and 60 fps tier), then audio bitrates.
  #[serde(default)]
  pub choices: Vec<CardChoice>,
}

/// One row of the copied-link card's format menu. Plain facts only, never a raw format id or URL.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardChoice {
  /// A video row (with sound when the link has any; its own row for a frame rate ≥ `HIGH_FPS_MIN`) or an
  /// audio row (a bitrate, or none for sound only inside the video). What the row sends when picked.
  pub choice: Choice,
  /// Bytes, video + best audio when the video has no sound of its own.
  pub size: Option<u64>,
}

const MAX_VIDEO_CHOICES: usize = 24;
const MAX_AUDIO_CHOICES: usize = 4;

/// Quick "Smaller" pick ceiling; the webview's own Smaller pick uses the same number (`crate::contract`).
pub const SMALL_MAX_HEIGHT: u64 = 720;
/// Rounded frame rates from here up are a high frame rate tier: a 60 fps row is its own choice.
pub const HIGH_FPS_MIN: u64 = 48;

fn text(item: &Value, key: &str, max: usize) -> String {
  item
    .get(key)
    .and_then(Value::as_str)
    .unwrap_or("")
    .trim()
    .chars()
    .take(max)
    .collect()
}

fn bytes(format: &Value) -> Option<u64> {
  ["filesize", "filesize_approx"]
    .iter()
    .find_map(|key| format.get(*key).and_then(Value::as_f64))
    // Site metadata is untrusted: anything past 1 TiB is noise, and the sum below saturates.
    .filter(|v| v.is_finite() && *v > 0.0 && *v < (1u64 << 40) as f64)
    .map(|v| v as u64)
}

pub fn summarize(item: &Value) -> PreviewDetails {
  let media_kind = if item.get("entries").is_some_and(Value::is_array) {
    Some("playlist".to_owned())
  } else {
    item
      .get("mediaKind")
      .and_then(Value::as_str)
      .map(str::to_owned)
  };
  let photos_only = matches!(media_kind.as_deref(), Some("image" | "carousel"));
  let formats: Vec<&Value> = item
    .get("formats")
    .and_then(Value::as_array)
    .filter(|_| !photos_only)
    .map(|f| f.iter().take(400).collect())
    .unwrap_or_default();
  // Raw yt-dlp formats name codecs (`acodec: "mp4a"`, `"none"`); the app's parsed model lists them
  // (`audioCodecs: [...]`). Gallery previews arrive parsed, so both shapes count.
  let codec = |f: &Value, key: &str| {
    let listed = match key {
      "acodec" => "audioCodecs",
      _ => "videoCodecs",
    };
    f.get(key)
      .and_then(Value::as_str)
      .is_some_and(|c| c != "none")
      || f
        .get(listed)
        .and_then(Value::as_array)
        .is_some_and(|codecs| !codecs.is_empty())
  };
  let height = |f: &Value| f.get("height").and_then(Value::as_u64).filter(|h| *h > 0);
  let is_video =
    |f: &Value| height(f).is_some() && (codec(f, "vcodec") || f.get("vcodec").is_none());
  let best_video = formats
    .iter()
    .filter(|f| is_video(f))
    .max_by_key(|f| (height(f), bytes(f)));
  let best_audio = formats
    .iter()
    .filter(|f| codec(f, "acodec") && !codec(f, "vcodec"))
    .max_by_key(|f| f.get("abr").and_then(Value::as_f64).map(|v| v as u64));
  let video_has_audio = best_video.is_some_and(|f| codec(f, "acodec"));
  let audio_bytes = best_audio.and_then(|f| bytes(f));
  // A video's download size: its bytes, plus the best audio when it carries no sound of its own.
  let with_audio = |video: &Value| match (bytes(video), audio_bytes) {
    (Some(v), Some(a)) if !codec(video, "acodec") => Some(v.saturating_add(a)),
    (v, _) => v,
  };
  let size = if media_kind.as_deref() == Some("file") {
    bytes(item)
  } else {
    match best_video {
      Some(video) => with_audio(video),
      None => audio_bytes,
    }
  };
  let best_height = best_video.and_then(|f| height(f));
  let small_video = best_height.and_then(|best| {
    let lower = formats.iter().filter(|f| {
      height(f).is_some_and(|h| h < best) && (codec(f, "vcodec") || f.get("vcodec").is_none())
    });
    let under_cap = lower
      .clone()
      .filter(|f| height(f).is_some_and(|h| h <= SMALL_MAX_HEIGHT))
      .max_by_key(|f| (height(f), bytes(f)));
    under_cap.or_else(|| lower.max_by_key(|f| (height(f), bytes(f))))
  });
  let choices = if media_kind.as_deref() == Some("file") {
    Vec::new()
  } else {
    card_choices(&formats, &is_video, &height, &with_audio, video_has_audio)
  };
  let best_ext = match best_video {
    Some(f) if video_has_audio => text(f, "ext", 8),
    Some(_) => "mp4".into(),
    None => best_audio.map(|f| text(f, "ext", 8)).unwrap_or_default(),
  };
  PreviewDetails {
    media_kind,
    photos: item
      .get("attachments")
      .and_then(Value::as_array)
      .map(|attachments| {
        attachments
          .iter()
          .filter(|attachment| attachment.get("kind").and_then(Value::as_str) == Some("image"))
          .count() as u64
      }),
    thumbnail: item
      .get("thumbnail")
      .and_then(Value::as_str)
      .filter(|u| u.starts_with("https://") && u.len() <= 2048)
      .map(str::to_owned),
    duration: item
      .get("duration")
      .and_then(Value::as_f64)
      .filter(|d| d.is_finite() && *d > 0.0)
      .map(|d| d.round() as u64),
    uploader: [text(item, "uploader", 120), text(item, "channel", 120)]
      .into_iter()
      .find(|s| !s.is_empty())
      .unwrap_or_default(),
    views: ["view_count", "views"]
      .iter()
      .find_map(|key| item.get(*key).and_then(Value::as_u64)),
    best_height,
    best_ext,
    size,
    has_audio: best_audio.is_some() || video_has_audio,
    audio_size: audio_bytes,
    small_height: small_video.and_then(|f| height(f)),
    small_size: small_video.and_then(|f| with_audio(f)),
    choices,
  }
}

/// High frame rate tier of a format (rounded fps ≥ `HIGH_FPS_MIN`), None for ordinary frame rates.
fn fps_tier(format: &Value) -> Option<u64> {
  format
    .get("fps")
    .and_then(Value::as_f64)
    .filter(|v| v.is_finite() && (HIGH_FPS_MIN as f64..=MAX_FPS as f64).contains(v))
    .map(|v| v.round() as u64)
}

fn abr(format: &Value) -> Option<u64> {
  format
    .get("abr")
    .and_then(Value::as_f64)
    .filter(|v| v.is_finite() && *v > 0.0 && *v < 10_000.0)
    .map(|v| v.round() as u64)
}

/// Menu rows: videos deduped by (height, fps tier), the biggest file of each tier, highest first;
/// audio deduped by bitrate (within 10 % counts as the same), highest first. Numbers past the `Choice` limits
/// (site metadata is untrusted) leave the row out.
fn card_choices(
  formats: &[&Value],
  is_video: &dyn Fn(&Value) -> bool,
  height: &dyn Fn(&Value) -> Option<u64>,
  with_audio: &dyn Fn(&Value) -> Option<u64>,
  video_has_audio: bool,
) -> Vec<CardChoice> {
  let mut videos: Vec<&Value> = formats.iter().copied().filter(|f| is_video(f)).collect();
  videos.sort_by_key(|f| std::cmp::Reverse((height(f), fps_tier(f), bytes(f))));
  let mut out: Vec<CardChoice> = Vec::new();
  for video in videos {
    let Some(choice) = height(video).and_then(|h| Choice::video(h, fps_tier(video))) else {
      continue;
    };
    if out.iter().any(|c| c.choice == choice) {
      continue;
    }
    out.push(CardChoice {
      choice,
      size: with_audio(video),
    });
    if out.len() == MAX_VIDEO_CHOICES {
      break;
    }
  }
  let audio_only = |f: &&Value| {
    let has = |key: &str, listed: &str| {
      f.get(key)
        .and_then(Value::as_str)
        .is_some_and(|c| c != "none")
        || f
          .get(listed)
          .and_then(Value::as_array)
          .is_some_and(|codecs| !codecs.is_empty())
    };
    has("acodec", "audioCodecs") && !has("vcodec", "videoCodecs") && height(f).is_none()
  };
  let mut audios: Vec<&Value> = formats.iter().copied().filter(audio_only).collect();
  audios.sort_by_key(|f| std::cmp::Reverse((abr(f), bytes(f))));
  let mut audio: Vec<CardChoice> = Vec::new();
  for format in audios {
    let rate = abr(format);
    let Some(choice) = Choice::audio(rate) else {
      continue;
    };
    let near = |c: &CardChoice| match (c.choice, rate) {
      (Choice::Audio { kbps: Some(a) }, Some(b)) => a.abs_diff(b) * 10 <= a.max(b),
      (Choice::Audio { kbps }, b) => kbps == b,
      _ => false,
    };
    if audio.iter().any(near) {
      continue;
    }
    audio.push(CardChoice {
      choice,
      size: bytes(format),
    });
    if audio.len() == MAX_AUDIO_CHOICES {
      break;
    }
  }
  // Sound only inside the video (no separate track): one plain audio row, the app extracts it.
  if audio.is_empty() && video_has_audio {
    audio.push(CardChoice {
      choice: Choice::Audio { kbps: None },
      size: None,
    });
  }
  out.extend(audio);
  out
}

#[cfg(test)]
pub async fn read_output(
  command: &mut Command,
  process_guard: tokio::sync::OwnedRwLockReadGuard<()>,
) -> Option<Vec<u8>> {
  read_output_reason(command, process_guard).await.ok()
}

/// Like `read_output`, but a failure says why (`fail_reason` over at most 64 KiB of stderr, or `slow`).
pub async fn read_output_reason(
  command: &mut Command,
  process_guard: tokio::sync::OwnedRwLockReadGuard<()>,
) -> Result<Vec<u8>, FailReason> {
  command
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
  let mut child = command.spawn().map_err(|_| FailReason::Unknown)?;
  // Drained concurrently so a chatty stderr can never block stdout; capped, never forwarded as text.
  let stderr = child.stderr.take().map(|stderr| {
    tokio::spawn(async move {
      let mut bytes = Vec::new();
      let _ = stderr.take(64 * 1024).read_to_end(&mut bytes).await;
      String::from_utf8_lossy(&bytes).into_owned()
    })
  });
  // One engine's read (`read::READ_LIMIT`). Cancellation signals the owner to kill + reap before releasing its process guard.
  let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
  let owner = tokio::spawn(async move {
    let _process_guard = process_guard;
    let read = tokio::time::timeout(super::read::READ_LIMIT, async {
      let stdout = child.stdout.take()?;
      let mut bytes = Vec::new();
      stdout
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
      if bytes.len() > 8 * 1024 * 1024 || !child.wait().await.ok()?.success() {
        return None;
      }
      Some(bytes)
    });
    let result = tokio::select! {
      result = read => result.map_err(|_| FailReason::Slow),
      _ = cancelled => Ok(None),
    };
    // Also reap on oversized output and timeout; kill_on_drop alone does not wait.
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
  });
  let result = owner.await.unwrap_or(Ok(None));
  drop(cancel);
  match result {
    Ok(Some(bytes)) => Ok(bytes),
    Err(reason) => Err(reason),
    Ok(None) => match stderr {
      Some(task) => Err(fail_reason(&task.await.unwrap_or_default())),
      None => Err(FailReason::Unknown),
    },
  }
}

#[cfg(test)]
mod summary_tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn playlist_previews_are_identified_even_with_no_entries() {
    for entries in [json!([]), json!([{"url": "https://example.com/video"}])] {
      assert_eq!(
        summarize(&json!({"entries": entries}))
          .media_kind
          .as_deref(),
        Some("playlist")
      );
    }
  }

  /// Gallery previews arrive in the parsed shape: listed codecs still count as sound.
  #[test]
  fn summarize_reads_the_parsed_shape() {
    let item = serde_json::json!({
      "views": 9,
      "formats": [
        {"id": "v", "height": 720, "audioCodecs": [], "videoCodecs": [{"id": "avc1", "label": "H.264"}]},
        {"id": "a", "abr": 128, "audioCodecs": [{"id": "aac", "label": "AAC"}], "videoCodecs": []}
      ]
    });
    let details = summarize(&item);
    assert!(details.has_audio);
    assert_eq!(details.best_height, Some(720));
    assert_eq!(details.views, Some(9));
  }

  #[test]
  fn summarizes_file_size_without_inventing_video_or_audio_quality() {
    let details =
      summarize(&json!({"mediaKind":"file", "title":"report.pdf", "filesize":4096, "formats":[]}));
    assert_eq!(
      (
        details.media_kind.as_deref(),
        details.size,
        details.best_height,
        details.best_ext.as_str()
      ),
      (Some("file"), Some(4096), None, "")
    );
  }

  #[test]
  fn failed_reads_say_why_in_a_fixed_vocabulary() {
    for (stderr, reason) in [
      ("ERROR: [youtube] abc: Sign in to confirm you're not a bot", "login"),
      ("ERROR: [youtube] abc: Join this channel to get access to members-only content", "login"),
      ("ERROR: [youtube] abc: Private video. Sign in if you've been granted access", "login"),
      ("ERROR: [vimeo] 123: Video has not been found", "unavailable"),
      ("ERROR: Unsupported URL: https://example.com/page", "unsupported"),
      ("ERROR: [youtube] abc: The uploader has not made this video available in your country", "region"),
      ("WARNING: something harmless\nERROR: [generic] Unable to download webpage: HTTP Error 429: Too Many Requests", "busy"),
      ("ERROR: Unable to download webpage: <urlopen error [Errno 11001] getaddrinfo failed>", "network"),
      ("some unrecognised failure", "unknown"),
      ("", "unknown"),
    ] {
      assert_eq!(fail_reason(stderr).as_str(), reason, "{stderr}");
    }
  }

  #[test]
  fn quick_picks_offer_the_smaller_quality_only_when_one_exists() {
    let fmt = |h: u64, bytes: u64| json!({"height": h, "vcodec": "h264", "acodec": "none", "filesize": bytes});
    let audio = json!({"abr": 128.0, "vcodec": "none", "acodec": "aac", "filesize": 10});
    // (heights, expected small height, expected small size)
    let cases: [(&[u64], Option<u64>, Option<u64>); 5] = [
      (&[2160, 1080, 720, 480], Some(720), Some(730)), // best ≤ 720 below the best
      (&[1080, 480, 360], Some(480), Some(490)),       // no 720: the nearest lower under the cap
      (&[2160, 1440], Some(1440), Some(1450)),         // nothing ≤ 720: the next lower one
      (&[720], None, None),                            // single quality: no Smaller pick
      (&[], None, None),                               // audio only: no video picks
    ];
    for (heights, small, size) in cases {
      let mut formats: Vec<Value> = heights.iter().map(|h| fmt(*h, *h)).collect();
      formats.push(audio.clone());
      let d = summarize(&json!({ "formats": formats }));
      assert_eq!(
        (d.small_height, d.small_size),
        (small, size),
        "heights {heights:?}"
      );
      assert!(d.has_audio);
      assert_eq!(d.audio_size, Some(10));
    }
    let silent = summarize(&json!({"formats": [fmt(1080, 1)]}));
    assert!(!silent.has_audio, "a video-only link offers no Audio pick");
  }

  #[test]
  fn card_choices_dedupe_heights_and_bitrates_highest_first() {
    let video = |h: u64, fps: f64, bytes: u64| json!({"height": h, "fps": fps, "vcodec": "vp9", "acodec": "none", "filesize": bytes});
    let audio = |abr: f64, bytes: u64| json!({"abr": abr, "vcodec": "none", "acodec": "opus", "filesize": bytes});
    let d = summarize(&json!({ "formats": [
      video(720, 30.0, 70), video(1080, 30.0, 300), video(1080, 30.0, 200), video(1080, 60.0, 500),
      video(360, 25.0, 20), audio(129.0, 13), audio(130.4, 14), audio(48.0, 5), audio(160.0, 16),
    ]}));
    let rows: Vec<(Choice, Option<u64>)> = d.choices.iter().map(|c| (c.choice, c.size)).collect();
    let video = |height, fps| Choice::Video { height, fps };
    let audio = |kbps| Choice::Audio { kbps: Some(kbps) };
    assert_eq!(
      rows,
      [
        // 60 fps is its own row; the bigger of two same-height files wins; size adds the best audio (16).
        (video(1080, Some(60)), Some(516)),
        (video(1080, None), Some(316)),
        (video(720, None), Some(86)),
        (video(360, None), Some(36)),
        // 130 and 129 kbps are one row (within 10 %).
        (audio(160), Some(16)),
        (audio(130), Some(14)),
        (audio(48), Some(5)),
      ]
    );
  }

  #[test]
  fn card_choices_for_muxed_photos_files_and_absurd_values() {
    // One muxed format: a single video row plus a plain audio row (the app extracts the sound).
    let muxed = summarize(
      &json!({"formats": [{"height": 720, "vcodec": "h264", "acodec": "aac", "filesize": 9, "fps": 1e9}]}),
    );
    assert_eq!(muxed.choices.len(), 2);
    assert_eq!(
      muxed.choices[0].choice,
      Choice::Video {
        height: 720,
        fps: None
      },
      "absurd fps is ignored"
    );
    assert_eq!(muxed.choices[1].choice, Choice::Audio { kbps: None });
    // Heights past the `Choice` limit are no row at all; the sane one stays.
    let tall = summarize(&json!({"formats": [
      {"height": 99_999, "vcodec": "h264", "acodec": "none"},
      {"height": 1080, "vcodec": "h264", "acodec": "none"},
    ]}));
    assert_eq!(
      tall.choices.iter().map(|c| c.choice).collect::<Vec<_>>(),
      [Choice::Video {
        height: 1080,
        fps: None
      }]
    );
    let photos =
      summarize(&json!({"mediaKind": "carousel", "formats": [{"height": 1080, "vcodec": "h264"}]}));
    assert!(photos.choices.is_empty());
    let file =
      summarize(&json!({"mediaKind": "file", "formats": [{"height": 1080, "vcodec": "h264"}]}));
    assert!(file.choices.is_empty());
    let many: Vec<Value> = (1..=60)
      .map(|h| json!({"height": h * 10, "vcodec": "h264"}))
      .collect();
    assert_eq!(
      summarize(&json!({ "formats": many })).choices.len(),
      MAX_VIDEO_CHOICES
    );
  }

  #[test]
  fn summarizes_facts_and_never_raw_format_ids() {
    let item = json!({
      "thumbnail": "https://cdn.example/t.jpg", "duration": 718.4, "uploader": "Ahmed", "view_count": 42400,
      "formats": [
        {"format_id": "903814006058504v", "height": 1280, "vcodec": "h264", "acodec": "none", "ext": "mp4", "filesize": 100},
        {"format_id": "903813999391838v", "height": 1920, "vcodec": "h264", "acodec": "none", "ext": "mp4", "filesize_approx": 300.0},
        {"format_id": "902828846157020a", "abr": 88.0, "vcodec": "none", "acodec": "aac", "ext": "m4a", "filesize": 20}
      ]
    });
    let d = summarize(&item);
    assert_eq!(d.thumbnail.as_deref(), Some("https://cdn.example/t.jpg"));
    assert_eq!(
      (d.duration, d.uploader.as_str(), d.views),
      (Some(718), "Ahmed", Some(42400))
    );
    assert_eq!(
      (d.best_height, d.best_ext.as_str(), d.size),
      (Some(1920), "mp4", Some(320))
    );
  }

  #[test]
  fn summarizes_gallery_photos_from_the_serialized_media_item() {
    let post = crate::parsers::gallery::parse_gallery(
      br#"[[3,"https://cdn.example/first.jpg",{"extension":"jpg"}],
          [3,"https://cdn.example/clip.mp4",{"extension":"mp4"}],
          [3,"https://cdn.example/second.png",{"extension":"png"}],
          [3,"https://cdn.example/third.webp",{"extension":"webp"}]]"#,
      "https://x.com/user/status/123",
    )
    .unwrap();
    for (post, kind) in [
      (post.clone(), "mixed"),
      (
        {
          let mut images = post;
          images.assets.remove(1);
          images
        },
        "carousel",
      ),
    ] {
      let mut item = serde_json::to_value(post.into_media("capture-1".into())).unwrap();
      if kind == "carousel" {
        item["formats"] = json!([{"height": 1080, "vcodec": "h264", "ext": "mp4"}]);
      }
      let details = serde_json::to_value(summarize(&item)).unwrap();
      assert_eq!(details["mediaKind"], kind);
      assert_eq!(details["photos"], 3);
      assert_eq!(details["thumbnail"], "https://cdn.example/first.jpg");
      assert!(details["bestHeight"].is_null());
      assert_eq!(details["bestExt"], "");
      item["thumbnail"] = json!("http://cdn.example/first.jpg");
      assert!(summarize(&item).thumbnail.is_none());
    }
  }

  #[test]
  fn audio_only_and_unsafe_thumbnails() {
    let item = json!({
      "thumbnail": "http://insecure/t.jpg", "channel": "Label",
      "formats": [{"abr": 128.0, "vcodec": "none", "acodec": "opus", "ext": "webm", "filesize": 5}]
    });
    let d = summarize(&item);
    assert_eq!(d.thumbnail, None);
    assert_eq!(
      (
        d.best_height,
        d.best_ext.as_str(),
        d.size,
        d.uploader.as_str()
      ),
      (None, "webm", Some(5), "Label")
    );
  }

  #[test]
  fn absurd_sizes_are_ignored() {
    let item = json!({"formats": [
      {"height": 720, "vcodec": "avc1", "acodec": "none", "ext": "mp4", "filesize": 1.0e30},
      {"abr": 128.0, "vcodec": "none", "acodec": "opus", "ext": "webm", "filesize": 7}
    ]});
    assert_eq!(summarize(&item).size, None);
  }

  #[test]
  fn progressive_format_keeps_its_own_container_and_size() {
    let item = json!({"formats": [{"height": 720, "vcodec": "avc1", "acodec": "mp4a", "ext": "mp4", "filesize": 9}]});
    let d = summarize(&item);
    assert_eq!(
      (d.best_height, d.best_ext.as_str(), d.size),
      (Some(720), "mp4", Some(9))
    );
  }
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;
  use std::{
    io::Write,
    sync::{Arc, Mutex},
  };

  /// The word the card shows for a failed read (the webview gets the vocabulary from `crate::contract`).
  #[test]
  fn failure_codes_map_to_the_fixed_reason_vocabulary() {
    for (code, reason) in [
      ("signInRequired", "login"),
      ("signInRequiredForBotDetection", "login"),
      ("membersOnly", "login"),
      ("videoPasswordRequired", "login"),
      ("videoNotFoundOrPrivate", "unavailable"),
      ("playlistPrivateOrMissing", "unavailable"),
      ("notFound404", "unavailable"),
      ("mediaUnavailable", "unavailable"),
      ("channelUnavailable", "unavailable"),
      ("copyrightClaimed", "unavailable"),
      ("accessForbidden403", "unavailable"),
      ("urlNotSupported", "unsupported"),
      ("urlUnsupported", "unsupported"),
      ("unknownUrlType", "unsupported"),
      ("noMediaFound", "unsupported"),
      ("embedOnly", "unsupported"),
      ("geoBlocked", "region"),
      ("drmProtected", "protected"),
      ("rateLimited429", "busy"),
      ("blockedGeneric", "busy"),
      ("networkNameResolutionFailed", "network"),
      ("unableToConnectToProxy", "network"),
      ("sslVerifyFailed", "network"),
      ("server5xx", "network"),
      // characterisation: codes the engine classes elsewhere fall to "unknown" here (no "slow" from a code).
      ("livestreamNotStarted", "unknown"),
      ("ageRestricted", "unknown"),
      ("photosOnly", "unknown"),
      ("fileNotFile", "unknown"),
      ("imageSourceUnavailable", "unknown"),
      ("", "unknown"),
    ] {
      assert_eq!(reason_for_code(code).as_str(), reason, "{code}");
    }
  }

  struct LogWriter(Arc<Mutex<Vec<u8>>>);
  impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
      self.0.lock().unwrap().extend_from_slice(bytes);
      Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
      Ok(())
    }
  }

  #[tokio::test]
  async fn cancelled_preview_keeps_update_lock_until_process_is_reaped() {
    use std::os::unix::ffi::OsStrExt;
    let root = std::env::temp_dir().join(format!("nezzel-preview-lock-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let fifo = root.join("input");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let input = fifo.clone();
    let pid_path = root.join("pid");
    let child_pid_path = pid_path.clone();
    let caller = tokio::spawn(async move {
      let mut command = Command::new("/bin/sh");
      command.args(["-c", "echo $$ > \"$1\"; exec /bin/cat \"$2\"", "sh"]);
      command.arg(child_pid_path).arg(input);
      let guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
      read_output(&mut command, guard).await
    });
    // Opening the writer completes only once the child has opened the FIFO.
    let writer = tokio::task::spawn_blocking(move || {
      std::fs::OpenOptions::new().write(true).open(fifo).unwrap()
    })
    .await
    .unwrap();
    let pid: i32 = std::fs::read_to_string(pid_path)
      .unwrap()
      .trim()
      .parse()
      .unwrap();
    assert!(crate::binaries::YTDLP_PROCESSES.try_write().is_err());
    caller.abort();
    let _ = caller.await;
    let _reaped = crate::binaries::YTDLP_PROCESSES.write().await;
    assert_eq!(
      unsafe { libc::kill(pid, 0) },
      -1,
      "write lock acquired before child was reaped"
    );
    assert_eq!(
      std::io::Error::last_os_error().raw_os_error(),
      Some(libc::ESRCH)
    );
    drop(writer);
    std::fs::remove_dir_all(root).unwrap();
  }

  #[tokio::test]
  async fn failed_capture_fetch_discards_url_from_output_logs_and_completion_event() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
      .with_max_level(tracing::Level::TRACE)
      .without_time()
      .with_ansi(false)
      .with_writer(move || LogWriter(sink.clone()))
      .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let secret = "https://youtu.be/private?token=capture-secret";
    let mut command = Command::new("/bin/sh");
    command.args([
      "-c",
      "printf '%s' \"$1\"; printf '%s' \"$1\" >&2; exit 2",
      "sh",
      secret,
    ]);
    let guard = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
    let result = read_output(&mut command, guard).await;
    assert!(
      result.is_none(),
      "failed preview must not expose raw stdout/stderr"
    );
    assert!(
      logs.lock().unwrap().is_empty(),
      "preview failures must be silent in logs"
    );
  }
}
