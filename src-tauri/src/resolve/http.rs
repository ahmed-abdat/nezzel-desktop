use crate::media_types::{base_mime, is_manifest_mime};
use crate::net;
use reqwest::Url;
use std::collections::HashSet;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

pub(crate) const FACEBOOK_CAP: usize = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FetchError {
  InvalidUrl,
  Address,
  Network,
  Redirect,
  BodyLimit,
  Encoding,
  Status,
}

#[derive(Clone)]
pub(crate) struct Response {
  pub status: u16,
  pub location: Option<String>,
  pub content_type: Option<String>,
  pub body: Vec<u8>,
  pub content_length: Option<u64>,
}
pub(crate) struct Page {
  pub url: Url,
  pub body: Vec<u8>,
  pub media: bool,
  pub content_type: Option<String>,
}
pub(crate) trait Transport {
  /// Distinguishes header/client variants of the same transport type within a row.
  fn cache_key(&self) -> &str {
    std::any::type_name::<Self>()
  }
  fn get(&self, url: &Url, cap: usize)
    -> impl Future<Output = Result<Response, FetchError>> + Send;
  fn head(&self, _url: &Url) -> impl Future<Output = Result<Response, FetchError>> + Send {
    async { Err(FetchError::Status) }
  }
  fn range(
    &self,
    url: &Url,
    cap: usize,
  ) -> impl Future<Output = Result<Response, FetchError>> + Send {
    self.get(url, cap)
  }
  fn admit_media(&self, url: &Url) -> impl Future<Output = Result<(), FetchError>> + Send {
    async move { addresses(url).await.map(|_| ()) }
  }
}
pub(crate) struct PublicHttp;
impl Transport for PublicHttp {
  async fn get(&self, url: &Url, cap: usize) -> Result<Response, FetchError> {
    self.request(url, cap, RequestKind::Page).await
  }
  async fn head(&self, url: &Url) -> Result<Response, FetchError> {
    self.request(url, 0, RequestKind::Head).await
  }
  async fn range(&self, url: &Url, cap: usize) -> Result<Response, FetchError> {
    self.request(url, cap, RequestKind::Range).await
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestKind {
  Page,
  Head,
  Range,
}
impl PublicHttp {
  async fn request(
    &self,
    url: &Url,
    cap: usize,
    kind: RequestKind,
  ) -> Result<Response, FetchError> {
    https_url(url.as_str())?;
    let addresses = addresses(url).await?;
    let client = pinned_client(url, &addresses)?;
    let method = if kind == RequestKind::Head {
      reqwest::Method::HEAD
    } else {
      reqwest::Method::GET
    };
    let mut request = client.request(method, url.clone());
    if kind == RequestKind::Range {
      request = request.header("Range", format!("bytes=0-{}", cap.saturating_sub(1)));
    }
    let mut response = request
      .header("User-Agent", net::CHROME_UA)
      .header(
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
      )
      .header("Accept-Encoding", "identity")
      .header("Accept-Language", "en-US,en;q=0.9")
      .header("Sec-Fetch-Mode", "navigate")
      .header("Sec-Fetch-Site", "none")
      .header("Sec-Fetch-Dest", "document")
      .header("Sec-Fetch-User", "?1")
      .header("Upgrade-Insecure-Requests", "1")
      .send()
      .await
      .map_err(|_| FetchError::Network)?;
    let status = response.status().as_u16();
    let location = response
      .headers()
      .get("location")
      .and_then(|h| h.to_str().ok())
      .map(str::to_owned);
    let content_type = response
      .headers()
      .get("content-type")
      .and_then(|h| h.to_str().ok())
      .map(str::to_owned);
    let mut result = Response {
      status,
      location,
      content_type,
      content_length: response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse().ok())
        .or_else(|| response.content_length()),
      body: Vec::new(),
    };
    // Drop redirects/errors unread; they never contribute data or forward cookies.
    if !(200..300).contains(&status)
      || kind == RequestKind::Head
      || (is_media(result.content_type.as_deref())
        && !manifest_response(url, result.content_type.as_deref())
        && (kind == RequestKind::Page || result.content_length.is_some_and(|n| n > 0)))
    {
      return Ok(result);
    }
    // Android also requests identity and rejects encoded bodies. This bounds *uncompressed* bytes,
    // even when another crate enables reqwest's optional decompression features globally.
    if response
      .headers()
      .get_all("content-encoding")
      .iter()
      .any(|h| {
        h.to_str()
          .map_or(true, |v| !v.trim().eq_ignore_ascii_case("identity"))
      })
    {
      return Err(FetchError::Encoding);
    }
    if response.content_length().is_some_and(|n| n > cap as u64) {
      return Err(FetchError::BodyLimit);
    }
    while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Network)? {
      if kind == RequestKind::Range
        && is_media(result.content_type.as_deref())
        && !manifest_response(url, result.content_type.as_deref())
      {
        append(&mut result.body, &chunk[..chunk.len().min(cap)], cap)?;
        break;
      }
      append(&mut result.body, &chunk, cap)?;
    }
    Ok(result)
  }
}

