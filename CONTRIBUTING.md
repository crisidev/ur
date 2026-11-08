# Contributing to ur

ur is `no_std` Rust on [embassy], for the [PineTime]'s [nRF52832], booted by
the [MCUBoot] bootloader every PineTime ships with ([PINE64's
build][pinetime-bootloader]). This covers how it's built, how to iterate
on it with a dev kit (the way it was developed), how to get logs off a
sealed watch, and how releases are made.

## Layout

- `src/`: the firmware. `main.rs` starts the tasks and runs the UI loop;
  each module's documentation explains its part. The hardware-free ones
  (`ppg`, `sleep`, `stages`, `alarm`, `glance`, `heart_schedule`,
  `brightness`, `temperature`, `battery`, `snapshot`, `nav`, `day`,
  `route`, `arrival`, `decimal`, `gesture`, `wake`) are pure logic, tested
  on the computer.
- `host-tests/<module>/`: one crate per hardware-free module (`sleep`'s
  covers `stages` too), which includes the firmware's source file with
  `#[path]` and tests it on the host. Some also replay real data (see
  [Field logs](#field-logs)).
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
`.elf` (with symbols, for decoding logs).

The host tests run from each crate's directory, where its own
`.cargo/config.toml` points cargo at the host instead of the watch:

```sh
cd host-tests/ppg && cargo test --release
```

### Flash size

The image must fit the bootloader's slot; `package.py` prints how full it
is. Only the loaded sections count (`.text`, `.rodata`, `.data`'s initial
values): symbols, debug info and defmt's strings stay in the ELF, so
`debug = 2` is free. `cargo bloat --release --crates` (or `-n 50`) shows
where the rest goes. Some things cost far more than they look:

- `str::parse::<f32>()` or `f64`: 18 KB of tables and code. Use
  `decimal::parse`.
- libm's `f64` `sin`, `cos`, `asin`, `acos`, `fmod`: ~7 KB on top of the
  `f32` ones, which are already in.
- `defmt::Debug2Format(&e)`: puts every variant name of `e`'s type in
  flash (bt-hci's errors alone are 2 KB). Log `e` itself when it
  implements `defmt::Format`, as trouble-host's types do.
- A static that isn't all zeros goes in `.data`, whose initial image takes
  as much flash as the static takes RAM.

## The simulator

`sim/` runs the firmware's UI on the computer, in a window (the mouse is
the finger) or from a script, against a clock it controls:

```sh
cd sim && cargo run --release
cargo run --release -- tour.txt --out ../tmp/tour
```

It renders exactly what the panel would, so most UI work can be checked
there before a dev kit, and its screenshots match `screenshot.py`'s pixel
for pixel; `sim/readme-screenshots.txt` makes the README's. It takes the
dev kit's inputs, every message from the phone, and sensor readings, and
skips ahead in time, for alarms and the quiet night. CI runs its tour
and uploads screenshots of every PR's UI. See [sim/README.md] for its
commands, and what it can't simulate (sensors, Bluetooth, flash).

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
first, so UI changes can be checked without looking at the watch:

```sh
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur clock.png
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur alarms.png --input left
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur day.png --theme HANDHELD
```

Inputs are taps, swipes, the button, a glance, and test notifications; see
the script's documentation. They act as on the watch, so with the screen
off, start with `button` to wake it. The README's screenshots are made this way,
from a `scripts/release.sh` build (so the Info screen shows a real
version).

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
- `scripts/screenshot.py` reads and writes words of RAM (`core read`,
  `core write`), in `read_words` and `write_word`.

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

A panic's report has the start of its message too, including a fault in
Nordic's SoftDevice Controller or MPSL (`SoftDevice Controller ASSERT:
<file>, <line>`, which identifies the assert to Nordic). A `defmt::panic!`
(or `defmt::unwrap!`, as dependencies use) only shows `explicit panic` at
defmt's own location: its message went to the log, which the reset lost,
so the backtrace is what says where it was. A hang with interrupts masked
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
   `--locked` (e.g. `cargo check`) to update Cargo.lock to match.
3. Commit, then tag (annotated, with a very brief summary of what changed
   since the last release: the key user-facing effects, and any
   particularly impactful change behind the scenes) and push:

   ```sh
   git tag -a v0.9.0 -m "ur 0.9.0: <headline>" -m "<two or three short lines>"
   git push origin main v0.9.0
   ```

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
  build of every PR and every commit on main, to try on a watch:
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
