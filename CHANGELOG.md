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

- Add a button grid for Home Assistant or Tasker, the first screen below the clock once turned on in Settings, of retro arcade buttons in one of two layouts (Settings → GRID): six buttons (the default, square, two rows of three in the middle of the screen), each tapped (sent after 0.3 s without a second tap), double-tapped, or held, with a long press too once held for 1.5 s, and released after a hold (for hold-to-dim), which reach Home Assistant as its own BTHome button events, `button_1` to `button_6`, a trigger each in the automation editor, with the battery in an advertisement of its own at each 5% step; or two layers of nine buttons, tapped alone or in pairs for codes 1–99 (2, then 3, is 23; none ends in 0; a lone tap sent after 0.6 s without a second), which reach Home Assistant as a button event for the gesture plus a dimmer turned by the code (right on `dimmer_1`, `dimmer_2` and `dimmer_3` for a tap, double tap and hold; a long press turns `dimmer_3` left, `rotate_left`, so a trigger must match the direction; a release turns none) (FAQ B04, which the watch points to as "*B04", explains why); sent over Bluetooth as encrypted BTHome, or through Gadgetbridge as an Android intent with the code and the gesture's name (see docs/buttons.md); with Gadgetbridge's "Allow Intents" off, say "INTENTS OFF" / "SEE FAQ C04"; and a press that can't go to Home Assistant (both of the watch's Bluetooth connections in use, usually the phone and a computer) turns red, with "BTHOME NOT SENT" / "SEE FAQ B05" (3f335fe, ff82ac9, e754fa6, 02160d9, 24d1126, 28f90ae, eb68544, c470936, 77bc5d6, 3d06fc2)
- Add a Settings screen right of the clock, between the alarms and Info: THEME, LIGHT, BUTTONS (where the button grid's presses go: BTHOME, VIA APP for Gadgetbridge, or BOTH), GRID (its layout), and, while BUTTONS includes BTHome, BTHOME KEY, the button grid's key for Home Assistant, as a QR code to scan and paste, or as text, either on screen until tapped away (3f335fe, b6cb498, 7c69c61, 02160d9, 5386e38, 3cae84f, 5d1fc55)
- Add a Home Assistant blueprint for each button grid layout, imported with one click (see docs/buttons.md), to set every button's actions in one automation from the editor rather than one automation each: for six buttons, an action per button for a tap, double tap, hold, long press, and release; for two layers, the same for each button on its own (bar the double tap), eight slots for any code and gesture (23 double-tapped, say), and one action for every press, given its `code` and `gesture` (f4a051f)
- Read a nearby Aranet4 air monitor's advertisements (with its Smart Home integrations on), listening briefly every 15 minutes while one is around and hourly otherwise, and show the strongest one's CO₂ (with its band), temperature, humidity, pressure, battery and age on a new Air screen below Music, which is there while one was heard within the hour (e1711da)
- Read nearby BTHome thermometers too (unencrypted, such as a Xiaomi LYWSD03MMC on pvvx's firmware or a Shelly BLU H&T) as room sensors alongside Aranet4s: the nearest of either gives the clock face's temperature, and the Air screen shows its temperature big, with its humidity, pressure and battery if it sends them; a scan listens up to 6 s while one was heard last time, rather than 3 s, and every eighth scan listens to its end, to find sensors beside those it knows (d71ecbb, 79bce0e)
- Stop looking for room sensors every 15 minutes as soon as the phone's location (when it sends one) is over 200 m from where they were heard, rather than after an hour without them, and look again at once on coming back; only locations the phone says are good to 75 m count (a coarse one keeps the phone looking, for up to the usual 2 minutes), and the watch asks for one when the scans stop hearing the sensors (at most hourly, and four times a day), and when it first hears them after starting (db26de7, 6615c88, 6112ed8)
- Ignore room sensors heard weaker than −80 dBm, so a neighbour's thermometer through a wall doesn't become the room's temperature (5151105)
- Show the heart rate and steps live in Gadgetbridge's Live activity chart (Charts, "Live activity"): while it's open, the watch measures the heart rate continuously and sends a reading and the steps since the last one every 10 s (or as often as Gadgetbridge asks), until the chart closes or the phone disconnects; off the wrist it doesn't measure until the chart is reopened (a8c4c5f)
- Answer Gadgetbridge's "Find lost device" with a screen of its own, "FIND MY WATCH" over a big HERE with rings spreading round it, and the motor's loudest buzz, rather than a call's buzz alone, until the button or a tap (whoever touched it found it), the phone's "Found it!", or two minutes; in the quiet night too, in the dim look, and after a call or an alarm that's ringing (f86fd26)
- Say "TEXT AS BITMAPS" / "SEE FAQ N02" (once a connection) when Gadgetbridge's "Text as Bitmaps" setting is on, and show each word it sends as a picture as a `?` rather than a run of junk; the README's FAQ N02 says how to switch it off (46d43f3)

### Changed

- Play or pause media with a double tap, and skip with a tap, then a tap held until the buzz (0.6 s; within half a second of the tap), the same with the screen off, on a glance, or on the clock face, rather than with a long press and a double tap (the other way round on the clock face), so an arm folded over the watch, resting against it, no longer plays or pauses: a long press on its own now does nothing. With the screen off or on a glance, nor does a touch near the edge (on a glance, a firm one still wakes the screen), and the hold must be in the middle of the screen; a firm first tap still wakes the screen at once, and the second touch then does the same on the clock face. On the awake clock face, both touches anywhere, bar the hold in the 3 s after a touch woke the screen (which the arm can do too), when it must be in the middle (introduced in 0.9.0) (1c1460d, 41c9753, 2e774fa, 9abc714)
- Move Info to the right of the clock, after the alarms, from the end of the screens below it (5f5acf2)
- Show the room's temperature on the clock face rather than the watch's own: a nearby room sensor's, to the tenth (`21.4°`), or else, to the degree (`21°`), on the wrist, an estimate from the watch's own that calibrates itself against any room sensor it meets at a room's temperature, 10–35 °C (roughly ±2–3 °C once calibrated), or the watch's own off the wrist (e1711da, 2e1f1e1, 13c30c8, 216a6f9)
- Redesign the clock face's lower half: the date with small seconds under the time, the heart rate and the next alarm or timer in one row, and the day's steps as ten blocks rather than a number: 1,000 a block (×1K) towards a fixed goal of 10,000, then 10,000 a block past it, its scale (×10K) in the accent colour to mark the goal reached, full at 100,000, the blocks still to fill faded even in the dim look (the exact count is on Today, a swipe right) (a02d355, 718c01d, 6147206, ca22cc1)
- Show a faded Bluetooth rune struck through in red on the clock face while the phone isn't connected, rather than a Bluetooth rune while it is; the note for music playing moves to the top left corner in its place (2e1f1e1, a1c8505)
- Move THEME and LIGHT from Info to the new Settings screen, leaving Info to the watch's status (3f335fe)
- Redesign notifications: with the screen off, and in the history, set who it's from big and centred if it fits on one line or two of about equal width, as before, and otherwise centred in a new bold medium face over as few balanced lines as it takes, rather than small; show both as one card, the app top left over a rule, and in the history when it arrived (as `SUN 20:13`) top right rather than at the bottom; and let the banner take a second line for a long name, ending in an ellipsis, rather than scrolling (86ebb6a, 5f98db4)
- Read the heart rate at a desk more often: a second of hand or finger movement in the 8 s the watch measures over is now left out, and the rest used, rather than the whole 8 s thrown away, and the first conversion after the sensor lights, which comes out darker, no longer drowns the first seconds' pulse; replaying field logs, desk sessions that ended in a reading went from 4 to 18 of 131 (within 5 BPM of a Garmin 9 times in 10), and night readings come sooner, at the same accuracy. A heart rate under 50 now also needs a still wrist unless you're asleep, so fidgeting at a desk isn't read as a slow pulse (676656c, 5abc884, 0b1c769)
- Read a faint pulse on a still wrist, such as lying down awake with cold hands: there, two 8 s stretches in a row that agree within 3 BPM now make a reading, even if neither is clear enough on its own, so most readings also come ~2 s sooner (a night's LED time falls ~15%); and if a still wrist gives no reading, try again within 10 minutes, for up to an hour, rather than backing off to 20, 40 and 60. Replaying field logs against a Garmin, readings went from 106 to 118 of 252 sessions, as accurate as before, and an hour lying still that gave none now gives one (4aefe15, 54c553d)
- Keep measuring the heart rate through movement: a background measurement waits for a still moment to start for two minutes at most, or not at all once there's been no reading for half an hour, rather than however long it takes (busy desk work held one back 18 minutes), and then runs its full 30 s however the wrist moves (unless it reads first, or finds no skin), rather than giving up after 12 s on a busy wrist, as a calm moment may come within it; the signal checks still keep a moving wrist from reading wrong, and a field log now holds whole measurements under movement, the data for reading through it one day. Replaying field logs, desk background measurements read 29 times in 91 rather than 27, for ~20% more LED, and the hourly LED budget rises from 180 to 300 s to make room, at most ~5.6 mAh a day rather than ~3.3; a waking day that's never still for long and never reads, which used to start no measurement at all, now spends up to ~2–5 mAh (bd66bc4, f4bbe32)
- Try a brighter heart-rate light, an experiment whose field logs will say whether more light helps, at a desk above all: awake, every other background measurement, together with its retries until one reads, now starts at 30 mA rather than 12.5 (or 20 mA if 30 would overwhelm the sensor), so a day's log compares the two on the same wrist, with first tries and retries alike on each; and one that gives no reading in its usual 30 s, on a calm wrist with only a faint pulse, now goes on for up to 15 s more at a brighter setting (and once more at 40 mA if that fails too). The hourly LED budget now counts a brighter second for the current it draws, and a brighter try is only made when the budget can spare it, so ordinary readings aren't held back (633489f, 6a47d2e, 28900de)
- Try harder for a heart rate once there's been none for half an hour, asleep or awake: until the next reading, a failed measurement is retried within 10 minutes rather than backing off to 20 minutes or an hour (unless the watch is off the wrist), and the hourly LED budget doubles, so more and brighter tries fit; at worst, a whole day without readings, that's ~5.6 mAh a day more. A still wrist that gives no reading now also keeps retrying every 10 minutes past its first hour (12e5c93, 090c5a3)
- Tell sleep stages apart better: deep sleep now also needs a heart rate that holds steady from one reading to the next, and REM a smaller rise (4 BPM over the night's baseline rather than 5) along with the odd twitch of the wrist; against a Garmin worn alongside, a night that came out with too much deep and too little REM went from 48% to 61% of minutes in agreement, and an earlier one from 67% to 70% (b526bbd)
- Show labels, titles, the seconds and other secondary marks in red in the dim look (glances, notifications with the screen off, and the quiet night), rather than in the same white as the text; on Last night, deep sleep is white, REM red, and light sleep grey (30fb455, ef1d58c)
- Leave a column more margin either side of notification text, as for a call's (0eaa4c1)

### Deprecated

### Removed

### Fixed

- Restart and charge when plugged in during the "battery empty" switch-off, rather than staying off until the battery runs flat again, possibly for months (3baa4ec)
- Get every step into Gadgetbridge, and so into Health Connect, which kept only about 40% of them (a day's 10,346 arrived as 4,206): Gadgetbridge merges samples less than 2 minutes apart, keeping the last one's steps, and the watch sent one a minute; it now sends one per 5 minutes, with all their steps, the last heart rate reading (earlier ones in the same 5 minutes are dropped), and the sleep stage most of them had, so Gadgetbridge lags the watch by up to 5 minutes, and sleep there is to the nearest 5 minutes. Steps also land in their own minutes in Health Connect, rather than a minute early. Updating from 0.9.x counts no step twice in Gadgetbridge (the first fetch after updating to 0.10.0-rc.4 did, over up to 2¾ hours), at the cost of up to 5 minutes' steps once; and each fetch sends only what Gadgetbridge doesn't have yet, rather than the last 2¾ hours again (13b318e)
- Gadgetbridge gets the night's sleep in the morning, once its stages are settled: about 2¼ hours after you last slept, the first fetch gets the whole night at once, steps included, and passes it on to Health Connect straight away; until then, fetches stop at when you fell asleep (a nap too, and the last 20-odd stepless minutes of sitting still, in case it's sleep; never more than 18 hours). Gadgetbridge keeps the first sleep stage it gets for a sample, so a fetch during the night, or soon after waking, fixed stages the watch was still revising in Gadgetbridge and Health Connect for good (6ce2535)
- Get notifications, music and alarms from the phone onto the watch within about a second once the connection has been quiet for 30 s, rather than a few seconds a line (a long notification took up to 12 s, a burst of them longer): the watch now asks for Bluetooth's long packets, and while the phone is sending, listens at every turn rather than every eighth (4b3ed08, b4575d7)
- Reconnect Gadgetbridge, once, when it connects but never subscribes to what the watch sends (it then shows no battery and firmware "N/A", and gets nothing from the watch), as it can when it asks before the first pairing has finished; it reconnects at once and subscribes properly, and the README's new FAQ C05 covers the rare case that doesn't fix (ff3b3b2)
- Show the battery level without overstating it while charging, so it no longer drops by ~10 points minutes after the watch comes off the charger: the percentage now follows a curve of the PineTime cell's voltage against its charge, checked against the field logs (the linear one before overstated the middle of the range by 10–20 points, so it reads lower there now, and red under 20% now means under 20%), with 0% where the watch switches itself off; on the charger it only climbs, no faster than the charger can fill the cell, to 99% until the charger says it's done; and off it, it never climbs, and falls at most a point every 3 minutes, only once the reading has stayed lower, so a load or the cell settling after a charge doesn't show as a jump. The same number goes to Gadgetbridge and Home Assistant (7f78fe8)
- Report the battery to Gadgetbridge every half hour even when it hasn't changed, so one report missed no longer leaves it stale until the next change (ff3b3b2)
- Remove a stored setting too short for what it's stored under (left by another build), rather than warning about it in the log at every boot (93fa3e2)
- Stop the watch sometimes resetting as it saved something to its flash (a setting, the alarms, a pairing, the temperature calibration, the button grid's counter, or confirming an update), or, more rarely, never finishing the save, which kept every later save from happening until a restart (caead12)
- Turn the screen off after a notification that lit it in the dark, even if a second one arrived just as the quiet night began while the alarm editor was left open: the screen stayed on, dimly, until the next touch or press, all night if need be; the screen going off now closes the alarm editor (keeping what was edited, as it's saved as it changes), so the next wake is to the clock face, where the button turns the screen off again, rather than back into the editor (323af29)
- Recover the step counter and heart rate when the sensors' connection is stuck at start-up, as a restart mid-reading (such as an update's) can leave it, rather than counting no steps and measuring no heart rate until the next restart: the sensors now keep trying to start, ever less often, and the connection is freed whenever a reading fails, not only at start-up (21d3cc1)
- Switch the heart-rate light off even when the first try fails, retrying every second, rather than leaving it lit (12.5–40 mA) until the next measurement, up to an hour away, or for as long as the clock face showed; and keep counting steps after a garbled reading of the step counter, which could add billions of steps to the day and then count none until a restart (neither yet seen on a watch) (7f3bb9c, 3f913b9)
- Keep timers and the snooze counting from when they were set, whatever the phone does to the clock: a new time zone shifted them by the difference (a 10-minute timer took 3 h 10 min after flying three zones west), and the clock set far back made opening a running timer reset the watch; a timer now also runs before the phone has set the time (34f5662, 9c65743)
- Count today's steps from midnight in the new time zone after a flight across midnight, as phones do, so the clock face and Today agree: flying west, the evening after landing counted from 0 as a new day (the clock face said 5 steps while Today said 1,205), then started over again at the new zone's midnight; flying east, the count started over at the change rather than at the new zone's midnight. Steps taken before the phone first set the clock that day are dropped by such a change, and a new time zone on the same date leaves the count alone (3438411)
- Always end a button grid hold with its release (Home Assistant's `hold_release`), at the lift: a hold lifted just as it became one, at 0.6 s, sent none (and a stray long press at the next touch), and in the quiet night a hold kept on past the screen's 10 s timeout lost it as the screen went off. The screen now stays on while a finger holds, quiet night or not, with the held button shown pressed, and goes off 10 s after the lift; a hold past a minute is taken for bedding or a sleeve resting on the screen, which then goes off as if lifted (in the quiet night, sending the release then) (6c5db54, 6fa7f9f, 11624d7)
- Make the Health Connect export from Gadgetbridge ("Sync after device sync") run again after later syncs, rather than only once when turned on, catching up on everything since each time: Gadgetbridge's Bangle.js driver never says a sync has finished, so the watch ends the first sync after it starts, and then one at most every 2 hours, with an empty list of recorded tracks, which does (Gadgetbridge shows a short "End : Fetching sports details" for it) (f733856)
- Keep the day's activity, sleep and Gadgetbridge's samples straight when the phone puts its clock back (it was wrong, say): what was recorded under the clock that was ahead is dropped, rather than the minutes recorded from then on being added on top of it (or, over a day back, filed under days still to come, and sent to Gadgetbridge as such), with sleep tracking stuck until the clock caught up; and if Gadgetbridge last fetched while its clock was ahead, it now gets everything rather than nothing recorded since, as does the record the watch saves on switching off (81d36d7, 55072bd, b8ecd08)
- Keep an alarm ringing through a call: the call ending (answered, declined, or over), or the phone's "find my watch" stopping, left the alarm on screen but silent; and stop a notification arriving during a call from cutting its buzz short (3451cca)
- Ring each alarm once the nights summer time starts and ends: one in the skipped hour (02:30, say) now rings as the clock jumps to 03:00 rather than not at all, and one in the repeated hour rings in the first 02:00 only, rather than in both; beyond an hour, a new time zone still skips what it jumps over, and rings again what comes round again (2c6e399)
- Keep a one-off alarm from the phone off once it has rung at the same minute as one of the watch's own: it was saved as still on, so after a restart before Gadgetbridge next sent its alarms, it rang again the next morning (fb51f17)
- Keep showing the wrist's estimate of the room's temperature while you sleep: a quarter of an hour without moving counted as the watch lying on a table, even with heart rate readings coming in, so for much of a still night the clock face showed the watch's own temperature (30 °C or so, warmed by the skin), and after waking calibration waited another 20 minutes; a heart rate reading since the watch went still now keeps it on the wrist, and stillness alone only means off the wrist with none (373457b)
- Leave no gap between Bluetooth and the temperature once media is paused or stopped (introduced in 0.9.0) (fcd0a6d)
- Keep notification names (the sender, or without one the title) of up to 40 characters whole, however many æ, ø and å they have (20 of them filled it), and end one that's cut with an ellipsis rather than passing it off as whole; leave emoji out, rather than showing each as `?` (and show the title for a sender that was only emoji), and draw Ø so it doesn't read as an 8 (86ebb6a, 00024b0)
- Show `&`, `#`, `@`, `"`, `*` and the rest of ASCII's symbols (and `×`) in notifications, and curly quotes, dashes and ellipses as their plain look-alikes, instead of `?` (929a956)
- Make releases rebuild byte for byte whether or not the builder already has the dependencies downloaded; 0.9.0 only reproduces from an empty cargo home (introduced in 0.9.0) (e49cb26)
- Draw deep, REM, and light sleep on Last night in three distinct colours in every theme; deep and REM were the same red in Arcade and Parchment (introduced in 0.9.0) (a1f642d)
- Show the Air screen's pressure from any nearby BTHome sensor without crashing a debug build or reading "0 HPA" when the sensor reports an absurdly high value (79c034e)

### Security

- Check a firmware update before installing it, and refuse one that's damaged, cut short, or not firmware for this watch, saying "UPDATE REFUSED" / "SEE FAQ U02" and keeping the running firmware; also give up on an update whose sender goes quiet for a minute, and take one update at a time (c569ee4)
- Keep pairings from outside meddling with the phone's bond: a pairing without the passkey no longer leaves a bond (one claiming the phone's address replaced the phone's), a failed one no longer drops the phone's bond until the next reboot, and after each failed pairing the next waits longer, up to a minute, so a passkey can't be guessed at speed or the screen kept lit (1700f15)
- Stop a malformed alarm list from the phone hanging the watch until the watchdog reset it, and ignore a time set before 2020 or after 2100 (e24a3bb)

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

[Unreleased]: https://github.com/jonhoo/ur/compare/v0.9.0...HEAD
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
