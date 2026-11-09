"""Runs the button grid's blueprints in Home Assistant's own test harness.

Each test validates a blueprint against Home Assistant's blueprint schema,
sets up an automation from it (which validates the substituted
automation), and fires the `bthome_ble_event`s the watch's presses make,
as the BTHome integration fires them (see docs/buttons.md), checking which
actions run.

To run (Python 3.14; the harness pins the Home Assistant release it
tests against, see requirements.txt):

    uv venv -p 3.14 tmp/venv-blueprints
    VIRTUAL_ENV=tmp/venv-blueprints uv pip install -r blueprints/test/requirements.txt
    tmp/venv-blueprints/bin/python -I -m pytest blueprints/test
"""

import asyncio
import pathlib
import shutil

import pytest
from pytest_homeassistant_custom_component.common import (
    MockConfigEntry,
    async_mock_service,
)

from homeassistant.components.blueprint.models import Blueprint
from homeassistant.components.blueprint.schemas import BLUEPRINT_SCHEMA
from homeassistant.core import HomeAssistant
from homeassistant.helpers import (
    device_registry as dr,
    entity_registry as er,
    selector,
)
from homeassistant.setup import async_setup_component
from homeassistant.util import dt as dt_util, yaml as yaml_util

BLUEPRINTS = pathlib.Path(__file__).resolve().parent.parent
ADDRESS = "E5:12:34:56:78:9A"
GESTURES = ["press", "double_press", "hold_press", "long_press", "hold_release"]


# ==============================================================================
# The harness: a config directory with the blueprints, and a watch
# ==============================================================================


@pytest.fixture
def hass_config_dir(hass_tmp_config_dir: str) -> str:
    """Give each test a config directory of its own, with the blueprints."""
    folder = pathlib.Path(hass_tmp_config_dir, "blueprints", "automation", "ur")
    folder.mkdir(parents=True)
    for path in BLUEPRINTS.glob("*.yaml"):
        shutil.copy(path, folder)
    return hass_tmp_config_dir


class Watch:
    """The watch as the BTHome integration sets it up, and its adverts.

    The integration fires one `bthome_ble_event` per event object in an
    advertisement, in order, and right after each, updates that event
    class's `event` entity (its state is the time, its attributes the
    event type and properties). The two-layer blueprint reads those
    entities, so they're mimicked here too, for the classes the watch
    sends.
    """

    def __init__(self, hass: HomeAssistant, name: str, address: str) -> None:
        self.hass = hass
        self.address = address
        entry = MockConfigEntry(domain="bthome", unique_id=address)
        entry.add_to_hass(hass)
        self.device = dr.async_get(hass).async_get_or_create(
            config_entry_id=entry.entry_id,
            identifiers={("bthome", address)},
            connections={(dr.CONNECTION_BLUETOOTH, address)},
            name=name,
        )
        self.entities = {}
        for event_class in [
            *(f"button_{n}" for n in range(1, 7)),
            "button",
            "dimmer_1",
            "dimmer_2",
            "dimmer_3",
        ]:
            self.entities[event_class] = (
                er.async_get(hass)
                .async_get_or_create(
                    "event",
                    "bthome",
                    f"{address}-{event_class}",
                    config_entry=entry,
                    device_id=self.device.id,
                    suggested_object_id=f"{name}_{event_class}",
                )
                .entity_id
            )

    def advert(self, *events: tuple[str, str, dict | None]) -> None:
        """Fire the events of one advertisement, as the integration does."""
        for event_class, event_type, properties in events:
            self.hass.bus.async_fire(
                "bthome_ble_event",
                {
                    "device_id": self.device.id,
                    "address": self.address,
                    "event_class": event_class,
                    "event_type": event_type,
                    "event_properties": properties,
                },
            )
            self.hass.states.async_set(
                self.entities[event_class],
                dt_util.utcnow().isoformat(timespec="microseconds"),
                {"event_type": event_type, **(properties or {})},
            )

    def six(self, button: int, gesture: str) -> None:
        """A press with GRID on 6 BUTTONS: the pressed button's event."""
        self.advert((f"button_{button}", gesture, None))

    def layers(self, code: int, gesture: str) -> None:
        """A press with GRID on 2 LAYERS: the button, then its dimmer.

        A long press turns the hold's dimmer the other way. A release turns
        none: it follows its hold.
        """
        events = [("button", gesture, None)]
        dimmer = {
            "press": ("dimmer_1", "rotate_right"),
            "double_press": ("dimmer_2", "rotate_right"),
            "hold_press": ("dimmer_3", "rotate_right"),
            "long_press": ("dimmer_3", "rotate_left"),
        }.get(gesture)
        if dimmer:
            events.append((*dimmer, {"steps": code}))
        self.advert(*events)


