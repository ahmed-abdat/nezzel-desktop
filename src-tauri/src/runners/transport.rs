use crate::binaries::bundled_tools::sidecar_candidates;
use serde_json::Value;
use std::{
  io::Read,
  path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

pub fn single_direct_http(info: &Value, source: &str) -> bool {
  let host = reqwest::Url::parse(source)
    .ok()
    .and_then(|u| u.host_str().map(str::to_owned))
    .unwrap_or_default();
  if host == "youtu.be"
    || host == "youtube.com"
    || host.ends_with(".youtube.com")
    || host.ends_with(".youtube-nocookie.com")
    || info["extractor_key"]
      .as_str()
      .is_some_and(|s| s.to_ascii_lowercase().contains("youtube"))
  {
    return false;
  }
  if info["_type"].as_str().is_some_and(|s| s != "video")
    || info.get("entries").is_some()
    || info.get("requested_formats").is_some()
    || info.get("fragments").is_some()
  {
    return false;
  }
  if let Some(downloads) = info["requested_downloads"].as_array() {
    return downloads.len() == 1 && single_direct_http(&downloads[0], source);
  }
  matches!(info["protocol"].as_str(), Some("http" | "https"))
    && info["url"].as_str().is_some_and(|s| {
      reqwest::Url::parse(s)
        .ok()
        .is_some_and(|u| matches!(u.scheme(), "http" | "https"))
    })
}
pub fn transfer_args(fragments: u8, aria: Option<&Path>, restart: bool) -> Vec<String> {
  let mut args = vec![
    "--concurrent-fragments".into(),
    fragments.clamp(1, 16).to_string(),
  ];
  if restart {
    args.push("--no-continue".into());
  }
  if let Some(path) = aria {
    args.extend([
      "--downloader".into(),
      // yt-dlp's "http" key covers HTTPS too; "http,https:" is not a valid scope.
      format!("http:{}", path.to_string_lossy()),
      "--downloader-args".into(),
      "aria2c:-x8 -s8 -k1M".into(),
    ]);
  }
  args
}
pub fn aria_denied(line: &str) -> bool {
  let line = line.to_ascii_lowercase();
  if line.contains("http error 403") {
    return true;
  }
  let error = line.trim_start().starts_with("error:")
    || line.contains("[error]")
    || line.contains("errorcode=");
  error
    && (line.contains("authorization failed")
      || line
        .split(|c: char| !c.is_ascii_digit())
        .any(|token| token == "403"))
}
const STUB_MARKER: &[u8] = b"NEZZEL_ARIA2_UNAVAILABLE\n";
pub fn resolve_aria2(app: &AppHandle) -> Option<PathBuf> {
  find_bundled_aria2(
    std::env::current_exe().ok().as_deref(),
    app.path().resource_dir().ok().as_deref(),
  )
}
fn find_bundled_aria2(exe: Option<&Path>, resources: Option<&Path>) -> Option<PathBuf> {
  let name = if cfg!(windows) {
    "aria2c.exe"
  } else {
    "aria2c"
  };
  sidecar_candidates(exe, resources, name)
    .into_iter()
    .find(|path| {
      if !executable(path) {
        return false;
      }
      let mut header = [0; STUB_MARKER.len()];
      std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut header))
        .is_ok_and(|size| !header[..size].starts_with(STUB_MARKER))
    })
}
fn executable(path: &Path) -> bool {
  let Ok(meta) = std::fs::metadata(path) else {
    return false;
  };
  if !meta.is_file() {
    return false;
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
  }
  #[cfg(not(unix))]
  {
    true
  }
}
#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;
  #[test]
  fn only_bundled_non_stub_sidecars_are_available() {
    let root = std::env::temp_dir().join(format!("aria2-paths-{}", uuid::Uuid::new_v4()));
    let bin = root.join("app");
    let resources = root.join("resources");
    let system = root.join("system");
    for dir in [&bin, &resources, &system] {
      std::fs::create_dir_all(dir).unwrap();
    }
    let name = if cfg!(windows) {
      "aria2c.exe"
    } else {
      "aria2c"
    };
    let write = |path: &Path, bytes: &[u8]| {
      std::fs::write(path, bytes).unwrap();
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
      }
    };
    write(&system.join(name), b"unbundled executable");
    assert!(find_bundled_aria2(Some(&bin.join("app")), Some(&resources)).is_none());
    write(&bin.join(name), STUB_MARKER);
    assert!(find_bundled_aria2(Some(&bin.join("app")), Some(&resources)).is_none());
    write(&resources.join(name), b"verified bundled executable");
    assert_eq!(
      find_bundled_aria2(Some(&bin.join("app")), Some(&resources)),
      Some(resources.join(name))
    );
    std::fs::remove_dir_all(root).unwrap();
  }
  #[test]
  fn aria_denials_do_not_match_progress_or_non_error_status_text() {
    for line in [
      "RAW|403|403.21MiB",
      "[download] 403 bytes",
      "HTTP status 403",
      "ERROR: HTTP Error 404",
      "Authorization failed",
    ] {
      assert!(!aria_denied(line), "{line}");
    }
    assert!(aria_denied("ERROR: HTTP Error 403: Forbidden"));
    assert!(aria_denied("[ERROR] errorCode=24 Authorization failed"));
  }
  #[test]
  fn only_proven_single_http_format_uses_aria2() {
    let direct = json!({"protocol":"https", "url":"https://cdn.example/a.mp4"});
    assert!(single_direct_http(&direct, "https://example.com/a"));
    for info in [
      json!({"protocol":"m3u8_native","url":"https://cdn.example/a.m3u8"}),
      json!({"protocol":"http_dash_segments","fragments":[{}],"url":"https://cdn.example/a"}),
      json!({"requested_formats":[direct.clone(),direct.clone()]}),
      json!({"requested_downloads":[direct.clone(),direct.clone()]}),
      json!({"_type":"playlist","entries":[direct.clone()]}),
      json!({}),
    ] {
      assert!(!single_direct_http(&info, "https://example.com/a"));
    }
    assert!(!single_direct_http(
      &direct,
      "https://www.youtube.com/watch?v=a"
    ));
    assert!(!single_direct_http(&direct, "https://youtu.be/a"));
    assert!(!single_direct_http(
      &json!({"extractor_key":"Youtube", "protocol":"https", "url":"https://cdn.example/a"}),
      "https://example.com/a"
    ));
  }
  #[test]
  fn args_bound_fragments_and_scope_aria2_to_http() {
    assert_eq!(
      transfer_args(4, None, false),
      ["--concurrent-fragments", "4"]
    );
    assert_eq!(transfer_args(0, None, false)[1], "1");
    assert_eq!(transfer_args(255, None, false)[1], "16");
    assert_eq!(
      transfer_args(8, Some(Path::new("/app resources/aria2c")), false),
      [
        "--concurrent-fragments",
        "8",
        "--downloader",
        "http:/app resources/aria2c",
        "--downloader-args",
        "aria2c:-x8 -s8 -k1M",
      ]
    );
  }
  #[test]
  fn aria_denial_restarts_with_native_args_and_retries_only_once() {
    use crate::runners::retry_policy::{RetryAction, RetryFailure, RetryState};
    let mut state = RetryState::default();
    let denied = aria_denied("ERROR: HTTP Error 403: Forbidden");
    assert_eq!(
      state.next(
        false,
        false,
        false,
        RetryFailure {
          aria_denied: denied,
          ..Default::default()
        }
      ),
      RetryAction::Native
    );
    assert_eq!(
      transfer_args(4, None, state.native_fallback),
      ["--concurrent-fragments", "4", "--no-continue"]
    );
    assert_eq!(
      state.next(
        false,
        false,
        false,
        RetryFailure {
          aria_denied: denied,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
  }
}
