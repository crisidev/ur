# Home Assistant blueprints

One automation for every button on ur's button grid, set up in Home
Assistant's editor. Add the watch to Home Assistant's BTHome integration
first; [docs/buttons.md](../docs/buttons.md#blueprints) has the details.

## Six buttons

For **GRID** on **6 BUTTONS**: an action for each button's tap, double
tap, hold, long press, and release.

[![Open your Home Assistant instance and show the blueprint import dialog with a specific blueprint pre-filled.](https://my.home-assistant.io/badges/blueprint_import.svg)](https://my.home-assistant.io/redirect/blueprint_import/?blueprint_url=https%3A%2F%2Fraw.githubusercontent.com%2Fjonhoo%2Fur%2Fmain%2Fblueprints%2Fur-six-buttons.yaml)

## Two layers

For **GRID** on **2 LAYERS**: an action for each button on its own (tap,
hold, long press, release), eight slots for any code and gesture (23,
double-tapped, say), and one action for every press, with `code` and
`gesture`, for the rest.

[![Open your Home Assistant instance and show the blueprint import dialog with a specific blueprint pre-filled.](https://my.home-assistant.io/badges/blueprint_import.svg)](https://my.home-assistant.io/redirect/blueprint_import/?blueprint_url=https%3A%2F%2Fraw.githubusercontent.com%2Fjonhoo%2Fur%2Fmain%2Fblueprints%2Fur-two-layers.yaml)

Both need Home Assistant 2024.10 or later (2026.11 for release).
[test/](test/) runs them in Home Assistant's test harness.
