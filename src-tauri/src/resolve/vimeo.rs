use super::http;

pub(super) const MATCHER: crate::routing::Matcher = crate::routing::Matcher::new(
  crate::routing::StageId::Ytdlp,
  "vimeo-player",
  30,
  crate::routing::Confidence::High,
  crate::routing::Pattern::Identity(|raw| player_url(raw).is_some()),
);

/// Vimeo's anonymous web client can mislabel an account gate as DRM, so the fetch chain treats
/// anonymous yt-dlp DRM on Vimeo as a login cause. Real page DRM still stops.
pub(crate) fn drm_may_be_login(url: &str) -> bool {
  reqwest::Url::parse(url).ok().is_some_and(|url| {
    url
      .host_str()
      .is_some_and(|host| host == "vimeo.com" || host.ends_with(".vimeo.com"))
  })
}

const REFERER: &str = "https://vimeo.com/";

pub(super) fn player_url(input: &str) -> Option<String> {
  let url = http::https_url(input).ok()?;
  let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
  let (id, path_hash) = match (url.host_str()?, parts.as_slice()) {
    ("vimeo.com" | "www.vimeo.com", [id]) => (*id, None),
    ("vimeo.com" | "www.vimeo.com", [id, hash]) => (*id, Some(*hash)),
    ("player.vimeo.com", ["video", id]) => (*id, None),
    _ => return None,
  };
  if id.is_empty() || id.len() > 25 || !id.bytes().all(|b| b.is_ascii_digit()) {
    return None;
  }
  let hashes: Vec<_> = url.query_pairs().filter(|(key, _)| key == "h").collect();
  if hashes.len() > 1 {
    return None;
  }
  let query_hash = hashes.first().map(|(_, value)| value.as_ref());
  if path_hash.is_some() && query_hash.is_some() && path_hash != query_hash {
    return None;
  }
  let mut player = reqwest::Url::parse(&format!("https://player.vimeo.com/video/{id}")).ok()?;
  if let Some(hash) = path_hash.or(query_hash) {
    if hash.is_empty() || hash.len() > 128 || !hash.bytes().all(|b| b.is_ascii_alphanumeric()) {
      return None;
    }
    player.query_pairs_mut().append_pair("h", hash);
  }
  Some(player.into())
}

pub(super) fn referer(input: &str) -> Option<&'static str> {
  let url = http::https_url(input).ok()?;
  (url.host_str() == Some("player.vimeo.com") && player_url(input).is_some()).then_some(REFERER)
}
