# Gadgetbridge settings for ur

Gadgetbridge sees ur as a Bangle.js, and a few of its defaults don't
suit ur. Checked against Gadgetbridge 2026.11.1; older versions may name
or place these differently.

**Watch settings** are the watch's own: the gear ("Device specific
settings") on its card in Gadgetbridge. **Settings** are Gadgetbridge's
own, from its main menu.

## Change these

| Setting | Where | Default | Set to | Why |
|---|---|---|---|---|
| Pair | The "Pair with Bangle.js ur?" prompt when adding the watch | — | **Pair**, then type the passkey the watch shows | ur talks only to paired phones; with "Don't Pair" it says NOT PAIRED ([C01]). |
| [Auto fetch activity data][auto-fetch] | Settings → Automations → Auto fetch activity data | Off | On | Steps, heart rate and sleep reach Gadgetbridge only when it fetches them, and the watch keeps the last 24 hours. Without this, a day you don't fetch by hand (the Sync icon under the watch on the home screen, or pull down on the device list) is lost. It fetches each time you unlock the phone, so it only works with a screen lock set. Bluetooth only: no mobile data, and no noticeable phone battery. |
| Minimum time between fetches | Below Auto fetch activity data | 0 (each unlock) | 60 minutes (a suggestion) | Each fetch resends the last ~2¾ hours (sleep stages are revised after the fact), about 4 KB over the watch's radio; hourly is plenty against a 24-hour history. |
| Use phone GPS data | Watch settings → Location | Off | On | The watch's only source of location: without it, themes and brightness follow the sun in Oslo, and the watch keeps looking for room sensors after you've left them ([C03]). Costs the phone's GPS for up to 2 minutes, or until a fix good to 75 m, when the watch asks: on connecting (at most hourly), on a time zone change, and on losing a room sensor (at most four times a day). |
| Keep location updated | Settings → General settings → Location | Shown as on, but off until that screen has been opened once | On: open the screen, and check it's on | Without it, Gadgetbridge drops the phone's fixes before they reach the watch ([C03]). Free in itself: it only lets the fixes above through. |
| Location permission | Android's app settings for Gadgetbridge → Permissions → Location | Asked when needed | Allow all the time | The watch asks while Gadgetbridge is in the background ([C03]). |
| Battery | Android's app settings for Gadgetbridge → Battery (Gadgetbridge's permission check calls it "Ignore battery optimizations") | Optimized | Unrestricted | Otherwise Android may stop Gadgetbridge in the background: no notifications, fetches, or locations until you open it. |
| Allow Intents | Watch settings → Developer options | Off | On, if **BUTTONS** is on **VIA APP** or **BOTH** | Without it, Gadgetbridge drops [button grid](buttons.md#gadgetbridge-and-tasker) presses, and the watch says INTENTS OFF ([C04]). The "display over other apps" permission its description mentions isn't needed: presses are broadcasts. |

## Leave these as they are

| Setting | Where | Default | Why |
|---|---|---|---|
| Request low power | Watch settings → Connection (the screen, near the end) | Off | ur asks for its own idle link 30 s after the last traffic: 100–125 ms, skipping up to 7 events in 8, about one radio event a second. Android's low-power priority skips at most 2 in 3: when ur asked for that, field logs showed about three times the idle radio events. So it gains nothing, and makes Gadgetbridge wait 45 s rather than 5 s before giving up on a stuck connection attempt. |
| Only use network to determine location | Watch settings → Location | Off | With it on, no location ever arrived in testing ([C03]). |
| GPS data update interval | Watch settings → Location | 1000 ms | The watch stops the phone's GPS itself once it has a good fix. |
| Text as Bitmaps | Watch settings → Notifications | Off | ur shows each picture as `?`, and long messages full of them never arrive ([N02]). |
| Transliteration | Watch settings → Notifications | None | ur shows ASCII and Latin-1's letters (æøå, é, ü, ß…). Add only languages written outside them (Russian, Greek, Polish…), which otherwise show as `?`; Scandinavian, German or French would only turn letters ur can show into ASCII (å → aa). |
| Allow high MTU | Watch settings → Connection (the heading) | On | Bigger pieces: faster fetches. |
| Auto reconnect to device, Connect on connection from device | Watch settings → Connection (the screen) | On | Gadgetbridge reconnects by itself when the watch comes back in range. |
| Sync time | Settings → General settings → Date and Time | On | The watch gets the time only from Gadgetbridge, and keeps an [update](../README.md#updating) only once it has. |
| Send notifications | Watch settings → Notifications | On | |
| Auto wake and unlock | Watch settings → Notifications | Off | ur never asks the phone to open a notification. |
| Send navigation to watch | Settings → External Integrations → Navigation apps | On, for Google Maps and OsmAnd | Turn CoMaps on there if you use it. |
| Sync calendar events, Allow Internet Access | Watch settings | Off | ur doesn't use them. Nor weather, the app loader, or loyalty cards. |

## Up to you

| Setting | Where | Default | What it does |
|---|---|---|---|
| …also when screen is on | Settings → Notifications | Off | Off: notifications that arrive while you use the phone don't reach the watch. |
| Do Not Disturb | Settings → Notifications | Off | On: notifications don't reach the watch while the phone is on Do Not Disturb. The watch's quiet nights silence them while you sleep either way. |
| Cache while out of range | Settings → Notifications | Off | On: notifications that came while the watch was away arrive once it's back. |
| Notify for missed call | Watch settings → Notifications | Off | On: a notification for each missed call. |
| Enable Health Connect Export, Select devices, Sync after device sync | Settings → External Integrations → Health Connect | Off, none, off | On, the watch, and on, to pass sleep, steps and heart rate on to Health Connect. Gadgetbridge 2026.11.1 doesn't count a Bangle.js fetch as a device sync ([#7289](https://codeberg.org/Freeyourgadget/Gadgetbridge/issues/7289)), so the watch ends a fetch as if it had GPS tracks to make it one: the first after the watch starts, then at most every 2 hours. Each of those shows a short "End : Fetching sports details" message. |

## What Gadgetbridge gets

The watch keeps a sample a minute, but sends Gadgetbridge one per
3 minutes: Gadgetbridge's Bangle.js support merges samples less than
2 minutes apart, keeping only the last one's steps, which lost about 60% of
them. In each 3 minutes:

- **Steps**: all of them.
- **Heart rate**: the last reading. Earlier ones in the same 3 minutes are
  dropped; the watch rarely measures that often, except while you look at
  the clock face or Gadgetbridge's live activity.
- **Sleep stage, or awake**: the one most of its minutes had. Sleep in
  Gadgetbridge and Health Connect is to the nearest 3 minutes.

Each sample is stamped with the last second of its 3 minutes, so steps land
in Health Connect in those minutes (they used to land a minute early), and
the 3 minutes under way wait for the next fetch.

Gadgetbridge keeps the first sleep stage (or awake) it gets for each
sample, but the watch revises the last couple of hours' as the night goes
on (sleep is only recognised after the fact, and stages need the hour
after). So a fetch during the night, or within an hour or so of waking,
fixes what it got in Gadgetbridge as it was then.

[C01]: ../README.md#c01-the-watch-says-not-paired
[C03]: ../README.md#c03-the-watch-never-gets-a-location-from-the-phone
[C04]: ../README.md#c04-the-watch-says-intents-off
[N02]: ../README.md#n02-the-watch-says-text-as-bitmaps
[auto-fetch]: https://gadgetbridge.org/basics/features/sync/#auto-fetch
