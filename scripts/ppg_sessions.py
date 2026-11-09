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

Usage: python -I scripts/ppg_sessions.py HEALTH_CONNECT_DB DECODED_LOG OUT

The Garmin is app 9 in the exports this was written against; check
`application_info_table` in yours.
"""
import re
import sqlite3
import sys

db = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
log = open(sys.argv[2]).read().splitlines()
GARMIN = 9
clock = [l for l in log if re.search(r"clock: \d+ UTC", l)][-1]
BASE = int(re.search(r"clock: (\d+) UTC", clock)[1]) - float(clock.split()[0])

ghr = list(db.execute(
    "select s.epoch_millis/1000.0, s.beats_per_minute from heart_rate_record_series_table s "
    "join heart_rate_record_table r on s.parent_key = r.row_id where r.app_info_id = ? "
    "and s.epoch_millis between ? and ? order by 1",
    (GARMIN, BASE * 1000, (BASE + float(log[-1].split()[0]) + 600) * 1000)))


def garmin(t):
    near = [(abs(u - t), b) for u, b in ghr if abs(u - t) <= 90]
    return min(near)[1] if near else 0


segments, current, period, asleep = [], None, 37500.0, False
# When the last background session was due: its samples follow within a
# second or two.
due_at = float("-inf")
for line in log:
    up = float(line.split()[0])
    if "heart: due (" in line:
        due_at = up
    if "sleep: asleep now" in line:
        asleep = True
    elif "sleep: awake now" in line:
        asleep = False
    # A reading is logged before the session's last part-second of samples
    # (0.9.1 on; earlier logs don't have it), so its segment closes at the
    # next heart line that isn't samples.
    if current and current.get("read") and "heart: " in line and "heart: samples" not in line:
        segments.append(current)
        current = None
    if m := re.search(r"heart: level .* period ([\d.]+)", line):
        period = float(m[1])
    elif m := re.search(r"heart: samples \[([\d, ]*)\]", line):
        if current is None:
            current = {"t": BASE + up - 1, "samples": [], "movement": [], "outcome": "-",
                       "asleep": asleep, "background": up - due_at < 3}
        current["samples"] += [int(v) for v in m[1].split(", ") if v]
        if mv := re.search(r"movement (\d+)", line):
            current["movement"].append(mv[1])
        current["period"] = period
        if current.get("read"):
            segments.append(current)
            current = None
    elif current and (m := re.search(r"INFO  heart: (\d+) BPM after", line)):
        current["outcome"] = m[1]
        current["read"] = True
    elif current and ("switching to level" in line or re.search(r"INFO  heart: (no reading|not on|gave up|stopped)", line)):
        if re.search(r"no reading|gave up|stopped", line):
            current["outcome"] = "none"
        segments.append(current)
        current = None
if current:
    segments.append(current)

with open(sys.argv[3], "w") as out:
    for s in segments:
        mid = s["t"] + len(s["samples"]) * s["period"] / 2e6
        flags = [f for f in ("asleep", "background") if s[f]]
        out.write(f"{s['t']:.0f} {garmin(mid)} {s['period']:.1f} {s['outcome']} "
                  + " ".join(map(str, s["samples"])) + " | " + " ".join(s["movement"])
                  + (" | " + " ".join(flags) if flags else "") + "\n")
print(f"{len(segments)} segments, {sum(len(s['samples']) for s in segments)} samples")
