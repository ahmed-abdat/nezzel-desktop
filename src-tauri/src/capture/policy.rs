use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::time::Duration;

use crate::sites::{site_by_domain, site_of, SiteId};

// Deliberately conservative: the generic extractor must never turn arbitrary copied text
// into a network request. A site's hosts and their subdomains (dot boundary) only.
const SITES: &[SiteId] = {
  use SiteId::*;
  &[
    Dropbox,
    GoogleDrive,
    GoogleDocs,
    YouTube,
    Vimeo,
    TikTok,
    Instagram,
    Facebook,
    X,
    Twitch,
    SoundCloud,
    Dailymotion,
    Reddit,
    Bilibili,
    Pinterest,
    Ted,
    Bandcamp,
    InternetArchive,
    Rumble,
    Odysee,
    // Photo and post readers (resolve/, gallery-dl).
    LinkedIn,
    Threads,
    Bluesky,
    Telegram,
    Snapchat,
  ]
};

pub fn supported_url(text: &str) -> Option<(String, String)> {
  let text = text.trim();
  if text.len() > 8192
    || text
      .chars()
      .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
  {
    return None;
  }
  let normalized;
  let text = if !text.contains("://") && crate::files::hosts::is_share(text) {
    normalized = format!("https://{text}");
    normalized.as_str()
  } else {
    text
  };
  let (scheme, rest) = text.split_once("://")?;
  let scheme = scheme.to_ascii_lowercase();
  if scheme != "https" && scheme != "http" {
    return None;
  }
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let authority = rest[..end].to_ascii_lowercase();
  // Credentials, nonstandard ports and escaped hosts are intentionally unsupported.
  let host = authority
    .strip_suffix(":443")
    .filter(|_| scheme == "https")
    .or_else(|| authority.strip_suffix(":80").filter(|_| scheme == "http"))
    .unwrap_or(&authority);
  let site = site_of(host).filter(|site| SITES.contains(site))?;
  if !host
    .bytes()
    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
  {
    return None;
  }
  if host.len() > 253
    || !host.split('.').all(|label| {
      !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
    })
  {
    return None;
  }
  let suffix = rest[end..].split('#').next().unwrap_or("");
  let path = suffix.split('?').next().unwrap_or("");
  if matches!(
    site,
    SiteId::GoogleDrive | SiteId::GoogleDocs | SiteId::Dropbox
  ) && !crate::files::hosts::is_share(text)
  {
    return None;
  }
  // Private Telegram invites and private channels never become a background request.
  if site == SiteId::Telegram
    && (path.starts_with("/+") || path.starts_with("/joinchat") || path.starts_with("/c/"))
  {
    return None;
  }
  // lnkd.in also shortens arbitrary external links; only LinkedIn's own post links (/p/).
  if crate::sites::host_is(host, "lnkd.in") && !path.starts_with("/p/") {
    return None;
  }
  Some((format!("{scheme}://{host}{suffix}"), host.into()))
}

/// True for a site's home/feed page (no specific post). These get no metadata fetch.
/// Expects a URL already normalized by `supported_url`.
pub fn is_feed_page(url: &str) -> bool {
  let Some((_, rest)) = url.split_once("://") else {
    return false;
  };
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let host = &rest[..end];
  let path = rest[end..].split(['?', '#']).next().unwrap_or("");
  let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
  let first = segments.first().copied().unwrap_or("");
  match site_by_domain(host) {
    Some(SiteId::LinkedIn) => first == "feed" && !path.contains("urn:li:"),
    Some(SiteId::Facebook) => segments.is_empty() || (segments.len() == 1 && first == "home"),
    Some(SiteId::Instagram) => segments.is_empty(),
    Some(SiteId::X) => segments.len() == 1 && first == "home",
    Some(SiteId::TikTok) => segments.is_empty() || (segments.len() == 1 && first == "foryou"),
    Some(SiteId::Reddit) => {
      segments.is_empty()
        // `/r/<sub>/comments/…` is a post; `/r/<sub>/s/<token>` is Reddit's Share → Copy link form.
        || (first == "r"
          && segments.len() >= 2
          && segments.get(2).is_none_or(|s| !matches!(*s, "comments" | "s")))
    }
    Some(SiteId::YouTube) => segments.is_empty() || first == "feed",
    _ => false,
  }
}

#[derive(Default)]
pub struct CapturePolicy {
  last: Option<u64>,
  seen: HashMap<u64, Duration>,
  own: HashMap<u64, Duration>,
}

fn fingerprint(text: &str) -> u64 {
  let mut hash = DefaultHasher::new();
  text.hash(&mut hash);
  hash.finish()
}

impl CapturePolicy {
  pub fn own_write(&mut self, text: &str, now: Duration) {
    if let Some((url, _)) = supported_url(text) {
      self.own.insert(fingerprint(&url), now);
    }
  }

