# Design sketch: watch taps → Home Assistant via BTHome v2

Status: built for 0.10 (2026-11-08), as the button grid, with the
changes below; the rest of this document is the original plan, kept for
its research (the radio, Home Assistant's parser, encryption). Written
2026-11-07 against the tree at `23ed848`; line references have drifted.

## What was built, and how it differs

User documentation: [buttons.md](buttons.md). Code: `src/buttons.rs`
(gestures), `src/bthome.rs` (payload, encryption, key and counter),
`ble::burst` (the radio).

- **Trigger:** not a 2×2 Home screen of labelled tiles, but a 3×3 grid
  of unlabelled buttons, the first screen below the clock (turned on in
  a new Settings screen), making 90 codes: 1–9, and pairs a→b as 10a+b,
  each tapped or held, and pairs double-tapped too (261 events; see
  `buttons.rs`). No swipe dimmer, no labels, no setup page on the title:
  the key is on Settings → HA KEY.
- **Payload:** §2's four button slots can't carry 90 codes (Home
  Assistant tells repeated objects apart only by position), and a
  button object has no number. So: packet id, battery, one **button**
  object for the gesture (`press`, `double_press`, `hold_press`, and
  `hold_release` on the lift), and three **dimmer** objects, one per
  gesture (`dimmer_1` tap, `dimmer_2` double tap, `dimmer_3` hold), the
  active one turning right by the code. 15 B of plaintext, 31 B
  encrypted. Checked against `bthome-ble`'s parser
  (`host-tests/bthome/check.py`). The button came in a rework, below.
- **Radio:** not a separate non-connectable burst. The watch's two
  connection slots mean one is nearly always advertising (connectably,
  even with the phone connected), so a burst swaps that advertising's
  data for the BTHome payload (name in the scan response) at 20 ms for
  1.2 s. No stop/start dance around connections, and a phone can still
  connect mid-burst. If both slots are serving (phone plus an update or
  log fetch), nothing advertises, and a burst is lost (a TODO in
  `ble::burst`).
- **Encryption:** as planned (key from the RNG at first boot, counter
  from 256, reserved 64 at a time in flash; storage keys 31 and 32), but
  CCM is hand-rolled over the already-linked `aes` 0.8 (four block
  encryptions), checked against bthome.io's vector in
  `host-tests/bthome`, rather than the `ccm` crate.
- **Also:** the same presses can go to the phone instead (or as well),
  as a Gadgetbridge `intent` message (`ur.watch.BUTTON`, extras
  `button` and `gesture`), for Tasker: the BUTTONS setting picks either,
  or both.
- **Feedback:** the last button shows the code for 0.8 s, with a buzz;
  red and two buzzes if it could go nowhere. No "SENT" text.

Sources: `tmp/ha-research/` (REPORT.md, `web/bthome_format.txt`,
`web/bthome_encryption.txt`, `web/ha_bthome.txt`, the Gadgetbridge sources),
the crate sources in `~/.cargo/registry/src/index.crates.io-*/`
(`trouble-host-0.8.0`, `nrf-sdc-0.4.0`, `nrf-sdc-sys-0.3.0`), and HA's
receiver code read on GitHub on 2026-11-07: `Bluetooth-Devices/bthome-ble`
`parser.py`, and `home-assistant/core` `components/bthome/{__init__,event,const}.py`.
`docs/bthome_vectors.py` checks the published encryption test vector and
produced every example payload below.

## The button-event rework (2026-11-08)

The user asked for BTHome's own button events (press, double_press,
long_press, and hold_press/hold_release for a hold's start and end)
rather than gestures as which dimmer turns. What the format and Home
Assistant allow, as read on 2026-11-08:

- **The button object** (`0x3A`, one byte) has the events `00` none,
  `01` press, `02` double_press, `03` triple_press, `04` long_press,
  `05` long_double_press, `06` long_triple_press, `80` hold_press, `81`
  hold_release, and nothing else: no number ([bthome.io/format]). The
  page gives no meaning beyond the names; `81` is "released after a
  `hold_press`" ([bthome-ble#423], [#430]). The only event objects that
  carry a number are the dimmer (`3C`, steps, `uint8`) and the new
  command (`3B`: on, off, toggle, step up/down, with manufacturer
  arguments, which Home Assistant shows as hex).
- **Home Assistant** (`homeassistant/components/bthome` at 2026.11.0,
  `bthome-ble` 3.25.0): every event fires `bthome_ble_event` with
  `device_id`, `address`, `event_class` (`button`, or `button_N` when an
  advertisement has more than one; likewise `dimmer`), `event_type`, and
  `event_properties` (`{steps: n}` for a dimmer, none for a button).
  Each class becomes an `event` entity ("Button", "Dimmer 2"; the
  properties become state attributes) and device triggers, one per
  class and type, listed once the class has been seen
  (`known_events` in the config entry; `device_trigger.py`). A device
  trigger matches `event_class` and `event_type` only, never the
  properties. `hold_release` arrived in 2026.11.0 ([core#188847]);
  older versions parse `81` as no event, so ignore it.
- **The local name** is no help: `bthome-ble` uses it only for the
  device's name and manufacturer (`_set_manufacture_name_type_and_title`),
  never in event data or triggers, and Home Assistant passes it to
  `async_get_or_create` with every event, so a name that changed per
  press would rename the device each time (shown unless the user renamed
  it). It also costs bytes the advertisement hasn't got, and in a burst
  it's in the scan response, which a passive scanner never asks for.
- **The budget:** 31 B, less flags (3), the service data's length, type
  and UUID (4), the device info byte (1), the counter (4) and the MIC
  (4): **15 B of objects**. The packet id (2) is what makes Home
  Assistant drop a burst's repeats within 4 s (`bthome-ble` drops an
  encrypted copy that's identical *after* 4 s, and no encrypted counter
  check catches an equal counter), so 13 B remain: six buttons, or five
  and the battery. Flags can't go: the burst rides the connectable
  advertising, which needs them, and BlueZ wants them for BTHome.
- **Extended advertising:** the SoftDevice Controller has it only in the
  multirole library, which the Aranet4 scanning now links, but
  trouble-host 0.8 needs its `extended-advertising` feature, and can't
  run a second set beside the connectable one (§5.3). The receivers
  decide it anyway: ESPHome's Bluetooth proxies scan with the legacy
  API only (`esp32_ble_tracker.cpp`: `esp_ble_gap_start_scanning`,
  `ESP_GAP_BLE_SCAN_RESULT_EVT`), so they'd never hear it, and they're
  how most Home Assistant installs reach Bluetooth.

So one button object per grid button (nine, 18 B) doesn't fit an
encrypted legacy advertisement, let alone one per code. The options:

| Option | Verdict |
|---|---|
| (a) `button_1`…`button_9`, gestures as event types; a sequence as two button events, or its first button some other way | 18 B; at most six fit. Rejected. Unencrypted would fit (23 B), but gives up spoofing protection and the stale-copy guard; not without the user's say. |
| (b) one button for the gesture, one dimmer for the code | Fits (9 B), but the code and the gesture are two events, and an automation's trigger matches one: "2, then 3, tapped" would need a dimmer trigger plus a condition on the button entity's state, which only works in that order (the button fires first). Worse than today for the common case. |
| (c) extended advertising, nine buttons | Proxies can't hear it. Rejected. |
| (d) buttons for singles, dimmers for sequences | Singles need nine buttons. Rejected. |
| **(e) today's per-gesture dimmers, plus one button for the gesture** | **Chosen.** 15 B exactly. The dimmer event still carries code and gesture together (one trigger; existing automations unchanged), and the button gives Home Assistant's own gesture names, as device triggers in the UI, and the release, which needs no code (one hold at a time). Costs a second event per press in the logbook. |
| (f) the code in the local name | No use (above). |

A hold's start is `hold_press`, sent as the hold is recognised, finger
still down, as before (the dimmer is `dimmer_3`); lifting sends
`hold_release` (no dimmer). `long_press` isn't sent: `hold_release`
pairs with `hold_press` ([bthome-ble#423]), and a third event for the
same hold would only duplicate one of them. The release's advertisement
mustn't cut the hold's short: `ble::burst` now gives a burst 0.8 s on
the air before the next replaces it (which also fixed the first of two
presses completed by one touch being lost).

Sources: [bthome.io/format]; `bthome-ble` 3.25.0 (`event.py`,
`parser.py`) from PyPI; `home-assistant/core` 2026.11.0 and `dev`
(`components/bthome/{event,device_trigger,const,__init__,logbook}.py`,
`strings.json`; `components/event/{entity,trigger}.py`;
`helpers/device_registry.py`); `habluetooth` 7.1.2 (`manager.py`:
it drops an advertisement identical to the last one heard); ESPHome
`dev` (`components/esp32_ble_tracker/esp32_ble_tracker.cpp`). Saved
under `tmp/research/` in the worktree that made the change.

[bthome.io/format]: https://bthome.io/format/
[bthome-ble#423]: https://github.com/Bluetooth-Devices/bthome-ble/issues/423
[#430]: https://github.com/Bluetooth-Devices/bthome-ble/pull/430
[core#188847]: https://github.com/home-assistant/core/pull/188847

---

## 0. Decisions at a glance

| Question | Decision |
|---|---|
| Trigger | A new **Home** screen in the swipe ring (Clock → Home → [Music] → Info) with 2×2 labelled tiles. Tap = `press`, long press = `long_press`, swipe up/down = dimmer `rotate_right`/`rotate_left`. |
| Labels | Compile-time `const` for now. Later, HA can push them over the existing Gadgetbridge intent path, with no phone app of our own. |
| Payload | One legacy `ADV_NONCONN_IND`: flags plus BTHome service data with packet id, battery, **always 4 button slots** and **1 dimmer slot**. That is 23 B in plaintext and exactly **31 B encrypted**. |
| HA side | Discovered by the stock BTHome integration. It creates `event` entities `button_1`…`button_4` and `dimmer`, and fires `bthome_ble_event` on the bus. |
| Encryption | Encrypted by default (device info byte `0x45`). The key is generated on the watch from the RNG at first boot, stored in flash, and shown on a setup page for typing into HA. The counter starts at 256 and the watch reserves blocks of 64 in flash. |
| Radio | Stays on **legacy advertising with one advertising set** (verified: our SDC library has no extended advertising). While connected, a burst runs alongside the connection. While disconnected, the connectable advertising pauses for the burst (about 1.2 s) and then resumes. |
| Burst | 20–30 ms interval for 1.2 s, about 45 events. This stays well under HA's 4 s duplicate window. It costs roughly 0.3 µAh per press. |
| Feedback | A buzz plus a tile flash plus the word "SENT", which only means the watch transmitted. No ack from HA (see 6.2). |
| Cost | About 0.3 KiB of static RAM if sealing runs on the stack. About 5–10 KiB of flash (estimate) because `aes 0.8` is already linked through trouble-host's `security` feature. |

---

## 1. UX

### 1.1 Where it lives

- Add `Screen::Home` to `ui.rs`, placed right after `Clock` in
  `App::screens()`. The ring becomes Clock → Home → [Music] → Info. From a
  sleeping watch, the path is button (wake) → swipe left → tap. The waking
  touch is swallowed by design (`ui.rs` header), so we wake with the button
  or a tap, then swipe.
- The screen exists once BTHome is set up, meaning a key is stored (§4.2).
  Before that, it shows a one-screen hint ("TAP TITLE TO SET UP").
- Home does not depend on the phone. BTHome is one-way broadcast, so it works
  whether or not Gadgetbridge is connected. It needs an HA Bluetooth adapter
  or ESPHome proxy within range.

### 1.2 Layout (80×80 canvas, 3×5 capitals font)

```
 y 0..9    "HOME"            ack text ("SENT", "NOT SENT")
 y 10..44  [ tile 1 ][ tile 2 ]     each 40×35 canvas px ≈ 120×105 panel px
 y 45..79  [ tile 3 ][ tile 4 ]     (≈13 mm targets; 1 px gutters)
```

- Labels fit 9 characters per line at 1× (9·4−1 = 35 px), and two lines fit
  per tile. The font is capitals plus Latin-1 accents.
- Use the existing theme colours: `ACCENT` for the frame and `TEXT` for the
  label. A pressed tile inverts for about 300 ms.

### 1.3 Gestures on Home

| Gesture | Where | Effect |
|---|---|---|
| Tap | tile *i* | button *i* `press` (`3A 01`) |
| Long press (≥600 ms, reported on lift) | tile *i* | button *i* `long_press` (`3A 04`) |
| Swipe up | anywhere | dimmer `rotate_right`, 1 step (`3C 02 01`) |
| Swipe down | anywhere | dimmer `rotate_left`, 1 step (`3C 01 01`) |
| Swipe left/right | anywhere | navigate the ring (unchanged) |
| Tap | title bar | setup page: MAC and key (§4.2) |
| Button | — | back to Clock (unchanged) |

Notes:
- On Home, the global "long press = next theme" (`ui.rs`, the
  `(Gesture::LongPress, _)` arm) must be shadowed by a Home-specific arm that
  sits before it.
- `touch.rs` only classifies a touch when the finger lifts, so `hold_press`
  and `hold_release` (`3A 80/81`) aren't possible without streaming touch
  state. That is a TODO, along with double tap, which would delay every
  single tap.
- The dimmer step count is always 1. A TODO is to scale it with swipe length,
  which requires adding the travel distance to `touch::Touch`.
- The side button is not mapped. It already means home or screen off.

### 1.4 Labels

- **Now:** `const HOME_TILES: [&str; 4] = ["LIGHTS", "TV", "FAN", "SCENE"];`
  in a new `src/bthome.rs` (or `ui.rs`). A label is only a hint to the user.
  HA decides what each button does, so a stale label is cosmetic.
- **Later (no phone app needed):** HA pushes labels through the path that
  REPORT.md option 2 verified. An HA script calls
  `notify.mobile_app_<phone>` with `command_broadcast_intent` to
  Gadgetbridge's `com.banglejs.uart.tx`. Gadgetbridge writes the `line` extra
  to our UART RX verbatim (`gadgetbridge/BangleJSDeviceSupport.java:293-330`),
  but only if the device setting "Allow intents" (`PREF_DEVICE_INTENTS`) is
  on. For example, the watch would receive:
  `GB({"t":"ur_home","l1":"LIGHTS","l2":"TV","l3":"FAN","l4":"SCENE"})`.
  The flat keys `l1`…`l4` avoid needing array support in `gb::Object`.
  `gb::phone_event` would map the message to `PhoneEvent::HomeLabels` and
  save it under a new storage key, 4 × ≤18 B (two lines of 9).

---

## 2. Advertisement payloads

### 2.1 Structure

Legacy `ADV_NONCONN_IND`, AdvA = our static random address (FICR-derived,
`main.rs`). AD structures:

```
02 01 06                       Flags: LE General Discoverable, BR/EDR not supported
                               (BTHome requires these. BlueZ, and therefore HA,
                               won't parse BTHome without them; bthome_format.txt.)
LL 16 D2 FC <info> <payload>   Service Data, 16-bit UUID 0xFCD2
```

The device info byte combines bit 0 (encrypted), bit 2 (trigger-based: "irregular
interval, e.g. only when someone pushes a button") and bits 5–7 (version 2):
- `0x44` = unencrypted, trigger-based, v2
- `0x45` = encrypted, trigger-based, v2

Bit 2 makes HA treat us as a "sleepy device", so the entities don't go
unavailable between presses (`parser.py: is_sleepy_device`, stored in the
config entry as `CONF_SLEEPY_DEVICE`).

Objects must be in ascending id order (HA only logs a warning otherwise):

| Object | Bytes | Meaning |
|---|---|---|
| `00 pp` | 2 | packet id, `pp` = low byte of our counter (§2.3) |
| `01 bb` | 2 | battery % (optional; gives HA a battery sensor that updates on presses) |
| `3A ee` ×4 | 8 | buttons 1–4. The pressed one carries its event, the rest carry `00` (None, which fires nothing) |
| `3C ee ss` | 3 | dimmer: `00 00` none, `01 ss` rotate left by ss steps, `02 ss` rotate right |

**Why all slots go in every packet:** HA names repeated objects by their
position. If an object type occurs more than once, every occurrence gets a
suffix `_1`, `_2`, …, and if it occurs only once it gets no suffix
(`parser.py`, `postfix_counter`). Sending a single `3A` would therefore make
every tile "button". Keep the slot count fixed forever, or append new slots
only at the end.

### 2.2 Byte budget

The plaintext object block is 2 + 2 + 8 + 3 = **15 B**.

- Unencrypted: 3 (flags) + 2 (AD header) + 2 (UUID) + 1 (info) + 15 = **23 B**.
- Encrypted: the same + 4 (counter) + 4 (MIC) = **31 B**, exactly the legacy
  limit.

There is no room left for a local name. HA can still learn the name
"Bangle.js ur" from our connectable advertisements, which use the same
address. If we ever want more tiles, drop the battery object (+2 B), then the
dimmer (+3 B). The maximum is 6 buttons with a packet id, 7 without (15 B of objects; see the rework above).

### 2.3 Examples (generated and checked by `docs/bthome_vectors.py`)

Example address: on-air (LSB first, as `main.rs` builds `address`)
`9A 78 56 34 12 E5`, displayed as `E5:12:34:56:78:9A`.
Example key: `231d39c1d7cc1ab1aee224cd096db932` (the bthome.io example).

Tile 2 tapped, battery 82 %, packet id 7, without the dimmer slot (20 B):
```
02 01 06  10 16 d2 fc 44  00 07  01 52  3a 00 3a 01 3a 00 3a 00
```

The full layout, swipe up (dimmer right 1), packet id 8, unencrypted (23 B):
```
02 01 06  13 16 d2 fc 44  00 08 01 52 3a 00 3a 00 3a 00 3a 00 3c 02 01
```

The same plaintext encrypted with counter 0x101 (31 B):
```
02 01 06  1b 16 d2 fc 45  94 5a 5b c8 5e 8c e9 ea e8 e5 8c ec 75 60 c3  01 01 00 00  43 01 ea fa
                      info └────────────── ciphertext (15) ──────────────┘ └ counter LE ┘ └─ MIC ─┘
```

### 2.4 Packet id, repeats, and HA's duplicate filtering (verified in `parser.py`)

- Every advertising event in a burst carries identical bytes. HA's
  `_skip_old_or_duplicated_advertisement` drops packets whose id isn't
  1–63 ahead of the last one, mod 256. **That filter only applies within
  4 s** of the previous accepted advertisement. After 4 s, any id is
  accepted. Consequences:
  - A burst must be **well under 4 s**, otherwise HA could accept a late copy
    as a second press. We use 1.2 s.
  - The packet id doesn't need to survive reboots, because a reboot takes
    about 10 s with the bootloader. We still derive it from the persisted
    counter (`counter as u8`), so one piece of state drives both.
  - Pressing the same tile twice quickly gives two ids, so HA fires two
    events.
- With encryption, `_is_same_as_last` additionally drops a byte-identical
  encrypted payload that arrives after more than 4 s, once the key is
  verified. That covers BlueZ re-reporting cached service data. Unencrypted
  mode has no such guard and relies on HA's Bluetooth manager not
  re-dispatching unchanged data. This is assumed; see test T7.

---

## 3. What HA does with it

- **Discovery:** the stock BTHome integration (`web/ha_bthome.txt`) offers
  the device when it hears FCD2 service data. A sleepy device is only heard
  when used, so **press a tile to get it discovered**. For an encrypted
  device, the config flow asks for the 32-hex-character bindkey.
- **Entities** (`components/bthome/event.py`): one `event` entity per event
  class, created the first time each class is seen. Button classes get the
  event types `press, double_press, triple_press, long_press,
  long_double_press, long_triple_press, hold_press, hold_release`. The dimmer
  gets `rotate_left, rotate_right`, with `event_properties: {steps: n}`.
  Names come from `translation_key = "button_numbered"` with the number
  filled in ("Button 1"; the exact string is assumed from the translation
  key). Entity ids follow the device name HA picks; rename the device to
  "ur watch" so they become `event.ur_watch_button_1` and so on.
- **Bus event** `bthome_ble_event` (`const.py`), with data `{device_id,
  address, event_class: "button_1"|…|"dimmer", event_type: "press"|…,
  event_properties}`.
- The battery object becomes a battery sensor, updated only when a tile is
  pressed.
- **Security caveats** (verified in `parser.py`):
  - With a bindkey configured, plaintext adverts from the device are ignored
    (`downgrade_detected`).
  - A counter that goes down is rejected **only if the new value is ≥ 100**
    and the key has been verified. We therefore start the counter at 256, so
    the check is always active.
  - Counter state lives only in memory, so **after an HA restart an old
    recorded packet can be replayed once**, for example a press captured
    while you were away from home.
  - Conclusion: don't map tiles to locks, garage doors or alarms. This matches
    BTHome's own warning (`web/bthome_encryption.txt`).

### 3.1 Example automations

```yaml
# Event-entity style (shows up nicely in the UI and logbook).
- alias: "Watch tile 1: toggle living-room lights"
  triggers:
    - trigger: state
      entity_id: event.ur_watch_button_1
      not_from: unavailable          # don't fire when HA restarts
  conditions:
    - condition: template
      value_template: "{{ trigger.to_state.attributes.event_type == 'press' }}"
  actions:
    - action: light.toggle
      target: { entity_id: light.living_room }

# Bus-event style (no entity lookup; filter by address and class).
- alias: "Watch tile 1 long press: everything off"
  triggers:
    - trigger: event
      event_type: bthome_ble_event
      event_data:
        address: "E5:12:34:56:78:9A"
        event_class: button_1
        event_type: long_press
  actions:
    - action: light.turn_off
      target: { area_id: living_room }

# Swipes on Home as a dimmer.
- alias: "Watch swipe: dim living room"
  triggers:
    - trigger: event
      event_type: bthome_ble_event
      event_data: { address: "E5:12:34:56:78:9A", event_class: dimmer }
  actions:
    - action: light.turn_on
      target: { entity_id: light.living_room }
      data:
        brightness_step_pct: >-
          {{ (10 if trigger.event.data.event_type == 'rotate_right' else -10)
             * (trigger.event.data.event_properties.steps | int(1)) }}
    # Optional ack back to the watch (§6.2):
    - action: notify.mobile_app_pixel
      data:
        message: command_broadcast_intent
        data:
          intent_package_name: nodomain.freeyourgadget.gadgetbridge
          intent_action: com.banglejs.uart.tx
          intent_extras: 'line:GB({"t":"ur_ha","b":0}):String.urlencoded'
```

---

## 4. Encryption

### 4.1 AES-CCM details (verified against the published vector)

- Cipher: AES-128-CCM with a 4-byte MIC (M=4) and a 13-byte nonce (so L=2),
  and no associated data. The v1 `update(b"\x11")` header is gone in v2.
- Nonce = MAC (6, **display order, most significant byte first**) ‖ `D2 FC` ‖
  info byte ‖ counter (4, little-endian).
  - **Pitfall:** `main.rs` builds `address` least significant byte first (the
    on-air order trouble-host wants), so the nonce needs
    `address.reversed()`. HA builds its nonce from the colon-hex string
    (`bytes.fromhex(mac_readable.replace(":",""))`).
- Service data = `D2 FC` ‖ info ‖ ciphertext ‖ counter (LE) ‖ MIC.
- Info bit 1 would put the MAC inside the payload, for devices with rotating
  addresses. We don't need it, because our address is static, and it would
  cost 6 B we don't have.
- Known-answer test (bthome.io): key `231d39c1d7cc1ab1aee224cd096db932`, MAC
  `54:48:E6:8F:80:A5`, info `0x41`, counter `0x00112233`, plaintext
  `02ca09 03bf13` → `41 e445f3c9962b 33221100 6c7c4519`.
  `docs/bthome_vectors.py` reproduces this with Python's `AESCCM`.

### 4.2 Key provisioning

Options considered:
1. **Compile-time key** (`option_env!("UR_BTHOME_KEY")`, parsed by a const
   hex decoder). Simple, but the key ends up in every built image and DFU
   zip. It also ties HA's configuration to whoever built the firmware.
2. **Generated on the watch** (chosen). At boot, if storage holds no key,
   fill 16 bytes from the hardware RNG and save them.
   - This is easy because `main.rs` initialises `storage` *before* it hands
     `&'static mut Rng` to `nrf_sdc::Builder::build`. A
     `bthome::init(rng).await` fits between the two.
   - Enable the RNG's bias correction for this, if embassy-nrf doesn't by
     default (to check).
   - Storage survives reflashing and DFU (`storage.rs` header), so the key and
     HA's configuration survive firmware updates.
3. Phone-provided key (via a GB line). Possible later, but it adds no
   security over option 2 and needs a sender.

**Display:** tapping the Home title opens a setup page with the MAC on one
line and the key as 4 rows of 8 hex digits at 2×: 8·8−2 = 62 px wide and
about 56 px tall in total. The user types the key into HA's BTHome config
flow.
- Showing the key on demand rather than "once" is deliberate. Anyone holding
  the watch can already press the tiles, and a show-once key is
  unrecoverable if HA's config is lost.
- TODO: a "new key" action, such as a long press on the setup page with a
  confirmation step.

**Storage keys** (next free numbers when this lands, currently 21+; never
reuse retired numbers):
- `BTHOME_KEY` (16 B)
- `BTHOME_COUNTER_RESERVED` (4 B)
- later `HOME_LABELS` (≤ 72 B, which means raising `storage::BUFFER` from 64
  or splitting it into per-tile keys)

### 4.3 Replay counter persistence

- The counter is a `u32`, incremented for every *distinct* payload (not per
  advertising event, because repeats within a burst are byte-identical). A
  nonce is never reused with different plaintext as long as the counter never
  goes backwards. That is the property persistence must guarantee.
- **Block reservation:** flash stores `reserved`, an upper bound that is
  always ≥ any counter value already used.
  - At boot: `next = reserved` (or 256 if nothing is stored). This skips
    whatever remained of the last block.
  - Before using `next` when `next >= reserved`: write
    `reserved = next + 64` first, *then* transmit.
  - If the write fails, don't send. Show "NOT SENT" and log it.
- Wear and latency: one ~16 B `sequential-storage` item per 64 presses. That
  is negligible against 3 × 4 KiB pages at 10k erase cycles each. The
  occasional write adds a few ms to that one press, or about 85 ms when it
  triggers a page erase, which goes through the MPSL timeslot (`storage.rs`).
- Not using the `RETAINED` RAM block: it doesn't survive power loss, which is
  exactly when the counter matters.

### 4.4 Implementation

- `ccm = { version = "0.5", default-features = false }` plus
  `aes = "0.8"`. **Pin both.** `ccm 0.6.x` (newest is 0.6.1, August 2026)
  moved to `cipher 0.5`/`aes 0.9` and would link a second AES.
  - Type: `Ccm<Aes128, U4, U13>`, using `encrypt_in_place_detached(nonce,
    b"", &mut buf[..15]) -> tag`.
  - Install with `cargo add ccm@0.5 --no-default-features` and
    `cargo add aes@0.8`. `cargo add` must run outside the sandbox.
- Fallback if the `ccm` crate costs too much flash: hand-roll CCM for a
  single block (plaintext ≤ 16 B, no AAD). That is 2 AES blocks for CBC-MAC
  (B0, then the data block) plus 2 for CTR (A0 for the tag, A1 for the data),
  about 40 lines. The KAT in §4.1 guards it either way.
- **RAM:** build the cipher inside a non-`async` `fn seal(...)`, so the
  expanded key (a few hundred bytes for `aes` fixslice) lives on the 16 KiB
  stack. If it lived in a task future, it would take static RAM, of which we
  have about 5 KiB left. Don't keep an `Aes128` in a static.
- Run the §4.1 known-answer test on the device at boot in debug builds
  (`defmt::assert!`). There is no host test harness (`Cargo.toml: test =
  false`), and this checks the real target code path.

---

## 5. Radio

### 5.1 What the crates support, verified from source

| Fact | Where |
|---|---|
| `Peripheral::advertise` uses **legacy** HCI (`LeSetAdvParams/Data/ScanResponseData/Enable`) on handle 0, and `Advertisement::NonconnectableNonscannableUndirected` maps to `AdvKind::AdvNonconnInd`. | `trouble-host-0.8.0/src/peripheral.rs:56-141`, `advertise.rs` `From<Advertisement>` |
| Only **one advertise operation** can be active. `advertise()` waits until the previous `Advertiser` is gone (`request_operation(advertise_command_state)`). Dropping an un-accepted `Advertiser` cancels it, and the runner then sends `LeSetAdvEnable(false)`. | `peripheral.rs:73-77, 381-388`; `host.rs:~2005-2020` |
| The legacy path ignores `AdvertisementParameters::timeout` for the controller. Legacy `LeSetAdvEnable` has no duration, so a burst ends only when we drop the `Advertiser`. | `peripheral.rs:126-134` |
| `update_adv_data` swaps legacy advertising data while advertising. We use it when a second press lands mid-burst. | `peripheral.rs:148-175` |
| `advertise_ext` sits behind the `extended-advertising` feature, which is **off** for us because `trouble-host` has `default-features = false`. Each call reconfigures *all* sets from handle 0, cancelling disables *all* sets (`LeSetExtAdvEnable(false, &[])`), and it needs `HostResources` `ADV_SETS ≥ n`. We pass `1`. | `peripheral.rs:186-296`, `host.rs:~2009`, `lib.rs:661-671` |
| `nrf-sdc 0.4`'s `support_ext_adv()` exists only with **both** `central` and `peripheral` features (the multirole library). We enable only `peripheral`, so `softdevice_controller_peripheral` is linked. Nordic's feature table lists "Advertising Extensions" and "Periodic Advertising" as multirole-only. | `nrf-sdc-0.4.0/src/sdc.rs:449-453`; `nrf-sdc-sys-0.3.0/build.rs:285-292`; `…/softdevice_controller/README.rst` |
| `SDC_DEFAULT_ADV_COUNT` is 1. One more advertising set would cost `SDC_MEM_PER_ADV_SET(31)` = (5184 + 31·18)/10 = **574 B** of SDC memory. | `…/softdevice_controller/include/sdc.h:35, 260-271` |
| `Advertisement::is_valid` only requires the flags AD for *connectable* types, so the non-connectable payload passes as built. | `advertise.rs` `is_valid` |

### 5.2 Assumed, to be verified in the spike (step 0)

- A1. The SDC peripheral library accepts **legacy non-connectable
  advertising while the single peripheral connection is up.** The Bluetooth
  spec allows this state combination, but the SDC headers don't state it
  either way. If it fails, the fallback is to send bursts only while
  disconnected. That would be a real UX loss, because the phone is usually
  connected.
- A2. The controller accepts a 20 ms interval for `ADV_NONCONN_IND`. Bluetooth
  5.0 dropped the old 100 ms minimum for non-connectable advertising, and SDC
  implements Core 6.x.
- A3. MPSL interleaves advertising events with connection events (at
  30–50 ms intervals) without dropping the connection. Some advertising
  events will be skipped, which is fine. During DFU (7.5–15 ms intervals),
  just don't burst.
- A4. HA scanner duty cycles: a local BlueZ adapter scans often, while ESP32
  proxies run at ~10 % by default and share their radio with Wi-Fi. These
  numbers come from memory, not checked sources.

### 5.3 Why not extended advertising with two sets

Two sets would need the multirole SDC library (larger, size unmeasured),
`trouble-host/extended-advertising`, `adv_count(2)` (+574 B SDC RAM), and
`ADV_SETS = 2`. Even then, trouble-host 0.8 can't add a set next to a running
`Advertiser`: `advertise_ext` waits for the single command slot, and
cancelling stops every set. We would still have to stop and restart the
connectable set around each burst, which is what the legacy plan does for
free. Rejected. Revisit if trouble-host gains independent per-set control.

### 5.4 Plumbing in `ble.rs`

- A new `pub static BTHOME: Channel<CriticalSectionRawMutex, bthome::Event, 4>`
  is fed by the UI.
- `bthome::Event` is `Button { index: u8, kind: ButtonEvent } | Dimmer { right: bool, steps: u8 }`.
- `ble.rs` owns the radio side.

**Disconnected** (the `connections` loop in `host_task`):
```text
loop {
  select(advertise_and_accept(&mut peripheral, &server), BTHOME.receive()):
    conn  → serve(...) as today
    event → (dropping the first future drops the Advertiser: connectable ads stop)
            burst(&mut peripheral, event)        // ≤1.2 s, coalesces more events
            if let Some(conn) = peripheral.try_accept() { serve(conn) }   // race, below
}
```
- **Race:** a phone may connect in the instant we cancel. That connection then
  waits in trouble-host's connection manager. If we restarted connectable
  advertising with `peripheral_count = 1` already used, the SDC would
  probably refuse it. So check `try_accept()` before re-advertising
  (verified to exist: `peripheral.rs:346`).

**Connected** (`serve`):
- Add a 4th `select` arm for `BTHOME.receive()`, and pass `&mut peripheral`
  into `serve`.
- On an event, start the burst with `peripheral.advertise(&burst_params,
  NonconnectableNonscannableUndirected { adv_data })`, then store
  `(Advertiser, deadline)` in local state. A 5th arm, `Timer::at(deadline)`,
  drops it.
- GATT traffic keeps flowing because the burst is never awaited inline.
- Skip the burst while `update.active()` (DFU), and show "BUSY".

**Burst details:**
- `AdvertisementParameters { interval_min: 20 ms, interval_max: 30 ms,
  tx_power: ZerodBm, ..Default }`. Keep the host's default random address, so
  BTHome traffic and Gadgetbridge share one identity. No new tracking
  exposure: we already advertise "Bangle.js ur" from this address all day
  while disconnected.
- If a second event arrives mid-burst, re-seal it with the next counter, call
  `update_adv_data`, and extend the deadline to 1.2 s after the new event.
- Each payload carries **only the latest** event, so if HA missed every copy
  of the first one, that press is lost. This is rare with 1.2 s bursts, and
  acceptable.
- Log every sealed advert with `info!("bthome: {=[u8]:x}", adv)` for testing.

### 5.5 Burst length and battery

- **Reliability:** each advertising event goes out on all three channels
  within about 1 ms. A scanner with duty cycle *d* catches a given event with
  probability ≈ *d*, and advDelay (0–10 ms) decorrelates successive events.
  With N ≈ 1.2 s / 25 ms ≈ 48 events, the chance of missing the whole burst
  is (1 − d)^N:
  - d = 10 %: 0.6 %
  - d = 50 %: ~0

  Median latency is a few events, under about 250 ms even at d = 10 %.
  Keeping 1.2 s also leaves a 2.8 s margin under HA's 4 s window (§2.4).
- **Energy** (estimate; datasheet figures from memory):
  - One event = 3 × (about 140 µs ramp-up + 47 B × 8 µs = 376 µs airtime)
    ≈ 1.5 ms of TX. At about 14 mA (LDO, 0 dBm), plus HFXO and CPU overhead,
    that is ≈ 25 µC.
  - One burst of ~48 events ≈ 1.2 mC ≈ **0.33 µAh**. 50 presses a day is
    about 17 µAh, or 0.01 % of a ~170 mAh battery.
  - For comparison, our 500 ms connectable advertising costs on the order of
    1 mAh/day while away from the phone.
  - The screen being on for the interaction costs far more than the radio.
  - Side finding (logged in SESSION.md): the DC/DC regulator is off, which
    roughly doubles radio current.

---

## 6. Feedback

### 6.1 Local feedback ("sent")

- On tap: invert the tile for 300 ms, give a short buzz (a new
  `motor::Pattern::TICK`, about 30 ms), and show "SENT" in the title bar for
  2 s.
- On failure (counter write failed, or `advertise` returned an error): show
  "NOT SENT" and a double buzz.
- Wording matters. "SENT" only means the watch transmitted. BTHome is one-way,
  so the watch can't know whether HA heard it. Never show a checkmark for
  this state.

### 6.2 No end-to-end ack

Out of scope (decided 2026-11-07): no confirmation path back from HA. An
ack through the HA Companion app and Gadgetbridge intents was considered
and dropped, so "SENT" is all the watch ever knows.

---

## 7. Code structure

| File | Change |
|---|---|
| `src/bthome.rs` (new) | `Event`, `HOME_TILES`, `encode(event, battery, counter) -> ([u8; 31], usize)`, `seal(key, mac_display, counter, &mut buf)` (non-async, on the stack), `Counter` (reservation logic), `init(rng)` (load or create the key, load the counter), the KAT. Pure logic, literate-style comments. |
| `src/ble.rs` | `BTHOME` channel; burst state machine in both the disconnected and connected loops; `try_accept` race; DFU guard; pass `&mut peripheral` to `serve`; `gb::phone_event` (in `src/gb.rs`) gains `ur_ha` / `ur_home`. |
| `src/ui.rs` | `Screen::Home` in `screens()`; `render_home`, `render_bthome_setup`; Home-specific gesture arms *before* the global long-press arm; ack state (`Option<(tile, Instant, AckKind)>`). |
| `src/main.rs` | `bthome::init(rng).await` between `storage::init` and the SDC `build`; `Status` gets `bthome_ready`. |
| `src/storage.rs` | New keys; maybe a bigger `BUFFER` for labels. |
| `src/motor.rs` | `Pattern::TICK`. |
| `Cargo.toml` | `aes 0.8`, `ccm 0.5` (pinned, `default-features = false`). |

### 7.1 Cost estimate

- **Flash:** currently `.text + .rodata + .data` ≈ 346 KB of the
  474,408-byte `FLASH` region (`llvm-size` on today's release build), so about
  127 KB is free.
  - `aes 0.8.4` and `cipher 0.4.4` are **already linked** through
    trouble-host's `security` feature (`Cargo.lock`;
    `trouble-host/src/security_manager/crypto.rs`).
  - New code: `ccm`, `ctr 0.9`, `aead 0.5` (`subtle 2.6` is already present),
    roughly 2–4 KB. The module, UI and plumbing add roughly 4–6 KB.
  - Total about **5–10 KB**, an estimate. Measure with `llvm-size -A` before
    and after step 1 and step 4.
- **RAM:** statics today are `.data + .bss + .uninit` = 43,904 of 49,152 B,
  about 5.1 KiB free.
  - New: the `BTHOME` channel (~4 × 4 B plus overhead), the key (16 B), the
    counter (8 B), burst state inside the `host_task` future (31 B buffer +
    `Advertiser` + `Instant` ≈ 60 B), and UI ack and label state (~80 B, or
    ~150 B with runtime labels).
  - Total about **0.2–0.3 KiB**, as long as the AES key schedule stays on the
    stack (§4.4).
  - SDC memory: **unchanged**, since it's still one legacy advertising set.

---

## 8. Implementation plan

0. **Spike (half a day; settles A1–A3).**
   - Temporarily repurpose the Info screen's "TAP: TEST BUZZ" to send a fixed
     unencrypted burst (the §2.3 example).
   - Check with btmon (§9) that:
     - (a) it works while the phone is connected (A1),
     - (b) the 20 ms interval is accepted (A2),
     - (c) the Gadgetbridge link survives repeated bursts (A3),
     - (d) when disconnected, connectable advertising resumes and the phone
       reconnects.
   - Record `llvm-size`.
   - If A1 fails, decide between "only while disconnected" and investigating
     the multirole library.
1. **`bthome.rs` encoder (unencrypted) + `ble.rs` plumbing** for both states,
   including the `try_accept` race and the DFU guard. Log every advert.
2. **Home screen**: const labels, gestures, local feedback, `Pattern::TICK`.
3. **Validate unencrypted, end to end:** scanner script (§9), then HA
   discovery, entities and the example automations. Use a throwaway HA
   device entry, because HA ignores plaintext once a bindkey is set.
4. **Encryption:**
   - `cargo add` the pinned crates; `seal` plus the on-device KAT.
   - Key generation in `main.rs`; storage keys; counter reservation.
   - Setup page.
   - Switch the device info byte to `0x45`.
   - In HA, delete the plaintext device and re-add it with the bindkey.
5. **Optional extras:** HA-pushed labels (`ur_home`) and ack (`ur_ha`) via the
   Gadgetbridge intent path.
6. **Docs:** README section (setup: open Home → tap title → type key in HA →
   press a tile to discover). TODOs in code for hold events, double tap,
   swipe-length steps and key rotation. Log oddities in SESSION.md.

---

## 9. Test plan

### 9.1 Without HA

- **On device:** the KAT assertion at boot (debug builds), plus `info!` hex
  of each sealed advert. Compare with `docs/bthome_vectors.py` for the same
  key, MAC and counter. This catches nonce byte-order bugs.
- **btmon** (on the laptop; may need sudo):
  ```
  sudo btmon | tee tmp/btmon.log      # terminal 1
  bluetoothctl                        # terminal 2
    menu scan → duplicate-data on → back → scan on
  ```
  Look for `ADV_NONCONN_IND` from our address with `Service Data (UUID
  0xfcd2): 4400…` (or `45…`). With duplicate-data on, counting reports per
  press shows the burst size and the real interval. Look at the `RSSI` and
  timestamp columns.
- **Scanner/decoder** (`tmp/bthome_scan.py`, run with
  `uv run --with bleak --with cryptography python -I tmp/bthome_scan.py <key-hex>`):
  ```python
  UUID = "0000fcd2-0000-1000-8000-00805f9b34fb"
  def on_adv(dev, adv):
      data = adv.service_data.get(UUID)
      if not data: return
      info = data[0]
      if info & 1:   # encrypted
          ct, ctr, mic = data[1:-8], data[-8:-4], data[-4:]
          nonce = bytes.fromhex(dev.address.replace(":", "")) + b"\xd2\xfc" + bytes([info]) + ctr
          body = AESCCM(KEY, tag_length=4).decrypt(nonce, ct + mic, None)  # raises on bad MIC
          print(dev.address, "ctr", int.from_bytes(ctr, "little"), body.hex(" "))
      else:
          print(dev.address, data[1:].hex(" "))
      # then decode objects: 00 pid, 01 batt, 3A ev ×4, 3C ev steps
  ```
  Track the last packet id and counter, and flag duplicates, regressions and
  MIC failures. Note that BlueZ through bleak deduplicates repeated identical
  adverts, so use btmon to count repeats.
- **HA's own parser offline:** feed captured service data into `bthome-ble`
  (`uv run --with bthome-ble`), using `BTHomeBluetoothDeviceData(bindkey=…)
  .update(service_info)`. The exact `BluetoothServiceInfo` constructor isn't
  checked yet; copy it from the library's tests. Assert that it yields
  `button_2`/`press` and so on, with no "not in numerical order" warning.
- **nRF Connect (Android):** a quick sanity check of the advertising type,
  interval and raw bytes from the phone.

### 9.2 With HA

| # | Case | Expect |
|---|---|---|
| T1 | First press after setup | Discovery notification. For encrypted, a bindkey prompt. The device has `button_1..4`, `dimmer`, and a battery entity. |
| T2 | Tap each tile; long press each | Exactly one `bthome_ble_event` per gesture with the right `event_class`/`event_type`. Check with Developer Tools → Events → listen to `bthome_ble_event`. |
| T3 | Same tile twice within 1 s | Two events, because the packet id differs. |
| T4 | Swipe up / down | `dimmer` `rotate_right` / `rotate_left`, `steps: 1`. |
| T5 | Phone connected vs. disconnected vs. during music | All deliver. The GB link stays up, and the phone reconnects after a disconnected-state burst. |
| T6 | Reboot the watch, then press | Accepted, because the counter jumped to the reserved block (≥ old value). After 64+ presses, check that one flash write happened (log). |
| T7 | Press, then leave the watch disconnected (connectable ads running) for >10 s | **No** second event from cached service data (§2.4). Run in both modes. |
| T8 | Replay: capture an encrypted advert, re-transmit it later (e.g. a second nRF board or `btmgmt add-adv`) | HA rejects it ("data might be compromised") while running. The documented exception is right after an HA restart. |
| T9 | Wrong key in HA | HA raises a repair issue. The watch still shows "SENT", which is honest. |
| T10 | Range: ESP32 proxy at the far end of the flat | Count misses over 50 presses. If more than 1 % are missed, lengthen the burst toward 2 s or raise the proxy scan window. |
| T11 | Battery | Compare the daily drain over 2 days with ~50 presses a day against a baseline. The difference should be indistinguishable. |

---

## 10. Open questions and risks

- **A1 (advertising while connected)** is the main unknown. Settle it first
  (step 0).
- The 31-byte budget is fully used. Any new object means dropping battery or
  dimmer, or moving to extended advertising (§5.3).
- A replay after an HA restart is possible (§3). Document that tiles are for
  convenience actions only.
- Labels and HA config can drift apart. Pushing labels from HA (§1.4) is the
  fix, so it is worth doing early in step 5.
- Gadgetbridge's "Allow intents" setting accepts any app's broadcast to the
  watch (REPORT.md: "unprotected broadcasts both ways"). The `ur_ha`/`ur_home`
  handlers must only affect cosmetic state: labels, ack text, a buzz.
