#!/usr/bin/env bash
# Checks the browser's simulator, headless: builds it (build.sh), drives
# it in node as the page would (smoke.mjs), then runs the sim's scripts
# on it and on the command line's, and checks that every screenshot is
# the same, byte for byte. Needs node, besides build.sh's needs.
#
#     web/check.sh [DIR]    # DIR: where the screenshots go (sim/target/web-check)
set -euo pipefail

sim=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$sim/target/web-check}
cd "$sim"

web/build.sh "$sim/target/web"

# The scripts whose screenshots and recordings must match: the tour (every
# screen), the README's (themes too, and its GIFs), and the whole watch,
# drawn round the panel.
scripts=(tour.txt readme-screenshots.txt readme-gifs.txt gadgetbridge.txt web/compare.txt)

rm -rf "$out"
node web/smoke.mjs target/web/ur_sim.wasm "$out/web" "${scripts[@]}"
for script in "${scripts[@]}"; do
    name=$(basename "$script" .txt)
    cargo run --quiet --locked --release --no-default-features -- \
        --out "$out/native/$name" "$script" 2>/dev/null
done

# Both made the same files, and each the same bytes: the same PNG and GIF
# encoders, fed the same pixels.
diff -r "$out/native" "$out/web"
echo "check: the browser's $(find "$out/web" -name '*.png' | wc -l) screenshots and $(find "$out/web" -name '*.gif' | wc -l) recordings match the command line's"
