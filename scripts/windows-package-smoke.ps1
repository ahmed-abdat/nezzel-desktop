# Installs the NSIS setup the way a user's machine would end up, then checks the result
# (CI: .github/workflows/windows-smoke.yml).
#   pwsh scripts/windows-package-smoke.ps1 -Setup <Nezzel_x.y.z_x64-setup.exe> -Version <x.y.z>
# Prints one PASS/FAIL/WARN/INFO line per check and exits non-zero on any FAIL.
# The network download is WARN-only: hosted runners are often refused by video sites.
param(
  [Parameter(Mandatory = $true)][string]$Setup,
  [Parameter(Mandatory = $true)][string]$Version
)
$ErrorActionPreference = 'Continue'
$script:failures = 0
function Pass($m) { Write-Output "PASS $m" }
function Fail($m, $why = '') { Write-Output ("FAIL $m" + $(if ($why) { ": $why" } else { '' })); $script:failures++ }
function Warn($m, $why = '') { Write-Output ("WARN $m" + $(if ($why) { ": $why" } else { '' })) }
function Info($m) { Write-Output "INFO $m" }
function Check($ok, $m, $why = '') { if ($ok) { Pass $m } else { Fail $m $why } }

function Get-NezzelUninstallKey {
  $roots = 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
           'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
           'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
  foreach ($root in $roots) {
    Get-ChildItem $root -ErrorAction SilentlyContinue | ForEach-Object {
      $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
      if ($p.DisplayName -eq 'Nezzel') { return $p }
    }
  }
}
function Get-NezzelProcesses { @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -eq 'nezzel' }) }
function Get-StartMenuEntries {
  @(Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu", "$env:ProgramData\Microsoft\Windows\Start Menu" `
      -Recurse -Filter 'Nezzel*.lnk' -ErrorAction SilentlyContinue)
}

# --- Install ---------------------------------------------------------------------------
$Setup = (Resolve-Path $Setup).Path
Info "installer signature: $((Get-AuthenticodeSignature $Setup).Status)"
$started = Get-Date
$proc = Start-Process $Setup -ArgumentList '/S' -PassThru -Wait
Check ($proc.ExitCode -eq 0) 'setup /S exits 0' "exit $($proc.ExitCode)"
Info "install took $([int]((Get-Date) - $started).TotalSeconds) s"

$key = Get-NezzelUninstallKey | Select-Object -First 1
Check ($null -ne $key) 'uninstall entry "Nezzel" exists'
if (-not $key) { Write-Output "FAILURES $script:failures"; exit 1 }
Check ($key.DisplayVersion -eq $Version) "uninstall entry version is $Version" "got '$($key.DisplayVersion)'"
Check ([bool]$key.Publisher) 'uninstall entry has a publisher' 'empty'
Info "publisher: $($key.Publisher)"

$dir = $key.InstallLocation.Trim('"')
Check (Test-Path $dir) 'install folder exists' $dir
Info "install folder: $dir"
$exe = Join-Path $dir 'nezzel.exe'
Check (Test-Path $exe) 'nezzel.exe present'
Check ((Get-StartMenuEntries).Count -ge 1) 'Start menu shortcut present'

$wv = @('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}') |
  ForEach-Object { (Get-ItemProperty $_ -ErrorAction SilentlyContinue).pv } | Where-Object { $_ } | Select-Object -First 1
Check ([bool]$wv) 'WebView2 runtime present after install'
Info "WebView2 $wv"

# --- Bundled tools ---------------------------------------------------------------------
$scratch = Join-Path $env:TEMP "nezzel-smoke-$PID"
New-Item -ItemType Directory $scratch -Force | Out-Null
$tools = @{}
foreach ($tool in 'yt-dlp', 'ffmpeg', 'ffprobe', 'deno', 'gallery-dl', 'aria2c') {
  $found = Get-ChildItem $dir -Recurse -Filter "$tool.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $found) { Fail "$tool present"; continue }
  $tools[$tool] = $found.FullName
  $flag = if ($tool -in 'ffmpeg', 'ffprobe') { '-version' } else { '--version' }
  Push-Location $scratch
  $all = & $found.FullName $flag 2>&1
  $rc = $LASTEXITCODE
  $out = $all | Select-Object -First 1
  Pop-Location
  Check (($rc -eq 0) -and $out) "$tool runs $flag" "exit $rc"
}
$shaFile = Get-ChildItem $dir -Recurse -Filter 'gallery-dl.sha256' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($shaFile -and $tools['gallery-dl']) {
  $expected = ((Get-Content $shaFile.FullName -Raw) -split '\s+')[0]
  $actual = (Get-FileHash $tools['gallery-dl'] -Algorithm SHA256).Hash
  Check ($expected -ieq $actual) 'gallery-dl matches gallery-dl.sha256'
} else { Fail 'gallery-dl.sha256 present' }

