//! The one table of what a desktop usage ping may count (spec: docs/specs/usage-counts.md §7). The server keeps the
//! same list for `platform=desktop`: change both together. Keys name a feature, setting or bucket only; no ID,
//! link, title, file name or path can form one.

use std::collections::BTreeSet;
use std::sync::LazyLock;

pub const ACTIVE: &str = "active";
pub const OPEN: &str = "open";
pub const INSTALL_NEW: &str = "install.new";
pub const UPDATE_INSTALLED: &str = "update.installed";
pub const UPDATE_OFFERED: &str = "update.offered";
pub const UPDATE_TAPPED: &str = "update.tapped";
pub const UPDATE_READY: &str = "update.ready";
pub const UPDATE_FAILED_NETWORK: &str = "update.failed.network";
pub const UPDATE_FAILED_VERIFY: &str = "update.failed.verify";
pub const UPDATE_FAILED_INSTALL: &str = "update.failed.install";
pub const ENGINE_UPDATE_OK: &str = "engine.update.ok";
pub const ENGINE_UPDATE_FAILED: &str = "engine.update.failed";
pub const PASTE: &str = "paste";
pub const CLIP_SHOWN: &str = "clip.shown";
pub const CLIP_ACCEPTED: &str = "clip.accepted";
pub const PAUSE: &str = "pause";
pub const PAUSE_ALL: &str = "pause_all";
pub const RESUME: &str = "resume";
pub const RETRY: &str = "retry";
pub const RESUME_FINISHED: &str = "resume.finished";
pub const RESUME_FAILED: &str = "resume.failed";
pub const FUNNEL_LINK: &str = "funnel.link";
pub const FUNNEL_OPTIONS: &str = "funnel.options";
pub const FUNNEL_TAPPED: &str = "funnel.tapped";
pub const FUNNEL_BATCH: &str = "funnel.batch";
pub const FUNNEL_FINISHED: &str = "funnel.finished";
pub const FUNNEL_FAILED: &str = "funnel.failed";
pub const SITE_OTHER: &str = "site.other";
pub const QUICK_DOWNLOAD: &str = "quick.download";
pub const QUICK_AUDIO: &str = "quick.audio";
pub const QUICK_SMALLER: &str = "quick.smaller";
pub const QUICK_OPTIONS: &str = "quick.options";
/// A row of the copied-link card's format menu (a chosen height, or an audio bitrate).
pub const CARD_FORMAT_VIDEO: &str = "card.format.video";
pub const CARD_FORMAT_AUDIO: &str = "card.format.audio";
pub const TRAY_HIDDEN: &str = "tray.hidden";
pub const SHORTCUT_USED: &str = "shortcut.used";
pub const REPORT_OPENED: &str = "report.opened";
pub const LOGIN_PROFILE: &str = "login.profile";
pub const USAGE_CAPPED: &str = "usage.capped";

/// Keys the web UI may count through `usage_count`; everything else is counted in Rust where it happens.
/// `funnel.batch` once per multi-link Download (the store calls `media_download` once per link).
pub const UI_KEYS: &[&str] = &[PASTE, PAUSE, PAUSE_ALL, RETRY, REPORT_OPENED, FUNNEL_BATCH];

