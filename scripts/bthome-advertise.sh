#!/usr/bin/env bash
# Advertises from this computer as an unencrypted BTHome v2 thermometer,
# for a watch's room sensor scans (see src/bthome_sensor.rs), until killed
# (or SECONDS pass). `ur-sim --devkit`'s `bthome-temp` runs this.
#
#     scripts/bthome-advertise.sh HUNDREDTHS [SECONDS]    # 2137: 21.37 °C
#
# Through `bluetoothctl`, which takes the advertisement down as it exits:
# kill this script's process group, not just the script.
set -euo pipefail

hundredths=$1
secs=${2:-3600}
# sint16 little-endian, two's complement below zero.
value=$((hundredths & 0xffff))
lo=$(printf '0x%02x' $((value & 0xff)))
hi=$(printf '0x%02x' $((value >> 8)))

{
	sleep 2
	printf 'menu advertise\n'
	sleep 0.5
	# Device information 0x40 (v2, unencrypted, regular), temperature
	# (0x02, 0.01 °C), humidity (0x03, 0.01 %: 45.60).
	printf 'service 0xfcd2 0x40 0x02 %s %s 0x03 0xd0 0x11\n' "$lo" "$hi"
	sleep 0.5
	printf 'interval 100 150\n'
	sleep 0.5
	printf 'back\n'
	sleep 0.5
	printf 'advertise on\n'
	sleep "$secs"
	printf 'advertise off\n'
	sleep 0.5
	printf 'quit\n'
} | bluetoothctl
