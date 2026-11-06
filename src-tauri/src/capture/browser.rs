// Called exclusively by the explicit global shortcut. No clipboard or browser URL is logged.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub async fn resolve(mut command: tokio::process::Command) -> Option<String> {
  command
    .kill_on_drop(true)
    .stderr(std::process::Stdio::null());
  // No console window on Windows: `tool_process::async_command` built `command` (CREATE_NO_WINDOW).
  let result = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
    .await
    .ok()?
    .ok()?;
  if !result.status.success() {
    return None;
  }
  String::from_utf8(result.stdout)
    .ok()
    .map(|s| s.trim().to_string())
}

#[cfg(target_os = "macos")]
pub async fn request() -> Option<tokio::process::Command> {
  // NSWorkspace reads the frontmost bundle without asking System Events for Accessibility.
  let mut frontmost = crate::runners::tool_process::async_command("/usr/bin/osascript").ok()?;
  frontmost.args([
    "-l",
    "JavaScript",
    "-e",
    "ObjC.import('AppKit'); $.NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier.js",
  ]);
  let bundle = resolve(frontmost).await?;
  use crate::runners::browsers::{BrowserKind, CookieStore};
  let kind = BrowserKind::from_mac_bundle(&bundle)?;
  let tab = if kind.store() == CookieStore::Safari {
    "current tab"
  } else {
    "active tab"
  };
  let script = format!(
    "tell application id \"{}\" to get URL of {tab} of front window",
    kind.mac_bundle()?
  );
  let mut command = crate::runners::tool_process::async_command("/usr/bin/osascript").ok()?;
  command.args(["-e", &script]);
  Some(command)
}

#[cfg(target_os = "windows")]
pub async fn request() -> Option<tokio::process::Command> {
  // UI Automation only inspects edit controls in the foreground browser; never activates it
  // or sends keystrokes. PowerShell ships with Windows, so no additional runtime is needed.
  let mut foreground = crate::runners::tool_process::async_command("powershell.exe").ok()?;
  foreground.args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type 'using System; using System.Runtime.InteropServices; public class NezzelForeground { [DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); }'; [Console]::Write([NezzelForeground]::GetForegroundWindow().ToInt64())"]);
  let hwnd: i64 = resolve(foreground).await?.parse().ok()?;
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
pub async fn request() -> Option<tokio::process::Command> {
  None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub async fn resolve(_command: tokio::process::Command) -> Option<String> {
  None
}
