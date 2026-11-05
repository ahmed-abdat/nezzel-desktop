#!/usr/bin/env bash
# Every bundled tool starts from the built Nezzel.app (CI: publish.yml, macOS legs).
#   scripts/macos-app-smoke.sh <path/to/Nezzel.app>
# Prints one PASS/FAIL line per check and exits non-zero on any FAIL. No network.
# Catches what signing does to tools: Tauri re-signs Contents/MacOS sidecars with the
# hardened runtime, which broke the onefile gallery-dl (issue 38).
set -euo pipefail

app=${1:?usage: macos-app-smoke.sh <Nezzel.app>}
app=$(cd "$app" && pwd)
contents=$app/Contents
failures=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+: $2}"; failures=$((failures + 1)); }

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

runs() { # runs <label> <path> <flag>: the tool starts and prints a version
  local label=$1 path=$2 flag=$3 out rc=0
  if [[ ! -x $path ]]; then fail "$label present"; return; fi
  if head -c 64 "$path" | grep -qa 'UNAVAILABLE'; then fail "$label is a real build" "disabled stub"; return; fi
  out=$(cd "$scratch" && HOME=$scratch "$path" "$flag" 2>&1) || rc=$?
  if ((rc == 0)) && [[ -n $out ]]; then pass "$label runs $flag"
  else fail "$label runs $flag" "exit $rc: $(head -c 160 <<<"$out" | tr '\n' ' ' | sed -E 's#/(private|var|Users|tmp)/[^ :]*#<path>#g')"; fi
}

for tool in aria2c deno ffmpeg ffprobe; do
  case $tool in ffmpeg | ffprobe) flag=-version ;; *) flag=--version ;; esac
  runs "$tool" "$contents/MacOS/$tool" "$flag"
done
runs yt-dlp "$contents/Resources/tools/yt-dlp/yt-dlp" --version
gallery=$contents/Resources/tools/gallery-dl/gallery-dl
runs gallery-dl "$gallery" --version

# The app only runs a gallery-dl whose bytes match gallery-dl.sha256 (gallery_process.rs).
expected=$(cut -d' ' -f1 "$contents/Resources/gallery-dl.sha256" 2>/dev/null || true)
actual=$( [[ -f $gallery ]] && shasum -a 256 "$gallery" | cut -d' ' -f1 || true)
if [[ -n $expected && $expected == "$actual" ]]; then pass "gallery-dl matches gallery-dl.sha256"
else fail "gallery-dl matches gallery-dl.sha256"; fi

if [[ -e $contents/MacOS/gallery-dl ]]; then fail "no onefile gallery-dl in Contents/MacOS"
else pass "no onefile gallery-dl in Contents/MacOS"; fi

# Nested tools carry the app's signer (Developer ID team, or ad-hoc for both): a mismatch means
# the --sign-macos step didn't run with the bundle's identity.
team() { codesign -dv "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p'; }
app_team=$(team "$app")
for nested in "$contents/Resources/tools/yt-dlp/yt-dlp" "$gallery"; do
  label=$(basename "$nested")
  nested_team=$( [[ -f $nested ]] && team "$nested" || true)
  if [[ -n $app_team && $nested_team == "$app_team" ]]; then pass "$label signed like the app ($app_team)"
  else fail "$label signed like the app" "app ${app_team:-?}, $label ${nested_team:-?}"; fi
done

if codesign --verify --deep --strict "$app" >/dev/null 2>&1; then pass "codesign verifies the app"
else fail "codesign verifies the app"; fi

if ((failures)); then echo "RESULT $failures check(s) failed"; exit 1; fi
echo "RESULT all checks passed"
