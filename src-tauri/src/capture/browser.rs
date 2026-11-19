use std::sync::atomic::{AtomicBool, Ordering};

// Called exclusively by the explicit global shortcut. No clipboard or browser URL is logged.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub async fn resolve(mut command: tokio::process::Command) -> Result<String, &'static str> {
  command
    .kill_on_drop(true)
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());
  // No console window on Windows: `tool_process::async_command` built `command` (CREATE_NO_WINDOW).
  let result = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
    .await
    .map_err(|_| "timeout")?
    .map_err(|_| "failed")?;
  outcome(result.status.success(), &result.stdout, &result.stderr)
}

/// A browser read's text, or why there is none, as a log-safe word. stderr is only classified, never kept.
#[cfg_attr(
  not(any(target_os = "macos", target_os = "windows", test)),
  allow(dead_code)
)]
fn outcome(success: bool, stdout: &[u8], stderr: &[u8]) -> Result<String, &'static str> {
  if !success {
    let error = String::from_utf8_lossy(stderr);
    let code = error
      .rsplit_once("(-")
      .and_then(|(_, rest)| rest.split_once(')'))
      .map(|(digits, _)| format!("-{digits}"));
    return Err(code.map_or("failed", |code| error_word(&code)));
  }
  match std::str::from_utf8(stdout).map(str::trim) {
    Ok(text) if !text.is_empty() => Ok(text.to_owned()),
    _ => Err("empty"),
  }
}

/// One `tccutil reset` per app run: an ad-hoc signed update keeps the old build's Automation grant on
/// record (System Settings still shows it on) while macOS refuses the new build without asking again.
static AUTOMATION_RESET: AtomicBool = AtomicBool::new(false);

/// What the quick bar does after a browser read.
#[derive(Debug, PartialEq, Eq)]
pub enum AfterRead {
  Use,
  /// Forget the stale grant and read once more; macOS asks again.
  ResetAndRetry,
  /// Still refused: say so instead of falling back to an unrelated clipboard link.
  Denied,
}

/// `mac`: only macOS has an Automation permission to reset or explain; Windows reads the address bar
/// through UI Automation without one, and Linux never reads the browser.
pub fn after_read(read: Result<&str, &str>, reset_used: bool, mac: bool) -> AfterRead {
  match read {
    Err("denied") if mac && !reset_used => AfterRead::ResetAndRetry,
    Err("denied") if mac => AfterRead::Denied,
    _ => AfterRead::Use,
  }
}

/// Claims this run's single reset; false once it was used.
pub fn claim_reset() -> bool {
  !AUTOMATION_RESET.swap(true, Ordering::Relaxed)
}

pub fn reset_used() -> bool {
  AUTOMATION_RESET.load(Ordering::Relaxed)
}

/// Forgets Nezzel's Automation decisions so the next read prompts again. Only Nezzel's own bundle.
#[cfg(target_os = "macos")]
pub async fn reset_automation(bundle: &str) -> bool {
  let Ok(mut command) = crate::runners::tool_process::async_command("/usr/bin/tccutil") else {
    return false;
  };
  command
    .args(["reset", "AppleEvents", bundle])
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
  matches!(
    tokio::time::timeout(std::time::Duration::from_secs(5), command.status()).await,
    Ok(Ok(status)) if status.success()
  )
}

#[cfg(not(target_os = "macos"))]
pub async fn reset_automation(_bundle: &str) -> bool {
  false
}

/// The foreground app as a log word (browser name, "nezzel", "other", "unknown") and the command that
/// reads its URL, when it is a browser Nezzel can read.
pub struct Request {
  pub frontmost: &'static str,
  pub command: Option<tokio::process::Command>,
}

