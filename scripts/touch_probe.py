#!/usr/bin/env python3
"""Find out whether the touch controller tells a fingertip from a bigger
contact, from a dev kit running a `touch-probe` build (see
src/touch_probe.rs), which logs every report's raw registers over RTT.

A session, with the dev kit on the desk and its screen on:

    RTT_SECS=900 cargo run --release --features touch-probe \\
      2>&1 | tee tmp/touch-probe.log
    python3 scripts/touch_probe.py guide        # in a second terminal
    python3 scripts/touch_probe.py analyse tmp/touch-probe.log

`guide` walks whoever is touching through the kinds of contact, ten of
each, with a pause of PAUSE seconds between kinds (and none that long
within one). `analyse` splits the log into kinds at those pauses, and
into touches at each lift, then prints:

- the chip's IDs and settings, as dumped at boot;
- per kind, each register's spread over the reports while touched (raw
  bytes, plus the packed fields: event, touch ID, X, Y, BPC0/1), the
  registers that never change folded into one line;
- per kind, each touch's shape: how long, how many reports, and how far
  the position drifted from where it landed.

A register that tells contacts apart shows up as a row whose spread
differs between the fingertip kinds and the rest.
"""

import argparse
import csv
import itertools
import math
import re
import statistics
import sys
import time

# The kinds of contact, in the order `guide` asks for them: short name,
# instructions.
KINDS = [
    ("tap", "fingertip taps, about a second apart"),
    ("hold", "fingertip held still for ~2 s, then lifted, ~1 s apart"),
    ("knuckle", "a knuckle pressed on for ~1 s, ~1 s apart"),
    ("thumb", "a flat thumb (its whole pad) for ~1 s, ~1 s apart"),
    ("palm", "the palm or forearm resting on the screen for ~2 s, ~1 s apart"),
    ("brush", "a forearm brushing across the screen's edge, as a sleeve would"),
]
REPEATS = 10
# Between kinds, hands off for this long; within one, never as long.
PAUSE = 12

REPORT = re.compile(r"touch-probe: #(\d+) (\d+)ms \[([0-9a-fA-F, ]*)\]")
SETTINGS = re.compile(r"touch-probe: regs ([0-9a-fA-F]+): \[([0-9a-fA-F, ]*)\]")
ANSI = re.compile(r"\x1b\[[0-9;]*m")

# What's known (or guessed) of each register in a report: see
# src/touch_probe.rs. Indices 16..19 are 0xB0..0xB3.
NAMES = {
    0x00: "mode?",
    0x01: "gesture",
    0x02: "fingers",
    0x03: "event|X hi",
    0x04: "X lo",
    0x05: "ID|Y hi",
    0x06: "Y lo",
    0x07: "pressure?",
    0x08: "misc/area?",
    0x09: "pt2 event|X hi",
    0x0A: "pt2 X lo",
    0x0B: "pt2 ID|Y hi",
    0x0C: "pt2 Y lo",
    0x0D: "pt2 pressure?",
    0x0E: "pt2 misc?",
}
SETTING_NAMES = {
    0xA5: "PowerMode",
    0xA6: "FwVersion(wasp)",
    0xA7: "ChipID",
    0xA8: "ProjID",
    0xA9: "FwVersion",
    0xEC: "MotionMask",
    0xED: "IrqPulseWidth",
    0xEE: "NorScanPer",
    0xEF: "MotionSlAngle",
    0xF0: "LpScanRaw1H",
    0xF1: "LpScanRaw1L",
    0xF2: "LpScanRaw2H",
    0xF3: "LpScanRaw2L",
    0xF4: "LpAutoWakeTime",
    0xF5: "LpScanTH",
    0xF6: "LpScanWin",
    0xF7: "LpScanFreq",
    0xF8: "LpScanIdac",
    0xF9: "AutoSleepTime",
    0xFA: "IrqCtl",
    0xFB: "AutoReset",
    0xFC: "LongPressTime",
    0xFD: "IOCtl",
    0xFE: "DisAutoSleep",
}
# Reports this far apart are separate touches even without a lift between
# them (the firmware's `LIFT_TIMEOUT`).
LIFT_TIMEOUT_MS = 250


