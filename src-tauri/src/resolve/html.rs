//! Small bounded HTML tokenizer. Raw script/style text and comments never become page metadata.
use super::Resolution;
use regex::Regex;
use reqwest::Url;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

static YOUTUBE_URL: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"(?i)(?:https?:)?//(?:www\.|m\.)?(?:youtube(?:-nocookie)?\.com/(?:embed/|watch\?)|youtu\.be/)[^\s\"'<>`\\]*"#)
    .expect("static YouTube URL grammar")
});

pub(crate) struct Tag<'a> {
  pub name: String,
  pub attrs: BTreeMap<String, String>,
  pub text: &'a str,
  pub closing: bool,
}

pub(super) fn tags(html: &str) -> Option<Vec<Tag<'_>>> {
  Some(
    all_tags(html)?
      .into_iter()
      .filter(|tag| {
        matches!(
          tag.name.as_str(),
          "script" | "meta" | "video" | "source" | "form"
        )
      })
      .collect(),
  )
}

/// Structural tokens for post readers. Comments and raw-text contents remain opaque.
pub(crate) fn all_tags(html: &str) -> Option<Vec<Tag<'_>>> {
  if html.len() > super::http::FACEBOOK_CAP {
    return None;
  }
  let lower = html.to_ascii_lowercase();
  let bytes = html.as_bytes();
  let mut pos = 0;
  let mut count = 0;
  let mut scripts = 0;
  let mut result = Vec::new();
  while let Some(offset) = html[pos..].find('<') {
    pos += offset;
    if html[pos..].starts_with("<!--") {
      pos = html[pos + 4..]
        .find("-->")
        .map(|n| pos + 7 + n)
        .unwrap_or(html.len());
      continue;
    }
    let start = pos + 1;
    let closing = bytes.get(start) == Some(&b'/');
    let name_start = start + usize::from(closing);
    let mut end_name = name_start;
    while bytes
      .get(end_name)
      .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
      end_name += 1;
    }
    if name_start == end_name {
      pos += 1;
      continue;
    }
    let name = &lower[name_start..end_name];
    let mut end = end_name;
    let mut quote = None;
    let mut value = false;
    while end < bytes.len() {
      let b = bytes[end];
      match quote {
        Some(q) if b == q => quote = None,
        None if value && (b == b'\'' || b == b'"') => {
          quote = Some(b);
          value = false;
        }
        None if b == b'>' => break,
        None if b == b'=' => value = true,
        None if !b.is_ascii_whitespace() => value = false,
        _ => {}
      }
      end += 1;
    }
    if end == bytes.len() {
      break;
    }
    count += 1;
    if count > 65_536 {
      return None;
    }
    let attrs = attributes(&html[end_name..end]);
    pos = end + 1;
    let mut text = &html[pos..html[pos..].find('<').map_or(html.len(), |n| pos + n)];
    if !closing
      && matches!(
        name,
        "script"
          | "style"
          | "textarea"
          | "title"
          | "xmp"
          | "iframe"
          | "noembed"
          | "noframes"
          | "plaintext"
      )
    {
      let marker = format!("</{name}");
      let stop = (name != "plaintext")
        .then(|| lower[pos..].find(&marker).map(|n| pos + n))
        .flatten()
        .unwrap_or(html.len());
      text = &html[pos..stop];
      pos = lower[stop..]
        .find('>')
        .map(|n| stop + n + 1)
        .unwrap_or(html.len());
      if name == "script" {
        scripts += 1;
      }
      if scripts > 1024 {
        return None;
      }
    }
    result.push(Tag {
      name: name.into(),
      attrs,
      text,
      closing,
    });
  }
  Some(result)
}

fn attributes(input: &str) -> BTreeMap<String, String> {
  let bytes = input.as_bytes();
  let mut pos = 0;
  let mut result = BTreeMap::new();
  while pos < bytes.len() {
    while bytes
      .get(pos)
      .is_some_and(|b| b.is_ascii_whitespace() || *b == b'/')
    {
      pos += 1;
    }
    let start = pos;
    while bytes
      .get(pos)
      .is_some_and(|b| !b.is_ascii_whitespace() && !b"=/<>\"'".contains(b))
    {
      pos += 1;
    }
    if start == pos {
      pos += 1;
      continue;
    }
    let name = input[start..pos].to_ascii_lowercase();
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
      pos += 1;
    }
    let mut value = "";
    if bytes.get(pos) == Some(&b'=') {
      pos += 1;
      while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos += 1;
      }
      if let Some(q @ (b'\'' | b'"')) = bytes.get(pos).copied() {
        pos += 1;
        let start = pos;
        while bytes.get(pos).is_some_and(|b| *b != q) {
          pos += 1;
        }
        value = &input[start..pos];
        pos += usize::from(pos < bytes.len());
      } else {
        let start = pos;
        while bytes
          .get(pos)
          .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>')
        {
          pos += 1;
        }
        value = &input[start..pos];
      }
    }
    // HTML uses the first attribute when malformed input repeats one.
    result.entry(name).or_insert_with(|| entities(value));
  }
  result
}