/// One osascript run reads the frontmost app and, when it is a known browser, its active tab: about 0.3 s,
/// instead of two processes. NSWorkspace needs no Accessibility; the tab read needs Automation for that
/// browser. Output: the bundle id, then the URL or `!<AppleScript error number>` on a second line.
#[cfg(target_os = "macos")]
pub async fn request(_own_bundle: &str) -> Request {
  let mut command = match crate::runners::tool_process::async_command("/usr/bin/osascript") {
    Ok(command) => command,
    Err(_) => {
      return Request {
        frontmost: "unknown",
        command: None,
      }
    }
  };
  command.args(["-l", "JavaScript", "-e", &mac_script()]);
  Request {
    frontmost: "pending",
    command: Some(command),
  }
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn mac_script() -> String {
  use crate::runners::browsers::{BrowserKind, CookieStore};
  let tabs = BrowserKind::ALL
    .into_iter()
    .filter_map(|kind| {
      let tab = if kind.store() == CookieStore::Safari {
        "currentTab"
      } else {
        "activeTab"
      };
      kind
        .mac_bundle()
        .map(|bundle| format!("{bundle:?}: {tab:?}"))
    })
    .collect::<Vec<_>>()
    .join(", ");
  format!(
    "ObjC.import('AppKit');
(() => {{
  const id = $.NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier.js || '';
  const tab = {{{tabs}}}[id];
  if (!tab) return id;
  try {{ return id + '\\n' + Application(id).windows[0][tab].url(); }}
  catch (e) {{ return id + '\\n!' + (e.errorNumber || 0); }}
}})()"
  )
}

/// The frontmost app as a log word and the tab read, from [`mac_script`]'s output.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn parse_mac(output: &str, own_bundle: &str) -> (&'static str, Result<String, &'static str>) {
  let mut lines = output.trim().lines();
  let bundle = lines.next().unwrap_or("").trim();
  let Some(kind) = crate::runners::browsers::BrowserKind::from_mac_bundle(bundle) else {
    let frontmost = match bundle {
      "" => "unknown",
      b if b == own_bundle => "nezzel",
      _ => "other",
    };
    return (frontmost, Err("skipped"));
  };
  let read = match lines.next().map(str::trim) {
    None | Some("") => Err("empty"),
    Some(code) if code.starts_with('!') => Err(error_word(&code[1..])),
    Some(url) => Ok(url.to_owned()),
  };
  (kind.name(), read)
}

/// AppleScript error numbers: -1743 Automation not allowed (also a grant made for another build),
/// -1728/-1719 no window or tab to read.
fn error_word(code: &str) -> &'static str {
  match code {
    "-1743" => "denied",
    "-1728" | "-1719" => "no-window",
    _ => "failed",
  }
}

#[cfg(target_os = "windows")]
pub async fn request(_own_bundle: &str) -> Request {
  Request {
    frontmost: "foreground",
    command: windows_request().await,
  }
}

#[cfg(target_os = "windows")]
async fn windows_request() -> Option<tokio::process::Command> {
  // UI Automation only inspects edit controls in the foreground browser; never activates it
  // or sends keystrokes. PowerShell ships with Windows, so no additional runtime is needed.
  let mut foreground = crate::runners::tool_process::async_command("powershell.exe").ok()?;
  foreground.args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type 'using System; using System.Runtime.InteropServices; public class NezzelForeground { [DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); }'; [Console]::Write([NezzelForeground]::GetForegroundWindow().ToInt64())"]);
  let hwnd: i64 = resolve(foreground).await.ok()?.parse().ok()?;
  let script = r#"
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type 'using System; using System.Runtime.InteropServices; public class NezzelForeground { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); }'
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]::new(NEZZEL_HWND))
$process = Get-Process -Id $root.Current.ProcessId
if ($process.ProcessName -notin @(NEZZEL_PROCESSES)) { exit }
$condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)
$edits = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
foreach ($edit in $edits) {
  $id = $edit.Current.AutomationId
  $name = $edit.Current.Name
  if ($id -notin @('addressEditBox','urlbar-input') -and $name -notmatch 'address|URL|adresse|direcci|عنوان') { continue }
  $value = $edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
  if ($value -match '^https?://') { [Console]::Write($value); exit }
  if ($value -match '^[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}/') { [Console]::Write('https://' + $value); exit }
}
"#;
  let processes = crate::runners::browsers::BrowserKind::windows_processes()
    .map(|name| format!("'{name}'"))
    .collect::<Vec<_>>()
    .join(",");
  let mut command = crate::runners::tool_process::async_command("powershell.exe").ok()?;
  command.args([
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    &script
      .replace("NEZZEL_HWND", &hwnd.to_string())
      .replace("NEZZEL_PROCESSES", &processes),
  ]);
  Some(command)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub async fn request(_own_bundle: &str) -> Request {
  Request {
    frontmost: "unsupported",
    command: None,
  }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub async fn resolve(_command: tokio::process::Command) -> Result<String, &'static str> {
  Err("unsupported")
}

