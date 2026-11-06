//! Chromium's cookie database layout (Chrome, Edge, Brave, ...), read for sign-in hints only.
//! Chromium cookie values are exported by yt-dlp (`exporter`), never decrypted here.
use super::hint::HintSchema;
use crate::runners::browser_logins::Os;
use std::collections::HashSet;

/// Seconds from 1601-01-01 (Chromium's clock) to the Unix epoch.
const CHROMIUM_EPOCH: i64 = 11_644_473_600;

pub(super) const HINT_SCHEMA: HintSchema = HintSchema {
  table: "cookies",
  host_column: "host_key",
  expiry: "expires_utc",
  access: "last_access_utc",
  app_bound,
  expired,
  access_micros,
};

/// Windows Chromium v20 values are bound to the browser app and cannot be exported.
fn app_bound(os: Os, columns: &HashSet<String>) -> &'static str {
  if matches!(os, Os::Windows) && columns.contains("encrypted_value") {
    "COALESCE(substr(encrypted_value, 1, 3) = x'763230', 0)"
  } else {
    "0"
  }
}

/// Microseconds since 1601; 0 is a session cookie, which never counts as expired.
fn expired(expires: i64, now: i64) -> bool {
  let expires = if expires == 0 {
    0
  } else {
    expires / 1_000_000 - CHROMIUM_EPOCH
  };
  expires != 0 && expires <= now
}

fn access_micros(access: i64) -> i64 {
  access.saturating_sub(CHROMIUM_EPOCH * 1_000_000)
}
