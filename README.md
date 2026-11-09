# ur

Finally, a smartwatch firmware for the 90s: as subtle as a retro watch,
but with modern conveniences.

ur, named after an archaic Norwegian word for a timepiece, runs on the
[PineTime] and syncs with [Gadgetbridge] on Android.

<p align="center">
  <img src="docs/screenshots/clock-arcade.png" width="150" alt="The clock face in the Arcade theme, white and red on black: a note for music playing, and 27° along the top, with the battery; then the time, with the date and the seconds under it; the heart rate; and ten blocks for the day's steps, six of them red, then ×1K: 6,000 steps so far, towards 10,000; a short red bar on the top edge means unseen notifications">
  <img src="docs/screenshots/clock-parchment.png" width="150" alt="The same clock face in the Parchment theme, for the day: brown on cream">
  <img src="docs/screenshots/last-night.png" width="150" alt="The Last night screen: 7:20 asleep, from 23:20 to 06:55, and a bar of deep, REM, and light sleep with each one's time">
  <img src="docs/screenshots/alarms.png" width="150" alt="The Alarms screen, in the Handheld theme: smart wake on, for 06:30 on weekdays and 08:30 at weekends, then two alarms and two timers">
  <img src="docs/screenshots/directions.png" width="150" alt="A glance while the phone navigates: the time, a right-turn arrow, 150 m, and the street">
</p>

<p align="center">
  <img src="docs/screenshots/swipe.gif" width="150" alt="Swiping around the clock face, each screen sweeping in as a wave of red dithered blocks: right to Today's steps and heart rate, right again to last night's sleep, back to the clock, left to the alarms, back, and down to the latest notification">
  <img src="docs/screenshots/notification.gif" width="150" alt="A notification arriving with the screen off: the dim screen shows SIGNAL in red over a dashed rule, and the sender's name, JANE SMITH, large and centred in white; a tap wakes it to the clock face, with the notification along the bottom for a moment">
  <img src="docs/screenshots/glance.gif" width="150" alt="A glance: from a dark screen, the time shows large and dim, 17 above 45, for five seconds, then the screen goes dark again">
  <img src="docs/screenshots/alarm.gif" width="150" alt="An alarm ringing at 07:30, the screen saying TAP: SNOOZE, BUTTON: STOP; a tap snoozes it, and the clock face shows when it rings again, 07:39">
</p>

It does what you'd expect of a smartwatch (time, notifications, steps,
heart rate, alarms, timers, music control, "find my watch"), and these
on top:

- **Quiet nights:** once you fall asleep, or nap, notifications neither
  buzz nor light the screen, and touches don't wake it, until your alarm
  or you're up. Alarms, timers, calls, and "find my watch" still ring.
- **Sleep stages and smart wake:** light, deep, and REM sleep, from
  movement and heart rate. Smart wake rings in the half hour before your
  time, as soon as you stir from light sleep, and alarms build up from
  faint taps.
- **Adaptive heart rate:** every 2 to 20 minutes, more often after you
  move, while you sleep, or while it's high, and only once your wrist is
  still, so the LED isn't spent on readings that fail. If your wrist gets
  busy again first (typing, say), it stops and tries at the next still
  moment. If a still wrist gives none, it tries again within 10 minutes,
  for up to an hour. To
  watch it live (with your steps), open Gadgetbridge's Live activity
  chart: the watch measures continuously while it's open.
- **A dim glance:** turn your wrist, and the time shows dimly for a few
  seconds without lighting the screen. While Google Maps, OsmAnd, or
  CoMaps navigates, so does the next turn, with two taps as it comes
  within 50 m.
- **Media gestures:** long-press the clock face to play or pause, and
  double-tap to skip, on a glance or with the screen off too (there, in
  the middle); a buzz says it took.
- **Screens in four directions** around the clock: the past to one side,
  alarms and settings to the other, notifications above, music below.
