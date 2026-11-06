//! The one place a download link is reduced for a report (owner ruling 2026-11-04).
//! Automatic reports get the site and the link's shape: known route words stay, every other
//! path segment (IDs, share keys, user names, slugs) becomes `{id}`, the query is dropped
//! (YouTube keeps parameter names, never values). A user-initiated report may include the
//! full link when they leave "Include the link" checked. Never local, private or credentials.

use crate::sites::{self, SiteId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportLink {
  /// Host without `www.`, e.g. `instagram.com`.
  pub site: SiteToken,
  /// `instagram.com/reel/{id}`, or the full link when the user chose to include it.
  pub link: LinkText,
}

/// Text only [`link_for_report`] builds (private field): a site or a reduced link, never a raw URL.
macro_rules! reduced_text {
  ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
    $(#[$doc])*
    #[derive(Clone, PartialEq, Eq)]
    pub struct $name(String);

    impl $name {
      // Not every type reads its text back outside Display.
      #[allow(dead_code)]
      pub fn as_str(&self) -> &str {
        &self.0
      }
    }

    impl std::fmt::Display for $name {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
      }
    }

    impl std::fmt::Debug for $name {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
      }
    }

    impl PartialEq<&str> for $name {
      fn eq(&self, other: &&str) -> bool {
        self.0 == *other
      }
    }
  )*};
}

reduced_text!(
  /// A public site as reports name it (`instagram.com`, `music.youtube.com`).
  SiteToken,
  /// The link's shape (`instagram.com/reel/{id}`), or the full link without credentials or secret parameters.
  LinkText,
);

/// Subdomains that name a product, not an account.
const SERVICE_LABELS: &[&str] = &[
  "m", "mobile", "music", "drive", "docs", "photos", "old", "new", "vm", "vt", "player", "video",
  "videos", "open", "play", "web", "embed", "media", "clips", "studio", "on", "fb",
];

/// Route words that name a page type, not a person or an item.
const ROUTE_WORDS: &[&str] = &[
  "watch",
  "shorts",
  "live",
  "embed",
  "playlist",
  "channel",
  "c",
  "user",
  "v",
  "e",
  "reel",
  "reels",
  "p",
  "tv",
  "stories",
  "status",
  "statuses",
  "video",
  "videos",
  "photo",
  "photos",
  "post",
  "posts",
  "file",
  "d",
  "view",
  "preview",
  "open",
  "uc",
  "s",
  "sh",
  "scl",
  "fi",
  "fo",
  "comments",
  "r",
  "share",
  "clip",
  "clips",
  "music",
  "track",
  "tracks",
  "album",
  "albums",
  "sets",
  "t",
  "i",
  "media",
  "watchv",
  "story",
  "pin",
  "gallery",
  "a",
  "w",
  "groups",
  "events",
  "permalink",
  "download",
  "dl",
  "folders",
  "folder",
  "drive",
  "u",
  "m",
  "plugins",
  "audio",
  "podcast",
  "episode",
  "episodes",
  "show",
  "shows",
  "embed_video",
  "broadcasts",
  "spaces",
  "highlights",
];

/// Last labels that name a private network, never a site on the internet (`acme.corp`, `nas.home.arpa`, `box.test`).
/// The one list both reports and usage counts check.
const PRIVATE_TLDS: &[&str] = &[
  "arpa",
  "test",
  "example",
  "invalid",
  "localhost",
  "local",
  "internal",
  "intranet",
  "corp",
  "home",
  "lan",
  "private",
];

/// A web link cut into the parts reports and counts read. `host` is lower case and public ([`public_host`]).
struct WebLink<'a> {
  scheme: String,
  /// Host and port, credentials dropped, case as typed.
  host_port: &'a str,
  host: String,
  /// Path, query and fragment.
  tail: &'a str,
}

fn web_link(url: &str) -> Option<WebLink<'_>> {
  let url = url.trim();
  let (scheme, rest) = match url.split_once("://") {
    Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
    None => ("https".to_string(), url),
  };
  if scheme != "http" && scheme != "https" {
    return None;
  }
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let (authority, tail) = rest.split_at(end);
  // Drop `user:password@`; never part of a report.
  let host_port = authority.rsplit('@').next()?;
  let host = host_port.split(':').next()?.to_ascii_lowercase();
  public_host(&host).then_some(WebLink {
    scheme,
    host_port,
    host,
    tail,
  })
}

