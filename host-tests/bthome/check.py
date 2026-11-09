"""Decodes the advertisements `cargo run --release` prints with Home
Assistant's own BTHome parser (`bthome-ble`), and checks it fires what
docs/buttons.md says, for either layout of the grid, with the key verified;
then feeds them all, in order, through one parser, as Home Assistant does
(its replay and repeat checks span advertisements of every kind):

    cargo run --release | uv run --with bthome-ble python -I check.py

Also takes a dev kit's log (`bthome: <counter> <hex>` lines, with the
watch's address and key as arguments), to check what went on the air,
against the `buttons: <code> <gesture>` line logged before each (the
layout told by the length: 30 bytes for six buttons, 31 for two layers,
20 for the battery alone):

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
    from_log = len(sys.argv) == 3
    if from_log:
        address, key = sys.argv[1], sys.argv[2]
        adverts = []
        # In order: one touch can complete two presses, logged before
        # either is sent. (With BUTTONS on HA, or HA+PHONE: every press
        # is sent.)
        presses = []
        for line in lines:
            if m := re.search(r"buttons: (\d+) ([a-z_]+)", line):
                presses.append((int(m[1]), m[2]))
            if m := re.search(r"bthome: (\d+) \[?([0-9a-f, ]+)\]?", line):
                advert = bytes.fromhex(re.sub("[^0-9a-f]", "", m[2]))
                if len(advert) == 20:
                    # The battery alone (six buttons): no press of its own.
                    adverts.append(("battery", None, None, advert))
                    continue
                layout = "six" if len(advert) == 30 else "two"
                press = presses.pop(0) if presses else (None, None)
                adverts.append((layout, *press, advert))
    else:
        address, key = lines[0].split()
        adverts = []
        for line in lines[1:]:
            layout, code, gesture, advert = line.split()
            adverts.append((layout, int(code), gesture, bytes.fromhex(advert)))
    # A parser for each layout: one keeps the last value of every sensor
    # it has seen (as Home Assistant does), so a battery from a two-layer
    # advertisement would linger in the six buttons'. (The battery alone
    # is its own "layout".)
    parsers = {}
    # What each advertisement should fire, for the pass below.
    expectations = []
    dimmer = {"press": "dimmer_1", "double_press": "dimmer_2", "hold_press": "dimmer_3"}
    for i, (layout, code, gesture, advert) in enumerate(adverts):
        if layout not in parsers:
            parsers[layout] = BTHomeBluetoothDeviceData(bindkey=bytes.fromhex(key))
        events, battery = decode(parsers[layout], address, advert, 1000.0 + 10 * i)
        print(f"{len(advert)} B: {events}, battery {battery}")
        if code is None:
            expectations.append(None)
            continue
        if layout == "battery":
            # The battery alone: no events.
            expected = {}
            assert battery == [code], battery
        elif layout == "six":
            # The pressed button alone, by its place: button_1 to button_6.
            # No battery: there's no room.
            expected = {f"button_{code}": (gesture, None)}
            assert battery == [], battery
        else:
            # The gesture, and the code on the gesture's dimmer.
            expected = {"button": (gesture, None)}
            if gesture in dimmer:
                expected[dimmer[gesture]] = ("rotate_right", {"steps": code})
            # The battery rides along (`main` sends 82%; a watch, its own).
            assert battery == [82] or (from_log and len(battery) == 1), battery
        assert events == expected, (events, expected)
        expectations.append(expected)
    # Then all of them through one parser, in order, as Home Assistant has
    # one per device: the battery's own advertisements between the
    # presses, a second apart (so its packet id check, for 4 s, is on),
    # each heard twice, as a burst's repeats are.
    ha = BTHomeBluetoothDeviceData(bindkey=bytes.fromhex(key))
    at = 2000.0
    for (_, _, _, advert), expected in zip(adverts, expectations):
        events, _ = decode(ha, address, advert, at)
        if expected is not None:
            assert events == expected, ("one parser", events, expected)
        repeat, _ = decode(ha, address, advert, at + 0.02)
        assert repeat == {}, ("a repeat fired", repeat)
        at += 1.0
    print("ok")


main()
