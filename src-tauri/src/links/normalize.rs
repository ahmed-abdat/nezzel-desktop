//! Input links, before engine selection. No cookies or competitor implementation code.
use crate::{net, sites::host_is};
use reqwest::{Method, Url};
use std::{collections::HashSet, future::Future, time::Duration};

pub(crate) const MAX_WRAPPERS: usize = 2;
const MAX_HOPS: usize = 5;
const TIMEOUT: Duration = Duration::from_secs(5);
const CURL_UA: &str = "curl/7.88.1";

// These tables are also emitted by contract.rs for the synchronous pending-row key.
pub(crate) const ALIASES: &[(&str, &str)] = &[
  ("ddinstagram.com", "instagram.com"),
  ("instagramez.com", "instagram.com"),
  ("kkinstagram.com", "instagram.com"),
  ("fixvx.com", "x.com"),
  ("fxtwitter.com", "x.com"),
  ("vxtwitter.com", "x.com"),
  ("vxtiktok.com", "tiktok.com"),
  ("tnktok.com", "tiktok.com"),
  ("rxddit.com", "reddit.com"),
];
pub(crate) const TRACKERS: &[&str] = &["fbclid", "gclid", "igsh", "igshid", "mibextid", "ref_src"];
type ScopedTrackers = (&'static [&'static str], &'static [&'static str], bool);
pub(crate) const SCOPED_TRACKERS: &[ScopedTrackers] = &[
  (&["youtube.com", "youtu.be"], &["si", "feature"], false),
  (&["spotify.com"], &["si"], false),
  (&["x.com", "twitter.com"], &["s", "t"], true),
];
pub(crate) const INSTAGRAM_POST_PATHS: &[&str] = &["p", "reel", "tv"];
pub(crate) const YOUTUBE_VIDEO_PATHS: &[&str] = &["shorts", "embed"];
pub(crate) const SHORTENERS: &[&str] = &[
  "bit.ly",
  "tinyurl.com",
  "t.co",
  "lnkd.in",
  "ow.ly",
  "buff.ly",
  "is.gd",
  "v.gd",
  "rebrand.ly",
  "cutt.ly",
  "shorturl.at",
  "fb.watch",
  "youtu.be",
  "dai.ly",
  "redd.it",
  "b23.tv",
  "pin.it",
  "flic.kr",
  "on.soundcloud.com",
];
// host, exact path, ordered target keys, permit subdomains, permit relative target.
type WrapperRule = (
  &'static str,
  &'static str,
  &'static [&'static str],
  bool,
  bool,
);
pub(crate) const WRAPPERS: &[WrapperRule] = &[
  ("l.facebook.com", "/l.php", &["u"], false, false),
  ("lm.facebook.com", "/l.php", &["u"], false, false),
  ("l.instagram.com", "/", &["u"], false, false),
  ("l.messenger.com", "/l.php", &["u"], false, false),
  ("google.com", "/url", &["q", "url"], true, false),
  ("youtube.com", "/redirect", &["q"], true, false),
  ("youtube.com", "/attribution_link", &["u"], true, true),
  (
    "safelinks.protection.outlook.com",
    "/",
    &["url"],
    true,
    false,
  ),
  ("t.umblr.com", "/redirect", &["z"], false, false),
  ("href.li", "/", &[], false, false),
  ("out.reddit.com", "/", &["url"], false, false),
  ("linkedin.com", "/safety/go", &["url"], true, false),
  ("t.me", "/iv", &["url"], false, false),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
  Invalid,
  Unsafe,
  Loop,
  Limit,
  Network,
  Timeout,
  /// The short link leads to Flickr's sign-in page: the photo id does not exist.
  NotFound,
}

impl From<net::Refusal> for Error {
  fn from(refusal: net::Refusal) -> Self {
    match refusal {
      net::Refusal::Address => Self::Unsafe,
      net::Refusal::Timeout => Self::Timeout,
      net::Refusal::Lookup(_) => Self::Network,
    }
  }
}

// Engine URLs may be local or use custom ports. Normalization is not network admission.
fn validate(url: &Url) -> Result<(), Error> {
  if !matches!(url.scheme(), "http" | "https")
    || !url.username().is_empty()
    || url.password().is_some()
  {
    return Err(Error::Unsafe);
  }
  Ok(())
}

