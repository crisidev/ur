"""Extract raw PPG sessions from a decoded field log (0.8.13+, ur::heart=trace)
for host-tests/ppg's replay, with the Garmin's heart rate alongside.

Writes one line per segment (samples at one level, uninterrupted):

    <utc start> <garmin bpm or 0> <conversion period µs> <outcome> <sample> ... | <movement> ...

with the wrist's movement (`activity::movement`) as the firmware saw it at
each second's level check, so the replay can veto windows as it does, and
a third section with `asleep` if the sleep classifier said so when the
segment started (logs from before it, 0.8.x, never do), and `background`
if the schedule started the session, rather than someone looking at the
clock face (logs from before 0.9.0's `heart: due` line never say so).
The third section also has the LED's `drive=N` (0-3: 12.5-40 mA; logs
before 0.10's brighter phases have it from the level lines too), and
`brighter` for a segment that went on with a brighter LED after the one
before it ran out (see `heart_schedule::brighter`): the same session, its
flags carried over. A segment after a level change within a session
(a bright start stepping back down, say) carries them over too, marked
`cont`. Awake background sessions from 0.10's LED experiment (see
`heart_schedule::Arm`) say their arm, `arm=dim` or `arm=bright`, and
background sessions after 0.10.0-rc.3 their round of tries, `round=N`
(numbered from boot, wrapping at 65536), whose sessions share an arm.
Background sessions in a logging build after 0.10.0 say their length arm
too (see `heart_schedule::Length`), `length=longer` or `length=usual`: a
longer one on a busy wrist may run 90 s rather than 30, which its
segment's samples show; and a longer one that reads on a busy wrist
records on to 90 s (`heart: recording on`, then `heart: recorded to`),
its later readings logged as `record-only` but not counted, so its
segment holds all 90 s, its outcome the first reading. (The same logs'
`accel:` lines, the accelerometer's samples through each session, aren't
extracted here.)

Usage: python -I scripts/ppg_sessions.py HEALTH_CONNECT_DB DECODED_LOG OUT

The Garmin is Garmin Connect's app in the export's
`application_info_table`.

A log may hold several boots (an update, a reset), each with uptime from
0 again: each boot's segments are dated by that boot's last `clock: N
UTC` line (after any `clock: was off by` correction), and a boot with
none is skipped. Lines without a leading timestamp (defmt-print's
`(HOST) malformed frame skipped`) are skipped.
"""
import re
import sqlite3
import sys

db = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
GARMIN = db.execute(
    "select row_id from application_info_table where package_name = ?",
    ("com.garmin.android.apps.connectmobile",)).fetchone()
if GARMIN is None:
    sys.exit(f"{sys.argv[1]}: no Garmin Connect in application_info_table")
GARMIN = GARMIN[0]

# ==============================================================================
# The log, by boot
# ==============================================================================

# Uptime restarts at each boot, so the log's UTC base (UTC minus uptime) is
# a boot's own. A boot starts at its `ur <version> booting` line or, if the
# ring overwrote that, where uptime falls back.
boots = []  # [[(uptime, line)], ...]
last_up = None
with open(sys.argv[2], encoding="utf-8", errors="replace") as f:
    raw = f.read().splitlines()
for line in raw:
    if not re.match(r"\d+\.\d+ ", line):
        continue
    up = float(line.split()[0])
    if not boots or re.search(r"INFO  ur \S+ booting", line) or up + 5 < last_up:
        boots.append([])
    boots[-1].append((up, line))
    last_up = up
log = []  # (boot, its UTC base, uptime, line)
for n, lines in enumerate(boots):
    clocks = [(up, int(m[1])) for up, l in lines if (m := re.search(r"clock: (\d+) UTC", l))]
    if not clocks:
        print(f"boot {n + 1} of {len(boots)}: no clock line, so its {len(lines)} lines are "
              "skipped", file=sys.stderr)
        continue
    # The last clock line is after every correction the boot made (the
    # first comes a few seconds after boot, once the phone says the time).
    base = clocks[-1][1] - clocks[-1][0]
    log += [(n, base, up, line) for up, line in lines]
if not log:
    sys.exit(f"{sys.argv[2]}: no boot with a clock line")

ghr = list(db.execute(
    "select s.epoch_millis/1000.0, s.beats_per_minute from heart_rate_record_series_table s "
    "join heart_rate_record_table r on s.parent_key = r.row_id where r.app_info_id = ? "
    "and s.epoch_millis between ? and ? order by 1",
    (GARMIN, min(b + u for _, b, u, _ in log) * 1000, (max(b + u for _, b, u, _ in log) + 600) * 1000)))


