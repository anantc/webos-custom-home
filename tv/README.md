# TV app internals

The TV app uses Rust, SDL3 and Skia/OpenGL. Its installed name is **Home** and
application ID is `com.anant.tvhome.native`.

See [Development](../docs/DEVELOPMENT.md) for builds, catalogs, installation on a
test TV and validation. End-user controls are in [Using Home](../docs/USAGE.md).

## Layout and input

The UI uses 1920×1080 coordinates, six columns, a first-row tray with 35.2 px
padding, and bottom-aligned rows. Focus uses a 1.14 scale and a 100 ms tween.
Weather and the time, settings, Wi-Fi group share the top edge. Wi-Fi aligns with
the tray's right edge. The clock is right-aligned so it keeps the same 40 px gap
to settings as settings has to Wi-Fi, regardless of the time's width. Both icons
use 32 px bounds with the same focus scale. Header controls have no background
containers. Settings and Wi-Fi panels anchor their top-right corner to the
bottom-right of their icon bounds; Inputs and Speed test keep their parent's
anchor. Rendering and pointer targets share these positions.

Confirm presses activate on release. Holding OK or the pointer button for 700 ms
on a tile enters move mode and consumes that release, preventing an accidental
launch. Arrows insert the selected tile at its new position; OK saves and Back
discards the draft. Pointer movement does not change the tile being moved.
Focus loss clears pending confirms and cancels an unsaved move.

Move mode gently rotates tiles around their centers with a 1.1-degree maximum
and phases derived from their IDs. The selected tile keeps its focus scale.
Tile positions follow app/input IDs through 180 ms eased slides. Reordering,
including row crossings, retargets from each tile's current position so rapid
arrow presses remain continuous. Back slides tiles back into the saved order;
catalog refreshes and saves preserve any animation already in progress. Scroll
and focus scale also retain their animations across reorders.
Only visible tiles are drawn, using the existing cached artwork; continuous
redrawing ends when move mode and its transitions finish, and drawing pauses
while unfocused.

SDL's webOS Back and Exit policy hints must be set before creating the window.
The system Home key remains available to webOS. Held Exit is read through SDL's
webOS-specific scancode.

## Screensaver and cursor visibility

LG owns the idle timer and screensaver activation. After creating its window,
Home calls SDL's `enable_screen_saver` to release SDL's default inhibition.
SDL's webOS backend handles LG's screensaver permission requests. Home has no
custom inactivity timer or activation calls.

LG sends cursor visibility as raw key codes 1198 (show) and 1199 (hide).
They describe the compositor's state and may have no key-up. Home enables the
local SDL patch in `patches/sdl-compositor-cursor.patch` before creating its
window. The patch updates SDL's cursor state and image directly, consumes these
notifications before keyboard auto-repeat, and prevents focus redraws from
sending global show/hide requests back to LG. Ordinary pointer motion, clicks
and the system cursor artwork still use SDL's normal Wayland handling.

Previously, Home called `show_cursor` for each notification. That API also tells
LG to change global pointer visibility; focus transitions and notification
repeats could make the two disagree. Keeping LG in charge also avoids a global
show request dismissing its screensaver or interfering with a content app.
The host emulator uses stock SDL, so changes here need a physical TV check.
With this patch, Magic Remote navigation, returning from a streaming app and
LG’s automatic clock screensaver were verified on webOS 25.

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
`tv/artwork/`. Edited Apple TV and ZEE5 icons are 512×512.
Provenance and edit instructions are in `tv/artwork/SOURCES.json` and
`tv/artwork/EDIT-PROMPTS.json`.

The LG Home icon source is `tv/packaging/home-icon.svg`. Packaged PNGs are
`tv/packaging/icon-flat.png` (80×80) and
`tv/packaging/largeIcon-flat.png` (130×130). When editing the source, update
both PNGs. Their opaque black background matches `iconColor: "#000000"` in
`tv/packaging/appinfo.json`.

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

## Tile preferences

Input pins and tile order are saved per TV in `state/preferences.json` inside
the installed app. Existing pin-only files load with the default catalog order.
The package includes a writable `state/` directory because webOS launches
native apps as an unprivileged user. The tvhome installer must retain directory
entries and their permissions when it personalizes the IPK. The native app
writes a temporary file, flushes it and renames it on a worker thread; the UI commits the toggle after a successful write.
Pin failures leave the previous pins visible; order-save failures keep move mode
open for retry or cancellation. These settings are
separate from the tvhome-published `assets/catalog.json`.

