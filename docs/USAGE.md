# Using Home

Home opens the streaming apps already installed on your TV. App names, accounts
and subscriptions remain managed in those apps.

## Navigation

Use the remote arrows or pointer to select an app, then press OK to open it.
The selected icon enlarges, and focus changes play a short navigation click.

Move up from the first app row to reach the top controls. Left and Right switch
between Wi-Fi and the clock/settings control; Down returns to the app grid.

The settings menu offers **Inputs**, **TV settings** and **Exit**. Back closes a
panel; from Inputs, it returns to the settings menu. Back on the app grid or
holding Back exits Home.

## Reorder tiles

Select an app or pinned input and hold **OK** for about a second. The tiles
start to jiggle; the selected tile stays enlarged. Release OK, then use the arrows
to move it. Surrounding tiles slide aside, including when moving between rows.
Press **OK** again to save, or **Back** to cancel. You can also click and hold
a tile with the pointer to enter move mode, then use the arrows.

Apps and pinned HDMI or Live TV tiles can go anywhere, including the top tray.
Each TV saves its own order. It survives Home restarts and app-list updates;
new apps and newly pinned inputs appear at the end. Temporarily unavailable
tiles return to their saved positions when available again. Leaving Home during
an unfinished move cancels it. Removing Home also removes its saved layout.

## Screensaver

LG controls the screensaver's timing and activation. Home allows LG's screensaver
and keeps the pointer hidden when it takes focus. The timeout and availability
depend on the TV's firmware and settings.

## Pin inputs to Home

Open **Inputs** from the settings menu. Select an input name to switch to it,
or select the pin beside it to add a tile to Home. A filled pin means the input
is pinned; select it again to remove the tile. Live TV can also be pinned.

With the remote, Right moves from an input name to its pin, Left returns to the
name, and OK toggles the pin. Up and Down stay in the same column. The pointer
can click either target directly. Back returns to settings.

Newly pinned inputs appear at the end; use move mode to reposition them. Each tile
shows the TV's input name and switches directly to that input when selected.
Choices are saved on that TV and survive app restarts and catalog refreshes.
If an input temporarily disappears from the TV's catalog, its tile returns when
the input is reported again. Removing Home also removes its saved pins.

## Wi-Fi

Select the top-left Wi-Fi icon to see the network name, signal level, IP address
and router address when the TV supplies them.

| Signal level | Display |
| --- | --- |
| 0–33 | One bar |
| 34–65 | Two bars |
| 66–100 | Three bars |
| Reading unavailable | Question mark |
| Disconnected or Wi-Fi off | Slash |

Bars describe wireless signal strength, not download or upload speed. The panel
displays information about the current connection.

### Speed test

Open **Wi-Fi → Speed test → Run speed test** to measure download and upload
speed in Mbps and latency in milliseconds using Netflix's Fast.com servers.
The test uses the TV's current internet connection, including Ethernet if connected.
It takes about 30 seconds and can transfer hundreds of megabytes on a fast connection.

**Cancel test**, Back or leaving Home stops the test. The latest completed result
and its time are saved on that TV; **Test again** starts a fresh measurement.
A failed or cancelled test keeps the previous completed result. Test results can
vary with other network traffic and the selected server.

## Opening Home

Open Home from LG's app row or use a numbered Quick Access shortcut. With
the tvhome watcher running, pressing LG's Home button redirects to Home. Return to LG
Home within three seconds to leave LG Home open.

## App list

tvhome watcher checks installed apps roughly once a minute and sends
changes to Home. Home refreshes when it returns to the foreground and every
five seconds while visible, preserving the selected app where possible.
For a manual check, run `tvhome --tv NAME apps --refresh` on the
computer running tvhome, replacing `NAME` with the TV profile.

Developer Mode and Home itself are hidden from the grid. The initial order,
additional exclusions and branding use tvhome's bundled catalog policy; changing
that policy is described in the [development guide](DEVELOPMENT.md).