// Only requests made by this direct, cookie-free resolver require public-address admission.
fn validate_request(url: &Url) -> Result<(), Error> {
  validate(url)?;
  let host = net::host(url).ok_or(Error::Invalid)?.trim_end_matches('.');
  if url.port().is_some()
    || host == "localhost"
    || host.ends_with(".localhost")
    || host.ends_with(".local")
    || (!host.contains('.') && !host.contains(':'))
    || host.parse().is_ok_and(|ip| !net::public_address(ip))
  {
    return Err(Error::Unsafe);
  }
  Ok(())
}

fn private_destination(url: &Url) -> bool {
  let Some(host) = net::host(url) else {
    return true;
  };
  let host = host.trim_end_matches('.');
  host == "localhost"
    || host.ends_with(".localhost")
    || host.ends_with(".local")
    || (!host.contains('.') && !host.contains(':'))
    || host.parse().is_ok_and(|ip| !net::public_address(ip))
}

fn wrapper(url: &Url) -> Option<(&'static [&'static str], bool)> {
  let host = url.host_str()?;
  WRAPPERS
    .iter()
    .find(|(h, path, _, sub, _)| url.path() == *path && (host == *h || (*sub && host_is(host, h))))
    .map(|(_, _, keys, _, relative)| (*keys, *relative))
}

fn unwrap_once(url: &Url) -> Result<Option<Url>, Error> {
  let Some((keys, relative)) = wrapper(url) else {
    return Ok(None);
  };
  let target = if keys.is_empty() {
    let raw = url.query().ok_or(Error::Invalid)?;
    if raw.starts_with("http://") || raw.starts_with("https://") {
      raw.to_owned()
    } else {
      String::from_utf8(crate::files::names::percent_bytes(raw)).map_err(|_| Error::Invalid)?
    }
  } else {
    keys
      .iter()
      .find_map(|key| {
        url
          .query_pairs()
          .find(|(k, v)| k == key && !v.is_empty())
          .map(|(_, v)| v.into_owned())
      })
      .ok_or(Error::Invalid)?
  };
  let next = if relative && target.starts_with('/') && !target.starts_with("//") {
    Url::parse("https://www.youtube.com").and_then(|base| base.join(&target))
  } else {
    Url::parse(&target)
  }
  .map_err(|_| Error::Invalid)?;
  validate(&next)?;
  Ok(Some(next))
}

pub(crate) fn alias_host(host: &str) -> Option<&'static str> {
  let host = host.strip_prefix("www.").unwrap_or(host);
  ALIASES
    .iter()
    .find(|(alias, _)| *alias == host)
    .map(|(_, real)| *real)
}

fn tracking(key: &str, url: &Url) -> bool {
  let host = url.host_str().unwrap_or("");
  let key = key.to_ascii_lowercase();
  let x_post = url
    .path()
    .split('/')
    .collect::<Vec<_>>()
    .windows(2)
    .any(|p| p[0] == "status" && !p[1].is_empty() && p[1].bytes().all(|b| b.is_ascii_digit()));
  key.starts_with("utm_")
    || TRACKERS.contains(&key.as_str())
    || SCOPED_TRACKERS.iter().any(|(hosts, keys, post_only)| {
      hosts.iter().any(|h| host_is(host, h))
        && keys.contains(&key.as_str())
        && (!post_only || x_post)
    })
}

