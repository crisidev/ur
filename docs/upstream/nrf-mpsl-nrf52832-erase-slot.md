# nrf-mpsl: `Flash` books too short a timeslot for an nRF52832 page erase

File at: <https://github.com/alexmoon/nrf-sdc/issues/new>

**Title:** `nrf-mpsl`: `Flash` erase timeslot on nRF52832 is shorter than the chip's worst-case page erase

`nrf-mpsl/src/flash.rs` (0.4.0 and `main`) uses `ERASE_PAGE_DURATION_US = 85_000`
for all `nrf52` targets. That is the nRF52840's tERASEPAGE maximum. The
nRF52832 has no partial erase, so its slot must hold a whole page erase, and
its product specification ([NVMC electrical specification][ps]) gives:

| | typ | max |
|---|---|---|
| tERASEPAGE | 2.05 ms | **89.7 ms** |
| tWRITE | 67.5 µs | **338 µs** |

The slot is `85_000 + TIMESLOT_SLACK_US` (1 ms) = 86 ms, so a worst-case
erase overruns it by ~3.7 ms, and MPSL's `OVERSTAYED` signal panics
("Used too much of our timeslot").

`WRITE_WORD_DURATION_US = 41` is the nRF52840's tWRITE too; on the nRF52832 a
word can take 338 µs. The 1 ms slack covers that today, but only by accident.

**Suggested fix:** under `feature = "nrf52832"`, use 89_700 µs and 338 µs.

Seen on a PineTime (nRF52832). Typical erases measure 2.04 ms, so this is a
worst-case margin bug, not a frequent failure.

[ps]: https://docs.nordicsemi.com/bundle/ps_nrf52832/page/nvmc.html