def record(which: str) -> list[dict]:
    """An action that records which input ran, with the run's variables."""
    return [
        {
            "action": "test.record",
            "data": {
                "which": which,
                "code": "{{ code | default(button | default(0)) }}",
                "gesture": "{{ gesture }}",
            },
        }
    ]


def ran(calls) -> list[tuple[str, int, str]]:
    """What ran since the last time, and clears the list."""
    out = [(c.data["which"], int(c.data["code"]), c.data["gesture"]) for c in calls]
    calls.clear()
    return out


async def setup(hass: HomeAssistant, path: str, inputs: dict) -> str:
    """Sets up an automation from the blueprint, and checks it's valid."""
    assert await async_setup_component(
        hass,
        "automation",
        {
            "automation": {
                "id": "watch",
                "alias": "watch",
                "use_blueprint": {"path": f"ur/{path}", "input": inputs},
            }
        },
    )
    await hass.async_block_till_done()
    # An automation that fails to validate is set up "unavailable".
    state = hass.states.get("automation.watch")
    assert state is not None and state.state == "on", state
    return "automation.watch"


# ==============================================================================
# Both blueprints are valid blueprints
# ==============================================================================


@pytest.mark.parametrize("path", ["ur-six-buttons.yaml", "ur-two-layers.yaml"])
def test_schema(path: str) -> None:
    data = yaml_util.load_yaml(BLUEPRINTS / path)
    # Home Assistant's own schema, as on import: the metadata, inputs and
    # sections, each selector's config (`selector.validate_selector`), and
    # that every `!input` is defined.
    blueprint = Blueprint(data, expected_domain="automation", schema=BLUEPRINT_SCHEMA)
    # min_version is met by the release under test.
    assert blueprint.validate() is None

    # What the schema leaves optional, but the blueprint docs ask for
    # (https://www.home-assistant.io/docs/blueprint/schema/).
    metadata = blueprint.metadata
    for key in ["description", "author", "source_url"]:
        assert metadata.get(key), key
    assert metadata["source_url"].endswith(f"/blob/main/blueprints/{path}")
    assert metadata["homeassistant"]["min_version"]
    for key, spec in metadata["input"].items():
        assert spec.get("name"), key
        for name, input_spec in spec.get("input", {}).items():
            assert input_spec.get("name"), name
            assert input_spec.get("selector"), name
            # "All collapsed inputs must also have a defined default."
            if spec.get("collapsed"):
                assert "default" in input_spec, name

    # Every input but the watch is optional, and its default is a value its
    # selector accepts (the schema takes any default).
    for name, spec in blueprint.inputs.items():
        if name == "watch":
            continue
        assert "default" in spec, name
        selector.selector(spec["selector"])(spec["default"])

    # Every input is used.
    assert yaml_util.extract_inputs(data) == set(blueprint.inputs)


# ==============================================================================
# Six buttons
# ==============================================================================


