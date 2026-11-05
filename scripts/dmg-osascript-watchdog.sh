#!/usr/bin/env bash
# Tauri's dmg AppleScript waits for Finder's .DS_Store with no time limit, which can hang
# a CI runner. Kill any osascript older than LIMIT seconds (default 300); bundle_dmg.sh
# then ships the plain dmg (no background) instead of hanging. Runs until killed.
LIMIT="${1:-300}"
while sleep "${INTERVAL:-15}"; do
  ps -axo pid=,etime=,comm= | awk -v limit="$LIMIT" '$3 ~ /osascript$/ {
    t = $2; d = 0
    if (t ~ /-/) { split(t, a, "-"); d = a[1]; t = a[2] }
    n = split(t, p, ":"); s = 0
    for (i = 1; i <= n; i++) s = s * 60 + p[i]
    if (s + d * 86400 > limit) print $1
  }' | while read -r pid; do
    echo "dmg watchdog: osascript $pid ran over ${LIMIT}s, killing it (dmg ships without layout)"
    kill "$pid" 2>/dev/null || true
  done
done
