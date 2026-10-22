#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
action="${1:-package}"
case "$action" in build|package|template|check|test|lint|fmt|preview) ;; *) echo "Usage: ./dev.sh {package TV_NAME|template|build|check|test|lint|fmt|preview}"; exit 2;; esac
tv_name="${2:-}"
if [ "$action" = package ]; then
  node -e 'require("./scripts/tv-profile.cjs").name(process.argv[1])' "$tv_name"
fi
repo="$(cd .. && pwd)"
# Assets and compiler caches are shared; serialize builds so catalogs cannot mix.
mkdir -p state
if ! mkdir state/build.lock 2>/dev/null; then
  echo "A build or preview is already running. Stop it before starting another."
  exit 1
fi
trap 'rmdir state/build.lock' EXIT
docker build --platform linux/arm64 -t tvhome-native-builder .
run_args=(--rm --platform linux/arm64)
if [ "$action" = preview ]; then
  run_args+=(-p 127.0.0.1:6081:6080
    -e TVHOME_PREVIEW_BACKGROUNDS -e TVHOME_PREVIEW_BACKGROUND_COLLECTION
    -e TVHOME_PREVIEW_BACKGROUND_DATE -e TVHOME_PREVIEW_BACKGROUND_SECONDS)
fi
docker run "${run_args[@]}" \
  -v "$repo:/workspace" -v tvhome-native-cargo:/usr/local/cargo/registry \
  -v tvhome-native-target:/workspace/tv/target \
  tvhome-native-builder bash scripts/run.sh "$action" "$tv_name"
