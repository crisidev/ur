# Authoring a theme

A theme is how every screen of the watch looks: its drawing, where its
buttons are, and its colours. The watch's own is Retro
([src/themes/retro/](src/themes/retro/)). This is how to add to it: a
colour scheme of Retro's, or a theme of your own, from the first file to
a package on a watch. [CONTRIBUTING.md] covers building, the simulator
and CI in general; this is the theme's part of each.

## A scheme, or a theme?

To the wearer, every colour scheme is a theme of its own: the settings'
THEME row steps through every scheme built in, for the time of day, and
shows the scheme's name. Behind it, though:

- **A colour scheme of Retro's** is Retro's screens, drawn by Retro's
  code, in colours of your own: a palette. It's part of `theme-retro`,
  in every build with Retro (the default), with no feature of its own,
  and costs about 80 bytes of flash. Choose this to recolour the watch.
- **A theme** is screens of your own: other layouts, other glyphs, a
  clock face of its own. It's a feature of its own, `theme-<name>`, in
  only the builds that name it, and costs about 9 KB if it started from
  a copy of Retro's code (below). Choose this to redraw the watch.

## Adding a colour scheme to Retro

Retro's schemes are in
[src/themes/retro/schemes/](src/themes/retro/schemes/) (`mod.rs`): ARCADE
and TERMINAL by night, HANDHELD and PARCHMENT by day. To add some:

1. Make them statics of `Retro`, each with its palette:

   ```rust
   /// Tokyo Night's night: blue-white on near-black indigo.
   #[rustfmt::skip]
   pub static TOKYO_NIGHT: Retro = Retro { palette: palette("TOKYO NIGHT", [
       0x1a1b26, /* 16 colours in all, `palette::Color`'s order */
   ]) };
   ```

   One or two can go in `schemes/mod.rs`; a set from one source, in a
   module of its own beside it, `schemes/tokyo_night.rs` (`use
   super::Retro;` and `use crate::palette::palette;`), with `mod
   tokyo_night;` in `schemes/mod.rs`. Either way it's pure data, using
   nothing but `crate::palette`, as `host-tests/theme` includes it.
2. Add them to `NIGHT` or `DAY` in `schemes/mod.rs`, after Retro's own
   four: ARCADE and HANDHELD must stay first, as a watch with nothing
   saved, one upgraded from 0.10, and one whose saved scheme isn't built
   in all land on them.
3. Check their colours: `cd host-tests/theme && cargo test`. They're
   drawn by Retro's code, so they must keep Retro's colour roles apart
   as Retro's own do (see [Colours](#colours)); a dim look of their own
   isn't needed, as every Retro scheme's dim look is GLANCE. The tests
   and the Kani proofs there (`cargo kani --harness NAME`) go through
   `NIGHT` and `DAY`, so they cover a scheme as soon as it's listed:
   nothing to add.
