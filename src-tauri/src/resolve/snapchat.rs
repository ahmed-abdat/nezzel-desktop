use super::{html, http, Resolution, Verdict};
use reqwest::Url;
use std::collections::BTreeSet;

pub(super) const MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Snapchat,
  "snapchat-spotlight",
  20,
  |raw| spotlight(raw).is_some_and(|(_, _, canonical)| !canonical),
);

pub(super) fn is_host(host: &str) -> bool {
  matches!(host, "snapchat.com" | "www.snapchat.com")
}

fn spotlight(input: &str) -> Option<(Url, String, bool)> {
  let url = http::https_url(input).ok()?;
  if !is_host(url.host_str()?) {
    return None;
  }
  let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
  let (id, canonical) = match parts.as_slice() {
    ["spotlight", id] => (*id, false),
    [user, "spotlight", id]
      if user.strip_prefix('@').is_some_and(|u| {
        !u.is_empty()
          && u.len() <= 64
          && u
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
      }) =>
    {
      (*id, true)
    }
    _ => return None,
  };
  if id.is_empty()
    || id.len() > 256
    || !id
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
  {
    return None;
  }
  Some((url.clone(), id.into(), canonical))
}

pub(super) async fn read<T: http::Transport>(input: &str, transport: &T) -> Verdict {
  let Some((url, id, false)) = spotlight(input) else {
    return Verdict::NoMatch;
  };
  let Ok(page) = http::fetch(transport, url, false).await else {
    return Verdict::Unknown;
  };
  let Some((_, page_id, canonical)) = spotlight(page.url.as_str()) else {
    return Verdict::Unknown;
  };
  if page_id != id {
    return Verdict::Unknown;
  }
  if canonical {
    return Verdict::Verified(Resolution::Passthrough(page.url.into()));
  }
  let Some(tags) = std::str::from_utf8(&page.body).ok().and_then(html::tags) else {
    return Verdict::Unknown;
  };
  // The bare page is a new app shell. Its og:url names the older user-scoped page
  // that yt-dlp can extract. Bind it to the requested ID, never a recommendation.
  let urls: BTreeSet<_> = html::metadata(&tags, "og:url")
    .into_iter()
    .filter_map(spotlight)
    .filter(|(_, found_id, canonical)| *canonical && *found_id == id)
    .map(|(url, _, _)| url.to_string())
    .collect();
  if urls.len() == 1 {
    Verdict::Verified(Resolution::Passthrough(urls.into_iter().next().unwrap()))
  } else {
    Verdict::Unknown
  }
}