pub const SITE: &str = "site:";
pub const SITE_IN: &str = "sitein:";
pub const SITE_FAIL: &str = "sitefail:";
pub const OPTIONS_SITE: &str = "opt:";
pub const FAIL_CODE: &str = "failcode:";
/// Fixed app error types (localized by `errors.runner`), never engine text or a token derived from it.
/// New codes require an explicit allow-list update here and on the desktop server.
pub const FAILURE_CODES: &[&str] = &[
  "browserAppBound",
  "browserLoginFailed",
  "fileAddressRefused",
  "fileBusy",
  "fileChanged",
  "fileEmpty",
  "fileEncoding",
  "fileIo",
  "fileNetwork",
  "fileNotFile",
  "fileNotPublic",
  "fileProxy",
  "fileRateLimited",
  "fileSpace",
  "fileState",
  "accessForbidden403",
  "accountAccessDenied",
  "blockedGeneric",
  "browserCookiesUnreadable",
  "browserDatabaseLocked",
  "browserKeychainDenied",
  "browserProfileMissing",
  "browserSafariAccessDenied",
  "cannotAllocateMemory",
  "channelUnavailable",
  "copyrightClaimed",
  "diskFull",
  "downloadRetryingFragment",
  "drmProtected",
  "embedOnly",
  "facebookVideoUnavailable",
  "fetchTimeout",
  "ffmpegFailed",
  "ffmpegNotFound",
  "geoBlocked",
  "imageEngineUnavailable",
  "imageFileConflict",
  "imageHeaderAuthUnavailable",
  "imageImpersonationUnavailable",
  "imageSelectionChanged",
  "imageSourceUnavailable",
  "internal",
  "livestreamNotStarted",
  "loginConsentDeclined",
  "loginConsentTimeout",
  "mediaNotReady",
  "mediaUnavailable",
  "membersOnly",
  "networkNameResolutionFailed",
  "networkTimeout",
  "noAudioTrack",
  "noMediaFound",
  "noWritePermission",
  "notFound404",
  "notPremieredYet",
  "nsigExtractionFailed",
  "outputNameTooLong",
  "permissionDenied",
  "photosOnly",
  "playlistPrivateOrMissing",
  "rateLimited429",
  "rejectedByFilter",
  "requestedFormatUnavailable",
  "server5xx",
  "signInRequired",
  "signInRequiredForBotDetection",
  "sponsorBlockUnreachable",
  "sslVerifyFailed",
  "unableToConnectToProxy",
  "unableToExtractIdOrInfo",
  "unableToExtractInitialData",
  "unknown",
  "unknownUrlType",
  "urlNotSupported",
  "urlUnsupported",
  "videoNotFoundOrPrivate",
  "videoPasswordRequired",
  "windowsFileMissing",
];

/// Prefixes whose rest is made up from a link (open keys); everything else is fixed.
pub const OPEN_PREFIXES: [&str; 4] = [SITE, SITE_IN, SITE_FAIL, OPTIONS_SITE];

pub const DOWNLOAD_EVENTS: [&str; 4] = ["started", "finished", "failed", "cancelled"];
const VIDEO_TIERS: [&str; 7] = ["low", "sd", "hd", "fhd", "qhd", "uhd", "unknown"];
const AUDIO_TIERS: [&str; 2] = ["mp3", "original"];
pub const OPTION_PATHS: [&str; 3] = ["ytdlp", "own", "gallery"];
pub const OPTION_TIMES: [&str; 4] = ["t0-2", "t2-5", "t5-10", "t10up"];
const SPEEDS: [&str; 4] = ["s0-100k", "s100k-1m", "s1m-5m", "s5mup"];
pub const LOCALES: [&str; 14] = [
  "ar", "de", "en", "es", "fr", "it", "ko", "nb", "nl", "pt-br", "pt-pt", "ru", "tr", "zh-tw",
];
const AGES: [&str; 4] = ["day0", "d1-7", "d8-30", "d31up"];
const ENGINE_FRESH: [&str; 4] = ["d0-7", "d8-30", "d31up", "bundled"];

