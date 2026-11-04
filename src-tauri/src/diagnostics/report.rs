//! Problem report text: pure builders, no I/O. Everything free-form passes through
//! [`super::redact::scrub`] before it gets here; identifiers are reduced to safe tokens.

use crate::state::config_models::Config;
use serde::{Deserialize, Serialize};

pub const LOG_LINES: usize = 50;
/// The user's own words in "Report a problem".
pub const MAX_NOTE_CHARS: usize = 2_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsInfo {
  pub name: String,
  pub version: String,
  pub arch: String,
  /// Linux only: distro, desktop and session type.
  pub distro: Option<String>,
  pub desktop: Option<String>,
  pub session: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Attempt {
  pub stage: String,
  pub login: bool,
  pub outcome: String,
  pub code: Option<String>,
  pub ms: u64,
}

/// A failed row: its error code and the stage trail. Never the URL or title.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadContext {
  pub error_code: Option<String>,
  /// Set in Rust from `link::link_for_report`, never by the webview.
  #[serde(skip_deserializing)]
  pub site: Option<String>,
  #[serde(skip_deserializing)]
  pub link: Option<String>,
  #[serde(skip_deserializing)]
  pub extractor: Option<String>,
  pub exit_code: Option<i32>,
  pub attempts: Vec<Attempt>,
}