pub(crate) fn entities(value: &str) -> String {
  let mut out = String::new();
  let mut remaining = value;
  while let Some(start) = remaining.find('&') {
    out.push_str(&remaining[..start]);
    remaining = &remaining[start..];
    // Entities are short: look at most 13 bytes ahead, so a page of bare '&' stays linear.
    let end = remaining.bytes().take(13).position(|b| b == b';');
    let decoded = end.and_then(|n| {
      let entity = &remaining[1..n];
      match entity {
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "lt" => Some('<'),
        "gt" => Some('>'),
        _ => entity
          .strip_prefix("#x")
          .or_else(|| entity.strip_prefix("#X"))
          .and_then(|n| u32::from_str_radix(n, 16).ok())
          .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse().ok()))
          .and_then(char::from_u32),
      }
    });
    if let (Some(end), Some(c)) = (end, decoded) {
      out.push(c);
      remaining = &remaining[end + 1..];
    } else {
      out.push('&');
      remaining = &remaining[1..];
    }
  }
  out.push_str(remaining);
  out
}

pub(crate) fn metadata<'a>(tags: &'a [Tag<'_>], property: &str) -> Vec<&'a str> {
  tags
    .iter()
    .filter(|t| {
      t.name == "meta"
        && !t.closing
        && t
          .attrs
          .get("property")
          .or_else(|| t.attrs.get("name"))
          .is_some_and(|p| p.eq_ignore_ascii_case(property))
    })
    .filter_map(|t| t.attrs.get("content").map(String::as_str))
    .collect()
}

pub(crate) fn generic(html: &str, url: &str) -> Option<Resolution> {
  let page = super::http::https_url(url).ok()?;
  let tags = all_tags(html)?;
  let mut candidates = Vec::new();
  for property in ["og:video:secure_url", "og:video:url", "og:video"] {
    candidates.extend(metadata(&tags, property));
  }
  let mut in_video = false;
  for tag in &tags {
    if tag.name == "video" {
      in_video = !tag.closing;
    }
    if !tag.closing && (tag.name == "video" || (tag.name == "source" && in_video)) {
      if let Some(src) = tag.attrs.get("src") {
        candidates.push(src);
      }
    }
  }
  for candidate in candidates {
    if let Some(url) = media_url(&page, candidate) {
      return Some(Resolution::Passthrough(url));
    }
  }
  youtube_frontend(&tags, &page)
}

pub(crate) fn youtube_frontend(tags: &[Tag<'_>], page: &Url) -> Option<Resolution> {
  // Structural tokens exclude comments while retaining JS embed templates.
  let html = tags
    .iter()
    .flat_map(|tag| {
      tag
        .attrs
        .values()
        .map(String::as_str)
        .chain(std::iter::once(tag.text))
    })
    .collect::<Vec<_>>()
    .join(" ");
  // JS templates are evidence of a frontend even when their ID only exists in the page URL.
  let html = entities(&html).replace("\\/", "/");
  let lower = html.to_ascii_lowercase();
  if ![
    "youtube.com/embed",
    "youtube-nocookie.com/embed",
    "youtube.com/watch",
    "youtu.be/",
    "ytimg.com",
  ]
  .iter()
  .any(|marker| lower.contains(marker))
  {
    return None;
  }
  let ids: BTreeSet<String> = YOUTUBE_URL
    .find_iter(&html)
    .filter_map(|found| page.join(found.as_str()).ok())
    // These embed endpoints identify a playlist/channel rather than an 11-character video ID.
    .filter(|url| {
      !matches!(
        url.path().trim_end_matches('/'),
        "/embed/videoseries" | "/embed/live_stream"
      )
    })
    .filter_map(|url| {
      if url.host_str() == Some("youtu.be") {
        let id = url.path().trim_matches('/');
        youtube_id(id).then(|| id.to_owned())
      } else {
        youtube_slot(&url)
      }
    })
    .collect();
  let slot = youtube_slot(page);
  let id = match ids.len() {
    0 => slot?,
    1 => ids.into_iter().next()?,
    _ => slot.filter(|id| ids.contains(id))?,
  };
  Some(Resolution::Passthrough(format!(
    "https://www.youtube.com/watch?v={id}"
  )))
}

fn youtube_slot(url: &Url) -> Option<String> {
  if let Some((slot, id)) = url.path().trim_matches('/').split_once('/') {
    if matches!(slot, "watch" | "v" | "embed" | "shorts" | "video" | "live") && youtube_id(id) {
      return Some(id.to_owned());
    }
  }
  url
    .query_pairs()
    .find_map(|(key, id)| (key == "v" && youtube_id(&id)).then(|| id.into_owned()))
}

fn youtube_id(id: &str) -> bool {
  id.len() == 11
    && id
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

pub(crate) fn media_url(page: &Url, value: &str) -> Option<String> {
  let url = page.join(value.trim()).ok()?;
  super::http::https_url(url.as_str())
    .ok()
    .map(|url| url.into())
}

#[cfg(test)]
#[path = "generic_fixtures.rs"]
pub(crate) mod generic_fixtures;
