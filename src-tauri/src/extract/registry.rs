//! URL-only capability declarations. Matching is pure; registration order breaks confidence ties.
use super::outcome::{Confidence, ReaderId, StageId};
use reqwest::Url;
use std::cmp::Reverse;

#[derive(Clone, Copy)]
pub(crate) struct Matcher {
  pub stage: StageId,
  pub name: &'static str,
  pub priority: u8,
  pub confidence: Confidence,
  pub exclusive: bool,
  pattern: Pattern,
}

#[derive(Clone, Copy)]
pub(crate) enum Pattern {
  Any,
  HostPath {
    hosts: &'static [&'static str],
    subdomains: bool,
    path: Path,
  },
  Identity(fn(&str) -> bool),
  UrlOnly(fn(&Url) -> bool),
}

#[derive(Clone, Copy)]
pub(crate) enum Path {
  Any,
  Prefixes(&'static [&'static str]),
  Contains {
    marker: &'static str,
    exclude: Option<&'static str>,
  },
}

impl Path {
  fn matches(self, path: &str) -> bool {
    match self {
      Self::Any => true,
      Self::Prefixes(prefixes) => prefixes.iter().any(|prefix| {
        path
          .strip_prefix(prefix)
          .is_some_and(|suffix| !suffix.is_empty())
      }),
      Self::Contains { marker, exclude } => {
        path
          .split_once(marker)
          .is_some_and(|(_, suffix)| !suffix.is_empty())
          && !exclude.is_some_and(|excluded| path.contains(excluded))
      }
    }
  }
}

impl Matcher {
  pub(crate) const fn new(
    stage: StageId,
    name: &'static str,
    priority: u8,
    confidence: Confidence,
    pattern: Pattern,
  ) -> Self {
    Self {
      stage,
      name,
      priority,
      confidence,
      exclusive: false,
      pattern,
    }
  }

  pub(crate) const fn reader(
    reader: ReaderId,
    name: &'static str,
    priority: u8,
    accepts: fn(&str) -> bool,
  ) -> Self {
    Self::new(
      StageId::Reader(reader),
      name,
      priority,
      Confidence::High,
      Pattern::Identity(accepts),
    )
  }

  fn matches(&self, url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
      return false;
    }
    match self.pattern {
      Pattern::Any => true,
      Pattern::Identity(accepts) => accepts(url.as_str()),
      Pattern::UrlOnly(accepts) => accepts(url),
      Pattern::HostPath {
        hosts,
        subdomains,
        path,
      } => {
        url.host_str().is_some_and(|host| {
          hosts.iter().any(|domain| {
            host == *domain
              || subdomains
                && host
                  .strip_suffix(domain)
                  .is_some_and(|prefix| prefix.ends_with('.'))
          })
        }) && path.matches(url.path())
      }
    }
  }
}

const fn host_path(hosts: &'static [&'static str], subdomains: bool, path: Path) -> Pattern {
  Pattern::HostPath {
    hosts,
    subdomains,
    path,
  }
}

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
      !stem.is_empty()
        && matches!(
          ext.to_ascii_lowercase().as_str(),
          "mp4"
            | "mp3"
            | "webm"
            | "m4a"
            | "ogg"
            | "flac"
            | "wav"
            | "mov"
            | "mkv"
            | "m3u8"
            | "m3u"
            | "mpd"
        )
    })
}

const FILES: &[Matcher] = &[
  Matcher::new(
    StageId::Files,
    "file-extension",
    30,
    Confidence::High,
    Pattern::UrlOnly(direct_file),
  ),
  Matcher {
    exclusive: true,
    ..Matcher::new(
      StageId::Files,
      "file-share",
      40,
      Confidence::High,
      Pattern::Identity(|raw| {
        crate::files::hosts::plan(raw, crate::files::hosts::Export::Pdf).is_some()
      }),
    )
  },
  Matcher::new(
    StageId::Files,
    "file-probe",
    10,
    Confidence::Medium,
    Pattern::Identity(crate::files::probe::should_probe),
  ),
];

const GALLERY: &[Matcher] = &[
  Matcher::new(
    StageId::Gallery,
    "twitter-status",
    30,
    Confidence::High,
    host_path(
      &[
        "x.com",
        "www.x.com",
        "twitter.com",
        "www.twitter.com",
        "mobile.twitter.com",
      ],
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
  Matcher {
    exclusive: true,
    ..Matcher::new(
      StageId::Ytdlp,
      "youtube-exclusive",
      30,
      Confidence::High,
      host_path(
        &["youtube.com", "youtu.be", "youtube-nocookie.com"],
        true,
        Path::Any,
      ),
    )
  },
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
  if crate::files::hosts::plan(raw, crate::files::hosts::Export::Pdf).is_some() {
    return FILES.iter().filter(|matcher| matcher.exclusive).collect();
  }
  let Ok(url) = Url::parse(raw) else {
    return Vec::new();
  };
  let mut routes: Vec<&Matcher> = Vec::new();
  for matcher in REGISTRY
    .iter()
    .flat_map(|group| group.iter())
    .filter(|matcher| matcher.matches(&url))
  {
    if let Some(existing) = routes
      .iter_mut()
      .find(|existing| existing.stage == matcher.stage)
    {
      if (matcher.confidence, matcher.priority) > (existing.confidence, existing.priority) {
        *existing = matcher;
      }
    } else {
      routes.push(matcher);
    }
  }
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
  if crate::resolve::reddit::is_host(host)
    || matches!(host, "vimeo.com" | "soundcloud.com" | "snapchat.com")
    || host.ends_with(".snapchat.com")
    || path.starts_with("/reel/")
    || path.starts_with("/reels/")
    || (host.ends_with("tiktok.com") && path.contains("/video/"))
    || ((host == "x.com" || host.ends_with("twitter.com")) && path.contains("/video/"))
  {
    return false;
  }
  match host {
    "x.com" | "twitter.com" | "mobile.twitter.com" => path.contains("/status/"),
    "instagram.com" => path.starts_with("/p/"),
    "bsky.app" => path.contains("/post/"),
    "pinterest.com" => path.starts_with("/pin/"),
    _ => true,
  }
}

pub(crate) fn exclusive(raw: &str) -> bool {
  routes(raw).iter().any(|matcher| matcher.exclusive)
}

pub(crate) fn readers(raw: &str) -> Vec<ReaderId> {
  let mut candidates: Vec<_> = routes(raw)
    .into_iter()
    .filter(|matcher| matches!(matcher.stage, StageId::Reader(_)))
    .collect();
  candidates.sort_by_key(|matcher| Reverse(matcher.priority));
  candidates
    .into_iter()
    .filter_map(|matcher| match matcher.stage {
      StageId::Reader(reader) => Some(reader),
      _ => None,
    })
    .collect()
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
    let host = url
      .host_str()
      .unwrap_or_default()
      .trim_start_matches("www.");
    (matches!(host, "x.com" | "twitter.com" | "mobile.twitter.com")
      && url.path().contains("/status/"))
      || ((host == "tiktok.com" || host.ends_with(".tiktok.com")) && url.path().contains("/video/"))
  })
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
