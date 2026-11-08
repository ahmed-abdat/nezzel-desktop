//! Site-session metadata and private, site-scoped cookie exports.
//! The site table below says which cookies belong to a site; each child module reads or writes
//! one thing: `hint` (sign-in hints), `chromium`/`firefox` (store layouts), `jar` (the Netscape
//! jar on disk), `exporter` (the yt-dlp export process), `access` (one prepared cookie source).
mod access;
mod chromium;
mod exporter;
mod firefox;
mod hint;
mod jar;
#[cfg(test)]
mod tests;

use crate::sites::{self, SiteId};
pub(super) use access::{CookieAccess, CookieContext, COOKIE_READ_ERROR};
pub(super) use hint::{cookie_hint, CookieHint};
pub(crate) use jar::sweep_cookie_exports;
use std::collections::{HashMap, HashSet};

pub(super) fn registrable_domain(host: &str) -> Option<String> {
  let host = host.trim_matches('.').to_ascii_lowercase();
  if host.is_empty()
    || !host
      .bytes()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
    || host.parse::<std::net::IpAddr>().is_ok()
  {
    return None;
  }
  psl::domain_str(&host).map(str::to_owned)
}

/// Sites whose sign-in covers all their hosts: consent is kept under the site's main domain.
const SHARED_SIGN_IN: &[SiteId] = &[SiteId::YouTube, SiteId::Reddit, SiteId::Threads, SiteId::X];

/// Canonical consent identity; cookie_domains enumerates the permitted cookie aliases.
pub(super) fn site_key(host: &str) -> Option<String> {
  let domain = registrable_domain(host)?;
  Some(match sites::site_of(&domain) {
    Some(site) if SHARED_SIGN_IN.contains(&site) => site.domain().into(),
    _ => domain,
  })
}

/// The site a consent identity names, by the site's own domain only (`fb.watch` is not Facebook here).
fn signed_in_site(host: &str) -> Option<SiteId> {
  site_key(host).and_then(|key| sites::site_by_domain(&key))
}

/// The aliases earlier versions already kept under one consent key, frozen: a stored entry under any other
/// host (e.g. `youtube-nocookie.com`, now YouTube via `sites`) was never reused without an explicit pick and is
/// not promoted into its site's consent by migration.
const LEGACY_ALIASES: [(&str, &str); 4] = [
  ("redd.it", "reddit.com"),
  ("youtu.be", "youtube.com"),
  ("threads.com", "threads.net"),
  ("twitter.com", "x.com"),
];

/// The key a stored entry migrates to: its registrable domain, or the canonical site for a legacy alias.
fn migration_key(host: &str) -> Option<String> {
  let domain = registrable_domain(host)?;
  Some(
    LEGACY_ALIASES
      .iter()
      .find(|(alias, _)| *alias == domain)
      .map_or(domain, |(_, canonical)| (*canonical).to_owned()),
  )
}

/// Canonical entries win collisions, then root hosts, www hosts, and other legacy hosts.
/// Existing browser values are moved verbatim; migration never discovers a new profile.
pub(super) fn migrate_site_browsers(browsers: &mut HashMap<String, String>) {
  let mut entries: Vec<_> = std::mem::take(browsers).into_iter().collect();
  entries.sort_by_key(|(host, _)| {
    let canonical = migration_key(host).unwrap_or_else(|| host.clone());
    let priority = if host == &canonical {
      0
    } else if registrable_domain(host).as_ref() == Some(host) {
      1
    } else if host.starts_with("www.") {
      2
    } else {
      3
    };
    (priority, host.clone())
  });
  for (host, browser) in entries {
    browsers
      .entry(migration_key(&host).unwrap_or(host))
      .or_insert(browser);
  }
}

pub(super) fn same_site(cookie_host: &str, host: &str) -> bool {
  registrable_domain(cookie_host).is_some_and(|domain| Some(domain) == registrable_domain(host))
}

fn cookie_domains(host: &str) -> Vec<String> {
  let Some(key) = site_key(host) else {
    return vec![];
  };
  let owned = |domains: &[&str]| domains.iter().map(|d| d.to_string()).collect();
  match signed_in_site(&key) {
    // Threads signs in with the Instagram session.
    Some(SiteId::Threads) => {
      owned(&[SiteId::Threads.domains(), SiteId::Instagram.domains()].concat())
    }
    Some(site) if SHARED_SIGN_IN.contains(&site) => owned(site.domains()),
    _ => vec![key],
  }
}

enum SessionCookies {
  Any(&'static [&'static str]),
  All(&'static [&'static str]),
}
fn session_cookies(host: &str) -> Option<SessionCookies> {
  use SessionCookies::{All, Any};
  use SiteId::*;
  match signed_in_site(host)? {
    Instagram | Threads | TikTok => Some(Any(&["sessionid"])),
    Facebook => Some(All(&["c_user", "xs"])),
    X => Some(Any(&["auth_token"])),
    YouTube => Some(Any(&["SAPISID", "__Secure-3PSID", "LOGIN_INFO"])),
    Reddit => Some(Any(&["reddit_session"])),
    LinkedIn => Some(Any(&["li_at"])),
    Pinterest => Some(Any(&["_pinterest_sess"])),
    Vimeo => Some(Any(&["vimeo"])),
    Snapchat => Some(Any(&["sc-a-session", "__Host-sc-a-session"])),
    _ => None,
  }
}
pub(crate) fn known_login_host(host: &str) -> bool {
  session_cookies(host).is_some()
}
pub(super) fn session_present(host: &str, names: &HashSet<String>) -> bool {
  match session_cookies(host) {
    Some(SessionCookies::Any(required)) => required.iter().any(|name| names.contains(*name)),
    Some(SessionCookies::All(required)) => required.iter().all(|name| names.contains(*name)),
    None => false,
  }
}

/// Use the same site-specific names for encryption hints and sign-in detection.
fn is_session_cookie(host: &str, name: &str) -> bool {
  match session_cookies(host) {
    Some(SessionCookies::Any(required) | SessionCookies::All(required)) => required.contains(&name),
    None => false,
  }
}
