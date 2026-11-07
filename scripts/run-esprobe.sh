#!/usr/bin/env bash
# Cargo runner for an esprobe (https://github.com/perara/esprobe-rs):
# package the ELF for the bootloader, flash it into the bootloader's
# primary slot over SWD, then stream its defmt logs.
#
# A runner for another probe does the same three things; see
# CONTRIBUTING.md ("With a different probe").
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

# The bootloader shows its logo for 5 s (watching for a held button) and
# checks the image's hash before starting us, and there's no RTT control
# block to attach to until then: wait for its "SEGGER RTT" magic.
for _ in $(seq 20); do
  if esprobe --target nRF52832_xxAA core read "$control_block" 2>/dev/null | grep -q 0x47474553; then
    break
  fi
  sleep 1
done
timeout "${RTT_SECS:-25}" esprobe --target nRF52832_xxAA rtt --control-block "$control_block" |
  defmt-print -e "$ELF"
