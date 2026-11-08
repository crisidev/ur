"""Decodes the advertisements `cargo run --release` prints with Home
Assistant's own BTHome parser (`bthome-ble`), and checks it fires what
docs/buttons.md says, with the key verified:

    cargo run --release | uv run --with bthome-ble python -I check.py

Also takes a dev kit's log (`bthome: <counter> <hex>` lines, with the
watch's address and key as arguments), to check what went on the air:

    uv run --with bthome-ble python -I check.py ADDRESS KEY < log
"""

import re
import sys

from bthome_ble import BTHomeBluetoothDeviceData
from habluetooth import BluetoothServiceInfoBleak

UUID = "0000fcd2-0000-1000-8000-00805f9b34fb"


def service_data(advert: bytes) -> bytes:
    """The BTHome service data, from the advertising data's structures."""
    i = 0
    while i < len(advert):
        length, kind = advert[i], advert[i + 1]
        if kind == 0x16 and advert[i + 2 : i + 4] == b"\xd2\xfc":
            return advert[i + 4 : i + 1 + length]
        i += 1 + length
    raise ValueError(f"no BTHome service data in {advert.hex()}")


def decode(parser: BTHomeBluetoothDeviceData, address: str, advert: bytes, at: float):
    # Each press is a new advertisement, seconds apart, as on the air.
    info = BluetoothServiceInfoBleak(
        name="Bangle.js ur",
        address=address,
        rssi=-60,
        manufacturer_data={},
        service_data={UUID: service_data(advert)},
        service_uuids=[UUID],
        source="local",
        device=None,
        advertisement=None,
        connectable=True,
        time=at,
        tx_power=None,
    )
    update = parser.update(info)
    assert parser.bindkey_verified, "the key"
    events = {
        key.key: (event.event_type, event.event_properties)
        for key, event in update.events.items()
    }
    battery = [v.native_value for k, v in update.entity_values.items() if k.key == "battery"]
    return events, battery


def main() -> None:
    lines = sys.stdin.read().splitlines()
    if len(sys.argv) == 3:
        address, key = sys.argv[1], sys.argv[2]
        adverts = []
        for line in lines:
            if m := re.search(r"bthome: (\d+) \[?([0-9a-f, ]+)\]?", line):
                adverts.append((None, None, bytes.fromhex(re.sub("[^0-9a-f]", "", m[2]))))
    else:
        address, key = lines[0].split()
        adverts = []
        for line in lines[1:]:
            code, gesture, advert = line.split()
            adverts.append((int(code), gesture, bytes.fromhex(advert)))
    parser = BTHomeBluetoothDeviceData(bindkey=bytes.fromhex(key))
    dimmer = {"press": "dimmer_1", "double_press": "dimmer_2", "hold_press": "dimmer_3"}
    for i, (code, gesture, advert) in enumerate(adverts):
        events, battery = decode(parser, address, advert, 1000.0 + 10 * i)
        print(f"{len(advert)} B: {events}, battery {battery}")
        if code is None:
            continue
        expected = {"button": (gesture, None)}
        if gesture in dimmer:
            expected[dimmer[gesture]] = ("rotate_right", {"steps": code})
        assert events == expected, (events, expected)
    print("ok")


main()
