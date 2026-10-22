# Publishing releases

This is the maintainer guide. End-user installation is in [TV setup](SETUP.md).

## Build pipeline

The `Build and release` workflow first builds and validates the generic Home
template on an ARM64 Linux runner. It then embeds that same payload into tvhome
builds on four native runners: macOS ARM64/x64 and Linux ARM64/x64.

Linux uses Ubuntu 22.04 for glibc 2.35 compatibility. The `portable` feature
bundles OpenSSL and compression support; release checks reject dynamic
OpenSSL/libssh2/zlib dependencies.
macOS sets a deployment target of 13.0 and applies an ad-hoc signature.
SSH/RSA and package parsing are implemented in the Rust executable.

Before compiling, `node cli/licenses.cjs` collects notices for the locked
host dependencies and the Home payload into `cli/target/release-licenses.txt`.
The build embeds these notices alongside Home's generic IPK. `TVHOME_LICENSES`
can select another notice file. `--licenses` prints the embedded notices;
`--install` writes them with the installed executable.

Each platform publishes a plain `tvhome-PLATFORM` executable, its `.sha256`, and
the notices as `.licenses.txt` with a checksum. `release.cjs` verifies that the
binary embeds the current Home package and exactly matches the prepared notices.

The generic payload contains no TV data. Runtime setup retrieves that TV's app
IDs and builds its personalized IPK. Personalized packages, keys and inventories
must stay out of public releases.

## Publish a version

1. Update `cli/Cargo.toml` and `cli/Cargo.lock`.
2. If the TV app changed, update its Cargo manifest/lock, packaging metadata,
   and package paths in build/install scripts and tests. Update the default
   template path in `cli/build.rs`.
3. Run [development checks](DEVELOPMENT.md#validation), test on a TV, then commit.
4. Push a new `vX.Y.Z` tag matching the tvhome manifest.

Replace `X.Y.Z` with the version being published:

```sh
git push origin main
git tag vX.Y.Z
git push origin vX.Y.Z
```

Publication waits for all platform builds, verifies checksums, attaches files to
a draft release and publishes it. A manual Actions run produces downloadable
workflow artifacts without publishing.

## Verify the download

There are four files per platform, sixteen total. Follow the exact platform
verification and `--install` commands in [TV setup](SETUP.md#verify-and-install)
on a clean target computer. Open a new terminal and check `tvhome --version`,
`tvhome --help` and `tvhome tvs`. Confirm setup and each optional background
feature work on that computer.

From a developer checkout, the optional Node download helper verifies the flat
executable and notices before installing:

```sh
node cli/install.cjs --version X.Y.Z --dir /tmp/tvhome-release-check
/tmp/tvhome-release-check/tvhome --version
```

## Update compatibility

`tvhome update` resolves the latest published stable `vX.Y.Z` release through the
GitHub API. Preserve the platform executable and `.sha256` asset names. The
updater checks the checksum and reported version, then runs the verified binary
with the internal `update --apply` handoff while its parent holds the update
lock. The new binary supplies its own embedded Home package and notices. Keep
this handoff compatible across releases.

The updater retains service definitions and their loaded/running state while
replacing per-TV background executables. A release that changes service command
arguments or scheduling needs an explicit update migration. Home state is
backed up and included in each personalized package before installation; extend
the state-file list in `cli/src/update.rs` and package preservation tests if
the TV adds another persistent file.