/// Pure routing URL. Retained query components keep their exact escaping and order.
pub(crate) fn pure(input: &str) -> Result<Url, Error> {
  let mut url = Url::parse(input).map_err(|_| Error::Invalid)?;
  validate(&url)?;
  if url.path() == "/" {
    if let Some(blog) = url
      .host_str()
      .and_then(|host| host.strip_suffix(".tumblr.com"))
      .filter(|blog| !matches!(*blog, "www" | "www2" | "t" | "assets") && !blog.contains('.'))
      .map(str::to_owned)
    {
      url
        .set_host(Some("www.tumblr.com"))
        .map_err(|_| Error::Invalid)?;
      url.set_path(&format!("/{blog}"));
    }
  }
  let mut seen = HashSet::from([url.to_string()]);
  for _ in 0..MAX_WRAPPERS {
    let Some(next) = unwrap_once(&url)? else {
      break;
    };
    if !seen.insert(next.to_string()) {
      return Err(Error::Loop);
    }
    url = next;
  }
  if wrapper(&url).is_some() {
    return Err(Error::Limit);
  }
  if let Some(host) = alias_host(url.host_str().unwrap_or("")) {
    url.set_host(Some(host)).map_err(|_| Error::Invalid)?;
  }
  let host = url.host_str().unwrap_or("").to_owned();
  let numeric = |id: &str| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
  if host == "platform.twitter.com" && url.path() == "/embed/Tweet.html" {
    if let Some(id) = url
      .query_pairs()
      .find(|(key, value)| key == "id" && numeric(value))
      .map(|(_, value)| value.into_owned())
    {
      url = Url::parse(&format!("https://x.com/i/status/{id}")).map_err(|_| Error::Invalid)?;
    }
  } else if host_is(&host, "tiktok.com") {
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let id = match parts.as_slice() {
      ["embed", "v2", id] | ["player", "v1", id] => Some(*id),
      ["v", id] => Some(id.strip_suffix(".html").unwrap_or(id)),
      _ => None,
    };
    if let Some(id) = id.filter(|id| numeric(id)) {
      let path = format!("/@_/video/{id}");
      url.set_path(&path);
    }
  }
  if let Some(query) = url.query() {
    let retained: Vec<_> = query
      .split('&')
      .filter(|pair| {
        let raw = pair.split('=').next().unwrap_or("").replace('+', " ");
        let key = String::from_utf8_lossy(&crate::files::names::percent_bytes(&raw)).into_owned();
        !tracking(&key, &url)
      })
      .collect();
    if retained.len() != query.split('&').count() {
      let query = retained.join("&");
      url.set_query((!query.is_empty()).then_some(&query));
    }
  }
  Ok(url)
}

/// Pending-row identity. Unlike routing, identity can equate post presentation variants.
pub(crate) fn canonical_key(input: &str) -> Result<String, Error> {
  let mut url = pure(input)?;
  let host = url.host_str().unwrap_or("").to_owned();
  let parts: Vec<_> = url
    .path()
    .strip_suffix('/')
    .unwrap_or(url.path())
    .split('/')
    .collect();
  if host_is(&host, "instagram.com")
    && parts.len() == 3
    && INSTAGRAM_POST_PATHS.contains(&parts[1])
    && !parts[2].is_empty()
  {
    let path = format!("/p/{}", parts[2]);
    url
      .set_host(Some("instagram.com"))
      .map_err(|_| Error::Invalid)?;
    url.set_path(&path);
    url.set_fragment(None);
  } else if host_is(&host, "youtube.com") || host == "youtu.be" {
    let video = if host == "youtu.be" {
      parts
        .get(1)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
    } else if parts.len() == 3 && YOUTUBE_VIDEO_PATHS.contains(&parts[1]) {
      Some(parts[2].to_string())
    } else if url.path() == "/watch" {
      url
        .query_pairs()
        .find(|(k, _)| k == "v")
        .map(|(_, v)| v.into_owned())
    } else {
      None
    };
    if let Some(video) = video {
      url
        .set_host(Some("youtube.com"))
        .map_err(|_| Error::Invalid)?;
      url.set_path("/watch");
      let mut pairs: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k != "v")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
      pairs.push(("v".into(), video));
      url.query_pairs_mut().clear().extend_pairs(pairs);
    }
  }
  let mut pairs: Vec<_> = url
    .query_pairs()
    .map(|(k, v)| (k.into_owned(), v.into_owned()))
    .collect();
  // URLSearchParams.sort is stable and compares UTF-16 code units.
  pairs.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
  if pairs.is_empty() {
    url.set_query(None);
  } else {
    url.query_pairs_mut().clear().extend_pairs(pairs);
  }
  Ok(url.into())
}

pub(crate) fn needs_redirect(url: &Url) -> bool {
  let host = url
    .host_str()
    .unwrap_or("")
    .strip_prefix("www.")
    .unwrap_or(url.host_str().unwrap_or(""));
  SHORTENERS.contains(&host)
    || (host_is(host, "instagram.com") && url.path().starts_with("/share/"))
    || (matches!(host, "threads.com" | "threads.net") && url.path().starts_with("/t/"))
    // flic.kr lands here; an unknown short id continues to the sign-in page.
    || (host == "flickr.com" && url.path() == "/photo.gne")
}

struct Hop {
  status: u16,
  location: Option<String>,
}

