//! DNS-admitted HTTP transport. Redirects never escape the same address policy.
use super::{Error, Result};
use reqwest::Url;
use std::{
  collections::HashMap,
  net::{IpAddr, SocketAddr},
  sync::{Arc, LazyLock, Mutex, Weak},
  time::Duration,
};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

static HOSTS: LazyLock<Mutex<HashMap<String, Weak<Semaphore>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
/// Fixed, enumerable cause classes only: free-text connector/TLS causes can carry addresses,
/// certificate names or paths, so nothing from the error text itself reaches the log.
fn cause_class(debug: &str) -> &'static str {
  const CLASSES: [(&str, &str); 12] = [
    ("InvalidCertificate", "tls_certificate"),
    ("UnknownIssuer", "tls_certificate"),
    ("NotValidForName", "tls_certificate"),
    ("Expired", "tls_certificate"),
    ("HandshakeFailure", "tls_handshake"),
    ("AlertReceived", "tls_handshake"),
    ("dns error", "dns"),
    ("failed to lookup address", "dns"),
    ("ConnectionRefused", "refused"),
    ("ConnectionReset", "reset"),
    ("UnexpectedEof", "eof"),
    ("TimedOut", "timed_out"),
  ];
  CLASSES
    .iter()
    .find(|(token, _)| debug.contains(token))
    .map_or("other", |(_, class)| class)
}
pub(crate) fn network_error(error: reqwest::Error) -> Error {
  let kind = if error.is_timeout() && error.is_connect() {
    "connect_timeout"
  } else if error.is_timeout() {
    "timeout"
  } else if error.is_connect() {
    "connect"
  } else if error.is_body() {
    "body"
  } else if error.is_builder() {
    "builder"
  } else {
    "request"
  };
  let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
  let mut io_kind = None;
  while let Some(error) = source {
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
      io_kind = Some(error.kind());
    }
    source = error.source();
  }
  let cause = cause_class(&format!("{:?}", error.without_url()));
  tracing::warn!(error_kind = kind, io_kind = ?io_kind, cause_class = cause, "File HTTP request failed");
  Error::Network
}
fn user_agent(url: &Url) -> &'static str {
  if url.host_str().is_some_and(super::hosts::dropbox_host) {
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36"
  } else {
    concat!("Nezzel/", env!("CARGO_PKG_VERSION"))
  }
}
#[derive(Clone, Copy, Default)]
enum AddressPolicy {
  #[default]
  Public,
  #[cfg(test)]
  Loopback,
}
impl AddressPolicy {
  fn allows(self, ip: IpAddr) -> bool {
    #[cfg(test)]
    if matches!(self, Self::Loopback) {
      return ip.is_loopback();
    }
    crate::resolve::http::public_address(ip)
  }
}

pub(crate) struct Response {
  pub response: reqwest::Response,
  // Cached probes release the host slot and reacquire it before consumption.
  _permit: Option<OwnedSemaphorePermit>,
  host: Option<Arc<Semaphore>>,
  pub ranged: bool,
}
impl Response {
  pub fn release(&mut self) {
    self._permit = None;
  }
  pub async fn acquire(&mut self, mut cancel: watch::Receiver<bool>) -> Result<()> {
    if self._permit.is_none() {
      if let Some(host) = &self.host {
        self._permit = Some(
          wait(&mut cancel, async {
            host.clone().acquire_owned().await.map_err(|_| Error::Busy)
          })
          .await?,
        );
      }
    }
    Ok(())
  }
}
#[cfg(test)]
pub(crate) type Script =
  Arc<dyn Fn(&str, Option<(u64, Option<u64>, &str)>) -> Result<reqwest::Response> + Send + Sync>;
