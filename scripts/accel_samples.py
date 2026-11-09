"""Extract the accelerometer's samples from a capture build's decoded field
log (`UR_ACTIVITY_LOG=trace scripts/release.sh`; see `trace_samples` in
src/activity.rs), for tuning a step counter offline against another watch.

Writes one line per poll whose samples were logged (every 2 s while the
wrist moves):

    <utc> <chip steps> <garmin steps> <x> <y> <z> <x> <y> <z> ...

where `utc` is the poll's time (its newest sample's, within ~0.1 s; the
samples before it are 80 ms apart, at 12.5 Hz), `chip steps` the steps the
BMA421's own counter added in that poll, `garmin steps` the Garmin's for
that minute (0 without an export), and the samples in 1/64 g. A gap of
more than one poll between lines means the wrist kept still (or a read
failed): a replay should start afresh there.

Usage: python3 -I scripts/accel_samples.py DECODED_LOG OUT [HEALTH_CONNECT_DB]

The Garmin is app 9 in the exports this was written against; check
`application_info_table` in yours. The log's clock is taken from its
`clock:` lines, so it must have one (the phone sets it on connecting).
"""
import re
import sqlite3
import sys
from collections import defaultdict

GARMIN = 9
LINE = re.compile(r"^(\d+\.\d+) (\w+)\s+(.*)$")
SAMPLES = re.compile(r"activity: samples \[([-\d, ]*)\]")
STEPS = re.compile(r"activity: \d+ samples, movement \d+, (\d+) new steps")

log_path, out_path = sys.argv[1], sys.argv[2]
db_path = sys.argv[3] if len(sys.argv) > 3 else None

# Uptime to UTC, through the latest `clock:` line so far (or, before the
# first, the first: the phone sets the clock within seconds of a boot).
clocks, polls = [], []
pending = None
for line in open(log_path, encoding="utf-8", errors="replace"):
    m = LINE.match(line.rstrip("\n"))
    if not m:
        continue
    up, text = float(m[1]), m[3]
    if c := re.match(r"clock: (\d+) UTC", text):
        clocks.append((up, int(c[1]) - up))
    elif s := SAMPLES.search(text):
        pending = (up, [int(v) for v in s[1].split(",") if v.strip()])
    elif (n := STEPS.search(text)) and pending:
        # The poll's own line follows its samples, at the same time.
        if abs(pending[0] - up) < 0.5:
            polls.append((pending[0], int(n[1]), pending[1]))
        pending = None
if not clocks:
    sys.exit("no clock: lines in the log")
if not polls:
    sys.exit("no samples in the log: is it from a capture build?")


def utc(up):
    later = [o for u, o in clocks if u <= up]
    return up + (later[-1] if later else clocks[0][1])


garmin = defaultdict(int)
if db_path:
    db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    first, last = utc(polls[0][0]), utc(polls[-1][0])
    for start, count in db.execute(
            "select start_time, count from steps_record_table where app_info_id = ? "
            "and start_time between ? and ?", (GARMIN, (first - 60) * 1000, (last + 60) * 1000)):
        garmin[start // 60000] += count

with open(out_path, "w") as out:
    for up, steps, samples in polls:
        t = utc(up)
        out.write(f"{t:.2f} {steps} {garmin[int(t // 60)]} {' '.join(map(str, samples))}\n")
print(f"{len(polls)} polls, {sum(len(s) // 3 for _, _, s in polls)} samples")