async fn request(url: Url, method: Method, ua: &'static str) -> Result<Hop, Error> {
  let host = net::host(&url).ok_or(Error::Invalid)?;
  let addresses = net::admit(
    host,
    url.port_or_known_default().ok_or(Error::Invalid)?,
    net::public_address,
  )
  .await
  .map_err(Error::from)?;
  let client = net::pinned_builder(host, &addresses)
    .timeout(TIMEOUT)
    .user_agent(ua)
    .build()
    .map_err(|_| Error::Network)?;
  let mut response = client
    .request(method.clone(), url.clone())
    .send()
    .await
    .map_err(|_| Error::Network)?;
  let mut location = response
    .headers()
    .get(reqwest::header::LOCATION)
    .map(|v| v.to_str().map(str::to_owned).map_err(|_| Error::Invalid))
    .transpose()?;
  let mut status = response.status().as_u16();
  if method == Method::GET && status == 200 && matches!(url.host_str(), Some("is.gd" | "v.gd")) {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
      if body.len() + chunk.len() > 64 * 1024 {
        return Err(Error::Network);
      }
      body.extend_from_slice(&chunk);
    }
    location = preview_target(&String::from_utf8_lossy(&body));
    if location.is_some() {
      status = 302;
    }
  }
  Ok(Hop { status, location })
}

fn preview_target(html: &str) -> Option<String> {
  crate::resolve::html::all_tags(html)?
    .into_iter()
    .find_map(|tag| {
      (tag.name == "a"
        && (tag.attrs.get("id").is_some_and(|id| id == "redirecturl")
          || tag
            .attrs
            .get("class")
            .is_some_and(|class| class.split_whitespace().any(|name| name == "biglink"))))
      .then(|| tag.attrs.get("href").cloned())
      .flatten()
    })
}

async fn resolve_with<F: Future<Output = Result<Hop, Error>>>(
  input: &str,
  mut fetch: impl FnMut(Url, Method, &'static str) -> F,
) -> Result<Url, Error> {
  // v.gd/is.gd serve an interstitial page that no engine can read: the unresolved link would
  // end as "unsupported", so retry once and report the network failure itself.
  let preview = Url::parse(input)
    .ok()
    .is_some_and(|url| matches!(url.host_str(), Some("is.gd" | "v.gd")));
  let mut result = resolve_hops(input, &mut fetch).await;
  if preview && matches!(result, Err(Error::Timeout | Error::Network)) {
    result = resolve_hops(input, &mut fetch).await;
  }
  match result {
    Err(Error::Timeout | Error::Network) if !preview => {
      Url::parse(input).map_err(|_| Error::Invalid)
    }
    result => result,
  }
}

async fn resolve_hops<F: Future<Output = Result<Hop, Error>>>(
  input: &str,
  mut fetch: impl FnMut(Url, Method, &'static str) -> F,
) -> Result<Url, Error> {
  let mut url = pure(input)?;
  if !needs_redirect(&url) {
    return Ok(url);
  }
  let mut instagram = false;
  let mut flickr = false;
  let mut seen = HashSet::from([url.to_string()]);
  for hops in 0..=MAX_HOPS {
    validate_request(&url)?;
    instagram |=
      host_is(url.host_str().unwrap_or(""), "instagram.com") && url.path().starts_with("/share/");
    flickr |= host_is(url.host_str().unwrap_or(""), "flickr.com") && url.path() == "/photo.gne";
    let ua = if instagram { CURL_UA } else { "Nezzel/1.0" };
    let response = tokio::time::timeout(TIMEOUT, async {
      let response = fetch(url.clone(), Method::HEAD, ua).await?;
      if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        Ok(response)
      } else {
        fetch(url.clone(), Method::GET, ua).await
      }
    })
    .await
    .map_err(|_| Error::Timeout)??;
    if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
      return if (200..300).contains(&response.status) {
        Ok(url)
      } else {
        Err(Error::Network)
      };
    }
    if hops == MAX_HOPS {
      return Err(Error::Limit);
    }
    let next = url
      .join(response.location.as_deref().ok_or(Error::Invalid)?)
      .map_err(|_| Error::Invalid)?;
    url = pure(next.as_str())?;
    if url.host_str() == Some("identity.flickr.com") {
      return Err(Error::NotFound);
    }
    if instagram && !host_is(url.host_str().unwrap_or(""), "instagram.com") {
      return Err(Error::Unsafe);
    }
    // photo.gne is Flickr's own resolver: it never legitimately leaves flickr.com.
    if flickr && !host_is(url.host_str().unwrap_or(""), "flickr.com") {
      return Err(Error::Unsafe);
    }
    if !seen.insert(url.to_string()) {
      return Err(Error::Loop);
    }
    // The destination's engine owns authentication/consent redirects and network policy,
    // but a public shortener never hands a private or local address to an engine.
    if !needs_redirect(&url) {
      return if private_destination(&url) {
        Err(Error::Unsafe)
      } else {
        Ok(url)
      };
    }
  }
  Err(Error::Limit)
}