#[derive(Debug, Clone, Default)]
pub struct ReportInput {
  /// Already-scrubbed note the user typed.
  pub note: Option<String>,
  pub app_version: String,
  pub os: OsInfo,
  /// (tool, version or "missing").
  pub tools: Vec<(String, String)>,
  /// (app language setting, system locale).
  pub locale: (String, String),
  pub settings: Vec<(&'static str, String)>,
  pub download: Option<DownloadContext>,
  /// Already-scrubbed crash text from the previous run.
  pub crash: Option<String>,
  /// Already-scrubbed recent log lines, oldest first.
  pub log_lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
  pub title: String,
  pub body: String,
}

/// Keep only characters that can't carry a URL, path or name: `[A-Za-z0-9_.:-]`, ≤ 64.
pub fn token(value: &str) -> String {
  let cleaned: String = value
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    .take(64)
    .collect();
  if cleaned.is_empty() {
    "?".into()
  } else {
    cleaned
  }
}

pub fn title(input: &ReportInput) -> String {
  if input.crash.is_some() {
    format!("Crash report (Nezzel {})", input.app_version)
  } else if let Some(code) = input
    .download
    .as_ref()
    .and_then(|d| d.error_code.as_deref())
  {
    format!("Download failed: {}", token(code))
  } else {
    "Problem report".into()
  }
}

pub fn render(input: &ReportInput) -> String {
  let mut out = String::new();
  if let Some(note) = input.note.as_deref().filter(|note| !note.trim().is_empty()) {
    out.push_str("### Note\n\n");
    out.push_str(note.trim());
    out.push_str("\n\n");
  }
  out.push_str("### Diagnostics\n\n");
  out.push_str(&format!("- App: Nezzel {}\n", input.app_version));
  out.push_str(&format!(
    "- OS: {} {} ({})\n",
    input.os.name, input.os.version, input.os.arch
  ));
  if input.os.distro.is_some() || input.os.desktop.is_some() || input.os.session.is_some() {
    let part = |value: &Option<String>| value.clone().unwrap_or_else(|| "unknown".into());
    out.push_str(&format!(
      "- Linux: {} · desktop {} · session {}\n",
      part(&input.os.distro),
      part(&input.os.desktop),
      part(&input.os.session)
    ));
  }
  out.push_str(&format!(
    "- Language: {} (system {})\n",
    input.locale.0, input.locale.1
  ));
  if !input.tools.is_empty() {
    let tools: Vec<String> = input
      .tools
      .iter()
      .map(|(tool, version)| format!("{tool} {version}"))
      .collect();
    out.push_str(&format!("- Tools: {}\n", tools.join(", ")));
  }
  if !input.settings.is_empty() {
    let settings: Vec<String> = input
      .settings
      .iter()
      .map(|(key, value)| format!("{key}={value}"))
      .collect();
    out.push_str(&format!("- Settings: {}\n", settings.join(", ")));
  }

  if let Some(download) = &input.download {
    out.push_str("\n### Failed download\n\n");
    out.push_str(&format!(
      "- Error: {}",
      download
        .error_code
        .as_deref()
        .map(token)
        .unwrap_or_else(|| "unknown".into())
    ));
    if let Some(exit) = download.exit_code {
      out.push_str(&format!(" (exit {exit})"));
    }
    out.push('\n');
    if let Some(link) = &download.link {
      out.push_str(&format!("- Link: {link}\n"));
    }
    if let Some(site) = &download.site {
      out.push_str(&format!("- Site: {site}"));
      if let Some(extractor) = &download.extractor {
        out.push_str(&format!(" (extractor {})", token(extractor)));
      }
      out.push('\n');
    }
    for (index, attempt) in download.attempts.iter().enumerate() {
      out.push_str(&format!(
        "- Attempt {}: {} · login {} · {} · {} · {} ms\n",
        index + 1,
        token(&attempt.stage),
        if attempt.login { "yes" } else { "no" },
        token(&attempt.outcome),
        attempt
          .code
          .as_deref()
          .map(token)
          .unwrap_or_else(|| "-".into()),
        attempt.ms
      ));
    }
  }

  if let Some(crash) = &input.crash {
    out.push_str("\n### Crash\n\n```\n");
    out.push_str(crash.trim_end());
    out.push_str("\n```\n");
  }

  if !input.log_lines.is_empty() {
    out.push_str("\n### Recent log (private data removed)\n\n```\n");
    for line in &input.log_lines {
      out.push_str(line.trim_end());
      out.push('\n');
    }
    out.push_str("```\n");
  }
  out
}

/// Title + body for a Sentry attachment, the newest log lines only.
pub fn build(input: &ReportInput) -> Report {
  let mut input = input.clone();
  let skip = input.log_lines.len().saturating_sub(LOG_LINES);
  input.log_lines.drain(..skip);
  Report {
    title: title(&input),
    body: render(&input),
  }
}

/// `/etc/os-release` → "NAME VERSION_ID".
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_os_release(text: &str) -> Option<String> {
  let field = |key: &str| {
    text.lines().find_map(|line| {
      let value = line.strip_prefix(key)?.strip_prefix('=')?;
      let value = value.trim().trim_matches('"').trim_matches('\'').trim();
      (!value.is_empty()).then(|| value.to_string())
    })
  };
  let name = field("NAME")?;
  Some(match field("VERSION_ID") {
    Some(version) => format!("{name} {version}"),
    None => name,
  })
}

/// Windows `ver` output → "10.0.22631.4317".
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn parse_windows_ver(text: &str) -> Option<String> {
  let start = text.find("Version ")? + "Version ".len();
  let rest = &text[start..];
  let version: String = rest
    .chars()
    .take_while(|c| c.is_ascii_digit() || *c == '.')
    .collect();
  (!version.is_empty()).then_some(version)
}

/// macOS SystemVersion.plist → ProductVersion.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn parse_macos_plist(text: &str) -> Option<String> {
  let after = &text[text.find("<key>ProductVersion</key>")?..];
  let start = after.find("<string>")? + "<string>".len();
  let end = after[start..].find("</string>")? + start;
  let version = after[start..end].trim();
  (!version.is_empty()).then(|| token(version))
}