pub static FIXED: LazyLock<BTreeSet<String>> = LazyLock::new(|| {
  let mut set: BTreeSet<String> = [
    ACTIVE,
    OPEN,
    INSTALL_NEW,
    UPDATE_INSTALLED,
    UPDATE_OFFERED,
    UPDATE_TAPPED,
    UPDATE_READY,
    UPDATE_FAILED_NETWORK,
    UPDATE_FAILED_VERIFY,
    UPDATE_FAILED_INSTALL,
    ENGINE_UPDATE_OK,
    ENGINE_UPDATE_FAILED,
    PASTE,
    CLIP_SHOWN,
    CLIP_ACCEPTED,
    PAUSE,
    PAUSE_ALL,
    RESUME,
    RETRY,
    RESUME_FINISHED,
    RESUME_FAILED,
    FUNNEL_LINK,
    FUNNEL_OPTIONS,
    FUNNEL_TAPPED,
    FUNNEL_BATCH,
    FUNNEL_FINISHED,
    FUNNEL_FAILED,
    SITE_OTHER,
    QUICK_DOWNLOAD,
    QUICK_AUDIO,
    QUICK_SMALLER,
    QUICK_OPTIONS,
    CARD_FORMAT_VIDEO,
    CARD_FORMAT_AUDIO,
    TRAY_HIDDEN,
    SHORTCUT_USED,
    REPORT_OPENED,
    LOGIN_PROFILE,
    USAGE_CAPPED,
  ]
  .into_iter()
  .map(String::from)
  .collect();
  for event in DOWNLOAD_EVENTS {
    for tier in VIDEO_TIERS {
      set.insert(format!("dl.{event}.video.{tier}"));
    }
    for tier in AUDIO_TIERS {
      set.insert(format!("dl.{event}.audio.{tier}"));
    }
    set.insert(format!("dl.{event}.photo.any"));
  }
  for path in OPTION_PATHS {
    for time in OPTION_TIMES {
      set.insert(format!("options.{path}.{time}"));
    }
  }
  set.extend(SPEEDS.iter().map(|s| format!("speed.{s}")));
  set.extend(ENGINE_FRESH.iter().map(|d| format!("engine.fresh.{d}")));
  set.extend(["macos", "windows", "linux"].map(|os| format!("os.{os}")));
  set.extend(["x64", "arm64"].map(|arch| format!("arch.{arch}")));
  set.insert("set.lang.auto".into());
  for locale in LOCALES {
    set.insert(format!("set.lang.{locale}"));
    set.insert(format!("lang.{locale}"));
  }
  set.extend(["system", "light", "dark"].map(|t| format!("set.theme.{t}")));
  set.extend(["1", "2", "3", "4up"].map(|n| format!("set.parallel.{n}")));
  for name in ["tray", "clipboard", "autostart", "reports"] {
    for value in ["on", "off"] {
      set.insert(format!("set.{name}.{value}"));
    }
  }
  set.extend(AGES.iter().map(|a| format!("age.{a}")));
  set
});

/// `[a-z0-9-]{1,63}(\.[a-z0-9-]{1,63})+`, ≤ 64 chars, and the last label not numeric (an IP is never a site).
fn domain_ok(domain: &str) -> bool {
  if domain.is_empty() || domain.len() > 64 {
    return false;
  }
  let labels: Vec<&str> = domain.split('.').collect();
  labels.len() >= 2
    && labels.iter().all(|label| {
      (1..=63).contains(&label.len())
        && label
          .bytes()
          .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
    && !labels
      .last()
      .is_some_and(|last| last.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
}

pub fn allowed(key: &str) -> bool {
  if key.len() > 80 {
    return false;
  }
  if FIXED.contains(key) {
    return true;
  }
  if let Some(code) = key.strip_prefix(FAIL_CODE) {
    return FAILURE_CODES.contains(&code);
  }
  if let Some(rest) = key.strip_prefix(OPTIONS_SITE) {
    return rest
      .split_once(':')
      .is_some_and(|(time, domain)| OPTION_TIMES.contains(&time) && domain_ok(domain));
  }
  [SITE, SITE_IN, SITE_FAIL]
    .iter()
    .any(|prefix| key.strip_prefix(prefix).is_some_and(domain_ok))
}

/// Exact fixed-code lookup: no trimming, tokenizing or copying untrusted text into usage keys.
pub fn failure_code(raw: Option<&str>) -> &'static str {
  FAILURE_CODES
    .iter()
    .copied()
    .find(|code| Some(*code) == raw)
    .unwrap_or("unknown")
}

/// The open prefix a key belongs to, if any.
pub fn open_prefix(key: &str) -> Option<&'static str> {
  OPEN_PREFIXES
    .into_iter()
    .find(|prefix| key.starts_with(prefix))
}

/// The site a usage key names: [`crate::diagnostics::link::site_of_url`] (the same rule reports use), in key form.
pub fn site_of(link: &str) -> Option<String> {
  crate::diagnostics::link::site_of_url(link).filter(|site| domain_ok(site))
}

pub fn site(prefix: &'static str, link: &str) -> Option<String> {
  site_of(link)
    .map(|site| format!("{prefix}{site}"))
    .filter(|key| allowed(key))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
  Video { height: Option<u32> },
  Audio { mp3: bool },
  Photo,
}

pub fn tier(kind: Kind) -> String {
  match kind {
    Kind::Video { height } => {
      let tier = match height.unwrap_or(0) {
        1..=479 => "low",
        480..=719 => "sd",
        720..=1079 => "hd",
        1080..=1439 => "fhd",
        1440..=2159 => "qhd",
        2160.. => "uhd",
        0 => "unknown",
      };
      format!("video.{tier}")
    }
    Kind::Audio { mp3 } => format!("audio.{}", if mp3 { "mp3" } else { "original" }),
    Kind::Photo => "photo.any".into(),
  }
}

pub fn download(event: &str, kind: Kind) -> String {
  format!("dl.{event}.{}", tier(kind))
}

pub fn option_time(millis: u128) -> &'static str {
  match millis {
    0..2_000 => "t0-2",
    2_000..5_000 => "t2-5",
    5_000..10_000 => "t5-10",
    _ => "t10up",
  }
}

