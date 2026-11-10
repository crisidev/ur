#!/usr/bin/env bash
# Builds the simulator for the browser: a static site in DIR (default:
# sim/target/web) of the page, its script, and the WebAssembly module.
# Serve DIR over HTTP to use it (the page fetches the module, which
# browsers don't allow from file:// pages), e.g.
#
#     python3 -m http.server -d target/web
#
# Needs only cargo and the wasm32-unknown-unknown target
# (`rustup target add wasm32-unknown-unknown`): the module's interface is
# plain functions (see src/web.rs), so there's no wasm-bindgen step.
set -euo pipefail

sim=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$sim/target/web}

# `cargo rustc --crate-type cdylib`: the library as a WebAssembly module,
# without making every native build link a shared library it has no use
# for. `--target` overrides .cargo/config.toml's host target, and the
# window (minifb) is left out, as it's the native front end's. Every
# theme is in (`themes-all`), for the page's picker to show them all.
cd "$sim"
cargo rustc --locked --release --lib --no-default-features --features themes-all \
    --target wasm32-unknown-unknown --crate-type cdylib

mkdir -p "$out"
cp web/index.html web/ur.js "$out/"
cp target/wasm32-unknown-unknown/release/ur_sim.wasm "$out/"
echo "built $out"
