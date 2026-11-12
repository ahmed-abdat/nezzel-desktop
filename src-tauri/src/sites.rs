//! Which site a host belongs to: a stable id, the hosts that mean it, and its display name.
//! Nothing else lives here. Each feature keeps its own per-site rules keyed by `SiteId`
//! (capture allow/feed rules, routing, cookie domains, report route words).
//! Usage counts never read this table: they count the registrable domain as written
//! (`youtu.be` stays `youtu.be`, docs/specs/usage-counts.md).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SiteId {
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
  LinkedIn,
  Threads,
  Bluesky,
  Telegram,
  Snapchat,
  GoogleDrive,
  GoogleDocs,
  Dropbox,
  Tumblr,
  Kick,
  Vk,
  Ok,
  Streamable,
  Imgur,
  NineGag,
  Likee,
  Kwai,
  Flickr,
}

pub(crate) struct Site {
  pub id: SiteId,
  /// Stable lowercase id (the contract's `SITES[].key`).
  // key, name and extractors reach the webview through contract.rs (built in tests only).
  #[cfg_attr(not(test), allow(dead_code))]
  pub key: &'static str,
  #[cfg_attr(not(test), allow(dead_code))]
  pub name: &'static str,
  /// The site's own domains, main one first (`x.com`, `twitter.com`). A host matches a domain
  /// or any subdomain of it, never a lookalike (`notx.com`).
  pub domains: &'static [&'static str],
  /// Short-link and embed hosts that open the site (`youtu.be`, `fb.watch`).
  pub short_links: &'static [&'static str],
  /// yt-dlp extractor keys (lowercase) that name this site, for display.
  #[cfg_attr(not(test), allow(dead_code))]
  pub extractors: &'static [&'static str],
}

const fn site(
  id: SiteId,
  key: &'static str,
  name: &'static str,
  domains: &'static [&'static str],
  short_links: &'static [&'static str],
  extractors: &'static [&'static str],
) -> Site {
  Site {
    id,
    key,
    name,
    domains,
    short_links,
    extractors,
  }
}

/// In `SiteId` order (`SiteId::site` indexes it).
pub(crate) const SITES: &[Site] = {
  use SiteId::*;
  &[
    site(
      YouTube,
      "youtube",
      "YouTube",
      &["youtube.com"],
      &["youtu.be", "youtube-nocookie.com"],
      &["youtube", "youtubetab"],
    ),
    site(Vimeo, "vimeo", "Vimeo", &["vimeo.com"], &[], &["vimeo"]),
    site(
      TikTok,
      "tiktok",
      "TikTok",
      &["tiktok.com"],
      &[],
      &["tiktok"],
    ),
    site(
      Instagram,
      "instagram",
      "Instagram",
      &["instagram.com"],
      &[],
      &["instagram"],
    ),
    site(
      Facebook,
      "facebook",
      "Facebook",
      &["facebook.com"],
      &["fb.watch"],
      &["facebook"],
    ),
    site(
      X,
      "x",
      "X",
      &["x.com", "twitter.com"],
      &["fxtwitter.com", "vxtwitter.com"],
      &["twitter"],
    ),
    site(Twitch, "twitch", "Twitch", &["twitch.tv"], &[], &["twitch"]),
    site(
      SoundCloud,
      "soundcloud",
      "SoundCloud",
      &["soundcloud.com"],
      &[],
      &["soundcloud"],
    ),
    site(
      Dailymotion,
      "dailymotion",
      "Dailymotion",
      &["dailymotion.com"],
      &["dai.ly"],
      &["dailymotion"],
    ),
    site(
      Reddit,
      "reddit",
      "Reddit",
      &["reddit.com"],
      &["redd.it"],
      &["reddit"],
    ),
    site(
      Bilibili,
      "bilibili",
      "Bilibili",
      &["bilibili.com"],
      &["b23.tv"],
      &[],
    ),
    site(
      Pinterest,
      "pinterest",
      "Pinterest",
      &["pinterest.com"],
      &["pin.it"],
      &[],
    ),
    site(Ted, "ted", "TED", &["ted.com"], &[], &[]),
    site(
      Bandcamp,
      "bandcamp",
      "Bandcamp",
      &["bandcamp.com"],
      &[],
      &[],
    ),
    site(
      InternetArchive,
      "archive",
      "Internet Archive",
      &["archive.org"],
      &[],
      &[],
    ),
    site(Rumble, "rumble", "Rumble", &["rumble.com"], &[], &[]),
    site(Odysee, "odysee", "Odysee", &["odysee.com"], &[], &[]),
    site(
      LinkedIn,
      "linkedin",
      "LinkedIn",
      &["linkedin.com"],
      &["lnkd.in"],
      &[],
    ),
    site(
      Threads,
      "threads",
      "Threads",
      &["threads.net", "threads.com"],
      &[],
      &[],
    ),
    site(Bluesky, "bluesky", "Bluesky", &["bsky.app"], &[], &[]),
    site(
      Telegram,
      "telegram",
      "Telegram",
      &["t.me", "telegram.me"],
      &[],
      &[],
    ),
    site(
      Snapchat,
      "snapchat",
      "Snapchat",
      &["snapchat.com"],
      &[],
      &[],
    ),
    site(
      GoogleDrive,
      "googledrive",
      "Google Drive",
      &["drive.google.com"],
      &[],
      &[],
    ),
    site(
      GoogleDocs,
      "googledocs",
      "Google Docs",
      &["docs.google.com"],
      &[],
      &[],
    ),
    site(Dropbox, "dropbox", "Dropbox", &["dropbox.com"], &[], &[]),
    site(
      Tumblr,
      "tumblr",
      "Tumblr",
      &["tumblr.com"],
      &[],
      &["tumblr"],
    ),
    site(Kick, "kick", "Kick", &["kick.com"], &[], &["kick:clips"]),
    site(Vk, "vk", "VK", &["vk.com", "vkvideo.ru"], &[], &["vk"]),
    site(Ok, "ok", "OK.ru", &["ok.ru"], &[], &["odnoklassniki"]),
    site(
      Streamable,
      "streamable",
      "Streamable",
      &["streamable.com"],
      &[],
      &["streamable"],
    ),
    site(Imgur, "imgur", "Imgur", &["imgur.com"], &[], &["imgur"]),
    site(NineGag, "9gag", "9GAG", &["9gag.com"], &[], &["9gag"]),
    site(Likee, "likee", "Likee", &["likee.video"], &[], &["likee"]),
    // Identity for display/refusal only; not in capture's supported-site allowlist.
    site(Kwai, "kwai", "Kwai", &["kwai.com"], &[], &[]),
    site(
      Flickr,
      "flickr",
      "Flickr",
      &["flickr.com"],
      &["flic.kr"],
      &["flickr"],
    ),
  ]
};