  #[cfg(test)]
  pub fn observe(&mut self, text: &str, now: Duration) -> Option<(String, String)> {
    let offer = self.decide(text, now).offer();
    if offer.is_some() {
      self.acknowledge(text, now);
    }
    offer
  }

  /// Only a card actually shown to the user consumes a link. Skipped/failed displays may retry.
  pub fn acknowledge(&mut self, text: &str, now: Duration) {
    if let Some((url, _)) = supported_url(text) {
      self.last = Some(fingerprint(text));
      self.seen.insert(fingerprint(&url), now);
    }
  }

  pub fn decide(&mut self, text: &str, now: Duration) -> Decision {
    if text.len() > 8192 {
      self.last = None;
      return Decision::TooLong;
    }
    let raw_hash = fingerprint(text);
    if self.last == Some(raw_hash) {
      return Decision::Unchanged;
    }
    self.last = Some(raw_hash);
    let ttl = Duration::from_secs(600);
    self.seen.retain(|_, at| now.saturating_sub(*at) < ttl);
    self.own.retain(|_, at| now.saturating_sub(*at) < ttl);
    let Some((url, site)) = supported_url(text) else {
      return Decision::Unsupported;
    };
    let key = fingerprint(&url);
    if self.own.contains_key(&key) {
      return Decision::Own;
    }
    if self.seen.contains_key(&key) {
      return Decision::Seen;
    }
    // The watcher may skip this link while the quick bar is open, or fail to show the card.
    // Keep it eligible until the native window confirms delivery.
    self.last = None;
    Decision::Offer(url, site)
  }
}

/// What the watcher made of one clipboard read; `kind` is the content-free label for diagnostics.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
  Unchanged,
  TooLong,
  Unsupported,
  Own,
  Seen,
  Offer(String, String),
}

