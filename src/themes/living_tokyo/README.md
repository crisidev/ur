# Living Tokyo

[Try it live](https://jonhoo.github.io/ur/sim/?theme=living_tokyo): the simulator, in your browser.

A city at night on the clock face, in [Tokyo Night]'s colours: the time
and its seconds across the top, the moon as the battery, windows lit by
the day's steps, a beacon blinking and a sign showing the heart rate,
and a billboard for notifications nobody has seen. It's drawn once a
second and stays still in the dim look. No weather.
(`theme-living_tokyo`.)

Every other screen is, for now, a copy of [Retro](../retro/)'s, in
Tokyo Night's colours, to be restyled in the city's look.

## Colour schemes

Two, both for the night: LIVING TOKYO and LIVING STORM, Tokyo Night's
Night and Storm. The dim look (a glance, a notification on a dark
screen, the quiet night) is white and red on black in both.

| LIVING TOKYO | LIVING STORM |
|---|---|
| <img src="shots/clock-living-tokyo.png" width="120" alt="The clock face in LIVING TOKYO: blue-white on near-black indigo, the windows yellow, cyan and magenta"> | <img src="shots/clock-living-storm.png" width="120" alt="The clock face in LIVING STORM: the same, on a lighter, bluer slate"> |

It has no schemes for the day: it's a city at night. Tokyo Night's Day
is one of Retro's schemes instead, TOKYO DAY, beside Retro's TOKYO NIGHT
and TOKYO STORM: Retro's screens in the same colours.

## By day

Its package, `ur-living_tokyo-<version>-dfu.zip`, has Living Tokyo
alone, so it shows the city by day too. FORCE, on the settings' LOOK,
can't change there, as there's no day scheme to force: a tap on it says
so with FAQ D02.

For Tokyo Night's Day by day, build it beside Retro: THEME then steps
through Retro's day schemes, TOKYO DAY among them, from sunrise, and
through both themes' night schemes from sunset.

```sh
cargo build --release --no-default-features --features theme-retro,theme-living_tokyo
```

(Then package it as [AUTHORING-THEMES.md](../../../AUTHORING-THEMES.md#on-a-watch)
says.)

## Last night and LOOK

| Screen | At night (LIVING TOKYO) |
|---|---|
| Last night | <img src="shots/night-last-night.png" width="120" alt="Last night, at night, in LIVING TOKYO"> |
| LOOK, in Settings | <img src="shots/night-look-settings.png" width="120" alt="LOOK's settings, at night, in LIVING TOKYO"> |

## The dim look

| A glance | A notification, with the screen off |
|---|---|
| <img src="shots/dim-glance.png" width="120" alt="A glance: the hours above the minutes, big, white on black"> | <img src="shots/dim-notification.png" width="120" alt="A notification that arrived with the screen off: the app in red, and who it's from, big, in white"> |

These are `shots.txt`'s, drawn in a build with every theme (where
FORCE is a choice), which the simulator's tests check (see
[AUTHORING-THEMES.md](../../../AUTHORING-THEMES.md#the-readme-and-its-screenshots)).
Every other screen, and the clock face in the states it shows
(notifications unseen, a low battery, charging, a second's blink), is
`screens.txt`'s, which they only run, to check that each draws; to look
at them all, run it with `--out` (see its first lines).

## Credits

- Inspired by: Matteo Bigoi's own Living Tokyo face for InfiniTime,
  [`WatchFaceLivingTokyo.cpp`] in his fork of it, which this redraws for
  the 80×80 canvas; and its colours by folke's [Tokyo Night] for Neovim.
  Its other screens are Retro's, copied.
- Contributed by: Matteo Bigoi ([crisidev](https://github.com/crisidev)).
- Maintained by: Matteo Bigoi ([crisidev](https://github.com/crisidev)).

[`WatchFaceLivingTokyo.cpp`]: https://github.com/crisidev/InfiniTime/blob/87341ef4d0b4276e83a3ce8c1d1540fe21adb9d2/src/displayapp/screens/WatchFaceLivingTokyo.cpp
[Tokyo Night]: https://github.com/folke/tokyonight.nvim
