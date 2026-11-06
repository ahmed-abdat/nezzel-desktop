//! Network admission shared by every engine request that fetches a link the user gave us
//! (`resolve::http` page reads, `files::client` downloads): which addresses a request may reach,
//! and the pinned client that connects to exactly the admitted answer. Each consumer keeps its
//! own user agent, headers, ports, redirect loop and timeouts.
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Desktop Chrome, for requests that must look like a browser navigation (page reads, Dropbox).
pub(crate) const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/// Bound on one DNS lookup during admission.
pub(crate) const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
  /// An empty answer, an answer on another port, or an address the policy refuses.
  Address,
  /// The lookup did not answer within [`LOOKUP_TIMEOUT`].
  Timeout,
  /// The lookup failed.
  Lookup(std::io::ErrorKind),
}

/// The URL host without IPv6 brackets, ready for an IP-literal parse or a lookup.
pub(crate) fn host(url: &Url) -> Option<&str> {
  url.host_str().map(|host| host.trim_matches(['[', ']']))
}

/// An IP-literal host is its own answer; any other host is looked up once. Every address in the
/// answer must pass `allows`, so one private address refuses the whole host.
pub(crate) async fn admit(
  host: &str,
  port: u16,
  allows: impl Fn(IpAddr) -> bool,
) -> Result<Vec<SocketAddr>, Refusal> {
  let answer = if let Ok(ip) = host.parse::<IpAddr>() {
    vec![SocketAddr::new(ip, port)]
  } else {
    tokio::time::timeout(LOOKUP_TIMEOUT, tokio::net::lookup_host((host, port)))
      .await
      .map_err(|_| Refusal::Timeout)?
      .map_err(|error| Refusal::Lookup(error.kind()))?
      .collect()
  };
  admit_answer(answer, port, allows)
}

pub(crate) fn admit_answer(
  answer: Vec<SocketAddr>,
  port: u16,
  allows: impl Fn(IpAddr) -> bool,
) -> Result<Vec<SocketAddr>, Refusal> {
  if answer.is_empty() || answer.iter().any(|a| a.port() != port || !allows(a.ip())) {
    return Err(Refusal::Address);
  }
  Ok(answer)
}

/// The hardened builder every admitted request starts from. It connects to `addresses` only:
/// reqwest never resolves `host` again (no DNS rebinding between check and connect), no proxy
/// resolves it instead, and reqwest never follows a redirect itself, so each hop returns to the
/// caller's admission. No retries, no referer, no decompression (callers bound raw bytes),
/// HTTP/1.1, TLS 1.2+. Timeouts, `https_only` and headers stay with the caller.
pub(crate) fn pinned_builder(host: &str, addresses: &[SocketAddr]) -> reqwest::ClientBuilder {
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
    .resolve_to_addrs(host, addresses)
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn public_address_rejects_private_android_ranges() {
    for ip in [
      "0.0.0.0",
      "10.1.2.3",
      "100.64.1.1",
      "127.0.0.1",
      "169.254.169.254",
      "172.16.0.1",
      "192.0.0.8",
      "192.168.1.1",
      "198.18.0.1",
      "192.0.2.1",
      "198.51.100.2",
      "203.0.113.1",
      "224.0.0.1",
      "255.255.255.255",
      "::",
      "::1",
      "fc00::1",
      "fe80::1",
      "ff02::1",
      "2001:db8::1",
      "2002:808:808::1",
      "2001::1",
      "3fff::1",
      "::ffff:127.0.0.1",
      "64:ff9b::a00:1",
    ] {
      assert!(!public_address(ip.parse().unwrap()), "{ip}");
    }
    for ip in [
      "8.8.8.8",
      "1.1.1.1",
      "2606:4700:4700::1111",
      "::ffff:8.8.8.8",
      "64:ff9b::808:808",
    ] {
      assert!(public_address(ip.parse().unwrap()), "{ip}");
    }
  }

  #[test]
  fn one_refused_or_off_port_address_refuses_the_whole_answer() {
    let public = |a: &str| a.parse::<SocketAddr>().unwrap();
    assert_eq!(
      admit_answer(vec![public("8.8.8.8:443")], 443, public_address),
      Ok(vec![public("8.8.8.8:443")])
    );
    for answer in [
      vec![],
      vec![public("8.8.8.8:443"), public("10.0.0.1:443")],
      vec![
        public("[2606:4700:4700::1111]:443"),
        public("[::ffff:127.0.0.1]:443"),
      ],
      vec![public("8.8.8.8:80")],
    ] {
      assert_eq!(
        admit_answer(answer.clone(), 443, public_address),
        Err(Refusal::Address),
        "{answer:?}"
      );
    }
  }

  #[tokio::test]
  async fn ip_literals_are_their_own_answer_and_hosts_lose_brackets() {
    let url = Url::parse("https://[::1]:8443/x").unwrap();
    assert_eq!(host(&url), Some("::1"));
    assert_eq!(
      admit("::1", 8443, |ip| ip.is_loopback()).await,
      Ok(vec!["[::1]:8443".parse().unwrap()])
    );
    assert_eq!(
      admit("::1", 8443, public_address).await,
      Err(Refusal::Address)
    );
  }
}
