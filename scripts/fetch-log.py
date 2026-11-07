#!/usr/bin/env python3
"""Fetch a watch's field log (see src/field_log.rs) over Bluetooth.

Writes the raw defmt stream, one file per firmware version found in the log
(`field-log-<version>.defmt`), oldest first, and prints how to decode each:

    defmt-print -e ur-<version>.elf < field-log-<version>.defmt

The ELF must be the exact build that wrote the log; `scripts/package.py`
keeps one next to each package it makes.

Needs `bleak`, and the computer paired with the watch (see scripts/dfu.py).

Usage: fetch-log.py --address <MAC> [--out DIR]
"""

import argparse
import asyncio
import struct
import sys
from pathlib import Path

from bleak import BleakClient, BleakScanner
from bleak.backends.device import BLEDevice

CONTROL = "75726c00-6f67-4c6f-8000-000000000002"
DATA = "75726c00-6f67-4c6f-8000-000000000003"
SECTORS = 512


def version_name(word: int) -> str:
    major, minor, revision = word & 0xFF, (word >> 8) & 0xFF, word >> 16
    return f"{major}.{minor}.{revision}"


async def fetch(address: str, out: Path) -> None:
    device = await BleakScanner.find_device_by_address(address, timeout=10.0)
    if device is None and sys.platform == "linux":
        # Already connected (e.g. to BlueZ): see scripts/dfu.py.
        path = f"/org/bluez/hci0/dev_{address.upper().replace(':', '_')}"
        device = BLEDevice(address, None, {"path": path, "props": {}})
    notifications: asyncio.Queue[bytes] = asyncio.Queue()
    sectors = []
    async with BleakClient(device or address, timeout=30.0) as client:
        await client.start_notify(DATA, lambda _, data: notifications.put_nowait(bytes(data)))
        for index in range(SECTORS):
            await client.write_gatt_char(CONTROL, struct.pack("<H", index), response=True)
            header = await asyncio.wait_for(notifications.get(), 10.0)
            _, got, present, sequence, version, length = struct.unpack("<BHBIII", header)
            assert got == index, f"asked for sector {index}, got {got}"
            data = bytearray(length)
            while True:
                message = await asyncio.wait_for(notifications.get(), 10.0)
                if message[0] == 0x02:
                    break
                (offset,) = struct.unpack_from("<H", message, 1)
                data[offset : offset + len(message) - 3] = message[3:]
            if present:
                # A sector cut off by a reset has no length; keep whole
                # frames only (each ends in a zero byte).
                end = data.rfind(b"\0") + 1
                sectors.append((sequence, version, bytes(data[:end])))
            print(f"\rsector {index + 1}/{SECTORS}, {len(sectors)} with data", end="", flush=True)
    print()

    out.mkdir(parents=True, exist_ok=True)
    sectors.sort()
    streams: dict[int, bytearray] = {}
    for _, version, data in sectors:
        streams.setdefault(version, bytearray()).extend(data)
    for version, stream in streams.items():
        name = version_name(version)
        path = out / f"field-log-{name}.defmt"
        path.write_bytes(stream)
        print(f"{path}: {len(stream)} bytes; decode with:")
        print(f"    defmt-print -e ur-{name}.elf < {path}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--address", required=True)
    parser.add_argument("--out", type=Path, default=Path("."))
    args = parser.parse_args()
    asyncio.run(fetch(args.address, args.out))


if __name__ == "__main__":
    main()
