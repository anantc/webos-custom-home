# Background operation reference

The `background` command uses the same `tvhome` download and TV profiles created
during setup. It asks which features to enable for each TV: Home watching with
catalog updates, daily Developer Mode renewal, and Healthchecks.io alerts.
Each is optional. Only the jobs you select are installed.

The jobs persist across restarts. On macOS they start when the user logs in;
on systemd Linux, enabled services and lingering start them after boot. The
computer must stay awake for jobs to run.

Follow [Set up Home](SETUP.md) for the complete installation flow,
including [choosing optional background features](SETUP.md#4-optional-background-features)
and configuring Healthchecks. This page explains the running jobs and their logs.

## Inspect jobs and logs

These examples use the saved TV name `living-room`. Substitute `bedroom` or
another name to inspect that TV's independent jobs. Run the commands for the
features you enabled; an unselected job has no service or log.

**Check on macOS:**

```sh
launchctl print "gui/$(id -u)/com.anant.tvhome.living-room.watch"
launchctl print "gui/$(id -u)/com.anant.tvhome.living-room.renew"
tail -n 30 "$HOME/.tvhome/tvs/living-room/watch.log"
```

`id -u` supplies your numeric user ID; `gui/<ID>` selects that login session.
The labels select living-room's jobs. `tail` displays the latest 30 log lines.

**Check on Linux:**

```sh
systemctl --user status tvhome-watch-living-room.service tvhome-renew-living-room.timer
systemctl --user list-timers tvhome-renew-living-room.timer
journalctl --user -u tvhome-watch-living-room.service -u tvhome-renew-living-room.service
```

`--user` selects your services. The watcher runs continuously; the renewal timer
starts the renewal service daily. `journalctl` reads their logs.

## What monitoring means

When its Healthchecks URL is configured, the watcher sends a heartbeat every minute. An unreachable TV is treated as
standby and retried. Where the TV exposes power state, catalog checks also skip
confirmed standby or screen-off states. Rejected requests, missing Home, unconfirmed launches and
failed catalog publishing report failure. A prior failure stays failed during
standby until a successful retry. Foreground progress does not hide a catalog
failure. Foreground inactivity beyond two minutes or a catalog operation stalled
for five minutes also fails the check.

Thus the watcher check measures tvhome health, not TV uptime. It cannot
distinguish standby from an incorrect address by reachability alone.

## Connections and credentials

| Connection | Credential | Purpose |
| --- | --- | --- |
| Computer → TV, secure WebSocket on port 3001 | Saved pairing key | Read installed apps/inputs, observe foreground changes and launch Home |
| Computer → TV, SSH on port 9922 | Saved Developer Mode key | Install Home, publish catalogs and read the session token |
| Computer → LG, HTTPS | TV session token | Extend and verify the server-side session |
| Computer → Healthchecks, HTTPS | Per-job ping URL | Report progress and failure |

Setup establishes both TV permissions. The SSH account `prisoner` and port
`9922` are LG's Developer Mode defaults. Its host fingerprint is pinned per
profile. Files and paths are explained in [TV profiles](CONFIGURATION.md).

## How renewal works

tvhome reads `/var/luna/preferences/devmode_enabled` over SSH and caches
this TV's session token privately. If the TV is unreachable, it uses that cache.
An explicitly configured `TV_DEVMODE_TOKEN` takes precedence.

It sends the token to LG's `ResetDevModeSession.dev` endpoint over HTTPS.
After a successful response it separately calls `CheckDevModeSession.dev` and
validates the reported lifetime. Only a verified lifetime of at least 72 hours
counts as a successful renewal. When Healthchecks monitoring is configured,
renewal sends `/start` first, a success ping after verification, and `/fail` after
three failed attempts. A failed success-ping delivery also makes the command fail.
With monitoring disabled, the same renewal and verification run without pings.

Renewal can therefore work while the TV is off: the transaction is between the
computer and LG. tvhome does not write the TV's displayed countdown or
verify that the TV has fetched it. A green check means LG confirmed the session;
the TV's refresh timing is not established by this implementation.

These unofficial endpoints are also used by
[webOS Dev Manager](https://github.com/webosbrew/dev-manager-desktop/blob/main/src-tauri/src/plugins/devmode.rs).
Unknown responses fail verification. If the session expires or is invalidated,
check Developer Mode, sign in again if needed and repeat setup with
`--refresh-key`. SSH can then refresh the cached token; a directly supplied
token must be replaced in the profile.

## Updates, stopping and foreground debugging

After verifying the updated executable and running its `--install` option, run
`tvhome --tv living-room install` for each TV with background features enabled.
This refreshes that TV's private executable copy and restarts its saved selection
of jobs. To update the TV app too, use
[TV app updates](SETUP.md#updates).

To intentionally stop one TV's jobs, pause any of its configured Healthchecks checks and run:

```sh
tvhome --tv living-room uninstall
```

Bedroom's jobs keep running. Configuration, keys, logs and Home on the TV remain.
To debug with visible output, run these after stopping background jobs:

```sh
tvhome --tv living-room renew
tvhome --tv living-room watch
```

The first runs once; the second continues until Ctrl+C. Stop it before running
`background` again, since two watchers for the same TV are blocked.

macOS logs rotate at 2 MB with three backups; timestamps are Unix seconds.
Startup errors go to `launchd-errors.log` in the TV's state directory.
Linux logs remain in the user journal. Secrets are not printed.
