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
cargo run --release -- --flash my.bin        # settings kept between runs
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
the motor buzzes. The title bar has the rest: the backlight's level (1–7),
the phone, the battery and charger, and the time since boot (and
`--speed`). `--zoom N` sets its size: N pixels a canvas pixel (4, a
320-pixel screen, by default).

The screen's colours are the real panel's, as near as a model can make
them (`src/panel.rs`): less saturated than a monitor's, with a cold,
bluish white, and black glowing a little with the backlight. P switches
to the colours as sent to the panel, as screenshots have them
(`--sent-colours` starts there).
Keys:

| Key | | Key | |
|---|---|---|---|
| Space, Enter, B | the button | G | a glance (a turn of the wrist) |
| N | a notification | M | music: play/pause |
| C | a call; again to end it | arrows | swipes |
| T, L | tap, long press | F, H | skip a minute, an hour |
| S | a screenshot | R | record a GIF; again to stop |
| P | the panel's colours, or as sent | Q, Esc | quit |

Everything else (the time, the phone, sensors) is a command typed into
the terminal, as in a script. The watch starts as after a reset: no time,
no phone. `time now` and `connect` get it going.

## In a browser

The same watch, compiled to WebAssembly, on a web page: the window's
watch, mouse (or touchscreen) and keys (bar quitting), time running with
the browser's clock, a button for each key (each showing its key, so
a phone can press them too), a theme picker, and a box for commands,
as typed into the terminal. Screenshots
(`S`, or `shot`) and recordings (`R`, or `record` … `stop`) download. A
script file runs from the page too. It
starts in use, not as after a reset: the time set from the browser's
clock, a phone connected, and a little of a day to look at
(`web/start.txt`).

```sh
rustup target add wasm32-unknown-unknown   # once
web/build.sh                               # into target/web
python3 -m http.server -d target/web       # then open http://localhost:8000
```

It needs a web server, as browsers won't load the module into a page
opened as a file. `web/check.sh` checks it headless (with node): it
drives it as the page does, then runs the tour, the README's
screenshots and GIFs, `gadgetbridge.txt` and `web/compare.txt` on it and
on the command line, and checks that every screenshot and recording is
the same, byte for byte.

