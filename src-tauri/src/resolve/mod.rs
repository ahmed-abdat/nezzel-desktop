//! Site readers run before yt-dlp; unknown or inconclusive pages preserve its input.
mod dom;
mod facebook;
pub(crate) mod html;
pub(crate) mod http;
mod linkedin;
mod post;
pub(crate) mod reddit;
mod snapchat;
mod telegram;
mod threads;
pub(crate) mod vimeo;
use crate::routing::{ReaderId, StageId};
use std::time::Duration;

pub(crate) const READER_MATCHERS: &[crate::routing::Matcher] = &[
  vimeo::MATCHER,
  reddit::MATCHER,
  snapchat::MATCHER,
  threads::MATCHER,
  linkedin::MATCHER,
  telegram::MATCHER,
  facebook::MATCHER,
  facebook::SHARE_MATCHER,
  facebook::PHOTO_MATCHER,
];

/// Readers whose identity grammar claims this URL, highest priority first. Cross-stage
/// exclusivity (file shares, YouTube) is the registry's: `extract::registry::readers`.
pub(crate) fn readers(raw: &str) -> Vec<ReaderId> {
  let Ok(url) = reqwest::Url::parse(raw) else {
    return Vec::new();
  };
  let mut matched = crate::routing::best_per_stage(READER_MATCHERS, &url);
  matched.sort_by_key(|matcher| std::cmp::Reverse(matcher.priority));
  matched
    .into_iter()
    .filter_map(|matcher| match matcher.stage {
      StageId::Reader(reader) => Some(reader),
      _ => None,
    })
    .collect()
}

pub(crate) async fn admit_post_attachments(
  items: &[crate::models::parsed::MediaAttachment],
) -> bool {
  post::admit_attachments(&http::PublicHttp, items).await
}

#[derive(Clone, PartialEq, Eq)]
pub enum Resolution {
  Passthrough(String),
  PostMedia {
    url: String,
    title: Option<String>,
    thumbnail: Option<String>,
    uploader: Option<String>,
  },
  Attachments {
    metadata: String,
  },
  Unsupported {
    reason: Reason,
  },
  Failed {
    failure: Failure,
  },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
  Network,
  Timeout,
}
impl Failure {
  pub fn code(self) -> &'static str {
    match self {
      Self::Network => "networkNameResolutionFailed",
      Self::Timeout => "networkTimeout",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
  PhotosOnly,
  NoMedia,
}
impl Reason {
  pub fn code(self) -> &'static str {
    match self {
      Self::PhotosOnly => "photosOnly",
      Self::NoMedia => "noMediaFound",
    }
  }
}

enum Verdict {
  NoMatch,
  Verified(Resolution),
  Unknown,
  Failed(Failure),
}

impl Verdict {
  fn resolution(self, original: &str) -> Resolution {
    match self {
      Self::Verified(result) => result,
      Self::Failed(failure) => Resolution::Failed { failure },
      Self::NoMatch | Self::Unknown => Resolution::Passthrough(original.into()),
    }
  }
}

impl std::fmt::Debug for Resolution {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Passthrough(_) => f.write_str("Passthrough(redacted)"),
      Self::PostMedia { .. } => f.write_str("PostMedia(redacted)"),
      Self::Attachments { .. } => f.write_str("Attachments(redacted)"),
      Self::Failed { failure } => f.debug_tuple("Failed").field(failure).finish(),
      Self::Unsupported { reason } => f
        .debug_struct("Unsupported")
        .field("reason", reason)
        .finish(),
    }
  }
}

pub async fn resolve(url: &str) -> Resolution {
  match tokio::time::timeout(Duration::from_secs(20), read_site(url, &http::PublicHttp)).await {
    Ok(verdict) => verdict.resolution(url),
    Err(_) => resolution_timeout(url),
  }
}

pub(crate) fn resolution_timeout(url: &str) -> Resolution {
  if threads::Identity::parse(url).is_some() {
    Resolution::Failed {
      failure: Failure::Timeout,
    }
  } else {
    Resolution::Passthrough(url.into())
  }
}