impl SiteId {
  pub(crate) const fn site(self) -> &'static Site {
    &SITES[self as usize]
  }

  /// The main domain (`x.com` for X).
  pub(crate) const fn domain(self) -> &'static str {
    self.site().domains[0]
  }

  pub(crate) const fn domains(self) -> &'static [&'static str] {
    self.site().domains
  }
}

/// `host` is `domain` or a subdomain of it (dot boundary, so `notx.com` is not `x.com`).
pub(crate) fn host_is(host: &str, domain: &str) -> bool {
  host
    .strip_suffix(domain)
    .is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
}

fn find(host: &str, hosts: fn(&Site) -> &[&'static str]) -> Option<SiteId> {
  SITES
    .iter()
    .find(|site| hosts(site).iter().any(|domain| host_is(host, domain)))
    .map(|site| site.id)
}

/// The site a lowercase host belongs to, through its domains or its short links.
pub(crate) fn site_of(host: &str) -> Option<SiteId> {
  find(host, |site| site.domains).or_else(|| find(host, |site| site.short_links))
}

/// Like `site_of`, but only the site's own domains count (`youtu.be` is not YouTube here).
pub(crate) fn site_by_domain(host: &str) -> Option<SiteId> {
  find(host, |site| site.domains)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_table_is_in_id_order_and_every_host_names_one_site() {
    let mut seen = std::collections::HashSet::new();
    for (index, site) in SITES.iter().enumerate() {
      assert_eq!(site.id as usize, index, "{}", site.key);
      assert!(!site.domains.is_empty(), "{}", site.key);
      for host in site.domains.iter().chain(site.short_links) {
        assert_eq!(*host, host.to_ascii_lowercase());
        assert!(seen.insert(*host), "{host} listed twice");
      }
    }
  }

  #[test]
  fn hosts_resolve_to_their_site_on_a_dot_boundary() {
    for (host, any, by_domain) in [
      ("x.com", Some(SiteId::X), Some(SiteId::X)),
      ("twitter.com", Some(SiteId::X), Some(SiteId::X)),
      ("mobile.twitter.com", Some(SiteId::X), Some(SiteId::X)),
      ("nottwitter.com", None, None),
      ("x.com.evil.net", None, None),
      ("youtu.be", Some(SiteId::YouTube), None),
      (
        "music.youtube.com",
        Some(SiteId::YouTube),
        Some(SiteId::YouTube),
      ),
      ("youtube-nocookie.com", Some(SiteId::YouTube), None),
      ("v.redd.it", Some(SiteId::Reddit), None),
      ("threads.com", Some(SiteId::Threads), Some(SiteId::Threads)),
      (
        "telegram.me",
        Some(SiteId::Telegram),
        Some(SiteId::Telegram),
      ),
      (
        "drive.google.com",
        Some(SiteId::GoogleDrive),
        Some(SiteId::GoogleDrive),
      ),
      ("google.com", None, None),
      (
        "www.dropbox.com",
        Some(SiteId::Dropbox),
        Some(SiteId::Dropbox),
      ),
      ("", None, None),
    ] {
      assert_eq!(site_of(host), any, "{host}");
      assert_eq!(site_by_domain(host), by_domain, "{host}");
    }
    assert_eq!(SiteId::X.domain(), "x.com");
    assert_eq!(SiteId::Threads.domain(), "threads.net");
  }
}
