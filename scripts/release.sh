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
# - `ur-<theme>`, for each theme (see src/themes/) other than the default:
#   `ur` with only that theme in it, for wearers who'd rather have that
#   look. One with colour schemes of one side only (night, say) shows them
#   by day and by night alike, and its FORCE can't change (README FAQ
#   D02); the release's notes say so. Only its `-dfu.zip`:
#   the other files are for dev kits and for decoding logs, and a wearer
#   who reports a problem can do so on `ur` or `ur-logging`. Its ELF is
#   held to every size constraint first (`scripts/size.py check`), as CI
#   holds the others.
#
# Both `ur` and `ur-logging` have the default themes (Cargo.toml's
# `default`).
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
# The registries' directories only exist once the dependencies have been
# downloaded: fetch first, so a fresh cargo home (as in the release
# workflow, which builds without a cache) gets the same remaps as one that
# has built before. (Without this, v0.9.0's published images kept the
# longer /cargo/registry/src/… paths, and didn't match a local rebuild.)
cargo fetch --locked >&2
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
  # The dev kit's injected input (`debug-input` in Cargo.toml) has no place
  # on a sealed watch, which has no probe to write it. It's off by default,
  # but a feature that came to imply it, or a cfg gone missing, would
  # bring it back without a word: refuse the build instead. (awk, not
  # `grep -q`, which would stop reading early and fail llvm-nm's write,
  # and so the pipeline.)
  if [[ -n $(llvm-nm "$work/$name" | awk '$3 == "UR_INPUT"') ]]; then
    echo "$name has the dev kit's debug input (UR_INPUT); releases must not" >&2
    exit 1
  fi
  python3 scripts/package.py "$work/$name" --version "$cargo_version"
  local header=${cargo_version%%[-+]*}
  mv "$work/$name-dfu.zip" "$out/$name-$version-dfu.zip"
  mv "$work/$name.img" "$out/$name-$version.img"
  mv "$work/$name-slot.elf" "$out/$name-$version-slot.elf"
  mv "$work/$name-$header.elf" "$out/$name-$version.elf"
}

variant ur "info"
# The field log adds the heart rate sensor's samples and the accelerometer's
# readings, and leaves out the dependencies whose every log line (at `info`
# and up) is the message of a `defmt::panic!`, `unwrap!` or `assert!`. In a
# field build that message only reaches the RTT buffer, whose RAM the
# panic's reset clears before the field log can save it: the next boot's
# report of the panic is all that's kept either way (see CONTRIBUTING's
# Field logs). Leaving them out saves ~960 bytes of flash. Their only other
# lines say the same on every boot of a build, which a probe on `ur` shows:
# nrf-sdc's that its memory is too big (or too small, and then `main`
# panics), embassy-nrf's about the reset pin. trouble-host stays: its errors
# and warnings are worth keeping. Check these crates' new log lines when
# updating them.
panics_only=embassy_executor=off,embassy_sync=off,embassy_time=off,embassy_nrf=off,bt_hci=off,nrf_mpsl=off,nrf_sdc=off
# `UR_ACTIVITY_LOG=trace` makes a capture build, whose field log also keeps
# the accelerometer's samples while the wrist moves, for tuning step
# counting offline (see `trace_samples` in src/activity.rs). It about
# doubles the log on a busy day, so it's not for releases: give it a
# `--pre` of its own (`--pre accel`) to tell its packages apart.
variant ur-logging "info,ur::heart=trace,ur::activity=${UR_ACTIVITY_LOG:-debug},$panics_only" --features field-log

# ==============================================================================
# A package for each other theme
# ==============================================================================

# The themes in the repository, and the default ones, from Cargo.toml's
# features (`theme-<name>`).
read -r -a all_themes < <(python3 -c 'import tomllib; print(" ".join(f.removeprefix("theme-") for f in tomllib.load(open("Cargo.toml", "rb"))["features"] if f.startswith("theme-")))')
read -r -a default_themes < <(python3 -c 'import tomllib; print(" ".join(f.removeprefix("theme-") for f in tomllib.load(open("Cargo.toml", "rb"))["features"]["default"] if f.startswith("theme-")))')
others=()
for theme in "${all_themes[@]}"; do
  if [[ " ${default_themes[*]} " != *" $theme "* ]]; then
    others+=("$theme")
  fi
done
# Which sides each has schemes for is the code's to say (each theme's
# `BUILT_IN`), not a list here: the simulator, with every theme, prints
# each with its night and day schemes' counts, and its name as people
# read it. Only built when there's a theme to ask about, as it takes a
# minute or two. With `UR_THEME_LIST` set, each package's theme goes in
# that file, a line each, `<theme>\t<its name as people read it>\t<its
# sides>` (`both`, `night` or `day`), for the release's notes.
if [[ -n ${UR_THEME_LIST-} ]]; then
  : >"$UR_THEME_LIST"
fi
if ((${#others[@]} > 0)); then
  listing=$(cd sim && cargo run --quiet --locked --release --no-default-features --features themes-all -- --list-themes)
  while IFS=$'\t' read -r theme night day name; do
    if [[ " ${others[*]} " != *" $theme "* ]]; then
      continue
    fi
    if ((night > 0 && day > 0)); then
      sides=both
    elif ((night > 0)); then
      sides=night
    else
      sides=day
    fi
    # `ur-logging` is taken, and a name becomes part of file names.
    if [[ ! $theme =~ ^[a-z0-9_]+$ || $theme == logging ]]; then
      echo "a theme can't be called '$theme' here: lowercase letters, digits and underscores, not 'logging'" >&2
      exit 1
    fi
    variant "ur-$theme" "info" --no-default-features --features "theme-$theme"
    python3 scripts/size.py check "ur-$theme=$out/ur-$theme-$version.elf" >&2
    rm "$out/ur-$theme-$version.img" "$out/ur-$theme-$version-slot.elf" "$out/ur-$theme-$version.elf"
    if [[ -n ${UR_THEME_LIST-} ]]; then
      printf '%s\t%s\t%s\n' "$theme" "$name" "$sides" >>"$UR_THEME_LIST"
    fi
  done <<<"$listing"
fi

# In the C locale, so the glob lists the files in the same (byte) order
# wherever this runs: in en_US.UTF-8, `ur-0.9.0.img` sorts before
# `ur-0.9.0-slot.elf`, in C after, and sha256.sum is itself released.
(cd "$out" && LC_ALL=C && sha256sum -- ur-*"$version"* >sha256.sum)
echo "== built $UR_VERSION into $out" >&2
cat "$out/sha256.sum" >&2
