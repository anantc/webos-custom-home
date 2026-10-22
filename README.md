# webOS Custom Home

Home is an Apple TV-inspired launcher for LG webOS. It opens the apps already
installed on your TV, with a glass tray, focus scaling, navigation sounds and a
Wi-Fi indicator.

You download **tvhome** once onto your Mac or Linux computer. It contains the
Home TV app, installs it on each TV, and can stay running to redirect the Home
button, update app lists, renew Developer Mode and report to Healthchecks.io.
One computer can manage several TVs.

## Get started

Follow the **[setup guide](docs/SETUP.md)** from the computer that will manage
your TVs. It covers the complete flow:

1. Download `tvhome` for your computer, verify it and run `--install`.
2. Enable Developer Mode on the TV.
3. Run `tvhome --tv living-room setup` to pair and install Home.
4. Optionally run `tvhome --tv living-room background` to choose Home watching,
   Developer Mode renewal and Healthchecks.io alerts.

Setup creates the TV's saved profile and app catalog. Repeat the TV steps with
another name for each additional TV, using the same download.

Home runs on the TV. Background features need the computer to stay awake and
running. Developer Mode must remain active to retain Home.

After setup, see [Using Home](docs/USAGE.md) for remote controls, inputs and Wi-Fi
details. [Background operation](docs/BACKGROUND.md) explains monitoring, renewal
and logs; [TV profiles](docs/CONFIGURATION.md) documents saved files and advanced
configuration.

Update the command and all saved TVs with `tvhome update`. TVs that are off are
skipped; turn them on and rerun it to catch up. See [Updates](docs/SETUP.md#updates).

## Development

[Development](docs/DEVELOPMENT.md) covers source setup, catalogs, builds, tests
and previews. [TV app internals](tv/README.md) covers rendering and webOS
integration. [Releases](docs/RELEASES.md) covers publishing downloads.

## Inspiration and attribution

- [OmkarK97/webos-vibehack](https://github.com/OmkarK97/webos-vibehack)
  inspired the custom Home launcher and Home-button redirection.
- [punktfunk/client-webos](https://github.com/punktfunk/client-webos) provided
  the native Rust/webOS foundation. Its window integration, compatibility shims
  and parts of the platform layer are adapted here; see [tv/NOTICE](tv/NOTICE).
- [webOSbrew](https://www.webosbrew.org/) provides the native toolchain and
  webOS-compatible SDL used to build and run Home.

## Licenses

Code is MIT; copyright notices are in [LICENSE](LICENSE). Native dependencies,
the Inter font and publisher artwork have separate notices. See [NOTICE](NOTICE),
[tv/NOTICE](tv/NOTICE) and [asset sources](docs/ASSETS.json).