def guide(args):
    """Walks the person touching the dev kit through the protocol."""
    print(
        f"{len(KINDS)} kinds of contact, {args.repeats} of each. Between kinds, "
        f"keep everything off the screen for {args.pause} s (counted down here); "
        f"within a kind, never pause that long.\n"
        "Keep the screen on (a button press wakes it, but then wait out a "
        "pause before starting)."
    )
    for i, (name, how) in enumerate(KINDS, 1):
        input(f"\n[{i}/{len(KINDS)}] {name}: {how}. Enter to start. ")
        input(f"  Go: {args.repeats} × {name}. Enter when done. ")
        for left in range(args.pause, 0, -1):
            print(f"\r  Hands off: {left:2d} s ", end="", flush=True)
            time.sleep(1)
        print("\r  Done.            ")
    print("\nAll done: stop the capture (or let RTT_SECS run out), then `analyse`.")


def parse(path):
    """The log's reports, as (seq, ms, bytes), and its settings dumps."""
    reports, settings = [], {}
    with open(path, encoding="utf-8", errors="replace") as log:
        for line in log:
            line = ANSI.sub("", line)
            if m := REPORT.search(line):
                data = [int(b, 16) for b in m[3].replace(",", " ").split()]
                reports.append((int(m[1]), int(m[2]), data))
            elif m := SETTINGS.search(line):
                start = int(m[1], 16)
                data = [int(b, 16) for b in m[2].replace(",", " ").split()]
                for i, value in enumerate(data):
                    settings.setdefault(start + i, set()).add(value)
    return reports, settings


def lifted(data):
    """Whether a report is a lift, as src/touch.rs decides."""
    return data[0x03] >> 6 == 1 or data[0x02] == 0


def fields(data):
    """The packed fields worth their own row: (name, value)."""
    out = [
        ("event", data[0x03] >> 6),
        ("X", (data[0x03] & 0x0F) << 8 | data[0x04]),
        ("touch ID", data[0x05] >> 4),
        ("Y", (data[0x05] & 0x0F) << 8 | data[0x06]),
    ]
    if len(data) >= 20:
        out += [("BPC0", data[16] << 8 | data[17]), ("BPC1", data[18] << 8 | data[19])]
    return out


def split(reports, gap_ms):
    """Splits reports into groups at gaps of `gap_ms`, and each group into
    touches: a press, its contacts, and its lift (with the chip's one
    trailing report after it)."""
    groups = []
    for report in reports:
        if not groups or report[1] - groups[-1][-1][-1][1] >= gap_ms:
            groups.append([[]])
        touches = groups[-1]
        last = touches[-1][-1] if touches[-1] else None
        # A new touch: after a lift, or when reports dried up.
        if last and (
            report[1] - last[1] > LIFT_TIMEOUT_MS
            or (lifted(last[2]) and not lifted(report[2]))
        ):
            touches.append([])
        touches[-1].append(report)
    return groups


def spread(values):
    if not values:
        return "-"
    lo, hi = min(values), max(values)
    if lo == hi:
        return f"={lo}"
    return f"{lo}..{statistics.median(values):g}..{hi}"


