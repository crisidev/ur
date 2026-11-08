#!/usr/bin/env python3
"""Print the sleep stages one app wrote to Health Connect (a Garmin's, by
default), one line per minute, for host-tests/sleep's replay to compare its
stages against:

    python3 -I scripts/hc-stages.py health_connect_export.db > garmin.txt
    cd host-tests/sleep && cargo run --release -- decoded.log ../../garmin.txt

The database is the SQLite file in the zip that Health Connect's "Export
data" makes. It's opened read-only. An export can hold several apps' data
(Gadgetbridge's from more than one watch, say): `--app` picks one by its
Android package name, and with `--list` this prints which apps wrote sleep.

Lines are `<UTC minute> <stage>`, UTC minutes since 1970 as in the
firmware's history, and stages `awake`, `light`, `deep`, or `rem`. Stages
without one of those meanings ("sleeping", unspecified) are left out.

Usage: hc-stages.py <export.db> [--app PACKAGE] [--list]
"""

import argparse
import sqlite3

# Health Connect's `SleepSessionRecord.STAGE_TYPE_*`: 1 awake, 2 sleeping
# (no stage), 3 out of bed, 4 light, 5 deep, 6 REM, 7 awake in bed.
STAGES = {1: "awake", 3: "awake", 4: "light", 5: "deep", 6: "rem", 7: "awake"}


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("db", help="Health Connect export (SQLite)")
    parser.add_argument("--app", default="com.garmin.android.apps.connectmobile")
    parser.add_argument("--list", action="store_true", help="list the apps that wrote sleep")
    args = parser.parse_args()

    db = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    if args.list:
        for package, sessions in db.execute(
            "select a.package_name, count(*) from sleep_session_record_table s "
            "join application_info_table a on s.app_info_id = a.row_id group by 1"
        ):
            print(f"{package}: {sessions} sleep sessions")
        return
    stages = {}
    for start, end, stage in db.execute(
        "select t.stage_start_time / 1000, t.stage_end_time / 1000, t.stage_type "
        "from sleep_stages_table t "
        "join sleep_session_record_table s on t.parent_key = s.row_id "
        "join application_info_table a on s.app_info_id = a.row_id "
        "where a.package_name = ? order by 1",
        (args.app,),
    ):
        if stage in STAGES:
            for minute in range(start // 60, end // 60):
                stages[minute] = STAGES[stage]
    for minute in sorted(stages):
        print(minute, stages[minute])


if __name__ == "__main__":
    main()