pub(crate) fn append(body: &mut Vec<u8>, chunk: &[u8], cap: usize) -> Result<(), FetchError> {
  if chunk.len() > cap.saturating_sub(body.len()) {
    return Err(FetchError::BodyLimit);
  }
  body.extend_from_slice(chunk);
  Ok(())
}

/// Page reads go to port 443 only and only to public addresses.
pub(crate) async fn addresses(url: &Url) -> Result<Vec<SocketAddr>, FetchError> {
  let host = net::host(url).ok_or(FetchError::InvalidUrl)?;
  net::admit(host, 443, net::public_address)
    .await
    .map_err(|refusal| match refusal {
      net::Refusal::Address => FetchError::Address,
      net::Refusal::Timeout | net::Refusal::Lookup(_) => FetchError::Network,
    })
}

/// Browser-navigation timeouts on top of the shared pinned builder; HTTPS only.
fn pinned_client(url: &Url, addresses: &[SocketAddr]) -> Result<reqwest::Client, FetchError> {
  net::pinned_builder(url.host_str().ok_or(FetchError::InvalidUrl)?, addresses)
    .https_only(true)
    .connect_timeout(Duration::from_secs(5))
    .read_timeout(Duration::from_secs(5))
    .timeout(Duration::from_secs(10))
    .build()
    .map_err(|_| FetchError::Network)
}

pub(crate) fn is_media(mime: Option<&str>) -> bool {
  mime.is_some_and(|m| {
    let m = m.to_ascii_lowercase();
    m.starts_with("video/")
      || m.starts_with("audio/")
      || m.starts_with("image/")
      || m.split(';').next().is_some_and(|m| {
        let m = m.trim();
        is_manifest_mime(m) || matches!(m, "application/pdf" | "application/octet-stream")
      })
  })
}

pub(crate) fn is_manifest(mime: Option<&str>) -> bool {
  mime.is_some_and(|m| {
    let m = base_mime(m);
    is_manifest_mime(&m) || matches!(m.as_str(), "audio/mpegurl" | "audio/x-mpegurl")
  })
}

pub(crate) fn is_octet_stream(mime: Option<&str>) -> bool {
  mime.is_some_and(|mime| {
    mime
      .split(';')
      .next()
      .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/octet-stream"))
  })
}

/// Some CDNs serve manifests as binary data; their suffix still requires a bounded DRM read.
pub(crate) fn manifest_response(url: &Url, mime: Option<&str>) -> bool {
  is_manifest(mime)
    || (is_octet_stream(mime)
      && [".m3u8", ".mpd"]
        .iter()
        .any(|suffix| url.path().to_ascii_lowercase().ends_with(suffix)))
}

pub(crate) struct Inspection {
  pub url: Url,
  pub response: Response,
}