#[cfg(test)]
type HopScript = Arc<dyn Fn(&reqwest::Request) -> Result<reqwest::Response> + Send + Sync>;
#[derive(Clone)]
pub(crate) struct Client {
  policy: AddressPolicy,
  #[cfg(test)]
  pub script: Option<Script>,
  #[cfg(test)]
  hop_script: Option<HopScript>,
}
impl Client {
  pub fn new() -> Result<Self> {
    Ok(Self {
      policy: AddressPolicy::Public,
      #[cfg(test)]
      script: None,
      #[cfg(test)]
      hop_script: None,
    })
  }
  #[cfg(test)]
  pub fn local() -> Self {
    Self {
      policy: AddressPolicy::Loopback,
      script: None,
      #[cfg(test)]
      hop_script: None,
    }
  }
  async fn addresses(&self, url: &Url) -> Result<Vec<SocketAddr>> {
    let host = url
      .host_str()
      .ok_or(Error::Address)?
      .trim_matches(['[', ']']);
    let port = url
      .port_or_known_default()
      .filter(|p| *p != 0)
      .ok_or(Error::Address)?;
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
      vec![SocketAddr::new(ip, port)]
    } else {
      tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
      )
      .await
      .map_err(|_| {
        tracing::warn!(error_kind = "dns_timeout", "File DNS lookup failed");
        Error::Network
      })?
      .map_err(|error| {
        tracing::warn!(error_kind="dns",io_kind=?error.kind(),"File DNS lookup failed");
        Error::Network
      })?
      .collect::<Vec<_>>()
    };
    if addresses.is_empty()
      || addresses
        .iter()
        .any(|a| a.port() != port || !self.policy.allows(a.ip()))
    {
      return Err(Error::Address);
    }
    Ok(addresses)
  }
  fn pinned_client(&self, url: &Url, addresses: &[SocketAddr]) -> Result<reqwest::Client> {
    reqwest::Client::builder()
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
      .connect_timeout(Duration::from_secs(15))
      .read_timeout(Duration::from_secs(30))
      .resolve_to_addrs(url.host_str().ok_or(Error::Address)?, addresses)
      .build()
      .map_err(network_error)
  }
  pub fn admit(&self, url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https")
      || url.host_str().is_none()
      || !url.username().is_empty()
      || url.password().is_some()
    {
      return Err(Error::Address);
    }
    if url
      .host_str()
      .and_then(|h| h.trim_matches(['[', ']']).parse::<IpAddr>().ok())
      .is_some_and(|ip| !self.policy.allows(ip))
    {
      return Err(Error::Address);
    }
    Ok(())
  }
  pub async fn get(
    &self,
    url: &str,
    range: Option<(u64, Option<u64>, &str)>,
    mut cancel: watch::Receiver<bool>,
  ) -> Result<Response> {
    #[cfg(test)]
    if let Some(script) = &self.script {
      return Ok(Response {
        response: script(url, range)?,
        _permit: None,
        host: None,
        ranged: range.is_some(),
      });
    }
    let mut url = Url::parse(url).map_err(|_| Error::Address)?;
    url.set_fragment(None);
    for hop in 0..=10 {
      self.admit(&url)?;
      let host = url.host_str().ok_or(Error::Address)?.to_ascii_lowercase();
      let sem = {
        let mut hosts = HOSTS.lock().map_err(|_| Error::State)?;
        hosts.retain(|_, v| v.strong_count() > 0);
        if hosts.len() >= 128 && !hosts.contains_key(&host) {
          return Err(Error::Busy);
        }
        let slot = hosts.entry(host).or_default();
        slot.upgrade().unwrap_or_else(|| {
          let sem = Arc::new(Semaphore::new(8));
          *slot = Arc::downgrade(&sem);
          sem
        })
      };
      let permit = wait(&mut cancel, async {
        sem.clone().acquire_owned().await.map_err(|_| Error::Busy)
      })
      .await?;
      #[cfg(test)]
      let scripted_hop = self.hop_script.is_some();
      #[cfg(not(test))]
      let scripted_hop = false;
      let addresses = if scripted_hop {
        Vec::new()
      } else {
        wait(&mut cancel, self.addresses(&url)).await?
      };
      let client = self.pinned_client(&url, &addresses)?;
      let mut request = client
        .get(url.clone())
        .header("Accept", "*/*")
        .header("Accept-Encoding", "identity")
        .header("User-Agent", user_agent(&url));
      if let Some((start, end, validator)) = range {
        request = request
          .header(
            "Range",
            format!(
              "bytes={start}-{}",
              end.map(|n| n.to_string()).unwrap_or_default()
            ),
          )
          .header("If-Range", validator);
      }
      let request = request.build().map_err(network_error)?;
      #[cfg(test)]
      let scripted = self.hop_script.as_ref().map(|script| script(&request));
      #[cfg(not(test))]
      let scripted: Option<Result<reqwest::Response>> = None;
      let response = match scripted {
        Some(response) => response?,
        None => {
          wait(&mut cancel, async {
            client.execute(request).await.map_err(network_error)
          })
          .await?
        }
      };
      if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        let location = response
          .headers()
          .get("location")
          .and_then(|h| h.to_str().ok())
          .ok_or(Error::Address)?;
        let next = url.join(location).map_err(|_| Error::Address)?;
        if next.host_str() == Some("accounts.google.com")
          && super::hosts::plan(url.as_str(), super::hosts::Export::Pdf).is_some()
        {
          return Err(Error::NotPublic);
        }
        if hop == 10 || (url.scheme() == "https" && next.scheme() != "https") {
          return Err(Error::Address);
        }
        url = next;
        continue;
      }
      return Ok(Response {
        response,
        _permit: Some(permit),
        host: Some(sem),
        ranged: range.is_some(),
      });
    }
    Err(Error::Address)
  }
}

