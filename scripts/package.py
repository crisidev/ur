#!/usr/bin/env python3
"""Package the firmware for the PineTime's MCUBoot bootloader.

Takes the linked ELF and produces, next to it:

- `<name>.img`: the MCUBoot image (header + firmware + hash), as it sits in
  the primary slot at 0x8000.
- `<name>-slot.elf`: the same image wrapped in a minimal ELF, because
  `esprobe flash` only takes ELFs. `scripts/run.sh` flashes this over SWD.
- `<name>-dfu.zip`: a Nordic legacy-DFU package of the image, for updating
  over Bluetooth (InfiniTime's DFU, or ours, with nRF Connect / the nRF
  Device Firmware Update app / `scripts/dfu.py`).

This does the work of MCUBoot's `imgtool create` and `adafruit-nrfutil dfu
genpkg` for the one configuration the PineTime bootloader uses (unsigned,
SHA-256 only), so neither tool, nor Python beyond the standard library, is
needed. The output was checked byte-for-byte against InfiniTime's release
packages (header layout, init packet, CRC).

Usage: package.py <elf> [--version MAJOR.MINOR.REVISION]
"""

import argparse
import binascii
import hashlib
import io
import json
import struct
import subprocess
import sys
import zipfile
from pathlib import Path

# ==============================================================================
# Layout (must match memory.x)
# ==============================================================================

SLOT_START = 0x8000
SLOT_SIZE = 0x74000
# Padded from MCUBoot's 32 bytes so the vector table that follows is 256-byte
# aligned, as the Cortex-M4 requires; see memory.x.
HEADER_SIZE = 0x100
# The end of the slot belongs to MCUBoot's swap-state trailer.
TRAILER_SIZE = 0x1B0

IMAGE_MAGIC = 0x96F3B83D
TLV_INFO_MAGIC = 0x6907
TLV_SHA256 = 0x10


def mcuboot_image(firmware: bytes, version: tuple[int, int, int]) -> bytes:
    """Wraps raw firmware (linked to run at SLOT_START + HEADER_SIZE) in an
    MCUBoot v1 image: header, firmware, then a TLV area holding its hash."""
    major, minor, revision = version
    header = struct.pack(
        "<IIHHIIBBHII",
        IMAGE_MAGIC,
        0,  # load address: unused for execute-in-place images
        HEADER_SIZE,
        0,  # protected TLV area: none
        len(firmware),
        0,  # flags: not encrypted, not a RAM-load image
        major,
        minor,
        revision,
        0,  # build number
        0,  # padding
    ).ljust(HEADER_SIZE, b"\xff")
    # The hash covers the header (with its padding) and the firmware.
    digest = hashlib.sha256(header + firmware).digest()
    tlv_entry = struct.pack("<HH", TLV_SHA256, len(digest)) + digest
    tlv_info = struct.pack("<HH", TLV_INFO_MAGIC, 4 + len(tlv_entry))
    image = header + firmware + tlv_info + tlv_entry
    if len(image) > SLOT_SIZE - TRAILER_SIZE:
        sys.exit(f"image is {len(image)} bytes; the slot fits {SLOT_SIZE - TRAILER_SIZE}")
    return image