4. In [Retro's README](src/themes/retro/README.md), add a section for
   them: their clock face, by night and by day as they have, and their
   credits (inspired by, contributed by, maintained by). Add each
   one's clock face to Retro's [shots.txt](src/themes/retro/shots.txt),
   in the night or the day part as it's for: `shot
   clock-tokyo-night.png "tokyo night"` draws the frame showing (the
   clock face) in its colours, as it's drawn like every Retro scheme.
   Approve the shots (see [The README and its
   screenshots](#the-readme-and-its-screenshots)).
5. Name them in Retro's row of [src/themes/README.md](src/themes/README.md),
   the list of themes. Nothing else counts Retro's schemes, so there's
   nothing else to update.

A scheme's name is what THEME shows, on its own, so it must say which
it is without the theme's: "TOKYO NIGHT", not "NIGHT". It must fit
Retro's THEME row: 50 pixels of the small font, about 12 letters
("TOKYO STORMS" fits, 47 px; "TOKYO STORM" is 43), which the
simulator's tests check for every scheme built in. It's also what the
wearer's choice is saved as (a hash of it), so once released, it's for
good. Retro's schemes get no package of their own: they're in every
`ur`.

## A theme of your own

A theme is a Rust type implementing `themes::Theme`
([src/themes/mod.rs](src/themes/mod.rs)), with its colour schemes as
statics of that type, each for the night or for the day. It draws every
screen with its own code. It may use `themes::helpers` (mechanics that
choose no colour, place, size or font: formatting, fitting text, which
row a tap is on; see [The trait](#the-trait)) and the shared crate
modules (below), but no other theme's module: the build refuses a
theme's feature that turns on another, and a theme built alone, as its
package is, can't reach code that isn't there. To start from Retro's
look, start from a copy of it.

### Starting from a copy of Retro

Say the theme is called Neon, its directory `neon` (lowercase letters,
digits and underscores: it's the module's name too, and the feature's,
`theme-neon`, and the package's, `ur-neon`; what people read is its
`theme_name`, anything at all, such as "Living Tokyo" for
`living_tokyo`). From the repository's root:

```sh
scripts/new-theme.sh neon "Neon"
```

does the copying and listing, and says what's next (nothing is
committed): take its screenshots (`UPDATE_SHOTS=1`, below) and look at
them, then restyle it, screen by screen, from Retro's look. For a theme
with schemes of one side only, say which: `--night-only` or
`--day-only` (`scripts/new-theme.sh --night-only neon "Neon"`). Such a
theme shows its schemes by day and by night alike, and its FORCE can't
change (a tap points to FAQ D02); its other side's list is empty
(`pub const DAY: [&Neon; 0] = [];`), and its README, `shots.txt` and
`screens.txt` have its side's half (`screens.txt`'s with switching
off). What it does, which is also what to do by hand:

1. Copies `src/themes/retro/` to `src/themes/neon/`, without Retro's
   README, screenshots, `shots.txt`, `screens.txt`, or any module of
   schemes beside `schemes/mod.rs` (more of Retro's).
2. Renames the type, `Retro`, to `Neon`, in the code (the
   `Scheme::Retro` of `mod.rs`'s `wrap` and `BUILT_IN` becomes
   `Scheme::Neon`), `BUILT_IN`'s `name: "retro"` to `"neon"`, and
   `theme_name`'s `"Retro"` to `"Neon"`; and in the comments, Retro's
   name (the theme's, in prose; the type's and the module's, in
   backticks and paths), re-wrapping any paragraph that grew too long,
   so each file's header says what it draws for Neon. `mod.rs`'s module
   docs become a TODO: say what Neon looks like.
3. Writes `schemes/mod.rs` afresh: the `Neon` type, a placeholder
   scheme a side, `NEON_NIGHT` and `NEON_DAY` (ARCADE's and HANDHELD's
   colours under names of their own, "NEON NIGHT" and "NEON DAY", of at
   most 12 letters, which always fit the THEME row: a longer IDENT's
   first word, or its first 6 letters), in `NIGHT` and `DAY`, and the
   dim look, `GLANCE`, a copy of Retro's, with
   a `const` assertion that it keeps to idle mode's colours
   (`Palette::idle_mode_safe`). Replace the placeholders with the theme's
   own: names that say which they are on their own (no two schemes of
   any theme may share one, as the name is what's saved), fitting the
   THEME row. `GLANCE` is the dim look's palette, not a scheme: its
   name isn't shown or saved, so it needn't change, nor be unique.
   - `Neon` is `repr(transparent)` over its palette, so a scheme's
     static is its palette, where `scripts/screenshot.py --theme` looks.
     Give it fields of its own (a `day: bool`, say) and it becomes
     `repr(C)`, the palette first.
   - `mod.rs`'s `wrap`, which lists the schemes for `BUILT_IN`, starts
     its array from any one of them (`Scheme::Neon(&NEON_NIGHT)`, every
     element then overwritten): keep it naming one that exists.
   - `mod.rs`'s `pub use schemes::{GLANCE, NEON_NIGHT, Neon};` brings
     what it uses (the type, `wrap`'s starting scheme, the dim look)
     from the private `schemes` module, and shows them in the docs:
     rename them there as they're renamed.
4. Drops Retro's `name_fits` (and its `pub use`): it's for the
   simulator's check of every scheme's name against Retro's THEME row,
   which is Retro's. A copy's own THEME row is checked as it's drawn
   (`row_fits`, a debug assertion).
5. Lists it (see [Listing it](#listing-it)): `themes!` and `pub mod` in
   `src/themes/mod.rs`, `theme-neon` and `themes-all` in both
   manifests, and a row in `src/themes/README.md`.
6. Writes its `README.md`, from Retro's tables of its screenshots, with
   its "Try it live" link and TODOs for the rest, and its `shots.txt`
   and `screens.txt`, Retro's for its schemes (see [The README and its
   screenshots](#the-readme-and-its-screenshots)).
7. Copies `host-tests/theme`, Retro's colour checks, to
   `host-tests/neon`, for its schemes, re-wording its comments as in the
   code, and adds it to CI's matrices (see
   [Its colours, checked](#its-colours-checked)).

Retro's code, a file a group of screens, so that a copy can replace one
without touching the rest:

| File | Draws |
|---|---|
| `mod.rs` | The `Theme` impl (which file draws which screen), every tap area, the layout's constants, the handles, and what several files draw: the big digits, and the media actions' icons |
| `clock.rs` | The clock face, and its battery |
| `glance.rs` | A glance: the time, or the next turn |
| `past.rs` | Today, and Last night |
| `alarms.rs` | Alarms, the phone's alarms, the editor, an alarm ringing, and "find my watch" |
| `messages.rs` | A notification, its banner, a call, and music |
| `air.rs` | Air |
| `grid.rs` | The button grid |
| `settings.rs` | The settings screens, Info, and Home Assistant's key |
| `system.rs` | Storage mode's prompt, switching off, pairing, and an update |
| `schemes/mod.rs` | The type, its schemes, and the dim look's palette |

The copy uses only what every theme may: `themes::helpers`, the canvas
and fonts (`crate::canvas`, `crate::font`), the palette
(`crate::palette`), what the UI hands a theme (`crate::themes`, and
`crate::ui`'s `Status`, `Editing` and `Fits`), the data a screen shows
(`alarm`, `aranet`, `battery`, `bthome`, `buttons`, `clock`, `day`,
`gb`, `nav`, `route`, `stages`, `temperature`, `activity`,
`system_off`, `version`), and nothing of `crate::themes::retro`. Built
alone, it needn't use all of those (a build without Retro doesn't
count what no theme uses of them as dead code: see `src/main.rs`), but
its own code is held to it, as its `mod.rs` keeps Retro's
`#![warn(dead_code)]`: restyled, a copy is told what of Retro's it no
longer draws with, to delete.

What it costs: a copy built beside Retro adds about 9 KB of flash (9.2
KB, measured); built alone, as its package is, the same as Retro. No
RAM.

### Starting from nothing

The directory is `src/themes/<name>/`, with a `mod.rs` (and any other
files the code wants), a `README.md`, a `shots.txt` and a
`screens.txt`. A starting
point, with one scheme for the night:

```rust
#![warn(dead_code)] // Its own, even built alone (see src/main.rs).

use super::{BuiltIn, Scheme, Theme};
use crate::palette::{Palette, palette};

/// The theme's type: here only a palette, as every scheme is drawn
/// alike. The palette first (`repr(C)` or `repr(transparent)`), where
/// `scripts/screenshot.py --theme` looks for it.
#[repr(transparent)]
pub struct Neon {
    palette: Palette,
}

/// Its schemes: statics, as the canvas tells palettes apart by address.
pub static NEON: Neon = Neon { palette: palette("NEON", [/* 16 × 0xRRGGBB */]) };

/// The theme in the firmware's list (see `themes::THEMES`): its schemes
/// for each side, in the order THEME steps through them.
pub const BUILT_IN: BuiltIn = BuiltIn {
    name: "neon",
    night: &[Scheme::Neon(&NEON)],
    day: &[],
};

/// The dim look's palette (see `Theme::dim`), checked as it's built.
pub static DIM: Palette = palette("NEON DIM", [/* 16 × 0xRRGGBB */]);
const _: () = assert!(DIM.idle_mode_safe(), "NEON DIM: idle mode's colours only");

impl Theme for Neon {
    fn theme_name(&self) -> &'static str {
        "Neon"
    }
    // Every other method: see below.
}
```

A scheme's fields can carry whatever its drawing needs beside its
palette: a `day: bool`, say, for a clock face that draws the sky (`Face`
says whether it's the dim look, not whether it's day), as each scheme is
`&self` in every method. Bear in mind that a scheme may show on the side
it isn't for: in a build with schemes of one side only, it shows on both,
and the settings' FORCE can pin either side's schemes.

## The trait

Every method takes `&self`, the scheme showing. None has a default:
the compiler lists what's missing. `themes` has what the screens show
beyond `ui::Status` (`Face`, `Glance`, `Music`, `Grid`, …, and
`STEP_GOAL`). Drawing is on the 80×80 canvas
([src/canvas.rs](src/canvas.rs)), in colour roles, with the fonts and
glyphs of [src/font.rs](src/font.rs). Retro's code, a file per group of
screens, is a working example of every method.

[`themes::helpers`](src/themes/helpers.rs) has what any theme is likely
to need, whatever it looks like: mechanics that choose no colour,
place, size or font, each taking those from the theme.

| Helper | Does |
|---|---|
| `countdown`, `hhmm`, `duration`, `hms` | Time left ("4:05", "42 MIN", "2H05"), a time ("06:30"), a timer's length ("3 MIN", "1H30"), an uptime ("49:02:13") |
| `iso_date`, `tenths`, `percent`, `volts` | "2026-11-09", "-4.2", "80%", "3.98V" |
| `hex`, `address` | A byte as two hex digits; a Bluetooth address as people write it |
| `fitted` | A line of text in a face, cut to the room with an ellipsis |
| `centred_lines` | Lines of the small font centred on an x, a pitch apart |
| `overflow` | How far text overflows its room, for `call_overflow` and `music_overflow` |
| `row_at` | Which of a stack of rows (the theme's top, height and count) a tap is on |
| `digit_glyph` | The 5×7 glyph of a digit, a wrong one drawn as 9 rather than panicking |

And `Palette::idle_mode_safe`, a `const fn`, says whether a dim look's
palette keeps to idle mode's colours, for a theme to assert (below).
What stays the theme's: layout, colours, which font where, and what a
screen shows. A helper a theme wants that would also suit a theme of
another look can join them; anything that decides how a screen looks
can't, as using it would be borrowing another theme's look.

The UI draws the screen showing on each pass of its loop: once a second,
after every input and message from the phone, and when the button
grid's timers fire; and while something scrolls (a long caller, artist
or track: see [Scrolling](#scrolling)), every 100 ms. A theme can't ask
for more: an animation advances once a second (Retro's update screen
flickers so), unless it scrolls.

### Names

- `theme_name()`: the theme's name as people read it, the same for
  every scheme: what `ur-sim --list-themes`, the web simulator's picker
  and a release's notes show, and the title of its README. It isn't
  shown on the watch, whose THEME row shows the scheme's own name, as
  the schemes are what it switches between. A scheme's name has to stand
  on its own (see [Adding a colour scheme to Retro](#adding-a-colour-scheme-to-retro)).

### Colours

- `palette()`: the scheme's palette, built with `palette::palette(NAME,
  [16 colours])`. The canvas holds 16 colour roles (`palette::Color`):
  their names say what Retro uses each for, and a theme may use them as
  it likes. NAME is what THEME shows, and what the choice is saved as:
  no two schemes of any theme may share one.
- `dim()`: the palette of the dim look: a glance, a notification that
  arrived with the screen off, and the quiet night. The panel shows it
  in idle mode, which keeps only the top bit of each of red, green and
  blue: every colour must be one of those eight (each channel fully on
  or off), or it comes out as another. A grey can only be a half-tone, a
  colour on every other panel column (`Palette::halftone`). It's the
  theme's own (a copy of GLANCE, if it's Retro's), may be every scheme's,
  and its name isn't shown or saved, so needn't be unique.

The dim look is the same methods, drawn in `dim()`'s palette: `glance`
always, `notification` for one that arrived with the screen off, and
every screen in the quiet night. `Face::dim` and `Grid::dim` say so, for
the clock face and the button grid to draw otherwise there (Retro's
greys the struck-through phone rune, and shades its keycaps for a dark
palette).

### Screens

Each draws its screen from scratch: clear the canvas first, then draw.

| Method | The screen |
|---|---|
| `clock` | The clock face (`Face`: the status, the next alarm, media, unseen notifications, whether it's in the dim look) |
| `glance` | A glance: the time, or the next turn while the phone navigates, and what a media gesture just did; always in the dim look |
| `today`, `last_night` | The day's steps and heart rate by the hour; last night's sleep |
| `alarms`, `phone_alarms` | The alarms, smart wake and timers; the phone's alarms, read-only |
| `notification` | A notification with the whole screen: a page of the history, or one that arrived with the screen off (in the dim look) |
| `music`, `air` | What the phone plays, with its controls; a room sensor's reading |
| `buttons` | The button grid (`Grid`: its layout, what's waiting, what was sent) |
| `settings`, `info` | A settings screen (`SettingsPage`, below); the watch's state |
| `storage_prompt`, `bthome_qr`, `bthome_key` | Asking to switch to storage mode; Home Assistant's key as a QR code (`Canvas::qr`) and as text |
| `editor`, `ringing`, `find`, `call`, `pairing`, `update`, `off` | What takes the screen over: the alarm editor, an alarm or timer ringing, "find my watch", a call, pairing, an update, switching off |

`settings` draws both settings screens, LOOK (THEME, FORCE, LIGHT) and
BUTTON GRID (BUTTONS, GRID, and BTHOME KEY while the grid's presses go
over BTHome), and any the UI adds: a `SettingsPage` is a title and up
to three rows, each already in words (`label`, `value`), so the theme
only lays them out and styles them. Style each row by its `kind`:
`Choice`, a value a tap steps on; `Fixed`, one that can't change in
this build (THEME with one scheme for the side, FORCE with schemes of
one side), whose tap shows a banner with an FAQ code; or `Link`, a
screen to go to (BTHOME KEY's SHOW). `note` is a caveat to draw beside
the value: GRID's "*B04". Draw row `i` in the same place whatever the
others say: every setting keeps its place on its screen, so none moves
under a finger tapping it round (BTHOME KEY, the one that comes and
goes, is last). `settings_row` (below) must agree.

And two drawn over the screen beneath, without clearing it:

- `handles`: where a swipe leads. `Handles::Line`, on a screen out along
  a line; `Handles::Unseen`, on the clock face when notifications came
  in unseen, which may draw nothing if the face shows them itself
  (`Face::unseen`).
- `banner`: a notification, or something to know, over the bottom of
  the screen.

### Where taps land

What a tap does is the UI's; where its targets are, the theme's: each
method says what a touch at (`x`, `y`), in canvas pixels, is on, which
must be where the drawing shows it.

| Method | Answers |
|---|---|
| `alarm_at` | Which alarm (smart wake, an alarm, a timer) a tap or long press is on, if any |
| `editor_button` | Which of the editor's buttons (switch, next or done) a tap is on, if any |
| `editor_field` | Whether a swipe up or down changes the hours or the minutes |
| `settings_row` | Which of a settings screen's rows (its index in `page.rows`, as `settings` draws them) a tap is on, if any |
| `grid_button` | Which button of the grid a touch is on: always one, 1 to `layout.buttons()` (Retro uses `buttons::at`) |
| `call_button` | Decline or answer, if either |
| `music_control` | Previous, play or pause, or next, if any |
| `storage_confirms` | Whether a tap there on storage mode's prompt switches off (anything else backs out) |

Swipes that move between screens, the clock face's media gestures, and
taps anywhere (to snooze, to find the watch) stay the UI's.

### Scrolling

`call_overflow` and `music_overflow` say how many pixels too wide a
caller's name, or the music's artist or track, is for the room the
theme gives it (0 or less if it fits). The UI draws frames every 100 ms
while something overflows, and keeps the music screen on for one whole
scroll (`Canvas::marquee` does the scrolling).

## Listing it

In [src/themes/mod.rs](src/themes/mod.rs):

```rust
#[cfg(feature = "theme-neon")]
pub mod neon;
```

(rustfmt sorts the `pub mod` lines by name; their order doesn't
matter.) And a line in `themes!`, which makes its `Scheme` variant, its
arm of the dispatch, and its place in `THEMES`: the order THEME steps
through the themes, so Retro stays first:

```rust
themes! {$
    retro: Retro if "theme-retro";
    neon: Neon if "theme-neon";
}
```

In both [Cargo.toml](Cargo.toml) and [sim/Cargo.toml](sim/Cargo.toml),
under `[features]`: `theme-neon = []`, turning on nothing (no other
theme), and `"theme-neon"` in `themes-all`. Keep each feature on one
line, as the build reads them line by line.

And a line in [src/themes/README.md](src/themes/README.md), the list of
themes, linking `(neon/)`, and the simulator showing it (see [The README
and its screenshots](#the-readme-and-its-screenshots)).

Leave any of these out and the build, or the simulator's tests, say
what to add where. Without the `pub mod` line, the compiler can't find
the module `themes!` names; for the rest, the build scripts
(`src/themes/manifests.rs`) check every directory against `themes!` and
both manifests:

```text
error: ur@0.10.0: src/themes/neon/ isn't in `themes!` (src/themes/mod.rs): add `neon: <its type> if "theme-neon";`
error: ur@0.10.0: src/themes/neon/ has no feature in the simulator's sim/Cargo.toml: add `theme-neon = []` under [features]
error: ur@0.10.0: the firmware's Cargo.toml leaves theme-neon out of `themes-all`: add "theme-neon" to it
error: ur@0.10.0: the firmware's Cargo.toml: theme-neon turns on theme-retro, but a theme draws every screen itself, without another theme's code: …
```

## Trying it

In the simulator ([sim/README.md](sim/README.md)), with only your theme,
or beside Retro, in the window (the mouse is the finger; the terminal
takes commands, as `theme NEON` to pick a scheme):

```sh
cd sim
cargo run --release --features theme-neon
cargo run --release --features theme-retro,theme-neon
```

On a dev kit, as any build ([CONTRIBUTING.md](CONTRIBUTING.md#iterating)),
and `scripts/screenshot.py --theme NEON` (a scheme's static's name, in
any case) draws a frame in a scheme's colours:

```sh
cargo run --release --no-default-features --features theme-neon
```

Which scheme shows: what the wearer last picked for the side, by its
name. A fresh install, with nothing saved, shows ARCADE by night and
HANDHELD by day in a build with Retro, else each side's first scheme
in `themes!` order; so does a watch whose saved scheme isn't built in
(and keeps it saved, should a build with it come back). One upgraded
from 0.10 shows the Retro scheme it had.

## The README and its screenshots

`README.md` shows what the theme looks like, under its `theme_name` as
the title. Right under the title, a link to it live, in the published
simulator (<https://jonhoo.github.io/ur/sim/>, as of `main`), by its
directory's name:

```markdown
[Try it live](https://jonhoo.github.io/ur/sim/?theme=neon): the simulator, in your browser.
```

That opens the simulator in the theme's first scheme for the side
the sun says (or its other side's, if it has none for this one);
`&side=day` or `&side=night` picks the side, and `?scheme=NEON%20DAY`
a scheme by its name, in any case. Its row in `src/themes/README.md`
links the same, as "Try it". The link only works once the theme is
on `main`, which the site is published from: until then, the
simulator shows its default and says so in the browser's console.

Then a few screenshots:

- the clock face in each of its schemes (so each side's too);
- for each side it has (night, day), in one scheme of it: Last night,
  and LOOK (Settings);
- the dim look: a glance, and a notification that came with the screen
  off. Once, if its dim look is the same in every scheme (as Retro's
  is, and a copy of Retro's); a side each if not.

Those are all: every committed screenshot is a Git LFS object, and so
is every approval of one, so a theme shows how it looks, not every
screen. Every other screen is still drawn, by CI, as a smoke test (see
below). And its credits:

- Inspired by: the theme or watch face it's after, if any, with a link.
- Contributed by: who made it.
- Maintained by: who looks after it now.

Retro's [README](src/themes/retro/README.md) is the model.

Two simulator scripts (their commands are `ur-sim --help`'s) go with
it, each starting from a copy of Retro's with its schemes changed:

- `shots.txt` takes the README's screenshots, and only those: Retro's
  is [shots.txt](src/themes/retro/shots.txt).
- `screens.txt` visits every screen, by night and by day where it has
  that side, and the dim look: Retro's is
  [screens.txt](src/themes/retro/screens.txt).

Their first line,

```text
version 1.0.0+1a2b3c4
```

makes the Info screen show a version of its own, rather than the
build's, which would change it at every release. Pick each side with
`theme NAME`, a scheme's name, in quotes if it has a space, in any case
(`theme "tokyo night"`), and `shot NAME.png` each screen. (`shot
NAME.png SCHEME` redraws the frame in another scheme's colours, which
is only right for a scheme drawn like the one showing: for one whose
drawing differs, pick it with `theme`.) Draw them, and look at them
all, every screen's as well as the README's:

```sh
cd sim
cargo run --release --features theme-neon -- --out ../src/themes/neon/shots ../src/themes/neon/shots.txt
cargo run --release --features theme-neon -- --out ../tmp/neon ../src/themes/neon/screens.txt
```

The README's are approved screenshots, in Git LFS like every PNG: the
simulator's tests check every theme's against its `shots/`, pixel for
pixel, and that its README shows each of them and no other. Approve
what `shots.txt` draws once it's right, and after any change you mean:

```sh
UPDATE_SHOTS=1 cargo test --no-default-features --features themes-all --test scripts every_themes_screenshots
```

`screens.txt`'s are never approved or committed: the tests only run it
(`every_themes_screens`, with `ur-sim --no-shots`, which writes no
PNG), and fail if it doesn't get to the end, on a panic, a debug
assertion (a `row_fits`, say) or a step of the script that fails (an
`expect`, a scheme that isn't one).

Approved shots are drawn with every theme built in (`themes-all`), so
they can differ from what the theme looks like alone, as its package
is. On LOOK, THEME and FORCE step through every theme's schemes: a
one-sided theme's FORCE is a choice there, and bright, as Retro brings
the other side, but dim and fixed alone, a tap showing "NO DAY THEMES"
or "NO NIGHT THEMES" (FAQ D02); likewise THEME with one scheme for the
side showing ("ONLY ONE THEME", FAQ D01). So the approved shots never
show that; look at a build of it alone (the `--out` runs above, with
`--features theme-neon`) to see it.

When a change to a script moves later steps in time (another swipe, a
longer wait), shots after it can change a little with no change to the
drawing: the clock's seconds, an uptime, a marquee's offset, an
update's progress, all following the simulator's clock. Check that the
differences are only that before approving them.

## Its colours, checked

Retro's colours have host tests of their own (`host-tests/theme`), as
its screens depend on which roles differ: deep and REM sleep on Last
night, a grid button's parts, the dim look's idle-mode colours; and
Kani proofs about every role of every scheme in `NIGHT` and `DAY`. A
theme's palettes and checks are its own. One that started from Retro's
drawing and kept its role meanings can keep its checks too:
`scripts/new-theme.sh` copies them, or by hand:

1. `cp -r host-tests/theme host-tests/neon`, without its `target/`
   (`rm -r host-tests/neon/target`), the build's, not the source's.
2. In `host-tests/neon/src/main.rs`, point the `#[path]` of `mod retro`
   at `src/themes/neon/schemes/mod.rs`, and rename the module and
   `Retro` to Neon's.
3. Delete the tests of Retro's own schemes: `the_old_collision_fails`,
   about ARCADE's reds.
4. Name the crate in both its `Cargo.toml` and its `Cargo.lock`
   (`name = "neon-theme-test"`), as CI's `--locked` wants them to agree.

CI formats and tests every crate in `host-tests/`, so there's no list
to add it to.

## What CI checks

- That it builds: alone (as its own package is built), linted, with
  clippy's warnings as errors (its own dead code among them), and with
  every theme and feature (`features.yml`); clippy with `themes-all`
  (`check.yml`).
- The simulator's tests, all of them, with every theme built in: your
  README's screenshots against `shots/` (and that the README shows
  exactly those), that `screens.txt` draws every screen to the end,
  that it's in `src/themes/README.md`, that both link it live,
  that every built-in theme has a scheme, that no two schemes share a
  name, and that every scheme's name fits Retro's THEME row; and the
  rest of the tests, so nothing assumes Retro is the only theme.
- On a pull request, `screenshots.yml` draws every script's
  screenshots, your `shots.txt`'s included (not `screens.txt`'s), for
  the PR and for where it branched off, and `screenshots-comment.yml`
  keeps one comment on the PR with each screen that changed (before,
  after, and where, side by side), was added, or was removed. A new
  theme's all show as added.

## On a watch

Every theme but the default gets a package of its own in each release,
`ur-<name>-<version>-dfu.zip` (the notes call it by its `theme_name`):
`ur` with only that theme. One with schemes of one side only shows them
by day and by night alike, and its FORCE can't change (a tap points to
the README's FAQ D02); the notes say so beside it. (On a pull request, CI's
packaged build has it too, in the run's `ur-pr.<number>` artifact.)
`scripts/release.sh` builds it, and checks it fits the watch's flash
and RAM. Install it as any release, over Bluetooth (the README's
[Installing](README.md#installing)).

To try a mix on a watch before a release, build it, alone or with
another, and package it for Bluetooth:

```sh
cargo build --release --no-default-features --features theme-retro,theme-neon
python3 scripts/package.py target/thumbv7em-none-eabihf/release/ur --version 0.10.0
```

(`--version`: Cargo.toml's.) That writes `ur-dfu.zip` beside the ELF.
`scripts/size.py check target/thumbv7em-none-eabihf/release/ur` says how
much flash and RAM it leaves.

[CONTRIBUTING.md]: CONTRIBUTING.md
