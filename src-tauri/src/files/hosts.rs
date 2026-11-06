//! Pure anonymous share-link plans. No cookies, credentials or network access.
use super::{Error, Result};
use reqwest::Url;

#[derive(Clone, Copy)]
pub enum Export {
  Pdf,
  Office,
}
#[derive(Clone, Copy)]
enum Host {
  Drive,
  Docs,
  Dropbox,
}
pub struct Plan {
  pub url: Url,
  host: Host,
  id: String,
  resource_key: Option<String>,
}
fn identifier(value: &str) -> bool {
  !value.is_empty()
    && value
      .bytes()
      .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn query(url: &Url, name: &str) -> Option<String> {
  url
    .query_pairs()
    .find(|(k, _)| k == name)
    .map(|(_, v)| v.into_owned())
}
fn secure(url: &Url) -> bool {
  url.scheme() == "https"
    && url.username().is_empty()
    && url.password().is_none()
    && url.port().is_none()
}
pub fn plan(input: &str, export: Export) -> Option<Plan> {
  let input = input.trim();
  // URL parsing discards explicit default ports, so enforce the no-port boundary first.
  let authority = input
    .split_once("://")
    .map_or(input, |(_, rest)| rest)
    .split(['/', '?', '#'])
    .next()?;
  if authority.contains(':') {
    return None;
  }
  let mut url = if input.contains("://") {
    Url::parse(input).ok()?
  } else {
    Url::parse(&format!("https://{input}")).ok()?
  };
  // Only the exact share hosts below can produce a plan; never relax redirect security.
  if url.scheme() == "http" {
    url.set_scheme("https").ok()?;
  }
  if !secure(&url) {
    return None;
  }
  let segments: Vec<_> = url.path_segments()?.collect();
  let host = url.host_str()?;
  let mut id = String::new();
  let kind = match host {
    "drive.google.com" | "drive.usercontent.google.com" => {
      id = match segments.as_slice() {
        ["file", "d", id, ..] | ["file", "u", _, "d", id, ..] => (*id).to_owned(),
        ["open" | "uc" | "download"] => query(&url, "id")?,
        _ => return None,
      };
      if !identifier(&id) {
        return None;
      }
      let key = query(&url, "resourcekey");
      url = Url::parse("https://drive.usercontent.google.com/download").ok()?;
      url
        .query_pairs_mut()
        .append_pair("id", &id)
        .append_pair("export", "download");
      if let Some(key) = key {
        url.query_pairs_mut().append_pair("resourcekey", &key);
      }
      Host::Drive
    }
    "docs.google.com" => {
      let (doc, value) = match segments.as_slice() {
        [kind @ ("document" | "spreadsheets" | "presentation"), "d", id, ..] if *id != "e" => {
          (*kind, *id)
        }
        _ => return None,
      };
      if !identifier(value) {
        return None;
      }
      id = value.to_owned();
      let doc = doc.to_owned();
      let format = match (export, doc.as_str()) {
        (Export::Pdf, _) => "pdf",
        (Export::Office, "document") => "docx",
        (Export::Office, "spreadsheets") => "xlsx",
        _ => "pptx",
      };
      let key = query(&url, "resourcekey");
      url.set_path(&format!(
        "/{doc}/d/{id}/export{}",
        if doc == "presentation" {
          format!("/{format}")
        } else {
          String::new()
        }
      ));
      url.set_query(None);
      if doc != "presentation" {
        url.query_pairs_mut().append_pair("format", format);
      }
      if let Some(key) = key {
        url.query_pairs_mut().append_pair("resourcekey", &key);
      }
      Host::Docs
    }
    "dropbox.com" | "www.dropbox.com" => {
      match segments.as_slice() {
        ["s" | "sh", id, _, ..] | ["scl", "fi" | "fo", id, _, ..] if !id.is_empty() => {}
        _ => return None,
      }
      let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k != "dl")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
      url.set_host(Some("www.dropbox.com")).ok()?;
      url.set_query(None);
      url
        .query_pairs_mut()
        .extend_pairs(pairs)
        .append_pair("dl", "1");
      Host::Dropbox
    }
    _ => return None,
  };
  url.set_fragment(None);
  Some(Plan {
    resource_key: query(&url, "resourcekey"),
    url,
    host: kind,
    id,
  })
}
/// Dropbox's anonymous endpoints require the browser UA, including on range retries.
pub(crate) fn dropbox_host(host: &str) -> bool {
  matches!(
    host,
    "www.dropbox.com" | "dropbox.com" | "dl.dropboxusercontent.com"
  ) || host.ends_with(".dl.dropboxusercontent.com")
}
/// True when the link is an anonymous share link (Drive, Docs, Dropbox). The export format
/// is irrelevant here, so callers that only ask "is this a share link" use this.
pub(crate) fn is_share(input: &str) -> bool {
  plan(input, Export::Pdf).is_some()
}
/// Share hosts answer 429/509 with a quota page the caller must see, so those are not retried.
pub(crate) fn relax_rate_limit(url: &str) -> bool {
  is_share(url)
    || Url::parse(url)
      .ok()
      .and_then(|u| u.host_str().map(dropbox_host))
      .unwrap_or(false)
}
/// Drive and Docs bounce non-public files to the Google sign-in page.
pub(crate) fn sign_in_page(url: &Url) -> bool {
  url.host_str() == Some("accounts.google.com")
}
/// A share link redirected to the sign-in page means the file is not public.
pub(crate) fn sign_in_redirect(from: &Url, to: &Url) -> bool {
  sign_in_page(to) && is_share(from.as_str())
}
/// Dropbox needs a browser UA; everything else identifies as Nezzel.
pub(crate) fn user_agent(url: &Url) -> &'static str {
  if url.host_str().is_some_and(dropbox_host) {
    crate::net::CHROME_UA
  } else {
    concat!("Nezzel/", env!("CARGO_PKG_VERSION"))
  }
}
fn drive_download(url: &Url) -> bool {
  secure(url)
    && matches!(
      (url.host_str(), url.path()),
      (Some("drive.usercontent.google.com"), "/download") | (Some("drive.google.com"), "/uc")
    )
}
impl Plan {
  pub(crate) fn folder(&self) -> bool {
    matches!(self.host, Host::Dropbox)
      && (self.url.path().starts_with("/scl/fo/") || self.url.path().starts_with("/sh/"))
  }
  pub(crate) fn native_export(&self) -> bool {
    matches!(self.host, Host::Docs)
  }
  pub fn next(&self, page: &Url, html: &str) -> Result<Url> {
    let lower = html.to_ascii_lowercase();
    if lower.contains("too many users have viewed")
      || lower.contains("downloadquotaexceeded")
      || lower.contains("quota exceeded")
      || lower.contains("links generating too much traffic")
    {
      return Err(Error::RateLimited);
    }
    let tags = crate::resolve::html::all_tags(html).ok_or(Error::NotFile)?;
    match self.host {
      Host::Drive => {
        let mut form: Option<(Url, Vec<(String, String)>)> = None;
        for tag in &tags {
          if tag.name == "form" {
            if tag.closing {
              if let Some((mut url, fields)) = form.take() {
                let field = |name: &str| {
                  fields
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.as_str())
                };
                if field("id") == Some(self.id.as_str())
                  && field("confirm").is_some_and(|v| !v.is_empty())
                {
                  let fields = fields
                    .into_iter()
                    .filter(|(k, _)| k != "resourcekey" || self.resource_key.is_none());
                  url.query_pairs_mut().extend_pairs(fields);
                  if let Some(key) = &self.resource_key {
                    let pairs: Vec<_> = url
                      .query_pairs()
                      .filter(|(k, _)| k != "resourcekey")
                      .map(|(k, v)| (k.into_owned(), v.into_owned()))
                      .collect();
                    url.set_query(None);
                    url
                      .query_pairs_mut()
                      .extend_pairs(pairs)
                      .append_pair("resourcekey", key);
                  }
                  return Ok(url);
                }
              }
            } else {
              form = tag
                .attrs
                .get("action")
                .and_then(|a| page.join(a).ok())
                .filter(drive_download)
                .filter(|_| {
                  tag
                    .attrs
                    .get("method")
                    .is_none_or(|m| m.eq_ignore_ascii_case("get"))
                })
                .map(|u| (u, Vec::new()));
            }
          } else if !tag.closing
            && tag.name == "input"
            && tag
              .attrs
              .get("type")
              .is_some_and(|t| t.eq_ignore_ascii_case("hidden"))
          {
            if let (Some((_, fields)), Some(name)) =
              (&mut form, tag.attrs.get("name").filter(|n| !n.is_empty()))
            {
              fields.push((
                name.clone(),
                tag.attrs.get("value").cloned().unwrap_or_default(),
              ));
            }
          }
        }
      }
      Host::Dropbox => {
        for tag in &tags {
          if !tag.closing && tag.name == "a" {
            if let Some(url) = tag
              .attrs
              .get("href")
              .and_then(|h| page.join(h).ok())
              .filter(|u| {
                secure(u)
                  && u.host_str().is_some_and(|h| {
                    h == "dl.dropboxusercontent.com" || h.ends_with(".dl.dropboxusercontent.com")
                  })
              })
            {
              return Ok(url);
            }
          }
        }
      }
      Host::Docs => {}
    }
    if [
      "you need access",
      "request access",
      "access denied",
      "accounts.google.com/servicelogin",
      "file isn't shared",
      "file is not shared",
      "/sm/password",
      "ajax_verify_code",
      "error_",
      "images/precaution",
    ]
    .iter()
    .any(|s| lower.contains(s))
      || tags.iter().any(|t| {
        t.name == "input"
          && t
            .attrs
            .get("type")
            .is_some_and(|v| v.eq_ignore_ascii_case("password"))
      })
    {
      Err(Error::NotPublic)
    } else {
      Err(Error::NotFile)
    }
  }
}
#[cfg(test)]
mod rules_tests {
  use super::*;
  #[test]
  fn share_host_rules() {
    let u = |s: &str| Url::parse(s).unwrap();
    assert!(is_share("https://drive.google.com/file/d/abc/view"));
    assert!(!is_share("https://example.com/file/d/abc/view"));
    assert!(relax_rate_limit("https://www.dropbox.com/s/abc/f.txt"));
    assert!(relax_rate_limit("https://uc1.dl.dropboxusercontent.com/f"));
    assert!(!relax_rate_limit("https://example.com/f"));
    assert!(sign_in_redirect(
      &u("https://drive.google.com/uc?id=abc"),
      &u("https://accounts.google.com/ServiceLogin")
    ));
    assert!(!sign_in_redirect(
      &u("https://example.com/f"),
      &u("https://accounts.google.com/ServiceLogin")
    ));
    assert_eq!(
      user_agent(&u("https://www.dropbox.com/s/a/b")),
      crate::net::CHROME_UA
    );
    assert!(user_agent(&u("https://example.com/")).starts_with("Nezzel/"));
  }
}
