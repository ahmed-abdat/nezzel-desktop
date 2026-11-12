//! The yt-dlp cookie exporter: yt-dlp reads (and decrypts) a Chromium-family jar into a full
//! Netscape export, which is filtered to the link's site and then removed.
use super::{
  access::{CookieContext, COOKIE_READ_ERROR},
  jar::{filter_export, private_file, CookieJar, COOKIE_HEADER},
};
use crate::extract::deadline::{wait_until, Interrupted, FETCH_TIMEOUT};
use crate::runners::browsers::{BrowserKind, BrowserSource};
use std::{io::Write, process::Stdio, time::Duration};

pub(super) async fn export(
  context: &CookieContext,
  browser: &BrowserSource,
  host: &str,
  jar: &CookieJar,
  scoped: &mut std::fs::File,
  budget: Duration,
  cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<usize, &'static str> {
  let full = jar.directory.join("full.txt");
  private_file(&full)
    .and_then(|mut file| file.write_all(COOKIE_HEADER.as_bytes()))
    .map_err(|_| COOKIE_READ_ERROR)?;
  let info = jar.directory.join("empty.json");
  private_file(&info).and_then(|mut file| file.write_all(br#"{"_type":"playlist","id":"cookie-export","title":"cookie-export","extractor":"generic","extractor_key":"Generic","entries":[]}"#)).map_err(|_| COOKIE_READ_ERROR)?;
  let mut command = std::process::Command::new(&context.exporter);
  command
    .args([
      "--ignore-config",
      "--no-cache-dir",
      "--cookies-from-browser",
      browser.spec().as_str(),
      "--cookies",
    ])
    .arg(&full)
    .args([
      "--skip-download",
      "--no-clean-info-json",
      "--load-info-json",
    ])
    .arg(&info)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
  let deadline = tokio::time::Instant::now() + budget;
  let (status, stderr) = match wait_until(run_cookie_export(command), deadline, cancel).await {
    Ok(status) => status?,
    Err(Interrupted::Cancelled) => return Err("cancelled"),
    Err(Interrupted::Timeout) => return Err(FETCH_TIMEOUT),
  };
  if !status.success() {
    return Err(classify_error(
      browser.kind,
      &String::from_utf8_lossy(&stderr),
    ));
  }
  let rows = filter_export(&full, host, scoped).map_err(|_| COOKIE_READ_ERROR)?;
  // yt-dlp can exit successfully after warning that every cookie failed decryption.
  if rows == 0 {
    let code = classify_error(browser.kind, &String::from_utf8_lossy(&stderr));
    if code != COOKIE_READ_ERROR {
      return Err(code);
    }
  }
  std::fs::remove_file(info).map_err(|_| COOKIE_READ_ERROR)?;
  Ok(rows)
}

// The exporter may start Keychain helpers: `ToolProcess` cancels the entire process tree, as for
// downloads, and holds the yt-dlp binaries lock until the exporter has exited.
async fn run_cookie_export(
  mut command: std::process::Command,
) -> Result<(std::process::ExitStatus, Vec<u8>), &'static str> {
  let process_lock = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
  let spawned = crate::runners::tool_process::ToolProcess::spawn(&mut command, process_lock)
    .map_err(|_| COOKIE_READ_ERROR)?;
  let mut process = spawned.process;
  // Drain concurrently so a full pipe cannot block the exporter. Retain at most 64 KiB,
  // classify it in memory, and return only a fixed code. Never log/export stderr.
  let mut stderr = tokio::process::ChildStderr::from_std(spawned.stderr.ok_or(COOKIE_READ_ERROR)?)
    .map_err(|_| COOKIE_READ_ERROR)?;
  let reader = async move {
    use tokio::io::AsyncReadExt;
    let mut kept = Vec::new();
    let mut buffer = [0; 4096];
    loop {
      match stderr.read(&mut buffer).await {
        Ok(0) | Err(_) => break,
        Ok(n) => kept.extend_from_slice(&buffer[..n.min(65536_usize.saturating_sub(kept.len()))]),
      }
    }
    kept
  };
  let (status, stderr) = tokio::join!(process.wait(), reader);
  let status = status.map_err(|_| COOKIE_READ_ERROR)?;
  Ok((status, stderr))
}

/// Untrusted exporter text never crosses this boundary; even fallback is a fixed code.
pub(super) fn classify_error(browser: BrowserKind, stderr: &str) -> &'static str {
  crate::parsers::ytdlp_error::CookieReadError::classify(
    stderr,
    true,
    browser == BrowserKind::Safari,
  )
  .map(crate::parsers::ytdlp_error::CookieReadError::code)
  .unwrap_or(COOKIE_READ_ERROR)
}
