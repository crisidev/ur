# ur

Firmware for the [PineTime] smartwatch, in Rust on [embassy]: a pixel-art
watch face that keeps out of your way, plus phone integration through
[Gadgetbridge].

- **Clock face:** time with seconds, date as YYYY-MM-DD, battery, steps, and
  heart rate.
- **Themes:** dark after sunset and light by day, worked out from your
  location. Long-press to cycle through the themes.
- **Notifications:** only which app and who it's from, briefly. Calls take
  over the screen, and you can answer or decline.
- **Media controls:** play, pause, next, and previous, while something is
  playing.
- **Activity:** steps and per-minute movement, synced to the phone.
- **Updates:** over Bluetooth, with automatic rollback.

Gadgetbridge sees the watch as a Bangle.js: ur speaks the [Bangle.js
protocol] so that stock Gadgetbridge works without patches.

**Status:** experimental and personal. Alarms and sleep tracking are still
to come. This was written largely by Claude (Anthropic's model), steered and
tested on real hardware by a human.

[PineTime]: https://pine64.org/devices/pinetime/
[embassy]: https://embassy.dev
[Gadgetbridge]: https://gadgetbridge.org
[Bangle.js protocol]: https://www.espruino.com/Gadgetbridge

## Using it

- **Button:** wakes the screen, and silences a call.
- **Swipes:** move between the clock, music (while something is playing),
  and info screens.
- **Connecting a phone:** in Gadgetbridge, add the watch "Bangle.js ur" and
  type the passkey the watch shows. Gadgetbridge then syncs time, location,
  notifications, and activity on its own. For sun-based themes away from
  Oslo, turn on "Use phone GPS data" in the watch's Gadgetbridge settings.

## Installing

The PineTime's bootloader (MCUBoot, as shipped with every PineTime) only
accepts firmware over Bluetooth, from the firmware that's running. ur
receives updates the same way InfiniTime does, so one package,
`ur-dfu.zip`, installs it over InfiniTime and updates it afterwards.

Tested so far: the first install from InfiniTime 1.14.1, and the updates,
rollbacks, and confirmations after it, all with `scripts/dfu.py` from Linux
on a dev kit. The phone apps below should work, since they speak the same
protocol, but have not been tried yet.

### Building the package

```sh
cargo build --release
python3 scripts/package.py target/thumbv7em-none-eabihf/release/ur --version 0.2.0
```

This needs `llvm-objcopy` on your `PATH`, and writes
`target/thumbv7em-none-eabihf/release/ur-dfu.zip`. `--version` (default
0.1.0) goes in the image header, and the watch logs it at boot; raise it
for each build so you can tell them apart.

To install from a phone, copy the zip onto it any way you like (USB, cloud
storage, email), or with `adb`:

```sh
adb push target/thumbv7em-none-eabihf/release/ur-dfu.zip /sdcard/Download/
```

### Sending the package

Use whichever of these suits you:

**Nordic's nRF Device Firmware Update app** (Android and iOS; first
install and updates):

1. Install "nRF Device Firmware Update" by Nordic Semiconductor from the
   Play Store or App Store.
2. On the first install, if Gadgetbridge is connected to InfiniTime,
   disconnect it there first: InfiniTime takes one connection at a time,
   so the app wouldn't find it. (ur takes a second connection
   alongside the phone's, so for updates Gadgetbridge can stay connected.)
3. Select File, choose the ZIP ("Distribution packet") type, and pick
   `ur-dfu.zip`.
4. Select Device, and pick the watch: "InfiniTime" for the first install,
   "Bangle.js ur" after that.
5. Start, and wait for the transfer to finish.

The app connects using the phone's existing Bluetooth pairing. ur only
accepts updates from a device paired with its passkey, so for updates use
the phone that Gadgetbridge paired with the watch. (InfiniTime needs no
pairing.)

**Gadgetbridge** (Android; first install only):

1. Add the watch, still running InfiniTime, as a new device in Gadgetbridge
   (it shows up as "InfiniTime"), and let it pair and connect.
2. While it's connected, open `ur-dfu.zip` from a file manager and pick
   Gadgetbridge's firmware installer ("Open with" → Gadgetbridge).
   Gadgetbridge only offers its installer while the watch is connected.
3. Tap Install.

After that, Gadgetbridge knows the watch as a Bangle.js, and its Bangle.js
support doesn't install firmware: use one of the other two for updates.

**`scripts/dfu.py`** (Linux, with a BlueZ adapter; needs `pip install
bleak`). `<MAC>` is the watch's Bluetooth address, which `bluetoothctl`
shows while scanning.

- First install: `python3 scripts/dfu.py ur-dfu.zip --address <MAC>`.
- Updates: pair once in `bluetoothctl` (`agent KeyboardOnly`,
  `default-agent`, `pair <MAC>`, then type the passkey the watch shows).
  Then `python3 scripts/dfu.py ur-dfu.zip --address <MAC> --packet-size 240`
  takes about half a minute. (InfiniTime only takes the default 20-byte
  packets.)

### First install, over InfiniTime

1. On InfiniTime 1.16 or later, turn on the OTA switch in Settings.
2. Send `ur-dfu.zip` with any of the methods above.
3. The watch resets and shows the PINE64 logo for about a minute while the
   bootloader swaps the images. Leave it alone until the ur clock face
   appears.
4. In Gadgetbridge, remove the InfiniTime device, then add "Bangle.js ur"
   (see [Using it](#using-it)). The watch keeps ur once the phone has synced
   the time with it. Until then, any reset puts InfiniTime back.

### Updating

Send a newer `ur-dfu.zip` with the nRF Device Firmware Update app or
`scripts/dfu.py`. The watch shows a progress bar, resets into the new
version, and keeps it once Gadgetbridge reconnects and syncs the time. If
the watch resets before that, it goes back to the previous version.

### Rolling back

Hold the button. After about 7 seconds the watch resets into the
bootloader. Keep holding while the logo shows: release when it turns blue to
go back to the previous firmware. Holding until it turns red installs
PINE64's recovery firmware from the SPI flash, a minimal InfiniTime for
reinstalling from.

### Field logs

A build with `--features field-log` keeps everything it logs in the
watch's SPI flash (about two days' worth), for debugging a sealed watch:

```sh
DEFMT_LOG="info,ur::heart=debug,ur::activity=debug" \
  cargo build --release --features field-log
python3 scripts/package.py target/thumbv7em-none-eabihf/release/ur --version 0.3.1
```

Install it as above and wear it. Then, from a computer paired with the
watch (the phone can stay connected), fetch and decode the log with the
ELF that `package.py` kept:

```sh
python3 scripts/fetch-log.py --address <MAC> --out logs
defmt-print -e target/thumbv7em-none-eabihf/release/ur-0.3.1.elf < logs/field-log-0.3.1.defmt
```

Every build also logs why the chip last reset, and where the previous run
panicked, if it did.

### With a debug probe

On a dev kit with SWD wired up, `cargo run --release` packages the firmware,
flashes it straight into the bootloader's slot, and streams the logs. The
runner is `scripts/run.sh`, which uses `esprobe`. The PineTime bootloader
must be at 0x0; `memory.x` describes the layout.

## Third-party code

`src/bma423_config.bin` is Bosch's BMA423 feature-engine configuration,
under the BSD-3-Clause licence in `src/bma423_config.LICENSE`.
