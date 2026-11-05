#!/usr/bin/env bash
# Install-and-run smoke check for one built Linux package (.deb, .rpm or .AppImage).
# Run it as root inside a clean distro container (CI: publish.yml linux-smoke job):
#   scripts/linux-package-smoke.sh <package-file>
# Prints one PASS/FAIL line per check and exits non-zero on any FAIL.
# Network: package-manager installs only.
# Prior art: Motrix scripts/smoke-pacman-package.sh (ldd "not found", tool --version),
# Firelink scripts/verify-linux-packages.js (package paths, desktop entry),
# Zajil ci.yml (xvfb-run open-then-close).
set -euo pipefail

APP=nezzel
TOOLS=(yt-dlp ffmpeg ffprobe aria2c deno gallery-dl)
LIVE_SECONDS=10
START_TIMEOUT=60
STOP_TIMEOUT=15

failures=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+: $2}"; failures=$((failures + 1)); }
check() { # check <name> <cmd...>: PASS when the command succeeds (its output is hidden)
  local name=$1; shift
  if "$@" >/dev/null 2>&1; then pass "$name"; else fail "$name"; fi
}
# App output may carry paths and links: keep the shape, drop the values.
scrub() { sed -E 's#[a-z]+://[^ "]*#<url>#g; s#(/home|/root|/tmp|/var/tmp|/run/user)/[^ :"]*#<path>#g'; }

# --- launch probe: runs inside xvfb-run (DISPLAY is set) -------------------------------
if [[ ${1:-} == --launch-probe ]]; then
  shift
  # Own session = own process group: AppRun and WebKit helpers are signalled together.
  setsid "$@" >/tmp/nezzel-smoke-app.log 2>&1 &
  pid=$!
  trap 'kill -KILL -- "-$pid" 2>/dev/null || true' EXIT
  alive() { kill -0 "$pid" 2>/dev/null; }
  window=''
  for ((i = 0; i < START_TIMEOUT * 2; i++)); do
    alive || break
    if xwininfo -root -tree 2>/dev/null | grep -qi '"nezzel"'; then window=yes; break; fi
    sleep 0.5
  done
  if [[ -z $window ]]; then
    if alive; then echo "probe: no window after ${START_TIMEOUT}s"; else echo "probe: exited before a window opened"; fi
    exit 1
  fi
  for ((i = 0; i < LIVE_SECONDS; i++)); do
    sleep 1
    alive || { echo "probe: exited after ${i}s"; exit 2; }
  done
  kill -TERM -- "-$pid" 2>/dev/null || true
  for ((i = 0; i < STOP_TIMEOUT * 2; i++)); do
    alive || break
    sleep 0.5
  done
  if alive; then
    echo "probe: still running ${STOP_TIMEOUT}s after SIGTERM"
    exit 3
  fi
  status=0
  wait "$pid" || status=$?
  # 0 = handled SIGTERM; 143 = default SIGTERM action. Anything else (abort, segfault) fails.
  if [[ $status -ne 0 && $status -ne 143 ]]; then
    echo "probe: exit status $status after SIGTERM"
    exit 4
  fi
  exit 0
fi

pkg=${1:?usage: linux-package-smoke.sh <package.deb|package.rpm|package.AppImage>}
[[ -f $pkg ]] || { echo "FAIL package file exists"; exit 1; }
pkg=$(cd "$(dirname "$pkg")" && pwd)/$(basename "$pkg")
[[ $(id -u) -eq 0 ]] || { echo "FAIL run as root inside a throwaway container"; exit 1; }
export DEBIAN_FRONTEND=noninteractive
work=''
cleanup() {
  pkill -KILL -x "$APP" 2>/dev/null || true
  [[ -z $work ]] || rm -rf "$work"
}
trap cleanup EXIT

case $pkg in
  *.deb) kind=deb ;;
  *.rpm) kind=rpm ;;
  *.AppImage) kind=appimage ;;
  *) echo "FAIL known package type"; exit 1 ;;
esac

# --- install -----------------------------------------------------------------------
# Test helpers first, so an install failure below is the package's own.
if command -v apt-get >/dev/null; then
  apt-get update -qq >/dev/null 2>&1 || { echo "FAIL apt-get update"; exit 1; }
  apt-get install -y -qq --no-install-recommends xvfb xauth x11-utils procps file >/dev/null 2>&1 \
    || { echo "FAIL install test helpers"; exit 1; }