def analyse(args):
    reports, settings = parse(args.log)
    if not reports:
        sys.exit(f"no `touch-probe:` reports in {args.log}")

    # Lines the RTT buffer dropped (the firmware numbers every report).
    # A reboot restarts the numbering.
    dropped = sum(
        b[0] - a[0] - 1 for a, b in itertools.pairwise(reports) if b[0] > a[0] + 1
    )
    print(f"{len(reports)} reports; {dropped} dropped by RTT")
    if dropped:
        print("  (a bigger buffer drops fewer: DEFMT_RTT_BUFFER_SIZE=4096 on the build)")

    if settings:
        print("\nChip settings (as dumped after configuring; several values if they changed):")
        for reg, name in SETTING_NAMES.items():
            if reg in settings:
                values = "/".join(f"{v:#04x}" for v in sorted(settings[reg]))
                print(f"  {reg:#04x} {name:16} {values}")
        others = [
            f"{reg:#04x}={'/'.join(f'{v:02x}' for v in sorted(vs))}"
            for reg, vs in sorted(settings.items())
            if reg not in SETTING_NAMES and vs != {0}
        ]
        if others:
            print("  undocumented, non-zero: " + " ".join(others))

    groups = split(reports, args.gap * 1000)
    names = args.kinds.split(",") if args.kinds else [k for k, _ in KINDS]
    if len(groups) != len(names):
        print(
            f"\n{len(groups)} groups (split at {args.gap} s pauses), but "
            f"{len(names)} kinds: numbering them instead (see --gap, --kinds)"
        )
        names = [f"group{i + 1}" for i in range(len(groups))]

    # Per kind, the reports while touched: lifts (and the chip's trailing
    # report after one) say where a touch ended, not what touched.
    touched = {
        name: [r[2] for t in touches for r in t if not lifted(r[2])]
        for name, touches in zip(names, groups)
    }

    width = max(len(n) for n in names)
    col = max(16, width)
    print("\nPer kind, each register while touched (min..median..max, or =value):")
    header = f"  {'register':22}" + "".join(f"{n:>{col}}" for n in names)
    print(header)
    constant = []
    nbytes = max(len(r[2]) for r in reports)
    for i in range(nbytes):
        reg = i if i < 16 else 0xB0 + i - 16
        label = f"{reg:#04x} {NAMES.get(reg, '')}".strip()
        columns = [[d[i] for d in touched[n] if len(d) > i] for n in names]
        values = {v for c in columns for v in c}
        if len(values) <= 1:
            constant.append(f"{reg:#04x}={next(iter(values), 0):02x}")
            continue
        print(f"  {label:22}" + "".join(f"{spread(c):>{col}}" for c in columns))
    for k, (field, _) in enumerate(fields([0] * nbytes)):
        columns = [[fields(d)[k][1] for d in touched[n] if len(fields(d)) > k] for n in names]
        print(f"  {field:22}" + "".join(f"{spread(c):>{col}}" for c in columns))
    if constant:
        print("  never changed: " + " ".join(constant))

    print("\nPer kind, each touch (median, max over its touches):")
    print(f"  {'':10}{'touches':>8}{'ms':>14}{'reports':>12}{'drift px':>12}{'net px':>12}")
    for name, touches in zip(names, groups):
        shapes = []
        for t in touches:
            down = [r for r in t if not lifted(r[2])]
            if not down:
                continue
            points = [((d[3] & 0x0F) << 8 | d[4], (d[5] & 0x0F) << 8 | d[6]) for _, _, d in down]
            x0, y0 = points[0]
            drift = max(math.dist((x0, y0), p) for p in points)
            net = math.dist(points[0], points[-1])
            shapes.append((t[-1][1] - t[0][1], len(down), drift, net))
        if not shapes:
            print(f"  {name:10}{0:>8}")
            continue

        cells = [
            f"{statistics.median(vs):.0f}, {max(vs):.0f}" for vs in zip(*shapes)
        ]
        print(
            f"  {name:10}{len(shapes):>8}{cells[0]:>14}"
            + "".join(f"{c:>12}" for c in cells[1:])
        )

    if args.csv:
        with open(args.csv, "w", newline="") as out:
            writer = csv.writer(out)
            regs = [f"r{(i if i < 16 else 0xB0 + i - 16):02x}" for i in range(nbytes)]
            writer.writerow(["kind", "touch", "seq", "ms", "lifted", *regs])
            for name, touches in zip(names, groups):
                for n, t in enumerate(touches):
                    for seq, ms, d in t:
                        writer.writerow([name, n, seq, ms, int(lifted(d)), *d])
        print(f"\nEvery report: {args.csv}")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = parser.add_subparsers(required=True)
    g = sub.add_parser("guide", help="walk through the protocol, with timed pauses")
    g.add_argument("--repeats", type=int, default=REPEATS)
    g.add_argument("--pause", type=int, default=PAUSE, help="seconds between kinds")
    g.set_defaults(run=guide)
    a = sub.add_parser("analyse", help="per-kind statistics from a captured log")
    a.add_argument("log", help="defmt-print's output, as tee'd during the session")
    a.add_argument(
        "--gap",
        type=float,
        default=PAUSE / 2,
        help="seconds without reports that separate kinds (default: %(default)s)",
    )
    a.add_argument("--kinds", help="comma-separated names for the groups, in order")
    a.add_argument("--csv", help="also write every report, labelled, to this file")
    a.set_defaults(run=analyse)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
