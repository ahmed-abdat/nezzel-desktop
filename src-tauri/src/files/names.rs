//! Content-Disposition naming, following Zajil's precedence and quoted-string design.
//! Zajil Download Manager (motalim-ai contributors), GPL-3.0-or-later; see Zajil-LICENSE.
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::Url;

use super::sanitize::sanitize_file_name;

pub(crate) fn percent_bytes(raw: &str) -> Vec<u8> {
  let mut out = Vec::new();
  let mut bytes = raw.as_bytes().iter().copied();
  while let Some(b) = bytes.next() {
    if b == b'%' {
      let (a, b) = (bytes.next(), bytes.next());
      if let (Some(a), Some(b)) = (a, b) {
        if let (Some(a), Some(b)) = ((a as char).to_digit(16), (b as char).to_digit(16)) {
          out.push((a * 16 + b) as u8);
          continue;
        }
        out.extend_from_slice(&[b'%', a, b]);
      } else {
        out.push(b'%');
        out.extend(a);
        out.extend(b);
      }
    } else {
      out.push(b);
    }
  }
  out
}
fn charset(bytes: Vec<u8>, name: &str) -> Option<String> {
  match name.to_ascii_lowercase().as_str() {
    "utf-8" | "utf8" => String::from_utf8(bytes).ok(),
    "iso-8859-1" | "latin1" => Some(bytes.into_iter().map(char::from).collect()),
    _ => None,
  }
}
fn legacy(raw: &str) -> String {
  if let Some(encoded) = raw.strip_prefix("=?").and_then(|s| s.strip_suffix("?=")) {
    let parts: Vec<_> = encoded.splitn(3, '?').collect();
    if parts.len() == 3 {
      let bytes = if parts[1].eq_ignore_ascii_case("b") {
        STANDARD.decode(parts[2]).ok()
      } else if parts[1].eq_ignore_ascii_case("q") {
        Some(percent_bytes(&parts[2].replace('_', " ").replace('=', "%")))
      } else {
        None
      };
      if let Some(decoded) = bytes.and_then(|b| charset(b, parts[0])) {
        return decoded;
      }
    }
  }
  String::from_utf8(percent_bytes(raw)).unwrap_or_else(|_| raw.to_owned())
}
pub fn disposition_name(value: &[u8]) -> Option<String> {
  let raw = String::from_utf8(value.to_vec())
    .unwrap_or_else(|_| value.iter().copied().map(char::from).collect());
  let mut parts = Vec::new();
  let mut start = 0;
  let mut quoted = false;
  let mut escaped = false;
  for (i, c) in raw.char_indices() {
    if escaped {
      escaped = false;
    } else if quoted && c == '\\' {
      escaped = true;
    } else if c == '"' {
      quoted = !quoted;
    } else if c == ';' && !quoted {
      parts.push(&raw[start..i]);
      start = i + 1;
    }
  }
  parts.push(&raw[start..]);
  let (mut plain, mut extended) = (None, None);
  for param in parts.into_iter().skip(1) {
    let Some((name, raw)) = param.split_once('=') else {
      continue;
    };
    let raw = raw.trim();
    if name.trim().eq_ignore_ascii_case("filename*") {
      if let Some((enc, rest)) = raw.split_once('\'') {
        if let Some((_, v)) = rest.split_once('\'') {
          extended = extended.or_else(|| charset(percent_bytes(v), enc).filter(|s| !s.is_empty()));
        }
      }
    } else if name.trim().eq_ignore_ascii_case("filename") {
      let raw = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
      let mut decoded = String::new();
      let mut chars = raw.chars().peekable();
      while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(|c| matches!(c, '"' | '\\')) {
          if let Some(c) = chars.next() {
            decoded.push(c);
          }
        } else {
          decoded.push(c);
        }
      }
      plain = plain.or_else(|| (!decoded.is_empty()).then(|| legacy(&decoded)));
    }
  }
  extended.or(plain)
}
pub fn suggested_name(disposition: Option<&[u8]>, url: &Url, mime: &str) -> String {
  let raw = disposition
    .and_then(disposition_name)
    .or_else(|| {
      url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .map(legacy)
    })
    .unwrap_or_else(|| url.host_str().unwrap_or("download").into());
  let name = sanitize_file_name(&raw);
  if std::path::Path::new(&name).extension().is_none() {
    let ext = match mime {
      "application/pdf" => Some("pdf"),
      "application/zip" => Some("zip"),
      "text/plain" => Some("txt"),
      "text/csv" => Some("csv"),
      "application/json" => Some("json"),
      "application/epub+zip" => Some("epub"),
      "image/jpeg" => Some("jpg"),
      "image/png" => Some("png"),
      _ => None,
    };
    if let Some(ext) = ext {
      return sanitize_file_name(&format!("{name}.{ext}"));
    }
  }
  name
}
