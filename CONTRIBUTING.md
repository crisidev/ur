# Contributing to ur

ur is `no_std` Rust on [embassy], for the [PineTime]'s [nRF52832], booted by
the [MCUBoot] bootloader every PineTime ships with ([PINE64's
build][pinetime-bootloader]). This covers how it's built, how to iterate
on it with a dev kit (the way it was developed), how to get logs off a
sealed watch, and how releases are made.

Images (`*.png`, `*.gif`: the README's screenshots and GIFs, and the
simulator's approved screenshots) are in [Git LFS]: run `git lfs install`
once before cloning, or `git lfs pull` after, or the simulator's tests
will compare against pointer files. Adding or re-approving an image then
needs nothing special; LFS stores it.

[Git LFS]: https://git-lfs.com

## Layout

- `src/`: the firmware. `main.rs` starts the tasks and runs the UI loop;
  each module's documentation explains its part. The hardware-free ones
  (`ppg`, `sleep`, `stages`, `alarm`, `aranet`, `glance`, `heart_schedule`,
  `brightness`, `temperature`, `battery`, `snapshot`, `nav`, `day`,
  `route`, `arrival`, `decimal`, `gesture`, `wake`, `gb`, `phone`,
  `link`, `buttons`, `qr`, `mcuboot`, `on_time` (`power`'s hourly split),
  `minute_ring` (`activity`'s history as the clock moves), and `bthome`'s
  encoding) are pure logic, tested on the computer, as are
  `theme`'s colours (that those with different meanings differ in every
  theme).
- `host-tests/<module>/`: one crate per hardware-free module (`sleep`'s
  covers `stages` too, and `gb`'s `phone`), which includes the firmware's
  source file with `#[path]` and tests it on the host. Some also replay
  real data (see [Field logs](#field-logs)).
- `sim/`: a simulator that runs the UI on the computer; see
  [The simulator](#the-simulator).
- `scripts/`: packaging (`package.py`, `release.sh`), flashing and
  screenshots over SWD (`run-esprobe.sh`, `screenshot.py`), and Bluetooth tools
  (`dfu.py`, `fetch-log.py`).
- `.cargo/config.toml`: the target, the linker scripts, and the runner
  for `cargo run`; see [Iterating](#iterating).
- `memory.x`: the flash and RAM layout around the bootloader.
- `docs/`: designs for work in progress, and the README's screenshots.

## Building

```sh
cargo build --release
```

`rust-toolchain.toml` pins the compiler ([rustup] installs it on first
use), with the `thumbv7em-none-eabihf` target. `.cargo/config.toml` makes
that target the default, and passes the linker scripts (`link.x` from
`cortex-m-rt`, which includes `memory.x`, and [defmt]'s `defmt.x`). The
release profile is what runs on watches; unoptimised builds are too large
and slow.

To build what a release ships, both variants, packaged for the
bootloader, into `dist/`:

```sh
scripts/release.sh
```

This needs Python 3.11 or later, and `llvm-objcopy` (from rustup's
`llvm-tools` component, which `rust-toolchain.toml` asks for, if not on
your `PATH`). The version is Cargo.toml's, with the commit appended
(`0.9.0+1a2b3c4`, plus `.dirty` with uncommitted changes); `--pre pr.12`
makes it a prerelease (`0.9.0-pr.12+1a2b3c4`). The watch shows it on its
Info screen. The variants:

- `ur`: logs at `info` over [RTT], read through a debug probe.
- `ur-logging`: keeps its log in the watch's SPI flash, for fetching
  over Bluetooth; see [Field logs](#field-logs). It writes to flash
  constantly, and costs a little battery.

Each comes as `-dfu.zip` (to send over Bluetooth), `.img` (the raw MCUBoot
image), `-slot.elf` (the image as an ELF, for flashing over SWD), and
`.elf` (with symbols, for decoding logs). Neither has the dev kit's
injected input (the `debug-input` feature; see [Iterating](#iterating)):
`release.sh` refuses a build that does.

The host tests run from each crate's directory, where its own
`.cargo/config.toml` points cargo at the host instead of the watch:

```sh
cd host-tests/ppg && cargo test --release
```

### Size checks

`scripts/size.py check` holds built ELFs to every size constraint, with
a table of each one's limit, use and headroom:

- the MCUBoot image (header, code, hash) fits the slot, short of its
  trailer, which is also what an update over Bluetooth may take;
- static RAM (`.data`, `.bss`, `.uninit`) ends below memory.x's `RAM`,
  where the retained block and the stack begin;
- the stack region is at least as deep as the deepest stack measured on
  a dev kit (`STACK_PEAK` in `size.py`: raise it when the hourly
  `power:` log shows deeper);
- the layout agrees everywhere it's written down: memory.x,
  `package.py`, and the Rust constants for storage, the image-OK flag,
  the retained block, the stack's end and the SPI flash's regions.

CI runs it on both released variants and on `compare` (which must fit
too, and has the least RAM to spare), and the release workflow on what it
ships. On a PR, CI also builds where it branched off, and one bot
comment, updated on each push, shows how each variant's sections and
headroom changed. To compare two builds locally, see the top of
`size.py`.

### Flash size

The image must fit the bootloader's slot; `package.py` prints how full it
is, and `size.py` (above) what's left. Only the loaded sections count
(`.text`, `.rodata`, `.data`'s initial values): symbols, debug info and
defmt's strings stay in the ELF, so `debug = 2` is free. `cargo bloat
--release --crates` (or `-n 50`) shows where the rest goes. Some things cost far more than they look:

- `str::parse::<f32>()` or `f64`: 18 KB of tables and code. Use
  `decimal::parse`.
- Any `f64` arithmetic: the FPU is single precision, so it's soft-float
  routines, ~3 KB of them, and none is linked now. That includes libm's
  `sinf` and `cosf`, which compute in `f64` (`geo::sin_deg` doesn't), and
  libm's `f64` functions, ~7 KB more.
- `defmt::Debug2Format(&e)`: puts every variant name of `e`'s type in
  flash (bt-hci's errors alone are 2 KB). Log `e` itself when it
  implements `defmt::Format`, as trouble-host's types do.
- A static that isn't all zeros goes in `.data`, whose initial image takes
  as much flash as the static takes RAM.
- `f32::clamp` (or `f64`'s) with bounds that aren't constants: its
  check of the bounds formats floats into its panic message, ~12 KB.
  Use `.max(low).min(high)`.

### Stack

Every task and interrupt shares one 11 KiB stack (see `memory.x`), and
the hourly `power:` log line reports each hour's deepest use. Pairing and
booting go deepest: both run trouble-host's P-256 maths, ~4 KB of it,
under the Bluetooth host's task. rustc can say how big each function's
frame is (nightly only, so this borrows the pinned compiler with
`RUSTC_BOOTSTRAP`; it changes no code):

```sh
RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR=target/stack-sizes cargo build --release \
  --config "target.thumbv7em-none-eabihf.rustflags=['-Zemit-stack-sizes']"
llvm-readobj --stack-sizes target/stack-sizes/thumbv7em-none-eabihf/release/ur
```

Summing them along call chains takes the call graph from the
disassembly, and that includes `blx rN`: to save flash, LLVM calls many
functions through a register loaded from a literal pool (p256's field
maths, for one). On a dev kit, the painted stack is the ground truth:
read 0x2000D520 up to 0x20010000 over SWD (`esprobe fast-dump --address
536925472 --size 10976 stack.bin`, in decimal), and the lowest word that
isn't 0xCCCCCCCC is the peak. After a reset,
give the bootloader a few seconds before reading: until our startup
code repaints, what's there is the previous run's.

With `opt-level = "z"`, what makes frames big:

- Big values returned or moved by value: each function on the way may
  build its own copy, in its own frame. Fill a `&mut` the caller keeps
  instead (as `ui::wrap` does), or copy into place.
- A task's `async` body is polled by one function with everything it
  awaits or joins inlined, so its frame holds all their locals at once,
  under every poll. A future beside a deep path can be polled in a frame
  of its own (`ble::OwnFrame`); a scope of its own isn't enough.

## The simulator

`sim/` runs the firmware's UI on the computer, in a window (the watch drawn
to scale, the mouse as the finger) or from a script, against a clock it
controls:

```sh
cd sim && cargo run --release
cargo run --release -- tour.txt --out ../tmp/tour
```

It renders exactly what the panel would, so most UI work can be checked
there before a dev kit; `sim/readme-screenshots.txt` makes the README's
screenshots. It takes the dev kit's inputs, every message from the phone
(also as Gadgetbridge's own lines), and sensor readings, and skips ahead
in time, for alarms and the quiet night.

With a dev kit, it checks itself against the watch: flash a build with
the `compare` feature (`cargo run --release --features compare`), and
`ur-sim --devkit <elf> devkit-compare.txt` follows it frame for frame,
failing if any pixel differs. See [sim/README.md] for its commands, and
what it can't simulate (sensors, Bluetooth, the SPI flash). It runs the
UI loop itself (`src/ui_loop.rs`) and the settings' storage, so a change
there needs nothing in `sim/` to match.

### Approved screenshots

The simulator draws the same pixels on every run and every computer, so
its scripts' screenshots are committed, and `cargo test` in `sim/` fails
if any screen changes: it names each screenshot that differs (and
writes an image of it before, after, and where, in magenta), each that's
new, and each no longer taken. Those of `readme-screenshots.txt` are the
README's own, in `docs/screenshots/`. When a change to what the watch
shows is meant, look at those images, then approve the new screenshots
and commit them with it:

```sh
cd sim && UPDATE_SHOTS=1 cargo test
```

See [sim/README.md](sim/README.md#approved-screenshots) for what's
checked, and the README's `info.png`, which isn't. CI's `sim` job
uploads what the tests drew, and the images of any differences, as its
`screenshots` artifact.

On every pull request, CI draws the screenshots for its head and for
where it branched off its base, and keeps one comment on the PR up to
date with each screen that changed (before, after, and where, side by
side), was added, or was removed. It says so when a push leaves no
screen changed, but doesn't comment at all on a PR that never changed
one. Two workflows do this, so that a PR from a fork never runs with
write access:

- `screenshots.yml` runs on the PR, with read-only access: it builds the
  simulator from both commits, draws both sets, compares them
  (`ur-sim --diff`), and uploads the differences, with the PR's number,
  as an artifact.
- `screenshots-comment.yml` runs after it, on `workflow_run`, from the
  default branch's copy of itself, never the PR's code. It treats the
  artifact as untrusted: the PR must exist and have the commit the run
  was for, and every file must be a PNG with a plain name, of limited
  size and number. It uploads the images as comment attachments (what
  `gh pr comment --attach` does, nothing stored in the repository), then
  edits its comment in place to show them. It skips Dependabot's PRs,
  whose runs only get read access.

If the images can't be uploaded, the comment still says what changed,
with each screen's name and how many pixels differ, notes that the
images couldn't be attached, and links the artifact of the run that
drew them (kept for 7 days).

## Finding bugs

Beyond the tests, four tools look for what nobody thought to test. All
but the dev kit's run on the computer; the heavy ones can take a lot of
memory, so run them in a capped scope (`systemd-run --user --scope -p
MemoryMax=6G --quiet …`): on a machine without much swap, an
out-of-memory kill can take down the terminal it ran from.

### Fuzzing the simulator

`sim/fuzz` drives the simulator (the firmware's UI, storage and
Gadgetbridge handling) with libFuzzer, guided by branch coverage: each
input's bytes choose the settings in storage and a script of steps, as
`ur-sim --fuzz` makes from a seed. It fails on a panic (overflows and
`debug_assert!`s included), on a step that never ends (the UI spinning),
and when the settings the UI has aren't the ones it would boot with:

```sh
cd sim && cargo +nightly fuzz run -s none steps -- -fork=16 -timeout=20
```

(`-s none`: it's safe Rust, so no sanitizer; `--` passes the rest to
libFuzzer.)

A finding is an input file; `ur-sim --fuzz-input FILE --out DIR` writes the
script and storage image it makes, to run in the simulator (`--flash
DIR/storage.bin`) or on a dev kit (`--devkit … --storage`, with
`--storage BASE` to keep a dev kit's bonds).

### Fuzzing a dev kit

`ur-sim --fuzz ELF` runs random scripts on a dev kit with a `compare`
build, each from random settings, and follows it as `--devkit` does: any
difference from the simulator, a restart the script didn't ask for (with
the dev kit's log of why), a step that leaves less than 512 bytes of
stack, or a grid hold without its release, is shrunk to a repro. See
[sim/README.md]. Two ways to make it less random:

- `--corpus DIR` runs the simulator fuzzer's inputs instead: what it
  found, by branch coverage, checked against the hardware. Distil the
  corpus first, to one input per branch (see sim/README.md).
- `--guided`, with a build from `sim/coverage-build.sh`, extends the runs
  that reached functions no run had before. It's function coverage, not
  branches (those don't fit in flash), but of everything on the watch,
  Bluetooth and the hardware included, which the simulator can't run;
  `--out`'s `coverage.txt` lists what's not been reached.

### Proofs

Some host test crates (`host-tests/alarm`, `buttons`, `gesture`, `day`,
`decimal`) have [Kani] proofs at the end of their `src/main.rs`: what the
tests check for some inputs, for every input, within bounds. `cargo kani`
in the crate runs them; `--harness NAME` one. They need tight
`kani::unwind` and `kani::assume` bounds: CBMC's memory grows fast with
loops (one unbounded alarm proof took 18 GB). `gb` and `day` also have
[Bolero] property tests, which `cargo test` runs briefly with random
inputs, and `cargo bolero test NAME` fuzzes for as long as you like.

### cargo-careful and Miri

`cargo +nightly careful test` (a standard library with its debug checks)
and `cargo +nightly miri test` (an interpreter that catches undefined
behaviour) both run the host tests unchanged. Neither found anything in
November 2026; Miri can't run `ppg` (libm's x86 square root is inline
assembly) and takes over 15 minutes on `gb`, `mcuboot` and `sleep`.

## A dev kit

A PineTime dev kit is a PineTime with its SWD port exposed. With a debug
probe on that port you can flash builds in seconds, read the logs live,
and take screenshots, instead of updating over Bluetooth and waiting.

### The probe

ur was developed with an [ESP32-C3] Super Mini board running [esprobe-rs]
as the SWD probe, wired to the dev kit as PINE64's [dev kit wiring] page
shows. Only SWDIO, SWDCLK and GND are needed: leave 3.3V unconnected with
the battery in, or the battery feeds power back into it.

| Dev kit | ESP32-C3 Super Mini |
| ------- | ------------------- |
| SWDIO   | GPIO 1              |
| SWDCLK  | GPIO 0              |
| GND     | GND                 |

Build the probe's firmware, and flash it with [espflash]:

```sh
git clone git@github.com:perara/esprobe-rs.git
cd esprobe-rs/crates/esprobe-firmware
cp wifi.env.example .env.local
openssl rand -hex 32  # the password for the probe's fallback Wi-Fi:
$EDITOR .env.local    # put it in here
paru -S clang21       # or your distribution's clang 21
rustup override set nightly-2026-10-17
rustup component add rust-src
# Pin scripts/build.sh to the same nightly, because of
# https://github.com/rust-lang/rust/issues/164555
$EDITOR scripts/build.sh
env LIBCLANG_PATH=/usr/lib/llvm21/lib/ \
  PIN_SWDIO=1 PIN_SWCLK=0 PIN_RESET=21 PIN_UART_TX=3 PIN_UART_RX=4 \
  ./scripts/build.sh
espflash flash --monitor target/riscv32imc-esp-espidf/release/esprobe-firmware
```

Then the host side, the [`esprobe`][esprobe] command:

```sh
cargo install esprobe --locked
ls /dev/ttyACM0   # the probe, over USB
id                # you need to be in the group that owns it (uucp on Arch)
esprobe pin-map   # check the pins
esprobe identify  # finds the nRF52832
esprobe lines     # SWD line levels, if it doesn't
```

Before flashing anything, back up the watch's internal flash (the
bootloader and InfiniTime):

```sh
esprobe --target nRF52832_xxAA fast-dump pinetime-factory.bin --size 524288
```

**Other probes** work too: PINE64's [reprogramming] page covers J-Link,
ST-Link, CMSIS-DAP (DAPLink), the Black Magic Probe, and a Raspberry Pi's
GPIO, with OpenOCD and nrfjprog; [probe-rs] also supports most of them.
Only the scripts that talk to the probe need changing; see below.

### Iterating

```sh
cargo run --release
```

`.cargo/config.toml` sets `scripts/run-esprobe.sh` as the runner, which packages
the build, flashes it into the bootloader's slot, and streams its defmt
logs (for `RTT_SECS` seconds, 25 by default). The bootloader must be at
0x0, as it ships; `memory.x` describes the layout. Flashing this way skips
the bootloader's swap and confirmation, so a broken build doesn't roll
back: flash a working one.

`scripts/screenshot.py` reads the screen out of RAM, and can inject input
first, so UI changes can be checked without looking at the watch. Input
needs a build with the `debug-input` feature (`compare` implies it);
releases leave it out, as a sealed watch has no probe to write it:

```sh
cargo run --release --features debug-input
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur clock.png
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur alarms.png --input left
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur day.png --theme HANDHELD
```

Inputs are taps, swipes, the button, a glance, test notifications, and
more (a battery voltage, steps, the wearer asleep for the quiet night,
which a dev kit on a desk never reaches, a panic or a hang, to test the
crash reports); see the script's documentation. They act as on the watch, so
with the screen off, start with `button` to wake it. Features combine:
`--features field-log,debug-input` to drive a field-log build. (The README's screenshots come
from the [simulator](#the-simulator), which matches these pixel for
pixel.)

`scripts/flash-stress.py` checks that saving to the internal flash can't
reset the watch: a build with the `flash-stress` feature saves back to
back (a page erase every few seconds), while the script reads RAM over
the probe as fast as it goes, and reports resets (with the boot log's
reason), failed or stalled saves, and how long erases took. Run it after
changing `src/nvmc.rs` or `src/storage.rs`:

```sh
RTT_SECS=3 cargo run --release --features flash-stress
python3 scripts/flash-stress.py target/thumbv7em-none-eabihf/release/ur --minutes 10
```

(`RTT_SECS=3`, so the runner detaches before the saving starts: see
below.) Its saves stay behind under a key of their own
(`storage::key::STRESS`), harmless to a normal build.

`scripts/touch_probe.py` looks for a touch controller register that
tells a fingertip from a bigger contact (a knuckle, a palm, a forearm
brushing the edge). A build with the `touch-probe` feature reads 0x00–0x0F
and 0xB0–0xB3 on every touch report and logs them all (see
`src/touch_probe.rs`). Capture its log while someone touches the dev kit
as `guide` asks, then analyse it:

```sh
DEFMT_RTT_BUFFER_SIZE=2048 RTT_SECS=900 cargo run --release --features touch-probe 2>&1 | tee tmp/touch-probe.log
python3 scripts/touch_probe.py guide    # in another terminal, with the screen on
python3 scripts/touch_probe.py analyse tmp/touch-probe.log --csv tmp/touch-probe.csv
```

(The bigger RTT buffer, as a finger down logs ~100 lines a second;
`analyse` counts the lines dropped anyway.)

**Probe sessions halt the core.** Most `esprobe` commands (`core read`
and `core write`, `rtt`, `flash`) are a probe-rs session, which halts
the core for ~20 ms as it attaches and again as it detaches (to clear
breakpoints). A halt in the middle of a flash timeslot overruns it, and
MPSL resets the watch, so a dev kit that saves while being read that
way can reset (a watch without a probe can't). So whatever reads
often (`screenshot.py`, `ur-sim --devkit`, the stress script) reads
with `esprobe fast-dump`, which doesn't halt; only their writes (an
injected input) and resets still halt. An injected input that saves
something (a setting, say) used to reset the dev kit every few taps,
as the save started before the `core write` had detached; the flash
driver now waits for a probe's session to end first (`src/nvmc.rs`), so
only a session that *starts* mid-save can still do it. (`esprobe
dap-poke` doesn't help: it doesn't halt, but its writes don't land.)
The stress script's `--halting` reads with `core read`, to see it.

**With a different probe**, nothing in the repository needs to change.
Write your own runner, doing what `scripts/run-esprobe.sh` does, and
point cargo at it for yourself: the environment variable overrides the
`runner` in `.cargo/config.toml` (in your shell's profile, or a `.envrc`):

```sh
export CARGO_TARGET_THUMBV7EM_NONE_EABIHF_RUNNER=~/bin/ur-run-jlink.sh
```

Two scripts talk to the probe, so these are what a runner (or a port of
the screenshot script) has to replace:

- `scripts/run-esprobe.sh` gets the ELF's path from cargo, packages it
  (`scripts/package.py`, which writes `<elf>-slot.elf` next to it: the
  image at the bootloader's slot), flashes that, waits for the
  firmware's RTT control block, then pipes RTT into
  [`defmt-print`][defmt-print]. With probe-rs, that's
  roughly `probe-rs download --chip nRF52832_xxAA "$ELF-slot.elf"`, a
  reset, and `probe-rs attach --chip nRF52832_xxAA "$ELF"`. Note that the
  nRF52 aliases its RAM at 0x00800000, so tools that scan RAM for the RTT
  control block find it twice: give them its address (the script reads it
  from the ELF's `_SEGGER_RTT` symbol).
- `scripts/screenshot.py` reads and writes words of RAM (`fast-dump`,
  `core write`), in `read_words` and `write_word`. A read mustn't halt
  the core (see above).

A runner for another probe, tested on one, would be a welcome
contribution (as `scripts/run-<probe>.sh`).

## Field logs

A sealed watch has no debug probe. The `ur-logging` build instead keeps
everything it logs in the watch's 4 MB SPI flash (2 MB of it, about a day
and a half), and serves it over Bluetooth. Install it like any update (see
the README), wear the watch, then, from a computer paired with it (the
phone can stay connected), fetch the log with `scripts/fetch-log.py`
(which needs [bleak]) and decode it with that build's ELF and
[`defmt-print`][defmt-print]:

```sh
python3 scripts/fetch-log.py --address <MAC> --out logs
defmt-print -e ur-logging-0.9.0.elf < logs/field-log-0.9.0.defmt > decoded.log
```

Every build also logs why the chip last reset, and, after a panic or a
watchdog reset, where the previous run was, with a backtrace: the call
sites found on the stack, innermost first. A few are stale leftovers, but
the real chain is among them. Decode them with the build's ELF (`-i`
unfolds what was inlined into each):

```sh
llvm-addr2line -fipe ur-logging-0.9.0.elf 0x2b8b2 0x25c0a
```

A panic's report has the start of its message too, if it's a plain
string (`explicit panic`, a `panic!("…")` without arguments): a
formatted one (`expect`'s, an index out of bounds, …) shows as
`(formatted: not kept)`, since formatting it would cost flash; its
location and backtrace say where it was, and the source says what. A
fault in Nordic's SoftDevice Controller or MPSL reports the assert's own
file and line, which identify it to Nordic, in place of the location:
`panicked at <file>:<line>: SoftDevice Controller ASSERT`. A
`defmt::panic!` (or `defmt::unwrap!`, as dependencies use) only shows
`explicit panic` at defmt's own location: its message went to the log,
which the reset lost, so the backtrace is what says where it was. (So
`ur-logging` leaves out the log lines of the dependencies that log
nothing else; see `scripts/release.sh`.) A hang with interrupts masked
can't be caught: the watchdog's interrupt never runs, so only the reset
reason says it happened.

Some host-test crates replay decoded logs, to tune against real wrists:

- `host-tests/sleep`: `cargo run --release -- decoded.log [stages.txt]`
  reclassifies and stages the log's sleep, minute by minute, from its
  movement and heart rate readings. With `stages.txt`, it prints another
  watch's stages under ours, and how well they agree;
  `scripts/hc-stages.py export.db > stages.txt` gets a Garmin's from a
  [Health Connect] export.
- `host-tests/ppg`: `cargo run --release -- sessions.txt` replays the
  heart rate sensor's raw samples. `scripts/ppg_sessions.py` extracts
  them from a decoded log, alongside another watch's heart rate from an
  Android [Health Connect] export (the tuning so far compared against a
  Garmin worn on the other wrist).

  Logs after 0.10.0-rc.3 have whole sessions on a busy wrist too (a
  background session no longer stops early for movement), the data for
  making the estimator tolerate motion; up to rc.3, they end within 3-19
  s. A session's samples, movement and verdicts take ~100 bytes of log a
  second of LED: a desk hour that won't read (six 30 s tries) takes ~18
  KB, where it took ~8 KB. Next to the ~75 KB an hour the log takes
  anyway (0.10.0-rc.3's, mostly the accelerometer's lines), the 2 MB
  still holds a day.

  For 0.10's LED experiment (`heart_schedule::Arm`), its last lines
  compare the awake background sessions that started dim with those that
  started bright: sessions, readings (replayed, and as recorded),
  readings per minute of LED (and per minute at 12.5 mA's cost), error
  against the reference, and the median pulse/DC and window quality;
  and, for logs after 0.10.0-rc.3, the rounds of tries (a scheduled
  session and its retries, which share an arm), how many read, and
  sessions a round. The rounds are the fair comparison: an arm is chosen
  per round, before its first try. Bright helps if it reads more often
  at no worse error, ideally even per minute at 12.5 mA's cost; a
  pulse/DC that doesn't change says the extra light only lit the same
  tissue. A desk day has a few dozen sessions on each arm, so look at a
  couple of days before deciding.

Steps come from the accelerometer's own counter, which a log can't
replay. A capture build, `UR_ACTIVITY_LOG=trace scripts/release.sh --pre
accel tmp/accel`, also logs the accelerometer's samples while the wrist
moves (about twice the log on a busy day, so fetch it twice a day), and
`scripts/accel_samples.py decoded.log samples.txt [export.db]` extracts
them, with the counter's steps and a Garmin's, for tuning a step counter
of our own.

## Releasing

[CHANGELOG.md]'s `[Unreleased]` section is kept up to date as changes
land: add an entry with each user-visible change (`/append-to-changelog`
in Claude Code does this). Releases are cut now and then, when there's
something worth having, not for every build tried on a watch; CI's
snapshot packages cover those.

Which number to bump, from what `[Unreleased]` holds:

- **Patch** (0.9.0 → 0.9.1): everything that isn't one of the below: fixes,
  tuning, small improvements, documentation.
- **Minor** (0.9.0 → 0.10.0): significant new features that keep working
  with what's already on a watch.
- **Major** (1.0.0 → 2.0.0): only when upgrading may lose configuration or
  data (settings, bonds, unsynced activity), or substantially changes how
  the watch is used, and only with the maintainer's say-so.

While ur is in early testing and development, it stays on 0.x: a change
that would be major is a minor bump instead (still flagged to the
maintainer first, and called out in the changelog).

Cargo.toml's version is the source of truth. To release:

1. Finalise `[Unreleased]` (`/populate-changelog` audits it against the
   commits since the last tag), then move its entries under a heading
   for the new version and update the compare links at the bottom.
2. Set the version in Cargo.toml, and run any cargo build without
   `--locked` (e.g. `cargo check`) to update Cargo.lock to match. The
   simulator's Info screen shows the version too, so approve its new
   screenshots (`cd sim && UPDATE_SHOTS=1 cargo test`; see
   [Approved screenshots](#approved-screenshots)).
3. Commit, then tag (annotated, with a very brief summary of what changed
   since the last release: the key user-facing effects, naming new
   concepts as the README does, and any particularly impactful change
   behind the scenes) and push:

   ```sh
   git tag -a v0.9.0 -m "ur 0.9.0: <headline>" -m "New: <…>" -m "Also: <…>"
   git push origin main v0.9.0
   ```

   Each `-m` is a paragraph: don't wrap them. GitHub shows every newline
   in a tag's message as a line break, and the message (minus its first
   line) leads the release's notes.

The release workflow refuses a tag that doesn't match Cargo.toml. It
builds with `scripts/release.sh`, attests every file's build provenance
([artifact attestations]), and publishes them as a GitHub release; a
version like `0.9.0-rc.1` makes a prerelease. The tags before v0.9.0
mark development builds from before Cargo.toml carried the version, so
they have no GitHub release.

Releases are [reproducible][reproducible builds]: the same commit gives
the same files, byte for byte, wherever it's built. The toolchain is
pinned, dependencies come from Cargo.lock (`--locked`), paths are
remapped out of the output, and the zip has no timestamps. To check a
release against its published checksums:

```sh
git checkout v0.9.0
scripts/release.sh
gh release download v0.9.0 --pattern sha256.sum --dir published
(cd dist && sha256sum -c ../published/sha256.sum)
```

## CI

- `check.yml`: rustfmt, clippy (both variants, with the pinned toolchain
  and on beta; and the simulator), [shellcheck], and that the Python
  scripts parse.
- `test.yml`: the host tests (pinned and beta), the simulator's tests
  and screenshots (as the run's `screenshots` artifact), and a packaged
  build (by `release.sh`, so without `debug-input`) of every PR and
  every commit on main, to try on a watch:
  `0.9.0-pr.<number>+<commit>`, or `0.9.0-snap.<commits>+<commit>`. The
  run's artifacts include each variant's `-dfu.zip` on its own, unzipped,
  ready to send to a watch, and everything (with the ELFs that decode
  logs) as one zip.
- `release.yml`: on a version tag; see [Releasing](#releasing).

Dependabot proposes updates to the actions, Cargo.lock, and the pinned
toolchain quarterly.

[embassy]: https://embassy.dev
[PineTime]: https://pine64.org/devices/pinetime/
[nRF52832]: https://www.nordicsemi.com/Products/nRF52832
[MCUBoot]: https://github.com/mcu-tools/mcuboot
[pinetime-bootloader]: https://github.com/InfiniTimeOrg/pinetime-mcuboot-bootloader
[rustup]: https://rustup.rs
[defmt]: https://defmt.ferrous-systems.com
[defmt-print]: https://crates.io/crates/defmt-print
[RTT]: https://defmt.ferrous-systems.com/setup
[ESP32-C3]: https://www.espressif.com/en/products/socs/esp32-c3
[esprobe-rs]: https://github.com/perara/esprobe-rs
[esprobe]: https://crates.io/crates/esprobe
[espflash]: https://github.com/esp-rs/espflash
[dev kit wiring]: https://pine64.org/documentation/PineTime/Further_information/Devkit_wiring/
[reprogramming]: https://pine64.org/documentation/PineTime/Software/Reprogramming/
[probe-rs]: https://probe.rs
[bleak]: https://github.com/hbldh/bleak
[Health Connect]: https://developer.android.com/health-and-fitness/guides/health-connect
[artifact attestations]: https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-to-establish-provenance-for-builds
[reproducible builds]: https://reproducible-builds.org
[CHANGELOG.md]: CHANGELOG.md
[sim/README.md]: sim/README.md
[shellcheck]: https://www.shellcheck.net
[Kani]: https://model-checking.github.io/kani/
[Bolero]: https://camshaft.github.io/bolero/