/// HEAD first; GET only a bounded prefix when HEAD is inconclusive or a manifest needs reading.
/// Every redirect is re-admitted, including redirects from HEAD and Range independently.
pub(crate) async fn inspect<T: Transport>(
  transport: &T,
  url: Url,
) -> Result<Inspection, FetchError> {
  let head = inspect_request(transport, url.clone(), false).await;
  match head {
    Ok(result)
      if (200..300).contains(&result.response.status)
        && result.response.content_length.is_some()
        && result.response.content_type.is_some()
        && !manifest_response(&result.url, result.response.content_type.as_deref()) =>
    {
      Ok(result)
    }
    Ok(result) if matches!(result.response.status, 401 | 403 | 404 | 410 | 429) => {
      Err(FetchError::Status)
    }
    Ok(result) => inspect_request(transport, result.url, true).await,
    Err(FetchError::Status) => inspect_request(transport, url, true).await,
    Err(error) => Err(error),
  }
}

async fn inspect_request<T: Transport>(
  transport: &T,
  mut url: Url,
  range: bool,
) -> Result<Inspection, FetchError> {
  let mut seen = HashSet::new();
  for hop in 0..=3 {
    https_url(url.as_str())?;
    transport.admit_media(&url).await?;
    if !seen.insert(url.clone()) {
      return Err(FetchError::Redirect);
    }
    let response = if range {
      transport.range(&url, 1024 * 1024).await?
    } else {
      transport.head(&url).await?
    };
    if [301, 302, 303, 307, 308].contains(&response.status) {
      if hop == 3 {
        return Err(FetchError::Redirect);
      }
      url = https_url(
        url
          .join(response.location.as_deref().ok_or(FetchError::Redirect)?)
          .map_err(|_| FetchError::InvalidUrl)?
          .as_str(),
      )?;
      continue;
    }
    if response.body.len() > 1024 * 1024 {
      return Err(FetchError::BodyLimit);
    }
    return Ok(Inspection { url, response });
  }
  Err(FetchError::Redirect)
}

pub(crate) async fn fetch<T: Transport>(
  transport: &T,
  mut url: Url,
  facebook: bool,
) -> Result<Page, FetchError> {
  let cap = if facebook { FACEBOOK_CAP } else { 1024 * 1024 };
  let mut seen = HashSet::new();
  let mut unwrapped = false;
  let short = facebook && super::facebook::Identity::parse(url.as_str()).is_some_and(|i| i.short);
  for hop in 0..=3 {
    https_url(url.as_str())?;
    if !seen.insert(url.as_str().to_owned()) {
      return Err(FetchError::Redirect);
    }
    let response = transport.get(&url, cap).await?;
    if [301, 302, 303, 307, 308].contains(&response.status) {
      if hop == 3 {
        return Err(FetchError::Redirect);
      }
      let location = response.location.ok_or(FetchError::Redirect)?;
      let next = url.join(&location).map_err(|_| FetchError::InvalidUrl)?;
      let mut next = https_url(next.as_str())?;
      if facebook {
        if super::facebook::is_wall(&next) {
          // A login wrapper may hold the public post in next=. Never request the wall itself.
          let targets: Vec<_> = next
            .query_pairs()
            .filter(|(k, _)| k == "next")
            .map(|(_, v)| v.into_owned())
            .collect();
          let identity =
            (super::facebook::is_login_wall(&next) && !unwrapped && targets.len() == 1)
              .then(|| super::facebook::Identity::parse(&targets[0]))
              .flatten()
              .filter(|i| !i.short)
              .ok_or(FetchError::Redirect)?;
          next = identity.url;
          unwrapped = true;
        }
        let mut identity =
          super::facebook::Identity::parse(next.as_str()).ok_or(FetchError::Redirect)?;
        if identity.short
          && !(url.host_str() != Some("web.facebook.com")
            && next.host_str() == Some("web.facebook.com")
            && url.path().trim_end_matches('/') == next.path().trim_end_matches('/'))
        {
          return Err(FetchError::Redirect);
        }
        if next.host_str() == Some("web.facebook.com") {
          identity
            .url
            .set_host(Some("web.facebook.com"))
            .map_err(|_| FetchError::InvalidUrl)?;
        }
        next = identity.url;
        // A shortlink's public video hop names the target. Its page may be login-limited;
        // do not read it (nor trust a video id reached through a login wrapper).
        if short && !unwrapped && identity.video_id.is_some() {
          return Ok(Page {
            url: next,
            media: false,
            content_type: None,
            body: Vec::new(),
          });
        }
      }
      url = next;
      continue;
    }
    if response.status != 200 {
      return Err(FetchError::Status);
    }
    if response.body.len() > cap {
      return Err(FetchError::BodyLimit);
    }
    if facebook && super::facebook::Identity::parse(url.as_str()).is_none_or(|i| i.short) {
      return Err(FetchError::Redirect);
    }
    return Ok(Page {
      url,
      media: is_media(response.content_type.as_deref()),
      content_type: response.content_type,
      body: response.body,
    });
  }
  Err(FetchError::Redirect)
}