The project's site, <https://jonhoo.github.io/ur/>, is this page under
`sim/`, beneath a landing page made from the top-level README.
`web/site.sh` builds it into `target/site`, as the Pages workflow does:
it renders the README with GitHub's markdown API (so it needs `gh`,
logged in), puts it into `web/site.html` with `web/site.py`, and copies
the images it shows (fetching them from Git LFS if they're missing).

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
  `long`, `up`/`down`/`left`/`right` (at `X,Y`), `double X,Y` (two taps
  0.1 s apart, quicker than two over a dev kit's probe), or a finger:
  `press X,Y` (again to move it), `lift`.
- **The phone:** `gb` (see below), `connect`, `time`, `location`,
  `notify`, `call`, `music`, `find`, `phone-alarms`, `nav`, `unpaired`,
  `passkey`, `update`, `slots full` (both Bluetooth connections in use,
  so the button grid's presses can't go to Home Assistant).
- **The wearer and the watch:** `battery` (and charging; the percentage
  starts over from it, as after a reboot), `steps`,
  `heart`, `temperature`, `asleep`, `movement`, `last-night`,
  `demo-day` (a day of steps and heart rates so far), `alarm`, `smart`,
  `timer`, `theme`.
- **Time:** `wait 1.5s`, `until HH:MM`.
- **Output:** `shot FILE.png [THEME]` (with a theme: in its colours, as
  `screenshot.py --theme`; with `--watch`, the whole watch, as the window
  draws it; with `--panel`, as the real panel shows it, as the window
  does), `record FILE.gif` … `stop` (see below), `expect`, and
  `settings` (what the flash holds, decoded as the firmware does).

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
two match pixel for pixel (unless `--panel`). `readme-screenshots.txt`
makes the README's; `tour.txt` goes round most screens, checking itself
as it goes; `cargo test` runs it, and `gadgetbridge.txt`, and checks
every screenshot they take.

## Approved screenshots

A script draws the same pixels on every run, on every computer, so its
screenshots are committed, as approved, and `cargo test` checks that
each script still draws exactly those: `tour.txt`'s in `shots/tour/`,
`gadgetbridge.txt`'s in `shots/gadgetbridge/`, and
`readme-screenshots.txt`'s in `../docs/screenshots/`, the README's own,
so it can't go stale, and each theme's (`../src/themes/<name>/shots.txt`,
into `shots/` beside it, for the theme's README), for the themes the
build has: the default one, unless it's built with others
(`--features themes-all` checks them all). A test fails on a screenshot that differs (with
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
here), so a release's new version needs approving too (the tour's, and
each theme's). The README's
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

On a pull request, CI draws the screenshots for its head and for where
it branched off, and comments on it with what changed (see
[CONTRIBUTING.md](../CONTRIBUTING.md#approved-screenshots)).

## Recordings

`record FILE.gif` starts an animated GIF of the screen, and `stop`
writes it; R in the window does both, to `ur-sim-DATE-TIME.gif` in
`--out`. Its frames are what the panel got, as screenshots are:
transitions, the dim look, and the screen going dark (black, with the
backlight off) appear as on the watch. `--watch` records the whole
watch instead, as `shot --watch` draws it, shaking while the motor
buzzes, as in the window.

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
fails if one grows past 130 KB.

## Against a dev kit

The simulator draws what the watch would, and this checks it, frame for
frame. With a dev kit running a build with the `compare` feature (which
implies `debug-input`, the input it injects), it follows the dev kit: it runs a script's inputs on it, replays what its UI
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
`media`, `steps`, `battery`, `find on|off`, `asleep on|off`), plus `wait`, `compare`, and `expect` (of
`screen-on`, `dim`, `quiet-night`). With the screen
off, the dev kit's panel keeps an old frame, and `compare` skips it. A
phone connected to the dev kit may play music: the dev kit exports the
artist and track it gets, so the simulator shows the same. Its other
messages the simulator can't know, and they stop the run.
`devkit-compare.txt` goes round every screen it
can reach (and puts back what it changes).

With `--peer ADDRESS` (the dev kit's Bluetooth address), this computer is
the dev kit's phone, through `scripts/gb-peer.py` (which needs `uv`, and
the computer paired with the dev kit; see `scripts/dfu.py`). Then a
script can `connect` and `disconnect`, and send `gb` lines over the air,
as Gadgetbridge would. `connect` sends this computer's time first, as
Gadgetbridge does on every connection: the watch takes a central for its
phone only once it writes. (`connect silent` doesn't, for a central that
never writes.) The simulator expects what each line makes the UI
do, and checks, at each `compare`, that the dev kit's UI got it, and that
what the dev kit sent the phone (calls answered, media commands, the
button grid's intents) is what the simulator's UI asked for. A phone of
your own must not be connected to the dev kit meanwhile: forget the dev
kit in Gadgetbridge and Android. Besides, with a dev kit:

- `bthome-temp C` advertises from this computer as a BTHome thermometer
  (`scripts/bthome-advertise.sh`), and `bthome-temp off` stops.
- `reset` restarts the dev kit, and checks it boots with the settings its
  UI had (what it saved); the simulator then follows the new boot.
- `check-storage` reads the dev kit's storage and checks it holds what
  its UI has, read as the firmware reads it at boot.
- `--storage FILE` writes FILE (the 12 KiB storage region; see below)
  over the dev kit's storage before it starts.

A tap on the storage mode prompt, which would switch the dev kit off, goes
in as a button press, which cancels it.

### Fuzzing

`--fuzz ELF` writes the scripts itself: random ones, from a seed, of
everything above (with `--peer`, Bluetooth too), each run from random
settings saved over the dev kit's own storage, and a `compare` after every
step. Runs are long (150 steps), as each starts with a reset (about 10 s,
mostly the bootloader's). A run that finds a difference is cut after the
step that showed it and shrunk, a fresh run each try: from the last reboot
before it (a `reset` step: its storage is all that came before), the last
few steps first, then twice as many, then steps taken out while it still
shows, for at most 30 tries or 20 minutes. What's left is written to
`finding-SEED/` (a script and a storage image, to run with `--devkit
--storage`, and what it found):

```sh
cargo run --release -- --fuzz ../target/thumbv7em-none-eabihf/release/ur \
    --peer E3:65:DE:6D:FE:A9 --seed 1 --runs 10 --out ../tmp/fuzz
```

The dev kit's storage is read first (`original-storage.bin`) and written
back at the end. `--dry` runs the scripts on the simulator alone, to check
the generator (`tests/fuzz.rs` does).

Besides a difference from the simulator, a run stops at:

- a restart the script didn't ask for: a panic, a fault, or the watchdog.
  Its boot log (drained over RTT once it's back) is in the run's
  `restart-log.txt`, with the previous run's breadcrumb.
- a step after which less than 512 bytes of the stack were left unused
  at its deepest, read from the stack's paint without halting the core:
  a little deeper, and the watch restarts.
- a grid hold with no release before the next press. The simulator can't
  see that one: it replays the gestures the dev kit's touch task made.

About a quarter of the runs are lone: from storage with no bonds at all,
and no Bluetooth steps, so the dev kit advertises throughout, for nobody.
Two ways to aim the runs:

- `--corpus DIR` runs the inputs of the simulator's own fuzzer (below)
  instead of seeds, smallest first: paths it found by branch coverage,
  checked against the hardware. With `--gadgetbridge`, `DIR` is the
  `gadgetbridge` target's, and each input is a session with the model of
  Gadgetbridge (`gbmock`) as the phone, through this computer's link
  (`src/gadgetbridge.rs`): a frame compared every step, as for any run,
  and every line the dev kit sends to the model, which mustn't refuse
  it. The model answers what the dev kit says, so a session can go
  differently from the simulator's own (an activity fetch's answer is
  the dev kit's history); `--out`'s scripts are what ran. Waits are cut
  to 20 seconds of real time (the model's clock moves the whole wait),
  and steps and heart rates, which only the simulator can be told, are
  comments. `--dry --gadgetbridge` runs the sessions so on the simulator
  alone.
- `--guided`, with a build from `coverage-build.sh` (a `compare` build
  that marks each function it runs), reads which functions have run after
  every step, and extends the runs that reached new ones rather than
  starting afresh. It covers everything on the watch, Bluetooth and the
  hardware included; `--out`'s `coverage.txt` lists what hasn't run.

### Fuzzing the simulator alone

`fuzz/` fuzzes the simulator with libFuzzer, guided by branch coverage,
with no dev kit: each input's bytes are a storage image's settings and a
script of steps, as `--fuzz` makes from a seed (the same generator, with
the bytes as its choices, and steps only the simulator can take besides:
sensor readings, last night's sleep, pairing and updates; for a dev kit,
those are comments). It fails on a panic (overflow checks and
`debug_assert!`s on), on a step that never ends (the UI spinning: the
timeout), and when the settings the UI has, two seconds on, aren't the
ones it would boot with from its storage:

```sh
cargo +nightly fuzz run -s none steps -- -fork=16 -timeout=20
```

`--fuzz-input FILE --out DIR` writes the script and storage an input makes
(`--storage BASE` lays its settings over a dev kit's storage, to keep its
bonds), to run here (`--flash DIR/storage.bin`) or on a dev kit
(`--devkit ELF --storage DIR/storage.bin`). Before `--corpus`, distil the
corpus to one input per branch:

```sh
fuzz/target/x86_64-unknown-linux-gnu/release/steps -merge=1 -use_counters=0 \
    -use_cmp=0 ../tmp/distilled fuzz/corpus/steps
```

(The same for the `gadgetbridge` target's, for `--corpus DIR
--gadgetbridge`.)

Each `compare` also says how long the dev kit took to push its frame
to the panel (if that pass pushed anything), and how long the
simulator's panel says that takes (see below), as a check on the
model.

## The flash

The watch keeps its settings (themes, brightness, the button grid,
alarms, the phone's alarms, the location, the temperature's
calibration) in 12 KiB of its internal flash, and so does the
simulator: the firmware's own storage (`src/storage.rs`, and the
sequential-storage map under it) runs over a model of NOR flash with the
chip's pages and addresses. The UI loop saves what changes, and loads it
at boot, as on the watch. Without `--flash`, the flash starts erased and
is forgotten at the end (so scripts and tests don't depend on what ran
before; so does the browser's); with `--flash FILE`, it's kept in `FILE`
from one run to the next.

`FILE` is the region's 12 KiB as on the chip, so it can be a watch's own,
dumped over SWD, to see what a release saved and boot with it:

```sh
esprobe --target nRF52832_xxAA fast-dump flash.bin --address 512000 --size 12288  # 0x7D000
echo settings | cargo run --release -- --flash flash.bin -
```

Switching off (storage mode, an empty battery) and back on (the button, the
charger) boots again as the watch does: the settings come from the flash,
uptime starts from 0, the time, the heart rate and the Aranet4s heard are
lost, and a connected phone sets the time again. Today's steps and
history are kept, as the watch keeps them in its SPI flash.

## How it works

`src/lib.rs` includes the firmware's hardware-free modules with
`#[path]`, as the host tests do, at the crate root where their `crate::…`
paths expect each other. That includes the UI loop itself
(`src/ui_loop.rs`): what the loop does on a tick, an input or its
timer, when it renders and with what, the temperature it shows, the
screen's power, and what it saves are the firmware's code, run by
`watch.rs` on a clock of its own (at the watch's 32 768 Hz), which only
moves when told: scripts are deterministic, and skip hours at no cost.
Stand-ins replace the rest:

- `display`: a fake ST7789 that collects what `Canvas::flush` and
  `Canvas::transition` push, so screenshots, recordings, and the window
  show what the panel would, transitions included, and takes as long as
  the real one to. A recording (`src/record.rs`) looks at it each time
  the clock is about to move.
- `watch::Hardware`: what `main.rs` drives for the UI loop (the panel's
  power, the backlight, the battery and die temperature readings, the
  button grid's advertisements, which are printed), as told by commands.
- `flash`: the internal flash, above.
- `activity`: steps, heart rates and sleep, as told.
- `ble`, `dfu`, `power`: what the Bluetooth and update tasks share with
  the UI loop (the phone's connection, a passkey, update progress, the
  Aranet4s a scan heard, the queue of messages for the phone, which are
  printed), and the hourly power report's counters.
- `motor` is the real one; the simulator plays its requests on the bezel.

Pushing pixels takes the watch's time: each transfer to the panel waits
on the simulator's clock as long as it takes on a dev kit (fitted to its
frames, `stub/display.rs`'s `cost`): 1.12 µs a byte (the 8 MHz SPI bus,
and the CPU building the rows), and 1.03 ms a transfer, whose cause
isn't known yet. So a transition takes ~0.47 s, as on the watch, and a
tick's flush a few milliseconds. The clock moves straight to the next
timer due, rather than a millisecond at a time.

The browser's build is the same library, as a WebAssembly module whose
exports (`src/web.rs`) the page calls each animation frame and for each
input, through plain functions and a few lines of JavaScript
(`web/ur.js`), rather than wasm-bindgen: only cargo builds it.
`src/platform.rs` has what differs: the wall clock comes from the page,
log lines go to it, and files download.

## What it can't simulate

- **Sensors.** Steps, movement, heart rate, sleep and temperature are
  whatever you set: the accelerometer and heart rate processing, the
  sleep classifier, and the wrist-turn detector don't run (`asleep on`
  says the classifier decided; `glance` that the wrist turned; `heart`
  that a measurement just finished, so it shows for half an hour, and
  the watch is taken to be worn). `temperature` is the die's, read once a
  minute as on the watch, and shown as the firmware would: as an
  Aranet4's if one's fresh, else as the wrist's estimate once worn. The
  watch is never lying still on a desk.
- **Bluetooth.** Only the phone's messages: `gb` parses its lines as the
  watch does, but no radio, GATT, or MTU-sized pieces carry them.
  Pairing, bonds, updates, GPS requests and the activity sync don't run;
  `passkey` and `update` only show their screens, and `slots full` only
  what the grid does when its presses can't go.
- **Flash, in part.** Settings are saved and loaded (see
  [The flash](#the-flash)), but a location from the phone isn't saved,
  and Bluetooth bonds and the SPI flash (the activity record, field logs)
  aren't simulated.
- **Timing.** Pushes take the panel's time (see above), within ~16% of
  a dev kit's; nothing else does. Rendering a frame takes no time (on
  the watch, a few milliseconds), nor do the panel's waits on waking
  (120 ms between sleep commands, a refresh before the backlight comes
  on). The panel's own refresh, which shows frame memory to the glass
  top to bottom at 60 Hz (~8 Hz in idle mode), isn't modelled: the
  window shows each pixel the moment it's sent. The backlight's
  switching off while redrawing into or out of the dim look isn't shown.
- **Logs.** The firmware's defmt logs go nowhere (decoding them isn't
  wired up). The simulator prints what happens instead.
- The touch controller's quirks (missed lifts, garbage reports), the
  watchdog (the UI loop feeds nothing), resets that keep RAM, panics'
  breadcrumbs, the bootloader, and anything else below the UI.
- **The panel's colours** (in the window) are a model (see
  `src/panel.rs`), from a similar panel's datasheet rather than this
  one's measurements.
