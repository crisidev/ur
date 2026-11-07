#!/usr/bin/env bash
# Cargo runner: package the ELF for the bootloader, flash it into the
# bootloader's primary slot over SWD, then stream its defmt logs.
#
# This needs the PineTime bootloader at 0x0 (see memory.x): the firmware is
# linked to start at 0x8100 and can't boot on its own.
set -euo pipefail
ELF="$1"

python3 "$(dirname "$0")/package.py" "$ELF"

# `esprobe program` would be nicer (it backs up flash first), but its
# read-back verification checks 0x08000000 (STM32's flash base) rather than
# the nRF52's 0x0, so it always reports a mismatch here. `flash` uses
# probe-rs's own flash algorithm and resets the core afterwards.
#
# The first flash after an RTT session often fails to attach; one retry
# always has so far.
esprobe --target nRF52832_xxAA flash "${ELF}-slot.elf" ||
  esprobe --target nRF52832_xxAA flash "${ELF}-slot.elf"

# The nRF52 aliases its data RAM at 0x00800000, so a RAM scan finds the RTT
# control block twice and gives up. Point esprobe at the real one.
control_block="0x$(llvm-nm "$ELF" | awk '$3 == "_SEGGER_RTT" { print $1 }')"

# The bootloader shows its logo for 5 s (watching for a held button) before
# starting us, and there's no RTT control block to attach to until then.
sleep 6
timeout "${RTT_SECS:-25}" esprobe --target nRF52832_xxAA rtt --control-block "$control_block" |
  defmt-print -e "$ELF"
