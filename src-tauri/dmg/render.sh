#!/bin/sh
# Renders background.html to background.png (@1x), background@2x.png and the
# HiDPI background.tiff that tauri.macos.conf.json points at. macOS only; uses the
# system Google Chrome (override with CHROME=/path/to/chrome) and tiffutil.
set -eu
cd "$(dirname "$0")"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
for s in 1 2; do
  out=background.png; [ "$s" = 2 ] && out=background@2x.png
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files \
    --force-device-scale-factor="$s" --window-size=720,472 \
    --screenshot="$PWD/$out" "file://$PWD/background.html" >/dev/null 2>&1
done
tiffutil -cathidpicheck background.png background@2x.png -out background.tiff >/dev/null
sips -g pixelWidth -g pixelHeight background.png background@2x.png | grep pixel
