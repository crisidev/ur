# Contributing to ur

ur is `no_std` Rust on [embassy], for the PineTime's nRF52832, booted by
the MCUBoot bootloader every PineTime ships with. This covers how it's
built, how to iterate on it with a dev kit (the way it was developed), how
to get logs off a sealed watch, and how releases are made.

[embassy]: https://embassy.dev

## Layout

- `src/`: the firmware. `main.rs` starts the tasks and runs the UI loop;
  each module's documentation explains its part. The hardware-free ones
  (`ppg`, `sleep`, `alarm`, `glance`) are pure logic, tested on the
  computer.
- `host-tests/<module>/`: one crate per hardware-free module, which
  includes the firmware's source file with `#[path]` and tests it on the
  host. Some also replay real data (see [Field logs](#field-logs)).
- `scripts/`: packaging (`package.py`, `release.sh`), flashing and
  screenshots over SWD (`run.sh`, `screenshot.py`), and Bluetooth tools
  (`dfu.py`, `fetch-log.py`).
- `memory.x`: the flash and RAM layout around the bootloader.
- `docs/`: designs for work in progress, and the README's screenshots.

## Building

```sh
cargo build --release
```

`rust-toolchain.toml` pins the compiler (rustup installs it on first use),
with the `thumbv7em-none-eabihf` target. The release profile is what runs
on watches; unoptimised builds are too large and slow.

To build what a release ships, both variants, packaged for the
bootloader, into `dist/`:

```sh
scripts/release.sh
```

The version is Cargo.toml's, with the commit appended (`0.9.0+1a2b3c4`,
plus `.dirty` with uncommitted changes); `--pre pr.12` makes it a
prerelease (`0.9.0-pr.12+1a2b3c4`). The watch shows it on its Info screen.
The variants:

- `ur`: logs at `info` over RTT, read through a debug probe.
- `ur-logging`: keeps its log in the watch's SPI flash, for fetching
  over Bluetooth; see [Field logs](#field-logs). It writes to flash
  constantly, and costs a little battery.

Each comes as `-dfu.zip` (to send over Bluetooth), `.img` (the raw MCUBoot
image), `-slot.elf` (the image as an ELF, for flashing over SWD), and
`.elf` (with symbols, for decoding logs).

The host tests run from each crate's directory, where its
`.cargo/config.toml` points cargo at the host instead of the watch:

```sh
cd host-tests/ppg && cargo test --release
```

## A dev kit

A PineTime dev kit is a PineTime with its SWD port exposed. With a debug
probe on that port you can flash builds in seconds, read the logs live,
and take screenshots, instead of updating over Bluetooth and waiting.

### The probe

ur was developed with an [ESP32-C3 Super Mini] running [esprobe-rs] as
the SWD probe, wired to the dev kit as PINE64's [dev kit wiring] page
shows. Only SWDIO, SWDCLK and GND are needed: leave 3.3V unconnected with
the battery in, or the battery feeds power back into it.

| Dev kit | ESP32-C3 Super Mini |
| ------- | ------------------- |
| SWDIO   | GPIO 1              |
| SWDCLK  | GPIO 0              |
| GND     | GND                 |

Build and flash the probe firmware:

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

Then the host side:

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

### Iterating

```sh
cargo run --release
```

The runner (`scripts/run.sh`) packages the build, flashes it into the
bootloader's slot, and streams its defmt logs. The bootloader must be at
0x0, as it ships; `memory.x` describes the layout. Flashing this way skips
the bootloader's swap and confirmation, so a broken build doesn't roll
back: flash a working one.

`scripts/screenshot.py` reads the screen out of RAM, and can inject input
first, so UI changes can be checked without looking at the watch:

```sh
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur clock.png
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur alarms.png --input tap left
python3 scripts/screenshot.py target/thumbv7em-none-eabihf/release/ur day.png --theme HANDHELD
```

Inputs are taps, swipes, the button, a glance, and test notifications; see
the script's documentation. The README's screenshots are made this way,
from a `scripts/release.sh` build (so the Info screen shows a real
version).

## Field logs

A sealed watch has no debug probe. The `ur-logging` build instead keeps
everything it logs in the watch's 4 MB SPI flash (2 MB of it, about a day
and a half), and serves it over Bluetooth. Install it like any update (see
the README), wear the watch, then, from a computer paired with it (the
phone can stay connected), fetch the log and decode it with that build's
ELF:

```sh
python3 scripts/fetch-log.py --address <MAC> --out logs
defmt-print -e ur-logging-0.9.0.elf < logs/field-log-0.9.0.defmt > decoded.log
```

Every build also logs why the chip last reset, and, after a panic or a
watchdog reset, where the previous run was.

Some host-test crates replay decoded logs, to tune against real wrists:

- `host-tests/sleep`: `cargo run --release -- decoded.log` reclassifies
  the log's sleep, minute by minute.
- `host-tests/ppg`: `cargo run --release -- sessions.txt` replays the
  heart rate sensor's raw samples. `scripts/ppg_sessions.py` extracts
  them from a decoded log, alongside another watch's heart rate from an
  Android Health Connect export (the tuning so far compared against a
  Garmin worn on the other wrist).

## Releasing

Cargo.toml's version is the source of truth. To release:

1. Set the version in Cargo.toml, and run any cargo build without
   `--locked` (e.g. `cargo check`) to update Cargo.lock to match.
2. Commit, then tag and push:

   ```sh
   git tag v0.9.0
   git push origin main v0.9.0
   ```

The release workflow refuses a tag that doesn't match Cargo.toml. It
builds with `scripts/release.sh`, attests every file's build provenance,
and publishes them as a GitHub release; a version like `0.9.0-rc.1` makes
a prerelease.

Releases are reproducible: the same commit gives the same files, byte for
byte, wherever it's built. The toolchain is pinned, dependencies come from
Cargo.lock (`--locked`), paths are remapped out of the output, and the
zip has no timestamps. To check a release:

```sh
git checkout v0.9.0
scripts/release.sh
(cd dist && sha256sum -c sha256.sum)  # against the release's sha256.sum
```

## CI

- `check.yml`: rustfmt, clippy (both variants, with the pinned toolchain
  and on beta), shellcheck, and that the Python scripts parse.
- `test.yml`: the host tests (pinned and beta), and a packaged build of
  every PR and every commit on main, uploaded as an artifact to try on a
  watch: `0.9.0-pr.<number>+<commit>`, or `0.9.0-snap.<commits>+<commit>`.
- `release.yml`: on a version tag; see [Releasing](#releasing).

Dependabot proposes updates to the actions, Cargo.lock, and the pinned
toolchain quarterly.
