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

### Changed

- Redesign the clock face's lower half: the date with small seconds under the time, the heart rate and the next alarm or timer in one row, and the day's steps as ten blocks of 1,000 towards a fixed goal of 10,000 rather than a number (the exact count is on Today, a swipe right) (a02d355)

### Deprecated

### Removed

### Fixed

### Security

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