async fn read_site<T: http::Transport>(url: &str, transport: &T) -> Verdict {
  if let Some(player) = vimeo::player_url(url) {
    return Verdict::Verified(Resolution::Passthrough(player));
  }
  let mut unknown = false;
  for reader in readers(url) {
    let result = match reader {
      ReaderId::Reddit => reddit::read(url, transport).await,
      ReaderId::Snapchat => snapchat::read(url, transport).await,
      ReaderId::Threads => threads::read(url, transport).await,
      ReaderId::Linkedin => linkedin::read(url, transport).await,
      ReaderId::Telegram => telegram::read(url, transport).await,
      ReaderId::Facebook => read_facebook(url, transport).await,
    };
    match result {
      Verdict::Verified(_) | Verdict::Failed(_) => return result,
      Verdict::NoMatch => {}
      Verdict::Unknown => unknown = true,
    }
  }
  if unknown {
    Verdict::Unknown
  } else {
    Verdict::NoMatch
  }
}

async fn read_facebook<T: http::Transport>(url: &str, transport: &T) -> Verdict {
  let Some(requested) = facebook::Identity::parse(url) else {
    return Verdict::NoMatch;
  };
  if let Some(id) = &requested.video_id {
    return Verdict::Verified(Resolution::Passthrough(facebook::watch(id)));
  }
  let page = match http::fetch(transport, requested.url.clone(), true).await {
    Ok(page) => page,
    Err(_) => return Verdict::Unknown,
  };
  let Some(identity) = facebook::Identity::parse(page.url.as_str()) else {
    return Verdict::Unknown;
  };
  if requested.short {
    if let Some(id) = &identity.video_id {
      return Verdict::Verified(Resolution::Passthrough(facebook::watch(id)));
    }
  }
  let Ok(html) = std::str::from_utf8(&page.body) else {
    return Verdict::Unknown;
  };
  let verdict = facebook::read(html, &identity, &requested);
  if let Verdict::Verified(result @ Resolution::Attachments { .. }) = &verdict {
    if !admit_resolution(transport, result).await {
      return Verdict::Unknown;
    }
  }
  verdict
}

/// Only the reader's bounded single-photo route can bypass yt-dlp for gallery-dl.
pub(crate) fn facebook_photo_target(raw: &str) -> bool {
  facebook::Identity::parse(raw).is_some_and(|identity| {
    identity.photo_id.is_some() && identity.url.as_str() == raw && identity.url.path() == "/photo/"
  })
}

pub fn unsupported_url(stderr: &str) -> bool {
  stderr.lines().any(|line| {
    let line = line.trim().to_ascii_lowercase();
    line.starts_with("error:") && line.contains("unsupported url:")
  })
}

/// These hosts keep their reader's identity and admission rules even after an inconclusive read.
/// Derived from READER_MATCHERS, so a registered reader cannot be left out.
pub(crate) fn reader_bound(url: &str) -> bool {
  let Some(host) = http::https_url(url)
    .ok()
    .and_then(|u| u.host_str().map(str::to_owned))
  else {
    return false;
  };
  READER_MATCHERS.iter().any(|matcher| match matcher.stage {
    StageId::Reader(reader) => reader_host(reader, &host),
    _ => false,
  })
}

/// Every reader names the hosts it owns; a new ReaderId does not compile without them.
fn reader_host(reader: ReaderId, host: &str) -> bool {
  match reader {
    ReaderId::Reddit => reddit::is_host(host),
    ReaderId::Facebook => facebook::is_host(host) || host == "fb.watch",
    ReaderId::Threads => threads::is_host(host),
    ReaderId::Linkedin => linkedin::is_host(host),
    ReaderId::Telegram => telegram::is_host(host),
    ReaderId::Snapchat => snapchat::is_host(host),
  }
}

/// Download fallback retains explicit unsupported gating and the shared reader boundary.
pub async fn after_unsupported(url: &str, stderr: &str) -> Option<Resolution> {
  if !unsupported_url(stderr) || reader_bound(url) {
    return None;
  }
  generic_scan(url).await
}

/// Fetch-chain fallback after engine extraction failures, including inconclusive site readers.
/// The caller supplies the last resolved URL; candidate URLs retain read_generic's admission gate.
pub(crate) async fn generic_scan(resolved_url: &str) -> Option<Resolution> {
  match tokio::time::timeout(
    Duration::from_secs(20),
    read_generic(resolved_url, &http::PublicHttp),
  )
  .await
  {
    Ok(Verdict::Verified(result)) => Some(result),
    _ => None,
  }
}

