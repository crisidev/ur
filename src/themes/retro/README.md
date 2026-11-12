# Retro

[Try it live](https://jonhoo.github.io/ur/sim/?theme=retro): the simulator, in your browser.

The watch's own look, after old handhelds, arcade cabinets, terminals
and printed game manuals: pixel art on the 80×80 canvas, big 5×7 digits
with a drop shadow, few colours a scheme, ramps of a single hue, and
dithering for the shades between. The default theme (`theme-retro`).

## Colour schemes

Retro's own, ARCADE and TERMINAL for the night, and HANDHELD and
PARCHMENT for the day, then any others below; the dim look (a glance, a
notification on a dark screen, the quiet night) is white and red on
black in every one.

| ARCADE | TERMINAL | HANDHELD | PARCHMENT |
|---|---|---|---|
| <img src="shots/clock-arcade.png" width="120" alt="The clock face in ARCADE: white and red on black"> | <img src="shots/clock-terminal.png" width="120" alt="The clock face in TERMINAL: phosphor green on near-black"> | <img src="shots/clock-handheld.png" width="120" alt="The clock face in HANDHELD: four shades of olive green, dark on light, with a dash of red"> | <img src="shots/clock-parchment.png" width="120" alt="The clock face in PARCHMENT: brown ink on cream, with a muted red"> |

### Tokyo Night

TOKYO NIGHT and TOKYO STORM for the night, and TOKYO DAY for the day:
Retro's screens in the colours of [Tokyo Night], the Neovim theme. The
accent is its blue, labels its fg_dark, the heart's highlight its
yellow, and deep and REM sleep its cyan and magenta.

| TOKYO NIGHT | TOKYO STORM | TOKYO DAY |
|---|---|---|
| <img src="shots/clock-tokyo-night.png" width="120" alt="The clock face in TOKYO NIGHT: blue-white on near-black indigo, with a blue accent"> | <img src="shots/clock-tokyo-storm.png" width="120" alt="The clock face in TOKYO STORM: blue-white on a lighter, bluer slate"> | <img src="shots/clock-tokyo-day.png" width="120" alt="The clock face in TOKYO DAY: indigo ink on pale grey-blue"> |

- Inspired by: [Tokyo Night], folke's colour scheme for Neovim.
- Contributed by: Matteo Bigoi ([crisidev](https://github.com/crisidev)).
- Maintained by: Matteo Bigoi ([crisidev](https://github.com/crisidev)).

[Tokyo Night]: https://github.com/folke/tokyonight.nvim

## Last night and LOOK

| Screen | At night (ARCADE) | By day (HANDHELD) |
|---|---|---|
| Last night | <img src="shots/night-last-night.png" width="120" alt="Last night, at night, in ARCADE"> | <img src="shots/day-last-night.png" width="120" alt="Last night, by day, in HANDHELD"> |
| LOOK, in Settings | <img src="shots/night-look-settings.png" width="120" alt="LOOK's settings, at night, in ARCADE"> | <img src="shots/day-look-settings.png" width="120" alt="LOOK's settings, by day, in HANDHELD"> |

## The dim look

| A glance | A notification, with the screen off | A glance on the charger |
|---|---|---|
| <img src="shots/dim-glance.png" width="120" alt="A glance: the hours above the minutes, big, white on black"> | <img src="shots/dim-notification.png" width="120" alt="A notification that arrived with the screen off: the app in red, and who it's from, big, in white"> | <img src="shots/dim-charging.png" width="120" alt="On the charger: the time small in red; the battery, three times the clock face's, with its red bolt and 66%; then FULL IN in red, and 1:12, big, in white"> |

These are `shots.txt`'s, which the simulator's tests check (see
[AUTHORING-THEMES.md](../../../AUTHORING-THEMES.md#the-readme-and-its-screenshots)).
Every other screen is `screens.txt`'s, which they only run, to check
that each draws; to look at them all, run it with `--out` (see its first
lines).

The simulator's [tour](../../../sim/shots/tour/) has most of Retro's
screens too.

## Credits

- Inspired by: no other theme. Its looks are those of old handhelds,
  arcade cabinets and terminals in general, not of any one watch face.
- Contributed by: Jon Gjengset ([jonhoo](https://github.com/jonhoo)).
- Maintained by: Jon Gjengset ([jonhoo](https://github.com/jonhoo)).
