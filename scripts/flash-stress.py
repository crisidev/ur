#!/usr/bin/env python3
"""Stress the internal flash's timeslots on a dev kit, under heavy probe reads.

    flash-stress.py ELF [--minutes M] [--light | --halting]

ELF is a build with the `flash-stress` feature, already flashed and running
(`cargo run --release --features flash-stress`, then Ctrl-C once it boots).
Its `flash_stress` task saves to storage back to back, so a page erases
every few seconds. Meanwhile this reads 2.4 KB of the dev kit's RAM over
the probe as fast as the probe goes (~25 times a second, with `esprobe
fast-dump`; `--light` reads only the counters, every 2 s), and watches
the task's counters (`UR_FLASH_STRESS`):

- a reset (the task stops running, or its save count drops): an erase or
  write that overran its timeslot makes MPSL assert, for one. Once it's
  back, the boot's log, which says why, is read over RTT and the lines
  that matter printed.
- failed saves, and stalls (no save finishing for 10 s).
- how long erases took: the longest and shortest in real time, and the
  longest as far as the slot's own timer (TIMER0) counted.

`--halting` reads with `esprobe core read` instead, as `screenshot.py`
and `ur-sim --devkit` once did. Each such read is a probe-rs session, which
halts the core for ~20 ms as it starts and again as it ends (to clear
breakpoints). A halt that lands in a timeslot overruns it, and MPSL
asserts: expect resets, from any flash driver. That's the probe, not the
firmware; a watch without one never halts.

Exits non-zero if the dev kit reset, a save failed or saving stalled.
Needs `esprobe`, `llvm-nm` and `defmt-print` on PATH.
"""

import argparse
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = ["esprobe", "--target", "nRF52832_xxAA"]
RAM_END = 0x20010000
# As much as `ur-sim --devkit` reads per frame.
HEAVY_WORDS = 600
COUNTERS = ["saves", "failures", "erases", "longest_us", "shortest_us", "longest_timer0_us"]
# What the task's last word holds once it runs (b"STRS", little-endian).
MARKER = int.from_bytes(b"STRS", "little")


def symbol(elf: Path, name: str) -> int:
    out = subprocess.run(["llvm-nm", str(elf)], capture_output=True, text=True, check=True)
    for line in out.stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] == name:
            return int(parts[0], 16)
    sys.exit(f"no symbol {name} in {elf}: not a flash-stress build?")


def read_words(address: int, count: int, halting: bool) -> list[int] | None:
    """Reads words over the probe; None if the probe failed (it does, now
    and then, and the dev kit resetting under it is one way)."""
    if halting:
        out = subprocess.run(
            TARGET + ["core", "read", hex(address), "--words", str(count)],
            capture_output=True,
            text=True,
            check=False,
        )
        values = [
            int(line.split()[1], 16) for line in out.stdout.splitlines() if line.startswith("0x")
        ]
        return values if out.returncode == 0 and len(values) == count else None
    # The bridge's bulk path, which never halts the core.
    dump = ROOT / "tmp" / "flash-stress.bin"
    dump.parent.mkdir(exist_ok=True)
    out = subprocess.run(
        TARGET
        + ["fast-dump", "--address", str(address), "--size", str(4 * count), str(dump)],
        capture_output=True,
        check=False,
    )
    data = dump.read_bytes() if out.returncode == 0 else b""
    if len(data) != 4 * count:
        return None
    return [int.from_bytes(data[i : i + 4], "little") for i in range(0, len(data), 4)]


