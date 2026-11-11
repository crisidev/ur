#!/usr/bin/env python3
"""Screenshot the watch's screen over the debug probe, optionally after
injecting inputs (see src/debug.rs), which needs a build with the
`debug-input` feature (`cargo run --release --features debug-input`).

Reads the canvas's front buffer (what the panel shows: 80×80 colour roles)
and the current theme's palette from RAM with `esprobe fast-dump`, and
writes a PNG at the panel's 3× scale. The watch runs on as it's read, so a
frame drawn in the meantime can come out half old, half new: take another.

Usage:
    screenshot.py <elf> <out.png> [--input STEP ...] [--wait SECONDS] [--theme NAME]

Each STEP is applied in order, with --wait seconds after each (default 1.5,
enough for a transition): `button` (`button:2,0` presses twice, 0.2 s
apart), `tap:X,Y` (`tap:X,Y,MS` held for MS milliseconds, up to 255,
rather than 80), `long:X,Y`, or `up`, `down`, `left`, `right` (swipes,
optionally with `:X,Y`), in canvas pixels (0–79);
`glance` (as if the wrist turned) and `notify` (a test notification;
`notify` alone has a two-word name, big on two lines,
`notify:1,0` a four-word one, `notify:2,0` a much longer one,
`notify:3,0` and `notify:4,0` (with accents) two-word ones on two lines
in the medium face,
`notify:5,0` a short one, big, `notify:6,0` and `notify:7,0`
every ASCII symbol between them, `notify:8,0` curly quotes, a dash, and an
ellipsis; `notify:X,H` arrived H
hours ago, for the history's arrival times). `hang`, `panic` (a `defmt::panic!`) and `fault` (a
`core::panic!` as from a SoftDevice Controller fault, at "line" Y;
`fault:1,Y` with a long file name) crash the watch on
purpose, to test the crash reports. `battery:MV` makes the watch read its
battery as MV millivolts from then on (`battery:0` for the real reading
again), e.g. `battery:3300` to test what a critically low battery does: a
minute later the watch saves its activity and switches itself off.
`media:1,0` makes the phone seem to be playing something (`media` alone:
paused; `media:2,0` the track alone, with no word on whether it plays),
until it disconnects, for the music screen and the clock's media icon; it
only shows while the phone is connected.
`steps:N` counts N steps as walked now (up to 65535), to test what keeps
them across resets and updates.
`doubletap:X,Y` taps twice, 0.1 s apart, quicker than two `tap`s over
the probe can (for the button grid's double taps).
`bitmaps` sends a notification as Gadgetbridge does with its "Text as
Bitmaps" setting on (a word as an image), through the watch's parser.
`find` sends Gadgetbridge's "find my watch" line, through the watch's
parser, and `find:0,0` the one that stops it.
`press:X,Y` puts a simulated finger down at X,Y (or moves it there, if
it's down already), and `lift` lifts it: these go through the touch
task as the touch controller's reports would, so a long press is
recognised while the finger is held (e.g. `--input press:40,40 lift`
holds it for --wait seconds), and moving it (`press:40,40 press:40,70
lift --wait 0.3`) makes a swipe.
`asleep:1,0` makes the watch take the wearer as asleep from then on, as
if sleep tracking said so (the quiet night starts a second later), and
`asleep:0,0` leaves it to sleep tracking again (the night then ends as
on waking, or at once with `button:2,0`).
`movement:N` makes the watch take the wrist's movement as N from then on
(`movement:0` for the measured again), e.g. `movement:1000` for a busy
wrist's heart rate session on a dev kit lying still on a desk (above 150
is busy, above 1500 too vigorous to measure through). `skin:1,0` makes
the heart rate sensor take whatever it's on as skin from then on, so a
dev kit's sessions run their whole length rather than ending at once as
"not on a wrist" (`skin:0,0` to stop).

`--theme` renders the frame in another theme's colours (e.g. HANDHELD, for
the day look at night): the canvas holds colour roles, not colours.
"""

import argparse
import re
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

