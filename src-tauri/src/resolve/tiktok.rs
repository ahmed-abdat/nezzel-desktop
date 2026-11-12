use super::{http, Resolution, Verdict};
use reqwest::Url;

fn is_host(host: &str) -> bool {
  host == "tiktok.com" || host.ends_with(".tiktok.com")
}

fn post(url: &Url) -> Option<&str> {
  if !url.host_str().is_some_and(is_host) {
    return None;
  }
  let mut parts = url.path().trim_matches('/').split('/');
  let (user, kind, id) = (parts.next()?, parts.next()?, parts.next()?);
  (user.starts_with('@')
    && user.len() > 1
    && matches!(kind, "video" | "photo")
    && !id.is_empty()
    && id.bytes().all(|c| c.is_ascii_digit())
    && parts.next().is_none())
  .then_some(kind)
}

pub(super) fn short(raw: &str) -> Option<Url> {
  let url = http::https_url(raw).ok()?;
  let path = url.path().trim_matches('/');
  let code = match url.host_str()? {
    "vt.tiktok.com" | "vm.tiktok.com" => path,
    "tiktok.com" | "www.tiktok.com" => path.strip_prefix("t/")?,
    _ => return None,
  };
  (!code.is_empty()
    && code
      .bytes()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')))
  .then_some(url)
}

pub(crate) fn photo_target(raw: &str) -> bool {
  http::https_url(raw)
    .ok()
    .is_some_and(|url| post(&url) == Some("photo"))
}

pub(super) async fn read<T: http::Transport>(url: Url, transport: &T) -> Verdict {
  match http::inspect_with_host(transport, url, is_host).await {
    Ok(result) if (200..300).contains(&result.response.status) && result.url.path() == "/" => {
      Verdict::Verified(Resolution::Unsupported {
        reason: super::Reason::Removed,
      })
    }
    Ok(result) if (200..300).contains(&result.response.status) && post(&result.url).is_some() => {
      let mut url = result.url;
      url.set_query(None);
      Verdict::Verified(Resolution::Passthrough(url.into()))
    }
    _ => Verdict::Unknown,
  }
}