pub fn options(path: &str, millis: u128) -> String {
  format!("options.{path}.{}", option_time(millis))
}

pub fn options_site(link: &str, millis: u128) -> Option<String> {
  site_of(link)
    .map(|site| format!("{OPTIONS_SITE}{}:{site}", option_time(millis)))
    .filter(|key| allowed(key))
}

pub fn speed(bytes_per_second: f64) -> String {
  let bucket = if bytes_per_second < 100_000.0 {
    "s0-100k"
  } else if bytes_per_second < 1_000_000.0 {
    "s100k-1m"
  } else if bytes_per_second < 5_000_000.0 {
    "s1m-5m"
  } else {
    "s5mup"
  };
  format!("speed.{bucket}")
}

pub fn age(days: u64) -> String {
  let bucket = match days {
    0 => "day0",
    1..=7 => "d1-7",
    8..=30 => "d8-30",
    _ => "d31up",
  };
  format!("age.{bucket}")
}

/// Days since the last good yt-dlp update; `bundled` = never updated on this computer.
pub fn engine_fresh(days: Option<u64>) -> String {
  let bucket = match days {
    None => "bundled",
    Some(0..=7) => "d0-7",
    Some(8..=30) => "d8-30",
    Some(_) => "d31up",
  };
  format!("engine.fresh.{bucket}")
}

pub fn parallel(max: usize) -> String {
  match max {
    0 | 1 => "set.parallel.1".into(),
    2 | 3 => format!("set.parallel.{max}"),
    _ => "set.parallel.4up".into(),
  }
}

