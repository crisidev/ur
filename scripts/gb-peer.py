#!/usr/bin/env python3
"""Stand in for Gadgetbridge over Bluetooth: connect to a watch's UART, write
the lines it's given, and pass on what the watch sends back.

For `ur-sim --devkit` (and `--fuzz`), which runs this as a child process and
talks to it a line at a time:

    connect          connect (the computer must be paired with the watch)
    disconnect       disconnect
    send HEX         write these bytes to the UART's RX characteristic, as
                     Gadgetbridge does: the follower frames the line (0x10,
                     the line in ISO-8859-1, a newline); without responses,
                     in chunks of the MTU, as Gadgetbridge writes
    send-acked HEX   the same, each chunk with a response (to compare: `ok`
                     comes once the watch has them all)
    quit             disconnect, and exit

Each is answered by one line: `ok` or `error WHY`. Besides, whenever they
happen:

    rx HEX           a notification on the UART's TX characteristic (part
                     of a line, or several: the follower splits them)
    dropped          the watch disconnected (or this did): connect again
                     with a new process of this, as a second connection in
                     the same one hears every notification twice (bleak's
                     BlueZ backend, it seems)

Needs `bleak` (`uv run --with bleak scripts/gb-peer.py ADDRESS`), and the
computer paired with the watch with its passkey (see scripts/dfu.py): every
service but GAP needs an encrypted, authenticated link.
"""

import argparse
import asyncio
import sys

from bleak import BleakClient, BleakScanner
from bleak.backends.device import BLEDevice

RX = "6e400002-b5a3-f393-e0a9-e50e24dcca9e"
TX = "6e400003-b5a3-f393-e0a9-e50e24dcca9e"


def say(line: str) -> None:
    print(line, flush=True)


async def find(address: str) -> BLEDevice | str:
    device = await BleakScanner.find_device_by_address(address, timeout=10.0)
    if device is None and sys.platform == "linux":
        # Already connected to BlueZ (by an earlier run of this, say), so
        # not advertising: address it by its D-Bus path (as scripts/dfu.py).
        path = f"/org/bluez/hci0/dev_{address.upper().replace(':', '_')}"
        return BLEDevice(address, None, {"path": path, "props": {}})
    return device or address


async def serve(address: str) -> None:
    loop = asyncio.get_running_loop()
    commands: asyncio.Queue[str | None] = asyncio.Queue()
    # stdin in a thread: asyncio's own pipe reading isn't portable, and
    # this is one line at a time.
    loop.run_in_executor(None, read_stdin, loop, commands)

    client: BleakClient | None = None

    def dropped(_: BleakClient) -> None:
        say("dropped")

    while (command := await commands.get()) is not None:
        verb, _, rest = command.partition(" ")
        try:
            if verb == "connect":
                if client is None or not client.is_connected:
                    client = BleakClient(
                        await find(address), timeout=30.0, disconnected_callback=dropped
                    )
                    await client.connect()
                    # BlueZ negotiates the MTU, but bleak only learns it
                    # when asked (else it assumes 23 bytes, and warns).
                    acquire = getattr(client._backend, "_acquire_mtu", None)
                    if acquire is not None:
                        await acquire()
                    await client.start_notify(
                        TX, lambda _, data: say(f"rx {bytes(data).hex()}")
                    )
            elif verb == "disconnect":
                if client is not None and client.is_connected:
                    await client.disconnect()
                client = None
            elif verb in ("send", "send-acked"):
                if client is None or not client.is_connected:
                    raise RuntimeError("not connected")
                data = bytes.fromhex(rest)
                # In chunks that fit one write, as Gadgetbridge splits lines
                # at the MTU (247 with the watch, so 244 bytes a write).
                # Gadgetbridge writes without responses: `ok` then means
                # BlueZ has them queued, not that the watch has them yet.
                size = max(20, client.mtu_size - 3)
                for start in range(0, len(data), size):
                    await client.write_gatt_char(
                        RX, data[start : start + size], response=verb == "send-acked"
                    )
            elif verb == "quit":
                break
            else:
                raise RuntimeError(f"unknown command {verb!r}")
            say("ok")
        # Whatever it is, the follower decides what it means.
        except Exception as e:  # noqa: BLE001
            say(f"error {type(e).__name__}: {e}".replace("\n", " "))
    if client is not None and client.is_connected:
        await client.disconnect()
    say("ok")


def read_stdin(
    loop: asyncio.AbstractEventLoop, commands: "asyncio.Queue[str | None]"
) -> None:
    for line in sys.stdin:
        loop.call_soon_threadsafe(commands.put_nowait, line.strip())
    loop.call_soon_threadsafe(commands.put_nowait, None)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("address", help="the watch's Bluetooth address")
    args = parser.parse_args()
    asyncio.run(serve(args.address))


if __name__ == "__main__":
    main()
