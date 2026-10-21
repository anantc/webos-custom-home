#!/bin/bash
set -euo pipefail
mkdir -p .toolchains
ln -sfn "$TVHOME_WEBOS_NDK" .toolchains/arm-webos-linux-gnueabi_sdk-buildroot
action="$1"
tv_name="${2:-}"
case "$action" in
fmt) cargo fmt; exit;;
test|preview)
  export SKIA_BINARIES_URL='https://github.com/rust-skia/skia-binaries/releases/download/0.153.3/skia-binaries-{key}.tar.gz'
  if [ "$action" = test ]; then
    node --test scripts/preview-server.test.cjs
    cargo test --locked --features host-sdl
    exit
  fi
  node scripts/prepare-assets.cjs --preview
  cargo build --locked --release --features host-sdl
  exec bash scripts/preview.sh
  ;;
lint) cargo fmt --check; cargo clippy --locked --target armv7-unknown-linux-gnueabi --all-targets -- -D warnings; exit;;
check) cargo check --locked --target armv7-unknown-linux-gnueabi; exit;;
esac
cargo build --locked --release --target armv7-unknown-linux-gnueabi
if [ "$action" = build ]; then exit; fi
if [ "$action" = template ]; then
  node scripts/prepare-template.cjs
  output="dist/template"
else
  node scripts/prepare-assets.cjs "$tv_name"
  output="dist/$tv_name"
fi
mkdir -p "$output"
stage="$(mktemp -d "$output/stage.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/bin" "$stage/lib"
cp target/armv7-unknown-linux-gnueabi/release/tvhome-native "$stage/bin/"
cp -L "$SDL3_PREFIX/lib/libSDL3.so.0" "$stage/lib/"
cp -L "$TVHOME_WEBOS_NDK/arm-webos-linux-gnueabi/sysroot/usr/lib/libfreetype.so.6" "$stage/lib/"
"$TVHOME_WEBOS_NDK/bin/arm-webos-linux-gnueabi-strip" "$stage/bin/tvhome-native" "$stage/lib/"*
cp packaging/appinfo.json "$stage/"
cp packaging/icon-flat.png packaging/largeIcon-flat.png "$stage/"
cp artwork/SOURCES.json "$stage/artwork-sources.json"
cp -R assets "$stage/"
cp -R vendor "$stage/"
cp ../LICENSE "$stage/LICENSE"
cp NOTICE "$stage/"
node scripts/collect-licenses.cjs "$stage"
if [ "$action" = template ]; then
  node scripts/template-notices.cjs "$stage" "$output/home-licenses.txt"
fi
ares-package "$stage" -o "$output"
if [ "$action" = template ]; then
  node scripts/verify-package.cjs "$output/com.anant.tvhome.native_0.1.0_arm.ipk" --template
else
  node scripts/verify-package.cjs "$output/com.anant.tvhome.native_0.1.0_arm.ipk"
  cp "../catalog/tvs/$tv_name/tv.json" "$output/tv.json"
fi
echo "Package ready in native/$output."