/// Booleans and enums only: never paths, templates, proxies, cookie files or site names.
pub fn settings_summary(config: &Config) -> Vec<(&'static str, String)> {
  let defaults = Config::default();
  let flag = |value: bool| if value { "on" } else { "off" }.to_string();
  let name = |value: serde_json::Value| match value {
    serde_json::Value::String(text) => token(&text),
    other => token(&other.to_string()),
  };
  let enum_name = |value: Result<serde_json::Value, serde_json::Error>| {
    value.map(name).unwrap_or_else(|_| "?".into())
  };
  let auth = &config.auth;
  let cookies = if auth
    .cookie_file
    .as_deref()
    .is_some_and(|file| !file.is_empty())
  {
    "file".to_string()
  } else {
    // A browser choice may carry a profile ("chrome:Profile 1"): keep the browser only.
    token(
      auth
        .cookie_browser
        .split([':', '+', '/'])
        .next()
        .unwrap_or(""),
    )
  };
  vec![
    ("theme", token(&config.appearance.theme)),
    ("language", token(&config.appearance.language)),
    ("cookies", cookies),
    (
      "alwaysUseBrowserLogins",
      flag(auth.always_use_browser_logins),
    ),
    (
      "rememberedSites",
      config.auth.site_browsers.len().to_string(),
    ),
    (
      "proxy",
      flag(
        config.network.enable_proxy.unwrap_or(false)
          && config
            .network
            .proxy
            .as_deref()
            .is_some_and(|p| !p.is_empty()),
      ),
    ),
    (
      "impersonate",
      flag(
        !config.network.impersonate.is_empty()
          && config.network.impersonate != defaults.network.impersonate,
      ),
    ),
    (
      "extractorArgs",
      flag(!config.network.extractor_args.trim().is_empty()),
    ),
    ("watchClipboard", flag(config.input.watch_clipboard)),
    ("autoFillClipboard", flag(config.input.auto_fill_clipboard)),
    ("globalShortcuts", flag(config.input.global_shortcuts)),
    (
      "videoContainer",
      enum_name(serde_json::to_value(config.output.video.container)),
    ),
    (
      "videoPolicy",
      enum_name(serde_json::to_value(config.output.video.policy)),
    ),
    (
      "audioFormat",
      enum_name(serde_json::to_value(config.output.audio.format)),
    ),
    ("addMetadata", flag(config.output.add_metadata)),
    ("addThumbnail", flag(config.output.add_thumbnail)),
    (
      "customFolder",
      flag(
        config
          .output
          .download_dir
          .as_deref()
          .is_some_and(|d| !d.is_empty()),
      ),
    ),
    (
      "customFileName",
      flag(config.output.file_name_template != defaults.output.file_name_template),
    ),
    ("restrictFilenames", flag(config.output.restrict_filenames)),
    ("subtitles", flag(config.subtitles.enabled)),
    (
      "maxConcurrency",
      config.performance.max_concurrency.to_string(),
    ),
    (
      "fragments",
      config.performance.concurrent_fragments.to_string(),
    ),
    ("aria2", flag(config.performance.aria2_enabled)),
    (
      "sponsorBlock",
      flag(
        !config.sponsor_block.remove_parts.is_empty()
          || !config.sponsor_block.mark_parts.is_empty(),
      ),
    ),
    ("updateTools", flag(config.update.update_binaries)),
    ("updateApp", flag(config.update.update_app)),
    ("tray", flag(config.system.tray_enabled)),
    (
      "closeBehavior",
      enum_name(serde_json::to_value(config.system.close_behavior)),
    ),
  ]
}

#[cfg(test)]
mod tests {
  use super::*;

  fn input() -> ReportInput {
    ReportInput {
      note: None,
      app_version: "1.2.0".into(),
      os: OsInfo {
        name: "Linux".into(),
        version: "6.8.0".into(),
        arch: "x86_64".into(),
        distro: Some("Ubuntu 24.04".into()),
        desktop: Some("GNOME".into()),
        session: Some("wayland".into()),
      },
      tools: vec![
        ("yt-dlp".into(), "2026.10.26".into()),
        ("aria2".into(), "missing".into()),
      ],
      locale: ("ar".into(), "ar-SA".into()),
      settings: vec![("theme", "dark".into()), ("aria2", "off".into())],
      download: None,
      crash: None,
      log_lines: vec!["line one".into(), "line two".into()],
    }
  }

  #[test]
  fn render_lists_diagnostics_and_linux_desktop() {
    let body = render(&input());
    for needle in [
      "- App: Nezzel 1.2.0",
      "- OS: Linux 6.8.0 (x86_64)",
      "- Linux: Ubuntu 24.04 · desktop GNOME · session wayland",
      "- Language: ar (system ar-SA)",
      "- Tools: yt-dlp 2026.10.26, aria2 missing",
      "- Settings: theme=dark, aria2=off",
      "line one\nline two\n```",
    ] {
      assert!(body.contains(needle), "missing {needle:?} in\n{body}");
    }
    assert!(!body.contains("### Failed download"));
    assert_eq!(title(&input()), "Problem report");
  }

