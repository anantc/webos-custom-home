# Native app internals

The TV app uses Rust, SDL3 and Skia/OpenGL. Its installed name is **Home** and
application ID is `com.anant.tvhome.native`.

See [Development](../docs/DEVELOPMENT.md) for builds, catalogs, installation on a
test TV and validation. End-user controls are in [Using Home](../docs/USAGE.md).

## Layout and input

The UI uses 1920×1080 coordinates, six columns, a first-row tray with 35.2 px
padding, and bottom-aligned rows. Focus uses a 1.14 scale and a 100 ms tween.
The Wi-Fi control aligns with the tray's left edge and the clock/settings control
with its right edge.

SDL's webOS Back and Exit policy hints must be set before creating the window.
The system Home key remains available to webOS. Held Exit is read through SDL's
webOS-specific scancode.

## Audio

Navigation audio is a generated 40 ms click, cached once and played asynchronously
through SDL. Focus changes have a 60 ms minimum interval. Repeated input at a
grid edge or pointer movement within one target stays silent.

Launching an app or losing focus pauses and clears audio. Audio failures fall
back to silent navigation. Test volume and latency on the TV; VNC preview does
not carry audio.

## Assets

The release template bundles the Inter font, brand artwork and catalog policy.
tvhome setup personalizes it with the target TV's real app and input IDs.
The optional per-TV developer build uses `catalog/tvs/<name>/apps.js`.
Publisher artwork in `catalog/artwork/` can be overridden by matching files in
`native/artwork/`. Edited Apple TV and ZEE5 icons are 512×512.
Provenance and edit instructions are in `native/artwork/SOURCES.json` and
`native/artwork/EDIT-PROMPTS.json`.

The LG Home icon source is `native/packaging/home-icon.svg`. Packaged PNGs are
`native/packaging/icon-flat.png` (80×80) and
`native/packaging/largeIcon-flat.png` (130×130). When editing the source, update
both PNGs. Their opaque black background matches `iconColor: "#000000"` in
`native/packaging/appinfo.json`.

Asset preparation applies ordering and exclusions from `catalog/config.js`,
then removes Home's own app ID. TV package validation rejects preview catalogs
and missing app IDs.

## Catalog updates

tvhome publishes to the installed app's `assets/` directory. Artwork
filenames contain a SHA-256 digest; new files are uploaded and checked before
`catalog.json` is replaced atomically. Existing content-named icons are reused;
older files remain available to a reader finishing the previous catalog.

A worker reads the catalog on foreground return and every five seconds while
visible. It validates the new list and decodes icons off the rendering thread.
Only a complete valid update is applied. Selection follows the same app ID,
or clamps to a remaining app if that app was removed; input-menu selection is
also clamped. Invalid updates retain the last working catalog.

## Wi-Fi service

A worker queries `com.webos.service.wifi/getstatus` with
`{"subscribe":false}` on startup, every ten seconds while foreground, and on
return or opening details. Each call has a five-second timeout; only one request
is in flight. Readings expire after thirty seconds. Errors clear the previous
reading.

The mapping uses `signalLevel`: 0–33 → one bar, 34–65 → two, 66–100 → three.
Missing or invalid readings produce a question mark; a confirmed disconnected
or disabled state produces a slash. Network details stay in memory.

Field handling follows the [Wi-Fi service reference](https://www.webosose.org/docs/reference/ls2-api/com-webos-service-wifi/),
with fallbacks for firmware differences. The
[ConnMan strength scale](https://kernel.googlesource.com/pub/scm/network/connman/connman/+/refs/heads/master/doc/service-api.txt)
is 0–100; [LG's adapter](https://github.com/webosose/webos-connman-adapter/blob/master/src/wifi_service.c)
uses different bar thresholds. Signal level does not measure throughput.

Preview uses labelled example data. To exercise the missing-reading state,
temporarily add `export TVHOME_PREVIEW_WIFI_UNAVAILABLE=1` just before the app
launch in `native/scripts/preview.sh`, then restart `./native/dev.sh preview`.
This sets the variable inside the container where the app runs. Remove the line
and restart to return to the normal fixture.

## Rendering and webOS integration

- SDL delivers input to the app's event loop.
- Skia draws through the webOS SDL fork's native GL surface.
- ARMv7 code uses VFP/NEON instructions with the SDK's soft-float ABI.
  Cortex-A7 is the compilation baseline.
- Image decoding, downsizing and rounded-alpha preparation run once on a worker.
- Icon images and the glow are cached.
- The first draw exercises image, text and panel paths before presentation to
  reduce first-use shader stalls.
- The event loop waits when unchanged and wakes on SDL input.
- Luna calls run in-process on workers, with checked replies and a five-second
  timeout. Each call owns its bus and callback context.

The C compatibility shim supplies libc entry points missing from the TV SDK.
The linker wrapper uses the relocated SDK sysroot and links libstdc++ statically,
so bundled C++ dependencies do not override the TV's C++ runtime.

## Performance checks

Press Blue on the remote or F8 in preview to show drawing statistics. Logs measure
SDL event receipt to GPU submission, and drawing plus swap duration. They exclude
remote transport, earlier input queueing, compositor latency and panel scanout.

On the TV, check repeated taps, held arrows, first navigation after idle, pointer
input, app launch/return, Inputs and Back. Use actual hardware measurements when
assessing responsiveness.

LLVM may warn about the unstable ARM floating-point target features. Those flags
enable hardware floating point while retaining the TV SDK's calling convention.

## Attribution

Platform code is adapted from [Punktfunk](https://github.com/punktfunk/client-webos).
See [NOTICE](NOTICE) and `vendor/punktfunk/` for attribution and licenses.
