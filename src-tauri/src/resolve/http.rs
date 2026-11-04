use reqwest::Url;
use std::collections::HashSet;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
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
    let client = reqwest::Client::builder()
      .https_only(true)
      .no_proxy()
      .redirect(reqwest::redirect::Policy::none())
      .referer(false)
      .retry(reqwest::retry::never())
      .no_gzip()
      .no_brotli()
      .no_deflate()
      .no_zstd()
      .http1_only()
      .min_tls_version(reqwest::tls::Version::TLS_1_2)
      .connect_timeout(Duration::from_secs(5))
      .read_timeout(Duration::from_secs(5))
      .timeout(Duration::from_secs(10))
      // Pin precisely the checked DNS answer: reqwest must never resolve this host again.
      .resolve_to_addrs(url.host_str().ok_or(FetchError::InvalidUrl)?, &addresses)
      .build()
      .map_err(|_| FetchError::Network)?;
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
      .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36")
      .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
      .header("Accept-Encoding", "identity").header("Accept-Language", "en-US,en;q=0.9")
      .header("Sec-Fetch-Mode", "navigate").header("Sec-Fetch-Site", "none")
      .header("Sec-Fetch-Dest", "document").header("Sec-Fetch-User", "?1")
      .header("Upgrade-Insecure-Requests", "1")
      .send().await.map_err(|_| FetchError::Network)?;
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

pub(crate) async fn addresses(url: &Url) -> Result<Vec<SocketAddr>, FetchError> {
  let host = url
    .host_str()
    .ok_or(FetchError::InvalidUrl)?
    .trim_matches(['[', ']']);
  let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
    vec![SocketAddr::new(ip, 443)]
  } else {
    tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 443)))
      .await
      .map_err(|_| FetchError::Network)?
      .map_err(|_| FetchError::Network)?
      .collect()
  };
  admit_addresses(addresses)
}

pub(crate) fn admit_addresses(addresses: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, FetchError> {
  if addresses.is_empty()
    || addresses
      .iter()
      .any(|a| a.port() != 443 || !public_address(a.ip()))
  {
    return Err(FetchError::Address);
  }
  Ok(addresses)
}

pub(crate) fn is_media(mime: Option<&str>) -> bool {
  mime.is_some_and(|m| {
    let m = m.to_ascii_lowercase();
    m.starts_with("video/")
      || m.starts_with("audio/")
      || m.starts_with("image/")
      || m.split(';').next().is_some_and(|m| {
        matches!(
          m.trim(),
          "application/vnd.apple.mpegurl"
            | "application/x-mpegurl"
            | "application/dash+xml"
            | "application/pdf"
            | "application/octet-stream"
        )
      })
  })
}

pub(crate) fn is_manifest(mime: Option<&str>) -> bool {
  mime.is_some_and(|m| {
    matches!(
      m.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str(),
      "application/vnd.apple.mpegurl"
        | "application/x-mpegurl"
        | "application/dash+xml"
        | "audio/mpegurl"
        | "audio/x-mpegurl"
    )
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

/// Conservative port of Android MediaHttpPolicy, including mapped IPv4 and well-known NAT64.
pub(crate) fn public_address(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => {
      let value = u32::from(v4);
      ![
        (0x00000000, 8),
        (0x0a000000, 8),
        (0x64400000, 10),
        (0x7f000000, 8),
        (0xa9fe0000, 16),
        (0xac100000, 12),
        (0xc0000000, 24),
        (0xc0000200, 24),
        (0xc0586300, 24),
        (0xc0a80000, 16),
        (0xc6120000, 15),
        (0xc6336400, 24),
        (0xcb007100, 24),
        (0xe0000000, 4),
        (0xf0000000, 4),
      ]
      .iter()
      .any(|(prefix, bits)| value & (u32::MAX << (32 - bits)) == *prefix)
    }
    IpAddr::V6(v6) => {
      if let Some(v4) = v6.to_ipv4_mapped() {
        return public_address(v4.into());
      }
      let b = v6.octets();
      if b[..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
        return public_address(Ipv4Addr::new(b[12], b[13], b[14], b[15]).into());
      }
      b[0] & 0xe0 == 0x20
        && !(b[0] == 0x20 && b[1] == 1 && b[2] & 0xfe == 0)
        && b[..4] != [0x20, 1, 0x0d, 0xb8]
        && b[..2] != [0x20, 2]
        && !(b[0] == 0x3f && b[1] == 0xff && b[2] & 0xf0 == 0)
    }
  }
}