async fn read_generic<T: http::Transport>(url: &str, transport: &T) -> Verdict {
  let Ok(parsed) = http::https_url(url) else {
    return Verdict::NoMatch;
  };
  let Ok(page) = http::fetch(transport, parsed, false).await else {
    return Verdict::Unknown;
  };
  let result = if page.media {
    Some(Resolution::Passthrough(page.url.to_string()))
  } else {
    std::str::from_utf8(&page.body)
      .ok()
      .and_then(|text| html::generic(text, page.url.as_str()))
  };
  match result {
    // Whatever yt-dlp or the gallery runner will fetch next gets the same public-address gate.
    Some(result) if admit_resolution(transport, &result).await => Verdict::Verified(result),
    _ => Verdict::Unknown,
  }
}

/// Every URL a resolution hands to a downloader must resolve to public addresses.
/// A failing candidate rejects the whole resolution (same as `Passthrough`).
async fn admit_resolution<T: http::Transport>(transport: &T, resolution: &Resolution) -> bool {
  async fn admit<T: http::Transport>(transport: &T, raw: &str) -> bool {
    match http::https_url(raw) {
      Ok(url) => transport.admit_media(&url).await.is_ok(),
      Err(_) => false,
    }
  }
  match resolution {
    Resolution::Passthrough(target) => admit(transport, target).await,
    Resolution::PostMedia { url, thumbnail, .. } => {
      admit(transport, url).await
        && match thumbnail {
          Some(thumbnail) => admit(transport, thumbnail).await,
          None => true,
        }
    }
    Resolution::Attachments { metadata } => {
      match serde_json::from_str::<crate::models::parsed::ParsedSingleVideo>(metadata) {
        Ok(media) => post::admit_attachments(transport, &media.attachments).await,
        Err(_) => false,
      }
    }
    Resolution::Unsupported { .. } | Resolution::Failed { .. } => true,
  }
}

#[derive(Clone)]
pub struct ResolvedInput {
  pub original: String,
  pub url: String,
  pub source_page: Option<String>,
  original_retried: bool,
  vimeo_impersonated: bool,
}
impl ResolvedInput {
  pub fn new(url: &str) -> Self {
    Self {
      original: url.into(),
      url: url.into(),
      source_page: None,
      original_retried: false,
      vimeo_impersonated: false,
    }
  }
  pub fn rewrite(&mut self, url: String) {
    self.source_page = vimeo::referer(&url)
      .map(str::to_owned)
      .or_else(|| (url != self.original).then(|| self.original.clone()));
    self.url = url;
  }
  /// Keep metadata, transport probes and downloads on the same target and referer.
  pub fn args(&self) -> Vec<&str> {
    let mut args = Vec::new();
    if let Some(page) = &self.source_page {
      args.extend(["--referer", page.as_str()]);
    }
    if self.vimeo_impersonated {
      args.extend(["--impersonate", "chrome"]);
    }
    args.extend(["--", self.url.as_str()]);
    args
  }
  /// Keep source identity for display/retries and reuse the verified target on download.
  pub(crate) fn retain_fetch_target(&self, media: &mut crate::models::ParsedMedia) {
    match media {
      crate::models::ParsedMedia::Single(video) => {
        video.resolved_url = Some(self.url.clone());
        if self.url != self.original {
          video.url = Some(self.original.clone());
        }
      }
      crate::models::ParsedMedia::Livestream(video) if self.url != self.original => {
        video.url = Some(self.original.clone());
      }
      _ => {}
    }
  }
  /// Vimeo sometimes rejects yt-dlp's TLS fingerprint. Retry only that response, once.
  pub(crate) fn retry_vimeo_401(&mut self, stderr: &str) -> bool {
    if self.vimeo_impersonated
      || !vimeo::drm_may_be_login(&self.url)
      || !stderr.to_ascii_lowercase().contains("http error 401")
    {
      return false;
    }
    self.vimeo_impersonated = true;
    true
  }

  pub(crate) fn impersonation_unavailable(&self, stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    self.vimeo_impersonated
      && lower.contains("impersonat")
      && ["not available", "unavailable", "not found", "not supported"]
        .iter()
        .any(|hint| lower.contains(hint))
  }

  pub fn retry_original(&mut self, stderr: &str) -> bool {
    if self.original_retried
      || self.url == self.original
      || !stderr.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.contains("http error 403") || line.contains("http error 404")
      })
    {
      return false;
    }
    self.url.clone_from(&self.original);
    self.source_page = None;
    self.original_retried = true;
    true
  }
}

#[cfg(test)]
mod tests;