/// Each hop has its own five-second DNS/HEAD/GET budget; engines own network fallbacks.
/// With a proxy, engines resolve their own shorteners; this resolver never bypasses the proxy.
pub(crate) async fn resolve(input: &str, proxy: bool) -> Result<Url, Error> {
  let url = pure(input)?;
  if proxy {
    return Ok(url);
  }
  resolve_with(input, request).await
}

/// The normalized clone is engine-only: the queue and history keep the submitted URL.
pub(crate) async fn fetch_entry(
  app: &tauri::AppHandle,
  entry: &crate::scheduling::fetch_pipeline::FetchEntry,
) -> Result<
  crate::scheduling::fetch_pipeline::FetchEntry,
  crate::runners::ytdlp_info::YtdlpInfoFetchError,
> {
  use crate::extract::{
    chain::StageResult,
    outcome::{Confidence, StageId},
    terminal,
  };
  use tauri::Manager;
  let proxy = app
    .state::<crate::SharedConfig>()
    .load()
    .network
    .enable_proxy
    == Some(true);
  if !crate::scheduling::group_state::is_group_running(&entry.group_id) {
    return Err(crate::runners::ytdlp_info::YtdlpInfoFetchError::NonZeroExit(1));
  }
  let mut normalized = entry.clone();
  let mut cancel = crate::scheduling::group_state::subscribe_group(&entry.group_id);
  let result = tokio::select! {
    result = resolve(&entry.url, proxy) => result,
    _ = cancel.changed() => return Err(crate::runners::ytdlp_info::YtdlpInfoFetchError::NonZeroExit(1)),
  };
  match result {
    Ok(url) => {
      normalized.url = url.into();
      Ok(normalized)
    }
    Err(error) => {
      let code = match error {
        Error::Timeout => "networkTimeout",
        Error::Network => "networkNameResolutionFailed",
        Error::NotFound => "notFound404",
        _ => "urlNotSupported",
      };
      let mut stage = StageResult::<()>::new(StageId::Ytdlp, Confidence::High);
      stage.record_code(code, "", false, std::time::Instant::now());
      terminal::publish_terminal(
        &stage.attempts,
        &entry.group_id,
        &entry.id,
        |code| code.to_owned(),
        |diagnostic, fatal| terminal::emit_terminal(app, diagnostic, fatal),
      );
      Err(crate::runners::ytdlp_info::YtdlpInfoFetchError::NonZeroExit(1))
    }
  }
}

#[cfg(test)]
mod tests {
  #[tokio::test]
  async fn threads_short_form_resolves_to_bound_post() {
    for host in ["www.threads.com", "www.threads.net"] {
      let url = resolve_with(&format!("https://{host}/t/Ab_c-12"), |_, _, _| async {
        Ok(Hop {
          status: 302,
          location: Some("https://www.threads.com/@nasa/post/Ab_c-12".into()),
        })
      })
      .await
      .unwrap();
      assert_eq!(url.as_str(), "https://www.threads.com/@nasa/post/Ab_c-12");
    }
  }

  #[test]
  fn embed_forms_normalize_before_routing() {
    for (source, expected) in [
      (
        "https://platform.twitter.com/embed/Tweet.html?id=12345",
        "https://x.com/i/status/12345",
      ),
      (
        "https://www.tiktok.com/embed/v2/12345",
        "https://www.tiktok.com/@_/video/12345",
      ),
      (
        "https://www.tiktok.com/player/v1/12345",
        "https://www.tiktok.com/@_/video/12345",
      ),
      (
        "https://www.tiktok.com/v/12345",
        "https://www.tiktok.com/@_/video/12345",
      ),
      (
        "https://www.tiktok.com/v/12345.html",
        "https://www.tiktok.com/@_/video/12345",
      ),
      (
        "https://www.tiktok.com/v/not-an-id",
        "https://www.tiktok.com/v/not-an-id",
      ),
      (
        "https://platform.twitter.com.evil.test/embed/Tweet.html?id=12345",
        "https://platform.twitter.com.evil.test/embed/Tweet.html?id=12345",
      ),
    ] {
      assert_eq!(pure(source).unwrap().as_str(), expected, "{source}");
    }
  }