def boot_log(elf: Path) -> list[str]:
    """The dev kit's log since it booted, as far as RTT's buffer kept it
    (defmt-rtt drops what doesn't fit, so the start survives)."""
    control_block = hex(symbol(elf, "_SEGGER_RTT"))
    rtt = subprocess.Popen(
        TARGET + ["rtt", "--control-block", control_block, "--idle-ms", "1500"],
        stdout=subprocess.PIPE,
    )
    decoded = subprocess.run(
        ["defmt-print", "-e", str(elf)],
        stdin=rtt.stdout,
        capture_output=True,
        text=True,
        check=False,
    )
    rtt.wait()
    return decoded.stdout.splitlines()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("elf", type=Path)
    parser.add_argument("--minutes", type=float, default=10)
    reads = parser.add_mutually_exclusive_group()
    reads.add_argument("--light", action="store_true", help="read only the counters, every 2 s")
    reads.add_argument(
        "--halting", action="store_true", help="read as `core read` does, halting the core"
    )
    args = parser.parse_args()

    counters_at = symbol(args.elf, "UR_FLASH_STRESS")
    # The counters, and RAM after them, as one read.
    words = len(COUNTERS) + 1 if args.light else HEAVY_WORDS
    start = min(counters_at, RAM_END - 4 * words)
    offset = (counters_at - start) // 4

    began = time.monotonic()
    last_report = began
    reads = failed_reads = 0
    resets: list[list[str]] = []
    # The last counters read from a running task; None while it's down
    # (resetting, or in the bootloader).
    previous: list[int] | None = None
    down = False
    progress_at = began
    stalls = 0
    # Totals over every run between resets.
    totals = [0, 0, 0]
    longest = longest_timer0 = 0
    shortest = None

    def finish_run(counters: list[int]) -> None:
        nonlocal longest, longest_timer0, shortest
        for i in range(3):
            totals[i] += counters[i]
        longest = max(longest, counters[3])
        longest_timer0 = max(longest_timer0, counters[5])
        if counters[4]:
            shortest = counters[4] if shortest is None else min(shortest, counters[4])

    def reset_seen(counters: list[int]) -> None:
        nonlocal down
        finish_run(counters)
        print(f"{time.monotonic() - began:7.1f} s  RESET after {counters[0]} saves", flush=True)
        down = True

    while time.monotonic() - began < args.minutes * 60:
        values = read_words(start, words, args.halting)
        reads += 1
        now = time.monotonic()
        if values is None:
            failed_reads += 1
            time.sleep(0.5)
            continue
        counters = values[offset : offset + len(COUNTERS)]
        running = values[offset + len(COUNTERS)] == MARKER
        if not running:
            if previous is not None and not down:
                reset_seen(previous)
            time.sleep(0.5)
            continue
        if previous is not None and counters[0] < previous[0] and not down:
            # Back up before we saw it down.
            reset_seen(previous)
        if down:
            # Up again: why did it reset? Its boot log says.
            down = False
            log = [
                line
                for line in boot_log(args.elf)
                if any(k in line for k in ("reset reason", "panicked", "hard-faulted", "hung"))
            ]
            for line in log:
                print(f"           {line}", flush=True)
            resets.append(log)
            previous = None
            progress_at = now
            continue
        if previous is None or counters[0] > previous[0]:
            progress_at = now
        elif now - progress_at > 10:
            # No save finished in 10 s: a flash operation that never
            # completes would hang every later save.
            stalls += 1
            print(f"{now - began:7.1f} s  STALLED at {counters[0]} saves", flush=True)
            progress_at = now
        previous = counters
        if now - last_report >= 30:
            last_report = now
            status = ", ".join(f"{n} {v}" for n, v in zip(COUNTERS, counters))
            print(f"{now - began:7.1f} s  {status}", flush=True)
        if args.light:
            time.sleep(2)

    if previous is not None and not down:
        finish_run(previous)
    minutes = (time.monotonic() - began) / 60
    print(
        f"\n{minutes:.1f} min, {reads} probe reads of {4 * words} B ({failed_reads} failed): "
        f"{totals[0]} saves, {totals[1]} failed, {totals[2]} erases, "
        f"{len(resets)} resets, {stalls} stalls"
    )
    if totals[2]:
        print(
            f"erases took {shortest}–{longest} µs (real time); "
            f"the longest ended {longest_timer0} µs into its slot by TIMER0"
        )
    sys.exit(1 if resets or stalls or totals[1] else 0)


if __name__ == "__main__":
    main()
