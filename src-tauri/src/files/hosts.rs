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
  Pixeldrain,
  Yandex,
  MediaFire,
  Archive,
  ArchiveFile,
  GitHub,
}
pub struct Plan {
  pub url: Url,
  host: Host,
  id: String,
  resource_key: Option<String>,
  pub(crate) name: Option<String>,
  pub(crate) size: Option<u64>,
  file: Option<String>,
}
fn identifier(value: &str) -> bool {
  !value.is_empty()
    && value
      .bytes()
      .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn archive_identifier(value: &str) -> bool {
  !value.is_empty()
    && value
      .bytes()
      .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
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
  let has_scheme = input.split_once(':').is_some_and(|(scheme, _)| {
    scheme
      .as_bytes()
      .first()
      .is_some_and(u8::is_ascii_alphabetic)
      && scheme
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
  });
  // URL parsing discards explicit default ports, so enforce the no-port boundary first.
  let rest = if has_scheme {
    input.split_once("://").map_or(input, |(_, rest)| rest)
  } else {
    input
  };
  let authority = rest.split(['/', '?', '#']).next()?;
  if authority.contains(':') {
    return None;
  }
  let mut url = if has_scheme {
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
  // These two-segment share pages accept the slash copied by mobile browsers.
  if matches!(
    url.host_str()?,
    "pixeldrain.com"
      | "www.pixeldrain.com"
      | "disk.yandex.ru"
      | "disk.yandex.com"
      | "disk.yandex.net"
      | "yadi.sk"
  ) {
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
  }
  let segments: Vec<_> = url.path_segments()?.collect();
  let host = url.host_str()?;
  let mut id = String::new();
  let mut file = None;
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
    "pixeldrain.com" | "www.pixeldrain.com" => {
      id = match segments.as_slice() {
        ["u", value] | ["api", "file", value] | ["api", "file", value, "info"]
          if identifier(value) =>
        {
          (*value).to_owned()
        }
        _ => return None,
      };
      url = Url::parse(&format!("https://pixeldrain.com/api/file/{id}/info")).ok()?;
      Host::Pixeldrain
    }
    "disk.yandex.ru" | "disk.yandex.com" | "disk.yandex.net" | "yadi.sk" => {
      match segments.as_slice() {
        ["d" | "i", key] if identifier(key) => {}
        _ => return None,
      }
      url.set_fragment(None);
      // Public keys are URLs; encode the whole key, never interpolate it as a query.
      let public_key = url.to_string();
      url = Url::parse("https://cloud-api.yandex.net/v1/disk/public/resources").ok()?;
      url.query_pairs_mut().append_pair("public_key", &public_key);
      Host::Yandex
    }
    "mediafire.com" | "www.mediafire.com" => {
      match segments.as_slice() {
        ["file", key, ..] if identifier(key) => {}
        [""] if url.query().is_some_and(identifier) => {
          id = url.query()?.to_owned();
          url.set_path(&format!("/file/{id}"));
        }
        _ => return None,
      }
      url.set_host(Some("www.mediafire.com")).ok()?;
      url.set_query(None);
      Host::MediaFire
    }
    "archive.org" | "www.archive.org" => match segments.as_slice() {
      ["download", item, rest @ ..]
        if archive_identifier(item) && !rest.is_empty() && rest.iter().all(|p| !p.is_empty()) =>
      {
        url.set_host(Some("archive.org")).ok()?;
        Host::ArchiveFile
      }
      ["details", item, rest @ ..] if archive_identifier(item) => {
        id = (*item).to_owned();
        let path = rest.join("/");
        if !path.is_empty() {
          file = Some(String::from_utf8(super::names::percent_bytes(&path)).ok()?);
        }
        url = Url::parse(&format!("https://archive.org/metadata/{id}")).ok()?;
        Host::Archive
      }
      _ => return None,
    },
    "github.com" | "www.github.com" => {
      match segments.as_slice() {
        [owner, repo, "blob" | "raw", reference, rest @ ..]
          if identifier(owner)
            && !repo.is_empty()
            && !reference.is_empty()
            && rest.iter().all(|p| !p.is_empty())
            && !rest.is_empty() =>
        {
          // GitHub's raw route follows LFS redirects as well as ordinary raw files.
          let path = format!("/{owner}/{repo}/raw/{reference}/{}", rest.join("/"));
          url.set_path(&path);
        }
        [owner, repo, "releases", "download", tag, file]
          if identifier(owner) && !repo.is_empty() && !tag.is_empty() && !file.is_empty() => {}
        [owner, repo, "releases", "latest", "download", file]
          if identifier(owner) && !repo.is_empty() && !file.is_empty() => {}
        _ => return None,
      }
      url.set_host(Some("github.com")).ok()?;
      url.set_query(None);
      Host::GitHub
    }
    "raw.githubusercontent.com" => {
      match segments.as_slice() {
        [owner, repo, reference, rest @ ..]
          if identifier(owner)
            && !repo.is_empty()
            && !reference.is_empty()
            && !rest.is_empty()
            && rest.iter().all(|p| !p.is_empty()) => {}
        _ => return None,
      }
      url.set_query(None);
      Host::GitHub
    }
    _ => return None,
  };
  url.set_fragment(None);
  Some(Plan {
    resource_key: query(&url, "resourcekey"),
    url,
    host: kind,
    name: None,
    size: None,
    file,
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
/// True when the link is an exclusive anonymous file share. The export format
/// is irrelevant here, so callers that only ask "is this a share link" use this.
pub(crate) fn is_share(input: &str) -> bool {
  plan(input, Export::Pdf).is_some_and(|p| !matches!(p.host, Host::Archive))
}
/// Archive item pages may contain several media files; preserve extractor fallback.
pub(crate) fn archive_details(input: &str) -> bool {
  plan(input, Export::Pdf).is_some_and(|p| matches!(p.host, Host::Archive))
}
/// Share hosts answer 429/509 with a quota page the caller must see, so those are not retried.
pub(crate) fn relax_rate_limit(url: &str) -> bool {
  is_share(url)
    || Url::parse(url)
      .ok()
      .and_then(|u| {
        u.host_str().map(|host| {
          dropbox_host(host)
            || matches!(
              host,
              "cloud-api.yandex.net"
                | "pixeldrain.com"
                | "downloader.disk.yandex.ru"
                | "downloader.disk.yandex.com"
                | "downloader.disk.yandex.net"
                | "archive.org"
            )
            || mediafire_download(host)
        })
      })
      .unwrap_or(false)
}
/// Share hosts may redirect non-public files to a sign-in page.
pub(crate) fn sign_in_page(url: &Url) -> bool {
  url.host_str() == Some("accounts.google.com")
    || url.host_str() == Some("github.com") && matches!(url.path(), "/login" | "/session")
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
fn mediafire_download(host: &str) -> bool {
  host.strip_suffix(".mediafire.com").is_some_and(|prefix| {
    prefix
      .strip_prefix("download")
      .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
  })
}
impl Plan {
  /// API envelopes are selected by the requested endpoint, never by Content-Type.
  pub(crate) fn api(&self, url: &Url) -> bool {
    match self.host {
      Host::Pixeldrain => {
        url.host_str() == Some("pixeldrain.com")
          && url.path() == format!("/api/file/{}/info", self.id)
      }
      Host::Yandex => url.host_str() == Some("cloud-api.yandex.net"),
      Host::Archive => {
        url.host_str() == Some("archive.org") && url.path() == format!("/metadata/{}", self.id)
      }
      _ => false,
    }
  }
  pub(crate) fn direct(&self, url: &Url) -> bool {
    match self.host {
      Host::Pixeldrain | Host::Yandex | Host::Archive => !self.api(url),
      Host::MediaFire => url.host_str().is_some_and(mediafire_download),
      Host::ArchiveFile => !url.path().ends_with('/'),
      Host::GitHub => true,
      _ => false,
    }
  }
  pub(crate) fn github_raw(&self) -> bool {
    matches!(self.host, Host::GitHub)
      && !(self.url.host_str() == Some("github.com")
        && self
          .url
          .path_segments()
          .is_some_and(|mut parts| parts.nth(2) == Some("releases")))
  }
  pub(crate) fn response_error(&self, status: u16, text: &str) -> Error {
    let value = serde_json::from_str::<serde_json::Value>(text).ok();
    let code = value
      .as_ref()
      .and_then(|v| v["error"].as_str().or_else(|| v["value"].as_str()))
      .unwrap_or_default()
      .to_ascii_lowercase();
    if matches!(status, 429 | 509)
      || [
        "rate_limit",
        "limitexceeded",
        "download_limit",
        "quota",
        "hotlink",
        "captcha_required",
      ]
      .iter()
      .any(|s| code.contains(s))
    {
      Error::RateLimited
    } else if !matches!(self.host, Host::Drive | Host::Docs | Host::Dropbox)
      && (matches!(status, 404 | 410) || code.contains("not_found") || code.contains("notfound"))
    {
      Error::Removed
    } else if matches!(status, 401 | 403 | 404 | 460)
      || code.contains("forbidden")
      || code.contains("access_denied")
    {
      Error::NotPublic
    } else {
      Error::Network
    }
  }
  pub(crate) fn next_api(&mut self, page: &Url, text: &str) -> Result<Url> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| Error::NotFile)?;
    if value.get("error").is_some() || value["success"] == false {
      return Err(self.response_error(200, text));
    }
    match self.host {
      Host::Pixeldrain => {
        if value["id"].as_str() != Some(self.id.as_str()) {
          return Err(Error::NotFile);
        }
        if value["can_download"] == false {
          return Err(Error::NotPublic);
        }
        self.set_metadata(&value);
        page
          .join(&format!("/api/file/{}?download", self.id))
          .map_err(|_| Error::Address)
      }
      Host::Yandex => {
        if page.path() == "/v1/disk/public/resources" {
          if value["type"] != "file" {
            return Err(Error::NotFile);
          }
          self.set_metadata(&value);
          let mut url = page.clone();
          url.set_path("/v1/disk/public/resources/download");
          Ok(url)
        } else {
          if value["method"].as_str().is_some_and(|m| m != "GET") || value["templated"] == true {
            return Err(Error::NotFile);
          }
          let url = value["href"]
            .as_str()
            .and_then(|s| Url::parse(s).ok())
            .ok_or(Error::NotFile)?;
          if secure(&url)
            && matches!(
              url.host_str(),
              Some(
                "downloader.disk.yandex.ru"
                  | "downloader.disk.yandex.com"
                  | "downloader.disk.yandex.net"
              )
            )
          {
            Ok(url)
          } else {
            Err(Error::Address)
          }
        }
      }
      Host::Archive => {
        if value["is_dark"] == true {
          return Err(Error::NotPublic);
        }
        let files = value["files"].as_array().ok_or(Error::Removed)?;
        let mut candidates = files.iter().filter(|f| {
          let Some(name) = f["name"].as_str() else {
            return false;
          };
          if let Some(want) = &self.file {
            return name == want;
          }
          f["source"] == "original"
            && !name.starts_with("__ia_thumb.")
            && ![
              "_meta.sqlite",
              "_meta.xml",
              "_files.xml",
              "_archive.torrent",
              "_itemimage.png",
              "_itemimage.jpg",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        });
        let file = candidates.next().ok_or(Error::NotFile)?;
        if candidates.next().is_some() {
          return Err(Error::NotFile);
        }
        if file["private"] == "true" || file["private"] == true {
          return Err(Error::NotPublic);
        }
        let name = file["name"].as_str().ok_or(Error::NotFile)?;
        if name
          .split('/')
          .any(|p| p.is_empty() || matches!(p, "." | "..") || p.contains('\\'))
        {
          return Err(Error::NotFile);
        }
        self.set_metadata(file);
        let mut url = Url::parse("https://archive.org/download/").map_err(|_| Error::Address)?;
        url
          .path_segments_mut()
          .map_err(|_| Error::Address)?
          .pop_if_empty()
          .push(&self.id)
          .extend(name.split('/'));
        Ok(url)
      }
      _ => Err(Error::NotFile),
    }
  }
  fn set_metadata(&mut self, value: &serde_json::Value) {
    self.name = value["name"]
      .as_str()
      .filter(|n| !n.is_empty())
      .map(super::sanitize::sanitize_file_name);
    self.size = value["size"]
      .as_u64()
      .or_else(|| value["size"].as_str().and_then(|s| s.parse().ok()));
  }
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
      || lower.contains("bandwidth exceeded")
      || lower.contains("download limit exceeded")
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
      Host::MediaFire => {
        if lower.contains("file you requested has been removed")
          || lower.contains("invalid or deleted file")
          || lower.contains("file has been deleted")
        {
          return Err(Error::Removed);
        }
        for tag in &tags {
          if !tag.closing
            && tag.name == "a"
            && tag.attrs.get("id").is_some_and(|id| id == "downloadButton")
          {
            use base64::{
              alphabet,
              engine::{general_purpose::GeneralPurpose, DecodePaddingMode, GeneralPurposeConfig},
              Engine,
            };
            // Like browser atob: accept the scrambled value with or without padding.
            const ATOB: GeneralPurpose = GeneralPurpose::new(
              &alphabet::STANDARD,
              GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
            );
            let scrambled = tag
              .attrs
              .get("data-scrambled-url")
              .and_then(|encoded| String::from_utf8(ATOB.decode(encoded.trim()).ok()?).ok());
            // A junk href ("#", "javascript:…") must not hide the scrambled URL.
            if let Some(url) = tag
              .attrs
              .get("href")
              .cloned()
              .into_iter()
              .chain(scrambled)
              .filter_map(|h| page.join(&h).ok())
              .find(|u| secure(u) && u.host_str().is_some_and(mediafire_download))
            {
              return Ok(url);
            }
          }
        }
      }
      _ => {}
    }
    if [
      "file is private",
      "file is currently set to private",
      "permission denied",
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
  fn additional_host_plans() {
    for (input, want) in [
      ("https://archive.org/details/item.v1", "https://archive.org/metadata/item.v1"),
      ("https://github.com/o/r/blob/feature/topic/a.txt", "https://github.com/o/r/raw/feature/topic/a.txt"),

      ("https://pixeldrain.com/u/Ab123xyz", "https://pixeldrain.com/api/file/Ab123xyz/info"),
      ("https://pixeldrain.com/api/file/Ab123xyz?download", "https://pixeldrain.com/api/file/Ab123xyz/info"),
      ("https://disk.yandex.ru/d/abc_123", "https://cloud-api.yandex.net/v1/disk/public/resources?public_key=https%3A%2F%2Fdisk.yandex.ru%2Fd%2Fabc_123"),
      ("https://yadi.sk/i/abc-123", "https://cloud-api.yandex.net/v1/disk/public/resources?public_key=https%3A%2F%2Fyadi.sk%2Fi%2Fabc-123"),
      ("https://www.mediafire.com/file/abc123/test.txt/file", "https://www.mediafire.com/file/abc123/test.txt/file"),
      ("https://mediafire.com/?abc123", "https://www.mediafire.com/file/abc123"),
      ("https://archive.org/download/item/test.txt", "https://archive.org/download/item/test.txt"),
      ("https://archive.org/details/item", "https://archive.org/metadata/item"),
      ("https://archive.org/details/item/test.txt", "https://archive.org/metadata/item"),
      ("https://github.com/o/r/blob/main/a%20b.txt?raw=true", "https://github.com/o/r/raw/main/a%20b.txt"),
      ("https://github.com/o/r/raw/refs/heads/main/a.txt", "https://github.com/o/r/raw/refs/heads/main/a.txt"),
      ("https://raw.githubusercontent.com/o/r/main/a.txt", "https://raw.githubusercontent.com/o/r/main/a.txt"),
      ("https://github.com/o/r/releases/download/v1/a.zip", "https://github.com/o/r/releases/download/v1/a.zip"),
      ("https://github.com/o/r/releases/latest/download/a.zip", "https://github.com/o/r/releases/latest/download/a.zip"),
    ] {
      assert_eq!(plan(input, Export::Pdf).map(|p| p.url.to_string()).as_deref(), Some(want), "{input}");
    }
    for input in [
      "https://archive.org/download/item/folder/",
      "https://archive.org/download/item/file.zip/",
      "https://pixeldrain.com/u/",
      "https://pixeldrain.com/l/abc",
      "https://pixeldrain.com/u/abc/extra",
      "https://disk.yandex.ru/client/disk",
      "https://yadi.sk/d/",
      "https://disk.yandex.ru.evil.test/d/abc",
      "https://mediafire.com/folder/abc",
      "https://mediafire.com/file/",
      "https://mediafire.com/?",
      "https://archive.org/details/",
      "https://archive.org/download/item",
      "https://archive.org/metadata/item",
      "https://github.com/o/r",
      "https://github.com/o/r/blob/main",
      "https://github.com/o/r/releases/tag/v1",
      "https://raw.githubusercontent.com/o/r/main",
      "https://github.com.evil.test/o/r/blob/main/a.txt",
      "https://user@pixeldrain.com/u/abc",
      "https://mediafire.com:443/file/abc",
    ] {
      assert!(plan(input, Export::Pdf).is_none(), "{input}");
    }
  }
  #[test]
  fn api_responses_validate_download_origins_and_file_selection() {
    for (share, body, code) in [
      (
        "https://pixeldrain.com/u/abc",
        r#"{"id":"other","name":"a.txt"}"#,
        "fileNotFile",
      ),
      ("https://pixeldrain.com/u/abc", "not JSON", "fileNotFile"),
      (
        "https://pixeldrain.com/u/abc",
        r#"{"id":"abc","can_download":false}"#,
        "fileNotPublic",
      ),
      ("https://yadi.sk/d/abc", r#"{"type":"dir"}"#, "fileNotFile"),
      (
        "https://archive.org/details/item",
        r#"{"files":[{"name":"../a.txt","source":"original"}]}"#,
        "fileNotFile",
      ),
      (
        "https://archive.org/details/item/a.txt",
        r#"{"files":[{"name":"b.txt","source":"original"}]}"#,
        "fileNotFile",
      ),
    ] {
      let mut p = plan(share, Export::Pdf).unwrap();
      assert_eq!(
        p.next_api(&p.url.clone(), body).unwrap_err().code(),
        code,
        "{share}"
      );
    }
    for href in [
      "http://downloader.disk.yandex.ru/disk/a",
      "https://127.0.0.1/a",
      "https://downloader.disk.yandex.ru.evil.test/a",
      "https://user@downloader.disk.yandex.ru/a",
    ] {
      let mut p = plan("https://yadi.sk/d/abc", Export::Pdf).unwrap();
      let page =
        Url::parse("https://cloud-api.yandex.net/v1/disk/public/resources/download?public_key=abc")
          .unwrap();
      let body = serde_json::json!({"href":href}).to_string();
      assert!(
        matches!(p.next_api(&page, &body), Err(Error::Address)),
        "{href}"
      );
    }
    let mut p = plan(
      "https://archive.org/details/item/folder/a%20b.json",
      Export::Pdf,
    )
    .unwrap();
    let url = p.next_api(&p.url.clone(), r#"{"files":[{"name":"folder/a b.json","source":"original","size":"7"},{"name":"other.txt","source":"original"}]}"#).unwrap();
    assert_eq!(
      url.as_str(),
      "https://archive.org/download/item/folder/a%20b.json"
    );
    assert_eq!(p.name.as_deref(), Some("a b.json"));
    assert_eq!(p.size, Some(7));
    let p = plan("https://mediafire.com/file/abc", Export::Pdf).unwrap();
    for html in [
      "<a id=downloadButton href='https://mediafire.com.evil.test/a.txt'>Download</a>",
      "<a id=downloadButton href='http://download123.mediafire.com/a.txt'>Download</a>",
      "<a href='https://download123.mediafire.com/ad.txt'>Advertisement</a>",
    ] {
      assert!(matches!(p.next(&p.url, html), Err(Error::NotFile)));
    }
  }
  #[test]
  fn mediafire_scrambled_downloads_keep_the_download_host_boundary() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let p = plan("https://mediafire.com/file/abc", Export::Pdf).unwrap();
    for (id, href, scrambled, expected) in [
      (
        "downloadButton",
        None,
        STANDARD.encode("https://download123.mediafire.com/token/a.txt?x=1&y=2"),
        Some("https://download123.mediafire.com/token/a.txt?x=1&y=2"),
      ),
      (
        "downloadButton",
        Some("https://download123.mediafire.com/href.txt"),
        STANDARD.encode("https://download123.mediafire.com/other.txt"),
        Some("https://download123.mediafire.com/href.txt"),
      ),
      (
        "downloadButton",
        Some("javascript:void(0)"),
        STANDARD.encode("https://download123.mediafire.com/junk-href.txt"),
        Some("https://download123.mediafire.com/junk-href.txt"),
      ),
      (
        "downloadButton",
        Some("#"),
        STANDARD
          .encode("https://download123.mediafire.com/a.txt1")
          .trim_end_matches('=')
          .to_string(),
        Some("https://download123.mediafire.com/a.txt1"),
      ),
      ("downloadButton", None, "%%%".into(), None),
      ("downloadButton", None, STANDARD.encode([0xff]), None),
      (
        "downloadButton",
        None,
        STANDARD.encode("http://download123.mediafire.com/a.txt"),
        None,
      ),
      (
        "downloadButton",
        None,
        STANDARD.encode("https://download123.mediafire.com.evil.test/a.txt"),
        None,
      ),
      (
        "downloadButton",
        None,
        STANDARD.encode("https://user@download123.mediafire.com/a.txt"),
        None,
      ),
      (
        "downloadButton",
        None,
        STANDARD.encode("https://www.mediafire.com/a.txt"),
        None,
      ),
      (
        "advertisement",
        None,
        STANDARD.encode("https://download123.mediafire.com/a.txt"),
        None,
      ),
    ] {
      let href = href.map(|v| format!(" href='{v}'")).unwrap_or_default();
      let html = format!("<a id='{id}'{href} data-scrambled-url='{scrambled}'>Download</a>");
      match (p.next(&p.url, &html), expected) {
        (Ok(url), Some(expected)) => assert_eq!(url.as_str(), expected, "{html}"),
        (Err(Error::NotFile), None) => {}
        (actual, expected) => panic!("{html}: got {actual:?}, expected {expected:?}"),
      }
    }
  }
  #[test]
  fn share_host_rules() {
    let u = |s: &str| Url::parse(s).unwrap();
    assert!(is_share("https://drive.google.com/file/d/abc/view"));
    assert!(!is_share("https://example.com/file/d/abc/view"));
    for host in [
      "downloader.disk.yandex.ru",
      "downloader.disk.yandex.com",
      "downloader.disk.yandex.net",
    ] {
      assert!(
        relax_rate_limit(&format!("https://{host}/disk/token/file.txt")),
        "{host}"
      );
    }
    assert!(relax_rate_limit("https://www.dropbox.com/s/abc/f.txt"));
    assert!(relax_rate_limit("https://uc1.dl.dropboxusercontent.com/f"));
    assert!(!relax_rate_limit("https://example.com/f"));
    assert!(sign_in_redirect(
      &u("https://drive.google.com/uc?id=abc"),
      &u("https://accounts.google.com/ServiceLogin")
    ));
    assert!(sign_in_redirect(
      &u("https://github.com/o/r/raw/main/a.txt"),
      &u("https://github.com/login?return_to=private")
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
