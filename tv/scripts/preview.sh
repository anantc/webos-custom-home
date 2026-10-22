#!/bin/bash
set -euo pipefail
export DISPLAY=:99
export LD_LIBRARY_PATH="$PWD/target/release${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
Xvfb :99 -screen 0 1920x1080x24 -ac -noreset >/tmp/tvhome-xvfb.log 2>&1 &
trap 'kill $(jobs -pr) 2>/dev/null || true' EXIT
for attempt in {1..50}; do
  [ -S /tmp/.X11-unix/X99 ] && break
  sleep 0.1
done
x11vnc -display :99 -forever -shared -nopw -localhost -rfbport 5900 >/tmp/tvhome-vnc.log 2>&1 &
node scripts/preview-server.cjs >/tmp/tvhome-preview-server.log 2>&1 &
echo "Preview: http://127.0.0.1:6081/vnc.html?autoconnect=1&resize=scale"
if [ "${TVHOME_PREVIEW_BACKGROUNDS:-}" = local ]; then
  [ -f dist/backgrounds-v1/manifest.json ] || { echo "Prepare photos with node tv/scripts/prepare-backgrounds.cjs first."; exit 1; }
  node scripts/background-server.cjs >/tmp/tvhome-photo-server.log 2>&1 &
  export TVHOME_PREVIEW_BACKGROUND_BASE=http://127.0.0.1:8793/
fi
target/release/tvhome-native --preview --assets assets