  use super::*;
  use std::{cell::RefCell, future::ready};

  #[test]
  fn pure_normalization_and_pending_identity_share_the_webview_fixtures() {
    let cases: Vec<serde_json::Value> =
      serde_json::from_str(include_str!("fixtures.json")).unwrap();
    for case in cases {
      let input = case["input"].as_str().unwrap();
      if case["error"].as_bool() == Some(true) {
        assert!(pure(input).is_err(), "{input}");
      } else {
        assert_eq!(
          pure(input).unwrap().as_str(),
          case["url"].as_str().unwrap(),
          "{input}"
        );
        if let Some(key) = case["key"].as_str() {
          assert_eq!(canonical_key(input).unwrap(), key, "{input}");
        }
      }
    }
  }

  #[test]
  fn nested_wrappers_stop_at_two_and_preserve_arabic_percent_encoded_targets() {
    let wrap = |input: &str| {
      let mut u = Url::parse("https://google.com/url").unwrap();
      u.query_pairs_mut().append_pair("q", input);
      u.to_string()
    };
    let target = "https://example.com/مرحبا?id=1";
    assert_eq!(
      pure(&wrap(&wrap(target))).unwrap(),
      Url::parse(target).unwrap()
    );
    assert_eq!(pure(&wrap(&wrap(&wrap(target)))), Err(Error::Limit));
  }

  #[test]
  fn timestamps_playlists_and_access_keys_never_collapse() {
    for (a, b) in [
      ("https://youtu.be/a?t=10", "https://youtu.be/a?t=20"),
      (
        "https://youtube.com/watch?v=a#t=10",
        "https://youtube.com/watch?v=a#t=20",
      ),
      (
        "https://youtube.com/watch?v=a&list=one",
        "https://youtube.com/watch?v=a&list=two",
      ),
      (
        "https://youtube.com/watch?v=a&index=1",
        "https://youtube.com/watch?v=a&index=2",
      ),
      (
        "https://facebook.com/story.php?story_fbid=1&id=1",
        "https://facebook.com/story.php?story_fbid=2&id=1",
      ),
      (
        "https://dropbox.com/a?rlkey=one",
        "https://dropbox.com/a?rlkey=two",
      ),
    ] {
      assert_ne!(canonical_key(a).unwrap(), canonical_key(b).unwrap());
    }
  }

  #[tokio::test]
  async fn proxy_shorteners_pass_through_to_their_engine() {
    for input in [
      "https://youtu.be/abc?t=10",
      "https://redd.it/abc",
      "https://fb.watch/abc",
      "https://vm.tiktok.com/abc",
    ] {
      assert_eq!(resolve(input, true).await.unwrap().as_str(), input);
    }
  }

  #[tokio::test]
  async fn terminal_destinations_are_not_requested_or_redirected_to_login() {
    for target in [
      "https://www.youtube.com/watch?v=abc",
      "https://www.instagram.com/reel/AbC/",
      "https://facebook.com/nasa/posts/123",
      "https://x.com/u/status/123",
    ] {
      let calls = RefCell::new(Vec::new());
      let result = resolve_with("https://bit.ly/a", |url, _, _| {
        calls.borrow_mut().push(url.to_string());
        ready(Ok(if url.host_str() == Some("bit.ly") {
          Hop {
            status: 302,
            location: Some(target.into()),
          }
        } else if url.path() != "/accounts/login" {
          Hop {
            status: 302,
            location: Some("/accounts/login".into()),
          }
        } else {
          Hop {
            status: 200,
            location: None,
          }
        }))
      })
      .await
      .unwrap();
      assert_eq!(result.as_str(), target);
      assert_eq!(calls.borrow().as_slice(), ["https://bit.ly/a"]);
    }
  }

  #[tokio::test]
  async fn local_and_port_urls_reach_engine_routing_without_resolver_io() {
    for input in [
      "http://nas.local/v.mp4",
      "http://192.168.1.5:8080/x.mp4",
      "http://nas/v.mp4",
      "http://[::1]:8080/v.mp4",
      "https://example.com:8443/x.mp4",
    ] {
      let mut calls = 0;
      let url = resolve_with(input, |_, _, _| {
        calls += 1;
        ready(Err(Error::Network))
      })
      .await
      .unwrap();
      assert_eq!(calls, 0, "normalizer must not request an engine URL");
      assert_eq!(url.as_str(), input);
      assert_eq!(validate_request(&url), Err(Error::Unsafe));
      assert_eq!(
        crate::extract::registry::stages_for(url.as_str()).first(),
        Some(&crate::routing::StageId::Files)
      );
    }
  }

