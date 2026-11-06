//! URL-only capability declarations. Matching is pure; registration order breaks confidence ties.
use super::outcome::{Confidence, ReaderId, StageId};
use crate::media_types::{ext_in, MANIFEST_EXTS, MEDIA_EXTS};
pub(crate) use crate::routing::Matcher;
use crate::routing::{host_path, Path, Pattern};
use crate::sites::{site_by_domain, site_of, SiteId};
use reqwest::Url;
use std::cmp::Reverse;

fn direct_file(url: &Url) -> bool {
  if crate::files::probe::file_extension(url) {
    return true;
  }
  let decoded = crate::files::names::percent_bytes(url.path());
  let path = String::from_utf8_lossy(&decoded);
  path
    .rsplit('/')
    .next()
    .and_then(|name| name.rsplit_once('.'))
    .is_some_and(|(stem, ext)| {
      !stem.is_empty() && (ext_in(MEDIA_EXTS, ext) || ext_in(MANIFEST_EXTS, ext))
    })
}

// `routes()` builds the share plan once, before matching: a share link takes no other stage.
const FILE_SHARE: Matcher = Matcher::new(
  StageId::Files,
  "file-share",
  40,
  Confidence::High,
  Pattern::Identity(share_link),
)
.exclusive();

fn share_link(raw: &str) -> bool {
  crate::files::hosts::is_share(raw)
}

const FILES: &[Matcher] = &[
  Matcher::new(
    StageId::Files,
    "file-extension",
    30,
    Confidence::High,
    Pattern::UrlOnly(direct_file),
  ),
  Matcher::new(
    StageId::Files,
    "file-probe",
    10,
    Confidence::Medium,
    Pattern::Identity(crate::files::probe::should_probe),
  ),
];

/// The X hosts whose status links route as posts: exact, so `archive.twitter.com` stays generic.
const X_ROUTE_HOSTS: &[&str] = &[
  "x.com",
  "www.x.com",
  "twitter.com",
  "www.twitter.com",
  "mobile.twitter.com",
];

const GALLERY: &[Matcher] = &[
  Matcher::new(
    StageId::Gallery,
    "twitter-status",
    30,
    Confidence::High,
    host_path(
      X_ROUTE_HOSTS,
      false,
      Path::Contains {
        marker: "/status/",
        exclude: Some("/video/"),
      },
    ),
  ),
  Matcher::new(
    StageId::Gallery,
    "instagram-post",
    30,
    Confidence::High,
    host_path(&["instagram.com"], true, Path::Prefixes(&["/p/"])),
  ),
  Matcher::new(
    StageId::Gallery,
    "bluesky-post",
    30,
    Confidence::High,
    host_path(
      &["bsky.app", "www.bsky.app"],
      false,
      Path::Contains {
        marker: "/post/",
        exclude: None,
      },
    ),
  ),
  Matcher::new(
    StageId::Gallery,
    "pinterest-pin",
    30,
    Confidence::High,
    host_path(&["pinterest.com"], true, Path::Prefixes(&["/pin/"])),
  ),
  Matcher::new(
    StageId::Gallery,
    "tiktok-photo",
    30,
    Confidence::High,
    host_path(
      &["tiktok.com"],
      true,
      Path::Contains {
        marker: "/photo/",
        exclude: None,
      },
    ),
  ),
  // An album must reach its photo-capable engine before yt-dlp can verify a partial video list.
  Matcher::new(
    StageId::Gallery,
    "imgur-album",
    30,
    Confidence::High,
    host_path(
      &["imgur.com", "www.imgur.com"],
      false,
      Path::Prefixes(&["/a/", "/gallery/"]),
    ),
  ),
];

const YTDLP: &[Matcher] = &[
  Matcher::new(
    StageId::Ytdlp,
    "youtube-exclusive",
    30,
    Confidence::High,
    Pattern::UrlOnly(|url| url.host_str().and_then(site_of) == Some(SiteId::YouTube)),
  )
  .exclusive(),
  Matcher::new(
    StageId::Ytdlp,
    "video-host",
    20,
    Confidence::High,
    host_path(
      &[
        "vimeo.com",
        "soundcloud.com",
        "twitch.tv",
        "dailymotion.com",
      ],
      true,
      Path::Any,
    ),
  ),
  Matcher::new(
    StageId::Ytdlp,
    "ytdlp-fallback",
    10,
    Confidence::Medium,
    Pattern::Any,
  ),
];

const FALLBACKS: &[Matcher] = &[
  Matcher::new(
    StageId::Gallery,
    "gallery-fallback",
    10,
    Confidence::Low,
    Pattern::Any,
  ),
  Matcher::new(
    StageId::PageScan,
    "page-scan",
    10,
    Confidence::Low,
    Pattern::Identity(|raw| !crate::resolve::reader_bound(raw)),
  ),
];

