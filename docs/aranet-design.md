# Design sketch: nearby Aranet4 CO₂ and temperature on the watch

Status: **implemented** (2026-11-08, branch `aranet`), with these
changes from the plan below, which was written 2026-11-07 against
`8288e3f` (line references to `src/` may drift):

- **It also scans in the background**, because the clock face's
  temperature now comes from the nearest Aranet4 when one is near, and
  calibrates the on-wrist estimate (`src/temperature.rs`). Sparsely
  (`aranet::Nearby::due`): every 15 min while a sensor was heard within
  the hour, hourly otherwise, never in the quiet night, off the wrist, or
  during an update; a look at the clock face with a sensor around scans
  once the last scan is 10 min old, and opening the Air screen once it's
  a minute old. A scan lasts at most 3 s, and stops once every sensor
  heard within the hour has been heard again. (Since the BTHome
  thermometers below: every sensor the last scan heard, and up to 6 s
  while that includes a BTHome one.)
- **Measured on a dev kit** (2026-11-08, two Aranet4s on firmware
  v2.0.7 in range, the phone connected): §4.0's questions answered. The
  readings come in `ADV_IND` (passive scanning is enough), from static
  random addresses, and each sensor advertised every **~0.57 s**, not
  ~4 s; the dev kit heard each at most 2.3 s apart (90% within 1.2 s).
  The first scan heard the nearest in 0.24 s. The busy office around it
  gave ~80 advertising reports a second, of any device. The vectors are in
  `host-tests/aranet`.