impl Decision {
  pub fn kind(&self) -> &'static str {
    match self {
      Self::Unchanged => "unchanged",
      Self::TooLong => "too-long",
      Self::Unsupported => "unsupported",
      Self::Own => "own-write",
      Self::Seen => "already-offered",
      Self::Offer(..) => "offer",
    }
  }

  pub fn offer(self) -> Option<(String, String)> {
    match self {
      Self::Offer(url, site) => Some((url, site)),
      _ => None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  const URL: &str = "https://www.youtube.com/watch?v=abc";

  #[test]
  fn accepts_supported_hosts_only_and_normalizes_for_dedupe() {
    assert_eq!(
      supported_url(" HTTPS://WWW.YouTube.com/watch?v=abc#t=2 "),
      Some((URL.into(), "www.youtube.com".into()))
    );
    assert!(supported_url("https://m.youtube.com/watch?v=abc").is_some());
    // Links the share buttons of post sites copy.
    for text in [
      "https://lnkd.in/p/eYKXMxJS",
      "https://www.linkedin.com/posts/someone_activity-1-abc",
      "https://www.threads.com/@a/post/b",
      "https://bsky.app/profile/a/post/b",
      "https://t.me/channel/12",
      "https://pin.it/abc",
      "https://www.dropbox.com/s/abc/report.pdf?dl=0",
      "dropbox.com/s/abc/report.pdf?dl=0",
      "http://drive.google.com/file/d/abc/view",
      "https://docs.google.com/document/d/abc/edit",
      "https://www.snapchat.com/spotlight/abc",
      "https://x.com/a/status/1/photo/1",
      // Every host of a site counts (sites.rs), embeds included.
      "https://www.youtube-nocookie.com/embed/abc",
    ] {
      assert!(supported_url(text).is_some(), "rejected share link: {text}");
    }
    for text in [
      "hello",
      "file:///youtube.com/a",
      "javascript:youtube.com",
      "https://youtube.com.evil.test/a",
      "https://notyoutube.com/a",
      "https://youtube.com@evil.test/a",
      "https://evil.test/youtube.com",
      "https://youtube.com\\@evil.test/a",
      "https://youtube.com/a another secret",
      "https://127.0.0.1/a",
      "https://youtube.com:99999/a",
      "https://foo..youtube.com/a",
      "https://youtube.com/\nprivate",
      "https://t.me/+AbCdEf123",
      "https://t.me/joinchat/AbCdEf",
      "https://t.me/c/123456/7",
      "https://lnkd.in/dXyZ123",
      "https://drive.google.com/drive/folders/abc",
      "https://docs.google.com/forms/d/abc/viewform",
      "https://dropbox.com/home",
      "https://sub.dropbox.com/s/abc/report.pdf",
    ] {
      assert!(
        supported_url(text).is_none(),
        "unexpectedly accepted fixture: {text}"
      );
    }
  }

  #[test]
  fn a_skipped_or_failed_card_does_not_consume_the_clipboard_link() {
    let mut policy = CapturePolicy::default();
    assert!(matches!(
      policy.decide(URL, Duration::ZERO),
      Decision::Offer(..)
    ));
    // A quick bar was open, or the native card failed to show: no delivery acknowledgement.
    assert!(matches!(
      policy.decide(URL, Duration::from_millis(500)),
      Decision::Offer(..)
    ));
    policy.decide("another copy", Duration::from_secs(1));
    assert!(matches!(
      policy.decide(URL, Duration::from_secs(2)),
      Decision::Offer(..)
    ));
  }

  #[test]
  fn a_shown_card_stays_acknowledged_after_dismiss_or_action_and_refocus() {
    let mut policy = CapturePolicy::default();
    assert!(matches!(
      policy.decide(URL, Duration::ZERO),
      Decision::Offer(..)
    ));
    policy.acknowledge(URL, Duration::ZERO);
    // Dismiss/download does not reset the clipboard policy; refocus must not resurrect the card.
    assert_eq!(
      policy.decide(URL, Duration::from_secs(1)),
      Decision::Unchanged
    );
    policy.decide("another copy", Duration::from_secs(2));
    assert_eq!(policy.decide(URL, Duration::from_secs(3)), Decision::Seen);
    assert!(matches!(
      policy.decide("https://youtu.be/second", Duration::from_secs(4)),
      Decision::Offer(..)
    ));
  }

  #[test]
  fn dedupes_urls_for_ten_minutes_even_after_other_copies() {
    let mut policy = CapturePolicy::default();
    assert!(policy.observe(URL, Duration::ZERO).is_some());
    assert!(policy.observe(URL, Duration::from_secs(1)).is_none());
    policy.observe("unrelated text", Duration::from_secs(2));
    assert!(policy.observe(URL, Duration::from_secs(599)).is_none());
    policy.observe("unrelated text", Duration::from_secs(600));
    assert!(policy.observe(URL, Duration::from_secs(600)).is_some());
  }

  #[test]
  fn self_write_suppression_survives_intervening_copies_but_expires() {
    let mut policy = CapturePolicy::default();
    policy.own_write(URL, Duration::ZERO);
    policy.observe("another copy", Duration::from_secs(1));
    // Diagnostics label each read without its text.
    let mut fresh = CapturePolicy::default();
    let kinds: Vec<_> = [URL, URL, "hello", URL]
      .iter()
      .map(|text| {
        let decision = fresh.decide(text, Duration::ZERO);
        if matches!(decision, Decision::Offer(..)) {
          fresh.acknowledge(text, Duration::ZERO);
        }
        decision.kind()
      })
      .collect();
    assert_eq!(
      kinds,
      ["offer", "unchanged", "unsupported", "already-offered"]
    );
    assert_eq!(
      fresh.decide(&"x".repeat(9000), Duration::ZERO).kind(),
      "too-long"
    );
    assert!(policy.observe(URL, Duration::from_secs(2)).is_none());
    policy.observe("another copy", Duration::from_secs(601));
    assert!(policy.observe(URL, Duration::from_secs(601)).is_some());
  }

  #[test]
  fn feed_pages_are_flagged_and_posts_are_not() {
    for url in [
      "https://www.linkedin.com/feed/",
      "https://www.linkedin.com/feed/?trk=x",
      "https://www.facebook.com/",
      "https://www.facebook.com/home",
      "https://www.instagram.com/",
      "https://x.com/home",
      "https://twitter.com/home",
      "https://www.tiktok.com/",
      "https://www.tiktok.com/foryou",
      "https://www.reddit.com/",
      "https://www.reddit.com/r/rust/",
      "https://www.reddit.com/r/rust",
      "https://www.reddit.com/r/rust/hot/",
      "https://www.reddit.com/r/rust/top/?t=week",
      "https://www.youtube.com/",
      "https://www.youtube.com/feed/subscriptions",
    ] {
      let (url, _) = supported_url(url).unwrap();
      assert!(is_feed_page(&url), "not flagged as feed: {url}");
    }
    for url in [
      "https://www.linkedin.com/feed/update/urn:li:activity:123/",
      "https://www.linkedin.com/posts/someone_activity-1-abc",
      "https://www.facebook.com/watch/?v=1",
      "https://www.facebook.com/user/posts/1",
      "https://www.instagram.com/reel/abc/",
      "https://www.instagram.com/p/abc/",
      "https://x.com/a/status/1",
      "https://www.tiktok.com/@a/video/1",
      "https://www.reddit.com/r/rust/comments/abc/title/",
      "https://www.reddit.com/r/soccer/s/AbC123xyz",
      "https://www.youtube.com/watch?v=abc",
      "https://www.youtube.com/shorts/abc",
      "https://youtu.be/abc",
    ] {
      let (url, _) = supported_url(url).unwrap();
      assert!(!is_feed_page(&url), "post flagged as feed: {url}");
    }
  }
}