def wrap_in_elf(data: bytes, address: int) -> bytes:
    """A minimal 32-bit ARM ELF holding `data` at `address`: one loadable
    segment, plus the one section in it (flashers such as probe-rs skip
    segments that no section covers)."""
    ehsize, phentsize, shentsize = 52, 32, 40
    data_offset = ehsize + phentsize
    names = b"\0.text\0.shstrtab\0"
    names_offset = data_offset + len(data)
    section_offset = (names_offset + len(names) + 3) & ~3
    elf_header = struct.pack(
        "<4sBBBB8sHHIIIIIHHHHHH",
        b"\x7fELF",
        1,  # 32-bit
        1,  # little-endian
        1,  # ELF version
        0,  # System V ABI
        b"",
        2,  # executable
        40,  # ARM
        1,  # ELF version
        address,  # entry: irrelevant for flashing
        ehsize,  # program header table offset
        section_offset,
        0x05000400,  # EABI v5, hard-float
        ehsize,
        phentsize,
        1,  # program headers
        shentsize,
        3,  # sections: null, .text, .shstrtab
        2,  # index of .shstrtab
    )
    program_header = struct.pack(
        "<IIIIIIII",
        1,  # PT_LOAD
        data_offset,
        address,  # virtual address
        address,  # physical (load) address: what flashers use
        len(data),
        len(data),
        5,  # read + execute
        4,
    )
    sections = b"\0" * shentsize + struct.pack(
        "<IIIIIIIIII",
        names.index(b".text"),
        1,  # SHT_PROGBITS
        6,  # alloc + exec
        address,
        data_offset,
        len(data),
        0,
        0,
        4,
        0,
    ) + struct.pack(
        "<IIIIIIIIII",
        names.index(b".shstrtab"),
        3,  # SHT_STRTAB
        0,
        0,
        names_offset,
        len(names),
        0,
        0,
        1,
        0,
    )
    body = elf_header + program_header + data + names
    return body.ljust(section_offset, b"\0") + sections


def dfu_package(image: bytes, name: str) -> bytes:
    """A legacy (SDK 11-era, "dfu_version 0.5") Nordic DFU zip, as produced
    by `adafruit-nrfutil dfu genpkg --dev-type 0x0052`, which is what
    InfiniTime's DFU service accepts."""
    crc = binascii.crc_hqx(image, 0xFFFF)  # CRC-16/CCITT-FALSE, as Nordic's
    # Init packet: device type (0x52, "R"), device revision, application
    # version, then the list of acceptable SoftDevices (0xFFFE: any), then the
    # image CRC. 0xFFFF / 0xFFFFFFFF mean "don't check".
    init_packet = struct.pack("<HHIHHH", 0x0052, 0xFFFF, 0xFFFFFFFF, 1, 0xFFFE, crc)
    manifest = {
        "manifest": {
            "application": {
                "bin_file": f"{name}.bin",
                "dat_file": f"{name}.dat",
                "init_packet_data": {
                    "application_version": 0xFFFFFFFF,
                    "device_revision": 0xFFFF,
                    "device_type": 0x52,
                    "firmware_crc16": crc,
                    "softdevice_req": [0xFFFE],
                },
            },
            "dfu_version": 0.5,
        }
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(f"{name}.bin", image)
        archive.writestr(f"{name}.dat", init_packet)
        archive.writestr("manifest.json", json.dumps(manifest, indent=4))
    return buffer.getvalue()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("elf", type=Path)
    parser.add_argument("--version", default="0.1.0")
    args = parser.parse_args()
    version = tuple(int(part) for part in args.version.split("."))
    if len(version) != 3:
        sys.exit("--version must be MAJOR.MINOR.REVISION")

    # `objcopy -O binary` emits every loadable section by load address, from
    # the lowest: here the vector table at 0x8100, through `.data`'s initial
    # values. A section accidentally loaded into RAM would show up as a
    # multi-gigabyte gap, which the size check in `mcuboot_image` catches.
    raw = args.elf.with_suffix(".bin")
    subprocess.run(["llvm-objcopy", "-O", "binary", str(args.elf), str(raw)], check=True)
    firmware = raw.read_bytes()
    raw.unlink()

    image = mcuboot_image(firmware, version)
    stem = args.elf.with_suffix("")
    Path(f"{stem}.img").write_bytes(image)
    Path(f"{stem}-slot.elf").write_bytes(wrap_in_elf(image, SLOT_START))
    Path(f"{stem}-dfu.zip").write_bytes(dfu_package(image, f"{stem.name}-{args.version}"))
    used = len(image) / (SLOT_SIZE - TRAILER_SIZE)
    print(f"package: {len(image)} bytes ({used:.0%} of the slot), version {args.version}")


if __name__ == "__main__":
    main()
