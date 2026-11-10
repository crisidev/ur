#!/usr/bin/env bash
# Builds the firmware for the dev kit fuzzer with edge coverage: a
# `compare` build (with the `coverage` feature) whose own code
# SanitizerCoverage instruments, a byte per branch (see src/coverage.rs),
# then packages it for the bootloader.
#
#     sim/coverage-build.sh OUT_DIR    # writes OUT_DIR/ur-coverage.elf, …-slot.elf
#
# Only the root crate gets the flags (`cargo rustc`), so only its code is
# instrumented, not embassy's, trouble's and the rest. That takes LTO off:
# with it, every crate is optimized (and would be instrumented) together in
# the root crate's module. (LLVM's allowlist, which would pick functions by
# name, is clang's alone.) Its own target directory, so an ordinary build
# isn't rebuilt after it.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=$1
mkdir -p "$out"

cd "$root"
# defmt-rtt's buffer at 256 bytes, not 1 KiB, to make room for the flags
# (see coverage.x): enough for the boot's first lines, which say why it
# restarted.
DEFMT_RTT_BUFFER_SIZE=256 CARGO_TARGET_DIR="$root/target/coverage" cargo rustc --locked --release --features coverage -- \
  -Cpasses=sancov-module \
  -Cllvm-args=-sanitizer-coverage-level=1 \
  -Cllvm-args=-sanitizer-coverage-inline-bool-flag \
  -Cllvm-args=-sanitizer-coverage-pc-table
cp "$root/target/coverage/thumbv7em-none-eabihf/release/ur" "$out/ur-coverage.elf"
python3 "$root/scripts/package.py" "$out/ur-coverage.elf"