TARGET = ["esprobe", "--target", "nRF52832_xxAA"]
WIDTH = HEIGHT = 80
BYTES = WIDTH * HEIGHT // 2
# Colour roles in a palette: `Color::COUNT` in src/palette.rs.
ROLES = 16
SCALE = 3
# `qr::SIZE`, and the panel pixels a module takes (see `Canvas::qr`).
QR_SIZE = 25
QR_MODULE = 7
KINDS = {
    "button": 1,
    "tap": 2,
    "long": 3,
    "up": 4,
    "down": 5,
    "left": 6,
    "right": 7,
    "glance": 8,
    "notify": 9,
    "hang": 10,
    "panic": 11,
    "fault": 12,
    "battery": 13,
    "media": 14,
    "steps": 15,
    "press": 16,
    "lift": 17,
    "bitmaps": 18,
    "doubletap": 19,
    "find": 20,
    "asleep": 21,
    "movement": 22,
    "skin": 23,
}


def symbol(elf: Path, name: str) -> int:
    out = subprocess.run(["llvm-nm", str(elf)], capture_output=True, text=True, check=True)
    for line in out.stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] == name:
            return int(parts[0], 16)
    if name == "UR_INPUT":
        sys.exit(f"no input in {elf}: build it with `--features debug-input`")
    sys.exit(f"no symbol {name} in {elf}")


def read_words(address: int, count: int) -> list[int]:
    # With `fast-dump`, which never halts the core. `core read` is a
    # probe-rs session, which halts it as it attaches and as it detaches,
    # and a halt in a flash timeslot makes MPSL reset the watch (see
    # CONTRIBUTING.md). To a file: fast-dump prints a summary on stdout.
    with tempfile.TemporaryDirectory() as scratch:
        dump = Path(scratch) / "dump.bin"
        subprocess.run(
            TARGET + ["fast-dump", "--address", str(address), "--size", str(4 * count), str(dump)],
            capture_output=True,
            check=True,
        )
        data = dump.read_bytes()
    if len(data) != 4 * count:
        sys.exit(f"short read at {hex(address)}: {len(data)} bytes of {4 * count}")
    return list(struct.unpack(f"<{count}I", data))


def write_word(address: int, value: int) -> None:
    subprocess.run(TARGET + ["core", "write", hex(address), hex(value)], capture_output=True, check=True)


def inject(elf: Path, step: str) -> None:
    name, _, where = step.partition(":")
    if name not in KINDS:
        sys.exit(f"unknown input {step!r}")
    if name in ("battery", "steps", "movement"):
        # Millivolts, steps or movement, little endian, in the two
        # coordinate bytes.
        value = int(where)
        x, y, rest = value & 0xFF, value >> 8, []
    else:
        x, y, *rest = (int(v) for v in where.split(",")) if where else (40, 40)
    # A tap's duration, in milliseconds (0: the default), in the top byte.
    held_ms = rest[0] if name == "tap" and rest else 0
    if not 0 <= held_ms <= 255:
        sys.exit(f"a tap is held for at most 255 ms here, not {held_ms}")
    write_word(symbol(elf, "UR_INPUT"), KINDS[name] | x << 8 | y << 16 | held_ms << 24)


def png(path: Path, pixels: list[tuple[int, int, int]], width: int, height: int) -> None:
    raw = b"".join(
        b"\0" + bytes(c for pixel in pixels[row * width : (row + 1) * width] for c in pixel)
        for row in range(height)
    )
    chunk = lambda kind, data: (
        struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    )
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    )


def theme_symbol(elf: Path, name: str) -> int:
    # A colour scheme: a static called NAME anywhere in a theme's modules,
    # `themes::<theme>::…::NAME` (v0 mangling: `6themes<len><theme>`, any
    # more modules, then `<len><NAME>`), whatever the crate's hash. Its
    # static starts with its palette (retro's type is `repr(transparent)`).
    out = subprocess.run(["llvm-nm", str(elf)], capture_output=True, text=True, check=True)
    scheme = re.compile(rf"6themes\d+[a-z0-9_]+(\d+[a-z0-9_]+)*?{len(name)}{re.escape(name)}$")
    for line in out.stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and scheme.search(parts[2]):
            return int(parts[0], 16)
    sys.exit(f"no colour scheme {name} in {elf}")


