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

## Screensaver and cursor visibility

LG owns the idle timer and screensaver activation. After creating its window,
Home calls SDL's `enable_screen_saver` to release SDL's default inhibition.
SDL's webOS backend handles LG's screensaver permission requests. Home has no
custom inactivity timer or activation calls.

LG sends cursor visibility as raw key codes 1198 (show) and 1199 (hide).
SDL can repeat these notifications without a key-up. Home mirrors their
visibility into SDL's mouse state: otherwise SDL's Wayland backend requests a
visible global pointer when mouse focus leaves Home, dismissing LG's screensaver
during the transition. This behavior was reproduced on webOS 25; synchronizing
the cursor allowed explicit screensaver activation, normal dismissal and pointer
use. LG's automatic idle activation was then verified on the same TV with Home's
custom timer and activation calls removed.

Startup logs record whether SDL accepted the screensaver permission change.
This confirms SDL's state, not that LG has activated a screensaver. The emulator
can exercise the UI but cannot validate LG's automatic screensaver behavior.

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

## Pinned inputs

Input pins are saved in `state/preferences.json` inside the installed app.
The package includes a writable `state/` directory because webOS launches
native apps as an unprivileged user. The tvhome installer must retain directory
entries and their permissions when it personalizes the IPK. The native app
writes a temporary file, flushes it and renames it on a worker thread; the UI commits the toggle after a successful write.
Failures leave the previous pins visible and show an error. These settings are
separate from the tvhome-published `assets/catalog.json`.

The displayed catalog overlays pinned inputs on the latest source catalog,
using input app IDs as stable identities. Pins follow their saved order after
apps, without duplicating existing tiles. Input names track the current catalog;
absent inputs retain their saved pin and reappear when available again. Launches
use the same Luna application launch path as the Inputs menu.

Preview includes four example HDMI inputs and saves pins separately in
`native/state/preview-preferences.json`. Test Right/Left between label and pin,
OK and pointer toggles, tile launch logging, restart persistence, catalog reloads
and failed saves. TV filesystem permissions still require a hardware check.

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

## Speed test

The Wi-Fi panel opens a native test screen. Only Run speed test starts network
traffic. A worker owns a current-thread Tokio runtime and reqwest with Rustls
and bundled certificate roots; the SDL/render thread only reads snapshots.

Discovery reads Fast.com's current app script and token, then calls its v2
server-discovery endpoint. Tokens and signed target URLs stay in memory. Only
HTTPS Netflix CDN range URLs are accepted, redirects are disabled, and discovery
responses are size-limited. This is an undocumented protocol: changes fail with
an error rather than falling back to fixed URLs or stale results.

After warming TLS, latency is the median of three empty POST round trips.
Four parallel connections download for ten seconds, then upload for ten seconds.
Download counts received chunks; upload counts only successful, acknowledged
requests, including the time to finish in-flight uploads. Upload requests adapt
towards 250 ms, capped at 4 MiB, sharing one buffer. Responses are discarded.
These are Home's measurements using Fast.com infrastructure, not a reproduction
of the website's adaptive algorithm. Latency is HTTP round-trip time, not ICMP
ping or loaded latency. TLS and CPU limits can affect results on TV hardware.

Discovery, latency and upload requests have eight-second total timeouts. Download
requests override that limit so their bodies can keep streaming for the full
ten-second measurement; its deadline cancels unfinished responses normally.
The complete test has a 65-second deadline. Errors identify timeouts, connection
failures and HTTP status codes without displaying signed server URLs. Cancel,
Back, focus loss and exit drop pending requests and worker tasks. Updates use a
single latest-snapshot slot so the render thread cannot accumulate a backlog.
Completed results are atomically saved to `state/speedtest-result.json` inside
the installed app; errors and cancellation preserve the previous completed result.

The emulator uses real internet transfers through the computer running Docker,
labelled **Emulator connection**, independently of its example Wi-Fi readings.
It saves to `native/state/preview-speedtest-result.json`. Check start, progress,
cancel/Back, failure, retry and result persistence in preview; check throughput
and TLS compatibility separately on hardware before treating it as calibrated.

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
