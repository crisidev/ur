# ur-sim

The watch's UI on your computer: the firmware's own screens, navigation,
alarms, themes and phone handling (`src/ui.rs` and what it uses, compiled
for the host), in a window you click, or run from a script that takes
screenshots. What a [dev kit](../CONTRIBUTING.md#a-dev-kit) does for the
UI, without one, and faster: a script's night passes in a second.

```sh
cd sim
cargo run --release                          # the window
cargo run --release -- tour.txt --out shots  # a script, headless
cargo run --release -- --window tour.txt     # a script, then the window
```

It builds for whatever computer you're on (`.cargo/config.toml` sets the
`host-tuple` target, overriding the firmware's). Without a display (CI),
build with `--no-default-features` to leave the window out.

## The window

The watch, drawn to scale from PINE64's dimensions and photos: the 1.3″
screen in a black 37.5 × 40 mm case, and the button on its left edge. The
mouse is a finger: click to tap, hold still for 0.6 s to long press, drag
to swipe, all through the firmware's gesture recogniser
(`src/gesture.rs`). Click the button to press it. The screen dims with
the backlight and goes black when it's off, and the watch shakes while
the motor buzzes. `--zoom N` sets its size: N pixels a canvas pixel (4,
a 320-pixel screen, by default). Keys:

| Key | | Key | |
|---|---|---|---|
| Space, Enter, B | the button | G | a glance (a turn of the wrist) |
| N | a notification | M | music: play/pause |
| C | a call; again to end it | arrows | swipes |
| T, L | tap, long press | F, H | skip a minute, an hour |
| S | a screenshot | R | record a GIF; again to stop |
| Q, Esc | quit | | |

Everything else (the time, the phone, sensors) is a command typed into
the terminal, as in a script. The watch starts as after a reset: no time,
no phone. `time now` and `connect` get it going.

## Scripts

One command a line; `#` starts a comment. `help` lists them all. Inputs
use the dev kit's coordinates (`scripts/screenshot.py`): canvas pixels,
0–79.

```text
time 2026-11-09 07:29:50 +01:00   # the phone sets the time
connect
alarm 1 07:30 on
until 07:30                       # skip ahead
expect buzzing                    # fail the run if not
shot ringing.png
tap                               # snooze
```

- **Inputs:** `button` (`button 2`: a double press), `glance`, `tap`,
  `long`, `up`/`down`/`left`/`right` (at `X,Y`), or a finger: `press X,Y`
  (again to move it), `lift`.
- **The phone:** `gb` (see below), `connect`, `time`, `location`,
  `notify`, `call`, `music`, `find`, `phone-alarms`, `nav`, `unpaired`,
  `passkey`, `update`.
- **The wearer and the watch:** `battery` (and charging), `steps`,
  `heart`, `temperature`, `asleep`, `movement`, `last-night`,
  `demo-day` (a day of steps and heart rates so far), `alarm`, `smart`,
  `timer`, `theme`.
- **Time:** `wait 1.5s`, `until HH:MM`.
- **Output:** `shot FILE.png [THEME]` (with a theme: in its colours, as
  `screenshot.py --theme`; with `--watch`, the whole watch, as the window
  draws it), `record FILE.gif` … `stop` (see below), `expect`.

`gb` takes a line exactly as Gadgetbridge sends it (less the `\x10`
before it), say from Gadgetbridge's log, and runs it through the
firmware's own parser (`src/gb.rs`), then acts on it as `src/ble.rs`
would; the other phone commands hand the UI its events directly. The
rest of the line is taken as it is (no quoting, no `#` comments), and
must be ISO-8859-1, as on the wire: escape anything else as
Gadgetbridge does (`\u20ac`). The first `gb` connects the phone, as
its first write does on the watch. `gadgetbridge.txt` replays one of
each kind.

```text
gb setTime(1794150000);E.setTimeZone(1.0);
gb GB({"t":"notify","id":1,"src":"Signal","title":"\xc5se","body":"Hi"})
```

Screenshots are the 240×240 the panel would show: the canvas's pixels
as pushed to the panel, in the same colours `screenshot.py` gives, so the
two match pixel for pixel. `readme-screenshots.txt` makes the README's;
`tour.txt` goes round most screens, checking itself as it goes; `cargo
test` runs it, and `gadgetbridge.txt`, and checks every screenshot they
take.

## Approved screenshots

A script draws the same pixels on every run, on every computer, so its
screenshots are committed, as approved, and `cargo test` checks that
each script still draws exactly those: `tour.txt`'s in `shots/tour/`,
`gadgetbridge.txt`'s in `shots/gadgetbridge/`, and
`readme-screenshots.txt`'s in `../docs/screenshots/`, the README's own,
so it can't go stale. A test fails on a screenshot that differs (with
how many pixels, and an image of it before, after, and where it differs,
in magenta, side by side), on a new one, and on one no longer taken.

When the change is what you meant, look at those images, then approve
what the scripts now draw, and commit it with the change:

```sh
UPDATE_SHOTS=1 cargo test
```

That makes each approved directory exactly what its script drew: it
adds new screenshots, rewrites changed ones, and deletes those no longer
taken. The Info screen shows the version, from Cargo.toml (`0.9.0-dev`
here), so a release's new version needs approving too. The README's
`info.png` is the exception: it shows a release's version and commit
(`UR_VERSION`; see `readme-screenshots.txt`), which the tests' build
never does, so they neither check nor update it, and the tour's
`11a-info.png` checks that screen instead. Only the PNGs there are
`readme-screenshots.txt`'s: the GIFs beside them are
`readme-gifs.txt`'s, which approving leaves alone (`tests/record.rs`
checks they're made, and small; a GIF's frames make a poor diff).

