#!/usr/bin/env bash
# Builds everything a release ships, the same way locally and in CI
# (`.github/workflows/`), reproducibly:
#
#   scripts/release.sh [--pre IDENTIFIER] [OUT]
#
# The version is Cargo.toml's, plus `-IDENTIFIER` if given (CI uses
# `pr.<number>` for pull requests and `snap.<commits>` for main), plus the
# commit as build metadata: `0.9.0+1a2b3c4`, `0.9.0-pr.12+1a2b3c4`. A tree
# with uncommitted changes adds `.dirty` to the commit. The firmware shows
# the whole version on its Info screen; see build.rs.
#
# OUT (default `dist`) receives, for each variant, named after the version
# without the commit:
#
# - `<variant>-<version>-dfu.zip`: the package to install or update over
#   Bluetooth (nRF Device Firmware Update app, `scripts/dfu.py`). This is
#   the file most people want.
# - `<variant>-<version>.img`: the raw MCUBoot image, for other tools.
# - `<variant>-<version>-slot.elf`: the image as an ELF, to flash a dev kit
#   over SWD (`esprobe flash`).
# - `<variant>-<version>.elf`: the linked firmware with symbols, which
#   decodes the watch's defmt logs (`defmt-print -e`).
#
# and `sha256.sum` over all of them. The variants:
#
# - `ur`: the normal build, logging at `info` to RTT (read only through a
#   debug probe).
# - `ur-logging`: keeps its log in the watch's SPI flash, for fetching over
#   Bluetooth (`scripts/fetch-log.py`) from a sealed watch, with the heart
#   rate's raw samples and the accelerometer's readings, for tuning. It
#   costs flash writes and a little battery.
#
# The same commit gives the same files, byte for byte, wherever it's built:
# rust-toolchain.toml pins the compiler, Cargo.lock (`--locked`) the
# dependencies, the paths of this checkout and of cargo's registry are
# replaced with fixed ones in the output, and `package.py` writes the zip
# without timestamps.

set -euo pipefail

pre=
if [[ ${1-} == --pre ]]; then
  pre=${2:?--pre needs an identifier}
  shift 2
fi
if [[ $# -gt 1 || ${1-} == -* ]]; then
  echo "usage: $0 [--pre IDENTIFIER] [OUT]" >&2
  exit 2
fi
out=${1:-dist}

cd "$(dirname "$0")/.."

# ==============================================================================
# The version
# ==============================================================================

cargo_version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')
version=$cargo_version
if [[ -n $pre ]]; then
  if [[ ! $pre =~ ^[0-9A-Za-z.-]+$ ]]; then
    echo "--pre must be dot-separated letters, digits and hyphens, not '$pre'" >&2
    exit 2
  fi
  # Semver: a version that's already a prerelease (0.9.0-rc.1) gets another
  # dot-separated identifier rather than a second hyphen.
  if [[ $version == *-* ]]; then
    version+=".$pre"
  else
    version+="-$pre"
  fi
fi
commit=$(git rev-parse --short=7 HEAD)
if [[ -n $(git status --porcelain --untracked-files=no) ]]; then
  commit+=.dirty
fi
# build.rs bakes this into the firmware.
export UR_VERSION="$version+$commit"

# The image header has a byte each for major and minor, and 16 bits for the
# revision.
IFS=. read -r major minor revision <<<"${cargo_version%%[-+]*}"
if ((major > 255 || minor > 255 || revision > 65535)); then
  echo "Cargo.toml's version must fit the image header: major and minor ≤ 255, revision ≤ 65535" >&2
  exit 2
fi

# ==============================================================================
# Building
# ==============================================================================

# `package.py` needs `llvm-objcopy`; without one on PATH, use rustup's (the
# `llvm-tools` component, which rust-toolchain.toml asks for).
command -v llvm-objcopy >/dev/null ||
  PATH="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin:$PATH"

# Panic locations in dependencies embed their source paths in the image,
# under cargo's registry, and debug info names this checkout. Replace both
# with fixed paths, so where the build ran doesn't change its output. (On
# the command line, `--config` adds to any rustflags in .cargo/config.toml
# rather than replacing them, as RUSTFLAGS would.)
#
# A registry's sources (`registry/src/index.crates.io-1949cf8c6b5b557f/`)
# become plain `/crates`: that's 46 bytes less per path, of which the image
# holds some sixty (3 KB). rustc applies the last remap that matches, so
# these come after the one for all of cargo's home.
cargo_home=$(realpath "${CARGO_HOME:-$HOME/.cargo}")
remap="target.thumbv7em-none-eabihf.rustflags = [
  '--remap-path-prefix=$cargo_home=/cargo',"
for registry in "$cargo_home"/registry/src/*/; do
  remap+="
  '--remap-path-prefix=${registry%/}=/crates',"
done
remap+="
  '--remap-path-prefix=$PWD=/ur',
]"

mkdir -p "$out"
# `package.py` names its outputs after the ELF it's given, and treats
# anything after a dot as an extension, so package under plain names and
# version them afterwards.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Builds one variant: its name, then the `DEFMT_LOG` filter, then any cargo
# flags.
variant() {
  local name=$1 log=$2
  shift 2
  echo "== $name $UR_VERSION" >&2
  DEFMT_LOG=$log cargo build --release --locked --config "$remap" "$@"
  cp target/thumbv7em-none-eabihf/release/ur "$work/$name"
  python3 scripts/package.py "$work/$name" --version "$cargo_version"
  local header=${cargo_version%%[-+]*}
  mv "$work/$name-dfu.zip" "$out/$name-$version-dfu.zip"
  mv "$work/$name.img" "$out/$name-$version.img"
  mv "$work/$name-slot.elf" "$out/$name-$version-slot.elf"
  mv "$work/$name-$header.elf" "$out/$name-$version.elf"
}

variant ur "info"
variant ur-logging "info,ur::heart=trace,ur::activity=debug" --features field-log

(cd "$out" && sha256sum -- ur-*"$version"* >sha256.sum)
echo "== built $UR_VERSION into $out" >&2
cat "$out/sha256.sum" >&2