pub(crate) async fn wait<T>(
  cancel: &mut watch::Receiver<bool>,
  future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
  if !*cancel.borrow_and_update() {
    return Err(Error::Cancelled);
  }
  tokio::pin!(future);
  loop {
    tokio::select! {biased;
      changed=cancel.changed()=>{if changed.is_err() || !*cancel.borrow_and_update() {return Err(Error::Cancelled);}},
      result=&mut future=>return result,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn network_diagnostics_keep_kind_without_url_or_credentials() {
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
      fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
      }
      fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
      }
    }
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = bytes.clone();
    let subscriber = tracing_subscriber::fmt()
      .without_time()
      .with_ansi(false)
      .with_writer(move || Capture(sink.clone()))
      .finish();
    let error = reqwest::Client::new()
      .get("https://example.invalid/")
      .header("invalid\nname", "value")
      .build()
      .unwrap_err()
      .with_url(
        Url::parse("https://user:password-secret@example.invalid/private?token=query-secret")
          .unwrap(),
      );
    tracing::subscriber::with_default(subscriber, || {
      assert!(matches!(network_error(error), Error::Network));
    });
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(log.contains("builder"));
    for (debug, class) in [
      (
        "Connect(93.184.215.14:443), InvalidCertificate(192.0.2.5, alt.example.net)",
        "tls_certificate",
      ),
      (
        "hyper_util::client::legacy::Error(Connect, TimedOut)",
        "timed_out",
      ),
      ("dns error: failed to lookup address information", "dns"),
      ("Os { code: 61, kind: ConnectionRefused }", "refused"),
      ("something new at /Users/name/secret", "other"),
    ] {
      assert_eq!(cause_class(debug), class, "{debug}");
    }
    for secret in [
      "example.invalid",
      "password-secret",
      "query-secret",
      "https://",
      "/private",
    ] {
      assert!(!log.contains(secret));
    }
  }
  #[tokio::test]
  async fn admitted_dns_answers_keep_the_requested_port() {
    let client = Client::local();
    for (url, port) in [
      ("http://localhost:8123/file", 8123),
      ("https://localhost/file", 443),
    ] {
      let addresses = client.addresses(&Url::parse(url).unwrap()).await.unwrap();
      assert!(!addresses.is_empty() && addresses.iter().all(|a| a.port() == port));
    }
  }
  #[tokio::test]
  async fn real_connector_uses_the_production_pinned_client_for_hostname_and_ip() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
      for _ in 0..2 {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
          let mut byte = [0];
          socket.read_exact(&mut byte).await.unwrap();
          request.push(byte[0]);
        }
        // Some real download origins close spoofed browser requests without a response.
        if String::from_utf8_lossy(&request).contains("Mozilla/") {
          continue;
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: 4\r\nConnection: close\r\n\r\n%PDF").await.unwrap();
      }
    });
    let client = Client::local();
    let (_tx, rx) = watch::channel(true);
    for host in ["localhost", "127.0.0.1"] {
      let response = client
        .get(
          &format!("http://{host}:{port}/fixture.pdf"),
          None,
          rx.clone(),
        )
        .await
        .unwrap();
      assert_eq!(response.response.bytes().await.unwrap(), b"%PDF".as_slice());
    }
    server.await.unwrap();
  }
}

#[cfg(test)]
mod share_tests {
  use super::*;
  #[tokio::test]
  async fn anonymous_redirects_recompute_ua_and_never_send_cookie_or_referer() {
    use reqwest::ResponseBuilderExt;
    let mut client = Client::local();
    client.hop_script = Some(Arc::new(|request| {
      let url = request.url();
      for name in ["cookie", "authorization", "referer"] {
        assert!(!request.headers().contains_key(name));
      }
      let ua = request.headers()["user-agent"].to_str().unwrap();
      let response = http::Response::builder().url(url.clone());
      if url.host_str() == Some("www.dropbox.com") {
        assert!(ua.contains("Chrome/140.0.0.0"));
        Ok(
          response
            .status(302)
            .header("location", "https://uc1.dl.dropboxusercontent.com/file")
            .header("set-cookie", "session=secret")
            .body(Vec::<u8>::new())
            .unwrap()
            .into(),
        )
      } else if url.host_str() == Some("uc1.dl.dropboxusercontent.com") {
        assert!(ua.contains("Chrome/140.0.0.0"));
        Ok(
          response
            .status(302)
            .header("location", "https://example.com/final")
            .body(Vec::<u8>::new())
            .unwrap()
            .into(),
        )
      } else {
        assert!(ua.starts_with("Nezzel/"));
        Ok(response.body(b"data".to_vec()).unwrap().into())
      }
    }));
    let (_tx, rx) = watch::channel(true);
    let response = client
      .get("https://www.dropbox.com/s/abc/file?dl=1", None, rx)
      .await
      .unwrap();
    assert_eq!(response.response.bytes().await.unwrap().as_ref(), b"data");
  }
  #[tokio::test]
  async fn shared_redirect_to_google_login_is_an_honest_error() {
    use reqwest::ResponseBuilderExt;
    let mut client = Client::local();
    client.hop_script = Some(Arc::new(|request| {
      assert_eq!(
        request.url().host_str(),
        Some("drive.usercontent.google.com")
      );
      Ok(
        http::Response::builder()
          .url(request.url().clone())
          .status(302)
          .header("location", "https://accounts.google.com/ServiceLogin")
          .body(Vec::<u8>::new())
          .unwrap()
          .into(),
      )
    }));
    let (_tx, rx) = watch::channel(true);
    assert!(matches!(
      client
        .get(
          "https://drive.usercontent.google.com/download?id=abc",
          None,
          rx
        )
        .await,
      Err(Error::NotPublic)
    ));
  }
}