async def test_six_buttons_every_input(hass: HomeAssistant) -> None:
    calls = async_mock_service(hass, "test", "record")
    watch = Watch(hass, "ur_watch", ADDRESS)
    other = Watch(hass, "other", "E5:00:00:00:00:01")
    inputs = {"watch": watch.device.id}
    for n in range(1, 7):
        for gesture in GESTURES:
            inputs[f"button_{n}_{gesture}"] = record(f"{n} {gesture}")
    await setup(hass, "ur-six-buttons.yaml", inputs)

    # Each button and gesture runs its own action, once.
    for n in range(1, 7):
        for gesture in GESTURES:
            watch.six(n, gesture)
            await hass.async_block_till_done()
            assert ran(calls) == [(f"{n} {gesture}", n, gesture)]

    # Another watch, and the two layers' events, run nothing.
    other.six(1, "press")
    watch.layers(23, "double_press")
    watch.layers(23, "hold_press")
    watch.layers(23, "hold_release")
    await hass.async_block_till_done()
    assert ran(calls) == []


async def test_six_buttons_defaults(hass: HomeAssistant) -> None:
    """With only the watch set, every event runs (and does) nothing."""
    calls = async_mock_service(hass, "test", "record")
    watch = Watch(hass, "ur_watch", ADDRESS)
    entity = await setup(hass, "ur-six-buttons.yaml", {"watch": watch.device.id})
    for gesture in GESTURES:
        watch.six(4, gesture)
    await hass.async_block_till_done()
    assert ran(calls) == []
    assert hass.states.get(entity).attributes["current"] == 0


async def test_six_buttons_hold_to_dim(hass: HomeAssistant) -> None:
    """The docs' hold-to-dim loop ends at the release, which runs meanwhile.

    The release comes while the hold's action is still looping, so this is
    what `mode: parallel` is for.
    """
    calls = async_mock_service(hass, "test", "record")
    watch = Watch(hass, "ur_watch", ADDRESS)
    hold = [
        {
            "repeat": {
                "sequence": [
                    *record("dim"),
                    {
                        "wait_for_trigger": [
                            {
                                "trigger": "event",
                                "event_type": "bthome_ble_event",
                                "event_data": {
                                    "device_id": "{{ trigger.event.data.device_id }}",
                                    "event_type": "hold_release",
                                },
                            }
                        ],
                        "timeout": {"milliseconds": 50},
                    },
                ],
                "until": "{{ wait.trigger is not none or repeat.index >= 20 }}",
            }
        }
    ]
    await setup(
        hass,
        "ur-six-buttons.yaml",
        {
            "watch": watch.device.id,
            "button_5_hold_press": hold,
            "button_5_long_press": record("long"),
            "button_5_hold_release": record("release"),
        },
    )
    watch.six(5, "hold_press")
    await asyncio.sleep(0.25)
    watch.six(5, "long_press")
    await asyncio.sleep(0.25)
    watch.six(5, "hold_release")
    await hass.async_block_till_done()
    whats = [which for which, _, _ in ran(calls)]
    # A few steps of dimming, the long press in between (not held back
    # until the hold's loop ends, as `mode: queued` would), the release,
    # and no more dimming after it.
    assert whats.count("dim") >= 3
    assert "dim" in whats[whats.index("long") :]
    assert whats[-1] == "release"
    await asyncio.sleep(0.2)
    await hass.async_block_till_done()
    assert ran(calls) == []


# ==============================================================================
# Two layers
# ==============================================================================