#[cfg(test)]
mod tests {
  use super::{after_read, mac_script, outcome, parse_mac, AfterRead};

  #[test]
  fn one_mac_read_reports_the_frontmost_app_and_its_tab() {
    let own = "dev.ahmed.nezzel";
    let url = "https://www.tiktok.com/@a/photo/1";
    for (output, frontmost, read) in [
      (format!("com.google.Chrome\n{url}\n"), "chrome", Ok(url)),
      ("com.google.Chrome\n!-1743".into(), "chrome", Err("denied")),
      (
        "com.apple.Safari\n!-1719".into(),
        "safari",
        Err("no-window"),
      ),
      ("com.brave.Browser\n!-600".into(), "brave", Err("failed")),
      ("com.google.Chrome".into(), "chrome", Err("empty")),
      ("dev.ahmed.nezzel".into(), "nezzel", Err("skipped")),
      ("com.cmuxterm.app".into(), "other", Err("skipped")),
      (String::new(), "unknown", Err("skipped")),
    ] {
      assert_eq!(
        parse_mac(&output, own),
        (frontmost, read.map(str::to_owned)),
        "{output}"
      );
    }
    let script = mac_script();
    assert!(
      script.contains(r#""com.apple.Safari": "currentTab""#),
      "{script}"
    );
    assert!(
      script.contains(r#""com.google.Chrome": "activeTab""#),
      "{script}"
    );
  }

  #[test]
  fn a_refused_browser_read_resets_once_then_reports_the_refusal() {
    let url = "https://www.tiktok.com/@a/photo/1";
    for (read, reset_used, mac, expected) in [
      (Err("denied"), false, true, AfterRead::ResetAndRetry),
      (Err("denied"), true, true, AfterRead::Denied),
      (Ok(url), false, true, AfterRead::Use),
      (Ok(url), true, true, AfterRead::Use),
      (Err("failed"), false, true, AfterRead::Use),
      (Err("timeout"), true, true, AfterRead::Use),
      (Err("skipped"), false, true, AfterRead::Use),
      // Windows and Linux: no permission state, never a reset.
      (Err("denied"), false, false, AfterRead::Use),
      (Err("denied"), true, false, AfterRead::Use),
      (Ok(url), false, false, AfterRead::Use),
    ] {
      assert_eq!(
        after_read(read, reset_used, mac),
        expected,
        "{read:?} {reset_used} {mac}"
      );
    }
  }

  #[test]
  fn browser_read_failures_become_log_words_without_the_error_text() {
    let denied =
      b"execution error: Not authorized to send Apple events to Google Chrome. (-1743)\n";
    assert_eq!(outcome(false, b"", denied), Err("denied"));
    let no_window = b"execution error: Google Chrome got an error: Can't get window 1. (-1728)";
    assert_eq!(outcome(false, b"", no_window), Err("no-window"));
    assert_eq!(outcome(false, b"", b"boom"), Err("failed"));
    assert_eq!(outcome(true, b" \n", b""), Err("empty"));
    assert_eq!(
      outcome(true, b"https://www.tiktok.com/@a/photo/1\n", b""),
      Ok("https://www.tiktok.com/@a/photo/1".to_owned())
    );
  }
}