def qr_matrix(key: bytes) -> list[int]:
    """`key`'s hex as the watch encodes it (`qr::encode`): rows of bits,
    bit x of row y set for a dark module. Needs `segno` (`uv run --with
    segno scripts/screenshot.py ...`)."""
    try:
        import segno
    except ImportError:
        sys.exit("a QR code is showing: run with segno (uv run --with segno ...)")
    code = segno.make(
        key.hex().upper(), version=2, error="m", mode="alphanumeric", mask=0, boost_error=False
    )
    return [sum(bool(m) << x for x, m in enumerate(row)) for row in code.matrix]


def screenshot(elf: Path, out: Path, theme_name: str | None = None) -> None:
    (canvas,) = read_words(symbol(elf, "UR_CANVAS"), 1)
    if canvas == 0:
        sys.exit("the firmware hasn't registered its canvas yet")
    # `Canvas` is repr(C): back[3200], front[3200], front_valid, then (at the
    # next 4-byte boundary) the theme pointer, and a QR code (below).
    front = b"".join(struct.pack("<I", w) for w in read_words(canvas + BYTES, BYTES // 4))
    (theme,) = read_words(canvas + 2 * BYTES + 4, 1)
    if theme_name is not None:
        theme = theme_symbol(elf, theme_name.upper())
    # `Palette` is repr(C): name (&str: pointer, length), then a big-endian
    # RGB565 colour per role, then the half-toned roles, a bit each (see
    # `Palette::columns`): every other panel column of those is background.
    theme_bytes = b"".join(
        struct.pack("<I", w) for w in read_words(theme + 8, (2 * ROLES + 2 + 3) // 4)
    )
    palette_bytes = theme_bytes[: 2 * ROLES]
    (halftone,) = struct.unpack("<H", theme_bytes[2 * ROLES : 2 * ROLES + 2])
    palette = []
    for i in range(ROLES):
        (rgb565,) = struct.unpack(">H", palette_bytes[2 * i : 2 * i + 2])
        r, g, b = rgb565 >> 11, (rgb565 >> 5) & 0x3F, rgb565 & 0x1F
        palette.append((r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2))
    # Then (after the theme pointer) the bytes a QR code shows (in hex),
    # and whether it's wanted and shown: drawn over the canvas at the
    # panel's resolution (see `Canvas::qr`). The watch only works out the
    # code as it sends it, so this does too, as its host test checks it.
    *key, flags = read_words(canvas + 2 * BYTES + 8, 4 + 1)
    qr_shown = (flags >> 8) & 0xFF != 0
    qr = qr_matrix(b"".join(struct.pack("<I", w) for w in key)) if qr_shown else []
    side = (QR_SIZE + 8) * QR_MODULE
    origin = (WIDTH * SCALE - side) // 2
    pixels = []
    for y in range(HEIGHT * SCALE):
        for x in range(WIDTH * SCALE):
            if qr_shown and origin <= x < origin + side and origin <= y < origin + side:
                mx, my = (x - origin) // QR_MODULE - 4, (y - origin) // QR_MODULE - 4
                dark = 0 <= mx < QR_SIZE and 0 <= my < QR_SIZE and qr[my] >> mx & 1
                pixels.append((0, 0, 0) if dark else (255, 255, 255))
                continue
            i = (y // SCALE) * WIDTH + x // SCALE
            role = front[i // 2] >> 4 if i % 2 else front[i // 2] & 0x0F
            if halftone >> role & 1 and x % 2:
                role = 0
            pixels.append(palette[role] if role < len(palette) else (255, 0, 255))
    png(out, pixels, WIDTH * SCALE, HEIGHT * SCALE)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("elf", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--input", nargs="*", default=[])
    parser.add_argument("--wait", type=float, default=1.5)
    parser.add_argument("--theme")
    args = parser.parse_args()
    for step in args.input:
        inject(args.elf, step)
        time.sleep(args.wait)
    screenshot(args.elf, args.out, args.theme)


if __name__ == "__main__":
    main()
