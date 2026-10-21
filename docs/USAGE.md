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

Developer Mode and Home itself are hidden from the grid. Ordering, additional
exclusions and branding use tvhome's bundled catalog policy; changing
that policy is described in the [development guide](DEVELOPMENT.md).
