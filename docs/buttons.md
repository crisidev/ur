# The button grid

Nine unlabelled buttons on the screen below the clock face, for Home
Assistant, or for Tasker (or any Android automation app) through
Gadgetbridge.

<p align="center">
  <img src="screenshots/buttons.png" width="150" alt="The button grid: nine chunky grey keycaps with red sides, like an arcade cabinet's, in three rows of three; the top right one is pressed in and white, showing 23 and 2X: 2, then 3 double-tapped, just sent">
  <img src="screenshots/settings.png" width="150" alt="The Settings screen: THEME, LIGHT, BUTTONS (set to HA), and HA KEY, a row each">
</p>

They're numbered like a phone's keypad:

```text
1 2 3
4 5 6
7 8 9
```

Tap one button, or two in a row, and the watch sends a **code** that
reads as what you pressed, with a **gesture** for what the last button
did:

| You press                      | Code                    | Gestures              |
|--------------------------------|-------------------------|-----------------------|
| one button                     | 1–9                     | tap, hold             |
| one, then another (or itself)  | 11–99: 2, then 3 is 23  | tap, double tap, hold |

So 2, then 3 twice, is 23 double-tapped; 1 three times is 11
double-tapped; 4, then holding 7, is 47 held. That's 90 codes, and 261
code-and-gesture pairs. What each does is up to Home Assistant or your
phone.

A tap waits briefly to see whether more follows: 0.8 s for a second
button to start, then 0.4 s for a double tap. The buttons it's waiting on
stay lit; once sent, the last one shows the code and buzzes. A hold goes
at once (after 0.6 s, finger still down), and lifting the finger then
sends a **release**, for what should last while you hold, like dimming a
light. To cancel, swipe away or press the button. Two buzzes, and the
code in red, mean it had nowhere to go (only the phone, which isn't
connected).

## Turning it on

Swipe left from the clock, past the alarms, to **Settings**, and tap
**BUTTONS** until it says where presses go: **HA** (Home Assistant),
**PHONE** (Gadgetbridge), or **HA+PHONE**. The grid is then the first
screen up from the clock.

## Home Assistant

The watch advertises each press over Bluetooth as [BTHome], which Home
Assistant's BTHome integration picks up through any Bluetooth adapter or
[ESPHome Bluetooth proxy][proxy] within range: no phone, no pairing.
Presses are encrypted with a key the watch made for itself.

1. On the watch, **Settings → HA KEY** shows the key as a QR code;
   scan it with the phone's camera, and copy the text it reads. A tap
   shows the key as text instead (32 characters), with the watch's
   Bluetooth address.
2. Press any grid button. Home Assistant discovers a BTHome device with
   that address (Settings → Devices & services); add it, and paste (or
   type) the key.
3. Press each gesture once (tap, double tap, hold): Home Assistant
   only lists a device's events once it has seen them.

Each press fires two events, `bthome_ble_event`s with the watch's
`address`, for two kinds of automation:

| Gesture             | `button` event (any code) | Dimmer event, by the code   |
|---------------------|---------------------------|-----------------------------|
| tap                 | `press`                   | `dimmer_1` `rotate_right`   |
| double tap          | `double_press`            | `dimmer_2` `rotate_right`   |
| hold                | `hold_press`              | `dimmer_3` `rotate_right`   |
| lifting after a hold | `hold_release`           | none                        |

- **The button** is the gesture alone, whichever code it was: the device
  page's **Button** entity, and device triggers in the automation
  editor ("Press" button, "Hold release" button, …). The release needs
  Home Assistant 2026.11 or later.
- **The dimmer** for the gesture turns right by the code (`steps: 23` for
  2, then 3), so one event says both: what most automations want.

(Home Assistant names BTHome's buttons by their place in the
advertisement, `button_1` to `button_9`, and an encrypted advertisement
has room for six; nor can a button carry a number. So the code rides on
dimmers, which can: see [bthome-design.md](bthome-design.md).) The
device also gets a battery sensor, which updates with each press.

An automation for 2, then 3, double-tapped:

```yaml
triggers:
  - trigger: event
    event_type: bthome_ble_event
    event_data:
      address: "E5:12:34:56:78:9A"   # from HA KEY
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

A release goes once its hold has been on the air for 0.8 s, so a quick
hold's release can come that late.

The key stays across updates. Should the watch lose track of how many
presses it has sent with it (its flash entry for that lost), it makes a
new key rather than risk reusing an encryption counter with the old one,
which would let a listener decode those presses: Home Assistant then
asks for the key again (or presses just stop arriving), and **HA KEY**
shows the new one to give it.

The watch can't know whether Home Assistant heard: a lit, buzzing button
means it was sent. Home Assistant forgets which presses it has seen when
it restarts, so a press recorded nearby could then be replayed once:
use the grid for lights and scenes, not locks.

[BTHome]: https://bthome.io
[proxy]: https://esphome.io/components/bluetooth_proxy.html

## Gadgetbridge and Tasker

Each press goes to Gadgetbridge, which sends it on as an Android
broadcast intent, for [Tasker], MacroDroid, Automate, and the like:

- action `ur.watch.BUTTON`
- extra `button`: the code, a number (`23`)
- extra `gesture`: `press` (a tap), `double_press`, `hold_press`, or
  `hold_release` (lifting after a hold), as BTHome names them

1. In Gadgetbridge, in the watch's settings, turn on **Allow Intents**.
   Without it, Gadgetbridge drops presses, and the watch says "INTENTS
   OFF".
2. In Tasker: a profile on **Event → System → Intent Received**, action
   `ur.watch.BUTTON`; in its task, `If %button eq 23 AND %gesture eq
   double_press`.

The phone must be connected; presses while it isn't are dropped.

[Tasker]: https://tasker.joaoapps.com
