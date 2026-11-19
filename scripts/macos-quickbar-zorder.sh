#!/usr/bin/env bash
# macOS repro for issue #77: the quick bar must not raise Nezzel's main window over the frontmost app.
# Presses the quick-download shortcut (Option+Shift+D) three times with Esc between, listing the on-screen
# layer-0 windows front to back after each step. Needs a running Nezzel (installed or dev build) that owns
# the shortcut, and Accessibility permission for the terminal (System Events keystrokes).
# Usage: scripts/macos-quickbar-zorder.sh [target app, default: Finder]
# PASS: the bar shows on each press and hides on each Esc, no Nezzel window >= 800 px wide sits above the target app's front window in any listing, and the
# target app is frontmost again after each Esc.
set -u
target="${1:-Finder}"
previous=$(osascript -e 'tell application "System Events" to get name of first process whose frontmost is true')
opened_finder=0
fail=0

list() {
  osascript -l JavaScript - "$1" "$target" <<'JXA'
ObjC.import('CoreGraphics');
function run(argv) {
  const [step, target] = argv;
  // kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, front to back.
  const all = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1 | 16, 0)));
  // The bar floats above layer 0 at 620 px wide.
  const bar = all.some(w => /nezzel/i.test(w.kCGWindowOwnerName) && w.kCGWindowLayer > 0 && w.kCGWindowBounds.Width === 620);
  const windows = all.filter(w => w.kCGWindowLayer === 0)
    .map(w => ({ owner: w.kCGWindowOwnerName, width: w.kCGWindowBounds.Width, height: w.kCGWindowBounds.Height }));
  const targetAt = windows.findIndex(w => w.owner === target);
  const raised = windows.slice(0, targetAt < 0 ? windows.length : targetAt)
    .filter(w => /nezzel/i.test(w.owner) && w.width >= 800);
  const shown = windows.slice(0, 6).map(w => `${w.owner} ${w.width}x${w.height}`).join(', ');
  const barWrong = step.startsWith('press') !== bar;
  return `${raised.length || barWrong ? 'BAD ' : 'ok  '}${step}: bar ${bar ? 'shown' : 'hidden'}; ${shown}`;
}
JXA
}

frontmost() {
  osascript -e 'tell application "System Events" to get name of first process whose frontmost is true'
}

check_list() {
  local line
  line=$(list "$1")
  echo "$line"
  [[ $line == BAD* ]] && fail=1
}

check_front() {
  local front
  front=$(frontmost)
  if [[ $front == "$target" ]]; then
    echo "ok  $1: frontmost $front"
  else
    echo "BAD $1: frontmost $front, expected $target"
    fail=1
  fi
}

check_bar_focus() {
  local front
  front=$(frontmost)
  if [[ $front == [Nn]ezzel ]]; then
    echo "ok  $1: frontmost $front"
  else
    echo "BAD $1: frontmost $front, expected Nezzel (the bar)"
    fail=1
    return 1
  fi
}

press() { osascript -e 'tell application "System Events" to keystroke "d" using {option down, shift down}'; sleep 1.5; }
esc() { osascript -e 'tell application "System Events" to key code 53'; sleep 1; }

if [[ $target == Finder ]] && [[ $(osascript -e 'tell application "Finder" to count of Finder windows') == 0 ]]; then
  opened_finder=$(osascript -e 'tell application "Finder" to id of (make new Finder window)')
  # Close only the window this script opened, also when interrupted.
  trap 'osascript -e "tell application \"Finder\" to close (every Finder window whose id is $opened_finder)" >/dev/null 2>&1' EXIT
fi
osascript -e "tell application \"$target\" to activate"
sleep 1
check_list "start"
for round in 1 2 3; do
  press
  check_list "press $round"
  # Another app took focus: Esc would land there, so stop (the bar may stay open; close it by hand).
  check_bar_focus "press $round" || { echo "aborted: Esc not sent"; break; }
  esc
  check_list "esc $round"
  check_front "esc $round"
done
osascript -e "tell application \"System Events\" to set frontmost of process \"$previous\" to true" >/dev/null 2>&1
if [[ $fail == 0 ]]; then echo PASS; else echo FAIL; fi
exit "$fail"
