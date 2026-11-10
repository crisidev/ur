#!/usr/bin/env python3
"""Check the firmware against every size constraint, and compare sizes.

    size.py check [--json OUT] [NAME=]ELF...
    size.py measure [--json OUT] [NAME=]ELF...
    size.py diff BASE.json HEAD.json [--base-label TEXT] [--head-label TEXT]

`check` measures each linked firmware ELF (a variant: `ur`, `ur-logging`,
`compare`), prints one table per variant of each constraint, its limit,
what's used and what's left, then checks that the layout's constants agree
everywhere they're written down. It exits non-zero if anything doesn't fit
or doesn't agree. The variant's name is NAME if given, else the file's
name without a version (`dist/ur-logging-0.9.0-pr.12.elf` is
`ur-logging`). `--json` also writes the measurements, for `diff`.
`measure` is `check` without the checks, for another commit's builds.

`diff` turns two `--json` files (the base branch's and a PR's) into the
Markdown size report that CI posts on pull requests.

To see what a change costs locally, build before and after, and diff:

    scripts/release.sh tmp/before                    # on the base commit
    scripts/size.py measure tmp/before/ur*[0-9].elf --json tmp/before.json
    scripts/release.sh tmp/after                     # with the change
    scripts/size.py check tmp/after/ur*[0-9].elf --json tmp/after.json
    scripts/size.py diff tmp/before.json tmp/after.json

Only the standard library is needed (like `package.py`, whose layout
constants this shares), and Python 3.11 or later.
"""

import argparse
import ast
import json
import os
import re
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
# The MCUBoot slot's geometry: `package.py` is what builds the image, so its
# numbers are the ones the image must fit.
import package  # noqa: E402

# The marker by which CI finds its own comment on a PR, to update it in
# place rather than adding another on every push.
MARKER = "<!-- ur:size-report -->"

# ==============================================================================
# Reading the ELF
# ==============================================================================

# Just enough of ELF32 (little-endian, as ARM is) for sizes and symbols:
# section headers, program headers (for load addresses) and the symbol
# table. `llvm-size` and `llvm-nm` would do, but CI's runner doesn't have
# them on PATH, and parsing ~50 bytes of headers beats finding rustup's.

SHF_ALLOC = 0x2
SHF_EXECINSTR = 0x4
SHT_SYMTAB = 2
SHT_NOBITS = 8
PT_LOAD = 1


@dataclass
class Section:
    name: str
    kind: int
    flags: int
    address: int
    offset: int
    size: int

    @property
    def loaded(self) -> bool:
        """Takes space at run time (as opposed to debug info, symbols)."""
        return bool(self.flags & SHF_ALLOC) and self.size > 0

    @property
    def in_image(self) -> bool:
        """Has bytes in the image (as opposed to RAM that starts zeroed)."""
        return self.loaded and self.kind != SHT_NOBITS


class Elf:
    def __init__(self, path: Path):
        self.data = data = path.read_bytes()
        if data[:6] != b"\x7fELF\x01\x01":
            sys.exit(f"{path}: not a little-endian 32-bit ELF")
        (phoff, shoff) = struct.unpack_from("<II", data, 0x1C)
        (phentsize, phnum, shentsize, shnum, shstrndx) = struct.unpack_from(
            "<HHHHH", data, 0x2A
        )
        raw = [
            struct.unpack_from("<IIIIIIIIII", data, shoff + i * shentsize)
            for i in range(shnum)
        ]
        names = raw[shstrndx][4]
        self.sections = [
            Section(self._string(names + h[0]), h[1], h[2], h[3], h[4], h[5]) for h in raw
        ]
        # Program headers map each loaded section's run-time address to where
        # its bytes sit in flash: `.data` runs in RAM, but its initial values
        # are stored in flash after `.rodata`, and copied at boot.
        self.segments = [
            struct.unpack_from("<IIIIIIII", data, phoff + i * phentsize) for i in range(phnum)
        ]
        self.symbols: dict[str, int] = {}
        for header, section in zip(raw, self.sections):
            if section.kind != SHT_SYMTAB:
                continue
            strings = raw[header[6]][4]  # sh_link: the symbol names
            for at in range(section.offset, section.offset + section.size, 16):
                name, value = struct.unpack_from("<II", data, at)
                if name:
                    self.symbols[self._string(strings + name)] = value

    def _string(self, at: int) -> str:
        return self.data[at : self.data.index(b"\0", at)].decode()

    def load_address(self, section: Section) -> int:
        for kind, _, vaddr, paddr, _, memsz, _, _ in self.segments:
            if kind == PT_LOAD and vaddr <= section.address < vaddr + memsz:
                return paddr + section.address - vaddr
        return section.address

    def symbol(self, name: str) -> int:
        if name not in self.symbols:
            sys.exit(f"the ELF has no `{name}`; is it this repository's firmware?")
        return self.symbols[name]


