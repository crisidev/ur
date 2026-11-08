#!/usr/bin/env python3
"""Install a firmware package on a PineTime over Bluetooth, from a computer.

Speaks Nordic's legacy DFU protocol, which both InfiniTime and this firmware
implement (see src/dfu.rs), so it does the first install over InfiniTime as
well as later updates. Takes the `*-dfu.zip` that `scripts/package.py` makes.

Needs `bleak` (`pip install bleak`), and on Linux a BlueZ adapter.

Our firmware only accepts updates from a device paired with the passkey it
shows, so pair first, e.g. in `bluetoothctl`:

    agent KeyboardOnly
    default-agent
    pair <address>          # then type the passkey the watch shows

(InfiniTime doesn't need this, but has an "OTA" switch in its settings since
1.16 that must be on.)

Usage: dfu.py <package.zip> --address <MAC> [--packet-size N]

For testing the watch's checks only, `--corrupt`, `--truncate` and
`--abort-after` damage the update on its way (see `--help`): the watch
must refuse the first two before installing anything, and recover from
the third.
"""

import argparse
import asyncio
import binascii
import json
import struct
import sys
import zipfile

from bleak import BleakClient, BleakScanner
from bleak.backends.device import BLEDevice

CONTROL_POINT = "00001531-1212-efde-1523-785feabcd123"
PACKET = "00001532-1212-efde-1523-785feabcd123"

START, INIT, RECEIVE, VALIDATE, ACTIVATE = 0x01, 0x02, 0x03, 0x04, 0x05
RECEIPT_REQUEST, RESPONSE, RECEIPT = 0x08, 0x10, 0x11
SUCCESS = 0x01
APPLICATION = 0x04
STATUS = {
    0x01: "success",
    0x02: "invalid state",
    0x03: "not supported",
    0x04: "data too big",
    0x05: "CRC error",
    0x06: "operation failed",
}

# The client waits for a receipt every this many packets: flow control, so we
# never get more than this far ahead of the watch's flash writes.
RECEIPT_INTERVAL = 10


def damage(image: bytes, init_packet: bytes, corrupt: int | None, truncate: int | None) -> tuple[bytes, bytes]:
    """Testing aids: flips every bit of the byte at `corrupt`, or cuts the
    image to `truncate` bytes, and gives the init packet the damaged image's
    CRC, as a sender that damaged it before computing the CRC would. So
    the protocol's CRC check passes, and only the watch's own checks of
    the image (see src/mcuboot.rs) stand between it and the bootloader."""
    if corrupt is not None:
        image = image[:corrupt] + bytes([image[corrupt] ^ 0xFF]) + image[corrupt + 1 :]
        print(f"TESTING: corrupted byte {corrupt:#x} of the image")
    if truncate is not None:
        image = image[:truncate]
        print(f"TESTING: cut the image to {truncate} bytes")
    crc = binascii.crc_hqx(image, 0xFFFF)
    return image, init_packet[:-2] + struct.pack("<H", crc)


async def run(
    package: str,
    address: str,
    packet_size: int,
    corrupt: int | None = None,
    truncate: int | None = None,
    abort_after: int | None = None,
) -> None:
    with zipfile.ZipFile(package) as archive:
        manifest = json.loads(archive.read("manifest.json"))["manifest"]["application"]
        image = archive.read(manifest["bin_file"])
        init_packet = archive.read(manifest["dat_file"])
    if corrupt is not None or truncate is not None:
        image, init_packet = damage(image, init_packet, corrupt, truncate)

    notifications: asyncio.Queue[bytes] = asyncio.Queue()

    async def expect(request: int, timeout: float = 10.0) -> bytes:
        while True:
            message = await asyncio.wait_for(notifications.get(), timeout)
            if message[0] == RESPONSE and message[1] == request:
                if message[2] != SUCCESS:
                    sys.exit(f"step {request:#x} failed: {STATUS.get(message[2], message[2])}")
                return message
            if message[0] != RECEIPT:
                print(f"ignoring notification {message.hex()}")

    # BlueZ only connects to devices it has seen advertising recently. If
    # it's already connected (and so not advertising), use that connection.
    print(f"looking for {address}")
    device = await BleakScanner.find_device_by_address(address, timeout=10.0)
    if device is None and sys.platform == "linux":
        # bleak only takes devices it has seen advertising; point it at
        # BlueZ's object for the connected device instead.
        path = f"/org/bluez/hci0/dev_{address.upper().replace(':', '_')}"
        device = BLEDevice(address, None, {"path": path, "props": {}})
    print(f"connecting to {address}")
    async with BleakClient(device or address, timeout=30.0) as client:
        await client.start_notify(CONTROL_POINT, lambda _, data: notifications.put_nowait(bytes(data)))
        control = lambda data: client.write_gatt_char(CONTROL_POINT, bytes(data), response=True)
        packet = lambda data: client.write_gatt_char(PACKET, bytes(data), response=False)

        print(f"starting: {len(image)} bytes")
        await control([START, APPLICATION])
        await packet(struct.pack("<III", 0, 0, len(image)))
        await expect(START)

        print("sending init packet; the watch erases its update slot")
        await control([INIT, 0x00])
        await packet(init_packet)
        await control([INIT, 0x01])
        await expect(INIT, timeout=60.0)

        await control([RECEIPT_REQUEST, *struct.pack("<H", RECEIPT_INTERVAL)])
        await control([RECEIVE])
        packets = [image[i : i + packet_size] for i in range(0, len(image), packet_size)]
        loop = asyncio.get_running_loop()
        started = loop.time()
        for number, chunk in enumerate(packets, start=1):
            if abort_after is not None and number * 100 >= abort_after * len(packets):
                print(f"\nTESTING: disconnecting after {number} of {len(packets)} packets")
                return
            await packet(chunk)
            if number % RECEIPT_INTERVAL == 0 and number != len(packets):
                while True:
                    message = await asyncio.wait_for(notifications.get(), 10.0)
                    if message[0] == RECEIPT:
                        break
                    sys.exit(f"unexpected notification mid-transfer: {message.hex()}")
                sent = number * packet_size
                rate = sent / (loop.time() - started) / 1024
                print(f"\r{sent * 100 // len(image):3}% ({rate:.1f} KiB/s)", end="", flush=True)
        print()
        await expect(RECEIVE, timeout=30.0)

        print("validating")
        await control([VALIDATE])
        await expect(VALIDATE, timeout=30.0)

        print("activating; the watch resets, and the bootloader swaps in the update")
        try:
            await control([ACTIVATE])
        except Exception as e:  # The watch may reset before acknowledging.
            print(f"(activate: {e})")
    print("done. Leave the watch alone for a minute while the bootloader swaps the images.")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("package")
    parser.add_argument("--address", required=True)
    parser.add_argument(
        "--packet-size",
        type=int,
        default=20,
        help="bytes per data packet; InfiniTime only takes 20, ours up to 244 (MTU permitting)",
    )
    testing = parser.add_argument_group(
        "testing aids", "damage the update on purpose, to check that the watch refuses it"
    )
    testing.add_argument(
        "--corrupt", type=lambda s: int(s, 0), metavar="OFFSET", help="flip the image's byte at OFFSET"
    )
    testing.add_argument("--truncate", type=int, metavar="BYTES", help="send only the image's first BYTES")
    testing.add_argument(
        "--abort-after", type=int, metavar="PERCENT", help="disconnect once PERCENT of the image is sent"
    )
    args = parser.parse_args()
    asyncio.run(run(args.package, args.address, args.packet_size, args.corrupt, args.truncate, args.abort_after))


if __name__ == "__main__":
    main()
