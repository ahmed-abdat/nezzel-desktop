//! Scrubber for anything that may leave the app in a problem report.
//! Builds on the engine's `redact_diagnostic` (URLs, credentials, paths, extractor IDs)
//! and adds emails, bearer/JWT/long tokens, secret-looking `key=value` pairs, IPs and
//! the local user name. Over-redaction is preferred to a leak.

use crate::parsers::ytdlp_error::redact_diagnostic;
use regex::Regex;
use std::sync::LazyLock;

/// Scrub one or more lines. `names` are local account names (see [`local_names`]).
pub fn scrub(text: &str, names: &[String]) -> String {
  static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[a-z0-9._%+-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)+").expect("valid email regex")
  });
  static COOKIE_ROW: LazyLock<Regex> = LazyLock::new(|| {
    // Netscape cookies.txt rows: domain, flag, path, secure, expiry, name, value.
    Regex::new(r"(?m)^[^\t\r\n]*\t(?:TRUE|FALSE)\t[^\r\n]*$").expect("valid cookie row regex")
  });
  static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:bearer|basic|token)\s+[A-Za-z0-9._~+/=-]{8,}")
      .expect("valid bearer regex")
  });
  static SECRET_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
      r#"(?i)\b(?P<key>[a-z0-9_-]*(?:token|secret|passw(?:or)?d|pwd|api[_-]?key|apikey|signature|sig|session|sid|auth|key|credential)s?)["']?\s*[:=]\s*["']?[^\s"',;&]+"#,
    )
    .expect("valid secret pair regex")
  });
  static JWT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)?")
      .expect("valid JWT regex")
  });
  static LONG_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    // Hashes, cookie values, API keys: long runs with digits and letters mixed.
    Regex::new(r"\b[A-Za-z0-9_+/=-]{32,}").expect("valid long token regex")
  });
  static PATH_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\[redacted path\][^\r\n"']*"#).expect("valid path tail regex"));
  static IPV4: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid IPv4 regex"));

  static OTHER_SCHEME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:data|magnet|blob|file|wss?|rtmps?|rtsp|mms|sftp|smb|ftps?|mailto|tel|sms):[^\s<>"']+"#)
      .expect("valid scheme regex")
  });
  static IPV6: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?P<pre>^|[\s\[(=@,"'])(?P<ip>(?:[0-9a-f]{0,4}:){2,7}[0-9a-f]{0,4})(?P<zone>%[0-9a-z]+)?(?P<post>$|[\s\]),"'/])"#)
      .expect("valid IPv6 regex")
  });
  static LAN_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9][a-z0-9-]*(?:\.[a-z0-9-]+)*\.(?:local|lan|home|internal|localdomain|intranet|corp)\b")
      .expect("valid LAN host regex")
  });
  static HOST_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?P<key>host(?:name)?|server(?:[ _-]?name)?|computer(?:[ _-]?name)?|machine(?:[ _-]?name)?|device[ _-]?name)\s*[:=]\s*["']?[^\s"',;]+"#)
      .expect("valid host pair regex")
  });

  // Undo JSON and percent encoding first, so encoded links can't slip past the URL rules.
  let text = canonicalize(text);
  let text = OTHER_SCHEME.replace_all(&text, "[redacted URL]");
  let text = COOKIE_ROW.replace_all(&text, "[redacted cookie]");
  let text = EMAIL.replace_all(&text, "[redacted email]");
  let text = redact_diagnostic(&text);
  // A file name may contain `]`: never keep what follows a redacted path on that line.
  let text = PATH_TAIL.replace_all(&text, "[redacted path]");
  let text = JWT.replace_all(&text, "[redacted token]");
  let text = BEARER.replace_all(&text, "[redacted token]");
  let text = SECRET_PAIR.replace_all(&text, "${key}=[redacted]");
  let text = LONG_TOKEN.replace_all(&text, |caps: &regex::Captures<'_>| {
    let value = &caps[0];
    let digits = value.chars().filter(char::is_ascii_digit).count();
    let letters = value.chars().filter(char::is_ascii_alphabetic).count();
    if digits > 0 && letters > 0 {
      "[redacted token]".to_string()
    } else {
      value.to_string()
    }
  });
  let text = IPV4.replace_all(&text, "[redacted IP]");
  let text = IPV6.replace_all(&text, |caps: &regex::Captures<'_>| {
    let ip = &caps["ip"];
    let looks_like_ip = (ip.matches(':').count() >= 3 || ip.contains("::"))
      && ip.chars().any(|c| c.is_ascii_hexdigit());
    if looks_like_ip {
      format!("{}[redacted IP]{}", &caps["pre"], &caps["post"])
    } else {
      caps[0].to_string()
    }
  });
  let text = HOST_PAIR.replace_all(&text, "${key}=[redacted]");
  let mut text = LAN_HOST.replace_all(&text, "[redacted host]").into_owned();
  for name in names.iter().filter(|name| !name.trim().is_empty()) {
    text = if name.chars().count() >= 3 {
      replace_word_ci(&text, name, "[user]")
    } else {
      // Short names ("al", "jo") are ordinary words: replace them only where a name is expected.
      replace_in_context(&text, name)
    };
  }
  text
}