// Readers own their identity grammars. Adding a reader doesn't add another routing host list.
const REGISTRY: &[&[Matcher]] = &[
  FILES,
  crate::resolve::READER_MATCHERS,
  GALLERY,
  YTDLP,
  FALLBACKS,
];

/// Keep the highest-confidence/priority rule for each stage, then sort by confidence and
/// static registration order. Priority chooses a named URL kind within a stage, not a new stage.
pub(crate) fn routes(raw: &str) -> Vec<&'static Matcher> {
  if share_link(raw) {
    return vec![&FILE_SHARE];
  }
  let Ok(url) = Url::parse(raw) else {
    return Vec::new();
  };
  let mut routes =
    crate::routing::best_per_stage(REGISTRY.iter().flat_map(|group| group.iter()), &url);
  if routes.iter().any(|matcher| matcher.exclusive) {
    routes.retain(|matcher| matcher.exclusive);
  }
  routes.sort_by_key(|matcher| Reverse(matcher.confidence));
  routes
}

pub(crate) fn claim(stage: StageId, raw: &str) -> Confidence {
  routes(raw)
    .iter()
    .find(|matcher| matcher.stage == stage)
    .map_or(Confidence::None, |matcher| matcher.confidence)
}

/// Capture/download preference is independent of fetch confidence: unknown sites still get
/// the legacy gallery probe, while explicit video routes and reader-bound Reddit stay excluded.
pub(crate) fn gallery_first(raw: &str) -> bool {
  if claim(StageId::Gallery, raw) == Confidence::None {
    return false;
  }
  let Ok(url) = Url::parse(raw) else {
    return false;
  };
  let host = url
    .host_str()
    .unwrap_or_default()
    .trim_start_matches("www.");
  let path = url.path();
  let site = site_by_domain(host);
  if crate::resolve::reddit::is_host(host)
    || matches!(host, "vimeo.com" | "soundcloud.com" | "snapchat.com")
    || host.ends_with(".snapchat.com")
    || path.starts_with("/reel/")
    || path.starts_with("/reels/")
    || (matches!(site, Some(SiteId::TikTok | SiteId::X)) && path.contains("/video/"))
  {
    return false;
  }
  if X_ROUTE_HOSTS.contains(&host) {
    return path.contains("/status/");
  }
  match host {
    "instagram.com" => path.starts_with("/p/"),
    "bsky.app" => path.contains("/post/"),
    "pinterest.com" => path.starts_with("/pin/"),
    _ => true,
  }
}

pub(crate) fn exclusive(raw: &str) -> bool {
  routes(raw).iter().any(|matcher| matcher.exclusive)
}

/// Readers that claim the URL, highest priority first. An exclusive route (file share, YouTube)
/// leaves no reader; otherwise the readers' own matchers decide.
pub(crate) fn readers(raw: &str) -> Vec<ReaderId> {
  if exclusive(raw) {
    return Vec::new();
  }
  crate::resolve::readers(raw)
}

/// Keep the legacy Files gate first unless a stage is exclusive. It declines unsupported
/// URL shapes without I/O. Readers still execute inside yt-dlp's resolver, once per input.
pub(crate) fn stages_for(raw: &str) -> Vec<StageId> {
  let candidates = routes(raw);
  let mut stages = if candidates.iter().any(|matcher| matcher.exclusive) {
    Vec::new()
  } else {
    vec![StageId::Files]
  };
  for matcher in candidates {
    let stage = match matcher.stage {
      StageId::Reader(_) => StageId::Ytdlp,
      stage => stage,
    };
    if !stages.contains(&stage) {
      stages.push(stage);
    }
  }
  // A status URL is usually a video. The chain still hands photo-only/no-format
  // results to gallery, including TikTok's audio-only slideshow representation.
  if video_first(raw) {
    prefer_ytdlp(&mut stages);
  }
  stages
}

fn prefer_ytdlp(stages: &mut Vec<StageId>) {
  if let (Some(gallery), Some(ytdlp)) = (
    stages.iter().position(|stage| *stage == StageId::Gallery),
    stages.iter().position(|stage| *stage == StageId::Ytdlp),
  ) {
    if gallery < ytdlp {
      let stage = stages.remove(ytdlp);
      stages.insert(gallery, stage);
    }
  }
}

fn video_first(raw: &str) -> bool {
  Url::parse(raw).is_ok_and(|url| {
    let host = url.host_str().unwrap_or_default();
    (X_ROUTE_HOSTS.contains(&host) && url.path().contains("/status/"))
      || (site_by_domain(host) == Some(SiteId::TikTok) && url.path().contains("/video/"))
  })
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