/// A UI locale code (`pt-BR`, `zh_TW`, `ar`) as a key part, or None when it isn't one of ours.
pub fn locale(code: &str) -> Option<String> {
  let code = code.trim().replace('_', "-").to_ascii_lowercase();
  if LOCALES.contains(&code.as_str()) {
    return Some(code);
  }
  let primary = code.split('-').next().unwrap_or_default();
  LOCALES.contains(&primary).then(|| primary.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn allow_list_accepts_feature_keys_and_rejects_everything_else() {
    for key in [
      "open",
      "dl.finished.video.fhd",
      "dl.cancelled.photo.any",
      "options.gallery.t2-5",
      "set.lang.pt-br",
      "lang.zh-tw",
      "os.linux",
      "arch.arm64",
      "set.parallel.4up",
      "site:youtube.com",
      "sitein:music.youtube.com",
      "sitefail:x.com",
      "failcode:signInRequired",
      "failcode:accessForbidden403",
      "failcode:urlNotSupported",
      "failcode:unknown",
      "opt:t10up:instagram.com",
    ] {
      assert!(allowed(key), "{key}");
    }
    for key in [
      "",
      "unknown",
      "sdk:34",
      "brand:samsung",
      "country:MR",
      "failcode:",
      "failcode:inventedCode",
      "failcode:sign-in required",
      "failcode:https://example.com/private",
      "failcode:youtube.com",
      "failcode:/Users/alice/video.mp4",
      &format!("failcode:{}", "a".repeat(49)),
      "site:",
      "site:localhost",
      "site:192.168.1.10",
      "site:Youtube.com",
      "site:youtube.com/watch",
      "site:a_b.com",
      "sitein:https://youtube.com",
      "opt:t1:youtube.com",
      "opt:t0-2:",
      "dl.finished.video.8k",
      &format!("site:{}.com", "a".repeat(61)),
      &format!("site:{}.com", "a".repeat(64)),
    ] {
      assert!(!allowed(key), "{key}");
    }
  }

  #[test]
  fn site_keys_carry_the_registrable_domain_only() {
    assert_eq!(
      site(SITE, "https://www.youtube.com/watch?v=abc123&t=4").as_deref(),
      Some("site:youtube.com")
    );
    assert_eq!(
      site(SITE_FAIL, "https://alice.tumblr.com/post/1").as_deref(),
      Some("sitefail:tumblr.com")
    );
    assert_eq!(site(SITE_IN, "http://127.0.0.1:8080/x"), None);
    assert_eq!(site(SITE, "file:///Users/me/a.mp4"), None);
    assert_eq!(site(SITE, "not a link"), None);
    // Private network names never become keys, even when they look like domains.
    for private in [
      "https://nas.acme.corp/x",
      "https://wiki.intranet/a",
      "http://printer.home.arpa/",
      "https://box.test/v",
      "https://files.mycompany/v",
    ] {
      assert_eq!(site(SITE, private), None, "{private}");
    }
    assert_eq!(
      site(SITE, "https://example.co.uk/v").as_deref(),
      Some("site:example.co.uk")
    );
    // characterisation: youtu.be is its own site key (the report side keeps the same split).
    assert_eq!(
      site(SITE, "https://youtu.be/abc").as_deref(),
      Some("site:youtu.be")
    );
    assert_eq!(site(SITE, "http://[2001:4860:4860::8888]/v"), None);
    assert_eq!(
      site(SITE, "https://vimeo.com/1").as_deref(),
      Some("site:vimeo.com")
    );
    assert_eq!(
      options_site("https://instagram.com/reel/XyZ", 3_100).as_deref(),
      Some("opt:t2-5:instagram.com")
    );
  }

  #[test]
  fn buckets_match_android() {
    assert_eq!(tier(Kind::Video { height: Some(1080) }), "video.fhd");
    assert_eq!(tier(Kind::Video { height: Some(479) }), "video.low");
    assert_eq!(tier(Kind::Video { height: Some(4320) }), "video.uhd");
    assert_eq!(tier(Kind::Video { height: None }), "video.unknown");
    assert_eq!(tier(Kind::Audio { mp3: true }), "audio.mp3");
    assert_eq!(options("ytdlp", 1_999), "options.ytdlp.t0-2");
    assert_eq!(options("ytdlp", 10_000), "options.ytdlp.t10up");
    assert_eq!(speed(99_999.0), "speed.s0-100k");
    assert_eq!(speed(5_000_000.0), "speed.s5mup");
    assert_eq!(age(0), "age.day0");
    assert_eq!(age(31), "age.d31up");
    assert_eq!(engine_fresh(None), "engine.fresh.bundled");
    assert_eq!(engine_fresh(Some(8)), "engine.fresh.d8-30");
    assert_eq!(parallel(8), "set.parallel.4up");
    assert_eq!(locale("pt-BR").as_deref(), Some("pt-br"));
    assert_eq!(locale("ar-MR").as_deref(), Some("ar"));
    assert_eq!(locale("xx"), None);
  }

  #[test]
  fn every_built_key_is_allowed() {
    for event in DOWNLOAD_EVENTS {
      for kind in [
        Kind::Video { height: Some(2000) },
        Kind::Audio { mp3: false },
        Kind::Photo,
      ] {
        assert!(allowed(&download(event, kind)));
      }
    }
    for key in UI_KEYS {
      assert!(allowed(key), "{key}");
    }
  }
}

#[cfg(test)]
mod failure_code_coverage {
  use super::{failure_code, FAILURE_CODES};
  use crate::files::Error;

  #[test]
  fn every_file_and_login_failure_code_is_counted_by_name() {
    let file_errors = [
      Error::Address,
      Error::Network,
      Error::Encoding,
      Error::State,
      Error::Busy,
      Error::Io,
      Error::Restart,
      Error::NotFile,
      Error::Empty,
      Error::Space,
      Error::NotPublic,
      Error::Removed,
      Error::RateLimited,
      Error::Proxy,
    ];
    for error in file_errors {
      assert_eq!(
        failure_code(Some(error.code())),
        error.code(),
        "{} missing from FAILURE_CODES",
        error.code()
      );
    }
    // A new local remedy does not widen the server vocabulary.
    assert_eq!(failure_code(Some("browserDpapiFailed")), "unknown");
    for reason in ["browserAppBound", "browserLoginFailed"] {
      assert!(
        FAILURE_CODES.contains(&reason),
        "{reason} missing from FAILURE_CODES"
      );
    }
  }
}
