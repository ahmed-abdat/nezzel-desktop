//! The one place a download link is reduced for a report (owner ruling 2026-11-04).
//! Automatic reports get the site and the link's shape: known route words stay, every other
//! path segment (IDs, share keys, user names, slugs) becomes `{id}`, the query is dropped
//! (YouTube keeps parameter names, never values). A user-initiated report may include the
//! full link when they leave "Include the link" checked. Never local, private or credentials.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportLink {
  /// Host without `www.`, e.g. `instagram.com`.
  pub site: String,
  /// `instagram.com/reel/{id}`, or the full link when the user chose to include it.
  pub link: String,
}

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

pub fn link_for_report(url: &str, include_full: bool) -> Option<ReportLink> {
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
  if !public_host(&host) {
    return None;
  }
  let site = host.strip_prefix("www.").unwrap_or(&host).to_string();
  // Subdomains can name a person or company (alice.tumblr.com, acme.sharepoint.com): keep the
  // registrable domain, plus a product label such as drive. or music.
  // Private-section suffixes (github.io, blogspot.com, herokuapp.com) host one account per
  // subdomain: the suffix itself is the site.
  let private_suffix = psl::suffix(site.as_bytes())
    .filter(|suffix| suffix.typ() == Some(psl::Type::Private))
    .and_then(|suffix| {
      std::str::from_utf8(suffix.as_bytes())
        .ok()
        .map(str::to_string)
    });
  let site = if let Some(suffix) = private_suffix {
    suffix
  } else {
    match psl::domain_str(&site).map(str::to_string) {
      Some(domain) if domain != site => match site
        .strip_suffix(&domain)
        .and_then(|rest| rest.strip_suffix('.'))
      {
        Some(label) if SERVICE_LABELS.contains(&label) => site.clone(),
        _ => domain,
      },
      Some(domain) => domain,
      None => return None,
    }
  };

  if include_full {
    return Some(ReportLink {
      link: format!(
        "{scheme}://{host_port}{tail}",
        host_port = host_port.to_ascii_lowercase(),
        tail = strip_secrets(tail)
      ),
      site,
    });
  }

  let (path, query) = match tail.split_once('?') {
    Some((path, query)) => (path, Some(query.split('#').next().unwrap_or(""))),
    None => (tail.split('#').next().unwrap_or(""), None),
  };
  let segments: Vec<String> = path
    .split('/')
    .filter(|segment| !segment.is_empty())
    .map(str::to_ascii_lowercase)
    .collect();
  let mut shape = site.clone();
  shape.push_str(&path_shape(&site, &segments));
  if site.ends_with("youtube.com") || site == "youtu.be" {
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
  Some(ReportLink { site, link: shape })
}

/// Positional schemas for the sites people report most: a literal segment is kept only where
/// the site's own routes put it, so an ID that happens to equal a route word stays masked.
/// `*` is any segment. The first matching schema wins.
const SCHEMAS: &[(&str, &[&[&str]])] = &[
  (
    "youtube.com",
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
  ("youtu.be", &[&["*"]]),
  (
    "instagram.com",
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
    "tiktok.com",
    &[
      &["*", "video", "*"],
      &["*", "photo", "*"],
      &["t", "*"],
      &["*"],
    ],
  ),
  (
    "x.com",
    &[
      &["*", "status", "*"],
      &["*", "status", "*", "video", "*"],
      &["i", "status", "*"],
      &["*"],
    ],
  ),
  (
    "twitter.com",
    &[
      &["*", "status", "*"],
      &["*", "status", "*", "video", "*"],
      &["i", "status", "*"],
      &["*"],
    ],
  ),
  (
    "facebook.com",
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
    "drive.google.com",
    &[
      &["file", "d", "*", "view"],
      &["file", "d", "*"],
      &["uc"],
      &["open"],
      &["drive", "folders", "*"],
    ],
  ),
  (
    "dropbox.com",
    &[
      &["s", "*", "*"],
      &["scl", "fi", "*", "*"],
      &["sh", "*", "*"],
    ],
  ),
  (
    "reddit.com",
    &[
      &["r", "*", "comments", "*", "*"],
      &["r", "*", "comments", "*"],
      &["r", "*", "s", "*"],
      &["user", "*", "comments", "*", "*"],
    ],
  ),
  ("vimeo.com", &[&["channels", "*", "*"], &["*"]]),
];

/// `/reel/{id}`: a known site's schema, else only a leading route word survives.
fn path_shape(site: &str, segments: &[String]) -> String {
  let key = SCHEMAS
    .iter()
    .find(|(domain, _)| site == *domain || site.ends_with(&format!(".{domain}")));
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
  if let Some((_, schemas)) = key {
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

/// A site on the internet: not localhost, an IP address or a single-label/LAN name.
fn public_host(host: &str) -> bool {
  let label_ok =
    |label: &str| !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
  let ip = host.chars().all(|c| c.is_ascii_digit() || c == '.') || host.starts_with('[');
  let local = host == "localhost"
    || [".localhost", ".local", ".lan", ".home", ".internal"]
      .iter()
      .any(|suffix| host.ends_with(suffix));
  host.contains('.') && host.split('.').all(label_ok) && !ip && !local
}

#[cfg(test)]
mod tests {
  use super::*;

  fn shape(url: &str) -> Option<String> {
    link_for_report(url, false).map(|link| link.link)
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
      ("http://[::1]/a", None),
      ("file:///Users/sara/Movies/a.mp4", None),
      ("not a link", None),
    ] {
      assert_eq!(shape(url).as_deref(), expected, "{url}");
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