/// JSON escapes (`\/`, `\u002F`) and percent-encoding (up to double-encoded) back to plain text.
fn canonicalize(text: &str) -> String {
  static UNICODE_ESCAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\u([0-9a-fA-F]{4})").expect("valid unicode escape regex"));
  let mut out = text.replace("\\/", "/");
  out = UNICODE_ESCAPE
    .replace_all(&out, |caps: &regex::Captures<'_>| {
      u32::from_str_radix(&caps[1], 16)
        .ok()
        .and_then(char::from_u32)
        .map_or_else(|| caps[0].to_string(), String::from)
    })
    .into_owned();
  for _ in 0..2 {
    if !out.contains('%') {
      break;
    }
    out = percent_decode(&out);
  }
  out
}

fn percent_decode(text: &str) -> String {
  let bytes = text.as_bytes();
  let mut out = Vec::with_capacity(bytes.len());
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%' && i + 2 < bytes.len() {
      let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
      if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
        out.push(value);
        i += 3;
        continue;
      }
    }
    out.push(bytes[i]);
    i += 1;
  }
  String::from_utf8_lossy(&out).into_owned()
}

fn replace_in_context(haystack: &str, name: &str) -> String {
  let pattern = format!(
    r#"(?i)(?P<pre>\b(?:user(?:name)?|login|account|owner|as|by|for|from)\s*[:=]?\s*["']?|~|[\\/](?:Users|home)[\\/]){}\b"#,
    regex::escape(name)
  );
  match Regex::new(&pattern) {
    Ok(regex) => regex.replace_all(haystack, "${pre}[user]").into_owned(),
    Err(_) => haystack.to_string(),
  }
}

/// The account names that would identify the person: `$USER`/`$USERNAME` and the home folder name.
pub fn local_names() -> Vec<String> {
  static NAMES: LazyLock<Vec<String>> = LazyLock::new(collect_local_names);
  NAMES.clone()
}

/// Account names plus this computer's names (host name, macOS computer name).
fn collect_local_names() -> Vec<String> {
  let mut names: Vec<String> = ["USER", "USERNAME", "LOGNAME"]
    .iter()
    .filter_map(|key| std::env::var(key).ok())
    .collect();
  if let Some(home) = dirs::home_dir() {
    if let Some(name) = home.file_name().and_then(|name| name.to_str()) {
      names.push(name.to_string());
    }
  }
  names.extend(machine_names());
  names.retain(|name| !name.trim().is_empty());
  names.sort();
  names.dedup();
  names
}