The comparison is `ur-sim --diff BEFORE AFTER --out DIR`, which
compares every PNG under two directories, and writes each that changed
(side by side, as above) to `DIR/changed/`, each only `AFTER` has to
`DIR/added/`, and each only `BEFORE` has to `DIR/removed/`, with a list,
`DIR/report.tsv`.

## Recordings

`record FILE.gif` starts an animated GIF of the screen, and `stop`
writes it; R in the window does both, to `ur-sim-DATE-TIME.gif` in
`--out`. Its frames are what the panel got, as screenshots are:
transitions, the dim look, and the screen going dark (black, with the
backlight off) appear as on the watch. `--watch` records the whole
watch instead, as `shot --watch` draws it.

```text
record swipe.gif --still 1s
right
wait 1.5s
stop
```

The timing is the watch's clock's, so it's exact, and the same every
run (and in the window, the watch's pace whatever `--speed`). Two rules
turn it into a GIF:

- **A still screen is held at most 2 s** (`--still DURATION` to change
  it): a `wait 7h` with the screen off is a moment of black, not seven
  hours. Anything that changes faster plays at its own pace.
- **Frames land on a 20 ms grid**, as if filmed at 50 frames a second:
  browsers play GIF delays under 20 ms as 100 ms. A frame replaced
  within 20 ms isn't seen, and is left out (about every other step of a
  transition). Each frame's start, not its length, is rounded, so a
  frame is never more than 20 ms late, however long the recording.

The last frame holds until `stop` (at least 20 ms): `wait` before it to
linger. A recording never stopped is written when the simulator exits.
Each frame holds only the rectangle that changed, so a clock's tick
costs a few hundred bytes. The screen's colours fit one exact palette;
the whole watch has more than a GIF's 256 (its anti-aliased edges, the
backlight's levels), so the commonest 255 stay exact and the rarest (all
thin edges) become the nearest of those. Unlike `shot`, `record` takes
no theme: pick one with `theme NAME` first.

`readme-gifs.txt` makes the README's GIFs, in one command (see its top);
rerun it when a screen they show changes. `cargo test` runs it too, and
fails if one grows past 100 KB.

## Against a dev kit

The simulator draws what the watch would, and this checks it, frame for
frame. With a dev kit running a build with the `compare` feature, it
follows the dev kit: it runs a script's inputs on it, replays what its UI
got and when (every tick and input, at the instants it got them), and
for each `compare NAME` draws the frame the dev kit last finished, with
the readings it had (the time, the battery, the sensors, today's
activity, the settings in its flash). Every pixel must match:

```sh
cargo run --release --features compare            # at the top: flash a dev kit
cd sim
cargo run --release -- --devkit ../target/thumbv7em-none-eabihf/release/ur \
    --out ../tmp/compare devkit-compare.txt
```

It resets the dev kit first, so both start from boot. Each `compare`
writes `NAME.png`: the simulator's frame, the dev kit's, and where they
differ (magenta), side by side; the run fails if any differ. The script
can only do what a dev kit can be made to do (`scripts/screenshot.py`'s
inputs: `button`, `glance`, gestures, `press`/`lift`, `notify N [HOURS]`,
`media`, `steps`, `battery`), plus `wait`, `compare`, and `expect` (of
`screen-on`, `dim`, `quiet-night`). With the screen
off, the dev kit's panel keeps an old frame, and `compare` skips it. A
phone connected to the dev kit would send messages the simulator can't
know, and stops the run. `devkit-compare.txt` goes round every screen it
can reach (and puts back what it changes).

## How it works

`src/main.rs` includes the firmware's hardware-free modules with
`#[path]`, as the host tests do, at the crate root where their `crate::…`
paths expect each other. Stand-ins replace the rest:

- `display`: a fake ST7789 that collects what `Canvas::flush` and
  `Canvas::transition` push, so screenshots, recordings, and the window
  show what the panel would, transitions included. A recording
  (`src/record.rs`) looks at it each time the clock is about to move.
- `activity`: steps, heart rates and sleep, as told.
- `ble`: the queue of messages for the phone, which are printed.
- `motor` is the real one; the simulator plays its requests on the bezel.

`watch.rs` is `main.rs`'s UI loop again (ticks, inputs, frames, screen
power), on a clock of its own (at the watch's 32 768 Hz), which only moves
when told: scripts are deterministic, and skip hours at no cost. **It mirrors `main.rs` by hand**:
a change to the UI loop there may need one here.

## What it can't simulate

- **Sensors.** Steps, movement, heart rate, sleep and temperature are
  whatever you set: the accelerometer and heart rate processing, the
  sleep classifier, and the wrist-turn detector don't run (`asleep on`
  says the classifier decided; `glance` that the wrist turned).
- **Bluetooth.** Only the phone's messages: `gb` parses its lines as the
  watch does, but no radio, GATT, or MTU-sized pieces carry them.
  Pairing, bonds, updates, GPS requests and the activity sync don't run;
  `passkey` and `update` only show their screens.
- **Flash.** Nothing is saved between runs. After switching off, the
  watch reboots with its settings, and the clock and uptime keep running
  (on the watch, uptime starts again, and the time is lost until the
  phone sets it).
- **Timing.** Pushing pixels takes no time (on the watch, SPI does), so
  a transition takes only its frame timer's ~0.1 s. The backlight's
  switching off while redrawing into or out of the dim look isn't shown.
- **Logs.** The firmware's defmt logs go nowhere (decoding them isn't
  wired up). The simulator prints what happens instead.
- The touch controller's quirks (missed lifts, garbage reports), the
  watchdog, panics' breadcrumbs, the bootloader, and anything else below
  the UI.