The displayed catalog overlays pinned inputs on the latest source catalog,
using input app IDs as stable identities. Saved tile order is applied after pins,
so apps and inputs can be mixed. New tiles append in catalog order. Missing IDs
retain their saved slots; history is bounded to 1024 IDs, preserving visible tiles.
Unpinning removes that input's order entry. Input names track the current catalog;
absent inputs retain their saved pin and reappear when available again. Launches
use the same Luna application launch path as the Inputs menu.

Catalog refreshes merge into an active move draft by ID. Removing the picked tile
cancels the draft. Artwork is remapped from source IDs after each reorder and
refresh, rather than relying on catalog indices. Saves run on the same worker as
pin updates; move controls wait for the write result before allowing another edit.

Preview includes four example HDMI inputs and saves pins separately in
`tv/state/preview-preferences.json`. Test Right/Left between label and pin,
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
launch in `tv/scripts/preview.sh`, then restart `./tv/dev.sh preview`.
This sets the variable inside the container where the app runs. Remove the line
and restart to return to the normal fixture.

## Weather and air quality

The open weather layout occupies the space above the tray; it compacts above
three rows and hides while the app grid is scrolled. Up from the first three
columns selects weather; columns four/five select settings, and six selects Wi-Fi.
Left/Right traverse weather, settings and Wi-Fi; Down returns to the grid. The details panel supports
pointer and remote controls. Weather fetching does not reset LG's idle timer.

`src/weather.rs` owns a background thread and current-thread Tokio runtime.
IPWhois (`https://ipwho.is/`) supplies city and approximate coordinates from the
TV's public IP. This stays in memory for six hours. Open-Meteo's forecast and
air-quality endpoints receive those coordinates in parallel. Requests have a
five-second connection limit, twelve-second whole-request limit, 256 KiB body
limit and no redirects; the entire operation has a 25-second deadline.

Only one update runs at a time. Polling starts while Home is foreground, every
15 minutes, with five-minute retries and a 30-second manual-refresh guard.
No requests start while Home is backgrounded; an in-flight request can finish.
Weather and AQI parse independently, so one failed provider need not hide the
other. Responses require valid timestamps; readings expire after two hours.
Automatic IP-location results and provider responses stay in memory. A manually
selected city's name, region, country and coordinates are saved in that TV's
preferences alongside pins and tile order. Saving must succeed before the
weather location changes. Switching city clears the previous readings and uses
a request generation to discard old in-flight responses. Automatic mode removes
the saved override and resumes IP detection.

`src/city.rs` implements the remote/pointer keyboard and bounded asynchronous
[Open-Meteo geocoding search](https://open-meteo.com/en/docs/geocoding-api).
Search runs only when requested, returns up to five cities with state/country,
and uses the same HTTPS, timeout and response-size limits as weather fetching.
Editing or leaving search invalidates pending results. Physical keyboards use
SDL text input only while the picker is open. Search terms and results are not
persisted; only a selected city is saved. The picker attributes Open-Meteo and
GeoNames.

Indian AQI is a **model estimate** from CAMS concentrations, not station data.
The implementation uses PM2.5, PM10, NO₂, SO₂, CO and O₃, applying
[CPCB's National AQI report, table 3.11 and section 3.5](https://airquality.cpcb.gov.in/ccr_docs/FINAL-REPORT_AQI_.pdf).
It averages the preceding 24 hourly samples (at least 16 valid) for PM and
NO₂/SO₂, and eight complete hours for CO/O₃. CO is converted from µg/m³ to mg/m³;
ozone's upper bands use the one-hour reading. At least three pollutants,
including PM2.5 or PM10, are required. The largest sub-index wins. Beyond the
last finite breakpoint it displays 400+ rather than extrapolating the open-ended
severe category. Lead and ammonia are not supplied for this calculation.

Provider references: [Open-Meteo weather](https://open-meteo.com/en/docs),
[Open-Meteo/CAMS air quality](https://open-meteo.com/en/docs/air-quality-api),
[IPWhois](https://ipwhois.io/documentation). The free Open-Meteo endpoint is for
non-commercial use; commercial distributions need an appropriate provider plan.
The details screen attributes Open-Meteo and CAMS. HDMI and weather vectors use
Lucide paths, parsed once at startup; licenses are in `vendor/lucide/`.

Preview uses real providers by default, including the preview host's approximate
IP location. Set `TVHOME_PREVIEW_WEATHER=sample` or `unavailable` inside the
preview container to test deterministic states. Sample readings are labelled.
The ignored `weather::tests::live_providers_return_current_conditions_and_indian_aqi`
test explicitly exercises live HTTPS; ordinary tests stay offline.

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
It saves to `tv/state/preview-speedtest-result.json`. Check start, progress,
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