# ==============================================================================
# Measuring
# ==============================================================================

# MCUBoot's TLV area after the code: its 4-byte header, then one TLV (4-byte
# header, 32-byte SHA-256), as `package.mcuboot_image` writes it.
TLV_SIZE = 4 + 4 + 32
RAM_START = 0x2000_0000


def measure(path: Path) -> dict:
    elf = Elf(path)
    loaded = [s for s in elf.sections if s.loaded]

    # The image is what `package.py` builds: the header, then exactly what
    # `objcopy -O binary` emits (every section with bytes, by load address,
    # from the lowest to the end of the highest, gaps included), then the
    # TLVs.
    spans = [(elf.load_address(s), s.size) for s in loaded if s.in_image]
    start = min(address for address, _ in spans)
    end = max(address + size for address, size in spans)
    image = package.HEADER_SIZE + (end - start) + TLV_SIZE

    def total(*names: str) -> int:
        return sum(s.size for s in loaded if s.name in names)

    # Static RAM: everything the linker placed in RAM, `.data`, `.bss` and
    # `.uninit`, ends at `__euninit` (cortex-m-rt's; the heap would start
    # there, but there's none). The limit is `_ram_end`, which cortex-m-rt
    # defines as the end of memory.x's `RAM` region: taken from the ELF, so
    # each build is held to the layout it was linked with, and memory.x
    # stays the one place the number is written. (The linker refuses
    # statics that overflow `RAM`, so this mostly reports the headroom.)
    statics = elf.symbol("__euninit") - RAM_START
    in_ram = [s for s in loaded if s.address >= RAM_START]
    statics = max(statics, max(s.address + s.size for s in in_ram) - RAM_START)

    return {
        # The vector table is code as far as size goes.
        "text": total(".vector_table", ".text"),
        "rodata": total(".rodata"),
        "data": total(".data"),
        "bss": total(".bss"),
        "uninit": total(".uninit"),
        "image": image,
        "slot": package.SLOT_SIZE - package.TRAILER_SIZE,
        "flash_start": start,
        "statics": statics,
        "ram": elf.symbol("_ram_end") - RAM_START,
        "stack_top": elf.symbol("_stack_start"),
        "stack_end": elf.symbol("_stack_end"),
    }


# ==============================================================================
# The constraints
# ==============================================================================

# The deepest the stack has been seen to go, on a dev kit: pairing, which
# runs trouble-host's P-256 maths (see the `opt-level` comment in
# Cargo.toml, and CONTRIBUTING's "Stack"; memory.x has an older 7.7 KB). Nothing can measure the stack
# without running the firmware, so this is the number to update when the
# hourly `power:` log shows a deeper one. The check only catches the stack
# region shrinking below it (say, `RAM` grown into it, or the retained
# block grown), not code growing deeper.
STACK_PEAK = 8_900


@dataclass
class Row:
    name: str
    limit: int
    used: int

    @property
    def ok(self) -> bool:
        return self.used <= self.limit


def constraints(sizes: dict, stack_guard: int) -> list[Row]:
    stack = sizes["stack_top"] - sizes["stack_end"] - stack_guard
    return [
        # The image (header, code, TLVs) must end before the slot's trailer,
        # which MCUBoot writes during a swap. The same limit holds for an
        # update over Bluetooth, which lands in the secondary slot, the same
        # size: `dfu.rs` refuses anything bigger, and the layout check below
        # holds its numbers to `package.py`'s. (InfiniTime's DFU, which
        # installs ur the first time, writes into the same slot.)
        Row("image in MCUBoot slot", sizes["slot"], sizes["image"]),
        Row("static RAM (.data+.bss+.uninit)", sizes["ram"], sizes["statics"]),
        Row("stack vs. deepest measured", stack, STACK_PEAK),
    ]


def table(name: str, rows: list[Row]) -> str:
    lines = [f"{name}:", f"  {'constraint':<34}{'limit':>9}{'used':>9}{'free':>9}"]
    for row in rows:
        mark = "" if row.ok else "  <-- DOESN'T FIT"
        free = row.limit - row.used
        lines.append(
            f"  {row.name:<34}{row.limit:>9}{row.used:>9}{free:>9}"
            f" ({free / row.limit:.1%}){mark}"
        )
    return "\n".join(lines)


# ==============================================================================
# Layout agreement
# ==============================================================================

