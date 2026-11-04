// Called exclusively by the explicit global shortcut. No clipboard or browser URL is logged.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub async fn resolve(mut command: tokio::process::Command) -> Option<String> {
  command
    .kill_on_drop(true)
    .stderr(std::process::Stdio::null());
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
  let mut frontmost = tokio::process::Command::new("/usr/bin/osascript");
  frontmost.args([
    "-l",
    "JavaScript",
    "-e",
    "ObjC.import('AppKit'); $.NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier.js",
  ]);
  let bundle = resolve(frontmost).await?;
  let script = match bundle.as_str() {
    "com.apple.Safari" => {
      "tell application id \"com.apple.Safari\" to get URL of current tab of front window"
    }
    "com.google.Chrome" => {
      "tell application id \"com.google.Chrome\" to get URL of active tab of front window"
    }
    "com.microsoft.edgemac" => {
      "tell application id \"com.microsoft.edgemac\" to get URL of active tab of front window"
    }
    "com.brave.Browser" => {
      "tell application id \"com.brave.Browser\" to get URL of active tab of front window"
    }
    "company.thebrowser.Browser" => {
      "tell application id \"company.thebrowser.Browser\" to get URL of active tab of front window"
    }
    _ => return None,
  };
  let mut command = tokio::process::Command::new("/usr/bin/osascript");
  command.args(["-e", script]);
  Some(command)
}

#[cfg(target_os = "windows")]
pub async fn request() -> Option<tokio::process::Command> {
  // UI Automation only inspects edit controls in the foreground browser; never activates it
  // or sends keystrokes. PowerShell ships with Windows, so no additional runtime is needed.
  let mut foreground = tokio::process::Command::new("powershell.exe");
  foreground.args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type 'using System; using System.Runtime.InteropServices; public class NezzelForeground { [DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); }'; [Console]::Write([NezzelForeground]::GetForegroundWindow().ToInt64())"]);
  let hwnd: i64 = resolve(foreground).await?.parse().ok()?;
  let script = r#"
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type 'using System; using System.Runtime.InteropServices; public class NezzelForeground { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); }'
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]::new(NEZZEL_HWND))
$process = Get-Process -Id $root.Current.ProcessId
if ($process.ProcessName -notin @('chrome','msedge','brave','firefox','opera','vivaldi','arc')) { exit }
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
  let mut command = tokio::process::Command::new("powershell.exe");
  command.args([
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    &script.replace("NEZZEL_HWND", &hwnd.to_string()),
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
