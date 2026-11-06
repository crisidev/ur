#!/usr/bin/env bash
# Cargo runner: flash the ELF onto the PineTime, then stream its defmt logs.
set -euo pipefail
ELF="$1"

# `esprobe program` would be nicer (it backs up flash first), but its
# read-back verification checks 0x08000000 (STM32's flash base) rather than
# the nRF52's 0x0, so it always reports a mismatch here. `flash` uses
# probe-rs's own flash algorithm and resets the core afterwards.
esprobe --target nRF52832_xxAA flash "$ELF"

# The nRF52 aliases its data RAM at 0x00800000, so a RAM scan finds the RTT
# control block twice and gives up. Point esprobe at the real one.
control_block="0x$(llvm-nm "$ELF" | awk '$3 == "_SEGGER_RTT" { print $1 }')"

timeout "${RTT_SECS:-20}" esprobe --target nRF52832_xxAA rtt --control-block "$control_block" |
  defmt-print -e "$ELF"