def garmin(t):
    near = [(abs(u - t), b) for u, b in ghr if abs(u - t) <= 90]
    return min(near)[1] if near else 0


segments, current, period, asleep, drive = [], None, 37500.0, False, 0
# The segment a brighter phase, or a new level, carries on from, until its
# samples start, and which of those it is.
carried, carried_flag = None, None
# The arm, round and length of the session in progress, from its `due`
# line.
arm, round_, length = None, None, None
# Whether a session that read is recording on (see above).
recording = False
# When the last background session was due: its samples follow within a
# second or two.
due_at = float("-inf")
boot = None
for n, base, up, line in log:
    if n != boot:
        # A reboot cuts the session under way short, and starts afresh.
        if current:
            segments.append(current)
        boot, current, carried, recording, due_at = n, None, None, False, float("-inf")
        period, asleep, drive, arm, round_, length = 37500.0, False, 0, None, None, None
    if "heart: due (" in line:
        due_at = up
        m = re.search(r"; (?:round (\d+) )?(Asleep|Dim|Bright) drive \d(?: (Usual|Longer))?", line)
        round_ = m[1] if m else None
        arm = m[2].lower() if m and m[2] != "Asleep" else None
        length = m[3].lower() if m and m[3] else None
    if "sleep: asleep now" in line:
        asleep = True
    elif "sleep: awake now" in line:
        asleep = False
    # A reading is logged before the session's last part-second of samples
    # (0.9.1 on; earlier logs don't have it), so its segment closes at the
    # next heart line that isn't samples.
    if "heart: recording on" in line:
        recording = True
    elif "heart: recorded to" in line:
        recording = False
        if current:
            segments.append(current)
        current = None
    if current and current.get("read") and not recording and "heart: " in line \
            and "heart: samples" not in line:
        segments.append(current)
        current = None
    if m := re.search(r"heart: level .*drive: (\d).* period ([\d.]+)", line):
        drive, period = int(m[1]), float(m[2])
    elif m := re.search(r"heart: samples \[([\d, ]*)\]", line):
        if current is None:
            current = {"t": base + up - 1, "samples": [], "movement": [], "outcome": "-",
                       "asleep": asleep, "background": up - due_at < 3,
                       "drive": drive, "brighter": False, "cont": False,
                       "arm": arm if up - due_at < 3 else None,
                       "round": round_ if up - due_at < 3 else None,
                       "length": length if up - due_at < 3 else None}
            if carried:
                current.update(asleep=carried["asleep"], background=carried["background"],
                               arm=carried["arm"], round=carried["round"],
                               length=carried["length"])
                current[carried_flag] = True
                carried = None
        current["samples"] += [int(v) for v in m[1].split(", ") if v]
        if mv := re.search(r"movement (\d+)", line):
            current["movement"].append(mv[1])
        current["period"] = period
        if current.get("read") and not recording:
            segments.append(current)
            current = None
    elif current and (m := re.search(r"INFO  heart: (\d+) BPM after", line)):
        current["outcome"] = m[1]
        current["read"] = True
    elif current and "heart: brighter after" in line:
        # The phase before ran out without a reading.
        current["outcome"] = "none"
        segments.append(current)
        carried, carried_flag, current = current, "brighter", None
    elif current and "switching to level" in line:
        segments.append(current)
        carried, carried_flag, current = current, "cont", None
    elif re.search(r"INFO  heart: (no reading|not on|gave up|stopped|too much)", line):
        if current:
            if re.search(r"no reading|gave up|stopped", line):
                current["outcome"] = "none"
            segments.append(current)
        current, carried = None, None
if current:
    segments.append(current)

with open(sys.argv[3], "w") as out:
    for s in segments:
        mid = s["t"] + len(s["samples"]) * s["period"] / 2e6
        flags = [f for f in ("asleep", "background", "brighter", "cont") if s[f]]
        flags.append(f"drive={s['drive']}")
        if s["arm"]:
            flags.append(f"arm={s['arm']}")
        if s["round"]:
            flags.append(f"round={s['round']}")
        if s["length"]:
            flags.append(f"length={s['length']}")
        out.write(f"{s['t']:.0f} {garmin(mid)} {s['period']:.1f} {s['outcome']} "
                  + " ".join(map(str, s["samples"])) + " | " + " ".join(s["movement"])
                  + " | " + " ".join(flags) + "\n")
print(f"{len(segments)} segments, {sum(len(s['samples']) for s in segments)} samples")
