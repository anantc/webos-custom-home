# Using Home

Home opens the streaming apps already installed on your TV. App names, accounts
and subscriptions remain managed in those apps.

## Navigation

Use the remote arrows or pointer to select an app, then press OK to open it.
The selected icon enlarges, and focus changes play a short navigation click.

Move up from the first app row to reach the header. Left and Right move between
weather, settings and Wi-Fi in that order. Down returns to the
app grid. The pointer can select each control directly. Wi-Fi sits beside the
clock and settings at the top right, ordered time, settings, Wi-Fi; weather aligns
with them at the top left. Settings and Wi-Fi panels open below their icons,
aligned to the icon's right edge.

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

## Weather and air quality

Home shows the detected city, temperature, conditions, daily high/low and Indian
AQI above the app tiles. The layout compacts when three rows need the space.
Select the weather area for feels-like temperature, reading age, the main air
pollutant, **Change city** and **Refresh**. Back or **Close** returns to Home.

To choose a city, select **Change city**, enter its name with the on-screen
keyboard or a physical keyboard, then select **Search**. Choose a result using
its state and country to distinguish cities with the same name. The selected
city is saved on that TV and survives restarts and app-list updates. Weather
and AQI both use that city's coordinates. **Back** cancels without changing the
saved city; **Use automatic location** restores detection from the public IP.

Automatic location is estimated from your public IP using IPWhois. It can identify your
ISP's routing location instead of your actual city, especially with a VPN.
The displayed city lets you check this. The lookup goes directly from the TV;
the computer running tvhome is not involved. City searches send the entered
name to Open-Meteo's GeoNames-based search service. Choosing a city bypasses the
IP lookup and sends that city's coordinates to the weather and air-quality services.

Weather comes from Open-Meteo. Air quality uses CAMS regional model data through
Open-Meteo, converted to the Indian AQI scale. It is a model estimate, not a local
CPCB monitoring-station reading or an indoor air-quality measurement. Severe
readings are shown as **400+**. Missing readings show a dash, never a good score.

Home updates every 15 minutes while open and retries failed updates after five
minutes. Readings older than two hours disappear. Nothing needs to be configured
on the computer. These requests use internet access; they do not change network
settings.

## Wi-Fi

Select the Wi-Fi icon beside the clock to see the network name, signal level, IP address
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
