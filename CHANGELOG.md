# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions up to 0.8.17 were development builds, installed only on the
author's own watches. Cargo.toml still said 0.1.0 then, so their
version lived only in the update image's header. Version numbers
missing below were test builds for the dev kit.

## [Unreleased]

### Added

- Add themes as build features: how every screen looks is a theme (`src/themes/`), Retro (today's look, with ARCADE and TERMINAL by night and HANDHELD and PARCHMENT by day) by default, and a build can have other themes instead or as well (`theme-<name>`, or `themes-all`), LOOK's THEME then stepping through every scheme built in for the time of day; a release has a package of each other theme, `ur-<theme>-<version>-dfu.zip` (one with only night or only day colour schemes shows them by day and by night alike, its FORCE fixed); and the chosen schemes are now saved by name, so they survive a build with other themes, and a watch updated from 0.10.0 keeps its own (6ff3133, c2d0b0c, 59aa7ad, bfa2d57)
- Add a FORCE row to LOOK, under THEME: NONE follows the sun as before, and DAY or NIGHT shows that side's colour schemes whatever the time, THEME then stepping through those and saving the choice as that side's; only the look, as the brightness and everything else still follow the sun. With one scheme for the time of day, THEME can't change, nor FORCE in a build with schemes of one side only: either is dim, and a tap says why, "ONLY ONE THEME" / "SEE FAQ D01", or "NO DAY THEMES" (or NIGHT) / "SEE FAQ D02" (98b856d)

### Changed

- Send the night to Gadgetbridge, and so to Health Connect, about 47 minutes after you get up rather than 2¼ hours after you last slept: once you've been awake for 46 minutes, walked or had the watch off your wrist for at least two minutes of them, and aren't lying still, the watch settles the night's sleep and stages for good and the next fetch gets it, and the morning's steps after it go as they come (but for a stillness of 10 minutes or more, which may yet be a nap); a lie-in after that is a sleep of its own, and a morning without walking or taking the watch off, or a short nap, waits the 2¼ hours as before (introduced in 0.10.0) (79c00c8, f41606c)
- Ring smart wake early only at a stir after ten minutes of light sleep (none of them deep or REM, nor all still), or at its time if that doesn't come, rather than at the first stir in its window, which came within its first few minutes on most mornings, from deep and REM sleep as often as light: against a Garmin over four nights, rings before the time from deep sleep went from 11% to almost none, and from REM from 37% to 21%, while about a quarter of mornings now ring at the time (a7f266e)
- Split the Settings screen in two, side by side where it was, before Info: LOOK, with THEME, FORCE and LIGHT, and BUTTON GRID, with BUTTONS, GRID and BTHOME KEY; three rows at most each, so every row is taller, about 7 mm rather than 5, for a fingertip, and none moves as another comes or goes (98b856d)
- Try longer heart-rate measurements on a busy wrist in the logging build (`ur-logging`), an experiment whose field logs will say whether more time reads more, and how well: on half the awake rounds of background measurements, one that has no reading after its usual 30 s on a busy wrist goes on to 90 s, and one that reads early on a busy wrist keeps recording to 90 s (its first reading counts as before; what it reads after is only logged, never shown or sent), each when the hourly LED budget can spare it and still leave the next try its share; and every awake measurement now logs the accelerometer's samples alongside, to tell a walk's rhythm from the pulse. That's ~0.4 MB more log a day, so the log holds ~22 hours rather than ~26 (fetch it daily), and ~2 mAh a day (up to ~8 if the light draws more than its datasheet says). The `ur` build is unchanged (5b59a9a)

### Deprecated

### Removed

### Fixed

### Security

## [0.10.0] - 2026-11-11

The new ideas in it: a **button grid** (Home Assistant over BTHome, or
Tasker, with a blueprint for each layout) and a **Settings screen**;
**room sensors** (Aranet4 air monitors and BTHome thermometers) with an
**Air screen**, and the **room's temperature** on the clock face, from a
sensor or the wrist's self-calibrating estimate; a **redesigned clock
face** (the day's steps as blocks towards 10,000) and **notifications**;
**new media gestures** (double-tap to play or pause, tap then hold to
skip, replacing 0.9.0's); a **heart rate** that reads more often, at a
desk, through movement, and from a faint pulse, and **sleep stages**
closer to a Garmin's; and **every step and the settled night in
Gadgetbridge and Health Connect** (5-minute samples, the night sent in
the morning).

### Added

- Add a button grid for Home Assistant or Tasker, the first screen below the clock face once turned on in Settings, of retro arcade buttons in one of two layouts (Settings → GRID): six buttons (the default), each tapped, double-tapped, held, or long-pressed (held 1.5 s), with a hold's release sent too (for hold-to-dim), which reach Home Assistant as BTHome button events of their own, `button_1` to `button_6`, a trigger each in the automation editor; or two layers of nine buttons, tapped alone or in pairs for codes 1–99, which reach Home Assistant as a button event for the gesture plus a dimmer turned by the code (FAQ B04, which the watch points to as "*B04", explains why a long press turns it left); sent over Bluetooth as encrypted BTHome, or through Gadgetbridge as an Android intent (see docs/buttons.md); a press that can't go to Home Assistant turns red, with "BTHOME NOT SENT" / "SEE FAQ B05", and with Gadgetbridge's "Allow Intents" off, the watch says "INTENTS OFF" / "SEE FAQ C04" (3f335fe, 4accb11, 92340bc, ff82ac9, e754fa6, 02160d9, 24d1126, 28f90ae, eb68544, c470936, 77bc5d6, 3d06fc2, 6c5db54, 6fa7f9f, 11624d7)
- Add a Settings screen right of the clock face, between Alarms and Info: THEME and LIGHT (moved from Info), BUTTONS (OFF by default, or where the button grid's presses go: BTHOME, VIA APP for Gadgetbridge, or BOTH), GRID (its layout), and, while BUTTONS includes BTHome, BTHOME KEY, the button grid's key for Home Assistant, as a QR code to scan and paste, or as text, either on screen until tapped away (3f335fe, b6cb498, 7c69c61, 02160d9, 5386e38, 3cae84f, 5d1fc55)
- Add a Home Assistant blueprint for each button grid layout, imported with one click (see docs/buttons.md), to set every button's actions in one automation from the editor rather than one automation each: for six buttons, an action per button for a tap, double tap, hold, long press, and release; for two layers, the same for each button on its own (bar the double tap), eight slots for any code and gesture (23 double-tapped, say), and one action for every press, given its `code` and `gesture` (f4a051f, 4d43551)
- Read a nearby Aranet4 air monitor's advertisements (with its Smart Home integrations on), listening briefly every 15 minutes while one is around and hourly otherwise, and show the nearest one's CO₂ (with its band), temperature, humidity, pressure, battery, and age on a new Air screen below Music, which is there while one was heard within the hour (e1711da, 099659e)
- Read nearby BTHome thermometers too (unencrypted, such as a Xiaomi LYWSD03MMC on pvvx's firmware) as room sensors alongside Aranet4s: the nearest of either, if heard stronger than −80 dBm (so a neighbour's thermometer through a wall doesn't become the room's), gives the clock face's temperature, and the Air screen shows its temperature big, with its humidity, pressure, and battery if it sends them; one that sends its temperature every few seconds is heard at each look, one that sends seldom (a Shelly BLU H&T, once a minute) only now and then, and a new sensor is found within about two hours even beside ones the watch already hears (d71ecbb, 79bce0e, 5151105)
- Forget room sensors as soon as the phone's location (when it sends one) is over 200 m from where they were heard, without waiting out the hour since they were last heard: the Air screen and the sensor's temperature on the clock face go at once, and the watch looks for sensors only hourly, until a location back within 200 m has it look again at once; only locations the phone says are good to 75 m count (a coarse one keeps the phone looking, for up to the usual 2 minutes), and the watch asks the phone for one when its scans stop hearing the sensors (at most hourly, and four times a day), and when it first hears them after starting (db26de7, 6615c88, 6112ed8)
- Show the heart rate and steps live in Gadgetbridge's Live activity chart (Charts, "Live activity"): while it's open, the watch measures the heart rate continuously and sends a reading and the steps since the last one every 10 s (or as often as Gadgetbridge asks), until the chart closes or the phone disconnects; off the wrist it doesn't measure until the chart is reopened (a8c4c5f, f9cd658)
- Say "TEXT AS BITMAPS" / "SEE FAQ N02" (once per connection) when Gadgetbridge sends a word as a picture, as its "Text as Bitmaps" setting does for words with characters the watch can't show, and show each such word as a `?` rather than a run of junk; FAQ N02 says how to switch the setting off (46d43f3)
- Add a guide to Gadgetbridge's settings for ur (docs/gadgetbridge.md): which to change, which to leave alone, and which are up to you, each with where it is, its default and why (above all "Auto fetch activity data", off by default, without which a day not fetched by hand is lost after 24 hours) (d1bf562, 76b493f)
- Add FAQ U03, for a phone that has to pair again after every update: turn on "Keep bond" in Nordic's nRF Device Firmware Update app (5b48603)

### Changed

- Play or pause media with a double tap, and skip with a tap then a touch held until the buzz (0.6 s), with the screen off, on a glance, or on the clock face, rather than with a long press and a double tap, so an arm folded over the watch no longer plays or pauses (as it could since 0.9.0); a long press on its own now does nothing, and with the screen off or on a glance, the hold must be in the middle of the screen and a touch near the edge does nothing (1c1460d, 41c9753, 2e774fa, 9abc714)
- Redesign the clock face's lower half: the date with small seconds under the time, the heart rate and the next alarm or timer in one row, and the day's steps as ten blocks rather than a number: 1,000 a block (×1K) towards a fixed goal of 10,000, then 10,000 a block past it, its scale (×10K) in the accent colour to mark the goal reached, full at 100,000, the blocks still to fill faded even in the dim look (the exact count is on Today, a swipe right) (a02d355, 718c01d, 6147206, d4525fe)
- Show the room's temperature on the clock face rather than the watch's own: a nearby room sensor's, to the tenth (`21.4°`); or else, on the wrist, an estimate from the watch's own, to the degree (`21°`), which calibrates itself against any room sensor it meets reading 10–35 °C while worn (roughly ±2–3 °C indoors once calibrated), and holds through a still night while heart rate readings keep coming; or, off the wrist, the watch's own, to the degree (e1711da, 13c30c8, 216a6f9, 373457b)
- Show a faded Bluetooth rune struck through in red on the clock face while the phone isn't connected, rather than a Bluetooth rune while it is; the note for music playing moves to the top left corner in its place (2e1f1e1, a1c8505)
- Redesign notifications: with the screen off, and in the history, set who it's from big and centred only if it fits on one line, or on two of about equal width, rather than whenever it fit on four lines (three in the history), which left stubs such as `ADA / LOVELACE`, and otherwise centred in a new bold medium face over as few balanced lines as it takes, rather than small; show both as one card, the app top left over a rule, and in the history when it arrived (as `SUN 20:13`) top right rather than at the bottom; let the banner take a second line for a long name, ending in an ellipsis, rather than scrolling; and leave a slightly wider margin either side of notification text, as a call's has (86ebb6a, 5f98db4, 116942b, 0eaa4c1)
- Show labels, titles, the seconds, and other secondary marks in red in the dim look (glances, notifications with the screen off, and the quiet night), rather than in the same white as the text; on Last night, deep sleep is white, REM red, and light sleep grey (30fb455, ef1d58c)
- Read the heart rate at a desk more often: up to 3 s of hand or finger movement in the 8 s the watch measures over are now left out, and the rest used, rather than the whole 8 s thrown away, and the first conversion after the sensor lights, which comes out darker, no longer drowns the first seconds' pulse; replaying field logs, desk sessions that ended in a reading went from 4 to 18 of 131 (within 5 BPM of a Garmin 9 times in 10), and night readings come sooner, at the same accuracy. A heart rate under 50 now also needs a still wrist unless you're asleep, so fidgeting at a desk isn't read as a slow pulse (5abc884, 676656c, 0b1c769)
- Read a faint pulse on a still wrist, such as lying down awake with cold hands: there, two 8 s stretches in a row that agree within 3 BPM now make a reading, even if neither is clear enough on its own, so most readings also come ~2 s sooner (a night's LED time falls ~15%). Replaying field logs against a Garmin, readings went from 106 to 118 of 252 sessions, as accurate as before, and an hour lying still that gave none now gives one (4aefe15)
- Keep measuring the heart rate through movement: a background measurement waits at most two minutes for a still moment to start (or not at all once there's been no reading for half an hour), rather than however long it takes (busy desk work held one back 18 minutes), and then runs its full 30 s however the wrist moves, rather than giving up after 12 s; to make room, the hourly LED budget rises from 180 to 300 s, at most ~5.6 mAh a day rather than ~3.3 (bd66bc4, f4bbe32)
- Try harder for a heart rate while measurements keep failing: after one fails on a still wrist, try again within 10 minutes rather than backing off to 20; and once there's been no reading for half an hour, asleep or awake, retry every failed measurement within 10 minutes until the next reading, rather than backing off to 20 minutes or an hour (unless the watch is off the wrist), with the hourly LED budget doubled meanwhile so more and brighter tries fit; at worst (a whole day without readings), that's ~5.6 mAh a day more (54c553d, 12e5c93, 090c5a3)
- Try a brighter heart-rate light, an experiment whose field logs will say whether more light helps, at a desk above all: awake, every other background measurement, together with its retries until one reads, now starts at 30 mA rather than 12.5 (or 20 mA if 30 would overwhelm the sensor), so a day's log compares the two on the same wrist, with first tries and retries alike on each; and one that gives no reading in its usual 30 s, on a calm wrist with only a faint pulse, now goes on for up to 15 s more at a brighter setting (and once more at 40 mA if that fails too). The hourly LED budget now counts a brighter second for the current it draws, and a brighter try is only made when the budget can spare it, so ordinary readings aren't held back (633489f, 6a47d2e, 28900de)
- Tell sleep stages apart better: deep sleep now also needs a heart rate that holds steady from one reading to the next, and REM a smaller rise (4 BPM over the slowest of the surrounding hour rather than 5) along with the odd twitch of the wrist; against a Garmin worn alongside, a night that came out with too much deep and too little REM went from 48% to 61% of minutes in agreement, and an earlier one from 67% to 70% (b526bbd)
- Move Info to the right of the clock face, last after Alarms and the new Settings screen, rather than at the end of the screens below it, and its THEME and LIGHT rows to Settings, leaving Info to the watch's status; storage mode's long press moves with Info (now with a "HOLD: STORAGE MODE" hint at its foot) (5f5acf2, 3f335fe)
- Answer Gadgetbridge's "Find lost device" with a screen of its own, "FIND MY WATCH" over a big HERE with rings spreading round it, and the motor's loudest buzz, rather than a call's buzz alone; it now stops at the button or a tap (whoever touched it found it), as well as at the phone's "Found it!", and after two minutes at most, rather than ringing until the phone said stop. It still rings in the quiet night (in the dim look), and waits its turn behind a ringing call or alarm (f86fd26)

### Fixed

- Get every step into Gadgetbridge, and so into Health Connect, which kept only about 40% of them (a day's 10,346 arrived as 4,206): Gadgetbridge merges samples 2 minutes or less apart, keeping the last one's steps, and the watch sent one a minute; it now sends one per 5 minutes, with all their steps, the last heart rate reading (earlier ones in the same 5 minutes are dropped), and the sleep stage most of them had, so Gadgetbridge lags the watch by up to 5 minutes, and sleep there is to the nearest 5 minutes. Steps also land in their own minutes in Health Connect, rather than a minute early. Updating from 0.9.0 counts no step twice in Gadgetbridge, at the cost of up to 5 minutes' steps once; and each fetch sends only what Gadgetbridge doesn't have yet, rather than the last 2¾ hours again (13b318e, 8f7afee, bba7496, 98f8253)
- Stop sleep stages freezing half-revised in Gadgetbridge and Health Connect: Gadgetbridge keeps the first sleep stage it gets for a sample, so a fetch during the night, or soon after waking, froze stages the watch was still revising there for good. The night now reaches Gadgetbridge in the morning, once its stages are settled: about 2¼ hours after you last slept, the first fetch gets the whole night at once, steps included, and passes it on to Health Connect straight away; until then, fetches stop where you fell asleep (or a nap began, or the last 20-odd stepless minutes of sitting still, in case it's sleep), holding back at most 18 hours (introduced in 0.9.0) (6ce2535)
- Make the Health Connect export from Gadgetbridge ("Sync after device sync") keep running, rather than only once when turned on: the watch now ends its first sync after starting, and then a sync at most every 2 hours, in a way that Gadgetbridge counts as a finished sync, so each export catches up on everything since the last one and Health Connect lags the watch by up to 2 hours (Gadgetbridge shows a short "End : Fetching sports details" each time) (f733856, cf88807)
- Get notifications, music, and alarms from the phone onto the watch within about a second once the connection has been quiet for 30 s, rather than a few seconds a line (a long notification took up to 12 s, a burst of them longer): the watch now asks for Bluetooth's long packets, and while the phone is sending, listens at every turn rather than every eighth (4b3ed08, b4575d7)
- Reconnect Gadgetbridge, once, when it connects but never subscribes to what the watch sends (it then shows no battery and firmware "N/A", and gets nothing from the watch), as it can when it asks before the first pairing has finished; it reconnects at once and subscribes properly, and the new FAQ C05 covers the rare case where that doesn't help (ff3b3b2)
- Show long notifications in scripts beyond Latin-1 (Cyrillic, Greek, CJK, Arabic…), which were lost whole, never shown nor buzzed, once their body passed about 150 characters (Gadgetbridge sends each such character as six bytes, and the message outgrew what the watch could take in); likewise, a very long caller, track, street, or title, such as one that Gadgetbridge's "Text as Bitmaps" turns into pictures, is now cut short rather than losing the whole message (47fa44d)
- Keep notification names (the sender, or without one the title) of up to 40 characters whole, however many accented letters such as æ, ø, and å they have (in 0.9.0, 20 of them filled it), and end one that's cut with an ellipsis rather than passing it off as whole; leave emoji out, rather than showing each as `?` (and show the title for a sender that was only emoji); and draw Ø and Å so they don't read as an 8 (86ebb6a, 00024b0)
- Show `&`, `#`, `@`, `"`, `*`, and the rest of ASCII's symbols (and `×`) in notifications, and curly quotes, dashes, and ellipses as their plain look-alikes, instead of `?` (929a956, 718c01d)
- Show the battery level without overstating it, so it no longer drops by ~10 points minutes after the watch comes off the charger: the percentage now follows the PineTime cell's voltage curve rather than a straight line, which overstated the middle of the range by 10–20 points (so red under 20% now means under 20%), climbs on the charger no faster than the cell fills, to 99% until charging is done, and off the charger never climbs and falls at most a point every 3 minutes; Gadgetbridge and Home Assistant get the same number, Gadgetbridge now every half hour even when it hasn't changed (7f78fe8, ff3b3b2)
- Restart and charge when plugged in during the "battery empty" switch-off, rather than staying off until the battery runs flat again, possibly for months (introduced in 0.9.0) (3baa4ec)
- Put the accelerometer, the heart-rate sensor, and the touch controller to sleep when the watch switches itself off (battery empty, or storage mode), even if one doesn't answer at first: one failed write used to fail every write after it, so the watch could switch off with the accelerometer running (~50 µA, more than everything else left on together), or the heart-rate light lit (12.5 mA or more) if a measurement was under way, draining the battery the switch-off was meant to save (not yet seen on a watch) (d5b117e)
- Stop the watch sometimes resetting as it saved something to its flash (a setting, the alarms, a pairing, the temperature calibration, the button grid's counter, or confirming an update), or, more rarely, never finishing the save, which kept every later save from happening until a restart (caead12)
- Refuse a firmware update that's damaged, cut short, or not firmware for this watch, checking it as it arrives and again before restarting into it, and say "UPDATE REFUSED" / "SEE FAQ U02" while keeping the running firmware, rather than restarting into the bootloader only for it to silently refuse it (or, for an image built for another device, install it); also give up on an update whose sender goes quiet for a minute, rather than keeping the progress screen lit while it stays connected, and take one update at a time (c569ee4)
- Recover the step counter, heart rate, and touch when the sensors' connection gets stuck, as a restart (such as an update's) can leave it, rather than counting no steps, measuring no heart rate, and ignoring touches until the next restart: the sensors now keep trying to start, ever less often down to once a minute, and the connection is freed whenever a reading fails, not only at start-up (21d3cc1)
- Switch the heart-rate light off even when the first try fails, retrying every second, rather than leaving it lit (12.5–40 mA) until the next measurement, up to an hour away, or for as long as the clock face showed (not yet seen on a watch) (7f3bb9c)
- Keep counting steps after a garbled reading of the step counter, which could add billions of steps to the day and then count none until a restart (not yet seen on a watch) (3f913b9)
- Ring the smart alarm when its time passes while another alarm rings, rather than not at all that morning (an alarm at 06:58 left to ring out skipped a smart wake at 07:00): alarms whose times come round while another rings now queue, each ringing as soon as the one before is done, at most an hour late; and an alarm at the smart alarm's own minute no longer makes the watch ring again straight after it's dismissed or snoozed (8c420f3)
- Ring each alarm, the smart alarm included, exactly once on the nights summer time starts and ends: one in the skipped hour (02:30, say) now rings as the clock jumps to 03:00 rather than not at all, and one in the repeated hour rings in the first 02:00 only, rather than in both; beyond an hour, a new time zone still skips what it jumps over, and rings again what comes round again (2c6e399, 8c420f3)
- Keep a one-off alarm from the phone off once it has rung at the same minute as one of the watch's own: it was saved as still on, so after a restart before Gadgetbridge next sent its alarms, it rang again the next morning (introduced in 0.9.0) (fb51f17)
- Keep an alarm ringing through a call: the call ending (answered, declined, or over), or Gadgetbridge's "Find lost device" stopping, left the alarm on screen but silent; and stop a notification arriving during a call from cutting its buzz short (3451cca)
- Keep timers and the snooze counting from when they were set, whatever the phone does to the clock: a new time zone shifted them by the difference (a 10-minute timer took 3 h 10 min after flying three zones west), and the clock set far back made opening a running timer reset the watch; a timer now also runs before the phone has set the time (34f5662, 9c65743)
- Count today's steps from midnight in the new time zone after a flight across midnight, as phones do, so the clock face and Today agree: flying west, the evening after landing counted from 0 as a new day (the clock face said 5 steps while Today said 1,205), then started over again at the new zone's midnight; flying east, the count started over at the change rather than at the new zone's midnight. Steps taken before the phone first set the clock that day are dropped by such a change, and a new time zone on the same date leaves the count alone (3438411)
- Keep the day's activity, sleep, and Gadgetbridge's samples straight when the phone puts its clock back (because it was wrong, say): what was recorded under the clock that was ahead is dropped, rather than the minutes recorded from then on being added on top of it (or, over a day back, filed under days still to come and sent to Gadgetbridge as such), with sleep tracking stuck until the clock caught up; and the record the watch saves on switching off now keeps what was recorded since the clock was put right, rather than none of it (81d36d7, b8ecd08)
- Stop a malformed alarm list from the phone hanging the watch until the watchdog reset it (on every connect, if the phone kept sending it; introduced in 0.9.0), and ignore a time set before 2020 or after 2100 (e24a3bb)
- Turn the screen off after a notification that lit it in the dark, even if a second one arrived just as the quiet night began while the alarm editor was left open: the screen stayed on, dimly, until the next touch or press, all night if need be; the screen going off now closes the alarm editor (keeping what was edited, as it's saved as it changes), so the next wake is to the clock face, where the button turns the screen off again, rather than back into the editor (323af29)
- Leave no empty space before the temperature on the clock face once media is paused or stopped: the temperature now moves to where the media icon was once the icon has gone (introduced in 0.9.0) (fcd0a6d)
- Draw deep, REM, and light sleep on Last night in three distinct colours in every theme; deep and REM were the same red in Arcade and Parchment (introduced in 0.9.0) (a1f642d)
- Make releases rebuild byte for byte whether or not the builder already has the dependencies downloaded; 0.9.0 only reproduces from an empty cargo home (e49cb26)

### Security

- Keep pairings from outside meddling with the phone's bond: a pairing without the passkey no longer leaves a bond (one claiming the phone's address replaced the phone's), a failed one no longer drops the phone's bond until the next reboot, and after each failed pairing the next waits longer, up to a minute, so a passkey can't be guessed at speed or the screen kept lit (1700f15)

## [0.9.0] - 2026-11-08

The first release published on GitHub. The new ideas in it:
**quiet nights** (notifications stay silent and dark while you sleep or
nap, until your alarm), **screens around the clock** (swipe for today and
last night, alarms, notifications, music, and info), **sleep stages**
(light, deep, and REM), **directions on the glance** while the phone
navigates, **media gestures** (long-press to play or pause, double-tap to
skip), **storage mode**, and a **paired phone** is now required (FAQ C01
if the watch says it isn't).

### Added

- Keep the last three notifications above the clock face, one a page, each with when it arrived, and a mark on the clock until you swipe down to them (0b74bd9, 9bd8cc8, de479e1)
- Play, pause, and skip media whenever the phone is connected, from the clock face, a glance, or with the screen off (long-press to play or pause, double-tap for the next track; only on the awake clock face during a quiet night), with a double note beside the Bluetooth icon while it plays (6667727, b9d4477, 36aa727, fff3477)
- Show the next turn on the glance while Google Maps, OsmAnd, or CoMaps navigates on the phone (an arrow, the distance, and the street), and buzz twice and show it as a turn comes within 50 m (49f66b8)
- Show today's steps and heart rate hour by hour, with a resting heart rate, and last night's sleep with its stages, on screens of their own left of the clock face (the Info screen's SLEEP row moves there) (0b74bd9, 8519198)
- Tell light, deep, and REM sleep apart from movement and heart rate, rather than deep sleep from long stillness alone, and sync them to Gadgetbridge (and from there to Health Connect) (d9249b8)
- Keep notifications quiet while the wearer sleeps or naps, without a buzz or light, the screen dim and ignoring touches (alarms, timers, and calls still ring), until their alarm, until they've been up a while, or until they double-press the button (20b73a8, 1cf1618)
- Sync the alarms set in Gadgetbridge to the watch, where they ring like its own (ending a quiet night) and show, read-only, on a screen of their own (c1e45cb)
- Save the activity the phone hasn't fetched and switch off when the battery runs flat, or when put away in storage mode (long-press the Info screen, then tap), keeping it until the phone next syncs; the watch then doesn't know the time, or ring alarms, until the phone reconnects (9d067b6)
- Show the watch's own temperature on the clock face (close to your skin's on the wrist, drifting towards the room's off it) (7874b05)
- Record a backtrace and fault details for crashes, hangs, and stack overflows, reported in the logging build's field log at the next boot (1e5b869, 6b4ded1)
- Publish releases on GitHub, with packages you can verify came unmodified from the tagged commit, and rebuild byte for byte from it (bca9899, 63215e2)
- License the firmware as MIT or Apache-2.0, at your option (9c1ec42)

### Changed

- Arrange the screens around the clock face (the past to the left, alarms to the right, notifications above, music and info below), without wrapping round, with transitions that sweep the way you went and handles on the edges where a swipe leads (0b74bd9, 786cf99, e3760a9)
- Wake the screen only with a firm tap or a double tap away from the edge, rather than any touch, so brushing against something no longer lights it; while a phone is connected, a double tap is the next track instead, leaving the screen off (or a glance up) if its first tap is light, while a firm one wakes the screen at once (ce53224, 56050d9, 36aa727, fff3477, 7c131db)
- Act on a long press, and buzz, as soon as it's been held for 0.6 s, rather than when the finger lifts, so there's no guessing how long to hold (65217d7)
- Change the theme from a new THEME row on the Info screen, rather than with a long press, which now plays or pauses media (0b74bd9, 6667727)
- Show the firmware's real version on the Info screen (it always said 0.1.0), and the commit it was built from (63215e2)
- Measure the heart rate in the background every 2 to 20 minutes rather than every 10, more often after movement, during sleep, and while it's well above the wearer's resting rate, and only once the wrist has kept still for a few seconds, giving up early on one that keeps moving, so desk work no longer spends minutes of LED an hour on failed measurements (4979861, 77be9c2)
- Turn the screen off after 10 s without input rather than 15 s, for a longer battery life; the music screen stays on until a long title has scrolled by once (93360e3)
- Set the screen's brightness by the time of day: full while the sun is up, dim from midnight until the wearer gets up, medium otherwise; switching LIGHT to SOFT on the Info screen turns it down a step (ec80454)
- Use less battery while the screen is on, while it shows the dim look (now pure black and white, without greys), and while the wrist keeps still or the wearer sleeps (ec80454, 9ea33f8)
- Start wake alarms gently, building up their vibration and the screen's brightness over about a minute (c1e45cb, ec80454)
- Stack the glance's hours above its minutes, in bigger digits (999279a, c1d9ba3)
- Shrink the firmware by a sixth, so updates over Bluetooth send 70 KB less (0804e1a, 241ee01, 903438d, afc34f4, d401549, e190c6d)

### Removed

- Drop the weekday from the clock face (7874b05)
- Drop sunrise and sunset from the Info screen (e2d2774)

### Fixed

- Stop the watch resetting when a heart rate reading came in while the screen was being redrawn (introduced in 0.6.1) (1bd5e6b)
- Stop the watch crashing while a phone pairs, which made updates from the nRF DFU app fail on a link that wasn't paired (d7ca4d1)
- Restart at once after a crash, instead of freezing until the watchdog resets the watch, and restart on a stack overflow rather than running on with corrupted memory (6b4ded1)
- Keep today's steps, and the activity the phone hasn't fetched yet, across updates (3f55661)
- Stop sleep tracking seeing false wakes, and the activity history sent to the phone gaining tens of thousands of phantom steps a minute, when the accelerometer's data queue overflowed about every half hour (introduced in 0.8.0) (0609657)
- Keep glances paused when the wearer stirs in their sleep (introduced in 0.8.17) (0609657)
- Use less battery while the phone is connected but idle, by no longer asking it for a busier link than it had settled on after 30 s without traffic (introduced in 0.6.1) (0e8ec77)
- Ask the phone for its location at most once an hour when reconnecting, instead of running its GPS for two minutes at every reconnect while none arrives (6f7891d)
- Stop reporting a falsely slow heart rate when the sensor shifts on the skin after the wrist moves (1651aa5)
- Keep a ringing alarm or timer buzzing when a notification arrives, instead of silencing it (introduced in 0.7.2) (c1e45cb)
- Keep the screen on when an alarm or timer starts ringing during a glance (introduced in 0.7.2) (ec80454)
- Keep the screen on until a notification banner has gone, rather than turning it off under a banner that arrived late (93360e3)
- Keep a glance up when the phone reports a new track or play state, rather than waking the screen fully (introduced in 0.7.2) (b9d4477)
- Show music as playing, rather than paused, once a track changes or is skipped after the watch restarts mid-song, since the phone doesn't say again whether it plays (d5ee29a)
- Ignore touches while pairing or an update fills the screen, which could change the screen beneath unseen (0b74bd9)
- Ignore impossible touch positions, which could start phantom swipes (14fb597)
- Centre the sender of a notification that arrives with the screen off (introduced in 0.8.4) (c1adca7)

### Security

- Require a passkey-paired phone for the Gadgetbridge link, so nearby devices can't read activity history or send notifications; the watch warns when a connected phone isn't paired or has lost its pairing, and one that connected without pairing (as Gadgetbridge's Bangle.js support does by default) must pair once, typing the passkey the watch shows (d7ca4d1, 815509e)

## [0.8.17] - 2026-11-07

### Added

- Pause wrist-turn glances while the wearer is asleep (ed8faeb)

## [0.8.16] - 2026-11-07

### Fixed

- Wrap long notification senders over the whole screen, at a smaller size if need be, rather than scrolling them (66cc97e)

## [0.8.15] - 2026-11-07

### Changed

- Wait for the wrist to be still before measuring the heart rate in the background, so more measurements give a reading (119494e)

## [0.8.14] - 2026-11-07

### Fixed

- Blank the panel while it sleeps, so a glance can't briefly show parts of the clock face (1d407e1)

## [0.8.13] - 2026-11-07

### Added

- Record raw heart-sensor samples in the logging build's field log (92d488a)

## [0.8.12] - 2026-11-07

### Added

- Show charging unmistakably, with a red bolt and a filling battery icon (0f6eeee)

## [0.8.10] - 2026-11-07

### Added

- Log where the watch was stuck after the watchdog resets it (db1804f)

### Changed

- Pause the field log's flash writes during a firmware update (db1804f)

## [0.8.7] - 2026-11-07

### Fixed

- Accept more heart-rate readings from real wrists, by loosening the signal's steadiness and periodicity limits (39958b9)

## [0.8.6] - 2026-11-07

### Fixed

- Measure the heart rate on the wrist: the motion check no longer discards almost every measurement (bd9eba6)
- End a heart-rate measurement at once when the watch isn't worn, rather than raising the LED current for 30 s (bd9eba6)

## [0.8.4] - 2026-11-07

### Added

- Show a notification that arrives with the screen off on the whole screen, dim and white on black like a glance (b661680)
- Use red in the handheld theme for the heart, low battery, and update progress (b661680)

### Fixed

- Keep a glance on screen for its full 5 seconds, not under one (b661680)

## [0.8.2] - 2026-11-07

### Changed

- Recognise the wrist turning to look in ur's own code, since the accelerometer's built-in gesture never fired on a wrist (f84295e)

### Fixed

- Let a third device pair by dropping the bond of one that isn't connected, rather than failing (4af2f02)

## [0.8.0] - 2026-11-07

### Added

- Add a second one-off alarm, side by side with the first (fbe1cd6)
- Add a second timer, and count timers down to the second (2aea7dd)

### Fixed

- Read the accelerometer's movement data correctly, which sleep tracking and the heart-rate motion check depend on (2193842)

## [0.7.2] - 2026-11-07

### Added

- Add smart wake, which rings at the first stir in the 30 minutes before the alarm time, plus a one-off alarm and a timer (4e95b20)
- Glance at the time by turning the wrist while the screen is off (6f9d69c)

### Fixed

- Advertise often enough after a disconnect for Android to reconnect within seconds (introduced in 0.6.1) (8288e3f)

## [0.6.1] - 2026-11-07

### Added

- Log an hourly power summary in the field log (e9dba03)

### Changed

- Run from the DC/DC regulator, roughly halving current draw while the CPU or radio is active (ab5b207)
- Save power by asking for a slower connection when idle, advertising less while the phone is connected, and backing off failed heart-rate measurements (e9dba03)

### Fixed

- Stop lighting the screen every time the phone disconnects (ab5b207)

## [0.5.1] - 2026-11-07

### Added

- Track sleep from wrist movement and sync it to Gadgetbridge (e2c3cc9)

## [0.4.3] - 2026-11-07

### Fixed

- Fix firmware updates failing validation on the logging build, when a log write put the SPI flash to sleep mid-update (introduced in 0.4.0) (bb4d0e1)

## 0.4.1 - 2026-11-07

Built from changes that weren't committed until 0.4.3, so it has no tag.

### Added

- Log the wall-clock time to the field log every 10 minutes (bb4d0e1)

## [0.4.0] - 2026-11-07

The first build for a sealed PineTime, and so the first release. Until
that morning, ur only ran on a dev kit: it was flashed straight onto
the chip over a debug probe, and couldn't run under the PineTime's
bootloader, which is the only way onto a sealed watch. The version
numbers before this one (0.1 to 0.3) were test images for that dev kit,
never packaged for anyone to install. This release gathers everything
built up to that point, below.

### Added

- Show a pixel-art clock face with the time, date, battery, steps, and heart rate, plus an Info screen (afbadc4)
- Switch between dark and light themes at sunset and sunrise, with two of each to choose from (c0de571)
- Pair with a phone by passkey, and keep bonds and settings across restarts (1278c23)
- Sync time, steps, and heart rate with Gadgetbridge, which sees the watch as a Bangle.js (f6788a9)
- Show notifications, calls, and music from the phone (659913c)
- Count steps with the accelerometer (d387718)
- Measure the heart rate with the optical sensor (6141e57)
- Install under the PineTime's stock MCUBoot bootloader, and receive firmware updates over Bluetooth (fdc5a70)
- Keep a field log in flash in the logging build, and fetch it over Bluetooth (57f261f)

[Unreleased]: https://github.com/jonhoo/ur/compare/v0.10.0...HEAD
[0.10.0]: https://github.com/jonhoo/ur/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/jonhoo/ur/compare/v0.8.17...v0.9.0
[0.8.17]: https://github.com/jonhoo/ur/compare/v0.8.16...v0.8.17
[0.8.16]: https://github.com/jonhoo/ur/compare/v0.8.15...v0.8.16
[0.8.15]: https://github.com/jonhoo/ur/compare/v0.8.14...v0.8.15
[0.8.14]: https://github.com/jonhoo/ur/compare/v0.8.13...v0.8.14
[0.8.13]: https://github.com/jonhoo/ur/compare/v0.8.12...v0.8.13
[0.8.12]: https://github.com/jonhoo/ur/compare/v0.8.10...v0.8.12
[0.8.10]: https://github.com/jonhoo/ur/compare/v0.8.7...v0.8.10
[0.8.7]: https://github.com/jonhoo/ur/compare/v0.8.6...v0.8.7
[0.8.6]: https://github.com/jonhoo/ur/compare/v0.8.4...v0.8.6
[0.8.4]: https://github.com/jonhoo/ur/compare/v0.8.2...v0.8.4
[0.8.2]: https://github.com/jonhoo/ur/compare/v0.8.0...v0.8.2
[0.8.0]: https://github.com/jonhoo/ur/compare/v0.7.2...v0.8.0
[0.7.2]: https://github.com/jonhoo/ur/compare/v0.6.1...v0.7.2
[0.6.1]: https://github.com/jonhoo/ur/compare/v0.5.1...v0.6.1
[0.5.1]: https://github.com/jonhoo/ur/compare/v0.4.3...v0.5.1
[0.4.3]: https://github.com/jonhoo/ur/compare/v0.4.0...v0.4.3
[0.4.0]: https://github.com/jonhoo/ur/releases/tag/v0.4.0
