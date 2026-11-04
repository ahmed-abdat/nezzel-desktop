//! Segment and validator policy; adapted from Zajil (GPL-3.0-or-later).
use super::{Error, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
pub const MIN_SEGMENT: u64 = 1024 * 1024;
pub const DEFAULT_CONNECTIONS: usize = 8;
pub const MAX_CONNECTIONS: usize = 32;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Segment {
  pub start: u64,
  pub end: Option<u64>,
  pub done: u64,
}
impl Segment {
  pub fn remaining(&self) -> Option<u64> {
    self.end.map(|end| end - self.start - self.done)
  }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Validator {
  pub etag: Option<String>,
  pub modified: Option<String>,
  pub date: Option<String>,
}
impl Validator {
  pub fn from_headers(h: &reqwest::header::HeaderMap) -> Self {
    let v = |k| h.get(k).and_then(|h| h.to_str().ok()).map(str::to_owned);
    Self {
      etag: v("etag"),
      modified: v("last-modified"),
      date: v("date"),
    }
  }
  pub fn if_range(&self) -> Option<&str> {
    if let Some(tag) = self.etag.as_deref() {
      if tag.len() >= 2 && tag.starts_with('"') && tag.ends_with('"') {
        return Some(tag);
      }
      return None;
    }
    let modified = httpdate::parse_http_date(self.modified.as_deref()?).ok()?;
    let date = httpdate::parse_http_date(self.date.as_deref()?).ok()?;
    (date.duration_since(modified).ok()? >= Duration::from_secs(1))
      .then_some(self.modified.as_deref()?)
  }
  pub fn matches(&self, other: &Self) -> bool {
    match (self.if_range(), other.if_range()) {
      (Some(a), Some(b)) => a == b,
      _ => false,
    }
  }
}
pub(crate) fn segments(size: Option<u64>, connections: usize, parallel: bool) -> Vec<Segment> {
  let n = if parallel {
    size.map_or(1, |n| {
      (n / MIN_SEGMENT)
        .min(connections.clamp(1, MAX_CONNECTIONS) as u64)
        .max(1) as usize
    })
  } else {
    1
  };
  let chunk = size.unwrap_or(0) / n as u64;
  (0..n)
    .map(|i| Segment {
      start: i as u64 * chunk,
      end: size.map(|size| {
        if i + 1 == n {
          size
        } else {
          (i + 1) as u64 * chunk
        }
      }),
      done: 0,
    })
    .collect()
}
pub(crate) fn content_range(value: &str) -> Result<(u64, u64, u64)> {
  let (range, total) = value
    .strip_prefix("bytes ")
    .and_then(|v| v.split_once('/'))
    .ok_or(Error::Restart)?;
  let (start, end) = range.split_once('-').ok_or(Error::Restart)?;
  let (start, end, total) = (
    start.parse().map_err(|_| Error::Restart)?,
    end.parse::<u64>().map_err(|_| Error::Restart)?,
    total.parse().map_err(|_| Error::Restart)?,
  );
  if start > end || end >= total {
    return Err(Error::Restart);
  }
  Ok((start, end, total))
}
pub(crate) fn backoff(failures: u32) -> Duration {
  let ceiling = (500u64.saturating_mul(1u64 << failures.min(6))).min(30_000);
  Duration::from_millis(ceiling / 2 + rand::random::<u64>() % (ceiling / 2 + 1))
}
pub(crate) fn retry_after(value: Option<&str>, now: SystemTime) -> Result<Option<Duration>> {
  let Some(value) = value else { return Ok(None) };
  let delay = value
    .trim()
    .parse::<u64>()
    .ok()
    .map(Duration::from_secs)
    .or_else(|| {
      httpdate::parse_http_date(value)
        .ok()
        .map(|t| t.duration_since(now).unwrap_or_default())
    });
  if delay.is_some_and(|d| d > Duration::from_secs(600)) {
    return Err(Error::Network);
  }
  Ok(delay)
}
#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn validators_require_strong_etag_or_last_modified_older_than_date() {
    for (tag, modified, date, expected) in [
      (Some("\"a\""), None, None, Some("\"a\"")),
      (
        Some("W/\"a\""),
        Some("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some("Sun, 06 Nov 1994 08:49:39 GMT"),
        None,
      ),
      (
        None,
        Some("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some("Sun, 06 Nov 1994 08:49:38 GMT"),
        Some("Sun, 06 Nov 1994 08:49:37 GMT"),
      ),
      (None, Some("Sun, 06 Nov 1994 08:49:37 GMT"), None, None),
    ] {
      assert_eq!(
        Validator {
          etag: tag.map(str::to_owned),
          modified: modified.map(str::to_owned),
          date: date.map(str::to_owned)
        }
        .if_range(),
        expected
      );
    }
  }
}

#[cfg(test)]
mod range_policy_tests {
  use super::*;
  #[test]
  fn ranges_and_retry_after_are_bounded() {
    for bad in [
      "bytes 1-0/3",
      "bytes 0-3/3",
      "bytes 0-1/*",
      "bytes */3",
      "bytes -1-2/3",
    ] {
      assert!(content_range(bad).is_err());
    }
    assert_eq!(content_range("bytes 5-9/10").unwrap(), (5, 9, 10));
    for size in [1, MIN_SEGMENT, 2 * MIN_SEGMENT, 100 * MIN_SEGMENT] {
      let planned = segments(Some(size), 999, true);
      assert!(planned.len() <= MAX_CONNECTIONS);
      assert_eq!(planned.first().unwrap().start, 0);
      assert_eq!(planned.last().unwrap().end, Some(size));
      assert!(planned.windows(2).all(|s| s[0].end == Some(s[1].start)));
    }
    let now = SystemTime::UNIX_EPOCH;
    assert_eq!(
      retry_after(Some("600"), now).unwrap(),
      Some(Duration::from_secs(600))
    );
    assert!(retry_after(Some("601"), now).is_err());
  }
}