  #[test]
  fn failed_row_shows_code_and_trail_as_tokens_only() {
    let mut input = input();
    input.download = Some(DownloadContext {
      error_code: Some("http403".into()),
      site: Some("instagram.com".into()),
      link: Some("instagram.com/reel/{id}".into()),
      extractor: Some("Instagram".into()),
      exit_code: Some(1),
      attempts: vec![
        Attempt {
          stage: "download".into(),
          login: false,
          outcome: "failed".into(),
          code: Some("http403".into()),
          ms: 1200,
        },
        // Hostile values from the webview must not smuggle text in.
        Attempt {
          stage: "https://x.com/a b".into(),
          login: true,
          outcome: "/Users/me".into(),
          code: None,
          ms: 5,
        },
      ],
    });
    let body = render(&input);
    assert!(body.contains("- Error: http403 (exit 1)"), "{body}");
    assert!(body.contains("- Link: instagram.com/reel/{id}"), "{body}");
    assert!(
      body.contains("- Site: instagram.com (extractor Instagram)"),
      "{body}"
    );
    assert!(
      body.contains("- Attempt 1: download · login no · failed · http403 · 1200 ms"),
      "{body}"
    );
    assert!(
      body.contains("- Attempt 2: https:x.comab · login yes · Usersme · - · 5 ms"),
      "{body}"
    );
    assert_eq!(title(&input), "Download failed: http403");
  }

  #[test]
  fn build_keeps_the_newest_log_lines_and_the_note() {
    let mut input = input();
    input.note = Some("التنزيل توقف عند 50%".into());
    input.log_lines = (0..400)
      .map(|n| format!("log line {n} العربية ok"))
      .collect();
    let report = build(&input);
    assert!(
      report
        .body
        .starts_with("### Note\n\nالتنزيل توقف عند 50%\n"),
      "{}",
      report.body
    );
    assert!(report.body.contains("log line 399"));
    assert!(!report.body.contains("log line 349 "));
    assert!(report.body.contains("log line 350 "));
  }

  #[test]
  fn os_parsers() {
    assert_eq!(
      parse_os_release("NAME=\"Linux Mint\"\nVERSION_ID=\"21.3\"\nID=linuxmint\n"),
      Some("Linux Mint 21.3".into())
    );
    assert_eq!(
      parse_os_release("NAME=Arch Linux\nID=arch\n"),
      Some("Arch Linux".into())
    );
    assert_eq!(parse_os_release("PRETTY_NAME=x\n"), None);
    assert_eq!(
      parse_windows_ver("\r\nMicrosoft Windows [Version 10.0.22631.4317]\r\n"),
      Some("10.0.22631.4317".into())
    );
    assert_eq!(
      parse_macos_plist("<dict>\n\t<key>ProductVersion</key>\n\t<string>15.6</string>\n</dict>"),
      Some("15.6".into())
    );
  }

  #[test]
  fn settings_summary_never_carries_free_text() {
    let mut config = Config::default();
    config.output.download_dir = Some("/Users/sara/Movies".into());
    config.output.file_name_template = "%(title)s secret".into();
    config.network.enable_proxy = Some(true);
    config.network.proxy = Some("socks5://user:pw@1.2.3.4".into());
    config.auth.cookie_browser = "chrome:Profile Sara".into();
    config
      .auth
      .site_browsers
      .insert("instagram.com".into(), "chrome".into());
    let text = format!("{:?}", settings_summary(&config));
    for secret in ["Sara", "sara", "secret", "socks5", "instagram", "Movies"] {
      assert!(!text.contains(secret), "{secret} in {text}");
    }
    assert!(text.contains("(\"cookies\", \"chrome\")"), "{text}");
    assert!(text.contains("(\"customFolder\", \"on\")"), "{text}");
    assert!(text.contains("(\"rememberedSites\", \"1\")"), "{text}");
  }
}