/// The site of a link: its registrable domain (drive.google.com → google.com, alice.tumblr.com → tumblr.com), or
/// for a private-section suffix that hosts one account per subdomain, that suffix's registrable name
/// (alice.github.io → github.io). No product label and no alias (youtu.be stays youtu.be, usage spec). None for a
/// non-web link, an IP, a private network name or a host without a listed public suffix.
pub fn site_of_url(url: &str) -> Option<String> {
  registrable(&web_link(url)?.host)
}

fn registrable(host: &str) -> Option<String> {
  let name = host.strip_prefix("www.").unwrap_or(host);
  let suffix = psl::suffix(name.as_bytes())?;
  match suffix.typ()? {
    psl::Type::Private => icann_registrable(std::str::from_utf8(suffix.as_bytes()).ok()?),
    psl::Type::Icann => psl::domain_str(name).map(str::to_string),
  }
}

/// The nearest ICANN suffix plus one label, for a name under a private-section suffix. Private suffixes can be a
/// whole host (ec2-54-12-3-4.compute-1.amazonaws.com carries an IP) or name an account (y.platformsh.site):
/// `amazonaws.com`, `platformsh.site`, `github.io`.
fn icann_registrable(name: &str) -> Option<String> {
  let labels: Vec<&str> = name.split('.').collect();
  let icann = (1..=labels.len())
    .filter(|&k| {
      let candidate = labels[labels.len() - k..].join(".");
      psl::suffix(candidate.as_bytes()).is_some_and(|suffix| {
        suffix.typ() == Some(psl::Type::Icann) && suffix.as_bytes() == candidate.as_bytes()
      })
    })
    .max()?;
  (icann < labels.len()).then(|| labels[labels.len() - icann - 1..].join("."))
}

pub fn link_for_report(url: &str, include_full: bool) -> Option<ReportLink> {
  let link = web_link(url)?;
  let domain = registrable(&link.host)?;
  // A product label (drive., music.) is kept for the report: it says which service failed. Other subdomains can
  // name a person or company (alice.tumblr.com, acme.sharepoint.com), and private-section suffixes host one
  // account per subdomain: those reduce to the site alone.
  let name = link.host.strip_prefix("www.").unwrap_or(&link.host);
  let icann =
    psl::suffix(name.as_bytes()).and_then(|suffix| suffix.typ()) == Some(psl::Type::Icann);
  let site = match name
    .strip_suffix(domain.as_str())
    .and_then(|rest| rest.strip_suffix('.'))
  {
    Some(label) if icann && SERVICE_LABELS.contains(&label) => name.to_string(),
    _ => domain,
  };
  let text = if include_full {
    format!(
      "{scheme}://{host_port}{tail}",
      scheme = link.scheme,
      host_port = link.host_port.to_ascii_lowercase(),
      tail = strip_secrets(link.tail)
    )
  } else {
    link_shape(&site, link.tail)
  };
  Some(ReportLink {
    site: SiteToken(site),
    link: LinkText(text),
  })
}

