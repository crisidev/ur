# The button grid

Unlabelled buttons on the screen below the clock face, for Home
Assistant, or for Tasker (or any Android automation app) through
Gadgetbridge. Two layouts, picked in Settings:

- **Six buttons** (the default): each tapped, double-tapped, or held.
  Home Assistant offers each button and gesture as a trigger in its
  automation editor, no YAML needed.
- **Two layers**: nine buttons, tapped alone or two in a row, for codes
  1–99. More to choose from, but Home Assistant automations for them
  are written by hand, or set up with a [blueprint](#blueprints).

<p align="center">
  <img src="screenshots/buttons.png" width="150" alt="Six buttons: chunky grey keycaps with red sides, like an arcade cabinet's, in two rows of three; the top middle one is pressed in and white, showing 2 and 2X: 2, double-tapped, just sent">
  <img src="screenshots/buttons-layers.png" width="150" alt="Two layers: nine smaller keycaps in three rows of three; the top right one is pressed in and white, showing 23 and 2X: 2, then 3 double-tapped, just sent">
  <img src="screenshots/settings.png" width="150" alt="The Settings screen: THEME, LIGHT, BUTTONS (set to BTHOME), GRID (set to 6 BUTTONS), and BTHOME KEY, a row each">
</p>

## Six buttons

Numbered in reading order:

```text
1 2 3
4 5 6
```

Each button is a **code**, 1–6, and has three **gestures**: tap, double
tap, and hold. That's 18 events, and a hold's long press and release
(see below).

A tap waits 0.3 s to see whether a double tap follows (a tap on another
button sends it at once), lit while it waits; once sent, the button
shows its number and the gesture, and buzzes.

## Two layers

Nine buttons, numbered like a phone's keypad:

```text
1 2 3
4 5 6
7 8 9
```

Tap one button, or two in a row, and the watch sends a **code** that
reads as what you pressed, with a **gesture** for what the last button
did. The first button is the tens digit, the second the ones, so codes
run 1–99, and none ends in 0 (10, 20, … 90 never occur):

| You press                      | Code                    | Gestures              |
|--------------------------------|-------------------------|-----------------------|
| one button                     | 1–9                     | tap, hold             |
| one, then another (or itself)  | 11–99: 2, then 3 is 23  | tap, double tap, hold |

So 2, then 3 twice, is 23 double-tapped; 1 three times is 11
double-tapped; 4, then holding 7, is 47 held. That's 261
code-and-gesture pairs.

A tap waits briefly to see whether more follows: 0.6 s for a second
button to start, then 0.3 s for a double tap. The buttons it's waiting on
stay lit; once sent, the last one shows the code and buzzes.

## Either way

What each press does is up to Home Assistant or your phone. A hold goes
at once (after 0.6 s, finger still down), and lifting the finger then
sends a **release**, for what should last while you hold, like dimming a
light. Held on, to 1.5 s, it's a **long press** too, sent then, finger
still down (the button shows LONG, and buzzes again), before the
release: a third thing a hold can do. A hold lifted sooner has none. To cancel a tap that's waiting, swipe away or press the button.
Two buzzes, and the code in red, mean it had nowhere to go (only the
app, on a phone that isn't connected), or it couldn't go to Home
Assistant: then **BTHOME NOT SENT** shows too, even if the phone had
it, as both of the watch's Bluetooth connections were in use (see
[FAQ B05](../README.md#b05-the-watch-says-bthome-not-sent)).

## Turning it on

Swipe left from the clock, past the alarms, to **Settings**, and tap
**BUTTONS** until it says where presses go: **BTHOME** (Home
Assistant), **VIA APP** (Gadgetbridge, for Tasker and the like), or
**BOTH**. Tap **GRID** for the layout: **6 BUTTONS** or **2 LAYERS**.
The grid is then the first screen up from the clock.

## Home Assistant

The watch advertises each press over Bluetooth as [BTHome], which Home
Assistant's BTHome integration picks up through any Bluetooth adapter or
[ESPHome Bluetooth proxy][proxy] within range: no phone, no pairing.
Presses are encrypted with a key the watch made for itself.

1. On the watch, **Settings → BTHOME KEY** (there while **BUTTONS** is
   on **BTHOME** or **BOTH**) shows the key as a QR code; scan it with
   the phone's camera, and copy the text it reads. A tap shows the key
   as text instead (32 characters), with the watch's Bluetooth address,
   and another goes back to Settings. The screen stays on meanwhile,
   however long it takes.
2. Press any grid button. Home Assistant discovers a BTHome device with
   that address (Settings → Devices & services); add it, and paste (or
   type) the key.
3. Make each event once: Home Assistant only lists a device's events
   once it has seen them. With six buttons, tap, double-tap, and hold
   (for 1.5 s, for the long press too) each button; with two layers,
   tap, double-tap, and hold any one.

Each press fires a `bthome_ble_event` with the watch's `address`, an
`event_class`, and an `event_type`. The gestures' event types are
BTHome's:

| Gesture              | `event_type`   |
|----------------------|----------------|
| tap                  | `press`        |
| double tap           | `double_press` |
| hold                 | `hold_press`   |
| hold, held to 1.5 s  | `long_press`   |
| lifting after a hold | `hold_release` |

"Hold release" needs Home Assistant 2026.11 or later.

### Blueprints

The easy way: a [blueprint] for each layout gives every button and
gesture an action in one automation, set up in Home Assistant's editor,
no YAML. It needs Home Assistant 2024.10 or later, but not step 3: it
knows all the events up front.

- **Six buttons**: a section per button, with an action for each
  gesture: tap, double tap, hold, long press, and release.

  [![Import the six-button blueprint](https://my.home-assistant.io/badges/blueprint_import.svg)](https://my.home-assistant.io/redirect/blueprint_import/?blueprint_url=https%3A%2F%2Fraw.githubusercontent.com%2Fjonhoo%2Fur%2Fmain%2Fblueprints%2Fur-six-buttons.yaml)
- **Two layers**: a section per button on its own (tap, hold, long
  press, release), eight slots for any code and gesture (23,
  double-tapped, say), and one action for every press, with `code` and
  `gesture` to tell them apart, for the rest.

  [![Import the two-layer blueprint](https://my.home-assistant.io/badges/blueprint_import.svg)](https://my.home-assistant.io/redirect/blueprint_import/?blueprint_url=https%3A%2F%2Fraw.githubusercontent.com%2Fjonhoo%2Fur%2Fmain%2Fblueprints%2Fur-two-layers.yaml)

1. Click the badge for your layout, or in Home Assistant, **Settings →
   Automations & scenes → Blueprints → Import blueprint**, and paste the
   link of [`blueprints/ur-six-buttons.yaml`](../blueprints/ur-six-buttons.yaml)
   or [`blueprints/ur-two-layers.yaml`](../blueprints/ur-two-layers.yaml).
2. Click the blueprint to create an automation from it, pick the watch,
   open the buttons you want, and give them actions. Leave the rest
   empty.

With two layers, a long press or release gets the code of the hold
before it, from the watch's dimmer entities: leave those enabled.

Each event's action starts as the event arrives, even while an earlier
one's still runs, so a hold's action can loop until the release. To dim
the kitchen while you hold, add this as the hold's action (**Edit in
YAML**):

```yaml
repeat:
  sequence:
    - action: light.turn_on
      target: { entity_id: light.kitchen }
      data: { brightness_step_pct: -10 }
    # The next step after 0.3 s, or none: the finger's lifted.
    - wait_for_trigger:
        - trigger: event
          event_type: bthome_ble_event
          event_data:
            device_id: "{{ trigger.event.data.device_id }}"
            event_type: hold_release
      timeout: { milliseconds: 300 }
  until: "{{ wait.trigger is not none or repeat.index >= 20 }}"
```

[blueprint]: https://www.home-assistant.io/docs/automation/using_blueprints/

### With six buttons

Each button is a BTHome button of its own, its event class `button_1` to
`button_6`: on the device's page, six event entities, **Button 1** to
**Button 6**, and in the automation editor, a **Device** trigger for
each button and gesture (the watch, then `button_2` "Double press", say).
No YAML needed; in YAML, 2 double-tapped is:

```yaml
triggers:
  - trigger: event
    event_type: bthome_ble_event
    event_data:
      address: "E5:12:34:56:78:9A"   # from BTHOME KEY
      event_class: button_2
      event_type: double_press
actions:
  - action: light.toggle
    target: { entity_id: light.kitchen }
```

Six buttons leave no room in a press's advertisement for the battery,
so the watch advertises it on its own: a minute after it starts, when
BUTTONS turns Home Assistant on, and then each time it moves by a 5%
step. The device's battery sensor updates then, with no event.

### With two layers

A button can't carry a number, and an encrypted advertisement has room
for six buttons, not one per code (Home Assistant names them by their
place in it). So each press fires two events (the watch's **\*B04**,
by the GRID row, is FAQ B04, which points here), for two kinds of
automation:

| Gesture              | `button` event (any code) | Dimmer event, by the code   |
|----------------------|---------------------------|-----------------------------|
| tap                  | `press`                   | `dimmer_1` `rotate_right`   |
| double tap           | `double_press`            | `dimmer_2` `rotate_right`   |
| hold                 | `hold_press`              | `dimmer_3` `rotate_right`   |
| hold, held to 1.5 s  | `long_press`              | none                        |
| lifting after a hold | `hold_release`            | none                        |

- **The button** is the gesture alone, whichever code it was: the device
  page's **Button** entity, and device triggers in the automation
  editor ("Press" button, "Hold release" button, …).
- **The dimmer** for the gesture turns right by the code (`steps: 23` for
  2, then 3), so one event says both: what most automations want. A
  long press and a release turn none: they follow their hold, which
  said the code (there's no room for a fourth dimmer). For a code's
  long press, trigger on its hold's dimmer, then wait for the "Long
  press" button event.

(See [bthome-design.md](bthome-design.md) for why.) The device also
gets a battery sensor, which updates with each press.

An automation for 2, then 3, double-tapped:

```yaml
triggers:
  - trigger: event
    event_type: bthome_ble_event
    event_data:
      address: "E5:12:34:56:78:9A"   # from BTHOME KEY
      event_class: dimmer_2          # double tap
      event_properties: { steps: 23 }
actions:
  - action: light.toggle
    target: { entity_id: light.kitchen }
```

In the automation editor, that's a **Manual event** trigger, with that
event type and data. What needs no code, such as "any hold ended", is a
**Device** trigger: the watch, then "Hold release" button.

Holding 5 to dim the kitchen, until you let go (or 6 s, should the
release be missed):

```yaml
triggers:
  - trigger: event
    event_type: bthome_ble_event
    event_data:
      address: "E5:12:34:56:78:9A"
      event_class: dimmer_3          # hold
      event_properties: { steps: 5 }
actions:
  - repeat:
      sequence:
        - action: light.turn_on
          target: { entity_id: light.kitchen }
          data: { brightness_step_pct: -10 }
        - delay: { milliseconds: 300 }
      until:
        - condition: or
          conditions:
            - condition: state      # the Button entity: rename it
              entity_id: event.ur_watch_button
              attribute: event_type
              state: hold_release
            - "{{ repeat.index >= 20 }}"
```

With six buttons, the same works with a `button_5` `hold_press` trigger,
and the condition on button 5's own entity.

### Releases, the key, and replays

A release goes once its hold (or long press) has been on the air for
0.8 s, so a quick hold's release can come that late.

The key stays across updates. Should the watch lose track of how many
presses it has sent with it (its flash entry for that lost), it makes a
new key rather than risk reusing an encryption counter with the old one,
which would let a listener decode those presses: Home Assistant then
asks for the key again (or presses just stop arriving), and **BTHOME
KEY** shows the new one to give it.

The watch can't know whether Home Assistant heard: a lit, buzzing button
means it was sent. Home Assistant forgets which presses it has seen when
it restarts, so a press recorded nearby could then be replayed once:
use the grid for lights and scenes, not locks.

[BTHome]: https://bthome.io
[proxy]: https://esphome.io/components/bluetooth_proxy.html

## Gadgetbridge and Tasker

With **BUTTONS** on **VIA APP** or **BOTH**, each press goes to
Gadgetbridge, which sends it on as an Android broadcast intent, for
[Tasker], MacroDroid, Automate, and the like, the same with either
layout:

- action `ur.watch.BUTTON`
- extra `button`: the code, a number: 1–6 with six buttons; 1–99, none
  ending in 0 (`23`), with two layers
- extra `gesture`: `press` (a tap), `double_press` (any of the six
  buttons, or a pair with two layers), `hold_press`, `long_press` (the
  hold, held to 1.5 s), or `hold_release` (lifting after a hold), as
  BTHome names them

1. In Gadgetbridge, in the watch's settings, turn on **Allow Intents**.
   Without it, Gadgetbridge drops presses, and the watch says "INTENTS
   OFF".
2. In Tasker: a profile on **Event → System → Intent Received**, action
   `ur.watch.BUTTON`; in its task, `If %button eq 23 AND %gesture eq
   double_press`.

The phone must be connected; presses while it isn't are dropped.

[Tasker]: https://tasker.joaoapps.com
