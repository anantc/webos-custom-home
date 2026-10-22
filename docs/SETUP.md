# Set up Home

Home is the app that runs on your LG webOS TV. **tvhome** is the command you run
on your Mac or Linux computer to install Home and manage your TVs. One download
contains both the TV app and all computer-side functions.

Use the computer that will stay running if you want background operation.
It and the TV must be on the same network. Complete the steps below on that
computer; you can manage additional TVs with the same installation.

Run the computer commands in Terminal. In paths, `~` and `$HOME` mean your
home directory. Choose a local name for each TV, such as `living-room` or
`bedroom`; this does not rename the TV on screen.

## 1. Download and install tvhome

**Why:** tvhome installs Home on your TVs and manages any background features you
choose. The executable contains the Home TV package and license notices.

Download the **executable and its checksum** from the same row below into your
Downloads folder. These links are for tvhome 0.1.5. Choose your computer's platform;
on a Mac, **About This Mac** shows an Apple chip or Intel processor. On Linux,
`uname -m` reports `aarch64` for ARM64 or `x86_64` for x64.

| Computer | Executable | Checksum | Requirements |
| --- | --- | --- | --- |
| Apple Silicon Mac | [tvhome-darwin-arm64](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-darwin-arm64) | [.sha256](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-darwin-arm64.sha256) | macOS 13+ |
| Intel Mac | [tvhome-darwin-x64](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-darwin-x64) | [.sha256](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-darwin-x64.sha256) | macOS 13+ |
| ARM64 Linux | [tvhome-linux-arm64](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-linux-arm64) | [.sha256](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-linux-arm64.sha256) | glibc 2.35+; systemd for background jobs |
| x64 Linux | [tvhome-linux-x64](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-linux-x64) | [.sha256](https://github.com/anantc/webos-custom-home/releases/download/v0.1.5/tvhome-linux-x64.sha256) | glibc 2.35+; systemd for background jobs |

### Verify and install

**Why:** the checksum compares the downloaded executable against the release's
SHA-256 digest. The installer then copies it to a stable location and configures
your shell to find it.

Run the block for your computer. Each `&&` means the next step runs only if the
previous one succeeded. `chmod +x` gives the downloaded file permission to run.

**Apple Silicon Mac**

```sh
cd "$HOME/Downloads" &&
shasum -a 256 -c tvhome-darwin-arm64.sha256 &&
chmod +x tvhome-darwin-arm64 &&
./tvhome-darwin-arm64 --install
```

**Intel Mac**

```sh
cd "$HOME/Downloads" &&
shasum -a 256 -c tvhome-darwin-x64.sha256 &&
chmod +x tvhome-darwin-x64 &&
./tvhome-darwin-x64 --install
```

**ARM64 Linux**

```sh
cd "$HOME/Downloads" &&
sha256sum --check tvhome-linux-arm64.sha256 &&
chmod +x tvhome-linux-arm64 &&
./tvhome-linux-arm64 --install
```

**x64 Linux**

```sh
cd "$HOME/Downloads" &&
sha256sum --check tvhome-linux-x64.sha256 &&
chmod +x tvhome-linux-x64 &&
./tvhome-linux-x64 --install
```

**Check:** verification prints the executable's filename followed by `OK`,
then the installer prints its installation paths. If verification fails, download
both files again from the same release before continuing.

`--install` puts the executable at `~/.local/bin/tvhome`, writes embedded notices
to `~/.tvhome/notices/LICENSES.txt`, and adds `~/.local/bin` to your
zsh or bash startup configuration. It preserves existing shell settings, and
running it again updates the installed executable. It prints each file it changes.

Open a **new terminal** so the shell loads the updated command path, then run:

```sh
tvhome --version
```

It should report `tvhome 0.1.5`. In the current terminal, or with another shell,
use `~/.local/bin/tvhome` in place of `tvhome` in subsequent commands.
The downloaded file can be deleted after installation.

If macOS blocks the executable, review it in System Settings → Privacy & Security
and use **Open Anyway**, then rerun the same installation block.

## 2. Enable Developer Mode

**Why:** Developer Mode grants permission to install Home and lets tvhome
keep Home's app list current.

On the TV, open **Apps/LG Content Store**, install LG's **Developer Mode** app,
and sign in with your LG developer account. Enable **Dev Mode Status**, complete
the reboot, then reopen Developer Mode and enable **Key Server**.

Key Server lets setup retrieve this TV's SSH key. The passphrase displayed by
the TV unlocks that key; it is not your LG account password.

**Check:** Developer Mode shows remaining session time, Key Server is on, and you
can see the TV's IP address and passphrase. Keep the session active: expiration
can remove sideloaded apps. [LG's Developer Mode guide](https://webostv.developer.lge.com/develop/getting-started/developer-mode-app)
describes the TV controls.

## 3. Set up this TV

**Why:** one saved name ties together the TV's address, installation key,
remote-control pairing and app catalog.

With the TV on, run:

```sh
tvhome --tv living-room setup
```

Setup asks for the address shown in Developer Mode and the key's passphrase.
On the first SSH connection it shows the TV's fingerprint and asks whether to
trust it. Confirm the target is your TV; compare with an existing trusted
fingerprint if you have one. The saved fingerprint detects an unexpected server
at that address on later connections.

Accept the remote-control pairing prompt on the TV when it appears. This is
a separate permission: SSH installs Home and writes its catalog, while pairing
allows reading installed apps, observing the foreground app and opening Home.

Setup then reads the app list, personalizes the bundled package, transfers and
installs it, and opens Home. It connects to LG's predefined Developer Mode SSH
account, `prisoner`, on port `9922`. Those are LG's account and port for this
purpose, not a computer username you need to create.

**Check:** Home opens and its icons launch the expected apps. You can turn
Key Server off after setup; the saved SSH key handles later connections.

## 4. Optional background features

Home is installed after step 3. Skip this section if you want to open Home
manually and extend Developer Mode in its TV app.

**Why:** an always-on computer can redirect the Home button, keep app lists current,
renew the Developer Mode session and report failures. Choose each feature during
setup; Healthchecks.io monitoring is optional too.

### Choose Home watching and session renewal

- **Home watching:** redirects LG Home to your Home app and checks installed apps
  roughly once a minute. Home picks up catalog changes on returning to the
  foreground or within five seconds while visible. Without the watcher, use LG's
  app row or a numbered Quick Access shortcut to open Home, and run
  `tvhome --tv living-room apps --refresh` after installing or removing TV apps.
- **Developer Mode renewal:** reads this TV's session token over SSH and stores
  it privately. It sends the token to LG's server to extend the session, then asks
  LG for the remaining time and requires at least 72 hours to confirm success.
  It runs immediately and daily. Once a token is cached, renewal can work while
  the TV is off. This verifies LG's server-side session; it does not change the
  TV's displayed timer directly or confirm when the TV refreshes that timer.
  If you skip renewal, extend the session yourself in the Developer Mode TV app
  before it expires; expiration can remove Home.

You can enable either feature, both, or neither.

### Optional Healthchecks.io alerts

**Why:** Healthchecks.io can notify you if an enabled job fails or the computer
stops sending pings. The job's ping URL reports status; your Healthchecks
notification integration determines where alerts are delivered.

If you choose alerts, the setup prompt guides you through these steps:

1. Sign in at [Healthchecks.io](https://healthchecks.io/) and select your project.
2. Open **Integrations**, connect your alert destination (for example email),
   complete any verification and send a test notification. Confirm it arrives.
3. Create one check for each job you enabled, using a **Simple** schedule:

| Example check | Period | Grace | Create when |
| --- | --- | --- | --- |
| `living-room-watch` | 5 minutes | 5 minutes | Home watching is enabled |
| `living-room-renew` | 1 day | 4 hours | Session renewal is enabled |

4. Turn the notification integration **ON** for each check. A connected
   integration can be disabled for an individual check; confirm the routing on
   each check's page. See [Healthchecks notification setup](https://healthchecks.io/docs/configuring_notifications/).
5. Copy each check's base ping URL, such as `https://hc-ping.com/…`, into the
   corresponding prompt and press Enter. You can see the URL as you type or paste
   it. Use a different check for every job and TV.

Names and slugs identify checks in Healthchecks; tvhome uses their full ping URLs
to route updates. Tags are optional labels.

Period and grace are alert deadlines, not job schedules. The watcher sends a
heartbeat every minute; a 5-minute period plus 5-minute grace permits roughly
10 minutes without a heartbeat before an alert. Renewal runs daily; its check
allows 1 day plus 4 hours. tvhome also reports job failures directly.

When the TV is off, the watcher retries while continuing its own heartbeat, and
renewal can use the cached token to contact LG. Turning off the TV alone does not
raise an alert; an existing job failure stays failed until a successful retry.
When the computer is off or asleep, both jobs stop and missed pings trigger alerts.

Setup asks you to confirm the integration is enabled and its test arrived;
it cannot inspect notification destinations using only ping URLs. Declining
monitoring lets the jobs run without Healthchecks.

### Start the selected features

**Why:** the operating system keeps the selected jobs running after Terminal
closes. The computer must remain awake and connected.

On systemd Linux, enable lingering once for the user running setup:

```sh
sudo loginctl enable-linger "$USER"
```

Lingering lets that user's services run without an active login. `$USER`
identifies your computer user. macOS uses LaunchAgents: the selected jobs start automatically when you log in
after a restart. Keep that user logged in and the Mac awake. Linux services start
after boot with lingering enabled. You only run `background` once per TV; closing
Terminal or restarting the computer does not remove the jobs.

On either platform, with the TV on:

```sh
tvhome --tv living-room background
```

Answer the Home watching, renewal and Healthchecks questions. Pressing Enter at
each yes/no question chooses **No**. Setup validates the selected features and
starts only those jobs. If you skip both jobs, it makes no changes.

On macOS, allow **tvhome** to access devices on your local network when prompted.
The background watcher needs this permission to reach your TV. If you dismissed
the prompt, enable tvhome in **System Settings → Privacy & Security → Local
Network**. macOS may ask again after a tvhome update. A successful connection from
Terminal or SSH does not establish permission for the background job: macOS
handles those command-line connections differently from LaunchAgents.

**Check:** if watching is enabled, press Home on the TV and confirm redirection.
Return to LG Home within three seconds to leave LG Home open. If renewal is
enabled, setup prints the remaining session hours confirmed by LG. If monitoring
is enabled, the corresponding Healthchecks checks should become **Up**.

To change these choices later, rerun `tvhome --tv living-room background`.
The existing jobs keep running while you answer. Your answers stay in memory
until the end; setup then stops this TV's jobs, saves the new settings and
restarts the selected jobs. If applying the settings fails or you interrupt that
step, setup attempts to restore the previous settings and jobs and reports the
result. Other TVs keep running. Pause checks in Healthchecks.io for any jobs or
monitoring you disable, to avoid missed-ping alerts. [Background operation](BACKGROUND.md) explains logs and failures.

## Add another TV

**Why:** each TV needs its own credentials and app list, even when the same
computer manages both.

Enable Developer Mode and Key Server on the second TV, then run:

```sh
tvhome --tv bedroom setup
tvhome tvs
tvhome --tv bedroom apps
```

Enter the bedroom TV's address and passphrase and approve pairing there.
tvhome creates a separate `~/.tvhome/tvs/bedroom/` directory.
Every later `--tv bedroom` command reuses those credentials and targets that TV.
Both TVs display the installed app as **Home**.

For optional background features, run `tvhome --tv bedroom background` and choose
which jobs and alerts to enable. Create new Healthchecks checks only for jobs
you choose to monitor. The same executable starts independent jobs for that TV.

## Where the TV name and credentials live

Each TV has one private folder on the computer: `~/.tvhome/tvs/<name>/`.
For `living-room`, it contains:

| Path | Purpose |
| --- | --- |
| `~/.tvhome/tvs/living-room/environment.json` | Saved address and configuration |
| `~/.tvhome/tvs/living-room/ssh-key` | Developer Mode SSH key |
| `~/.tvhome/tvs/living-room/known_hosts` | Trusted TV SSH identity |
| `~/.tvhome/tvs/living-room/pairing.json` | Remote-control pairing key |
| `~/.tvhome/tvs/living-room/inventory.json` | Last observed apps and inputs |

The passphrase is used to unlock the SSH key and is not saved. The saved key
can be used unattended and is readable only by your computer user.
Keep these credentials private.

Later commands with `--tv living-room` load this same profile, even in a new
terminal. For example:

```sh
tvhome tvs
tvhome --tv living-room apps
tvhome --tv living-room apps --refresh
```

`tvs` lists saved TVs, not a network scan. `apps` displays the cached installed
app inventory. `apps --refresh` reads the TV now and sends any catalog changes
to Home. The inventory includes Developer Mode and Home; those two apps remain
hidden from Home's launcher grid. Add `--json` to either listing command for
structured output, including the last-check timestamp.

## Updates

With your TVs on, run this on the computer that manages them:

```sh
tvhome update
```

The command checks this project's latest stable GitHub release, downloads the
executable for your computer and verifies its SHA-256 checksum before installing
it. It then checks every saved TV profile and installs the bundled Home app
where a newer version is available. Each TV keeps its own app catalog, tile
order, pinned inputs, city selection and last speed-test result. Home closes
briefly during installation and opens when the update has been verified.

Existing background jobs pause during that TV's update and restart afterward,
including after a failed installation. Their executable is updated too. Jobs
that were stopped stay stopped; updating does not enable optional features.
Other TVs' jobs keep running. Allow Local Network access again if macOS prompts.

To update only one TV, plus this computer's executable:

```sh
tvhome --tv living-room update
```

A TV that is off, in standby or unreachable is **skipped**; the other TVs still
update. The command neither wakes the TV nor queues a later installation. Turn
it on and rerun `tvhome update`. This checks the TVs even when the computer
already has the newest executable, so an offline TV can catch up later.

The final summary separates updated, current, skipped and failed TVs. Pairing,
Developer Mode or installation errors are reported per TV without stopping the
remaining updates. Newer development builds are never downgraded. If you press
Ctrl+C during a TV update, it finishes that TV and restores its jobs before
stopping. Settings backups remain in that TV's
`~/.tvhome/tvs/<name>/update-backups/` folder if recovery is needed.

If your installed version does not recognize `update`, download and install the
new executable once using [step 1](#1-download-and-install-tvhome). After that,
`tvhome update` handles subsequent releases.

Ordinary app installs/removals are picked up by the running watcher roughly once
a minute. Home refreshes when it returns to the foreground and within five seconds
of an update while visible. A new Home package is not needed for each app change.

## Removal and recovery

To remove Home, first stop its background jobs if configured, then run:

```sh
tvhome --tv living-room uninstall
tvhome --tv living-room remove-app
```

`remove-app` asks before removing Home from that TV. Profiles and credentials
remain on the computer for reuse. Pause the TV's Healthchecks checks while
intentionally stopping its jobs.

- **Home disappears:** check Developer Mode, sign in again if necessary, enable
  Key Server and run `tvhome --tv living-room setup --refresh-key`. A fresh session
  may require a new SSH key and remote-control pairing.
- **TV address changed:** stop its background jobs, then run
  `tvhome --tv living-room setup --host NEW_ADDRESS`, replacing `NEW_ADDRESS`
  with the TV's current IP. Setup clears the old renewal-token cache and verifies
  access at the new address. Restart with `background` after setup succeeds.
- **SSH identity changed:** verify the TV and address first. If the TV genuinely
  regenerated its host key, move that profile's `known_hosts` file aside and rerun
  setup to review the new fingerprint.
- **App list is stale:** turn on the TV and run `apps --refresh`. Check watcher logs if
  background updates keep failing; rerun setup to reinstall Home if needed.