elif command -v dnf >/dev/null; then
  dnf install -y -q xorg-x11-server-Xvfb xorg-x11-xauth xwininfo procps-ng file >/dev/null 2>&1 \
    || { echo "FAIL install test helpers"; exit 1; }
elif command -v pacman >/dev/null; then
  pacman -Syu --noconfirm --needed xorg-server-xvfb xorg-xauth xorg-xwininfo procps-ng file >/dev/null 2>&1 \
    || { echo "FAIL install test helpers"; exit 1; }
else
  echo "FAIL supported package manager"; exit 1
fi

log=/tmp/nezzel-smoke-install.log
case $kind in
  deb)
    command -v apt-get >/dev/null || { echo "FAIL .deb needs an apt distro"; exit 1; }
    if apt-get install -y --no-install-recommends "$pkg" >"$log" 2>&1; then pass "install deb (dependencies resolved)"
    else fail "install deb (dependencies resolved)" "$(grep -m3 -E 'Depends|E:' "$log" | tr '\n' ' ')"; exit 1; fi
    name=$(dpkg-deb -f "$pkg" Package)
    files=$(dpkg -L "$name")
    root=''
    ;;
  rpm)
    command -v dnf >/dev/null || { echo "FAIL .rpm needs a dnf distro"; exit 1; }
    if dnf install -y "$pkg" >"$log" 2>&1; then pass "install rpm (dependencies resolved)"
    else fail "install rpm (dependencies resolved)" "$(grep -m3 -iE 'nothing provides|error' "$log" | tr '\n' ' ')"; exit 1; fi
    name=$(rpm -qp --qf '%{NAME}' "$pkg")
    files=$(rpm -ql "$name")
    root=''
    ;;
  appimage)
    work=$(mktemp -d)
    cp "$pkg" "$work/app.AppImage" && chmod 0755 "$work/app.AppImage"
    if (cd "$work" && ./app.AppImage --appimage-extract >/dev/null 2>&1) && [[ -x $work/squashfs-root/AppRun ]]; then
      pass "AppImage extracts"
    else fail "AppImage extracts"; exit 1; fi
    # Classic AppDir keeps a usr/ tree; the sharun format (TAURI_BUNDLER_NEW_APPIMAGE_FORMAT)
    # puts bin/, share/ and lib -> shared/lib at the top, with the real binaries in shared/bin.
    if [[ -d $work/squashfs-root/usr ]]; then root=$work/squashfs-root/usr; else root=$work/squashfs-root; fi
    files=$(cd "$root" && find . -mindepth 1 | sed 's|^\.|/usr|')
    root=${root%/usr}
    ;;
esac

if [[ $kind == appimage && ! -d $root/usr ]]; then
  prefix=$root
  bin=$root/shared/bin/$APP
else
  prefix=$root/usr
  bin=$prefix/bin/$APP
fi
toolsdir=$prefix/lib/Nezzel/tools
# Tauri resolves resources under $APPDIR/usr/lib/<product>; accept either AppImage layout.
if [[ $kind == appimage && ! -d $toolsdir && -d $root/usr/lib/Nezzel/tools ]]; then toolsdir=$root/usr/lib/Nezzel/tools; fi

# --- static checks ---------------------------------------------------------------------
check "nezzel binary is executable" test -x "$bin"

# ldd lists unresolved libraries as "not found". For the AppImage use its bundled libs.
if [[ $kind == appimage ]]; then
  # Resolve with the AppImage's own loader and libs: its libc is older than the host's,
  # so LD_LIBRARY_PATH + host ldd would break ldd itself.
  libpath=$(find "$prefix/lib/" -name '*.so*' -printf '%h\n' 2>/dev/null | sort -u | paste -sd:) || true
  loader=$(find "$prefix/lib/" -maxdepth 2 -name 'ld-linux*.so*' 2>/dev/null | head -1) || true
  lddrc=0
  if [[ -n $loader ]]; then lddout=$("$loader" --library-path "$libpath" --list "$bin" 2>&1) || lddrc=$?
  else lddout=$(LD_LIBRARY_PATH=$libpath ldd "$bin" 2>&1) || lddrc=$?; fi