# The flash and RAM layout is written down in several places that can't
# share a definition: memory.x (for the linker), package.py (for the image),
# and Rust constants the firmware uses to find the bootloader's flags, its
# storage, its retained RAM and the update slot. Each is read here from its
# source, so that changing one without the others fails CI rather than a
# watch.

# The nRF52832's internal flash and RAM, and the 4 MiB SPI flash.
FLASH_END = 0x8_0000
RAM_END = 0x2001_0000
SPI_FLASH_END = 0x40_0000
# MCUBoot's swap scratch area, right after the primary slot.
SCRATCH_SIZE = 0x1000
# What the bootloader overwrites on every boot, measured by painting RAM
# over SWD (see memory.x): up to here from the bottom...
BOOTLOADER_LOW = 0x2000_CB80
# ...and down to here from the top. The retained block must be between.
BOOTLOADER_HIGH = 0x2000_DD60


def evaluate(expression: str, names: dict[str, int]) -> int:
    """An integer expression of the kind linker scripts and Rust constants
    use: literals (hex, `_` separators, a `K` suffix), names, `ORIGIN(X)`,
    `LENGTH(X)`, + - * and parentheses. Anything else is an error, so a
    constant written some new way is noticed rather than misread."""
    expression = re.sub(r"\b(\d+)K\b", r"(\1*1024)", expression)
    expression = re.sub(r"\b(ORIGIN|LENGTH)\((\w+)\)", r"\1_\2", expression)

    def walk(node: ast.AST) -> int:
        match node:
            case ast.Expression(body=body):
                return walk(body)
            case ast.Constant(value=int() as value):
                return value
            case ast.Name(id=name) if name in names:
                return names[name]
            case ast.BinOp(left=left, op=ast.Add(), right=right):
                return walk(left) + walk(right)
            case ast.BinOp(left=left, op=ast.Sub(), right=right):
                return walk(left) - walk(right)
            case ast.BinOp(left=left, op=ast.Mult(), right=right):
                return walk(left) * walk(right)
        raise ValueError(f"can't evaluate {ast.unparse(node)!r}")

    return walk(ast.parse(expression.strip(), mode="eval"))


def memory_x() -> dict[str, int]:
    """memory.x's regions (as `ORIGIN_X` and `LENGTH_X`) and symbols."""
    text = re.sub(r"/\*.*?\*/", "", (ROOT / "memory.x").read_text(), flags=re.S)
    names: dict[str, int] = {}
    for name, origin, length in re.findall(
        r"(\w+)\s*:\s*ORIGIN\s*=\s*([^,]+),\s*LENGTH\s*=\s*([^\n]+)", text
    ):
        names[f"ORIGIN_{name}"] = evaluate(origin, names)
        names[f"LENGTH_{name}"] = evaluate(length, names)
    outside = re.sub(r"MEMORY\s*\{.*?\}", "", text, flags=re.S)
    for name, value in re.findall(r"(\w+)\s*=\s*([^;]+);", outside):
        names[name] = evaluate(value, names)
    return names


def rust(file: str, pattern: str) -> int | tuple[int, int]:
    """The value of the constant (or range) `pattern` matches in
    src/`file`: its one group is the expression, or two groups a range."""
    source = (ROOT / "src" / file).read_text()
    found = re.search(pattern, source)
    if found is None:
        sys.exit(f"layout: src/{file} no longer has /{pattern}/; update scripts/size.py")
    values = tuple(evaluate(group, {}) for group in found.groups())
    return values[0] if len(values) == 1 else values


def const(file: str, name: str) -> int:
    return rust(file, rf"const {name}: \w+ = ([^;]+);")


def const_range(file: str, name: str) -> tuple[int, int]:
    return rust(file, rf"const {name}: [\w:]*Range<u32> = ([^;.]+)\.\.([^;]+);")