fn machine_names() -> Vec<String> {
  let mut names: Vec<String> = ["COMPUTERNAME", "HOSTNAME"]
    .iter()
    .filter_map(|key| std::env::var(key).ok())
    .collect();
  if let Ok(text) = std::fs::read_to_string("/etc/hostname") {
    names.push(text.trim().to_string());
  }
  #[cfg(unix)]
  {
    let run = |program: &str, args: &[&str]| {
      std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    names.extend(run("hostname", &[]));
    #[cfg(target_os = "macos")]
    {
      names.extend(run("scutil", &["--get", "ComputerName"]));
      names.extend(run("scutil", &["--get", "LocalHostName"]));
    }
  }
  // "sara-mbp.local" → also "sara-mbp".
  let short: Vec<String> = names
    .iter()
    .filter_map(|name| name.split('.').next().map(str::to_string))
    .collect();
  names.extend(short);
  names
}

/// Replace `needle` case-insensitively where it is not part of a longer word.
fn replace_word_ci(haystack: &str, needle: &str, with: &str) -> String {
  let pattern = format!(r"(?i)(?:^|\b){}(?:\b|$)", regex::escape(needle));
  match Regex::new(&pattern) {
    Ok(regex) => regex.replace_all(haystack, with).into_owned(),
    Err(_) => haystack.to_string(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn names() -> Vec<String> {
    vec!["samira".into()]
  }

  #[test]
  fn scrub_removes_private_data_from_real_looking_lines() {
    for (line, secret) in [
      ("2026-11-04T10:00:00Z  WARN nezzel_lib::runners: failed https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL1", "youtube.com"),
      ("[download] Destination: /Users/samira/Movies/Nezzel/My Holiday Video [abc].mp4", "Holiday"),
      ("[download] /home/sara/Videos/Song [Official] Remix.webm has already been downloaded", "Remix"),
      ("[Merger] Merging formats into \"/home/sara/Videos/فيديو العائلة.mkv\"", "العائلة"),
      (r"ERROR: unable to open C:\Users\Fatima\Downloads\رحلة الصيف.mp4", "رحلة"),
      ("تعذر التنزيل من https://www.instagram.com/reel/Cx1/?igsh=abc للمستخدم", "instagram.com"),
      ("contact me at sara.ahmed+test@gmail.com please", "gmail.com"),
      ("Cookie: sessionid=31337abc; csrftoken=zz", "31337abc"),
      ("Authorization: Bearer ya29.a0AfH6SMBx-token-value", "ya29"),
      ("proxy socks5://bob:hunter2@10.0.0.5:1080 refused", "hunter2"),
      ("proxy 192.168.1.20:8080 refused", "192.168.1.20"),
      ("signed url param sig=AbC123xyz expires", "AbC123xyz"),
      ("api_key=sk_live_51HxYz secret", "sk_live_51HxYz"), // gitleaks:allow (fake fixture)
      ("jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig in header", "eyJhbGci"),
      ("hash 3f786850e387550fdab836ed7e6dc881de23001b mismatch", "3f786850e387"),
      (".youtube.com\tTRUE\t/\tTRUE\t1735689600\tSID\tg.a000secret", "g.a000secret"),
      ("[youtube] dQw4w9WgXcQ: Downloading webpage", "dQw4w9WgXcQ"),
      ("fetch https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Dsecret1 failed", "youtube"),
      ("fetch https%253A%252F%252Fvimeo.com%252F12345 failed", "vimeo"),
      (r#"{"url":"https:\/\/www.instagram.com\/reel\/Cx1\/"}"#, "instagram"),
      (r#"{"url":"https:\u002F\u002Fx.com\u002Fsara_private"}"#, "sara_private"),
      ("thumbnail data:image/png;base64,iVBORw0KGgo= is broken", "iVBOR"),
      ("queued magnet:?xt=urn:btih:abcdef&dn=Family.Film", "Family"),
      ("connect to 2001:db8::1 failed", "2001:db8"),
      ("proxy [fe80::1%en0]:8080 refused", "fe80"),
      ("resolve nas.local failed", "nas.local"),
      ("hostname=Saras-MacBook-Pro refused", "Saras"),
      ("running as samira on this Mac", "samira"),
      ("instagram.com/p/DfQIowKutL3/ unavailable", "DfQIowKutL3"),
      ("file:///Users/samira/Desktop/x.html failed", "Desktop"),
    ] {
      let redacted = scrub(line, &names());
      assert!(!redacted.contains(secret), "{line:?} -> {redacted:?}");
    }
  }

  #[test]
  fn scrub_keeps_useful_diagnostics() {
    for (line, keep) in [
      (
        "ERROR: [youtube] Sign in to confirm you are not a bot",
        "Sign in to confirm you are not a bot",
      ),
      ("HTTP Error 403: Forbidden", "HTTP Error 403: Forbidden"),
      (
        "yt-dlp 2026.10.26, ffmpeg 7.1",
        "yt-dlp 2026.10.26, ffmpeg 7.1",
      ),
      ("فشل التنزيل: الرمز 403", "فشل التنزيل: الرمز 403"),
      (
        "Download transport completed transport=\"native\"",
        "transport=\"native\"",
      ),
      (
        "panicked at src/runners/ytdlp_download.rs:12:5",
        "src/runners/ytdlp_download.rs:12:5",
      ),
      ("macOS 15.6 aarch64", "macOS 15.6 aarch64"),
      (
        "2026-11-04T10:00:00.123Z  INFO nezzel_lib::scheduling::download_pipeline: ok",
        "2026-11-04T10:00:00.123Z  INFO nezzel_lib::scheduling::download_pipeline: ok",
      ),
      (
        "at <Vec<T> as core::ops::Add>::add 10:00:00",
        "<Vec<T> as core::ops::Add>::add 10:00:00",
      ),
      ("progress 50% done, 100%", "progress 50% done, 100%"),
    ] {
      let redacted = scrub(line, &names());
      assert!(redacted.contains(keep), "{line:?} -> {redacted:?}");
    }
  }

  #[test]
  fn user_name_only_replaced_as_a_word() {
    let names = vec!["sam".to_string()];
    assert_eq!(scrub("user sam samples", &names), "user [user] samples");
    // Too-short names would mangle ordinary words.
    assert_eq!(scrub("al is here", &["al".to_string()]), "al is here");
    // …but replaced where a name is expected.
    assert_eq!(
      scrub("logged in as jo, user=jo", &["jo".to_string()]),
      "logged in as [user], user=[user]"
    );
  }
}