- **A button grid** for Home Assistant or Tasker: six buttons, each
  tapped, double-tapped, or held (and released, for hold-to-dim), or
  two layers of nine for codes 1–99 (2, then 3, is 23). Sent over
  Bluetooth as BTHome button events, a trigger each in Home Assistant's
  automation editor (or all in one, with a
  [blueprint](docs/buttons.md#blueprints) for each layout), or through
  Gadgetbridge. See [docs/buttons.md](docs/buttons.md).
- **The room's air:** a nearby [Aranet4] (with Smart Home integrations
  on) gives the clock face the room's temperature, and its CO₂ gets a
  screen. So does an unencrypted [BTHome] thermometer (such as a Xiaomi
  LYWSD03MMC on pvvx's firmware); the nearest sensor wins, and one only
  heard faintly (through a wall, say) doesn't count. Without one,
  the watch estimates the room from its own warmth, learning from any
  sensor it meets: roughly ±2–3 °C on the wrist once
  it has, worse before, in the cold, or while you exercise.
- **Follows the sun:** two dark themes after sunset and two light ones
  by day, and a screen that's brightest while the sun is up and dim from
  midnight until you get up.
- **Gadgetbridge without gaps:** with [InfiniTime]'s protocol,
  Gadgetbridge records steps and heart rate only while connected, and no
  sleep. ur speaks the [Bangle.js protocol] instead, and keeps a
  minute-by-minute history for stock Gadgetbridge to fetch, however long
  the phone was away.
- **Nothing lost:** before the battery runs flat, or when put away in
  storage mode, the watch saves what the phone hasn't fetched and
  switches off. A crash restarts it at once, and leaves a backtrace in
  the [logging build](#installing)'s log.
- **Releases you can check:** each one rebuilds byte for byte from its
  tag, and carries a GitHub attestation (see [Verifying](#verifying)).

**Caveats:** ur is experimental and personal: it's tested on two
watches, and heart rate and sleep are still being tuned against a
Garmin, so treat sleep stages as a rough guide. It needs an Android phone
with Gadgetbridge, which can't install updates on a Bangle.js, so those
go through another app or a computer. Once it has switched off (a flat
battery, or storage mode), the watch doesn't know the time, or ring
alarms, until the phone reconnects.

**How it was built:** in Rust, on [embassy]. It was written largely by
Claude (Anthropic's model), steered and tested on real hardware by a
human. That's me, Jon, 👋, but yes, this is very much LLM-written,
Jon-guided! Although I'm also quite happy with what I've now landed on,
and am daily driving this in real life :)

[PineTime]: https://pine64.org/devices/pinetime/
[Gadgetbridge]: https://gadgetbridge.org
[Bangle.js protocol]: https://www.espruino.com/Gadgetbridge
[InfiniTime]: https://github.com/InfiniTimeOrg/InfiniTime
[Aranet4]: https://aranet.com
[BTHome]: https://bthome.io
[embassy]: https://embassy.dev

## Using it

In Gadgetbridge, add the watch "Bangle.js ur", choose Pair, and type the
passkey the watch shows when Android asks. Gadgetbridge then sets the
time and forwards notifications on its own, and shows the battery. It
fetches activity (steps, heart rate, sleep) only when you pull down on
its device list (or tap Synchronize), or, with Settings → Automations →
"Auto fetch activity data" on, when you unlock the phone (that needs a
screen lock; "Minimum time between fetches" spaces them out). It passes
steps, heart rate and sleep on to [Health Connect] if you like (its
"Sync after device sync"): after the first sync since the watch started,
then after a sync at most every 2 hours, which ends with a short "End :
Fetching sports details" message. For the themes
to follow your sun rather than Oslo's, the phone has to share its
location: see [C03](#c03-the-watch-never-gets-a-location-from-the-phone).
A few Gadgetbridge defaults need changing: see [docs/gadgetbridge.md](docs/gadgetbridge.md).

**The clock face** shows the time, date, battery, heart rate, the next
alarm or timer, and the room's temperature: a nearby room sensor's, to
the tenth (21.4°), or else, to the degree (21°), estimated from the
watch's own on the wrist, or the watch's own off it. Steps fill ten
blocks of 1,000 (×1K) towards 10,000, then of 10,000 (×10K); Today has
the exact count. A bar along the top means unseen notifications; a note
in the top left, that music is playing; a faded Bluetooth rune struck
through in red there, that the phone isn't connected.

**Notifications** show the app and who it's from (the sender, or the
title without one), never what it says, and without emoji. One that
comes with the screen on shows along the bottom for 4 seconds; with the
screen off, it gets the whole screen, dimly, the name large and centred
if it's short.

**Screens:** swipe right for Today (steps and heart rate by the hour),
then Last night; left for Alarms, the alarms set in Gadgetbridge,
Settings, then Info; down for the last three notifications; up for the
button grid (if on), Music, and Air (while a room sensor is near: its
CO₂, temperature, humidity and pressure, or those it measures). A short
bar on an edge means there's more that way.

**Touch and button:**

- **Wake** the screen with the button, or a firm tap away from the edge
  (brushes don't count). During a quiet night, only the button wakes it.
- **The button** goes back to the clock, or off from it. It also
  silences a call or stops an alarm or "find my watch" (as does a tap),
  and a double press ends a quiet night.
- **Long-press** (the watch buzzes once it's taken, at 0.6 s) plays or
  pauses on the clock face (with the screen off or glancing, or for 3 s
  after a touch woke it, only in the middle: a quarter of the screen
  away from the top and bottom, a fifth from the sides, clear of a
  folded arm), switches an alarm or timer on or off, and on Info, offers
  storage mode.
- **Double-tap** the clock face for the next track. Tap lightly to skip
  without waking the screen; a firm first tap wakes it. Without a phone
  connected, a double tap wakes the screen instead.
- **A ringing alarm:** tap to snooze for 9 minutes; press the button to
  stop it.

On Settings, tap **THEME** for the next theme, **LIGHT** to switch
brightness from AUTO to SOFT, a step dimmer, for a longer battery, and
**BUTTONS** to turn on the [button grid](docs/buttons.md), and **GRID**
for its layout: six buttons, or two layers. To set
an alarm or timer, tap it, then swipe the hours and minutes. Alarms set
in Gadgetbridge are read-only on the watch; a one-off one switches
itself off once it has rung, without Gadgetbridge hearing of it, so
switch it off and on there to set it again.

Gadgetbridge forwards directions from Google Maps and OsmAnd by default;
turn CoMaps on in its settings, under "Navigation apps".

[Health Connect]: https://developer.android.com/health-and-fitness/guides/health-connect

## Installing

Download `ur-<version>-dfu.zip` from the [releases]: the same package
installs ur over InfiniTime and updates it afterwards.
`ur-logging-<version>-dfu.zip` is the same firmware keeping a log in the
watch's flash, for reporting problems; it costs a little battery, and
[CONTRIBUTING.md](CONTRIBUTING.md#field-logs) explains fetching the log.

Send the zip over Bluetooth with either of these:

- **Nordic's [nRF Device Firmware Update][nrf-dfu-android] app** (also
  on the [App Store][nrf-dfu-ios]): select File, the ZIP ("Distribution
  packet") type, and the zip; then Device, the watch ("InfiniTime" the
  first time, "Bangle.js ur" after that); then Start. For updates, use
  the phone Gadgetbridge paired with the watch: the app relies on that
  pairing (see [U01](#u01-a-phone-update-fails-at-enabling-bootloader-or-with-gatt-conn-timeout)),
  and on Android, turn on **Keep bond** in the app's Settings first, or
  the phone forgets that pairing after each update (see
  [U03](#u03-the-phone-has-to-pair-again-after-every-update)).
- **A Linux computer**, with `scripts/dfu.py` from this repository
  (`pip install bleak` first). `<MAC>` is the watch's address, which
  `bluetoothctl` shows while scanning.

  ```sh
  # The first install:
  python3 scripts/dfu.py ur-<version>-dfu.zip --address <MAC>
  # Updates, once paired in bluetoothctl (agent KeyboardOnly,
  # default-agent, pair <MAC>, then type the passkey the watch shows),
  # take about half a minute:
  python3 scripts/dfu.py ur-<version>-dfu.zip --address <MAC> --packet-size 240
  ```

[releases]: https://github.com/jonhoo/ur/releases
[nrf-dfu-android]: https://play.google.com/store/apps/details?id=no.nordicsemi.android.dfu
[nrf-dfu-ios]: https://apps.apple.com/app/nrf-device-firmware-update/id1624454660

### The first install, over InfiniTime

1. On InfiniTime 1.16 or later, turn on the OTA switch in Settings. If
   you're sending with the nRF app and Gadgetbridge is connected to
   InfiniTime, disconnect it there: InfiniTime takes one connection at a
   time, so the app wouldn't find the watch.
2. Send the zip. If the nRF app stalls at 14% with "GATT CONN TIMEOUT"
   (InfiniTime's side of the transfer), send it from a computer instead.
3. The watch shows the PINE64 logo for about a minute while the
   bootloader swaps the images. Leave it be until ur's clock face
   appears.
4. In Gadgetbridge, remove the InfiniTime device, and forget the watch
   in Android's Bluetooth settings too. Then add "Bangle.js ur" in
   Gadgetbridge, choose Pair, and type the passkey the watch shows.
   Within seconds, Gadgetbridge shows the battery, and firmware "ur"
   with its version; if not, disconnect and connect again there (see
   [C05](#c05-gadgetbridge-shows-no-battery-or-firmware-na)). The watch
   keeps ur once the phone has set the time; until then, a reset puts
   InfiniTime back.

### Updating

Send a newer zip; Gadgetbridge can stay connected. The watch resets into
the new version, and keeps it once Gadgetbridge has synced the time
again; a reset before that goes back to the old one. Today's steps, and
the activity the phone hasn't fetched, carry over.

To roll back by hand, hold the button for about 7 seconds, until the
watch resets into the [bootloader], and keep holding: release when the
logo turns blue for the previous firmware, or red for PINE64's recovery
firmware, a minimal InfiniTime to reinstall from.

[bootloader]: https://github.com/InfiniTimeOrg/pinetime-mcuboot-bootloader

### Verifying

Each release file carries a GitHub [attestation][attestations]. To check
that a download is exactly what this repository's release workflow
built, with the [GitHub CLI][gh attestation verify]:

```sh
gh attestation verify ur-<version>-dfu.zip --repo jonhoo/ur
```

Releases are also reproducible: [CONTRIBUTING.md](CONTRIBUTING.md#releasing)
shows how to rebuild one and compare.

[attestations]: https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-to-establish-provenance-for-builds
[gh attestation verify]: https://cli.github.com/manual/gh_attestation_verify

## FAQ

When the watch points you here, it shows an entry's code: a letter for
its area (B the button grid, C connectivity, U updates, P power, S
sensors, D display and touch, A alarms, N notifications, E errors), and a
number.

### B01: Home Assistant fires two events for each grid press

With **GRID** on **2 LAYERS**, that's by design: see
[B04](#b04-with-two-layers-presses-reach-home-assistant-as-dimmers).

### B02: Home Assistant has no trigger for a grid gesture

It lists a BTHome device's events only once it has seen each: tap,
double-tap and hold (for 1.5 s, for "Long press" too) each grid button
once (with two layers, any one).
"Hold release" needs Home Assistant 2026.11 or later; older versions
ignore it.

### B03: Home Assistant asks for the watch's key again

The watch made a new key: it lost track of how many presses it had
encrypted with the old one, and reusing that count would let anyone
listening decode them. Give Home Assistant the new key from **Settings →
BTHOME KEY**, as when you [set it up](docs/buttons.md#home-assistant).

### B04: With two layers, presses reach Home Assistant as dimmers

The watch shows **\*B04** by **GRID** when it's on **2 LAYERS** and
**BUTTONS** on **BTHOME** or **BOTH**. Two layers make codes 1–99 (none
ends in 0: the first button is the tens digit, the second the ones),
and BTHome can't send each as a button of its own. Its buttons carry no
number, so Home Assistant tells them apart only by their place in the
advertisement, and an encrypted advertisement has room for about 15
bytes of data: six buttons, not one per code. So each press fires two
events instead: a `button` event for the gesture (`press`,
`double_press`, `hold_press`, `long_press` if held on to 1.5 s, then
`hold_release` on lifting after a hold), and a dimmer event, which can
carry a number, turning by the code: `dimmer_1` right for a tap,
`dimmer_2` right for a double tap, `dimmer_3` right for a hold, and
`dimmer_3` **left** for a long press (none for a release). To act on
one code, trigger on its dimmer and direction (`rotate_right` or
`rotate_left`: without it, a hold's trigger fires for its long press
too), in YAML (a **Manual event** trigger in the automation editor).
For 2, then 3, double-tapped:

```yaml
triggers:
  - trigger: event
    event_type: bthome_ble_event
    event_data:
      address: "E5:12:34:56:78:9A"   # from BTHOME KEY
      event_class: dimmer_2          # double tap
      event_type: rotate_right
      event_properties: { steps: 23 }
```

Or skip the YAML: the two-layer [blueprint](docs/buttons.md#blueprints)
matches codes and gestures for you. [docs/buttons.md](docs/buttons.md#with-two-layers)
covers both ways in full.

With **6 BUTTONS** instead, each button is its own BTHome button
(`button_1` to `button_6`), with a trigger in the automation editor for
each button and gesture, no YAML needed. See
[docs/buttons.md](docs/buttons.md#home-assistant).

### B05: The watch says "BTHOME NOT SENT"

The watch couldn't send that grid press to Home Assistant: both its
Bluetooth connections were in use, usually the phone plus a computer
(fetching the field log, or updating it). Disconnect the computer, and
press again.

### C01: The watch says "NOT PAIRED"

A phone or computer connected without pairing, and ur only talks to
devices paired with its passkey. Remove the watch from Gadgetbridge, if
it's there, and forget it in Android's Bluetooth settings (Android may
still hold InfiniTime's pairing). Then add it in Gadgetbridge, choose
Pair, and type the 6-digit passkey the watch shows.

### C02: The watch says "PAIRING LOST"

The watch remembers pairing with this phone, but the phone has forgotten
it, for instance after the watch was removed from Android's Bluetooth
settings. Remove the watch from both Android's Bluetooth settings and
Gadgetbridge, then add it again in Gadgetbridge and type the passkey.

### C03: The watch never gets a location from the phone

Without one, the themes and brightness follow the sun in Oslo. The watch
asks Gadgetbridge for the phone's location when it connects (at most
once an hour), when the time zone changes, and when it stops hearing a
room sensor (at most four times a day), and keeps the last one.
Gadgetbridge only passes on fresh locations, and only with all of these
on:

1. In the watch's settings in Gadgetbridge: "Use phone GPS data", with
   "Only use network to determine location" off (with it on, no location
   ever arrived in testing).
2. In Gadgetbridge's own settings, under Location: "Keep location
   updated".
3. In Android's settings for Gadgetbridge: location permission "Allow
   all the time", and location itself on.

Phones without Google Play services may have no network location, so
nothing arrives indoors until the watch happens to ask while the phone
is outside.

### C04: The watch says "INTENTS OFF"

Gadgetbridge didn't pass a [button grid](docs/buttons.md) press on to
other apps: turn on "Allow Intents" in the watch's settings in
Gadgetbridge.

### C05: Gadgetbridge shows no battery, or firmware "N/A"

Nothing the watch sends reaches Gadgetbridge: it most likely asked for
it before the link was paired, was refused, and carried on regardless.
The watch notices within about 10 seconds and disconnects once, and
Gadgetbridge reconnects and asks again. If that didn't help:

1. In Gadgetbridge, disconnect the watch and connect it again.
2. Check its device type in Gadgetbridge is Bangle.js. An entry left
   from InfiniTime for the same watch never works: remove it.
3. Remove the watch from Gadgetbridge and Android's Bluetooth settings,
   and add it again as in [C01](#c01-the-watch-says-not-paired).

Steps and sleep come separately, when Gadgetbridge fetches them: see
[Using it](#using-it).

### N01: Notifications don't buzz, and the screen stays dim

The watch thinks you're asleep: it's a quiet night. It ends with the
morning's first alarm, or once you've been up for 22 minutes (7 after a
nap, which is sleep between 09:00 and 20:00). If you're up before
either, double-press the button.

### N02: The watch says "TEXT AS BITMAPS"

Gadgetbridge's "Text as Bitmaps" is on: it sends what the watch's font
can't show as pictures, which ur shows as `?` (and long messages full of
them never arrive). Switch it off in the watch's settings in
Gadgetbridge, under Notifications.

### P01: How do I put the watch away for a while?

Use storage mode: long-press the Info screen (swipe left from the clock,
past Alarms, any alarms from Gadgetbridge, and Settings), then tap to
confirm. The watch saves the activity the phone hasn't fetched and
switches itself off, using roughly 40% of a full charge in six weeks.
Press the button or plug in the charger to wake it; the phone sets the
time when it reconnects, and gets the saved activity with its next sync.

### U01: A phone update fails at "Enabling bootloader" or with "GATT CONN TIMEOUT"

The nRF Device Firmware Update app only works over a paired link: if the
phone hasn't paired with the watch, the app tries a different kind of
update, and fails. Pair first (see [C01](#c01-the-watch-says-not-paired)
and [C02](#c02-the-watch-says-pairing-lost)), then send the update again.

### U02: The watch says "UPDATE REFUSED"

The update arrived, but the watch checked it before installing it and
found something wrong: it was damaged or cut short on the way, or it
isn't firmware for this watch (a zip for another device, say). The watch
keeps the firmware it has. Send the zip again; if it's refused again,
download it again, and [verify](#verifying) it.

### U03: The phone has to pair again after every update

The Android nRF Device Firmware Update app makes the phone forget its
pairing with the watch when an update finishes, unless **Keep bond** is
on in the app's Settings (under "Legacy DFU"); it's off by default. The
watch still remembers the phone, so it says "PAIRING LOST". Pair again
as in [C02](#c02-the-watch-says-pairing-lost), and turn on Keep bond
before the next update.

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) covers building, iterating on a dev
kit, fetching logs from a sealed watch, and making releases.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

The firmware images also contain third-party code under its own terms:

- Nordic Semiconductor's Bluetooth controller and protocol scheduler
  (through the `nrf-sdc` and `nrf-mpsl` crates), provided in binary form
  under the Nordic 5-Clause licence, which only permits their use with
  Nordic chips, such as the PineTime's nRF52832.
- `src/bma423_config.bin`, Bosch's BMA423 feature-engine configuration,
  under the BSD-3-Clause licence in `src/bma423_config.LICENSE`.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
