#!/bin/bash
# Run manually after packaging. Uses an existing Developer Mode profile only.
set -euo pipefail
cd "$(dirname "$0")"
device="${1:-}"
node -e 'require("./scripts/tv-profile.cjs").name(process.argv[1])' "$device"
package="dist/$device/com.anant.tvhome.native_0.1.7_arm.ipk"
if [ ! -f "$package" ]; then
  echo "Package missing. Run ./dev.sh package $device first."
  exit 1
fi
ares() {
  local action="$1"
  shift
  if command -v "ares-$action" >/dev/null 2>&1; then
    "ares-$action" "$@"
  else
    npx -y -p @webos-tools/cli "ares-$action" "$@"
  fi
}
ares setup-device --listfull | node scripts/tv-profile.cjs "$device"
ares install -d "$device" "$package"
ares launch -d "$device" com.anant.tvhome.native
