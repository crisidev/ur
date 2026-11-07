#!/usr/bin/env bash
# Builds everything a release ships, the same way locally and in CI
# (`.github/workflows/release.yml`):
#
#   scripts/release.sh VERSION [OUT]
#
# VERSION is MAJOR.MINOR.REVISION, as it goes into the MCUBoot image header
# (and the watch logs at boot). OUT (default `dist`) receives, for each
# variant:
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
# - `ur-field`: keeps its log in the watch's SPI flash, for fetching over
#   Bluetooth (`scripts/fetch-log.py`) from a sealed watch, with the heart
#   rate's raw samples and the accelerometer's readings, for tuning. It
#   costs flash writes and a little battery.

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 VERSION [OUT]" >&2
  exit 2
fi
version=$1
out=${2:-dist}
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "VERSION must be MAJOR.MINOR.REVISION, not '$version'" >&2
  exit 2
fi
# The image header has a byte each for major and minor, and 16 bits for the
# revision.
IFS=. read -r major minor revision <<<"$version"
if ((major > 255 || minor > 255 || revision > 65535)); then
  echo "VERSION must fit the image header: major and minor ≤ 255, revision ≤ 65535" >&2
  exit 2
fi

cd "$(dirname "$0")/.."

# `package.py` needs `llvm-objcopy`. Without one on PATH, use rustup's
# (`rustup component add llvm-tools`).
command -v llvm-objcopy >/dev/null ||
  PATH="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin:$PATH"

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
  echo "== $name $version" >&2
  # `--locked`: the release is built from exactly the dependencies in
  # Cargo.lock, as tested.
  DEFMT_LOG=$log cargo build --release --locked "$@"
  cp target/thumbv7em-none-eabihf/release/ur "$work/$name"
  python3 scripts/package.py "$work/$name" --version "$version"
  mv "$work/$name-dfu.zip" "$out/$name-$version-dfu.zip"
  mv "$work/$name.img" "$out/$name-$version.img"
  mv "$work/$name-slot.elf" "$out/$name-$version-slot.elf"
  mv "$work/$name-$version.elf" "$out/$name-$version.elf"
}

variant ur "info"
variant ur-field "info,ur::heart=trace,ur::activity=debug" --features field-log

(cd "$out" && sha256sum -- ur-* >sha256.sum)
ls -l "$out" >&2
