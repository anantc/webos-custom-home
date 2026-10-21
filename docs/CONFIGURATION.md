# TV profiles and configuration

A profile is a saved connection to one TV. Create it with
`tvhome --tv living-room setup`, following [TV setup](SETUP.md).
Use `tvhome tvs` to list profiles and `tvhome --tv living-room apps` to list
that TV's cached installed apps.

Names start with a lowercase letter and contain lowercase letters, digits or
hyphens, up to 48 characters. They select local directories and background jobs;
they do not rename the television. A second name such as `bedroom` has its own
address, keys, inventory, jobs and Healthchecks URLs.

## Saved files

All paths belong to the computer user running tvhome. Use that same user
for setup and background jobs. `~` means their home directory.

| Per-TV resource | Default path for living-room |
| --- | --- |
| Configuration | `~/.tvhome/tvs/living-room/environment.json` |
| SSH key | `~/.tvhome/tvs/living-room/ssh-key` |
| Trusted SSH server identity | `~/.tvhome/tvs/living-room/known_hosts` |
| Pairing key | `~/.tvhome/tvs/living-room/pairing.json` |
| App/input inventory | `~/.tvhome/tvs/living-room/inventory.json` |
| Last published catalog hashes | `~/.tvhome/tvs/living-room/publication.json` |
| Cached Developer Mode token | `~/.tvhome/tvs/living-room/devmode-token` |
| Background executable | `~/.tvhome/tvs/living-room/bin/tvhome` |
| Locks and macOS logs | `~/.tvhome/tvs/living-room/` |

Private files are readable only by your user. The passphrase used during setup
is not retained; the unlocked SSH key allows scheduled jobs to authenticate.

Changing a profile's host clears its renewal-token cache. Cached inventories
record the host and cannot be reused for a different address. Running jobs must
be stopped before configuration changes.

## Advanced: provision settings from environment variables

**Why:** environments managed by scripts may already supply addresses, credentials
and Healthchecks URLs. `configure` imports those values into the same saved
profile used by interactive setup and services.

For each TV, copy the [environment template](../cli/.env.example) into a
private file such as `~/.tvhome/env/living-room.sh`.
Create the parent directory and protect the file:

```sh
mkdir -p "$HOME/.tvhome/env"
chmod 700 "$HOME/.tvhome/env"
# After saving your edited template in this directory:
chmod 600 "$HOME/.tvhome/env/living-room.sh"
```

`700` restricts a directory to your user; `600` does the same for a file.
Fill in the TV's values, then save the profile:

```sh
(
  . "$HOME/.tvhome/env/living-room.sh"
  tvhome --tv living-room configure
)
tvhome --tv living-room setup
```

The dot command loads the file's exported variables. Parentheses keep them in a
temporary shell. `--tv living-room` chooses the profile; the filename alone does
not choose a TV. Setup completes key retrieval, pairing and TV installation,
reusing any credentials supplied in the profile.

Only `configure` reads environment settings. All other commands use the saved
profile, so variables left in your terminal cannot redirect another TV's job.
Keep the template's `unset` lines when copying it for another TV.

To replace an existing profile, first stop that TV's jobs with `uninstall`,
then use `configure --replace`. Replacement saves a **complete snapshot**:
omitted settings are cleared. Include the existing values you intend to retain.
Then run `setup` if access changed and `background` to validate and restart.

| Variable | Meaning / default |
| --- | --- |
| `TV_HOST` | Required TV IPv4 address or hostname |
| `TV_WATCH_ENABLED`, `TV_RENEW_ENABLED` | Optional background jobs: `1` enables, `0` or omitted disables; interactive `background` saves your choices |
| `HC_WATCHER_URL`, `HC_RENEWAL_URL` | Optional Healthchecks.io base ping URLs, unique per job and TV; enable notification routing in its dashboard |
| `TV_SSH_KEY` | Override the profile's saved SSH key path; must be an unlocked private key |
| `TV_SSH_KNOWN_HOSTS` | Override the profile's trusted-host file |
| `TV_SSH_USER`, `TV_SSH_PORT` | LG Developer Mode defaults: `prisoner`, `9922` |
| `TV_DEVMODE_TOKEN` | Optional session token; overrides SSH token retrieval for renewal |
| `TV_CLIENT_KEY` | Optional supplied remote-control pairing key |
| `TV_CLIENT_KEY_FILE` | Optional host-to-key JSON file; use a separate file per TV |
| `TV_LAUNCHER` | Foreground redirect target; default `com.anant.tvhome.native` |
| `TVHOME_STATE_DIR` | State base directory; `tvs/<name>/` is always appended |

A direct Developer Mode token changes renewal's credential source. Setup and
catalog publishing still use SSH. `TV_LAUNCHER` changes the redirect target;
catalog publishing always targets the installed native Home application.