  #[test]
  fn admission_errors_keep_network_failures_distinct_from_unsafe_addresses() {
    assert_eq!(Error::from(net::Refusal::Address), Error::Unsafe);
    assert_eq!(Error::from(net::Refusal::Timeout), Error::Timeout);
    assert_eq!(
      Error::from(net::Refusal::Lookup(std::io::ErrorKind::NotFound)),
      Error::Network
    );
  }

  #[tokio::test]
  async fn redirects_use_get_fallback_and_route_the_final_url() {
    let calls = RefCell::new(Vec::new());
    let url = resolve_with("https://bit.ly/abc", |url, method, _| {
      calls.borrow_mut().push((url.to_string(), method.clone()));
      ready(Ok(if url.host_str() == Some("bit.ly") {
        if method == Method::HEAD {
          Hop {
            status: 405,
            location: None,
          }
        } else {
          Hop {
            status: 302,
            location: Some("https://ddinstagram.com/p/AbC?igsh=x".into()),
          }
        }
      } else {
        Hop {
          status: 200,
          location: None,
        }
      }))
    })
    .await
    .unwrap();
    assert_eq!(url.as_str(), "https://instagram.com/p/AbC");
    assert_eq!(
      canonical_key(url.as_str()).unwrap(),
      canonical_key("https://www.instagram.com/reel/AbC/").unwrap()
    );
    assert_eq!(calls.borrow()[1].1, Method::GET);
    assert!(crate::extract::registry::stages_for(url.as_str())
      .contains(&crate::routing::StageId::Gallery));
  }

  #[tokio::test]
  async fn a_short_link_to_instagram_share_switches_user_agent_before_following() {
    let url = resolve_with("https://t.co/a", |url, _, ua| {
      let response = match url.host_str().unwrap() {
        "t.co" => Hop {
          status: 302,
          location: Some("https://www.instagram.com/share/a".into()),
        },
        _ if url.path().starts_with("/share/") => {
          assert_eq!(ua, CURL_UA);
          Hop {
            status: 302,
            location: Some("/reel/AbC/".into()),
          }
        }
        _ => Hop {
          status: 403,
          location: None,
        },
      };
      ready(Ok(response))
    })
    .await
    .unwrap();
    assert_eq!(url.as_str(), "https://www.instagram.com/reel/AbC/");
  }

  #[tokio::test]
  async fn redirect_loops_limits_and_unsafe_targets_stop_before_a_request() {
    for target in [
      "https://bit.ly/a",
      "file:///etc/passwd",
      "https://user:pass@example.com/",
      "http://127.0.0.1/",
      "http://[::1]:8080/x",
      "http://192.168.1.5/v.mp4",
      "http://nas.local/v.mp4",
    ] {
      let mut calls = 0;
      assert!(resolve_with("https://bit.ly/a", |_, _, _| {
        calls += 1;
        ready(Ok(Hop {
          status: 302,
          location: Some(target.into()),
        }))
      })
      .await
      .is_err());
      assert_eq!(calls, 1, "{target}");
    }
    let mut calls = 0;
    assert_eq!(
      resolve_with("https://bit.ly/a", |_, _, _| {
        calls += 1;
        ready(Ok(Hop {
          status: 302,
          location: Some(format!("/{calls}")),
        }))
      })
      .await,
      Err(Error::Limit)
    );
    assert_eq!(calls, 6);
  }

  #[tokio::test]
  async fn instagram_share_uses_curl_and_rejects_off_site_hops() {
    for target in ["/reel/AbC/", "https://example.com/reel/AbC/"] {
      let result = resolve_with("https://www.instagram.com/share/reel/abc", |url, _, ua| {
        assert_eq!(ua, CURL_UA);
        ready(Ok(if url.path().starts_with("/share/") {
          Hop {
            status: 302,
            location: Some(target.into()),
          }
        } else {
          Hop {
            status: 200,
            location: None,
          }
        }))
      })
      .await;
      if target.starts_with('/') {
        assert_eq!(result.unwrap().path(), "/reel/AbC/");
      } else {
        assert_eq!(result, Err(Error::Unsafe));
      }
    }
  }

