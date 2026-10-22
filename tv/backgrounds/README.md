# Scenic backgrounds

The TV downloads reviewed 1920×1080 JPEGs from this project's GitHub release
assets. The package embeds only `manifest.json`, so the first launch needs an
internet connection for its first photo. Each photo includes its photographer,
Unsplash source page and license. The screen displays the photographer credit.

## Calendar and rotation

The TV's local date selects the collection. These are Indian seasonal ranges,
independent of the weather city: February–March spring, April–May summer,
June–September monsoon, October–November autumn, December–January winter.

Holiday collections take priority:

- Halloween: October 27–November 1.
- Christmas: December 19–27.
- Thanksgiving: two days before through two days after the US fourth Thursday
  of November.
- Diwali and Holi: explicit, year-specific windows in `holidays.json`. These take
  precedence over fixed holidays if their windows overlap.

The initial lunar calendar covers 2026–2027: Holi March 1–5 / March 20–24, and
Diwali November 5–10 / October 26–31, respectively.
The [Maharashtra holiday calendar](https://www.mmrda.maharashtra.gov.in/en/public-holidays)
lists Holi on March 3 and Diwali on November 8 in 2026. The 2027 windows use
Drik Panchang’s [Holi](https://www.drikpanchang.com/festivals/holi/festivals-holi-rangwali.html?year=2027)
and [Diwali](https://www.drikpanchang.com/diwali/diwali-puja-calendar.html?year=2027)
calendars. Add verified windows for each new
year; missing years use the seasonal collection rather than reuse old dates.

Home shuffles each collection without repeating a photo until the set is used.
It rotates after ten minutes of foreground time, with a 1.2-second crossfade.
Time spent in another app does not advance rotation. Calendar changes are checked
once a minute while visible. Photo changes send no input or power-management
requests and leave LG's screensaver policy untouched.

## Download and cache

A worker downloads, verifies SHA-256 and decodes photos before they reach the
renderer. Current, previous (during a fade) and next images are the only decoded
photos retained. The JPEG cache holds at most 12 photos / 32 MiB; the displayed
photo and newly downloaded replacement are protected from eviction. Cache files
live under the installed app's `state/backgrounds/` directory, separately on each
TV. The last displayed photo returns on restart, including offline.

If LG leaves the app folder unwritable after an update, Home uses
`/tmp/com.anant.tvhome.native-backgrounds/` with the same cache limits. That
fallback survives an app restart but is cleared by a TV reboot. It uses temporary
memory-backed storage; it does not change system permissions or mounts.

The versioned photo URLs point at the `backgrounds-v1` GitHub release. Home checks
the small manifest on `main` at most daily while preparing photos. A failed
manifest or photo is retried after 15 minutes. Failed photos are skipped; the
current image remains until a verified alternative is ready. If none has ever
been cached, the canvas stays black. HTTP errors are never saved as photos.
Deleting Home removes its app-folder cache; a temporary fallback clears on reboot.

## Preparing and publishing photos

`selection.json` is the reviewed source list, including the replacement SP2.
Only free photos under the [Unsplash License](https://unsplash.com/license) belong
here. This collection supports the launcher; it is not a stock-photo service.

From the repository root:

```sh
node tv/scripts/prepare-backgrounds.cjs
```

The script prepares individual JPEGs, credits, checksums and a manifest in the
ignored `tv/dist/backgrounds-v1/` directory. It verifies existing cached files and
refuses silently changed source images. To deliberately change the selection or
calendar, review the sources and run `--update`; this writes a new manifest
revision to `tv/backgrounds/manifest.json`. Review that diff with the new images.

Before releasing an app with new photos, upload the new content-addressed JPEGs
and `CREDITS.txt` to the `backgrounds-v1` release using GitHub's release editor or
`gh release upload`. Create that asset release the first time. Keep it marked as
not the latest application release. Never replace/delete a JPEG that an existing
manifest references. Publish the manifest on `main` only after its new assets are
available; update the release's manifest and SHA256SUMS for auditing too.
The ordinary application build neither downloads nor bundles these JPEGs.

The initial `backgrounds-v1` tag, and photo-manifest or publishing changes on
`main`, run the **Publish background photos** workflow.
It fetches the approved source images, checks every hash against the reviewed
manifest, and publishes the complete asset set without changing the latest app
release. A rerun skips identical assets and refuses to overwrite different bytes.

## Preview and validation

Prepare the photos first. For a normal development preview, set
`TVHOME_PREVIEW_BACKGROUNDS=local` when running `./tv/dev.sh preview`. This starts a
loopback photo server in the preview container and uses the same download, hash,
decode and cache path as the TV. Its cache is `tv/state/preview-backgrounds/`.

Optional preview-only controls:

- `TVHOME_PREVIEW_BACKGROUND_COLLECTION=diwali` (any collection name).
- `TVHOME_PREVIEW_BACKGROUND_DATE=2026-11-08` (test calendar selection).
- `TVHOME_PREVIEW_BACKGROUND_SECONDS=10` (accelerated rotation; minimum 3).

These are ignored by the installed TV app. Stop the preview's photo server to
check cached rotation and offline recovery. Remove only the preview cache to
exercise a cold start. The regular Rust tests cover calendar boundaries,
foreground timing, shuffle cycles, HTTP 404s, corrupt downloads and cache recovery.
The ignored `all_prepared_photos_have_valid_hashes_and_decode_at_tv_resolution`
test additionally checks every downloaded JPEG; set `TVHOME_BACKGROUND_ASSETS`
to the prepared asset directory when running it with `--ignored`.
