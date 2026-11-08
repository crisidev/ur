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
| S | a screenshot | Q, Esc | quit |

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
  draws it), `expect`.

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
test` runs it, and `gadgetbridge.txt`.

## How it works

`src/main.rs` includes the firmware's hardware-free modules with
`#[path]`, as the host tests do, at the crate root where their `crate::…`
paths expect each other. Stand-ins replace the rest:

- `display`: a fake ST7789 that collects what `Canvas::flush` and
  `Canvas::transition` push, so screenshots and the window show what the
  panel would, transitions included.
- `activity`: steps, heart rates and sleep, as told.
- `ble`: the queue of messages for the phone, which are printed.
- `motor` is the real one; the simulator plays its requests on the bezel.

`watch.rs` is `main.rs`'s UI loop again (ticks, inputs, frames, screen
power), on embassy's mock clock, which only moves when told: scripts are
deterministic, and skip hours at no cost. **It mirrors `main.rs` by hand**:
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
