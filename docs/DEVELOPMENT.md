# Development

This guide covers source changes, builds and validation. End-user installation
is in [TV setup](SETUP.md). Commands below run from the repository root.

## Source and tools

```sh
git clone https://github.com/anantc/webos-custom-home.git
cd webos-custom-home
```

| Work | Tools |
| --- | --- |
| TV app build, preview and checks | ARM64 computer, Docker, Node.js 20+ |
| Pair, install and inspect a test TV | Released `tvhome` executable |
| tvhome source builds | Stable Rust; macOS Command Line Tools or Linux C toolchain, Perl and pkg-config |

The Docker builder targets an ARM64 host and produces an ARMv7 webOS executable.
Its first run downloads the toolchain, SDL and Skia. Cargo uses committed
lockfiles; the SDK download follows webosbrew's latest release, so a clean Docker
build can select a newer SDK.

## Code map

| Path | Responsibility |
| --- | --- |
| `tv/src/main.rs`, `model.rs` | Input, focus, layout, panels and scheduling |
| `tv/src/backgrounds/`, `tv/backgrounds/` | Seasonal calendar, photo downloads, cache and reviewed photo manifest |
| `tv/src/catalog.rs` | Validated catalog reloads and background icon decoding |
| `tv/src/render.rs`, `audio.rs`, `gl.rs` | Drawing, sound and GPU context |
| `tv/src/platform/` | webOS services and compatibility layer |
| `cli/src/common.rs` | Named profiles, private files, locks and HTTP |
| `cli/src/self_install.rs` | Install the computer executable, notices and shell command path |
| `cli/src/ssh.rs`, `setup.rs` | SSH transport, key retrieval, installation and setup prompts |
| `cli/src/package.rs` | Embedded IPK validation, personalization and catalog policy |
| `cli/src/tv.rs`, `sync.rs`, `inventory.rs` | Pairing, app capture, per-TV inventory and atomic publishing |
| `cli/src/watch.rs`, `renew.rs`, `install.rs` | Background jobs, health and service installation |
| `catalog/config.js`, `presentation.js` | App ordering, exclusions and branding |
| `tv/scripts/` | Asset preparation, package checks and preview server |
| `tests/` | Downloads, catalogs, target selection, packages and SSH fixtures |

## Build a setup template

```sh
./tv/dev.sh template
```

This creates a generic IPK and `home-licenses.txt` in `tv/dist/template/`.
The template has the native executable, font, brand artwork and policy, but no
TV-specific IDs or credentials. tvhome setup reads the target TV and replaces
the empty template catalog before installation. The template is not installed
directly.

Build/preview commands share generated assets and compiler caches.
`tv/dev.sh` serializes them with `tv/state/build.lock`. Stop a preview
before building. If interruption leaves the lock directory, confirm its container
has stopped before removing that empty directory.

## Build and test tvhome

After building the template, collect the license notices that the executable embeds:

```sh
node cli/licenses.cjs
cargo build --locked --release --manifest-path cli/Cargo.toml --features portable,bundled-home
./cli/target/release/tvhome --help
```

`bundled-home` embeds the template selected by `cli/build.rs`.
`TVHOME_IPK` can override its path at build time. The `portable` feature bundles
OpenSSL; SSH is provided by libssh2. macOS HTTP uses system TLS.

On a test TV, follow [TV setup](SETUP.md) using this executable. For a locally
built template override:

```sh
./cli/target/release/tvhome --tv living-room setup --package tv/dist/template/com.anant.tvhome.native_0.1.6_arm.ipk
```

Stop that TV's jobs before rerunning setup. The override affects that installation;
the watcher's catalog policy comes from the template embedded in its executable.
Rebuild tvhome when changing ordering or brand policy.

Launch through the installed app: Luna permissions depend on the executable's
installed path. Test app launches, inputs, navigation, Back, sound, foreground
return and catalog additions/removals on the TV.

## Catalog policy and artwork

Edit `catalog/config.js`:

- `preferredApps`: brand keys to place first.
- `dockApps`: exact TV app IDs to place before preferred brands.
- `hiddenApps`: exact IDs to exclude from Home.

Brand keys and matching expressions are in `catalog/presentation.js`.
Use `tvhome --tv living-room apps --refresh` to inspect real installed IDs.
Developer Mode and Home are always excluded from the launcher grid.
These settings affect the launcher, not apps installed on the TV.

Artwork in `tv/artwork/` overrides matching brand files in `catalog/artwork/`.
See [TV app assets](../tv/README.md#assets) for provenance and icon editing.

### Optional per-TV development package

For inspecting the generated assets of one TV:

```sh
tvhome --tv living-room sync
./tv/dev.sh package living-room
```

`sync` exports actual launch IDs, inputs and fallback icons into
`catalog/tvs/living-room/`. Its `tv.json` records the profile name and address.
The resulting `tv/dist/living-room/` package is already personalized.
Catalogs and per-TV packages are ignored by Git.

The optional `tv/install.sh living-room` helper uses LG's CLI for this
development-only package path. Register its separate CLI target first:

```sh
npm install --global @webos-tools/cli
ares-setup-device --add living-room -i "host=TV_ADDRESS" -i "port=9922" -i "username=prisoner"
ares-novacom --device living-room --getkey
./tv/install.sh living-room
```

Replace `TV_ADDRESS` with the intended test TV's address. Enable Key Server
before `--getkey`; enter its displayed passphrase. The `ares-` commands are
LG's CLI, which keeps names in `~/.webos/tv/novacom-devices.json` and keys in
`~/.ssh/<name>_webos`. The install helper checks that CLI target against the
captured `tv.json`. This separate developer helper does not consume the
tvhome's profiles. Each later `--device living-room` uses LG's saved entry.
Use tvhome template workflow above for release builds.

## Preview

```sh
./tv/dev.sh preview
```

Open [the local preview](http://127.0.0.1:6081/vnc.html?autoconnect=1&resize=scale).
It displays example apps on a virtual 1080p screen. Arrows, Return, Escape and
pointer input work through VNC. Stop with Ctrl+C.

VNC does not carry navigation sound. Software-rendered preview timings do not
represent TV GPU performance. [TV app internals](../tv/README.md) describes
instrumentation and Wi-Fi fixtures.

Scenic photo preparation, calendar maintenance, release assets and preview controls
are in [Scenic backgrounds](../tv/backgrounds/README.md).

## Validation

tvhome formatting, tests and lint:

```sh
node cli/licenses.cjs
cargo fmt --manifest-path cli/Cargo.toml --check
cargo test --locked --manifest-path cli/Cargo.toml --features portable,bundled-home
cargo clippy --locked --manifest-path cli/Cargo.toml --features portable,bundled-home --all-targets -- -D warnings
node --test tests/*.test.cjs
```

Native UI, adapters and ARMv7 lint:

```sh
./tv/dev.sh test
./tv/dev.sh lint
```

The SSH integration fixture starts a local RSA SSH server and exercises actual
authentication, uploads, fragmented install-status replies and unknown-host
rejection. Run it in the native builder, whose developer tooling includes the
Node SSH server library:

```sh
docker run --rm --platform linux/arm64 \
  -v "$PWD:/workspace" -w /workspace \
  tvhome-native-builder node tests/ssh-fixture.cjs
```

The fixture invokes Cargo and requires the template to exist. It does not connect
to a real TV. A developer installation elsewhere can point `TVHOME_SSH2_MODULE`
to the `ssh2` Node module for this fixture.

Firmware integration, permissions on the installed asset directory and physical
remote responsiveness also need verification on the TV.

## Publishing

See [Releases](RELEASES.md) for versioning, platform builds and publication.
