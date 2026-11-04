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
mod vimeo;
use std::time::Duration;

pub(crate) const READER_MATCHERS: &[crate::extract::registry::Matcher] = &[
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
  PhotosOnly,
}
impl Reason {
  pub fn code(self) -> &'static str {
    match self {
      Self::PhotosOnly => "photosOnly",
    }
  }
}

enum Outcome {
  NoMatch,
  Verified(Resolution),
  Unknown,
}

impl Outcome {
  fn resolution(self, original: &str) -> Resolution {
    match self {
      Self::Verified(result) => result,
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
      Self::Unsupported { reason } => f
        .debug_struct("Unsupported")
        .field("reason", reason)
        .finish(),
    }
  }
}

pub async fn resolve(url: &str) -> Resolution {
  tokio::time::timeout(Duration::from_secs(20), read_site(url, &http::PublicHttp))
    .await
    .unwrap_or(Outcome::Unknown)
    .resolution(url)
}

async fn read_site<T: http::Transport>(url: &str, transport: &T) -> Outcome {
  if let Some(player) = vimeo::player_url(url) {
    return Outcome::Verified(Resolution::Passthrough(player));
  }
  let mut unknown = false;
  for reader in crate::extract::registry::readers(url) {
    use crate::extract::outcome::ReaderId;
    let result = match reader {
      ReaderId::Reddit => reddit::read(url, transport).await,
      ReaderId::Snapchat => snapchat::read(url, transport).await,
      ReaderId::Threads => threads::read(url, transport).await,
      ReaderId::Linkedin => linkedin::read(url, transport).await,
      ReaderId::Telegram => telegram::read(url, transport).await,
      ReaderId::Facebook => read_facebook(url, transport).await,
    };
    match result {
      Outcome::Verified(_) => return result,
      Outcome::NoMatch => {}
      Outcome::Unknown => unknown = true,
    }
  }
  if unknown {
    Outcome::Unknown
  } else {
    Outcome::NoMatch
  }
}

async fn read_facebook<T: http::Transport>(url: &str, transport: &T) -> Outcome {
  let Some(requested) = facebook::Identity::parse(url) else {
    return Outcome::NoMatch;
  };
  if let Some(id) = &requested.video_id {
    return Outcome::Verified(Resolution::Passthrough(facebook::watch(id)));
  }
  let page = match http::fetch(transport, requested.url.clone(), true).await {
    Ok(page) => page,
    Err(_) => return Outcome::Unknown,
  };
  let Some(identity) = facebook::Identity::parse(page.url.as_str()) else {
    return Outcome::Unknown;
  };
  if requested.short {
    if let Some(id) = &identity.video_id {
      return Outcome::Verified(Resolution::Passthrough(facebook::watch(id)));
    }
  }
  let Ok(html) = std::str::from_utf8(&page.body) else {
    return Outcome::Unknown;
  };
  facebook::read(html, &identity, &requested)
}

pub fn unsupported_url(stderr: &str) -> bool {
  stderr.lines().any(|line| {
    let line = line.trim().to_ascii_lowercase();
    line.starts_with("error:") && line.contains("unsupported url:")
  })
}

/// These hosts keep their reader's identity and admission rules even after an inconclusive read.
pub(crate) fn reader_bound(url: &str) -> bool {
  http::https_url(url)
    .ok()
    .and_then(|u| u.host_str().map(str::to_owned))
    .is_some_and(|host| {
      facebook::is_host(&host)
        || reddit::is_host(&host)
        || host == "fb.watch"
        || threads::is_host(&host)
        || linkedin::is_host(&host)
        || telegram::is_host(&host)
    })
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
    Ok(Outcome::Verified(result)) => Some(result),
    _ => None,
  }
}

async fn read_generic<T: http::Transport>(url: &str, transport: &T) -> Outcome {
  let Ok(parsed) = http::https_url(url) else {
    return Outcome::NoMatch;
  };
  let Ok(page) = http::fetch(transport, parsed, false).await else {
    return Outcome::Unknown;
  };
  let result = if page.media {
    Some(Resolution::Passthrough(page.url.to_string()))
  } else {
    std::str::from_utf8(&page.body)
      .ok()
      .and_then(|text| html::generic(text, page.url.as_str()))
  };
  match result {
    Some(Resolution::Passthrough(target)) => {
      // yt-dlp will fetch this extracted URL itself: apply the same public-address boundary.
      let admitted = http::https_url(&target).ok();
      if let Some(url) = admitted {
        if http::addresses(&url).await.is_ok() {
          return Outcome::Verified(Resolution::Passthrough(target));
        }
      }
      Outcome::Unknown
    }
    Some(result) => Outcome::Verified(result),
    None => Outcome::Unknown,
  }
}

#[derive(Clone)]
pub struct ResolvedInput {
  pub original: String,
  pub url: String,
  pub source_page: Option<String>,
  original_retried: bool,
}
impl ResolvedInput {
  pub fn new(url: &str) -> Self {
    Self {
      original: url.into(),
      url: url.into(),
      source_page: None,
      original_retried: false,
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
