//! The yt-dlp cookie exporter: yt-dlp reads (and decrypts) a Chromium-family jar into a full
//! Netscape export, which is filtered to the link's site and then removed.
use super::{
  access::{CookieContext, COOKIE_READ_ERROR},
  jar::{filter_export, private_file, CookieJar, COOKIE_HEADER},
};
use crate::extract::deadline::{wait_until, Interrupted, FETCH_TIMEOUT};
use crate::runners::browsers::BrowserSource;
use std::{io::Write, process::Stdio, time::Duration};

pub(super) async fn export(
  context: &CookieContext,
  browser: &BrowserSource,
  host: &str,
  jar: &CookieJar,
  scoped: &mut std::fs::File,
  budget: Duration,
  cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), &'static str> {
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
    .stderr(Stdio::null());
  let deadline = tokio::time::Instant::now() + budget;
  let status = match wait_until(run_cookie_export(command), deadline, cancel).await {
    Ok(status) => status?,
    Err(Interrupted::Cancelled) => return Err("cancelled"),
    Err(Interrupted::Timeout) => return Err(FETCH_TIMEOUT),
  };
  if !status.success() {
    return Err(COOKIE_READ_ERROR);
  }
  filter_export(&full, host, scoped).map_err(|_| COOKIE_READ_ERROR)?;
  std::fs::remove_file(info).map_err(|_| COOKIE_READ_ERROR)?;
  Ok(())
}

// The exporter may start Keychain helpers: `ToolProcess` cancels the entire process tree, as for
// downloads, and holds the yt-dlp binaries lock until the exporter has exited.
async fn run_cookie_export(
  mut command: std::process::Command,
) -> Result<std::process::ExitStatus, &'static str> {
  let process_lock = crate::binaries::YTDLP_PROCESSES.clone().read_owned().await;
  let mut process = crate::runners::tool_process::ToolProcess::spawn(&mut command, process_lock)
    .map_err(|_| COOKIE_READ_ERROR)?
    .process;
  process.wait().await.map_err(|_| COOKIE_READ_ERROR)
}