def layout() -> list[tuple[str, bool, str]]:
    """(what, whether it holds, the numbers) for every place the layout is
    repeated."""
    m = memory_x()
    slot_end = package.SLOT_START + package.SLOT_SIZE
    flash_end = m["ORIGIN_FLASH"] + m["LENGTH_FLASH"]
    retained = m["ORIGIN_RETAINED"]
    retained_end = retained + m["LENGTH_RETAINED"]
    storage = const_range("storage.rs", "RANGE")
    in_flash = rust("field_log.rs", r"let in_flash = \|address: usize\| \((\w+)\.\.(\w+)\)")
    dfu_slot = const("dfu_slot.rs", "SLOT_START")
    spi_regions = sorted(
        [
            ("dfu_slot.rs secondary slot", dfu_slot, dfu_slot + const("dfu_slot.rs", "SLOT_SIZE")),
            ("system_off.rs RECORD", *const_range("system_off.rs", "RECORD")),
            ("field_log.rs REGION", *const_range("field_log.rs", "REGION")),
        ],
        key=lambda region: region[1],
    )

    def same(what: str, expected: int, actual: int) -> tuple[str, bool, str]:
        shown = f"{actual:#x}" if actual == expected else f"{actual:#x}, expected {expected:#x}"
        return what, actual == expected, shown

    def before(what: str, end: int, start: int) -> tuple[str, bool, str]:
        return what, end <= start, f"{end:#x} <= {start:#x}"

    checks = [
        # Internal flash: the image starts after its header and ends, TLVs
        # included, before the slot's trailer; MCUBoot's scratch area, then
        # our storage, follow.
        same("memory.x FLASH origin = slot + header", package.SLOT_START + package.HEADER_SIZE, m["ORIGIN_FLASH"]),
        same("memory.x FLASH end = slot end - trailer - TLVs", slot_end - package.TRAILER_SIZE - TLV_SIZE, flash_end),
        same("memory.x STORAGE origin = slot end + scratch", slot_end + SCRATCH_SIZE, m["ORIGIN_STORAGE"]),
        before("memory.x STORAGE ends within flash", m["ORIGIN_STORAGE"] + m["LENGTH_STORAGE"], FLASH_END),
        same("storage.rs RANGE start = STORAGE", m["ORIGIN_STORAGE"], storage[0]),
        same("storage.rs RANGE end = STORAGE", m["ORIGIN_STORAGE"] + m["LENGTH_STORAGE"], storage[1]),
        same("firmware.rs IMAGE_OK_ADDRESS = slot end - 24", slot_end - 24, const("firmware.rs", "IMAGE_OK_ADDRESS")),
        same("field_log.rs in_flash start = FLASH", m["ORIGIN_FLASH"], in_flash[0]),
        same("field_log.rs in_flash end = slot end", slot_end, in_flash[1]),
        # The update slot in SPI flash: the same size as the primary, and
        # clear of the other things kept there.
        same("dfu_slot.rs SLOT_SIZE = package.py", package.SLOT_SIZE, const("dfu_slot.rs", "SLOT_SIZE")),
        same("dfu_slot.rs TRAILER_SIZE = package.py", package.TRAILER_SIZE, const("dfu_slot.rs", "TRAILER_SIZE")),
        before(f"{spi_regions[-1][0]} ends within the SPI flash", spi_regions[-1][2], SPI_FLASH_END),
        # RAM: statics, then the retained block (where the bootloader
        # doesn't reach), then the stack, up to the top.
        same("memory.x RETAINED origin = RAM end", m["ORIGIN_RAM"] + m["LENGTH_RAM"], retained),
        before("bootloader's statics end before RETAINED", BOOTLOADER_LOW, retained),
        before("RETAINED ends before bootloader's stack", retained_end, BOOTLOADER_HIGH),
        same("activity.rs RETAINED_ADDRESS = RETAINED + 0", retained, const("activity.rs", "RETAINED_ADDRESS")),
        same("clock.rs RETAINED_ADDRESS = RETAINED + 0x20", retained + 0x20, const("clock.rs", "RETAINED_ADDRESS")),
        same("field_log.rs BREADCRUMB_ADDRESS = RETAINED + 0x40", retained + 0x40, const("field_log.rs", "BREADCRUMB_ADDRESS")),
        same("memory.x _stack_end = RETAINED end", retained_end, m["_stack_end"]),
        same("field_log.rs STACK_END = _stack_end", m["_stack_end"], const("field_log.rs", "STACK_END")),
        same("memory.x _stack_start = top of RAM", RAM_END, m["_stack_start"]),
    ]
    for (name, _, end), (next_name, start, _) in zip(spi_regions, spi_regions[1:]):
        checks.append(before(f"{name} ends before {next_name}", end, start))
    return checks


# ==============================================================================
# The report
# ==============================================================================

# Rows of the PR comment, (label, key), each what's used; what's left
# against the slot and RAM follows them.
REPORT = [
    ("image", "image"),
    (".text", "text"),
    (".rodata", "rodata"),
    (".data", "data"),
    (".bss", "bss"),
    (".uninit", "uninit"),
    ("static RAM", "statics"),
]


def signed(delta: int) -> str:
    return f"+{delta:,}" if delta > 0 else f"{delta:,}" if delta < 0 else "±0"


