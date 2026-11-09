# Power plan: from about a week to two or three

**Status, 2026-11-08 (`9f01ada`).** Most of the plan has shipped: the
meters and executor loop (step 0), DC/DC, the disconnect wake, button
polling, the idle connection, advertising, the heart-rate back-off (now
an adaptive schedule), the glance heart-rate rule, batched log flushes,
the 10 s timeout, AUTO/SOFT brightness, touch sleep in the quiet night,
and the panel's voltages and idle mode. Since then: the accelerometer's
motion interrupt, and System OFF with storage mode. What's still open
is the accelerometer rate (`#6`) and heart-rate polling (`#14`). The
[opportunities table](#opportunities-ranked) has a status for each item,
and the steps say what landed where. The first two sections below,
[where the current goes now](#where-the-current-goes-now-2026-11-08) and
[what's left](#whats-left-re-ranked-2026-11-08), work from the field
data. Everything after them is the plan as first written, kept for its
reasoning.

## Where the current goes now (2026-11-08)

The hourly `power:` lines come from the sealed watch on 2026-11-08
(`tmp/day-20261108/report.txt`, `tmp/sealed-log19/night.log`). The night
was build `snap.76` (`1e5b869`), with the phone off from 01:21. The day
was `crash`, `hard` (`1e5b869`), `pair` (`d7ca4d1`), and `all`
(`9d067b6`). Only the first three logged hourly lines. All of them are
older than the 10 s timeout and the asleep heart-rate interval, and most
are older than the panel savings and the motion interrupt. So the table
uses the measured on-times (**L**), adjusted for what has changed since,
times the currents from the [budget](#current-budget) (**M**/**D**/**E**).
A night hour is asleep, worn, with the phone away, as that night was. A
day hour is awake, with the phone connected, the sun up, and AUTO.

| Item | Night hour µA | Day hour µA | Basis |
|---|---:|---:|---|
| Heart-rate LED | ~110 | ~50 | L: 77–116 s/h, all at 12.5 mA, on the old 10-minute schedule. Asleep it is now every 5 min, ~144 s/h (`heart_schedule.rs:72`). By day it's ~40–100 s/h, capped at 180 s/h (`LED_BUDGET`), or 360 s/h an hour after the last reading (`OVERDUE`; see [below](#the-led-budget-overdue-and-brighter)). × 2.8 mA (E, never measured) |
| Screen | ~20 | ~310 | L: panel 5–51 s/h at night, and 58–107 s/h by day at the old 15 s timeout. By day, now: ~40 s/h of full wakes at AUTO's 19.6 mA backlight plus ~4.5 mA panel, and ~25 s/h of glances at low (1.83 mA) in idle mode. At night: dim, at low |
| BMA421, 100 Hz, 4× averaging | 50 | 50 | M (aykevl) |
| Rest of the floor | ~32 | ~40 | nRF 3, ST7789 asleep 10, touch 8 by day and ~0 in the quiet night, flash 1, divider 2, regulator and leakage 15, button polls ~1 (D/E) |
| Bluetooth | ~24 | ~16 | Night: advertising at 417.5 ms, 2.4 events/s × ~10 µC. Day: 120 ms, latency 2 (L, logged) ≈ 2.8 events/s × ~4 µC ≈ 11, plus the second slot advertising every 2 s ≈ 5 (E) |
| CPU, thread mode | ~11 | ~20 | L: awake 3‰ at night, 5–6‰ by day, × ~3.7 mA (D) |
| Field log flushes (logging build) | ~2 | ~2 | L: flash awake ~2 s/h; E |
| Aranet4 and BTHome scans (added after these logs) | 0 | ~5–12 | E: receiving at ~5.7 mA (5.4 mA RX with DC/DC, plus HFXO), every 15 min (and on looks, at most every 10 min) for ~1 s with an Aranet4 near (~2.5 s, up to 6 s, with a BTHome thermometer near: ~12 µA), hourly for 3 s without, or at once when a phone fix is 500 m from the sensors (`Nearby::located`); none in the quiet night or off the wrist (`aranet::Nearby::due`). Logged as `scanning` ms and `scans` |
| **Total** | **~250** | **~490** | |

That's ~410 µA over a 16 h day and 8 h night: **~16 days** on 160 mAh.
SOFT instead of AUTO takes ~150 µA off a day hour (~100 µA averaged).

**The battery says otherwise, and that comes first.** The night's
voltage fell ~4 mV/h (4108 → 4080 mV, 02:03–09:03). On the firmware's
linear percentage that's the 0.63 %/h the report shows. Near 4.1 V a LiPo
drops ~4–6 mV per mAh (E: 4.2 → 4.0 V spans only ~10–15% of its charge),
so 4 mV/h is **~0.7–1 mA**. The `snap.76` night that logged it models at
~0.2 mA (LED at 80 s/h, touch in standby). The day hours (09:03–14:03)
fell at the same ~4 mV/h, though their meters show about twice the
night's load. Either about half a milliamp of constant drain is invisible
to the meters, or the slope is mostly measurement: relaxation after a
charge, ±5–10 mV per line, a cell whose curve we don't know. Which it is
outweighs every remaining item together. See item **A** below.

### The 135 wake-ups a second

The night's hourly lines show ~486 000 wake-ups an hour (135/s) with no
connection, the screen off, and awake 3‰. That is understood now, and
it's cheap. `power::sleep` counts every return from WFE
(`power.rs:33–56`). Each interrupt makes about two (E): one to wake, and
one straight after the next poll, because the ISR (and the executor's
SEV pender, `main.rs:226–235`) leaves the event register set. Fitting the
night's hours to their LED and panel time gives a base of ~118/s, plus
~740 per LED-second and ~260 per panel-second. In `snap.76` that splits
as:

| Source | Rate | Wake-ups/s |
|---|---|---:|
| Button poll (`main.rs:153`, P0.15 pulsed) | 20 Hz | ~40 |
| Advertising at 417.5 ms (`ble.rs:359`): RTC0, HFXO start, TIMER0, TX and RX on three channels, MPSL's low-priority signal, ~12 interrupts an event | 2.4 Hz | ~55–60 (the residual) |
| Heart-rate sessions: 100 Hz `Ticker`, one I2C read each (~7 per poll) | ~80 s/h | ~16 |
| `glance_task`'s 200 ms timer (it skipped the I2C read asleep, but not the timer) | 5 Hz | ~10 |
| UI tick and SAADC sample (`main.rs:507`) | 1 Hz | ~5 |
| Activity FIFO drain (two timers, ~5 transfers) | every 3 s | ~5 |
| Field-log writer's check (`field_log.rs:243`) | 1 Hz | ~2 |
| embassy's RTC1 overflow | 2 per 512 s | ~0 |

The cost is the awake ‰. ISR time counts as asleep: it's part of each
radio event's charge, which is already in the budget. Thread mode is 3‰,
~3 ms a second (~22 µs per wake-up), so **~11 µA** at ~3.7 mA. Nothing is
spinning: anomaly 87 is handled, and a stuck pending interrupt would read
~1000‰. The current build should show ~140/s at night. The motion
interrupt removed the glance timer while still (`activity.rs:865`, −10/s).
The FIFO drain went to every 2 s (+2/s). Sessions every 5 min asleep add
~+14/s. Advertising is the only large share that can go (item **E**).
The button's 40/s cost ~1 µA in all (~15 µs each). The 1 Hz tick's cost
is <1 µA, and it feeds the watchdog.

## What's left, re-ranked (2026-11-08)

Savings are 24-hour averages against the ~410 µA above. The letters are
new, so they don't clash with the original numbers.

| # | Change | Saves µA | Effort | Risk | Notes |
|---|---|---:|---|---|---|
| A | **Find the missing half milliamp, if it's real** | 0–~450 | S–M | None | First, with no gear: run the current build from full to empty (the field log keeps the last battery line). ~16 days means the model holds. A week or so means something is unmetered. Then, on the dev kit, measure the shunt floor with the screen off and nothing scheduled: the model says ~110 µA. Suspects, by how far off they could be: the HRS3300's real LED current (the 2.8 mA is a guess, see [below](#current-budget)), the ST7789 not quite in SLPIN (5.6 mA awake), the BMA421, the CST816S, the regulator's quiescent current |
| B | AUTO by day at high alone (12.3 mA) instead of all three (19.6 mA); or SOFT as the default; or the backlight to low for the last ~3 s before the timeout | ~55 (high), ~100 (SOFT), ~40 (fade) | XS | UX | The biggest lever left, all UX. ~40 s/h of full wakes by day at +7.3 or +14 mA. Readability in sunlight is the cost. `brightness.rs`, `ui.rs:554` |
| C | `#6`: accelerometer at 50 Hz with 2× averaging | ~30–35 | S | Medium | Still open (`activity.rs:298–300` writes `0x28`). The motion interrupt didn't change the chip's sampling, only our polling. Retune the movement thresholds (`sleep`, `heart::CALM`, `MOTION_LIMIT`, glances) after. Do it alongside the step undercount, not before it: the day's log counted 388 steps to the Garmin's 1324, and a rate change would muddy that |
| D | Heart rate every 10 min asleep instead of 5 (`ASLEEP_INTERVAL`) | ~18 (~55 at night) | XS | Accuracy | Halves the night's biggest item. The cost is sleep staging: its simulation gave ~64% agreement at 5 min and ~59% at 20 min, so maybe ~62% at 10. Scales with the LED's true current (A) |
| F | Keep the phone's latency 7: `slow_down` swaps Gadgetbridge's 112.5–120 ms, latency 7 (~1.05 events/s) for latency 2 (~2.8/s) | ~6 | XS | Low | A regression from `#1`, seen in every connection in the day's logs that went 30 s without traffic: latency 7 by 0.5 s, then latency 2 30 s later. Ask for `max_latency: 7` (`ble.rs:486–497`). With a 6 s timeout that's in spec. Phone-to-watch delay ≤ ~1 s, as the phone itself already chooses |
| E | Advertising at 1022.5 ms once the phone has been gone ~1 h (417.5 ms until then) | ~4 (with the phone off 8 h a night); ~14 while away | S | Low | `ble.rs:359–367`. The cost is a reconnect of up to ~4 min after a long absence (what 1022.5 ms took before `8288e3f`). The first 5 minutes stay fast |
| H | Find what spends the 3‰ of thread-mode CPU at night | ≤~5 | M | None | Count polls per task on the dev kit. Likely suspects: the per-minute sleep classifier, the FIFO maths, defmt encoding |
| G | The second slot's advertising at 5 s instead of 2 s while the phone is connected | ~2–3 | XS | Low | An updater or log fetcher finds the watch in ~5–10 s instead of ~2–5 s |
| — | Watch the hourly `disconnects` | 0, or tens | — | — | Each disconnect buys 5 min of 152.5 ms advertising (~5 µAh) and a fast reconnection. `crash` counted 16 in an hour (19 "Connection Timeout"s in its 1.3 h), which keeps the fast advertising running almost throughout: up to ~65 µA. If it happens on the current build, find out why |

Checked again, and still not worth it:

- **`#14`, heart-rate polling at ~27 Hz.** Each 10 ms poll is a
  9-byte write-read at 250 kHz (~0.4 ms of TWIM and HFINT) and ~7
  wake-ups, so ~25 µA while measuring. At 27 Hz that's ~7 µA, but the
  LED is on only ~3–4% of the time now: **~1 µA**, for M effort and the
  timing resolution the 100 Hz was there for.
- **The 1 Hz UI tick with the screen off**: <1 µA, as before. It also
  feeds the watchdog and keeps the battery smoothing and alarms simple.
- **Log flushes per page only** (dropping the 30 s): ~1 µA, logging
  builds only.
- **Button polls at 100 ms**: ~0.5 µA, and a quick press could fall
  between polls.

## Summary (as first written, against `99c5f56`)

Research and plan only. Nothing here had been flashed or measured on the
watch yet. The patches in the [appendix](#appendix-the-patch) compiled
(`cargo build --release`, with and without `--features field-log`)
against `99c5f56`, but they hadn't been run.

The battery is ~170–180 mAh. I take 160 mAh as usable. My model of an
ordinary day (phone connected, ~18 minutes of screen-on, worn around the
clock) puts today's firmware at **~0.7 mA average (plausible range
0.5–1.2 mA), so about 9 days (5½–13)**. The plan below brings the model to
**~0.31 mA, about 3 weeks (15–27 days)**, without changing what the watch
does. Two UX knobs (screen timeout, backlight level) would buy a few more
days.

Most of today's budget goes to four things, in this order: the Bluetooth
connection's interval, the heart-rate LED, the screen, and a ~125 µA floor
that two small changes halve. The top five actions:

1. **Slow the phone connection down while idle** (~−130 µA). Gadgetbridge
   asks for Android's "balanced" priority (30–50 ms, about 25 connection
   events a second) and we never ask for anything else. Request
   100–125 ms with peripheral latency 2 after 30 s of quiet, and speed up
   again for bulk transfers. Zero-code version for a quick A/B:
   Gadgetbridge's per-device "low power connection priority" setting.
2. **Stop the heart-rate LED climbing to 40 mA and staying there**
   (~−85 µA worn, ~−140 µA more on days it spends a night off the wrist).
   A failed session escalates to the top level (40 mA, ×8 gain) and the
   next session *starts* there. Off the wrist, `NO_CONTACT` never
   triggers, so every session burns its full 30 s at 40 mA (field log
   0.4.0 shows exactly this, every 10 minutes).
3. **Enable the DC/DC regulator** (~−70 µA, one line). The PineTime has the
   inductors. InfiniTime, Mynewt, and TinyGo all enable it on this board.
4. **Stop waking the screen on every disconnect** (~−50 µA, one line).
   `show_passkey(None)` at the end of every connection sends `Attention`,
   so each disconnect turns on ~15 s of panel, backlight, *and* the
   heart-rate LED.
5. **Halve the floor** (~−63 µA): poll the button instead of powering it
   through P0.15 all the time (~34 µA, as measured for the PINE64 wiki),
   and run the accelerometer at 50 Hz instead of 100 Hz with 4× averaging
   (~50 µA measured → 13–25 µA per Bosch).

Before any of that, **step 0** adds instrumentation (`src/power.rs`): an
hourly log line with CPU awake time, wake-ups, and on-time per
power-hungry state. It also hand-rolls the executor's sleep loop, which
applies the nRF52832's FPU erratum workaround (anomaly 87). That is
probably dormant here, but if it isn't, it costs milliamps.

## What I measured, and what I didn't

Nothing was measured on this watch's current. Each figure below is labelled:

- **M**: measured by someone on a PineTime, with the source cited.
- **D**: from a datasheet.
- **E**: my estimate, with its reasoning.
- **L**: read from our own field logs (`tmp/field2/*.defmt`).

The field logs we have are too short and too noisy to give a battery
slope. Over 50 minutes of 0.4.0 the 10-minute lines read 3803, 3783, 3817,
3815, 3814, 3782 mV, which is ±20 mV of load and noise around a ~2 mV true
drop. So the "today" total is a model, not an observation. If the watch
actually lasts much less than ~6 days, something outside the model is
drawing current. Step 0 is there to find out what.

## Current budget

As first written. For today's, from the field data, see
[where the current goes now](#where-the-current-goes-now-2026-11-08).

Assumptions for "a day":

- Phone connected 22 h and away 2 h.
- 60 glances by button, each running to the 15 s timeout (900 s on the
  clock face).
- 50 notifications, each showing for 4 s (200 s, no heart rate).
- About 20 disconnects that wake the screen.
- Worn 24 h, since sleep tracking wants that.
- 3.3 V rail. The board's LDO is linear, so rail current is battery
  current.

| Item | Today µA | After µA | Basis |
|---|---:|---:|---|
| **Floor** | | | |
| nRF52832 System ON idle, LFXO + RTC, RAM retained | 3 | 3 | D: 1.9 µA (no RAM, RTC) + retention/LFXO (E) |
| BMA421: 100 Hz, 4× averaging, step counter | 50 | 20 | M: aykevl, ~50 µA at "100 Hz, norm_avg4" (InfiniTime #53); D: Bosch 13 µA low-power at 50 Hz, 25 µA step counting |
| Button powered via P0.15 high | 34 | 1 | M: PINE64 wiki / aykevl "~34 µA when P0.15 is left high"; after: E, polling |
| CST816S standby (auto, between touches) | 8 | 8 | D: 6 µA (V1.4) to 10 µA (V1.3) |
| ST7789 in SLPIN | 10 | 10 | E, unverified; wiki: 5.6 mA if *not* slept, so the sleep matters |
| SPI flash in deep power-down | 1 | 1 | D; wiki: +14 µA if left in standby |
| Battery divider (2 × 1 MΩ) | 2 | 2 | D: wiki schematic notes |
| HRS3300 off (PON = 0) | 0 | 0 | D: 0.01 µA |
| Regulator, charger, leakage, rest | 15 | 15 | E; aykevl's whole-board floor was 60–97 µA |
| **Housekeeping** (1 Hz UI pass + SAADC, FIFO every 3 s, sleep classifier per minute) | 5 | 3 | E: ~50 µs CPU per second, ~10 ms of TWIM per 3 s |
| **Bluetooth** | | | |
| Phone link at 30–50 ms, latency 0, 22 h/day | 140 | 11 | E: ~25 events/s × ~6 µC (LDO). Cross-check, M: InfiniTime connected vs BLE-off +130 µA with DC/DC (#1420). After: ~3 events/s × ~4 µC |
| Advertising on the free slot, 500 ms, around the clock | 30 | 5 | E: ~15 µC/event (LDO). Cross-check, M: InfiniTime ~1 s advertising +30 µA (#1420). After: 2 s while connected, 1 s while away |
| **Screen** | | | |
| Panel 5.6 mA + mid backlight 5.5 mA + rendering, 1100 s/day | 148 | 148 | M: wiki table (LCD 5.61, backlight mid 5.51 mA); rendering E |
| Screen woken by every disconnect (~15 s × ~15 mA, 20/day) | 52 | 0 | E: code reading; the 0.4.0 log shows 4 disconnects in 50 min on a desk |
| **Heart rate** | | | |
| Continuous measurement while the clock face shows (900 s/day) | 52 | 20 | E: LED ~3–8 mA by level (see below); after: only while the reading is >5 min old |
| Background session every 10 min, worn | 150 | 65 | E: ~15–20 s at a sticky, often high level today (range 30–250); after: ~14 s at the last good level (~2.8 mA) |
| **Total** | **~700** | **~310** | |
| Battery life at 160 mAh | ~9.5 days | ~21 days | |

Not in the total:

- **Off the wrist** (nightstand, drawer) today: 30 s at 40 mA every
  10 min ≈ 0.43 mA *for as long as it lies there*. That is ~+145 µA
  averaged over a day with 8 hours off. After: about 5 µA. The early
  no-contact exit plus back-off means a ~4 s session every 40 min.
- **Field builds:** flushing the log every second wakes the SPI flash and
  programs a partial page each time, ~10–15 µA (E). Batching (patch)
  brings that to ~1 µA.
- **Debug probe attached:** ~3 mA (M: wiki). The chip stays in debug
  interface mode after the probe detaches, until a *power* cycle. Never
  measure with SWD connected, or right after.

The heart-rate LED's current is the shakiest number in the table, and
it's a big one. The HRS3300 datasheet gives an average "LEDA current, HRS
on" of 2.6 mA (typ.) at a 40 ms sample time, without saying at which
drive setting. I assume that's the 12.5 mA setting and that it scales
with drive: ~2.7 mA at 12.5 mA, up to ~8.4 mA at 40 mA, plus 0.11 mA for
the chip. Polling I2C every 10 ms adds ~0.07 mA (E). If the LED is
cheaper than that, the heart-rate rows shrink. Either way, step 0's LED
on-time counters let a single dev-kit measurement settle it.

### The LED budget, overdue and brighter

`heart_schedule::LED_BUDGET` caps the LED at 180 s an hour, counted at
12.5 mA's rate: a second at 30 mA counts 2.4, at 40 mA 3.2
(`budget_seconds`). On the assumption above, a budget second costs
~2.6 mA·s of LED, plus up to ~0.18 mA·s of chip and I2C polling (0.11 mA
and ~0.07 mA, per real second, which is at most one per budget second).
So the cap is ~0.14 mA on average, ~3.3 mAh a day.

An hour without a reading (`OVERDUE`), the cap and its refill double
until the next reading, and failures retry every 10 minutes. At worst
(every hour of the day overdue, every session failing and spending all
it may), that's 360 s an hour: ~0.28 mA, **~3.3 mAh a day more** than
the usual cap, about 2% of the battery. A host test
(`the_doubled_budget_caps_an_overdue_day`) pins the cap. Off the wrist
(the nightstand) is exempt from the faster retries, and sessions there
end in a second or two, so it costs nothing there; what can reach the
worst case is a wrist that won't read all day, or a still surface that
reflects the LED like skin.

### What a connection event and an advertising event cost

Nordic's figures for the nRF52832: radio TX at 0 dBm is 5.3 mA and RX is
5.4 mA (product brief, DC/DC, 3 V, radio only), or 7.1 / 6.5 mA for the
whole chip (DevZone, Nordic staff). On the LDO, roughly twice that. CPU
from flash: 58 µA/MHz with DC/DC (~3.7 mA at 64 MHz). An empty peripheral
connection event costs the HFXO ramp, a widened receive window (we
declare 50 ppm), one RX and one TX, and the ISRs. That comes to ~4 µC with
DC/DC and ~6 µC on the LDO (E). A connectable advertising event is three
TX+RX pairs, ~10 µC with DC/DC and ~15 µC on the LDO (E). Both agree with
InfiniTime's measured deltas to within the precision either deserves. A
Power Profiler Kit would pin them down in a minute each.

## Opportunities, ranked

Savings are against the ~700 µA modelled when this was written. Effort:
XS (a line or two), S (an hour), M (a day). Risk is the chance of
breaking something users notice.

| # | Change | Status (2026-11-08) | Saves µA | Effort | Risk | Notes |
|---|---|---|---:|---|---|---|
| 1 | Idle connection parameters (100–125 ms, latency 2) after 30 s quiet; fast again for activity sync, DFU, log fetch | **Done** in `e9dba03` (`ble.rs:476–497`, `slow_down` after `IDLE_AFTER`). The phone grants it (L: 112.5–123.8 ms, latency 2). But Gadgetbridge now picks latency 7 on its own, so this *adds* events: see **F** | ~130 | S | Medium | The phone may refuse, so log what it gives (patch). Phone→watch latency +≤375 ms. The 0.4.0 log already has an "Instant Passed" disconnect, so watch for more after the change. Zero-code A/B: Gadgetbridge's per-device "low power connection priority" |
| 2 | Heart rate: start each session at the last *successful* level; back off ×2, ×4 after failures; exit with `NoContact` when raising drive doesn't raise the signal | **Done** in `e9dba03` (`heart.rs:401`, the last good level; `NoContact` at `heart.rs:660–664`). Then replaced by the adaptive schedule, `4979861` (`heart_schedule.rs`: failure back-off to 1 h, `give_up`, `LED_BUDGET`). L: every LED millisecond since is at 12.5 mA | ~85 worn, ~140 more when off-wrist | S | Low | The drive-response test needs one on-wrist check first (step 3) |
| 3 | DC/DC regulator (`config.dcdc.reg1 = true`) | **Done** in `ab5b207` (`main.rs:247`) | ~70 (on today's radio/CPU load; less after #1) | XS | Low–medium | If the inductor were missing the chip would brown out and reset in a loop. Holding the button rolls back. Try the dev kit first |
| 4 | No `Attention` wake on a plain disconnect | **Done** in `ab5b207` (`ble.rs:331`, only a cut-short pairing wakes) | ~50 (scales with disconnects) | XS | None | Pure bug |
| 5 | Poll the button every 50 ms with P0.15 low between polls | **Done** in `e9dba03` (`main.rs:151–185`) | ~33 | S | Low–medium | Must keep the 7 s hold-to-rollback escape working |
| 6 | Accelerometer at 50 Hz, 2× averaging (FIFO downsampling 4× keeps 12.5 Hz) | **Still open** (`activity.rs:298–300`). Re-ranked as **C** | ~30 | XS | Medium | Step counter accuracy; the movement noise floor shifts, so the sleep thresholds need a recheck. 50 Hz is the least the feature engine's motion features (`activity::still`) accept in low-power mode; its single and double tap would need 200 Hz |
| 7 | Advertising: 2 s while the phone is connected, 152.5 ms for a minute after it leaves, then 1022.5 ms | **Done** in `e9dba03`. Changed by `8288e3f`: 152.5 ms for 5 min, then 417.5 ms, not 1022.5 ms (Android took ~4 min to find that). See **E** (`ble.rs:359–367`) | ~25 | S | Low | Updaters and log fetchers scan actively, so 2 s costs them ~2 s |
| 8 | Heart rate on a glance only while the last reading is >5 min old, and stop at the first reading | **Done** in `e9dba03`, made panic-safe in `1bd5e6b` (`main.rs:657–667`) | ~30 | XS | UX | The face may show a reading up to 5 min old (the background one is ≤10 min) |
| 9 | Executor sleep loop with the anomaly 87 workaround and timing | **Done** in `e9dba03` (`main.rs:220–237`, `power.rs:33–56`). L: awake 3‰ at night, so nothing spins | 0, or mA | S | Low | Insurance and instrumentation in one; see below |
| 10 | Screen timeout 15 → 10 s | **Done** in `93360e3` (`ui.rs:154`) | ~45 | XS | UX | **Done** (`ui`), as the user asked. Glances by button dominate screen-on time. The music screen stays on long enough for a long title to scroll by once (up to ~20 s), and a banner holds the screen until it's gone |
| 11 | Backlight low (1.83 mA, M) instead of mid (5.51 mA) after dark | **Done** in `ec80454` (`brightness.rs`). AUTO's daytime cost is now the biggest lever: **B** | ~15–25 (SOFT); AUTO costs more by day | XS | UX | **Done** (`brightness`), as the user asked: AUTO is all three drivers (~19.6 mA) while the sun is up, low from midnight until getting up, mid otherwise. That *costs*: ~+14 mA over mid while the screen is on by day, roughly +50–90 µA averaged (300–550 of the budget's 1100 s/day by day), against ~15–25 µA saved at night. SOFT on the Info screen (mid by day, low otherwise) gets the saving without the cost. A wake alarm ramps from low over a minute |
| 12 | Field-log flushes per page or per 30 s instead of per second | **Done** in `e9dba03` (`field_log.rs:241–256`) | ~10 (field builds) | XS | Low | Loses ≤30 s of log on a hard reset |
| 13 | CST816S into deep sleep (`0xA5 = 0x03`, as InfiniTime does) during detected sleep; button-only wake | **Done** in `ec80454` (`touch.rs:87`, `main.rs:670–672`) | ~5, plus no stray touch wakes | M | UX | **Done**, as the quiet night with the screen off (taps already didn't wake it then): ~2–3 µA averaged over a day, plus the I2C reads and task wake-ups that bedding's touches cost. Woken by a reset when the screen comes on |
| 15 | ST7789 at InfiniTime's lowest voltages (PWCTRL1, PWCTRL2, GCTRL) and VDV; idle mode (8 colours, ~8 Hz) for the dim look | **Done** in `ec80454` (`display.rs:116–144`, `set_idle` at `display.rs:207`) | ~15–40 | S | Low–medium | **Done** (`display`). E: the voltages ~0.5–1.5 mA off the panel's 5.6 mA whenever it's on (~6–20 µA at 1100 s/day); idle mode at ~8 Hz perhaps 1.5–3 mA more while dim (~10–20 µA at ~600 s/day of glances and nights). Unmeasured: contrast and flicker need eyes on the panel |
| 14 | Heart-rate polling at the conversion period (~27 Hz) instead of 100 Hz | **Still open; not worth it now** (~1 µA, see [what's left](#whats-left-re-ranked-2026-11-08)) | ~2–5 | M | Medium | Timing resolution was the reason for 100 Hz (`Conversions`) |

### Anomaly 87 (FPU), in more detail

Nordic's erratum: once the FPU has set an exception flag in FPSCR, its
interrupt is pending, and that "unexpectedly" wakes the CPU from System ON
idle. Almost any float arithmetic sets "inexact". We use floats in
`activity` (`sqrtf`), `ppg`, and `geo`. The documented workaround is to
clear FPSCR's flags and the FPU's pending bit before every WFE.

On a Cortex-M4, a pending but disabled interrupt only wakes WFE if
`SCR.SEVONPEND` is set. embassy's thread executor never sets it. Our
`clean_up_after_bootloader` disables every IRQ. In the 0.4.3 binary, the
only code that sets SEVONPEND is MPSL's internal blocking-wait helper
(around `0x4f8c4`), which clears it again afterwards. So I expect the
erratum to be dormant here, apart from making those MPSL waits spin.

But "expect" is all I can say, and if I'm wrong it costs several mA,
because the CPU never sleeps. The patch's `power::sleep` applies the
workaround for one `vmrs`/`vmsr` and one store per sleep. It also counts
wake-ups and RTC ticks spent asleep, so the hourly line shows it either
way:

- Expect an awake fraction of a few ‰ with the screen off.
- Expect wake-ups in the tens of thousands per hour (every radio IRQ
  counts).
- A spinning CPU would show ~1000‰ and billions of wake-ups. The u32
  counter wraps at that rate, but the ‰ figure won't.

### Checked, and not worth changing

- **SAADC.** embassy's `sample()` stops the SAADC (TASKS_STOP) before
  returning. The wiki's "ADC left enabled: 1.3 mA" is about leaving it
  *started*. One 40 µs sample a second is <0.1 µA.
- **Waking once a minute instead of once a second with the screen off.**
  The 1 Hz pass (ADC sample, battery EMA, watchdog, clock checkpoint) is
  ~50 µs of CPU, so <1 µA. Slowing it would also make the clock checkpoint
  up to a minute stale after a reset. Leave it. (Re-checked against the
  field log's wake-ups on 2026-11-08: still <1 µA.)
- **HFCLK held by something.** Nothing requests the HFXO except MPSL, for
  radio events. embassy's `hfclk_source` stays on the internal oscillator.
  TWIM and SPIM take HFINT only during transfers, and the RNG only runs
  when the controller asks for bytes.
- **TWIM/SPIM left enabled (anomaly 89).** The 400 µA static current needs
  "GPIOTE configured in event mode" plus EasyDMA. embassy-nrf's
  `Input::wait_for_*` uses PORT/SENSE, not GPIOTE IN channels in event
  mode, and the bootloader's GPIOTE state is reset at boot. The wiki
  measured idle SPI/I2C at ~1 µA. InfiniTime's 490 µA win in PR #1758 came
  from a GPIOTE+PPI workaround for anomaly 58 left armed, which we don't
  have. Step 0's floor reading confirms this. If it doesn't, power-cycle
  TWIM1/SPIM2 after use via their 0xFFC registers, as the erratum says.
- **Edge-triggered pin interrupts** (wiki: "0–0.47 mA?"). Those use GPIOTE
  IN channels. SENSE-based waits (what we use) don't have the problem.
- **Floating pins.** nRF52 pins reset to "input buffer disconnected", so
  unused pins draw nothing. The configured inputs (charger lines, touch
  INT) have defined levels.
- **defmt/RTT in non-field builds.** Encoding a frame is a few µs, at a
  few hundred frames an hour, so nothing measurable.
- **Watchdog feeding.** It's part of the 1 Hz pass; the WDT runs on LFCLK.
- **ST7789 partial/idle modes.** Maybe 1–2 mA less while on, ~20 µA
  averaged, for real complexity. Later, if ever. (Idle mode was done
  after all, as `#15`, since the dim look is black and white anyway.)

### Incidental findings (also in SESSION.md)

- `heart::off()` failing leaves the LED lit at up to ~8 mA until the next
  session (the existing TODO). Retrying, or at least counting failures in
  the hourly line, is cheap insurance. **Still open** (`heart.rs:464`).
- `GattConnectionEvent::ConnectionParamsUpdated` falls into `_ => {}`, so
  we don't know what interval the phone actually picked. The patch logs
  it. This is the single most useful line to have before tuning #1.
  **Done** in `e9dba03` (`ble.rs:768–777`). It showed Gadgetbridge
  settling on 112.5–123.8 ms, latency 7, by itself (see **F**).
- The 0.4.0 log (dev kit on a desk next to the laptop and phone) has three
  "Connection Timeout" disconnects and one "Instant Passed" in 50 minutes.
  Each one also woke the screen (#4). Unclear why. The wake is gone
  (`ab5b207`), and disconnects are counted hourly. The `crash` build
  logged 19 "Connection Timeout"s in 1.3 h on 2026-11-08 (`hard`: 3,
  the others none). **Still unexplained**; see the last row of
  [what's left](#whats-left-re-ranked-2026-11-08).

## How to measure

### Without lab gear

1. **Firmware on-time counters** (step 0, `src/power.rs`), logged hourly
   right before the battery line. Multiply each counter by the currents in
   the budget table to get µAh per hour by component. That says where to
   look next, and the total predicts the battery slope. A small
   `scripts/power-report.py` could do the arithmetic from a decoded field
   log (TODO, not written).

   The hourly line holds:
   - awake ‰ and wake-ups;
   - ms with the panel and backlight on;
   - ms of LED at each drive level;
   - ms connected (credited at disconnect, so read it over a day, not an
     hour) and ms of SPI flash awake;
   - wakes by cause (button, touch, phone, attention);
   - heart-rate sessions and disconnects.
2. **The battery slope**, from the 10-minute lines, but only over ≥24 h and
   only within the same charge window between A/B runs. Mid-range, a
   175 mAh LiPo drops roughly 2–3 mV per mAh, so 0.7 mA is ~40 mV a day
   and 0.3 mA is ~17 mV a day. Against ±20 mV of per-line noise that needs
   a day of lines and a least-squares fit. Cheap improvements:
   - take the logged sample only with the panel off and no heart-rate
     session running (load sag is most of the noise);
   - turn on SAADC oversampling.
3. **Runtime to empty** is the ground truth for the average: charge to
   full, wear it, note when it dies (the field log's last battery line
   survives). It's slow, a week or more per configuration, so save it for
   checking the final result, not for A/B testing each step.
4. **A USB power meter doesn't work well.** The PineTime has no power path:
   USB feeds the charger, which feeds the battery and the system together,
   so the meter reads charge current plus load. Most meters also resolve
   only 1–10 mA. At best it shows mA-sized effects (backlight, the LED,
   a spinning CPU) on a full battery.

### With the dev kit (needs the hardware owner)

- **Shunt plus multimeter.** Put a 10 Ω resistor in the battery's positive
  lead, with ~1000 µF on the watch side so the multimeter's DC-mV reading
  averages the radio spikes: 1 mV = 100 µA. A 15 mA screen-on peak drops
  only 150 mV, which the 3.3 V regulator tolerates down to a ~3.6 V
  battery. Use 1 Ω for screen-on tests. Disconnect SWD and power-cycle
  first (the debug interface costs ~3 mA and survives a reset).
- **A Nordic Power Profiler Kit II** (~€100) in source-meter mode at 3.8 V
  on the battery terminals is the real tool. It shows the charge per
  connection event and per advertising event directly, and the floor to
  0.2 µA. Below ~0.3 mA everything left is µA-sized, and only a PPK can
  see the difference between one experiment and the next.

## The plan, step by step

Each step is a separate commit and a separate field build. Every
measurement runs the same day pattern (worn, phone nearby), with
`DEFMT_LOG=info` so the log's own cost stays constant. I'd do steps 1–2
before 3–4, since they're bigger wins than their effort.

### Step 0: instrument, and take a baseline (the patch's `power.rs`, `main.rs` executor and meters, conn-param logging)

- Change: hand-rolled executor loop with the anomaly 87 workaround, the
  hourly `power:` line, and logging of `ConnectionParamsUpdated`.
- Test:
  - The watch boots, connects, takes an update, and rolls back as before.
  - With the screen off, the hourly line shows awake ≤ ~10‰.
  - A glance adds ~15 000 ms to `panel`.
  - A background session adds ~13 000–30 000 ms to one LED bucket.
  - The log shows the phone's connection interval.
- Baseline: wear it 48 h with the field log. Record the hourly lines, the
  battery fit, and the phone's interval. If the fit says ≫ 1 mA, stop
  here and find out why before going on: look at the awake ‰, the LED
  buckets (an LED that never turned off), and wake causes.
- **Status: done** in `e9dba03`, and `9ea33f8` added `still`. The
  dev kit read 99.8% asleep and ~150 wake-ups a second. The sealed
  watch logs 3‰ and 135/s at night, 5–6‰ by day. The battery fit is
  item **A**.

### Step 1: the free ones (`#4` attention, `#3` DC/DC, `#7` advertising)

- `#4`: plain disconnects no longer wake the screen. Test: turn the
  phone's Bluetooth off and on. The screen stays dark, and `attention`
  wakes stay 0.
- `#3`: `config.dcdc.reg1 = true`. Test first on the dev kit, then a day
  in the field:
  - no new resets (`reset reason` lines), especially during transmit
    bursts (DFU at 7.5 ms) and with the motor buzzing at a low battery;
  - the shunt, if available, should show the screen-off average drop by
    tens of µA with the phone connected.
- `#7`: advertising intervals. Test:
  - with the phone connected, the nRF DFU app still finds "Bangle.js ur"
    within ~5 s;
  - with the phone's Bluetooth toggled, Gadgetbridge reconnects within
    ~10 s;
  - after it has been away >1 min, it reconnects within ~1 min of coming
    back.
- **Status: done.** `#4` and `#3` in `ab5b207`, `#7` in `e9dba03`.
  The last test failed at 1022.5 ms (a reconnect took ~4 min), so
  `8288e3f` made it 152.5 ms for 5 min, then 417.5 ms.

### Step 2: the connection interval (`#1`)

- First the no-code A/B: a day with Gadgetbridge's "low power connection
  priority" on for this device. The logged interval should show
  100–125 ms with latency 2 (AOSP's defaults, which I haven't verified on
  this phone), and the battery fit should flatten.
- Then the patch's `slow_down` after 30 s of quiet, with `speed_up`
  before activity syncs, DFU, and log fetches. Test:
  - a notification still buzzes within ~0.5 s;
  - call accept/reject and the music buttons still feel immediate;
  - an activity sync still finishes in seconds;
  - DFU speed is unchanged (~9.7 KiB/s);
  - no rise in "Instant Passed" or connection-timeout disconnects
    (they're counted in the hourly line now).

  If the phone refuses (a warning, or the logged interval doesn't
  change), keep the Gadgetbridge setting and drop the patch.
- **Status: done** in `e9dba03`, and the phone grants it. But by
  2026-11-08 Gadgetbridge's own parameters were already 112.5–123.8 ms
  with latency 7, half a second into each connection. So `slow_down`
  now *lowers* the latency to 2, and costs ~7 µA while connected. The
  fix is **F**.

### Step 3: heart rate (`#2`, `#8`)

- First, on the wrist, one debug-level session log with the level ladder
  forced through 12.5 → 20 mA at ×4 gain. The mean must rise well past 5%
  (the fingertip reading gave ~3700 at ×2 against ~584 off-wrist). If
  it doesn't, PDRIVE isn't doing what we think, and the ladder's
  drive steps cost power for nothing. Drop them in favour of gain-only
  steps instead of shipping the drive-response test.
- Then the patch: start at the last good level, back off after failures,
  exit early with no contact, and on a glance measure only when the
  reading is stale. Test:
  - on the wrist, one reading per 10 min as before (Gadgetbridge's HR
    chart) and LED time mostly in `led0`;
  - on the desk, sessions end in ~4–6 s with "not on a wrist", spaced
    20 then 40 min;
  - sleep tracking now gets `worn = false` from those sessions, so check
    the not-worn classification on a desk night.
- **Status: done**, differently. `e9dba03` landed the last good level,
  the failure back-off, and the stale-only glance. Instead of the
  drive-response test, a low-light check at the lowest drive ends
  sessions off the wrist (`heart.rs:657–664`). `4979861` replaced the
  fixed 10 minutes with `heart_schedule`: 2–20 min by movement and
  readings, every 5 min asleep for sleep staging, failures backing off
  to 1 h, a 180 s/h LED budget, and later a calm start and an early
  give-up at a desk. Every LED millisecond since has been at 12.5 mA.

### Step 4: the floor (`#5`, `#6`)

- Button polling. Test:
  - a quick tap still registers, ten in ten;
  - holding 7 s still resets into the bootloader, and holding through
    the logo still selects rollback (this is the sealed-watch escape
    hatch, so test it on the dev kit before any field build);
  - shunt: ~−30 µA.
- Accelerometer at 50 Hz. Test:
  - walk 500 counted steps on both builds (same wrist, same route); the
    counts must agree within a few %;
  - re-check the still-wrist movement floor (~25 at 100 Hz) on a desk and
    one night, and adjust the `sleep` thresholds if it moved;
  - the heart-rate motion veto (`MOTION_LIMIT`) uses the same units.
- **Status:** button polling **done** in `e9dba03`. The accelerometer
  rate is **still open** (item **C**). `9ea33f8` added the motion
  interrupt instead, which stops *our* polling while the wrist is
  still (~1–3 µA), but the chip samples at 100 Hz as before.

### Step 5: UX knobs (`#10`, `#11`), if wanted

- A 10 s timeout and the low backlight after dark are each a line. Look
  at the hourly `panel` ms and the wake causes before and after. If touch
  wakes are frequent and unintended, consider `#13`.
- **Status: done.** `#11` and `#13` in `ec80454` (with `#15`), and
  `#10` in `93360e3`. AUTO by day is now the largest single item:
  **B**.

### Step 6: field builds (`#12`)

- Batch the log flushes. Test: a fetched log is still complete up to
  ≤30 s before the fetch, and the `flash` ms in the hourly line drops by
  an order of magnitude.
- **Status: done** in `e9dba03`. L: flash awake ~2 s/h.

After each step, update the budget table with what the hourly lines and
the battery fit say. The model is only as good as its worst current
figure, and the LED and the radio are the ones I trust least.

## Sources

Nordic:

- [nRF52832 anomaly 87, FPU wake from System ON idle](https://docs.nordicsemi.com/bundle/errata_nRF52832_Rev3/page/ERR/nRF52832/Rev3/latest/anomaly_832_87.html)
- [nRF52832 anomaly 89, static 400 µA with GPIOTE and TWIM/SPIM](https://docs.nordicsemi.com/bundle/errata_nRF52832_Rev3/page/ERR/nRF52832/Rev3/latest/anomaly_832_89.html)
- [nRF52832 product brief](https://www.nordicsemi.com/-/media/Software-and-other-downloads/Product-Briefs/nRF52832-product-brief.pdf):
  5.3/5.4 mA TX/RX, 58 µA/MHz, 1.9 µA System ON with RTC
- [DevZone: 5.3 mA vs 7.1 mA TX](https://devzone.nordicsemi.com/f/nordic-q-a/17076/nrf52832-current-consumption)

PINE64 and InfiniTime:

- [PINE64 wiki, PineTime hardware](https://pine64.org/documentation/PineTime/Further_information/Hardware):
  the component current table (LCD, backlight levels, button, ADC, SWD,
  SPI flash) and the divider values
- [InfiniTime discussion #1420](https://github.com/InfiniTimeOrg/InfiniTime/discussions/1420):
  PPK2 measurements before and after the DC/DC, HRS sleep, and SPI fixes;
  16 days on a charge
- [InfiniTime issue #53](https://github.com/InfiniTimeOrg/InfiniTime/issues/53):
  aykevl's measurements (accelerometer configurations, P0.15, touch,
  ~60–97 µA floors)
- [InfiniTime PR #1755](https://github.com/InfiniTimeOrg/InfiniTime/pull/1755)
  (DC/DC) and [PR #1758](https://github.com/InfiniTimeOrg/InfiniTime/pull/1758)
  (SPI/PPI, 490 µA)
- [InfiniTime main.cpp](https://github.com/InfiniTimeOrg/InfiniTime/blob/main/src/main.cpp)
  (`NRF_POWER->DCDCEN = 1`) and
  [Cst816s.cpp](https://github.com/InfiniTimeOrg/InfiniTime/blob/main/src/drivers/Cst816s.cpp)
  (sleep: `0xA5 = 0x03`)
- [TinyGo's PineTime board file](https://gitrepo.ru/NeonXP/board/src/branch/master/board-pinetime.go)
  (mirror): sets DCDCEN, "significantly reduces runtime power"

Parts:

- CST816S datasheets: [V1.4](https://files.waveshare.com/upload/5/51/CST816S_Datasheet_EN.pdf)
  and [V1.3](https://www.buydisplay.com/download/ic/DS-CST816S_DS_V1.3.pdf)
- [BMA423 flyer](https://datasheet.chipsmall.com/SHUTTLE-BOARD-BMA423/651171.pdf):
  13 µA low-power at 50 Hz, 25 µA step counting
- HRS3300 datasheet, `tmp/hr-research/HRS3300_datasheet.pdf` p. 5: LEDA
  2.6 mA typ., chip 110 µA, sleep 0.01 µA

Connection intervals:

- Espruino on idle intervals: [Reducing Bangle.js power draw](https://forum.espruino.com/conversations/352818)
  and [connecting intermittently](https://forum.espruino.com/conversations/399937)
- [Android `BluetoothGatt.requestConnectionPriority`](https://developer.android.com/reference/android/bluetooth/BluetoothGatt).
  The numeric intervals are AOSP defaults, from memory, and unverified.
- Gadgetbridge requests BALANCED or LOW_POWER at init:
  `tmp/gbsrc/AbstractBTLESingleDeviceSupport.java` (around line 383)

Our own code and logs:

- embassy-executor 0.10 `platform/cortex_m.rs`: plain `wfe`, SEV pender
- embassy-nrf 0.11:
  - `saadc.rs`: `sample()` stops the SAADC
  - `gpiote.rs`: waits use SENSE/PORT
  - `lib.rs`: `dcdc.reg1` defaults to false
- Field logs: `tmp/field2/field-log-0.4.0.defmt` (decode with
  `tmp/release-0.4.0/ur-0.4.0.elf`). They show the off-wrist level climb,
  the 30 s sessions, the battery noise, and the disconnects.

## Appendix: the patch

**Historical.** It landed, adapted and split, as `ab5b207` and
`e9dba03`, and the code has moved on since. Read the commits, not
this, for what's there.

This applies to `99c5f56` with `git apply`, and is also at
`tmp/power-research/power.patch`. It covers steps 0–4 and 6. Steps 5 and
13–14 aren't in it. Split it per step when landing.

Things the patch knowingly leaves rough:

- `Meter::Connected` is credited at disconnect, not hourly.
- `WakePhone` counts any phone event that arrives with the screen off,
  not only the ones that wake it.
- `SpiFlash::wake()` forgets an awake interval it overwrites.

```diff
diff --git a/src/activity.rs b/src/activity.rs
index b1a8dec..b994113 100644
--- a/src/activity.rs
+++ b/src/activity.rs
@@ -179,13 +179,17 @@ impl Bma421 {
         features[1 + STEP_COUNTER_ENABLE.0] |= STEP_COUNTER_ENABLE.1;
         self.write_bytes(&features).await?;
 
-        // Accelerometer: 100 Hz (what the step counter wants), ±2 g,
-        // averaging 4 samples, low-power averaging mode, as InfiniTime does.
-        self.write(REG_ACC_CONF, 0x28).await?;
+        // Accelerometer: 50 Hz, ±2 g, averaging 2 samples, low-power
+        // averaging mode. InfiniTime runs 100 Hz with 4-sample averaging,
+        // which aykevl measured at ~50 µA; Bosch rates the low-power mode at
+        // 13 µA at 50 Hz, and aykevl counted steps at 50 Hz.
+        // TODO: confirm step counts (walk a counted 500 steps) and the
+        // still-wrist movement floor (sleep thresholds) at this rate.
+        self.write(REG_ACC_CONF, 0x17).await?;
         self.write(REG_ACC_RANGE, 0x00).await?;
         // FIFO: filtered accelerometer data only, no frame headers,
-        // downsampled 8× to 12.5 Hz (75 bytes/s; the 1 KB FIFO holds 13 s).
-        self.write(REG_FIFO_DOWNSAMPLING, 0x80 | (3 << 4)).await?;
+        // downsampled 4× to 12.5 Hz (75 bytes/s; the 1 KB FIFO holds 13 s).
+        self.write(REG_FIFO_DOWNSAMPLING, 0x80 | (2 << 4)).await?;
         self.write(REG_FIFO_CONFIG_1, 0x40).await?;
         self.write(REG_POWER_CTRL, 0x04).await?;
         // Advanced power save back on (with FIFO self-wake-up), now that
diff --git a/src/ble.rs b/src/ble.rs
index d937cab..89731f6 100644
--- a/src/ble.rs
+++ b/src/ble.rs
@@ -21,11 +21,11 @@ use core::sync::atomic::{AtomicBool, Ordering};
 
 use defmt::{debug, info, warn};
 use embassy_futures::join::join3;
-use embassy_futures::select::{Either3, select3};
+use embassy_futures::select::{Either4, select, select4};
 use embassy_sync::blocking_mutex::Mutex;
 use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
 use embassy_sync::channel::Channel;
-use embassy_time::{Duration, Instant, Timer};
+use embassy_time::{Duration, Instant, Timer, with_timeout};
 use heapless::Vec;
 use nrf_sdc::mpsl::MultiprotocolServiceLayer;
 use trouble_host::prelude::*;
@@ -203,6 +203,8 @@ pub async fn host_task(controller: Controller, address: [u8; 6]) {
         }
     };
     let gps = RefCell::new(GpsPolicy::default());
+    // When the phone last left, for `advertising_interval`.
+    let last_disconnect: Cell<Option<Instant>> = Cell::new(None);
     // Each slot advertises until something connects, serves it until it
     // disconnects, and repeats. While one slot serves, the other advertises,
     // so a second client can always find us.
@@ -210,8 +212,16 @@ pub async fn host_task(controller: Controller, address: [u8; 6]) {
         loop {
             let conn = {
                 let mut peripheral = peripheral.lock().await;
-                match advertise(&mut peripheral, &server).await {
-                    Ok(conn) => conn,
+                match advertise(
+                    &mut peripheral,
+                    &server,
+                    advertising_interval(last_disconnect.get()),
+                )
+                .await
+                {
+                    Ok(Some(conn)) => conn,
+                    // Time to review the interval (see `ADVERTISING_REVIEW`).
+                    Ok(None) => continue,
                     Err(e) => {
                         warn!("ble: advertising failed: {}", defmt::Debug2Format(&e));
                         Timer::after_secs(5).await;
@@ -226,26 +236,65 @@ pub async fn host_task(controller: Controller, address: [u8; 6]) {
             // Whether this is the phone; see `serve`. The status reporter
             // waits for that, and never finishes on its own.
             let phone = Cell::new(false);
-            embassy_futures::select::select(
+            let since = Instant::now();
+            select(
                 serve(&stack, &server, &conn, &gps, &phone),
                 report_status(&phone),
             )
             .await;
+            crate::power::add(crate::power::Meter::Connected, since);
+            crate::power::count(crate::power::Count::Disconnects);
             if phone.get() {
                 CONNECTED.store(false, Ordering::Relaxed);
                 gps.borrow_mut().on_disconnect(Instant::now());
+                last_disconnect.set(Some(Instant::now()));
+            }
+            // Only if a pairing was cut short: a plain disconnect has nothing
+            // to show, and waking the screen for it costs ~15 s of panel,
+            // backlight, and heart-rate LED.
+            if PASSKEY.lock(Cell::get).is_some() {
+                show_passkey(None);
             }
-            show_passkey(None);
         }
     };
     join3(host, slot(), slot()).await;
 }
 
-/// Advertises until the phone connects.
+/// How long one advertising run lasts before its interval is reconsidered
+/// (the phone may have connected or left on the other slot meanwhile).
+const ADVERTISING_REVIEW: Duration = Duration::from_secs(30);
+
+/// How often to advertise, given when the phone last disconnected.
+///
+/// Each advertising event costs about as much as several connection events
+/// (three channels, each transmit followed by a listen), so this is most of
+/// what Bluetooth costs while the phone is away:
+///
+/// - Right after the phone leaves, it usually comes straight back (a brief
+///   dropout), so advertise quickly for a minute.
+/// - While the phone is connected, the other slot only serves an updater or
+///   a log fetcher, whose active scans find even a slow advertiser in a few
+///   seconds.
+/// - Otherwise the phone is away (out of range, Bluetooth off), possibly for
+///   days: advertise slowly. Android's background reconnect scan still
+///   catches a ~1 s advertiser within tens of seconds.
+fn advertising_interval(last_disconnect: Option<Instant>) -> Duration {
+    if CONNECTED.load(Ordering::Relaxed) {
+        Duration::from_millis(2000)
+    } else if last_disconnect.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
+        Duration::from_micros(152_500)
+    } else {
+        Duration::from_micros(1_022_500)
+    }
+}
+
+/// Advertises at `interval` until something connects, or until
+/// [`ADVERTISING_REVIEW`] passes (`None`).
 async fn advertise<'values, 'server, C: trouble_host::Controller>(
     peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
     server: &'server Server<'values>,
-) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
+    interval: Duration,
+) -> Result<Option<GattConnection<'values, 'server, DefaultPacketPool>>, BleHostError<C::Error>> {
     let mut data = [0; 31];
     let len = AdStructure::encode_slice(
         &[
@@ -255,11 +304,8 @@ async fn advertise<'values, 'server, C: trouble_host::Controller>(
         &mut data,
     )?;
     let parameters = AdvertisementParameters {
-        // Slower than the 160 ms default, to save power while the phone is
-        // away. A reconnecting phone still finds us within a second or two.
-        // TODO: advertise quickly for a while after disconnect, then back off.
-        interval_min: Duration::from_millis(500),
-        interval_max: Duration::from_millis(500),
+        interval_min: interval,
+        interval_max: interval,
         ..Default::default()
     };
     let advertiser = peripheral
@@ -271,7 +317,11 @@ async fn advertise<'values, 'server, C: trouble_host::Controller>(
             },
         )
         .await?;
-    Ok(advertiser.accept().await?.with_attribute_server(server)?)
+    // Dropping the advertiser (on timeout) stops advertising.
+    match with_timeout(ADVERTISING_REVIEW, advertiser.accept()).await {
+        Ok(conn) => Ok(Some(conn?.with_attribute_server(server)?)),
+        Err(_) => Ok(None),
+    }
 }
 
 /// Sends one field log sector; see [`LogService`].
@@ -358,6 +408,36 @@ async fn speed_up(
     }
 }
 
+/// After this long without traffic, ask the phone to [`slow_down`].
+const IDLE_AFTER: Duration = Duration::from_secs(30);
+
+/// Asks for connection parameters for a phone that's just keeping in
+/// touch: 100–125 ms, and we may skip two events in three when we have
+/// nothing to send. These are AOSP's defaults for its own
+/// `CONNECTION_PRIORITY_LOW_POWER`, so Android should accept them (the
+/// `ConnectionParamsUpdated` log line says whether it did). Gadgetbridge asks
+/// for "balanced" (30–50 ms, no skipping) at connect, i.e. ~25 connection
+/// events a second; this is ~3. Anything from the phone waits at most
+/// ~375 ms longer.
+async fn slow_down(
+    stack: &Stack<'_, Controller, DefaultPacketPool>,
+    conn: &GattConnection<'_, '_, DefaultPacketPool>,
+) {
+    let params = RequestedConnParams {
+        min_connection_interval: Duration::from_millis(100),
+        max_connection_interval: Duration::from_millis(125),
+        max_latency: 2,
+        supervision_timeout: Duration::from_secs(6),
+        ..Default::default()
+    };
+    if let Err(e) = conn.raw().update_connection_params(stack, &params).await {
+        warn!(
+            "ble: asking for a slower connection failed: {}",
+            defmt::Debug2Format(&e)
+        );
+    }
+}
+
 /// What a received line asks us to do; collected while parsing (which
 /// happens inside a non-async callback) and acted on afterwards.
 enum Action {
@@ -400,7 +480,19 @@ async fn serve(
         let _ = OUTBOX.try_send(Outgoing::GpsPower(true));
     };
 
+    // When the link last carried anything, and whether we've since asked for
+    // the idle parameters (see `slow_down`).
+    let mut last_traffic = Instant::now();
+    let mut slowed = false;
+
     let reason = loop {
+        let idle = async {
+            if phone.get() && !slowed && !update.active() {
+                Timer::at(last_traffic + IDLE_AFTER).await
+            } else {
+                core::future::pending().await
+            }
+        };
         let gps_timeout = async {
             match gps_deadline {
                 Some(deadline) => Timer::at(deadline).await,
@@ -414,13 +506,18 @@ async fn serve(
                 core::future::pending().await
             }
         };
-        let event = match select3(conn.next(), outgoing, gps_timeout).await {
-            Either3::First(event) => event,
-            Either3::Second(message) => {
+        let event = match select4(conn.next(), outgoing, gps_timeout, idle).await {
+            Either4::First(event) => event,
+            Either4::Second(message) => {
                 send(server, conn, message).await;
                 continue;
             }
-            Either3::Third(()) => {
+            Either4::Fourth(()) => {
+                slow_down(stack, conn).await;
+                slowed = true;
+                continue;
+            }
+            Either4::Third(()) => {
                 warn!("ble: no location from the phone; is GPS forwarding enabled?");
                 gps_deadline = None;
                 send(server, conn, Outgoing::GpsPower(false)).await;
@@ -430,6 +527,7 @@ async fn serve(
         match event {
             GattConnectionEvent::Disconnected { reason } => break reason,
             GattConnectionEvent::Gatt { event } => {
+                last_traffic = Instant::now();
                 let mut actions: Vec<Action, 4> = Vec::new();
                 // DFU writes are acted on after replying (flash work is
                 // async; this callback isn't), so keep a copy.
@@ -466,6 +564,7 @@ async fn serve(
                     // A fetch is hundreds of requests; make them quick.
                     if index == 0 {
                         speed_up(stack, conn).await;
+                        slowed = false;
                     }
                     send_log_sector(server, conn, index).await;
                 }
@@ -486,6 +585,7 @@ async fn serve(
                     }
                     if update.active() && !was_updating {
                         speed_up(stack, conn).await;
+                        slowed = false;
                     }
                 }
                 for action in actions {
@@ -517,7 +617,14 @@ async fn serve(
                             send(server, conn, Outgoing::GpsPower(gps_deadline.is_some())).await
                         }
                         Action::ActivityFetch { since_ms } => {
-                            send_activity(server, conn, since_ms).await
+                            // Hundreds of samples: a few per connection
+                            // event, so not at the idle interval.
+                            if slowed {
+                                speed_up(stack, conn).await;
+                                slowed = false;
+                            }
+                            send_activity(server, conn, since_ms).await;
+                            last_traffic = Instant::now();
                         }
                     }
                 }
@@ -552,6 +659,20 @@ async fn serve(
             GattConnectionEvent::Encrypted { security_level, .. } => {
                 info!("ble: link encrypted ({})", security_level);
             }
+            // What the phone actually gave us: the single biggest factor in
+            // what the connection costs (see `docs/power-plan.md`).
+            GattConnectionEvent::ConnectionParamsUpdated {
+                conn_interval,
+                peripheral_latency,
+                supervision_timeout,
+            } => {
+                info!(
+                    "ble: connection interval {} µs, latency {}, timeout {} ms",
+                    conn_interval.as_micros(),
+                    peripheral_latency,
+                    supervision_timeout.as_millis()
+                );
+            }
             _ => {}
         }
     };
diff --git a/src/field_log.rs b/src/field_log.rs
index 5ff8c30..75f6ab3 100644
--- a/src/field_log.rs
+++ b/src/field_log.rs
@@ -92,6 +92,16 @@ fn rtt() -> &'static RttChannel {
     unsafe { &_SEGGER_RTT.up }
 }
 
+#[cfg(feature = "field-log")]
+/// How many bytes are waiting in the RTT buffer.
+fn waiting() -> usize {
+    let channel = rtt();
+    let size = channel.size as usize;
+    let write = channel.write.load(Ordering::Acquire) as usize;
+    let read = channel.read.load(Ordering::Relaxed) as usize;
+    (write + size - read) % size
+}
+
 #[cfg(feature = "field-log")]
 /// Copies whatever whole frames are waiting in the RTT buffer to `out`, and
 /// marks them read. Returns how many bytes were copied.
@@ -218,8 +228,21 @@ pub async fn task() {
     info!("field log: writing sector {} (#{})", sector, sequence);
 
     let mut buffer = [0u8; 512];
+    let mut last_flush = embassy_time::Instant::now();
     loop {
         Timer::after_secs(1).await;
+        // Each flush wakes the SPI flash and programs a (partial) page, which
+        // costs about the same for 10 bytes as for 256. At debug level the
+        // log grows ~10 bytes a second, so flushing every second was ~10×
+        // the energy of flushing full pages. Wait for a page's worth, or 30 s
+        // (so a reset loses little), whichever comes first; the 1 KiB RTT
+        // buffer holds far more than either.
+        if waiting() < spi_flash::PAGE as usize
+            && last_flush.elapsed() < embassy_time::Duration::from_secs(30)
+        {
+            continue;
+        }
+        last_flush = embassy_time::Instant::now();
         let n = drain(&mut buffer);
         if n == 0 {
             continue;
diff --git a/src/heart.rs b/src/heart.rs
index b3dd455..bca3066 100644
--- a/src/heart.rs
+++ b/src/heart.rs
@@ -332,9 +332,13 @@ pub async fn heart_task(i2c: Device) {
     }
     info!("heart: sensor ready");
 
-    // Remembered across sessions: the wrist (and the sensor's oscillator)
-    // don't change much in ten minutes.
-    let mut level = 0;
+    // The level the last *successful* session ended at: the wrist (and the
+    // sensor's oscillator) don't change much in ten minutes. A failed
+    // session (off the wrist, typically) climbs to the top level, 40 mA, and
+    // starting every later session there would triple the LED's cost.
+    let mut good_level = 0;
+    // Consecutive background sessions without a reading, for backing off.
+    let mut failures = 0u32;
     let mut conversions = Conversions::new();
     // Measure right away, so the clock face has a number soon after boot.
     let mut next_due = Instant::now();
@@ -352,7 +356,12 @@ pub async fn heart_task(i2c: Device) {
         }
 
         let started = Instant::now();
+        let mut level = good_level;
+        crate::power::count(crate::power::Count::HeartSessions);
         let outcome = measure(&mut hrs, &mut level, &mut conversions).await;
+        if matches!(outcome, Ok(Ended::Measured)) {
+            good_level = level;
+        }
         if hrs.off().await.is_err() {
             // TODO: retry, or the LED stays lit until the next session.
             warn!("heart: couldn't turn the sensor off");
@@ -378,9 +387,20 @@ pub async fn heart_task(i2c: Device) {
         // A reading resets the background schedule; so does a failed
         // background session (retrying right away would most likely fail
         // the same way: the watch is off the wrist, or the wrist is busy).
-        // TODO: retry a failed background session sooner than `INTERVAL`.
-        if matches!(outcome, Ok(Ended::Measured)) || Instant::now() >= next_due {
+        //
+        // Repeated failures mean the watch is most likely off the wrist (on
+        // a nightstand, in a drawer), where each session burns up to
+        // `SESSION_LIMIT` at the top LED level; back off to at most four
+        // times the interval. Being looked at (`WATCH_STARTED`) still starts
+        // a session right away.
+        // TODO: also end the back-off as soon as the accelerometer sees
+        // movement again.
+        if matches!(outcome, Ok(Ended::Measured)) {
+            failures = 0;
             next_due = Instant::now() + INTERVAL;
+        } else if Instant::now() >= next_due {
+            failures = (failures + 1).min(2);
+            next_due = Instant::now() + INTERVAL * (1 << failures);
         }
 
         // A session only ends while someone's watching if there's no
@@ -391,6 +411,42 @@ pub async fn heart_task(i2c: Device) {
     }
 }
 
+/// Accounts the LED's on-time to its drive current, for [`crate::power`].
+struct LedMeter {
+    drive: u8,
+    since: Instant,
+}
+
+impl LedMeter {
+    fn new(drive: u8) -> Self {
+        Self {
+            drive,
+            since: Instant::now(),
+        }
+    }
+
+    fn switch(&mut self, drive: u8) {
+        crate::power::add(Self::meter(self.drive), self.since);
+        *self = Self::new(drive);
+    }
+
+    fn meter(drive: u8) -> crate::power::Meter {
+        use crate::power::Meter;
+        [
+            Meter::HeartLed0,
+            Meter::HeartLed1,
+            Meter::HeartLed2,
+            Meter::HeartLed3,
+        ][usize::from(drive & 3)]
+    }
+}
+
+impl Drop for LedMeter {
+    fn drop(&mut self) {
+        crate::power::add(Self::meter(self.drive), self.since);
+    }
+}
+
 /// Why a session ended.
 enum Ended {
     /// We have a reading (and nobody's watching for more).
@@ -411,6 +467,8 @@ async fn measure(
     conversions: &mut Conversions,
 ) -> Result<Ended, ()> {
     hrs.on(LEVELS[*level]).await?;
+    // Ends the LED's on-time when the session returns, however it returns.
+    let mut led = LedMeter::new(LEVELS[*level].drive);
     conversions.restart();
     let mut ppg = Ppg::new();
     let mut consensus = Consensus::new();
@@ -419,6 +477,9 @@ async fn measure(
     let mut moved_at: Option<Instant> = None;
     // This second's signal, for the level check.
     let (mut peak, mut sum, mut count) = (0u32, 0u32, 0u32);
+    // The previous level check's level and mean, for the drive-response
+    // test below.
+    let mut previous: Option<(Level, u32)> = None;
 
     let mut ticker = Ticker::every(POLL);
     loop {
@@ -459,6 +520,23 @@ async fn measure(
                 conversions.period_us()
             );
             let before = *level;
+            // Light reflected by skin (or anything else) scales with the LED
+            // current; the sensor's dark offset doesn't. Off the wrist, going
+            // from 12.5 to 20 mA at the same gain moved the mean by ~1.5%
+            // (field log, 0.4.0), and the session then sat at 40 mA for the
+            // rest of its 30 s. Under 5% means nothing is reflecting the LED.
+            //
+            // TODO: confirm on the wrist that a step in drive raises the
+            // mean well past 5% before relying on this.
+            let current = LEVELS[*level];
+            if let Some((was, was_mean)) = previous
+                && was.gain == current.gain
+                && current.drive > was.drive
+                && mean * 20 < was_mean * 21
+            {
+                return Ok(Ended::NoContact);
+            }
+            previous = Some((current, mean));
             if peak >= SATURATED {
                 if *level == 0 {
                     return Ok(Ended::TooBright);
@@ -473,6 +551,7 @@ async fn measure(
             if *level != before {
                 debug!("heart: switching to level {}", LEVELS[*level]);
                 hrs.on(LEVELS[*level]).await?;
+                led.switch(LEVELS[*level].drive);
                 conversions.restart();
                 ppg.reset();
                 continue;
diff --git a/src/main.rs b/src/main.rs
index 221c2bb..decf11f 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -44,6 +44,7 @@ mod geo;
 mod heart;
 mod i2c;
 mod motor;
+mod power;
 mod ppg;
 mod sleep;
 mod spi;
@@ -122,11 +123,30 @@ pub static INPUT: Channel<CriticalSectionRawMutex, InputEvent, 8> = Channel::new
 /// stops feeding the watchdog: see [`firmware`].
 static BUTTON_HELD: AtomicBool = AtomicBool::new(false);
 
+/// How often to look at the button. A press lasts 100 ms or more, so this
+/// can't miss one, and it's at most this much slower to respond.
+const BUTTON_POLL: Duration = Duration::from_millis(50);
+
 /// Watches the side button and reports each (debounced) press.
+///
+/// The button only works while P0.15 (`enable`) is driven high, and powering
+/// it costs ~34 µA continuously (measured by aykevl for the PINE64 wiki),
+/// more than the nRF52 and the accelerometer's step counter together. So it
+/// is powered only for a moment every [`BUTTON_POLL`] to read it, and
+/// continuously only while it's held. The polls cost about a µA in all.
 #[embassy_executor::task]
-async fn button_task(mut button: Input<'static>) {
+async fn button_task(mut enable: Output<'static>, mut button: Input<'static>) {
     loop {
-        button.wait_for_rising_edge().await;
+        enable.set_high();
+        // The line settles within a few µs of being powered (it's a switch
+        // and a pull-down); ~5 µs at 64 MHz.
+        cortex_m::asm::delay(320);
+        let pressed = button.is_high();
+        if !pressed {
+            enable.set_low();
+            Timer::after(BUTTON_POLL).await;
+            continue;
+        }
         BUTTON_HELD.store(true, Ordering::Relaxed);
         INPUT.send(InputEvent::Button).await;
         // The button is mechanical, so ignore bounces on both the press and
@@ -134,6 +154,7 @@ async fn button_task(mut button: Input<'static>) {
         Timer::after_millis(20).await;
         button.wait_for_low().await;
         BUTTON_HELD.store(false, Ordering::Relaxed);
+        enable.set_low();
         Timer::after_millis(20).await;
     }
 }
@@ -161,12 +182,34 @@ impl Backlight {
     }
 }
 
-#[embassy_executor::main]
+/// The thread-mode executor, by hand rather than `#[embassy_executor::main]`,
+/// so that the sleep between polls can work around the FPU erratum and be
+/// timed (see [`power::sleep`]).
+#[cortex_m_rt::entry]
+fn entry() -> ! {
+    static EXECUTOR: StaticCell<embassy_executor::raw::Executor> = StaticCell::new();
+    // `usize::MAX` is the context embassy's own thread-mode executor uses, so
+    // its `__pender` (a SEV) is what wakes this loop when a task is woken.
+    let executor: &'static _ =
+        EXECUTOR.init(embassy_executor::raw::Executor::new(usize::MAX as *mut ()));
+    let spawner = executor.spawner();
+    spawner.spawn(main(spawner).expect("spawned once"));
+    loop {
+        // SAFETY: Only ever polled from here, in thread mode.
+        unsafe { executor.poll() };
+        power::sleep();
+    }
+}
+
+#[embassy_executor::task]
 async fn main(spawner: Spawner) {
-    // TODO: enable the DC/DC regulator (the PineTime has the inductor for it)
-    // to roughly halve current draw; the LDO default is just the safe choice.
     firmware::clean_up_after_bootloader();
     let mut config = embassy_nrf::config::Config::default();
+    // The PineTime has the DC/DC regulator's inductors (InfiniTime, Mynewt,
+    // and TinyGo all enable it on this board). It roughly halves the
+    // nRF52's own current while the CPU or radio runs; in System ON idle the
+    // chip uses its low-power regulator regardless.
+    config.dcdc.reg1 = true;
     // The PineTime has a 32.768 kHz crystal. embassy defaults to the internal
     // RC oscillator, which is far less accurate, and the wall clock is kept by
     // RTC1 off this clock.
@@ -228,10 +271,12 @@ async fn main(spawner: Spawner) {
         high: Output::new(p.P0_23, Level::High, OutputDrive::Standard),
     };
 
-    // The button is only powered while P0.15 is driven high, presumably so
-    // firmware can cut its (small) bias current. It reads high when pressed.
-    let _button_enable = Output::new(p.P0_15, Level::High, OutputDrive::Standard);
-    spawner.spawn(button_task(Input::new(p.P0_13, Pull::None)).expect("spawned once"));
+    // The button is only powered while P0.15 is driven high, and that costs
+    // ~34 µA, so `button_task` powers it only to read it. It reads high when
+    // pressed.
+    let button_enable = Output::new(p.P0_15, Level::Low, OutputDrive::Standard);
+    spawner
+        .spawn(button_task(button_enable, Input::new(p.P0_13, Pull::None)).expect("spawned once"));
 
     // Vibration motor, active low: start off.
     let motor = Output::new(p.P0_16, Level::High, OutputDrive::Standard);
@@ -410,6 +455,9 @@ async fn main(spawner: Spawner) {
     let mut ticker = Ticker::every(Duration::from_secs(1));
     // The first reading is still settling (it's smoothed); log after a minute.
     let mut next_battery_log = Instant::now() + Duration::from_secs(60);
+    let mut last_power_report = Instant::now();
+    // Since when the panel (and backlight) have been on, for `power`.
+    let mut panel_since = Instant::now();
     loop {
         let now = Instant::now();
         let mut samples = [0i16; 1];
@@ -424,6 +472,15 @@ async fn main(spawner: Spawner) {
         // For battery-life analysis from the field log.
         if now >= next_battery_log {
             clock::log_now();
+            if now - last_power_report >= Duration::from_secs(60 * 60) {
+                if panel_awake {
+                    power::add(power::Meter::Panel, panel_since);
+                    power::add(power::Meter::Backlight, panel_since);
+                    panel_since = now;
+                }
+                power::report(now - last_power_report);
+                last_power_report = now;
+            }
             info!(
                 "battery: {} mV, {}%, charging: {}",
                 report.millivolts, report.percent, report.charging
@@ -464,15 +521,22 @@ async fn main(spawner: Spawner) {
             if !panel_awake {
                 backlight.on();
                 panel_awake = true;
+                panel_since = Instant::now();
             }
         } else if panel_awake {
             backlight.off();
+            power::add(power::Meter::Panel, panel_since);
+            power::add(power::Meter::Backlight, panel_since);
             // In sleep the ST7789 keeps its frame memory, so the canvas's
             // idea of what's on the panel stays valid across sleep/wake.
             display.sleep().await;
             panel_awake = false;
         }
-        heart::set_watching(app.showing_clock());
+        // Someone looking at the clock face gets a fresh heart rate, but one
+        // is enough: the LED (3–8 mA) needn't stay on for the rest of the
+        // glance once there's a reading from the last few minutes.
+        let fresh = heart::latest().is_some_and(|(_, at)| now - at < Duration::from_secs(5 * 60));
+        heart::set_watching(app.showing_clock() && !fresh);
         // Holding the button long enough to starve the watchdog is the
         // user's way back into the bootloader (and to rollback), even when
         // this firmware is otherwise wedged in a way that still runs this loop.
@@ -502,6 +566,14 @@ async fn main(spawner: Spawner) {
             }
             Either3::Second(event) => {
                 info!("input: {}", event);
+                if !app.awake {
+                    power::count(match event {
+                        InputEvent::Button => power::Count::WakeButton,
+                        InputEvent::Touch(_) => power::Count::WakeTouch,
+                        InputEvent::Phone(_) => power::Count::WakePhone,
+                        InputEvent::Attention => power::Count::WakeAttention,
+                    });
+                }
                 let before = app.themes;
                 app.on_input(event, Instant::now());
                 if app.themes.light != before.light {
diff --git a/src/power.rs b/src/power.rs
new file mode 100644
index 0000000..fbc4750
--- /dev/null
+++ b/src/power.rs
@@ -0,0 +1,133 @@
+//! Where the battery goes: the executor's sleep loop (with the nRF52832's
+//! FPU erratum worked around), and on-time counters for everything that
+//! draws real current, logged once an hour.
+//!
+//! The counters don't measure current; they measure *time* in each
+//! power-hungry state. Multiplied by each state's current (the table in
+//! `docs/power-plan.md`), they give an energy budget per hour that can be
+//! checked against the battery voltage's slope, and they show which knob
+//! to turn next.
+
+use core::sync::atomic::{AtomicU32, Ordering};
+
+use defmt::info;
+use embassy_time::Instant;
+
+// ==============================================================================
+// Sleeping
+// ==============================================================================
+
+/// RTC ticks (32768 Hz) spent in WFE since the last report. ISRs that run
+/// while we're "asleep" (the radio's, mostly) count as asleep; this is the
+/// thread-mode duty cycle.
+static ASLEEP_TICKS: AtomicU32 = AtomicU32::new(0);
+/// How often WFE returned since the last report. Thousands per hour is
+/// normal (timers, radio events); millions means something keeps waking the
+/// CPU, e.g. a pending interrupt nobody services.
+static WAKEUPS: AtomicU32 = AtomicU32::new(0);
+
+/// Sleeps until the next event. Call only from the thread-mode executor
+/// loop, after polling.
+pub fn sleep() {
+    // Nordic anomaly 87: once the FPU has raised an exception flag (and
+    // "inexact" is raised by almost any arithmetic), its interrupt stays
+    // pending and can wake the CPU from WFE straight away. There's no
+    // interrupt enable at the peripheral, so clear the flags and the pending
+    // bit before every sleep, as the erratum's workaround does.
+    //
+    // SAFETY: Only the cumulative exception flags (IOC, DZC, OFC, UFC, IXC,
+    // IDC) are cleared; rounding mode and the other control bits are kept.
+    unsafe {
+        let mut fpscr: u32;
+        core::arch::asm!("vmrs {}, fpscr", out(reg) fpscr, options(nomem, nostack));
+        fpscr &= !0x9F;
+        core::arch::asm!("vmsr fpscr, {}", in(reg) fpscr, options(nomem, nostack));
+    }
+    cortex_m::peripheral::NVIC::unpend(embassy_nrf::pac::Interrupt::FPU);
+
+    let before = Instant::now().as_ticks();
+    cortex_m::asm::wfe();
+    let slept = Instant::now().as_ticks().wrapping_sub(before);
+    ASLEEP_TICKS.fetch_add(slept as u32, Ordering::Relaxed);
+    WAKEUPS.fetch_add(1, Ordering::Relaxed);
+}
+
+// ==============================================================================
+// On-time counters
+// ==============================================================================
+
+/// Each power-hungry state we track, in milliseconds per report.
+#[derive(Clone, Copy, defmt::Format)]
+pub enum Meter {
+    /// Panel out of sleep (ST7789 ~5.6 mA).
+    Panel,
+    /// Backlight on at the mid level (~5.5 mA).
+    Backlight,
+    /// HRS3300 measuring, by LED drive: 12.5, 20, 30, 40 mA settings.
+    HeartLed0,
+    HeartLed1,
+    HeartLed2,
+    HeartLed3,
+    /// A phone (or other client) connected; see the logged connection
+    /// interval for what that costs.
+    Connected,
+    /// SPI flash out of deep power-down.
+    SpiFlash,
+}
+
+const METERS: usize = 8;
+static MILLIS: [AtomicU32; METERS] = [const { AtomicU32::new(0) }; METERS];
+
+/// Adds time spent in `meter`'s state.
+pub fn add(meter: Meter, since: Instant) {
+    let ms = since.elapsed().as_millis() as u32;
+    MILLIS[meter as usize].fetch_add(ms, Ordering::Relaxed);
+}
+
+/// Event counts per report.
+#[derive(Clone, Copy, defmt::Format)]
+pub enum Count {
+    WakeButton,
+    WakeTouch,
+    WakePhone,
+    WakeAttention,
+    HeartSessions,
+    Disconnects,
+}
+
+const COUNTS: usize = 6;
+static EVENTS: [AtomicU32; COUNTS] = [const { AtomicU32::new(0) }; COUNTS];
+
+pub fn count(event: Count) {
+    EVENTS[event as usize].fetch_add(1, Ordering::Relaxed);
+}
+
+/// Logs and resets everything. Call about once an hour (from the UI loop),
+/// right before the battery line, so the two can be lined up.
+pub fn report(period: embassy_time::Duration) {
+    let asleep = ASLEEP_TICKS.swap(0, Ordering::Relaxed) as u64;
+    let total = period.as_ticks().max(1);
+    let take = |a: &AtomicU32| a.swap(0, Ordering::Relaxed);
+    info!(
+        "power: awake {=u64}‰, {=u32} wakeups; ms: panel {=u32}, backlight {=u32}, \
+         led {=u32}/{=u32}/{=u32}/{=u32}, connected {=u32}, flash {=u32}; \
+         wakes: button {=u32}, touch {=u32}, phone {=u32}, attention {=u32}; \
+         hr sessions {=u32}, disconnects {=u32}",
+        1000 - asleep.min(total) * 1000 / total,
+        WAKEUPS.swap(0, Ordering::Relaxed),
+        take(&MILLIS[Meter::Panel as usize]),
+        take(&MILLIS[Meter::Backlight as usize]),
+        take(&MILLIS[Meter::HeartLed0 as usize]),
+        take(&MILLIS[Meter::HeartLed1 as usize]),
+        take(&MILLIS[Meter::HeartLed2 as usize]),
+        take(&MILLIS[Meter::HeartLed3 as usize]),
+        take(&MILLIS[Meter::Connected as usize]),
+        take(&MILLIS[Meter::SpiFlash as usize]),
+        take(&EVENTS[Count::WakeButton as usize]),
+        take(&EVENTS[Count::WakeTouch as usize]),
+        take(&EVENTS[Count::WakePhone as usize]),
+        take(&EVENTS[Count::WakeAttention as usize]),
+        take(&EVENTS[Count::HeartSessions as usize]),
+        take(&EVENTS[Count::Disconnects as usize]),
+    );
+}
diff --git a/src/spi_flash.rs b/src/spi_flash.rs
index e85619d..4b91f51 100644
--- a/src/spi_flash.rs
+++ b/src/spi_flash.rs
@@ -71,13 +71,19 @@ pub struct SpiFlash {
     /// several users share the chip and each sleeps it when done, every
     /// operation wakes it first if needed.
     awake: bool,
+    /// Since when it's been awake, for [`crate::power`].
+    awake_since: Instant,
 }
 
 impl SpiFlash {
     /// Takes the chip-select (P0.05) and puts the chip to sleep. The
     /// bootloader may have left it awake.
     pub async fn new(cs: Output<'static>) -> Self {
-        let mut flash = Self { cs, awake: true };
+        let mut flash = Self {
+            cs,
+            awake: true,
+            awake_since: Instant::now(),
+        };
         flash.sleep().await;
         flash
     }
@@ -98,6 +104,9 @@ impl SpiFlash {
 
     pub async fn sleep(&mut self) {
         self.command(&[DEEP_POWER_DOWN], &mut []).await;
+        if self.awake {
+            crate::power::add(crate::power::Meter::SpiFlash, self.awake_since);
+        }
         self.awake = false;
     }
 
@@ -107,6 +116,7 @@ impl SpiFlash {
             // tRES1: at most 20 µs before it accepts commands again.
             Timer::after_micros(30).await;
             self.awake = true;
+            self.awake_since = Instant::now();
         }
     }
 
```