/// `instagram.com/reel/{id}`: the site, known route words, `{id}` for every other segment, no query (YouTube
/// keeps parameter names, never values).
fn link_shape(site: &str, tail: &str) -> String {
  let (path, query) = match tail.split_once('?') {
    Some((path, query)) => (path, Some(query.split('#').next().unwrap_or(""))),
    None => (tail.split('#').next().unwrap_or(""), None),
  };
  let segments: Vec<String> = path
    .split('/')
    .filter(|segment| !segment.is_empty())
    .map(str::to_ascii_lowercase)
    .collect();
  let mut shape = site.to_string();
  shape.push_str(&path_shape(site, &segments));
  if sites::site_by_domain(site) == Some(SiteId::YouTube) || site == "youtu.be" {
    if let Some(query) = query.filter(|query| !query.is_empty()) {
      let names: Vec<String> = query
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .filter(|name| {
          !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(|name| format!("{name}={{id}}"))
        .collect();
      if !names.is_empty() {
        shape.push('?');
        shape.push_str(&names.join("&"));
      }
    }
  }
  shape
}

/// Positional schemas for the sites people report most: a literal segment is kept only where
/// the site's own routes put it, so an ID that happens to equal a route word stays masked.
/// `*` is any segment. The first matching schema wins.
const SCHEMAS: &[(SiteId, &[&[&str]])] = &[
  (
    SiteId::YouTube,
    &[
      &["watch"],
      &["playlist"],
      &["shorts", "*"],
      &["live", "*"],
      &["embed", "*"],
      &["channel", "*"],
      &["c", "*"],
      &["user", "*"],
      &["*", "videos"],
      &["*", "shorts"],
      &["*", "streams"],
      &["*", "live"],
      &["*"],
    ],
  ),
  (
    SiteId::Instagram,
    &[
      &["p", "*"],
      &["reel", "*"],
      &["reels", "*"],
      &["tv", "*"],
      &["stories", "*", "*"],
      &["*", "p", "*"],
      &["*", "reel", "*"],
      &["*"],
    ],
  ),
  (
    SiteId::TikTok,
    &[
      &["*", "video", "*"],
      &["*", "photo", "*"],
      &["t", "*"],
      &["*"],
    ],
  ),
  (
    SiteId::X,
    &[
      &["*", "status", "*"],
      &["*", "status", "*", "video", "*"],
      &["i", "status", "*"],
      &["*"],
    ],
  ),
  (
    SiteId::Facebook,
    &[
      &["watch"],
      &["reel", "*"],
      &["share", "v", "*"],
      &["share", "r", "*"],
      &["*", "videos", "*"],
      &["*", "posts", "*"],
      &["*"],
    ],
  ),
  (
    SiteId::GoogleDrive,
    &[
      &["file", "d", "*", "view"],
      &["file", "d", "*"],
      &["uc"],
      &["open"],
      &["drive", "folders", "*"],
    ],
  ),
  (
    SiteId::Dropbox,
    &[
      &["s", "*", "*"],
      &["scl", "fi", "*", "*"],
      &["sh", "*", "*"],
    ],
  ),
  (
    SiteId::Reddit,
    &[
      &["r", "*", "comments", "*", "*"],
      &["r", "*", "comments", "*"],
      &["r", "*", "s", "*"],
      &["user", "*", "comments", "*", "*"],
    ],
  ),
  (SiteId::Vimeo, &[&["channels", "*", "*"], &["*"]]),
];

/// Short links that carry their own route shape (matched by host, not by site).
const SHORT_LINK_SCHEMAS: &[(&str, &[&[&str]])] = &[("youtu.be", &[&["*"]])];

/// `/reel/{id}`: a known site's schema, else only a leading route word survives.
fn path_shape(site: &str, segments: &[String]) -> String {
  let key = sites::site_by_domain(site)
    .and_then(|id| SCHEMAS.iter().find(|(schema_site, _)| *schema_site == id))
    .map(|(_, schemas)| *schemas)
    .or_else(|| {
      SHORT_LINK_SCHEMAS
        .iter()
        .find(|(host, _)| sites::host_is(site, host))
        .map(|(_, schemas)| *schemas)
    });
  let masked = |keep: &dyn Fn(usize, &str) -> bool| {
    segments
      .iter()
      .enumerate()
      .map(|(index, segment)| {
        if keep(index, segment) {
          format!("/{segment}")
        } else {
          "/{id}".to_string()
        }
      })
      .collect::<String>()
  };
  if let Some(schemas) = key {
    let found = schemas.iter().find(|schema| {
      schema.len() == segments.len()
        && schema
          .iter()
          .zip(segments)
          .all(|(part, segment)| *part == "*" || part == segment)
    });
    return match found {
      Some(schema) => masked(&|index, _| schema[index] != "*"),
      // An unknown route on a known site: nothing is trusted.
      None => masked(&|_, _| false),
    };
  }
  masked(&|index, segment| index == 0 && ROUTE_WORDS.contains(&segment))
}

/// The checked full link still never carries keys: drop the `#…` part (Mega keys live there)
/// and query parameters that hold secrets or tracking.
fn strip_secrets(tail: &str) -> String {
  let tail = tail.split('#').next().unwrap_or("");
  let Some((path, query)) = tail.split_once('?') else {
    return tail.to_string();
  };
  let kept: Vec<&str> = query
    .split('&')
    .filter(|pair| {
      let name = pair.split('=').next().unwrap_or("").to_ascii_lowercase();
      !name.is_empty() && !secret_param(&name)
    })
    .collect();
  if kept.is_empty() {
    path.to_string()
  } else {
    format!("{path}?{}", kept.join("&"))
  }
}

fn secret_param(name: &str) -> bool {
  const EXACT: &[&str] = &[
    "sig",
    "st",
    "e",
    "x-amz-credential",
    "code",
    "sid",
    "si",
    "igsh",
    "igshid",
    "fbclid",
    "gclid",
    "s",
    "ref",
    "share_id",
    "u",
    "usp",
  ];
  const PARTS: &[&str] = &[
    "token",
    "key",
    "secret",
    "sign",
    "auth",
    "pass",
    "pwd",
    "session",
    "credential",
    "expires",
    "utm_",
    "share",
  ];
  EXACT.contains(&name) || PARTS.iter().any(|part| name.contains(part))
}

/// A site on the internet: not localhost, an IP address, a single-label name or a private network name
/// ([`PRIVATE_TLDS`]).
fn public_host(host: &str) -> bool {
  let label_ok =
    |label: &str| !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
  let ip = host.chars().all(|c| c.is_ascii_digit() || c == '.') || host.starts_with('[');
  let private = host
    .rsplit('.')
    .next()
    .is_some_and(|tld| PRIVATE_TLDS.contains(&tld));
  host.contains('.') && host.split('.').all(label_ok) && !ip && !private
}

#[cfg(test)]
mod tests {
  use super::*;

  fn shape(url: &str) -> Option<String> {
    link_for_report(url, false).map(|link| link.link.to_string())
  }

  #[test]
  fn automatic_reports_get_the_link_shape_only() {
    for (url, expected) in [
      (
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s&list=PL1",
        Some("youtube.com/watch?v={id}&t={id}&list={id}"),
      ),
      (
        "https://youtu.be/dQw4w9WgXcQ?si=share",
        Some("youtu.be/{id}?si={id}"),
      ),
      (
        "https://www.youtube.com/shorts/abc123",
        Some("youtube.com/shorts/{id}"),
      ),
      // Aliases keep their own name; a lookalike gets no site rules (sites.rs dot boundary).
      (
        "https://twitter.com/alice/status/123",
        Some("twitter.com/{id}/status/{id}"),
      ),
      ("https://redd.it/abc123", Some("redd.it/{id}")),
      (
        "https://notyoutube.com/watch?v=1",
        Some("notyoutube.com/watch"),
      ),
      (
        "https://www.youtube.com/@SomeChannel/videos",
        Some("youtube.com/{id}/videos"),
      ),
      (
        "https://www.instagram.com/reel/Cx1AbC/?igsh=MTc4",
        Some("instagram.com/reel/{id}"),
      ),
      (
        "https://www.instagram.com/sara.private/",
        Some("instagram.com/{id}"),
      ),
      (
        "https://www.tiktok.com/@user.name/video/7301234567890",
        Some("tiktok.com/{id}/video/{id}"),
      ),
      (
        "https://x.com/someone/status/1790000000000000000?s=20",
        Some("x.com/{id}/status/{id}"),
      ),
      (
        "https://drive.google.com/file/d/1AbCSecretId_x/view?usp=sharing",
        Some("drive.google.com/file/d/{id}/view"),
      ),
      (
        "https://drive.google.com/uc?id=1Secret&export=download",
        Some("drive.google.com/uc"),
      ),
      (
        "https://www.dropbox.com/scl/fi/abc123/My%20Video.mp4?rlkey=secret&dl=0",
        Some("dropbox.com/scl/fi/{id}/{id}"),
      ),
      (
        // An ID or user name equal to a route word stays masked.
        "https://www.instagram.com/music/",
        Some("instagram.com/{id}"),
      ),
      ("https://x.com/status", Some("x.com/{id}")),
      (
        "https://www.youtube.com/watch/extra",
        Some("youtube.com/{id}/{id}"),
      ),
      (
        "https://alice.github.io/blog/video.mp4",
        Some("github.io/{id}/{id}"),
      ),
      (
        "https://sara.blogspot.com/2026/11/post.html",
        Some("blogspot.com/{id}/{id}/{id}"),
      ),
      (
        "https://ec2-54-12-3-4.compute-1.amazonaws.com/video.mp4",
        Some("amazonaws.com/{id}"),
      ),
      (
        "https://x.y.platformsh.site/clip.mp4",
        Some("platformsh.site/{id}"),
      ),
      (
        "https://alice.tumblr.com/post/123/my-trip",
        Some("tumblr.com/post/{id}/{id}"),
      ),
      (
        "https://acme-corp.sharepoint.com/:v:/s/team/EabC",
        Some("sharepoint.com/{id}/{id}/{id}/{id}"),
      ),
      (
        "https://vm.tiktok.com/ZMabc123/",
        Some("vm.tiktok.com/{id}"),
      ),
      (
        "https://music.youtube.com/watch?v=abc",
        Some("music.youtube.com/watch?v={id}"),
      ),
      (
        "https://www.reddit.com/r/videos/s/AbCdEf123",
        Some("reddit.com/r/{id}/s/{id}"),
      ),
      (
        "https://www.reddit.com/r/funny/comments/1abc/my_title_slug/",
        Some("reddit.com/r/{id}/comments/{id}/{id}"),
      ),
      (
        "https://www.aljazeera.net/video/2026/11/4/فيديو-الحدث",
        Some("aljazeera.net/video/{id}/{id}/{id}/{id}"),
      ),
      (
        "https://user:hunter2@vimeo.com:443/123456",
        Some("vimeo.com/{id}"),
      ),
      ("instagram.com/p/DfQIowKutL3/", Some("instagram.com/p/{id}")),
      ("http://192.168.1.4:8080/video.mp4", None),
      ("http://localhost:3000/v.mp4", None),
      ("http://nas.local/films/a.mkv", None),
      // The same private network names usage counts skip.
      ("https://nas.acme.corp/v", None),
      ("http://printer.home.arpa/", None),
      ("https://box.test/v", None),
      ("https://wiki.intranet/a", None),
      ("http://[::1]/a", None),
      ("file:///Users/sara/Movies/a.mp4", None),
      ("not a link", None),
    ] {
      assert_eq!(shape(url).as_deref(), expected, "{url}");
    }
  }

  #[test]
  fn usage_counts_get_the_registrable_domain_only() {
    for (url, site) in [
      (
        "https://drive.google.com/file/d/abc/view",
        Some("google.com"),
      ),
      ("https://music.youtube.com/watch?v=1", Some("youtube.com")),
      ("https://www.bbc.co.uk/news", Some("bbc.co.uk")),
      ("https://alice.github.io/v.mp4", Some("github.io")),
      (
        "https://ec2-54-12-3-4.compute-1.amazonaws.com/v",
        Some("amazonaws.com"),
      ),
      ("https://x.y.platformsh.site/v", Some("platformsh.site")),
      ("https://nas.acme.corp/v", None),
      ("https://box.test/v", None),
      ("http://192.168.1.4/v", None),
      // characterisation: a short link host is counted apart from its site.
      ("https://youtu.be/dQw4w9WgXcQ", Some("youtu.be")),
      // Site aliases (sites.rs) never reach usage: each domain is counted as written.
      ("https://twitter.com/a/status/1", Some("twitter.com")),
      ("https://redd.it/abc", Some("redd.it")),
      ("https://www.threads.com/@a/post/1", Some("threads.com")),
      ("https://fb.watch/abc", Some("fb.watch")),
      (
        "https://www.youtube-nocookie.com/embed/1",
        Some("youtube-nocookie.com"),
      ),
      ("https://telegram.me/a/1", Some("telegram.me")),
      ("https://m.youtube.com/watch?v=1", Some("youtube.com")),
      ("https://WWW.YouTube.com/Watch", Some("youtube.com")),
      ("https://youtube.com:8443/x", Some("youtube.com")),
      ("https://bob:pw@vimeo.com/1", Some("vimeo.com")),
      ("instagram.com/p/abc", Some("instagram.com")),
      ("https://alice.herokuapp.com/v", Some("herokuapp.com")),
      ("https://sara.blogspot.com/v", Some("blogspot.com")),
      ("http://[2001:4860:4860::8888]/v", None),
      ("http://[::1]:8080/v", None),
      ("https://co.uk/v", None),
      ("ftp://youtube.com/x", None),
      ("", None),
    ] {
      assert_eq!(site_of_url(url).as_deref(), site, "{url}");
    }
  }

  #[test]
  fn the_user_may_include_the_full_link_without_credentials() {
    let link =
      link_for_report("https://bob:pw@WWW.Instagram.com/reel/Cx1/?igsh=abc", true).unwrap();
    assert_eq!(link.site, "instagram.com");
    assert_eq!(link.link, "https://www.instagram.com/reel/Cx1/");
    for (url, full) in [
      (
        "https://www.dropbox.com/scl/fi/abc/v.mp4?rlkey=secret&dl=0",
        "https://www.dropbox.com/scl/fi/abc/v.mp4?dl=0",
      ),
      (
        "https://mega.nz/file/AbCd#KeyMaterial",
        "https://mega.nz/file/AbCd",
      ),
      (
        "https://cdn.example.com/v.mp4?token=abc&Expires=1&Signature=x&Key-Pair-Id=k",
        "https://cdn.example.com/v.mp4",
      ),
      (
        "https://www.youtube.com/watch?v=abc&si=share&t=10",
        "https://www.youtube.com/watch?v=abc&t=10",
      ),
      (
        "https://drive.google.com/file/d/1A/view?usp=sharing&resourcekey=0-x",
        "https://drive.google.com/file/d/1A/view",
      ),
    ] {
      assert_eq!(link_for_report(url, true).unwrap().link, full, "{url}");
    }
    // Private and local links are never included, even when asked.
    assert_eq!(link_for_report("http://10.0.0.2/a.mp4", true), None);
  }
}