async def test_two_layers(hass: HomeAssistant) -> None:
    calls = async_mock_service(hass, "test", "record")
    watch = Watch(hass, "ur_watch", ADDRESS)
    other = Watch(hass, "other", "E5:00:00:00:00:01")
    inputs = {"watch": watch.device.id, "every_press": record("every")}
    for n in range(1, 10):
        for gesture in ["press", "hold_press", "long_press", "hold_release"]:
            inputs[f"button_{n}_{gesture}"] = record(f"{n} {gesture}")
    inputs |= {
        "slot_1_code": 23,
        "slot_1_gesture": "double_press",
        "slot_1_action": record("slot 23 double"),
        # A slot for a single button's code wins over the button's input.
        "slot_2_code": 2.0,  # the number selector may give a float
        "slot_2_gesture": "press",
        "slot_2_action": record("slot 2 tap"),
        "slot_3_code": 47,
        "slot_3_gesture": "hold_release",
        "slot_3_action": record("slot 47 release"),
        "slot_4_code": 47,
        "slot_4_gesture": "long_press",
        "slot_4_action": record("slot 47 long"),
    }
    await setup(hass, "ur-two-layers.yaml", inputs)

    async def press(code: int, gesture: str) -> list:
        watch.layers(code, gesture)
        await hass.async_block_till_done()
        return sorted(ran(calls))

    # Single buttons: once each (the `button` event's copy runs nothing).
    assert await press(5, "press") == [("5 press", 5, "press"), ("every", 5, "press")]
    assert await press(2, "press") == [("every", 2, "press"), ("slot 2 tap", 2, "press")]

    # A hold of 7: its long press and release get its code.
    assert await press(7, "hold_press") == [
        ("7 hold_press", 7, "hold_press"),
        ("every", 7, "hold_press"),
    ]
    assert await press(7, "long_press") == [
        ("7 long_press", 7, "long_press"),
        ("every", 7, "long_press"),
    ]
    assert await press(7, "hold_release") == [
        ("7 hold_release", 7, "hold_release"),
        ("every", 7, "hold_release"),
    ]

    # Pairs: through a slot, or Every press alone.
    assert await press(23, "double_press") == [
        ("every", 23, "double_press"),
        ("slot 23 double", 23, "double_press"),
    ]
    assert await press(99, "press") == [("every", 99, "press")]
    # A tap in between doesn't confuse which hold a release follows.
    assert await press(47, "hold_press") == [("every", 47, "hold_press")]
    assert await press(47, "hold_release") == [
        ("every", 47, "hold_release"),
        ("slot 47 release", 47, "hold_release"),
    ]
    assert await press(3, "press") == [("3 press", 3, "press"), ("every", 3, "press")]
    assert await press(31, "hold_press") == [("every", 31, "hold_press")]
    assert await press(31, "hold_release") == [("every", 31, "hold_release")]

    # A long press says its own code, so it doesn't need its hold heard:
    # here, the last dimmer heard is a tap of 5.
    assert await press(5, "press") == [("5 press", 5, "press"), ("every", 5, "press")]
    assert await press(47, "long_press") == [
        ("every", 47, "long_press"),
        ("slot 47 long", 47, "long_press"),
    ]
    assert await press(5, "press") == [("5 press", 5, "press"), ("every", 5, "press")]
    assert await press(4, "long_press") == [
        ("4 long_press", 4, "long_press"),
        ("every", 4, "long_press"),
    ]

    # The `button` event's long press runs nothing on its own: the dimmer's
    # carries it, with the code.
    watch.advert(("button", "long_press", None))
    await hass.async_block_till_done()
    assert ran(calls) == []

    # Another watch, and the six buttons' events, run nothing.
    other.layers(5, "press")
    watch.six(1, "press")
    await hass.async_block_till_done()
    assert ran(calls) == []


async def test_two_layers_release_before_any_hold(hass: HomeAssistant) -> None:
    """A release with no dimmer state to take a code from runs nothing."""
    calls = async_mock_service(hass, "test", "record")
    watch = Watch(hass, "ur_watch", ADDRESS)
    await setup(
        hass,
        "ur-two-layers.yaml",
        {"watch": watch.device.id, "every_press": record("every")},
    )
    watch.layers(0, "hold_release")
    await hass.async_block_till_done()
    assert ran(calls) == []


async def test_two_layers_defaults(hass: HomeAssistant) -> None:
    """With only the watch set, every event runs (and does) nothing."""
    calls = async_mock_service(hass, "test", "record")
    watch = Watch(hass, "ur_watch", ADDRESS)
    await setup(hass, "ur-two-layers.yaml", {"watch": watch.device.id})
    for gesture in GESTURES:
        watch.layers(12, gesture)
    await hass.async_block_till_done()
    assert ran(calls) == []