def diff(base: dict, head: dict, base_label: str, head_label: str) -> str:
    lines = [
        MARKER,
        "### Firmware size",
        "",
        f"{head_label}, compared with {base_label}. Sizes in bytes.",
        "",
    ]
    failing = [
        f"`{variant}`: {check}"
        for variant, sizes in head.items()
        for check in sizes.get("failed", [])
    ]
    if failing:
        lines += [":x: **Doesn't fit:**", ""] + [f"- {f}" for f in failing] + [""]

    variants = list(head)
    lines.append("| | " + " | ".join(f"`{v}`" for v in variants) + " |")
    lines.append("|---|" + "---:|" * len(variants))

    def cell(variant: str, key: str) -> str:
        now = head[variant][key]
        before = base.get(variant, {}).get(key)
        if before is None:
            return f"{now:,} (new)"
        if now == before:
            return f"{now:,}"
        return f"{now:,} (**{signed(now - before)}**)"

    for label, key in REPORT:
        lines.append(f"| {label} | " + " | ".join(cell(v, key) for v in variants) + " |")

    # What's left, against each limit, which is what decides whether the
    # next change fits.
    def left(variant: str, used: str, limit: str) -> str:
        sizes = head[variant]
        free = sizes[limit] - sizes[used]
        shown = f"{free:,} ({free / sizes[limit]:.1%})"
        if (before := base.get(variant)) and free != before[limit] - before[used]:
            shown += f" (**{signed(free - (before[limit] - before[used]))}**)"
        return shown

    lines.append("| **slot free** | " + " | ".join(left(v, "image", "slot") for v in variants) + " |")
    lines.append("| **RAM free** | " + " | ".join(left(v, "statics", "ram") for v in variants) + " |")
    if missing := sorted(set(base) - set(head)):
        lines += ["", "Not built for this PR: " + ", ".join(f"`{v}`" for v in missing)]
    lines += [
        "",
        "The stack, above the statics, can only be measured on a watch"
        " (see \"Stack\" in CONTRIBUTING.md).",
    ]
    return "\n".join(lines) + "\n"


# ==============================================================================
# The command line
# ==============================================================================


def annotate(message: str) -> None:
    """In GitHub Actions, an error annotation, which shows on the PR's
    summary rather than only in the log."""
    if os.environ.get("GITHUB_ACTIONS"):
        print(f"::error title=size::{message}")


def variant_name(argument: str) -> tuple[str, Path]:
    if "=" in argument:
        name, path = argument.split("=", 1)
        return name, Path(path)
    path = Path(argument)
    # release.sh's names: `<variant>-<version>.elf`.
    return re.sub(r"-\d+\.\d+\.\d+.*$", "", path.name.removesuffix(".elf")), path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    for command, help in [
        ("check", "check ELFs against the constraints"),
        ("measure", "only measure ELFs, say of another commit, for `diff`"),
    ]:
        subcommand = commands.add_parser(command, help=help)
        subcommand.add_argument("elfs", nargs="+", metavar="[NAME=]ELF")
        subcommand.add_argument("--json", type=Path, help="also write the sizes here")
    compare = commands.add_parser("diff", help="the PR report, from two --json files")
    compare.add_argument("base", type=Path)
    compare.add_argument("head", type=Path)
    compare.add_argument("--base-label", default="the base")
    compare.add_argument("--head-label", default="This PR")
    args = parser.parse_args()

    if args.command == "diff":
        base = json.loads(args.base.read_text())
        head = json.loads(args.head.read_text())
        print(diff(base, head, args.base_label, args.head_label), end="")
        return

    ok = True
    results = {}
    stack_guard = const("field_log.rs", "STACK_GUARD")
    for argument in args.elfs:
        name, path = variant_name(argument)
        sizes = measure(path)
        rows = constraints(sizes, stack_guard)
        print(table(name, rows))
        sizes["failed"] = [
            f"{row.name}: {row.used:,} of {row.limit:,}" for row in rows if not row.ok
        ]
        results[name] = sizes
        if args.command == "check":
            ok &= not sizes["failed"]
            for failure in sizes["failed"]:
                annotate(f"{name}: {failure}")

    if args.json:
        args.json.write_text(json.dumps(results, indent=2) + "\n")
    if args.command == "measure":
        return

    # The layout as this checkout has it (the ELFs don't record it).
    print("layout:")
    for what, holds, shown in layout():
        print(f"  {'ok ' if holds else 'BAD'} {what}: {shown}")
        if not holds:
            annotate(f"layout: {what}: {shown}")
        ok &= holds

    if not ok:
        sys.exit("size: a constraint doesn't hold (see above)")


if __name__ == "__main__":
    main()
