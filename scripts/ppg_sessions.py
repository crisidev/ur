"""Extract raw PPG sessions from a decoded field log (0.8.13+, ur::heart=trace)
for host-tests/ppg's replay, with the Garmin's heart rate alongside.

Writes one line per segment (samples at one level, uninterrupted):

    <utc start> <garmin bpm or 0> <conversion period µs> <outcome> <sample> ...

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


segments, current, period = [], None, 37500.0
for line in log:
    up = float(line.split()[0])
    if m := re.search(r"heart: level .* period ([\d.]+)", line):
        period = float(m[1])
    elif m := re.search(r"heart: samples \[([\d, ]*)\]", line):
        if current is None:
            current = {"t": BASE + up - 1, "samples": [], "outcome": "-"}
        current["samples"] += [int(v) for v in m[1].split(", ") if v]
        current["period"] = period
    elif current and ("switching to level" in line or re.search(r"INFO  heart: (no reading|\d+ BPM after|not on)", line)):
        if m := re.search(r"INFO  heart: (\d+) BPM", line):
            current["outcome"] = m[1]
        elif "no reading" in line:
            current["outcome"] = "none"
        segments.append(current)
        current = None

with open(sys.argv[3], "w") as out:
    for s in segments:
        mid = s["t"] + len(s["samples"]) * s["period"] / 2e6
        out.write(f"{s['t']:.0f} {garmin(mid)} {s['period']:.1f} {s['outcome']} "
                  + " ".join(map(str, s["samples"])) + "\n")
print(f"{len(segments)} segments, {sum(len(s['samples']) for s in segments)} samples")