- **Energy** (estimate; RX ~5.7 mA with the DC/DC regulator, which is now
  on): ~3 µA with no Aranet4 around (16 hourly 3 s scans a day), ~5 µA
  with one at home (~70 scans a day of ~1 s), ~20 µA at worst (a scan
  every 10 min that always runs the full 3 s, e.g. a sensor heard within
  the hour that's gone). `power:` logs `scanning` ms and `scans`.
- **Where:** the Air screen sits in the line below the clock, after
  Music, and only exists while a sensor was heard within the hour (as
  Music only exists with media). No pinning, no tap to cycle sensors,
  no labels (the strongest is shown, with 3 dB hysteresis); no extra
  theme colours (the band uses GOOD, TEXT and BAD, and the bar's lit
  third).
- **Radio:** as planned (multirole SDC, `support_scan`,
  `central_count(0)`), but the packet pool stays at 16: the RAM was
  there, and DFU and log fetches keep their buffers.
- **Cost** (against `main` at e06a0e3): flash +47 KB (77% → 87% of the
  slot), ~31.5 KB of it the multirole library; static RAM +2.8 KB (the
  controller's `Mem` 5184 → 6064 B, the library's own ~1.1 KB, the
  rest ours), leaving 2.7 KB free (2.2 KB in the logging build, 0.3 KB
  in the `compare` build); stack +104 B at the deepest measured. A DFU
  ran at the same 5.7 KiB/s into this build as into main's. The packet
  pool at 8 would win 2 KB of RAM back, untested.

- **BTHome thermometers too** (2026-11-08, branch `bthome-temp`): a
  scan also reads unencrypted BTHome v2 service data (UUID `0xFCD2`),
  in `src/bthome_sensor.rs`, and they're room sensors like the Aranet4,
  in the same `Nearby` table (four slots for both kinds, so no RAM
  grew: `Reading` holds a kind, signed hundredths of a degree, and
  optional humidity and battery, and `Sighting` stays 40 bytes; flash
  +1.9 KB, +2.4 KB in the logging build). Decisions:
  - **Parsing** follows <https://bthome.io/format/> and Home Assistant's
    `bthome-ble` (its object table is copied into a host test): the
    objects are walked by their spec lengths (a length byte for text,
    raw and command), the walk stops at an unknown id or an object
    past the end, keeping what came before; temperature from `0x02`
    (hundredths) or `0x45` (tenths), the first of several; humidity
    `0x03`/`0x2E`, battery `0x01`, pressure `0x04`, CO₂ `0x12`.
    Encrypted adverts (no key; the watch's own presses among them) and
    other versions are passed over; bit 1, which the spec reserves,
    means a MAC first, as `bthome-ble` reads it; bit 2 ("trigger
    based") marks a sensor that advertises irregularly. An advert
    without a temperature isn't a sensor heard (pvvx's firmware
    alternates its temperature with its battery voltage).
  - **Which:** the nearest of either kind by RSSI, with the same 3 dB
    hysteresis; its temperature is the clock face's, with the same
    marker (`Source::Sensor`, `temperature_from_sensor`), and it
    calibrates the wrist estimate like an Aranet4's: it's a room
    thermometer, rated to a few tenths. But no pair is learnt outside
    10–35 °C, since BTHome thermometers often sit in fridges,
    freezers, saunas and outdoors.
  - **Air screen:** with no CO₂, the temperature (to the tenth,
    signed) takes the big digits, labelled `°C`, with no band; then
    humidity, and pressure and battery if sent.
  - **Scans:** a LYWSD03MMC on pvvx's firmware advertises every 2.5 s
    by default (`app.c`), so its temperature comes every 5 s
    (`bthome_beacon.c`), which a 3 s scan hears ~60% of the time. A scan
    that waits for a BTHome thermometer may listen 6 s
    (`aranet::SLOW_SCAN_LIMIT`), still stopping once it has heard
    everyone it waits for. And it waits only for what the last scan
    heard: one it missed has likely gone, or advertises too seldom
    (a Shelly BLU H&T sends once a minute, by Shelly's docs), and
    waiting for it held every scan to its limit for the hour. Scans
    come no more often than before. Energy (estimate): with a pvvx
    thermometer at home, ~70 scans a day of ~2.5 s, ~12 µA, against
    ~5 µA with an Aranet4.
  - **New sensors beside known ones:** most scans stop within a
    fraction of a second, once the known Aranet4s are heard, which on
    the dev kit left a laptop's fake BTHome thermometer (advertising
    every second) unheard. So every eighth scan (`LOOK_EVERY`; every
    two hours at the quarter-hour pace) listens to its limit whatever
    it hears: ~2 s more each, ~1 µA in all.
  - **Not done:** a minimum RSSI, so a neighbour's thermometer through
    a wall isn't taken for the room when nothing nearer is heard;
    BTHome v1; a sensor's 1-byte temperatures (`0x57`, `0x58`).
- **Forgotten when far away** (same branch): sensors don't move, so a
  fix from the phone more than 500 m from where sensors were near
  (`aranet::Nearby::located`; the place is the phone's location at a
  fix within 15 min of hearing one) forgets the sensors known, and the
  scans drop to hourly at once rather than after an hour; a fix back
  near the place scans at once. The distance is equirectangular, with
  a series for the cosine (within 2% to 70° N). 500 m is well beyond a
  sensor's range and a Wi-Fi fix's error; a cell-only fix can be
  further off, at worst an hour without the room's reading. +8 B RAM,
  ~0.5 KB flash. The catch: the watch only gets a fix after a 2 h
  Bluetooth gap, at a new time zone, or on connecting with a day-old
  fix, so most departures (with the phone along) still take the hour.
  Each one caught saves ~3–6 scans of 3 s, ~15–30 µAh.

Sources:

- Aranet4-Python (Anrijs, MIT), the reference decoder behind Home Assistant's
  `aranet` integration (`aranet4==2.6.0`):
  [`aranet4/client.py`](https://github.com/Anrijs/Aranet4-Python/blob/master/aranet4/client.py)
  (`ManufacturerData`, `Aranet4Advertisement`, `CurrentReading._set`) and
  [`tests/test_advertisements.py`](https://github.com/Anrijs/Aranet4-Python/blob/master/tests/test_advertisements.py)
  (the test vectors in §1.3).
- Aranet4-ESP32 (Anrijs):
  [`src/Aranet4.h`](https://github.com/Anrijs/Aranet4-ESP32/blob/main/src/Aranet4.h)
  (`fromAdvertisement`, `parseFromAdvertisement`, the flag bitfield).
- ESPHome's native `aranet` component, merged 2026-10-13
  ([PR #18263](https://github.com/esphome/esphome/pull/18263)):
  [`components/aranet/aranet.cpp`](https://github.com/esphome/esphome/blob/dev/esphome/components/aranet/aranet.cpp).
- Home Assistant
  [`components/aranet/manifest.json`](https://github.com/home-assistant/core/blob/dev/homeassistant/components/aranet/manifest.json)
  and `config_flow.py` (`MIN_VERSION = 1.2.0`, `integrations_disabled`).
- The Aranet4 user manual v2.6 (`Aranet4_User_Manual_v24_WEB.pdf` from
  naltic.com), p. 5: default CO₂ thresholds and the fact that they can be
  changed.
- Advertising cadence (weak sources): "about every 3 s … in passive mode
  too" ([deskair-aranet4 PR #1](https://github.com/romanshevchuk/deskair-aranet4/pull/1)),
  and "typically 4+ seconds" in a code comment in the `aranet-core` crate
  (`src/passive.rs`).
- Crate sources in `~/.cargo/registry/src/index.crates.io-*/`:
  `trouble-host-0.8.0` (`src/scan.rs`, `src/host.rs`, `src/connection.rs`,
  `Cargo.toml`), `nrf-sdc-0.4.0` (`src/sdc.rs`), `nrf-sdc-sys-0.3.0`
  (`build.rs`, `third_party/…/softdevice_controller/{README.rst,include/sdc.h,limitations.rst}`),
  `bt-hci-0.10.1` (`src/param/le.rs`).
- Nordic,
  [SoftDevice Controller scheduling](https://nrfconnectdocs.nordicsemi.com/ncs/latest/nrfxlib/softdevice_controller/doc/scheduling.html).
- My own measurements: release builds of a throwaway copy of this tree, with
  the prototype in `tmp/aranet-research/prototype.diff` (§2.4). The
  downloaded sources are in `tmp/aranet-research/web/`, and
  `tmp/aranet-research/scripts/decode.py` checks the offsets against the
  upstream vectors.

I did **not** scan from the laptop. Its adapter (`BC:F4:D4:81:DC:B4`) was
`Powered: no`, and the brief said not to power it on. §4.0 says how to
capture real bytes.

---

## 0. Decisions at a glance

| Question | Decision |
|---|---|
| Data source | Aranet4's own advertisement: manufacturer data for company `0x0702` (SAF Tehnika), 22 bytes, sent only with "Smart Home integrations" on and firmware ≥ v1.2.0. No connection and no pairing. |
| Scan type | **Passive.** The readings are in the `ADV_IND` itself. Flags plus the 26-byte manufacturer AD fill 29 of 31 bytes, so the name and service UUID must be in the scan response, which we skip. §4.0 confirms this with a capture. |
| Radio stack | Switch nrf-sdc to the **multirole** library (`central` + `peripheral` features), call `support_scan()`, set `central_count(0)`, and enable trouble-host `scan`. The peripheral-only library has no scanner. |
| Cost | Measured: **+37.7 KiB flash** and **+1.6 KiB static RAM** before the SDC's `Mem<N>` grows. The `Mem<N>` growth is about +0.6–0.7 KiB more (computed). That doesn't fit in today's ~2.1 KiB headroom. We pay for it by shrinking trouble-host's packet pool from 16 to 8 packets (**−2.0 KiB**, measured). Net: about +0.3 KiB RAM. |
| Concurrency | Scanning runs alongside the phone connection, the second slot's advertising, and a second connection. In SDC a scanner with window = interval has the lowest priority and yields to everything else (Nordic scheduling doc). Don't scan during DFU. |
| Where | A new **Air** screen in the swipe ring, next to the clock. The clock face gets nothing in v1. |
| What | Big CO₂ digits in the band colour, a 3-segment band bar (so it reads without colour), the band word, temperature and humidity, and age ("NOW", "4 MIN"). The sensor label goes at the top. |
| Which sensor | The **strongest RSSI** heard in the scan, unless a sensor is **pinned** (long press). Tap cycles through the others that were heard. Labels come from a compile-time table keyed by address, with an address-suffix fallback. |
| When to scan | Only when the Air screen opens and the cache can't answer: nothing cached, or the shown sensor's next measurement is due (known from `interval − age`). At most 4 s, and it stops early once the target is heard. Never on glance, never while the screen is off. |
| Energy | Estimated ~7–12 µAh per 4 s scan, so ~5 µA averaged over 10 looks a day. Opening the screen costs about 4× more than that. |

---

## 1. The Aranet4 advertisement

### 1.1 Prerequisites and firmware dependence

- **Smart Home integrations** must be on. Set it in the Aranet Home app, or
  with `aranetctl <mac> --set-integrations on` from Aranet4-Python. When it
  is off, the manufacturer data shrinks to 7 bytes of header with no
  readings, and flag bit 5 is clear (upstream vector `01 00 02 01 00 00 00`,
  v1.2.0).
- **Firmware ≥ v1.2.0.** Older firmware sends only the 7-byte header, even
  with the integrations bit set (upstream vector `21 0a 04 00 00 00 00`,
  v0.4.10). HA rejects such devices with `outdated_version`. v1.2.0 also
  moved the service UUID from the 128-bit `f0cd1400-95da-4f4b-9ac8-aa55d312af0c`
  to the 16-bit `0xFCE0` (`client.py`, `SERVICE_SAF_TEHNIKA*`). We don't
  need either UUID.
- **Model:** Aranet4 is the only Aranet model *without* a leading
  device-type byte. Aranet2 (`01`), Radiation (`02`), and Radon (`03`) put
  one first and are 24 bytes long. So the length tells them apart:
  ESPHome uses `22 → Aranet4`, `24 → other`, and Aranet4-Python uses the name
  or a length of 7/22.

### 1.2 Byte layout (Aranet4, 22 bytes after the company ID)

The offsets index the manufacturer-specific AD payload *after* the 2-byte
company ID `02 07`. That is also what btmon prints as `Data:`. All
multi-byte fields are little-endian.

| Off | Size | Field | Scaling and validity | Corroborated by |
|---|---|---|---|---|
| 0 | 1 | flags | bit 0 "disconnected" (meaning unclear, and set in the upstream vector), bit 1 unknown, bits 2–3 calibration state (0 none, 1 end requested, 2 in progress, 3 error), bit 4 DFU active, **bit 5 integrations**, bits 6–7 unknown | `ManufacturerData.decode`; `Aranet4.h` bitfield; ESPHome `data[0] & 0x20` |
| 1 | 1 | fw patch | | Python `Version(value[3], value[2], value[1])` |
| 2 | 1 | fw minor | | " |
| 3 | 1 | fw major | | " |
| 4 | 1 | hw revision | unused | `Aranet4.h` `hw_rev` |
| 5–6 | 2 | unknown | unused | `__unknown3` |
| 7 | 1 | "packing" | unused | `Aranet4.h` |
| **8** | 2 | **CO₂** | ppm; **invalid if bit 15 is set** (calibration "magic") | Python fmt `<xxxxxxxxxHHHBBBHH` with a 0 inserted first; `Aranet4.h` `data + 8`; ESPHome `data[8..9]` |
| **10** | 2 | **temperature** | raw/20 °C; **invalid if bit 14 is set**; tenths = (raw + 1)/2 | same three; Python `multiplier = 0.05` |
| 12 | 2 | pressure | raw/10 hPa; invalid if bit 15 is set | same |
| **14** | 1 | **humidity** | % (integer) | same |
| 15 | 1 | battery | % | same; ESPHome `trailer[0]` |
| **16** | 1 | **status colour** | 1 green, 2 yellow, 3 red (0 = "error" in Python's `Color` enum). This is the colour on the device's own display, so it follows thresholds the user changed in the app. | Python `Color(value[5])`; `Aranet4.h` `status` |
| 17 | 2 | interval | seconds between measurements (60/120/300/600) | all three |
| 19 | 2 | age | seconds since that measurement | all three |
| 21 | 1 | counter | increments per measurement; ESPHome dedups on it | `Aranet4.h` `counter`; ESPHome `trailer[6]`; Python ignores it |

The default bands are green < 1000 ppm, yellow 1000–1400, and red > 1400
(manual p. 5). They "can be reconfigured using the Aranet4 app", so we
prefer byte 16 and fall back to 1000/1400 only when it is 0.

### 1.3 Worked example (upstream test vector, v1.3.5)

```text
21 05 03 01 00 05 00 01 43 04 9f 01 8b 27 35 0c 02 3c 00 10 00 55
flags 0x21 (integrations) · v1.3.5 · CO₂ 0x0443 = 1091 ppm · T 0x019f = 415 → 20.75 °C
p 0x278b = 1012.3 hPa · RH 53 % · battery 12 % · status 2 (yellow) · interval 60 s · age 16 s · counter 0x55
```

The upstream test expects 1091, 20.8, 53, 1012.3, 12, 60, and 16;
`tmp/aranet-research/scripts/decode.py` reproduces them. A 20-byte
truncation of this vector must be rejected (upstream "short" case).

### 1.4 Advertisement vs scan response, cadence, identity

- **Passive is enough (inferred, two sources agree).**
  - Size: flags (3 B) plus the manufacturer AD (1 + 1 + 2 + 22 = 26 B) is
    29 of the 31 bytes of legacy advertising data. A 16-bit UUID list (4 B)
    or "Aranet4 XXXXX" (15 B) can't fit, so they must be in the scan
    response.
  - Aranet4-Python's parser comment says "Passive scan may return result
    with no name". The deskair author reports the readings arrive "in
    passive mode too".
  - HA's matcher is `connectable: false`, with manufacturer ID 1794 plus a
    service UUID. BlueZ merges the scan response, so this doesn't tell us
    where the UUID sits.
  - §4.0 confirms all this from the PDU type.
- **Name:** "Aranet4 " plus five characters (e.g. "Aranet4 12345"),
  presumably only in the scan response. With passive scanning we never see
  it, so we identify sensors by **address** (ESPHome does the same). The
  address is stable: Aranet4-Python and ESPHome configure devices by MAC.
  The address type (public or static random) is still unknown, and the
  capture records it.
- **Cadence:** one report measures ~3 s, and an unsourced comment says
  "4+ s". Neither is solid. The capture measures it. The design assumes
  ≤ 4 s, which sets the scan length (§3.4).
- **Measurement interval:** 1, 2, 5, or 10 min (default 5; manual p. 2).
  Readings only change once per interval, and `interval − age` says exactly
  when the next one lands. That is what makes caching work (§3.4).
- **"Nearby":** take the **maximum RSSI per sensor within one scan**. Fast
  fading swings single reports by about ±5–10 dB, and the max over 1–2
  reports is steadier than the latest. At the wrist, body shadowing adds a
  similar error, so treat sensors within about 3 dB as a tie and keep the
  one already shown.

---

## 2. Radio: what the crates need

### 2.1 Facts, verified from source

| Fact | Where |
|---|---|
| The **peripheral-only SDC library has no scanner**. "Scanner" is listed under Central-only and Multirole only. | `nrf-sdc-sys-0.3.0/third_party/…/softdevice_controller/README.rst` feature table |
| For nRF52, nrf-sdc-sys links `multirole` when both `peripheral` and `central` are on, `peripheral` with only `peripheral` (us today), and `central` with only `central`. | `nrf-sdc-sys-0.3.0/build.rs:283-295` |
| `Builder::support_scan()` exists only with nrf-sdc feature `central`, and `support_ext_scan()` only with both. Legacy scanning is enough here. | `nrf-sdc-0.4.0/src/sdc.rs:462-472` |
| `SDC_DEFAULT_CENTRAL_COUNT` is 1. In multirole, set **`central_count(0)`** explicitly, so no central link (~2.3 KiB with our 251-byte × 3 + 3 buffers) gets reserved. | `sdc.h:63`, `sdc.rs:333` |
| Scanner memory is `SDC_MEM_SCAN(n) = 408 + 104·n`. The default is n = 3 (720 B), and the minimum is 2 (616 B). With 2 buffers "the scan response report will likely not be generated", which doesn't matter for passive scanning. | `sdc.h:91, 246, 628-636` |
| SDC ignores `Filter_Duplicates`, so we get every advertisement. | `limitations.rst` DRGN-8628 |
| trouble-host's `scan` feature implies `central`. `Scanner::new(&mut central).scan(&ScanConfig)` sends legacy `LeSetScanParams` + `LeSetScanEnable` and returns a `ScanSession`. **Dropping or calling `stop()` disables scanning.** The legacy session ignores `ScanConfig::timeout` (only the extended `ScanSession<true>` is a `Future`), so we time the scan ourselves. | `trouble-host-0.8.0/Cargo.toml` (`scan = ["central"]`); `src/scan.rs` |
| `ScanConfig::default()` is **`active: true`**, 1 s interval and 1 s window. Set `active: false` explicitly. | `src/connection.rs:87-99` |
| Reports aren't returned by the session. They arrive through `EventHandler::on_adv_reports(LeAdvReportsIter)`, called by `Runner::run_with_handler(&handler)`. Today we call `runner.run()`. | `src/host.rs:1220-1225, 1292, 1464-1475` |
| The LE event mask already enables advertising reports. | `src/host.rs:1782-1792` |
| Scanning and advertising use **separate** host command slots (`scan_command_state` vs `advertise_command_state`), so a scan neither waits for nor cancels our `Advertiser`. `stack.central()` is a separate lightweight handle, so the `peripheral` mutex in `host_task` is untouched. | `src/host.rs:174-176, 2018-2033`; `src/lib.rs:921-933` |
| A report is `LeAdvReport { event_kind, addr_kind, addr: BdAddr, data: &[u8], rssi: i8 }`. Use `addr.raw()` (`&[u8]`); there's no array accessor. | `bt-hci-0.10.1/src/param/le.rs:620-628`, `src/param.rs:71-82` |

### 2.2 Scheduling against our connections and advertising

From Nordic's scheduling page:

- "The SoftDevice Controller supports running multiple connections,
  Advertisers, Scanners, and timeslot events simultaneously."
- A scanner with **scan window = scan interval** has the lowest BLE
  priority (4th of 5).
- The scanner "is scheduled cooperatively". After an interleaved event it
  "will always return to primary channel scanning".

So with window = interval, connection events (phone at 100–125 ms when
idle), the free slot's advertising (2 s while connected, 152.5/417.5 ms
while away), and MPSL flash timeslots all pre-empt the scanner. Nothing of
ours loses radio time. The scanner loses a few percent of its listening
time, which is harmless because each Aranet event repeats on all three
channels.

- **DFU:** at 7.5–15 ms connection intervals the scanner would get little
  time, and DFU must not be disturbed. Don't start a scan while
  `update.active()`; show "BUSY".
- **Not verified on hardware:** that the multirole library behaves
  identically for our peripheral use (pairing, DLE, reconnects, DFU). It is
  the same SDC release but a different binary. Regression-test it (§4.3).

### 2.3 Settings

```rust
ScanConfig {
    active: false,                         // readings are in ADV_IND; no SCAN_REQ from us
    interval: Duration::from_millis(100),  // window == interval: continuous, lowest priority
    window: Duration::from_millis(100),
    ..Default::default()                   // timeout is ignored for legacy scans anyway
}
```

Builder in `main.rs`:

```rust
b.support_adv().support_peripheral().support_scan()
    .peripheral_count(ble::CONNECTIONS as u8)
// then:
.and_then(|b| b.central_count(0))
.and_then(|b| b.scan_buffer_cfg(2))   // passive only; saves 104 B
```

Cargo features:

- `nrf-sdc`: add `central`.
- `trouble-host`: add `scan` and `default-packet-pool-size-8`.

Edit these by hand, since `cargo add` would only add features. No new
crates are needed.

Passive scanning transmits nothing. If active scanning is added later
(names, §5), our `SCAN_REQ`s would carry the same static random address we
already advertise all day, so there's no new tracking exposure.

### 2.4 Measured cost

These are release builds of a copy of this tree (`llvm-size -A`). The
sizes include the prototype in `tmp/aranet-research/prototype.diff`, which
compiles against the real crates. It has a scan branch in `host_task`, an
`EventHandler` that keeps raw payloads for 6 sensors, and a
`Signal<Duration>` request.

| Build | .text | .rodata | .data | .bss | Δ flash | Δ static RAM |
|---|---|---|---|---|---|---|
| Today (peripheral SDC) | 316 112 | 47 712 | 4 392 | 46 680 | — | — |
| Multirole SDC, `central_count(0)`, no scanner | 331 080 | 48 004 | 4 420 | 47 760 | +15.0 KiB | +1 108 B |
| … plus `support_scan()` | 342 104 | 48 004 | 4 436 | 47 760 | +25.7 KiB | +1 124 B |
| … plus trouble-host `scan` and prototype host code | 353 888 | 48 480 | 4 436 | 48 256 | **+37.7 KiB** | **+1 620 B** |
| … plus `default-packet-pool-size-8` | 353 896 | 48 480 | 2 420 | 48 256 | +35.7 KiB | **−396 B** |

Notes:

- The multirole library's extra ~1.1 KiB of `.bss` is the SDC's own state.
  It has no sized symbols, only `$d` markers, so it can't be trimmed.
- In the prototype, the host-side RAM was the table (184 B), the signal
  (16 B), and the `host_task` future (+296 B).
- **Still to add:** `Mem<N>` growth, `SDC_MEM_SCAN(2)` = 616 B by the
  header's macros. The exact N comes from the boot log:
  `Memory buffer too small. N bytes needed.` (`sdc.rs:785-795`). Today's
  `Mem<5184>` is already below what the header macros predict for our
  config, so trust the log, not the macros.
- **Budget:**
  - RAM limit 0xD400 = 54 272 B. Today: 52 096 B including `.uninit`, so
    2 176 B are free. The field-log build has less.
  - With scanning and pool 16: about 54 340 B. **Doesn't fit.**
  - With pool 8: about 52 320 B, leaving ~1.9 KiB free.
  - Flash: 368.4 KB of 474.4 KB today (22 % free). With scanning: ~405 KB
    (**~15 % free**), plus ~2–3 KiB for the UI (estimate).
- **Pool 8 risk:** 16 → 8 shared 251-byte host packets for 2 connections.
  DFU (write-without-response bursts) and field log fetches (notification
  bursts) are the users that could stall or slow down. Time a full DFU and
  a log fetch before and after (§4.3). Fallbacks, if pool 8 hurts:
  - SDC `buffer_cfg(…, 3, 2)` or `(…, 2, 2)`, which saves 2 × 266 B per
    count dropped (computed from `sdc.h:160-192`).
  - Plan B (§2.5).
- If BTHome (`docs/bthome-design.md`) lands too, it adds about 0.3 KiB
  more. Its "about 5 KiB left" figure is out of date (logged in
  SESSION.md).

### 2.5 Plan B: raw-radio observer in an MPSL timeslot (not recommended first)

We could keep the peripheral-only SDC and receive advertising PDUs
ourselves in an MPSL timeslot. We already open one session for flash,
`SessionMem<1>` → `<2>`.

- **Cost:** about 0.1 KiB RAM and a few KiB of flash.
- **But:**
  - nrf-mpsl 0.4 exposes timeslots only through its `Flash`, so we'd write
    raw `mpsl_timeslot_*` FFI and drive RADIO registers from the
    priority-0 signal callback (BLE 1M, access address `0x8E89BED6`, CRC
    init `0x555555`, whitening per channel).
  - Slots are ≤ 100 ms (`MPSL_TIMESLOT_LENGTH_MAX_US`) and need
    extensions.
  - It's a lot of unsafe code to own.

Only worth it if flash, not RAM, becomes the constraint.

### 2.6 Alternative considered: relay through Home Assistant

HA already decodes the Aranets, through BlueZ or ESPHome proxies. It could
push values to the watch over Gadgetbridge's intent path, at zero radio or
RAM cost on the watch. But it needs HA, the phone, and a proxy near each
sensor. It also loses "the one near me", which is the point here. Rejected
for v1. It is still a fine way to see *other* rooms' CO₂.

---

## 3. UX

### 3.1 Where

- Add `Screen::Air` to `ui.rs`, placed right after `Clock` in
  `App::screens()`: Clock → Air → Alarms → [Music] → Info. That's one swipe
  from the clock.
- `docs/bthome-design.md` also wants the slot after the clock for Home.
  Whichever lands second takes the next slot.
- The screen always exists. With nothing heard, it shows a hint (§3.3).
- **Not on the clock face in v1.** Showing a cached value there is cheap,
  but on a screen you glance at it would push toward scanning on every
  glance. TODO: a small CO₂ number and band pip on the clock while a
  reading is < 10 min old. It would read from the cache only and never
  trigger a scan.

### 3.2 Layout (80×80 canvas)

```text
 y 0..6     KITCHEN                 ((·))     label (TEXT_DIM); right: pulsing dot while scanning
 y 10..30   1 0 9 1                           CO₂, DIGITS at 3× (15×21 each, 66 px total), band colour, SHADOW
 y 33..37   PPM CO2                 FAIR      TEXT_DIM; band word in band colour (GOOD/FAIR/POOR)
 y 41..44   [=====][=====][=====]             3 segments; the lit one is the band, others TRACK
 y 50..59   20.8°   53%                       small font at 2× (TEXT); "°" is a new 3×3 sprite
 y 74..78   4 MIN                    2/3      age (TEXT_FAINT); index among sensors heard
```

- **Band colours:** add roles `AIR_GOOD`, `AIR_FAIR`, and `AIR_POOR`.
  `Color::COUNT` goes from 13 to 16, the maximum for 4-bit roles, so this
  uses up the palette. Cheaper alternative: reuse `BAD` for red and add
  only `AIR_FAIR`, but `GOOD` is white in ARCADE, so that's no better.
  - Suggested values:
    - ARCADE: `0x40c040`, `0xe8b020`, `0xd82828`
    - PARCHMENT: `0x4a6a2a`, `0xb07818`, `0xa02820`
    - HANDHELD and TERMINAL, which are monochrome by design: three
      intensities of their single hue
  - Because of the monochrome themes, the bar segment's **position** and
    the word carry the meaning, and colour is a bonus.
- **Bands:** byte 16 (1/2/3) when it's non-zero, otherwise 1000/1400 ppm.
- **Invalid fields:** if CO₂ is invalid (bit 15, calibrating), show `----`
  and "CAL". If temperature is invalid, show `--.-°`.
- **Staleness:** `age_now = adv_age + (now − received_at)`.
  - Under 60 s: "NOW". Then "N MIN", then "OLD" after 60 min.
  - Once `age_now > 2·interval + 60 s` (the sensor missed measurements, or
    we're out of range), draw the digits in `TEXT_FAINT`.
- **Sensor battery:** when the sensor's battery is ≤ 15 %, replace the age
  with "BAT 12%" every other second.

### 3.3 States

| State | Shows |
|---|---|
| Scanning, nothing cached | "LOOKING" centred, plus the pulsing dot |
| Nothing heard after the scan | "NO ARANET NEARBY", then "TAP TO RETRY" |
| An Aranet4 heard, but 7-byte payload with bit 5 clear | "TURN ON SMART HOME" / "IN ARANET APP" |
| 7-byte payload with bit 5 set (firmware < 1.2) | "UPDATE ARANET" / "FIRMWARE" |
| Pinned sensor not heard, but others were | The best other one, with the label prefixed "≠" (TODO: pick a glyph), so the user knows the pin didn't answer |
| DFU active | "BUSY" |

### 3.4 Which sensor, and when to scan

**Selection:**

- If a sensor is **pinned** and was heard in the last scan, show it.
  Otherwise show the **strongest max-RSSI** sensor from the last scan,
  keeping the one currently shown when it is within 3 dB.
- **Tap:** show the next sensor heard, in RSSI order. If the cache can't
  answer for it (rule below), it also starts a scan.
- **Long press:** pin or unpin the shown sensor. Buzz once to pin, twice to
  unpin. This shadows the global theme long press, as Alarms already does.
  The pin is stored as `storage::key::AIR_PINNED = 22`, holding the address
  and its type (7 bytes).
- **Labels:** a compile-time table
  `const SENSORS: &[([u8; 6], &str)] = &[([…], "KITCHEN"), …]`, filled in
  from the capture (§4.0). Unknown sensors show "ARANET 12AB" (the last two
  address bytes). TODO: labels pushed from the phone, as BTHome plans.

**Scanning, the cache-first rule:**

- On entering Air while awake, **scan only if** one of these holds:
  - nothing is cached;
  - for the sensor we'd show, `now ≥ received_at + (interval − age)`, so a
    newer measurement exists;
  - the cache is older than 10 min. This catches walking into another room
    while the old room's sensor is still on a long interval.
- Otherwise draw from the cache immediately and don't scan. With the
  default 5 min interval, most repeat looks within a few minutes cost no
  radio at all.
- **Length:** at most **4 s** (≥ one advertising period, §1.4). Stop early:
  - when pinned: as soon as the pinned sensor is heard with a new counter;
  - when nearest: once every sensor in `SENSORS` (and every one seen
    before) has been heard, but not before 1 s.

  The screen timeout is 10 s, so a scan always finishes on screen.
- **Never** while the screen is off, on a glance, or during DFU. TODO, as a
  `const` switch, off by default: scan on glance. That would cost ~35 µA at
  60 glances a day.
- The cache lives in RAM only: 6 entries holding address, type, max RSSI,
  the parsed reading, `received_at`, and the counter. A reboot just
  rescans.

### 3.5 Energy (estimate)

- **RX:** about 6.5 mA with the DC/DC regulator, ~11 mA on the LDO (we're
  on the LDO; see SESSION.md and power-plan action 3). That's from the
  datasheet, quoted from memory, and includes HFXO and report handling.
- **Per scan:** 4 s ≈ 26–44 mC ≈ **7–12 µAh**.
- **Per day:** 10 scans ≈ 0.1 mAh/day ≈ **5 µA** average, about 1.5 % of
  the power plan's ~310 µA target. Cache hits cost 0.
- **Comparison:** opening the screen at all costs ~10 s × ~11 mA
  (panel + backlight) ≈ 110 mC, about 3× the scan.
- **Report load:** with ~100 advertisers in a house, the host sees on the
  order of 100–300 reports/s while scanning. `on_adv_reports` has to
  reject most of them cheaply: walk the AD, compare the company ID, and
  never log per report.
- **Metering:** add `power::Meter::Scanning` and `power::Count::Scans`, so
  the field log shows the real numbers.

---

## 4. Implementation plan

Each step leaves the firmware working and is committed on its own.

### 4.0 Capture real advertisements (laptop, by you)

The adapter was off, so I didn't capture anything. To do it:

```sh
bluetoothctl power on                        # you, explicitly
sudo btmon -T | tee tmp/aranet-btmon.txt     # terminal 1: all HCI traffic, with time and date
bluetoothctl --timeout 60 scan le            # terminal 2: BlueZ discovery (active scan)
uvx --from aranet4 aranetctl --scan          # terminal 3, same minute: reference decode
                                             # (or: pipx run --spec aranet4 aranetctl --scan)
bluetoothctl power off                       # when done
grep -n -B16 -A4 '(1794)' tmp/aranet-btmon.txt
```

For each sensor, note the following. Paste the hex lines with
`aranetctl`'s decoded values into `host-tests/aranet/vectors.txt` (§4.1).

- **Which PDU carries `Company: … (1794)`.** btmon prints either
  `Event type: … ADV_IND` (legacy reports) or
  `Legacy PDU Type: ADV_IND (0x0013)` (extended reports, which newer
  kernels use). The design assumes `ADV_IND`, not `SCAN_RSP`. If it's
  `SCAN_RSP`, we need active scanning and 3 scan buffers (+104 B). That is
  a small change; §2.3's `active` flag covers it.
- **Where the name `Aranet4 XXXXX` comes from:** expected to be `SCAN_RSP`.
- **`Address:` and `Address type:`** (public or random), for the `SENSORS`
  table and the pin.
- **The advertising period:** the time between successive `ADV_IND`s from
  one address over a minute. If it's well over 4 s, raise the scan length
  in §3.4.
- **The `Data:` hex** (the 22 bytes after the company ID) and `RSSI:`.
- **One sensor with integrations off** (7-byte case) and, if you can, one
  mid-calibration (CO₂ bit 15).

### 4.1 Pure decoder plus host tests (no radio)

- **`src/aranet.rs`, pure:** no embassy, defmt only via
  `cfg_attr(target_os = "none", …)` as in `alarm.rs`, and time as `u64`
  milliseconds so the host can test it.
  - `fn manufacturer_payload(ad: &[u8]) -> Option<&[u8]>`: walk the AD
    structures. Stop on length 0 or truncation. Match type `0xFF` with
    company `[0x02, 0x07]`.
  - `fn parse(payload: &[u8]) -> Result<Reading, Skip>`:
    - `Skip::IntegrationsOff`
    - `Skip::OldFirmware { major, minor, patch }`
    - `Skip::OtherModel(u8)` (24-byte payloads; TODO: Aranet2
      temperature and humidity)
    - `Skip::Malformed`
    - `Reading` has `co2: Option<u16>`, `tenths_c: Option<i16>`,
      `humidity: u8`, `pressure_dhpa: Option<u16>`, `battery`,
      `band: Band`, `interval_s`, `age_s`, `counter`, `calibrating: bool`.
  - `struct Sightings<const N: usize>`:
    - `update(addr, kind, rssi, reading, now)`: keeps the max RSSI within
      a scan, dedups on counter, and evicts the oldest entry when full.
    - `begin_scan(now)`
    - `choose(pinned, shown) -> Option<usize>` with the 3 dB hysteresis
    - `needs_scan(index, now) -> bool` (the §3.4 rule)
    - `age_now`
- **`host-tests/aranet/`:** copy `host-tests/alarm/`'s shape, with
  `#[path = "../../../src/aranet.rs"]`, `[workspace]`, and no deps.
  Tests:
  - The upstream vectors (§1.3; full, short, old firmware, integrations
    off, bad), plus the Aranet2, Radiation, and Radon vectors as
    `OtherModel`.
  - Invalid-bit cases, built by setting bit 15 or bit 14 in the upstream
    vector.
  - Temperature rounding: raw 415 → 208 tenths.
  - Status 0 → threshold fallback at 999/1000/1400/1401.
  - The AD walker on zero-length, truncated, foreign-company, and
    too-short-for-company-ID input.
  - `Sightings`: max RSSI, counter dedup, eviction, pinned vs nearest,
    hysteresis, and `needs_scan` around `interval − age`.
  - Every vector you captured in §4.0, checked against `aranetctl`'s
    numbers.
  - `quickcheck` (`cargo add --dev quickcheck`, outside the sandbox): the
    walker and `parse` never panic on arbitrary bytes, and anything
    `parse` accepts round-trips its fields.
- **Test:** `cd host-tests/aranet && cargo test`. `cargo build --release`
  for the firmware is unchanged because nothing calls the module yet. Use
  `#[allow(dead_code)]` until step 4.3.

### 4.2 Switch the controller to multirole (no scanning yet)

- Change the Cargo features as in §2.3: nrf-sdc `central`, trouble-host
  `scan` and `default-packet-pool-size-8`.
- `main.rs`: `support_scan()`, `central_count(0)`, `scan_buffer_cfg(2)`.
- Boot with the probe and read `Memory buffer too small. N bytes needed.`.
  Set `Mem<N>` to N, and add a comment noting that the multirole library
  and the scanner raised it. Reboot and confirm there's no "too big"
  warning.
- **Regression tests:**
  - The phone reconnects after a reboot.
  - Pair a fresh bond with the passkey.
  - Notifications work.
  - A field log fetch works while the phone is connected.
  - A full DFU (and its rollback-confirm) works.
  - Time the DFU and the log fetch against the same steps on today's
    build (pool 8 vs 16). If DFU is more than ~10 % slower or stalls, try
    pool 16 with `buffer_cfg(…, 3, 2)` instead.
- Record `llvm-size -A` in the commit message, both for the normal build
  and with `--features field-log`.

### 4.3 Scan plumbing in `ble.rs`, triggered by hand

- `runner.run()` → `runner.run_with_handler(&AIR_REPORTS)`.
  `AIR_REPORTS: impl EventHandler` does the following for each report:
  1. `aranet::manufacturer_payload`, then `aranet::parse`.
  2. `SIGHTINGS.lock(|s| s.borrow_mut().update(…))`.

  `SIGHTINGS` is a
  `blocking_mutex::Mutex<CriticalSectionRawMutex, RefCell<Sightings<6>>>`.
  Also record `Skip` kinds per address, for the hint states in §3.3.
- `pub static AIR_SCAN: Signal<CriticalSectionRawMutex, ScanRequest>` and
  `pub static SCANNING: AtomicBool`. `ScanRequest` carries the max length
  and the stop-early target (pinned address, or "all known").
- Add a 4th future to `host_task`'s `join` (`join3` → `join4`) rather than
  a new task, because the arena would cost the same RAM. The loop:
  1. Wait for a request.
  2. Skip it if DFU is active.
  3. `Scanner::new(&mut central).scan(&config)`.
  4. Poll every 250 ms until the deadline or the stop-early condition.
  5. `session.stop().await`.
  6. Meter the time.

  The prototype's version is in `tmp/aranet-research/prototype.diff`.
- **Temporary trigger:** tap on Info. Today that only buzzes (`ui.rs`
  `(Gesture::Tap, Screen::Info)`). Log each new reading as
  `info!("air: {=[u8]:x} rssi {} co2 {} t {} rh {} age {}/{}", …)`.
- **On-device tests:**
  - With `aranetctl --scan` running on the laptop at the same time, the
    numbers match.
  - It works in each radio state:
    - phone connected (idle, 100–125 ms);
    - phone away (fast advertising at 152.5 ms);
    - during a field log fetch.
  - No `ble:` disconnects or supervision timeouts in the log across
    20 scans.
  - Advertising keeps going: a second client can still connect mid-scan.
  - With DFU running, the scan is refused.
  - Stop-early works: a pinned scan ends in about one advertising period.

### 4.4 The Air screen

- `ui.rs`:
  - `Screen::Air` in `screens()`.
  - `render_air` (§3.2–3.3).
  - Gestures (§3.4), with the long-press arm placed before the global
    theme arm.
  - On `go_to(Screen::Air)` and on tap, call `needs_scan`, then
    `ble::AIR_SCAN.signal(…)`.
  - The cached sighting goes into `Status` (a small `Copy` snapshot taken
    in `main`'s loop, the way `passkey` is), so `render` stays a pure
    function of `App` + `Status`.
- `theme.rs`: three roles, with values for all five themes. `GLANCE` needs
  them too but never uses them.
- `font.rs`: a degree sprite.
- `storage.rs`: `key::AIR_PINNED = 22`. It's loaded at boot into
  `App::new` like `alarms`, and saved when it changes.
- `aranet.rs`: the `SENSORS` label table from §4.0. Decide whether real
  addresses belong in the repo. They're low-sensitivity, but an untracked
  include works too.
- Remove the temporary Info trigger.
- **Device tests:**
  - Every state in §3.3. The "TURN ON SMART HOME" state is reproducible by
    switching integrations off on one sensor with `aranetctl`.
  - Each theme, including the monochrome ones, still reads by the bar
    position.
  - Walk between rooms: the nearest sensor wins, and the pin overrides
    it.
  - A repeat look within the interval doesn't scan (log line).
  - Watch timeout mid-scan: the scan still stops at its deadline.

### 4.5 Power check

After a normal day, read the field log's power meters. Expect `Scanning`
to be ≤ 60 s/day at 10 looks, and the `Scans` count to be well below the
number of Air openings, thanks to cache hits. Add a row to
`docs/power-plan.md`'s budget table.

---

## 5. Open questions and TODOs

- The capture (§4.0) settles four things: `ADV_IND` vs `SCAN_RSP`, the
  advertising period, the address type, and real vectors. Everything in
  §1.4 marked "inferred" depends on it.
- Whether `central_count(0)` is accepted by the multirole library. If
  `build` returns `EINVAL`, drop it and see what `Mem<N>` the log asks for.
- Aranet2 (temperature/humidity only, 24 bytes, type `01`): an easy
  follow-up once the Aranet4 path works.
- Names via active scanning in a "list sensors" mode (needs 3 scan buffers,
  +104 B).
- Use the controller's filter accept list (8 entries, `SDC_DEFAULT_FAL_SIZE`)
  with the known sensors' addresses. The host would then only see Aranet
  reports, so less CPU per scan. This needs the address type from §4.0.
  Trouble-host sets the list for us when `ScanConfig::filter_accept_list`
  is non-empty.
- A clock-face complication from the cache (§3.1).