  #[tokio::test(start_paused = true)]
  async fn each_hop_has_a_budget_and_network_failure_preserves_original() {
    let original = "https://flic.kr/p/example?utm_source=share";
    assert_eq!(
      resolve_with(
        original,
        |_, _, _| std::future::pending::<Result<Hop, Error>>()
      )
      .await
      .unwrap()
      .as_str(),
      original
    );
    assert_eq!(
      resolve_with(original, |_, _, _| ready(Err(Error::Network)))
        .await
        .unwrap()
        .as_str(),
      original
    );
    let mut calls = 0;
    let result = resolve_with("https://v.gd/first", |_, _, _| {
      calls += 1;
      let target = if calls == 1 {
        "https://is.gd/second"
      } else {
        "https://example.com/video"
      };
      async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        Ok(Hop {
          status: 302,
          location: Some(target.into()),
        })
      }
    })
    .await
    .unwrap();
    assert_eq!(result.as_str(), "https://example.com/video");
    assert_eq!(calls, 2);
    for short in [
      "https://vt.tiktok.com/abc/",
      "https://vm.tiktok.com/abc/",
      "https://www.tiktok.com/t/abc/",
    ] {
      assert_eq!(
        resolve_with(short, |_, _, _| async {
          panic!("TikTok reader owns redirects")
        })
        .await
        .unwrap()
        .as_str(),
        short
      );
    }
    // Real v.gd preview page (curl, 2026-11-18): XHTML with the link as `class="biglink"`.
    let recorded = include_str!("fixtures/vgd-preview-LfQhkN.html");
    assert_eq!(
      preview_target(recorded).as_deref(),
      Some("https://www.instagram.com/p/DfxRiPZIXJo/")
    );
    let recorded_hop = Hop {
      status: 302,
      location: preview_target(recorded),
    };
    let mut calls = 0;
    let retried = resolve_with("https://v.gd/LfQhkN", |_, _, _| {
      calls += 1;
      ready(if calls == 1 {
        Err(Error::Network)
      } else {
        Ok(Hop {
          status: recorded_hop.status,
          location: recorded_hop.location.clone(),
        })
      })
    })
    .await
    .unwrap();
    assert_eq!(retried.as_str(), "https://www.instagram.com/p/DfxRiPZIXJo/");
    // A preview shortener that stays unreachable is a network error, not an unsupported link.
    assert!(matches!(
      resolve_with("https://v.gd/LfQhkN", |_, _, _| ready(Err(Error::Network))).await,
      Err(Error::Network)
    ));
    let target = preview_target(
      r#"<a id="redirecturl" href="https://www.instagram.com/p/DfxRiPZIXJo/">Continue</a>"#,
    )
    .unwrap();
    assert_eq!(
      preview_target(
        r#"<p>Preview</p><a href="https://v.gd/stats">x</a><a href="https://www.instagram.com/p/DfxRiPZIXJo/" class="biglink">https://www.instagram.com/p/DfxRiPZIXJo/</a>"#
      )
      .as_deref(),
      Some("https://www.instagram.com/p/DfxRiPZIXJo/")
    );
    assert_eq!(
      resolve_with("https://v.gd/LfQhkN", |_, _, _| ready(Ok(Hop {
        status: 302,
        location: Some(target.clone())
      })))
      .await
      .unwrap()
      .as_str(),
      target
    );
    let missing = resolve_with("https://flic.kr/p/2q1x1x1", |url, _, _| {
      let location = match url.host_str() {
        Some("flic.kr") => "https://www.flickr.com/photo.gne?short=2q1x1x1",
        _ => "https://identity.flickr.com/login?redir=%2Fphoto.gne",
      };
      ready(Ok(Hop {
        status: 302,
        location: Some(location.into()),
      }))
    })
    .await;
    assert!(matches!(missing, Err(Error::NotFound)));
    let offsite = resolve_with("https://flic.kr/p/2q1x1x1", |url, _, _| {
      let location = match url.host_str() {
        Some("flic.kr") => "https://www.flickr.com/photo.gne?short=2q1x1x1",
        _ => "https://evil.example/landing",
      };
      ready(Ok(Hop {
        status: 302,
        location: Some(location.into()),
      }))
    })
    .await;
    assert!(matches!(offsite, Err(Error::Unsafe)));
    assert_eq!(
      resolve("https://bit.ly/a", true).await.unwrap().as_str(),
      "https://bit.ly/a"
    );
  }
}