pub(crate) fn https_url(input: &str) -> Result<Url, FetchError> {
  let url = Url::parse(input).map_err(|_| FetchError::InvalidUrl)?;
  if url.scheme() != "https"
    || url.host_str().is_none()
    || url.port_or_known_default() != Some(443)
    || !url.username().is_empty()
    || url.password().is_some()
    || url.fragment().is_some()
  {
    return Err(FetchError::InvalidUrl);
  }
  Ok(url)
}

#[cfg(test)]
mod tests {
  use super::*;

  // Loopback, private v4/v6, link-local and v4-mapped v6 are refused before any connect.
  #[tokio::test]
  async fn page_reads_refuse_non_public_literals_and_dns_answers() {
    for url in [
      "https://127.0.0.1/",
      "https://[::1]/",
      "https://localhost/",
      "https://10.0.0.1/",
      "https://192.168.1.1/",
      "https://[fc00::1]/",
      "https://169.254.169.254/",
      "https://[fe80::1]/",
      "https://[::ffff:127.0.0.1]/",
      "https://[::ffff:10.0.0.1]/",
    ] {
      let url = Url::parse(url).unwrap();
      assert_eq!(addresses(&url).await, Err(FetchError::Address), "{url}");
      assert_eq!(
        PublicHttp.admit_media(&url).await,
        Err(FetchError::Address),
        "{url}"
      );
      assert!(
        matches!(PublicHttp.get(&url, 1024).await, Err(FetchError::Address)),
        "{url}"
      );
    }
    for url in [
      "https://8.8.8.8/",
      "https://[2606:4700:4700::1111]/",
      "https://[::ffff:8.8.8.8]/",
    ] {
      let url = Url::parse(url).unwrap();
      let ports: Vec<_> = addresses(&url)
        .await
        .unwrap()
        .iter()
        .map(|a| a.port())
        .collect();
      assert_eq!(ports, [443], "{url}");
    }
  }

  // DNS rebinding: the client connects to the admitted answer and never looks the host up again
  // (`.invalid` never resolves, so only the pinned address can reach the listener).
  #[tokio::test]
  async fn page_client_connects_only_to_the_admitted_answer() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pinned = listener.local_addr().unwrap();
    let url = Url::parse(&format!("https://rebind.invalid:{}/", pinned.port())).unwrap();
    let client = pinned_client(&url, &[pinned]).unwrap();
    let request = client.get(url.clone()).send();
    tokio::pin!(request);
    let mut socket = tokio::select! {
      accepted = listener.accept() => accepted.unwrap().0,
      result = &mut request => panic!("the pinned address was never reached: {result:?}"),
    };
    // Read the first byte, then hang up so the request ends.
    let first = async move {
      let mut first = [0];
      socket.read_exact(&mut first).await.unwrap();
      first[0]
    };
    let (first, result) = tokio::join!(first, &mut request);
    assert_eq!(first, 0x16, "a TLS handshake reached the pinned address");
    assert!(result.is_err());
  }
}