# --- Launch, single instance, close ----------------------------------------------------
if (Test-Path $exe) {
  Start-Process $exe | Out-Null
  $deadline = (Get-Date).AddSeconds(45)
  do { Start-Sleep 3; $ps = Get-NezzelProcesses } until (($ps | Where-Object { $_.MainWindowTitle }) -or ((Get-Date) -gt $deadline))
  Check ($ps.Count -ge 1) 'app is running after launch'
  $title = ($ps | Where-Object { $_.MainWindowTitle } | Select-Object -First 1).MainWindowTitle
  Check ([bool]$title) 'main window opened' 'no window title within 45 s'
  Info "window title: $title"
  Check (@(Get-Process msedgewebview2 -ErrorAction SilentlyContinue).Count -ge 1) 'WebView2 process started for the window'

  $before = (Get-NezzelProcesses).Count
  Start-Process $exe | Out-Null
  Start-Sleep 8
  $after = (Get-NezzelProcesses).Count
  Check ($after -eq $before) 'second launch does not start a second instance' "before $before, after $after"

  Start-Sleep 5
  $alive = (Get-NezzelProcesses).Count
  Check ($alive -ge 1) 'app still running 20 s after start (no early crash)'

  (Get-NezzelProcesses | Where-Object { $_.MainWindowHandle -ne 0 }) | ForEach-Object { $_.CloseMainWindow() | Out-Null }
  Start-Sleep 6
  Info "after closing the window: $((Get-NezzelProcesses).Count) nezzel process(es) (tray keeps it alive when that setting is on)"
  Get-NezzelProcesses | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep 3
}

$crashes = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Level = 2; StartTime = $started } -ErrorAction SilentlyContinue |
  Where-Object { $_.Message -match 'nezzel' })
Check ($crashes.Count -eq 0) 'no Application error events mention nezzel' ($crashes | Select-Object -First 1 | ForEach-Object { $_.Message.Substring(0, [Math]::Min(200, $_.Message.Length)) })

# The app's own (already scrubbed) crash file and log tail explain a FAIL above.
Get-ChildItem $env:APPDATA, $env:LOCALAPPDATA -Recurse -Depth 4 -Include 'crash-report.txt', 'nezzel.log' -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -match 'nezzel' } | ForEach-Object {
    Info "---- $($_.FullName)"
    Get-Content $_.FullName -Tail 40 -ErrorAction SilentlyContinue | ForEach-Object { Write-Output "LOG  $_" }
  }

# --- One real download with the bundled tools (WARN-only) ------------------------------
if ($tools['yt-dlp'] -and $tools['ffprobe']) {
  $out = Join-Path $scratch 'dl'
  New-Item -ItemType Directory $out -Force | Out-Null
  $env:PATH = "$(Split-Path $tools['ffmpeg']);$(Split-Path $tools['deno']);$env:PATH"
  $log = & $tools['yt-dlp'] --no-playlist -o "$out\%(id)s.%(ext)s" 'https://www.youtube.com/watch?v=jNQXAC9IVRw' 2>&1
  $file = Get-ChildItem $out -File -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($file) {
    $dur = & $tools['ffprobe'] -v error -show_entries format=duration -of csv=p=0 $file.FullName 2>&1
    if ($LASTEXITCODE -eq 0 -and $dur) { Pass "bundled yt-dlp downloaded a video and ffprobe reads it ($dur s)" }
    else { Warn 'ffprobe could not read the downloaded file' "$dur" }
  } else { Warn 'bundled yt-dlp download (network-dependent)' (($log | Select-Object -Last 1) -replace '\s+', ' ') }
}

# --- Uninstall -------------------------------------------------------------------------
$uninstaller = ($key.UninstallString -replace '"', '')
if (Test-Path $uninstaller) {
  $proc = Start-Process $uninstaller -ArgumentList '/S' -PassThru -Wait
  $deadline = (Get-Date).AddSeconds(60)
  while ((Test-Path $exe) -and ((Get-Date) -lt $deadline)) { Start-Sleep 2 }
  Check (-not (Test-Path $exe)) 'uninstall removes nezzel.exe'
  Check ($null -eq (Get-NezzelUninstallKey | Select-Object -First 1)) 'uninstall removes the uninstall entry'
  Check ((Get-StartMenuEntries).Count -eq 0) 'uninstall removes the Start menu shortcut'
  Check ((Get-NezzelProcesses).Count -eq 0) 'no nezzel process left after uninstall'
  Get-ChildItem $env:APPDATA, $env:LOCALAPPDATA -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match 'nezzel' } | ForEach-Object { Info "left behind (user data): $($_.FullName)" }
} else { Fail 'uninstaller present' $uninstaller }

Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
Write-Output "FAILURES $script:failures"
if ($script:failures -gt 0) { exit 1 }
# A failed network download above leaves a non-zero $LASTEXITCODE, which the runner would report.
exit 0
