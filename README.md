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
receives updates the same way InfiniTime does, so the same package installs
it over InfiniTime and updates it afterwards.

### Building the package

```sh
cargo build --release
python3 scripts/package.py target/thumbv7em-none-eabihf/release/ur --version 0.2.0
```

This writes `target/thumbv7em-none-eabihf/release/ur-dfu.zip`. Use a higher
version each time; the watch logs it at boot.

### First install, over InfiniTime

1. On InfiniTime 1.16 or later, turn on Settings → OTA updates.
2. Send `ur-dfu.zip` to the watch as a firmware update. Any of these will
   do:
   - Nordic's **nRF Device Firmware Update** app (Android/iOS): pick the zip,
     then the watch ("InfiniTime").
   - Gadgetbridge, with the watch added as an InfiniTime device: open the
     zip with Gadgetbridge's firmware installer.
   - From a Linux computer: `scripts/dfu.py ur-dfu.zip --address <MAC>`
     (needs `pip install bleak`).
3. The watch resets and shows the PINE64 logo for about a minute while the
   bootloader swaps the images. Leave it alone until the ur clock face
   appears.
4. In Gadgetbridge, remove the InfiniTime device, then add "Bangle.js ur".
   The watch keeps the new firmware once the phone has synced the time with
   it. Until then, any reset puts InfiniTime back.

### Updating

Send a newer `ur-dfu.zip` the same way. The watch shows a progress bar,
resets into the new version, and keeps it once Gadgetbridge reconnects. ur
only accepts updates from devices paired with its passkey:

- **On the phone:** it's paired already; use the nRF Device Firmware Update
  app.
- **From a computer:** pair once in `bluetoothctl` (`agent KeyboardOnly`,
  `default-agent`, `pair <MAC>`, then type the passkey). After that,
  `scripts/dfu.py ur-dfu.zip --address <MAC> --packet-size 240` takes about
  half a minute.

### Rolling back

Hold the button. After about 7 seconds the watch resets into the
bootloader. Keep holding while the logo shows: release when it turns blue to
go back to the previous firmware. Holding until it turns red installs
PINE64's recovery firmware from the SPI flash, a minimal InfiniTime for
reinstalling from.

### With a debug probe

On a dev kit with SWD wired up, `cargo run --release` packages the firmware,
flashes it straight into the bootloader's slot, and streams the logs. The
runner is `scripts/run.sh`, which uses `esprobe`. The PineTime bootloader
must be at 0x0; `memory.x` describes the layout.

Tested so far: the first install from InfiniTime 1.14.1, and the updates,
rollbacks, and confirmations after it, all with `scripts/dfu.py` from Linux
on a dev kit. The phone apps above should work, since they speak the same
protocol, but have not been tried yet.

## Third-party code

`src/bma423_config.bin` is Bosch's BMA423 feature-engine configuration,
under the BSD-3-Clause licence in `src/bma423_config.LICENSE`.
