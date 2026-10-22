# Scenic backgrounds

The TV downloads reviewed 1920×1080 JPEGs from this project's GitHub release
assets. The package embeds only `manifest.json`, so the first launch needs an
internet connection for its first photo. Each photo includes its photographer,
Unsplash source page and license. The screen displays its Unsplash caption, when
available, followed by the photographer credit. Captions are short excerpts of
the public photo page's `ImageObject.caption`, capped at 160 characters and 25
words; they are separate from our curated `title`. Longer excerpts end in an
ellipsis, and the renderer shortens them further if needed to fit the screen.

## Calendar and rotation

The TV's local date selects a tag. These are Indian seasonal ranges,
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
renderer. Home keeps the displayed photo and one ready for the next transition.
After the crossfade finishes, it fetches the next replacement. Only after that
photo has downloaded, passed verification and been saved does it delete the old
photo. Failed downloads leave the existing photos intact. A completed cache holds
two JPEGs; a third can exist briefly while saving the replacement. Each JPEG is
limited to 4 MiB. The next decode waits until the crossfade has released the old
image, keeping at most two decoded photos. Cache files
live under the installed app's `state/backgrounds/` directory, separately on each
TV. The last displayed photo returns on restart, including offline.

If LG leaves the app folder unwritable after an update, Home uses
`/tmp/com.anant.tvhome.native-backgrounds/` with the same cache limits. That
fallback survives an app restart but is cleared by a TV reboot. It uses temporary
memory-backed storage; it does not change system permissions or mounts.

The photo URLs and remotely fetched `manifest.json` point at the `backgrounds-v1`
GitHub release. Home checks the manifest on startup, then when preparing a photo
if at least 15 minutes have elapsed. With ten-minute rotation, this normally
means a check every 20 minutes while Home is visible. Returning from another app
resumes rotation and these checks. The app uses a saved manifest, or its bundled
seed, if the remote copy is unavailable or invalid. No photos are bundled.
A failed manifest or photo is retried after 15 minutes. Failed photos are skipped; the
current image remains until a verified alternative is ready. If none has ever
been cached, the canvas stays black. HTTP errors are never saved as photos.
Deleting Home removes its app-folder cache; a temporary fallback clears on reboot.

## Preparing and publishing photos

`selection.json` is the reviewed source list, including the replacement SP2.
Only free photos under the [Unsplash License](https://unsplash.com/license) belong
here. This collection supports the launcher; it is not a stock-photo service.

Each entry has a `tags` array. Selection recognizes `spring`, `summer`, `monsoon`,
`autumn`, `winter`, `diwali`, `halloween`, `christmas`, `thanksgiving` and `holi`.
A photo may have multiple selection tags, for example `["winter", "christmas"]`.
Other descriptive tags are allowed but do not select a photo by themselves;
every entry needs at least one recognized tag. Keep 1–12 photos per recognized
tag. The same JPEG can belong to several groups without being downloaded twice.
The optional `caption` is shown alongside `author`; omit it if the source has
no caption. `title` is an editorial label used in the credits file.

The manifest also carries the Diwali/Holi date windows, photo URLs, checksums,
credits and tags. Updating these fields and publishing a new revision changes
the TV's lineup without an application build or installation. New tag meanings
would need application code; changing photos assigned to existing tags does not.

From the repository root:

```sh
node tv/scripts/prepare-backgrounds.cjs
```

The script prepares individual JPEGs, credits, checksums and a manifest in the
ignored `tv/dist/backgrounds-v1/` directory. It reuses verified cached or published
JPEGs before requesting the Unsplash source, and refuses silently changed source
images. Use a new photo ID for a replacement image. To deliberately change the selection or
calendar, review the sources and run `--update`; this writes a new manifest
revision to `tv/backgrounds/manifest.json`. Review that diff with the new images.

Commit the selection, calendar and generated manifest to `main`. The **Publish
background photos** workflow fetches and verifies the reviewed JPEGs, uploads any
new files, updates credits/checksums, and replaces the remote manifest last. That
order ensures all photos exist before TVs see the new lineup. Check that the
workflow succeeds; a source commit alone does not update the live manifest.

The workflow also runs for the initial `backgrounds-v1` tag or a manual dispatch.
It skips identical assets and refuses to overwrite different JPEG bytes. Never
replace/delete a JPEG referenced by an existing manifest; changed photos get new
content-addressed filenames. Metadata can be replaced. The asset release remains
separate from the latest application release, and the ordinary application build
neither downloads nor bundles these JPEGs.

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