else
  lddrc=0
  lddout=$(ldd "$bin" 2>&1) || lddrc=$?
fi
missing=$({ grep 'not found' <<<"$lddout" || true; } | awk '{print $1}' | paste -sd' ')
if ((lddrc)); then fail "ldd: every library resolves" "ldd exit $lddrc"
elif [[ -z $missing ]]; then pass "ldd: every library resolves"; else fail "ldd: every library resolves" "$missing"; fi

# Files we put in the system /usr/bin clash with distro packages (the AppImage has no system paths).
if [[ $kind != appimage ]]; then
  extra_bin=$({ grep -E '^/usr/bin/.' <<<"$files" || true; } | { grep -vx "/usr/bin/$APP" || true; } | paste -sd' ')
  if [[ -z $extra_bin ]]; then pass "only nezzel in /usr/bin"; else fail "only nezzel in /usr/bin" "$extra_bin"; fi
fi

for tool in "${TOOLS[@]}"; do
  path=$toolsdir/$tool
  if [[ -d $path ]]; then path=$path/$tool; fi # onedir build (yt-dlp/yt-dlp)
  if [[ ! -f $path ]]; then fail "tool $tool present in /usr/lib/Nezzel/tools"; continue; fi
  mode=$(stat -c %a "$path")
  if [[ $mode == 755 ]]; then pass "tool $tool mode 0755"; else fail "tool $tool mode 0755" "mode $mode"; fi
  case $tool in ffmpeg | ffprobe) flag=-version ;; *) flag=--version ;; esac
  # HOME in a scratch dir: deno and yt-dlp may write caches.
  rc=0
  out=$(cd /tmp && HOME=$(mktemp -d) timeout 60 "$path" "$flag" 2>&1) || rc=$?
  if ((rc == 0)) && [[ -n $out ]]; then
    pass "tool $tool runs $flag"
  else fail "tool $tool runs $flag" "exit $rc: $(head -c 200 <<<"$out" | tr '\n' ' ' | scrub)"; fi
done

desktop=$({ grep -E '^/usr/share/applications/[^/]+\.desktop$' <<<"$files" || true; } | head -1)
desktop_file=$prefix${desktop#/usr}
if [[ -n $desktop && -f $desktop_file ]] && grep -qE "^Exec=$APP( |$)" "$desktop_file"; then
  pass "desktop entry (Exec=$APP)"
  icon=$({ grep -m1 '^Icon=' "$desktop_file" || true; } | cut -d= -f2)
  if [[ -n $icon ]] && grep -qE "^/usr/share/icons/hicolor/[^/]+/apps/$icon\.(png|svg)$" <<<"$files"; then
    pass "icon $icon in hicolor"
  else fail "icon in hicolor" "Icon=$icon"; fi
else
  fail "desktop entry (Exec=$APP)"
  fail "icon in hicolor" "no desktop entry"
fi

# --- launch: window opens, stays up, exits cleanly on SIGTERM ---------------------------
if [[ $kind == appimage ]]; then
  launch=(env APPIMAGE_EXTRACT_AND_RUN=1 "$work/app.AppImage")
else
  launch=("$APP")
fi
self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
HOME=$(mktemp -d) XDG_RUNTIME_DIR=$(mktemp -d)
export HOME XDG_RUNTIME_DIR
chmod 0700 "$XDG_RUNTIME_DIR"
if probe=$(xvfb-run -a -s "-screen 0 1600x1000x24" bash "$self" --launch-probe "${launch[@]}" 2>&1); then
  pass "app opens a window, stays up ${LIVE_SECONDS}s, exits cleanly on SIGTERM"
else
  fail "app opens a window, stays up ${LIVE_SECONDS}s, exits cleanly on SIGTERM" "$({ grep '^probe:' <<<"$probe" || true; } | head -1)"
  # Last app log lines, scrubbed of paths and links.
  if [[ -f /tmp/nezzel-smoke-app.log ]]; then tail -n 15 /tmp/nezzel-smoke-app.log | tr -d '\r' | grep -v '^[0-9%]*$' | scrub || true; fi
fi

if ((failures)); then echo "RESULT $failures check(s) failed"; exit 1; fi
echo "RESULT all checks passed"
