use super::http;

pub(super) const MATCHER: crate::extract::registry::Matcher =
  crate::extract::registry::Matcher::new(
    crate::extract::outcome::StageId::Ytdlp,
    "vimeo-player",
    30,
    crate::extract::outcome::Confidence::High,
    crate::extract::registry::Pattern::Identity(|raw| player_url(raw).is_some()),
  );

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
