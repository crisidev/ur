#!/usr/bin/env bash
# Starts a new theme as a copy of Retro: the mechanical steps of
# AUTHORING-THEMES.md's "Starting from a copy of Retro", so that what's
# left is the theme itself.
#
#     scripts/new-theme.sh [--night-only | --day-only] IDENT "Display Name"
#
# IDENT is the theme's directory, module, and feature (`theme-IDENT`):
# lowercase letters, digits and underscores, starting with a letter.
# "Display Name" is what people read (`Theme::theme_name`): anything.
# `--night-only` or `--day-only` makes a theme with schemes of that side
# only, which it shows by day and by night alike.
#
# It writes:
#
# - src/themes/IDENT/: Retro's code (not its README, shots, or any of its
#   schemes beyond the four in `schemes/mod.rs`), with its type renamed
#   (`living_tokyo` gets `LivingTokyo`) and its comments re-wrapped where
#   that changed them, its module docs saying what it is, and a
#   placeholder scheme a side, "<NAME> NIGHT" and "<NAME> DAY" (Retro's
#   ARCADE's and HANDHELD's colours, under names of their own, at most
#   12 letters, which always fit the THEME row), which the theme
#   replaces. Retro's `name_fits` goes, as only the simulator's check of
#   Retro uses it.
# - Its README.md (to fill in) and shots.txt (Retro's, for its schemes,
#   a side's half only for a one-sided theme).
# - The lines that list it: `themes!` and `pub mod` in
#   src/themes/mod.rs, its feature and `themes-all` in both Cargo.toml
#   and sim/Cargo.toml, and a row in src/themes/README.md.
# - host-tests/IDENT: a copy of host-tests/theme's checks of Retro's
#   colours, for its schemes (without `target/`, and without the test of
#   an old collision of ARCADE's), with CI's fmt and test matrices
#   running it.
#
# Then, from sim/, `UPDATE_SHOTS=1 cargo test --release
# --no-default-features --features themes-all` takes its screenshots
# (look at them: they're Retro's look, in the placeholders' colours), and
# the theme is the author's to restyle. Nothing is committed. It refuses
# an IDENT that's taken.
set -euo pipefail

sides=both
case ${1-} in
  --night-only) sides=night && shift ;;
  --day-only) sides=day && shift ;;
esac
if (($# != 2)); then
  echo "usage: scripts/new-theme.sh [--night-only | --day-only] IDENT \"Display Name\"" >&2
  exit 2
fi
ident=$1
display=$2
# `ur-logging` is a package's name already, and `helpers` a module's.
if [[ ! $ident =~ ^[a-z][a-z0-9_]*$ || $ident == logging || $ident == helpers ]]; then
  echo "a theme can't be called '$ident': lowercase letters, digits and underscores, starting with a letter, not 'logging' or 'helpers'" >&2
  exit 1
fi
# It goes in Rust string literals as it is.
if [[ -z $display || $display == *[\"\\]* ]]; then
  echo "a theme's name can't be empty, nor have a quote or a backslash in it" >&2
  exit 1
fi
cd "$(dirname "$0")/.."
for taken in "src/themes/$ident" "host-tests/$ident"; do
  if [[ -e $taken ]]; then
    echo "$taken exists already" >&2
    exit 1
  fi
done

python3 - "$ident" "$display" "$sides" <<'EOF'
import re
import shutil
import sys
import textwrap
from pathlib import Path

ident, display, sides = sys.argv[1], sys.argv[2], sys.argv[3]
kind = "".join(part.capitalize() for part in ident.split("_"))
if kind == "Retro":
    sys.exit("that's Retro's type name")
dashed = ident.replace("_", "-")
retro = Path("src/themes/retro")
theme = Path("src/themes") / ident

# The placeholders' names. The THEME row fits 50 px of the small font,
# whose ASCII glyphs are all 3 px and a gap: 12 characters always fit
# ("TOKYO STORMS" is 47 px). So the IDENT's first 6 (" NIGHT" is the
# other 6), cut at a word if it can be.
stem = ident.upper().replace("_", " ")
if len(stem) > 6:
    cut = stem[:7].rfind(" ")
    stem = (stem[:cut] if cut > 0 else stem[:6]).strip()
statics = {
    "night": (f"{stem.replace(' ', '_')}_NIGHT", f"{stem} NIGHT", "ARCADE"),
    "day": (f"{stem.replace(' ', '_')}_DAY", f"{stem} DAY", "HANDHELD"),
}
shown = ["night", "day"] if sides == "both" else [sides]
if len(ident) > 6:
    named = " and ".join(statics[side][1] for side in shown)
    print(f"placeholder schemes: {named} (THEME fits 12 letters)", file=sys.stderr)
first = statics[shown[0]][0]


def edit(path, *pairs, count=1):
    """Replaces each `old` with `new` in `path`, which must have it."""
    text = path.read_text()
    for old, new in pairs:
        if old not in text:
            sys.exit(f"{path}: no {old!r}: has Retro changed? (update scripts/new-theme.sh)")
        text = text.replace(old, new, count)
    path.write_text(text)


# ==============================================================================
# Renaming: Retro's names as the new theme's
# ==============================================================================

COMMENT = re.compile(r"^(\s*//[/!]? ?)(.*)$")


def rename_comment(text):
    """In a comment's text: in backticks, the type and the module (code);
    in paths, the module; elsewhere, the theme's name as people read it."""
    parts = re.split(r"(`[^`]*`)", text)
    for i, part in enumerate(parts):
        if i % 2:
            part = re.sub(r"\bRetro\b", kind, part)
            part = re.sub(r"\bretro\b", ident, part)
        else:
            part = re.sub(r"(?<=[/:])retro\b|\bretro(?=[/:])", ident, part)
            part = re.sub(r"\b[Rr]etro\b", display, part)
        parts[i] = part
    return "".join(parts)


def rename_code(line):
    line = line.replace('"Retro"', f'"{display}"')
    line = re.sub(r"\bRetro\b", kind, line)
    line = re.sub(r"\bARCADE\b", first, line)
    return re.sub(r"\bretro\b", ident, line)


def rename(source, width=76):
    """Renames Retro in Rust `source`, and re-wraps each comment paragraph
    the renaming changed, to `width` columns: prose only, not lists,
    tables or code."""
    out = []
    lines = source.split("\n")
    i = 0
    while i < len(lines):
        found = COMMENT.match(lines[i])
        if not found:
            out.append(rename_code(lines[i]))
            i += 1
            continue
        # A paragraph: comment lines with the same prefix, up to a blank
        # one, or a list's item, a table's row or code.
        prefix = found.group(1).rstrip() + " "
        paragraph = []
        while i < len(lines):
            line = COMMENT.match(lines[i])
            if not line or line.group(1).rstrip() + " " != prefix:
                break
            body = line.group(2)
            special = not body or re.match(r"(- |\* |\||```|\d+\. | )", body)
            if special and paragraph:
                break
            paragraph.append(body)
            i += 1
            if special:
                break
        renamed = [rename_comment(body) for body in paragraph]
        prose = not any(re.match(r"(- |\* |\||```|\d+\. | )", b) or not b for b in paragraph)
        if renamed != paragraph and prose and any(len(prefix + b) > width for b in renamed):
            renamed = textwrap.wrap(
                " ".join(renamed),
                width - len(prefix),
                break_long_words=False,
                break_on_hyphens=False,
            )
        out.extend((prefix + b).rstrip() for b in renamed)
    return "\n".join(out)


# ==============================================================================
# The code: Retro's, renamed
# ==============================================================================

shutil.copytree(
    retro,
    theme,
    ignore=lambda dir, names: [n for n in names if n in ("README.md", "shots", "shots.txt")],
)
for path in theme.rglob("*.rs"):
    path.write_text(rename(path.read_text()))

# Its module docs say what it is, until the author does.
mod = theme / "mod.rs"
text = mod.read_text()
docs = re.match(r"(//!.*\n)+", text)
one_sided = {
    "both": "",
    "night": " It has schemes for the night only, which it shows by day too.",
    "day": " It has schemes for the day only, which it shows by night too.",
}[sides]
mod.write_text(
    "\n".join(
        "//! " + line
        for line in textwrap.wrap(
            f"{display}: TODO: what it looks like, and what it's after. Started "
            f"as a copy of Retro (`scripts/new-theme.sh`), so it draws every "
            f"screen as Retro does until restyled. Its schemes are in "
            f"[`schemes`].{one_sided}",
            72,
            break_long_words=False,
        )
    )
    + f"\n//!\n//! The screens are drawn in the files beside this one, a file a group of\n"
    f"//! screens; [`{kind}`]'s [`Theme`] says which draws which.\n"
    + text[docs.end():]
)
# `name_fits` is for the simulator's check of Retro's names, not a copy's.
edit(
    mod,
    ("#[allow(unused_imports)] // For the simulator's tests.\npub use settings::name_fits;\n", ""),
)
settings = theme / "settings.rs"
text = settings.read_text()
text = re.sub(r"/// Whether a scheme's name fits on the THEME row.*?\n}\n\n", "", text, count=1, flags=re.S)
settings.write_text(text)

# The schemes: a placeholder a side, in Retro's colours under names of
# their own, and the dim look; any modules of Retro's beside
# `schemes/mod.rs` (more of its schemes) aren't the new theme's.
schemes = theme / "schemes"
for path in schemes.iterdir():
    if path.name != "mod.rs":
        path.unlink() if path.is_file() else shutil.rmtree(path)
source = (retro / "schemes" / "mod.rs").read_text()


def colours(name):
    found = re.search(rf'palette\("{name}", \[(.*?)\]\)', source, re.S)
    if not found:
        sys.exit(f"Retro has no {name}: has it changed? (update scripts/new-theme.sh)")
    return found.group(1).strip()


glance = re.search(r'(pub static GLANCE: Palette = palette\("GLANCE", \[.*?\]\)\n\.halftone\([^;]*;)', source, re.S)
if not glance:
    sys.exit("Retro has no GLANCE: has it changed? (update scripts/new-theme.sh)")
lists, placeholders, names = "", "", []
for side, label in (("night", "darkness"), ("day", "daylight")):
    static, name, colours_of = statics[side]
    constant = side.upper()
    if side in shown:
        names.append(name)
        lists += f"/// The schemes for {label}, in the order THEME steps through them: the\n/// first is what a watch with nothing saved shows.\npub const {constant}: [&{kind}; 1] = [&{static}];\n\n"
        placeholders += f"""/// A placeholder: Retro's {colours_of}'s colours.
#[rustfmt::skip]
pub static {static}: {kind} = {kind} {{ palette: palette("{name}", [
    {colours(colours_of)}
]) }};

"""
    else:
        other = "night" if side == "day" else "day"
        lists += f"/// None for {label}: the {other}'s show then too (see\n/// `themes::schemes`), and FORCE can't change (README FAQ D02).\npub const {constant}: [&{kind}; 0] = [];\n\n"
(schemes / "mod.rs").write_text(
    "\n".join(
        "//! " + line if line else "//!"
        for paragraph in [
            f"{display}'s colour schemes: the [`{kind}`] type, its schemes "
            "(statics of it, each with a palette of its own), and which side "
            "of the day each is for.",
            f"Pure data, needing only `crate::palette`: `host-tests/{ident}` "
            "includes this file to check the schemes' colours. The drawing is "
            "`super`'s.",
            f"TODO: {' and '.join(names)} "
            + (
                "are placeholders, Retro's colours under names of their own: replace them"
                if len(names) > 1
                else "is a placeholder, Retro's colours under a name of its own: replace it"
            )
            + " with the theme's own, each with a name that says which it is on "
            "its own, as that's what THEME shows and what's saved (see "
            "AUTHORING-THEMES.md).",
        ]
        for line in textwrap.wrap(paragraph, 72, break_long_words=False) + [""]
    ).rstrip("/!\n")
    + f"""

use crate::palette::{{Color, Palette, palette}};

/// {display}: for now only a palette, every scheme drawn alike.
/// `repr(transparent)`, so a scheme's static is its palette, where
/// `scripts/screenshot.py --theme` looks for it: with fields of its own
/// too, `repr(C)`, the palette first.
#[repr(transparent)]
pub struct {kind} {{
    /// Its colours; every scheme's dim look is [`GLANCE`].
    pub(super) palette: Palette,
}}

{lists}// Statics, not consts: the canvas detects palette changes by address.

{placeholders}/// The dim look, for every scheme: Retro's, black, white and red, with
/// its grey (light sleep, the track) white half-toned with black. Only
/// idle mode's eight colours (see `Theme::dim`), which the assertion
/// below checks as it's built.
#[rustfmt::skip]
{glance.group(1)}

const _: () = assert!(GLANCE.idle_mode_safe(), "GLANCE: idle mode's colours only");
"""
)

# ==============================================================================
# Its README and screenshots
# ==============================================================================

night_name, day_name = statics["night"][1], statics["day"][1]
readme = (retro / "README.md").read_text()
every = readme[readme.index("## Every screen") : readme.index("## Credits")]
every = every.replace("ARCADE", night_name).replace("HANDHELD", day_name)
if sides != "both":
    # The one side's column of the table of every screen, with switching
    # off (which Retro shoots by day) on that side.
    keep, drop = (1, 2) if sides == "night" else (2, 1)
    rows = []
    for row in every.split("\n"):
        cells = [cell.strip() for cell in row.strip().strip("|").split("|")]
        if row.startswith("| ") and len(cells) == 3:
            cell = cells[keep] or cells[drop].replace("day-off", f"{sides}-off").replace(", by day,", ", at night,").replace(day_name, night_name)
            row = f"| {cells[0]} | {cell} |"
        elif row == "|---|---|---|":
            row = "|---|---|"
        rows.append(row)
    every = "\n".join(rows)
schemes_table = " | ".join(names)
clocks = " | ".join(
    f'<img src="shots/clock-{dashed}-{side}.png" width="120" alt="The clock face in {statics[side][1]}">' for side in shown
)
by_side = {
    "both": f"{night_name} by night and {day_name} by day are placeholders.",
    "night": f"{night_name}, by night, and by day too (it has no day schemes), is a placeholder.",
    "day": f"{day_name}, by day, and by night too (it has no night schemes), is a placeholder.",
}[sides]
(theme / "README.md").write_text(
    f"""# {display}

TODO: what it looks like, and what it's after. Started as a copy of
Retro, so for now it's Retro's look, in placeholder colours. The
`theme-{ident}` feature.

## Colour schemes

{textwrap.fill("TODO: " + by_side, 72, break_long_words=False)}

| {schemes_table} |
|{'---|' * len(names)}
| {clocks} |

{every}## Credits

- Inspired by: TODO.
- Contributed by: TODO.
- Maintained by: TODO.
"""
)
shots = (retro / "shots.txt").read_text()
day_at = shots.index("# ==============================================================================\n# By day")
off_at = shots.index("# Switching off, last:")
midday_at = shots.index("# Midday, with the day's readings so far.")
if sides != "both":
    # One side's: Retro's night half (every screen, and the dim look),
    # for that side, then switching off.
    shots = shots[:midday_at] + shots[off_at:]
    shots = shots.replace("shot day-off.png", f"shot {sides}-off.png")
    if sides == "day":
        shots = shots.replace("At night, in ARCADE", "By day, in HANDHELD")
        shots = re.sub(r"^shot night-", "shot day-", shots, flags=re.M)
        shots = shots.replace("\ntheme arcade\n", "\ntheme handheld\n")
        shots = shots.replace("shot clock-arcade.png", "shot clock-handheld.png")
    header = "every screen, at night (in ARCADE)\n# and by day (in HANDHELD), the clock face in each scheme, and the dim\n# look."
    if header not in shots:
        sys.exit("Retro's shots.txt has another header: has it changed? (update scripts/new-theme.sh)")
    scheme = "HANDHELD" if sides == "day" else "ARCADE"
    shots = shots.replace(header, f"every screen (in\n# {scheme}), its clock face, and the dim look.")
shots = shots.replace("Retro's screenshots", f"{display}'s screenshots")
shots = shots.replace("src/themes/retro/", f"src/themes/{ident}/")
shots = re.sub(r"^shot clock-(terminal|parchment)\.png .*\n", "", shots, flags=re.M)
shots = shots.replace("shot clock-arcade.png", f"shot clock-{dashed}-night.png")
shots = shots.replace("shot clock-handheld.png", f"shot clock-{dashed}-day.png")
shots = shots.replace("\ntheme arcade\n", f'\ntheme "{night_name}"\n')
shots = shots.replace("\ntheme handheld\n", f'\ntheme "{day_name}"\n')
shots = shots.replace("ARCADE", night_name).replace("HANDHELD", day_name)
(theme / "shots.txt").write_text(shots)

# ==============================================================================
# Listing it
# ==============================================================================

edit(
    Path("src/themes/mod.rs"),
    ("\n}\n\nimpl Theme for Scheme {", f'\n    {ident}: {kind} if "theme-{ident}";\n}}\n\nimpl Theme for Scheme {{'),
    ('#[cfg(feature = "theme-retro")]\npub mod retro;\n', f'#[cfg(feature = "theme-retro")]\npub mod retro;\n#[cfg(feature = "theme-{ident}")]\npub mod {ident};\n'),
)
for manifest in [Path("Cargo.toml"), Path("sim/Cargo.toml")]:
    text = manifest.read_text()
    text, found = re.subn(r"^(themes-all = \[)(.*)\]$", rf'theme-{ident} = []\n\1\2, "theme-{ident}"]', text, count=1, flags=re.M)
    if not found:
        sys.exit(f"{manifest} has no `themes-all`")
    manifest.write_text(text)
index = Path("src/themes/README.md")
listed = {
    "both": f"{night_name} by night, {day_name} by day",
    "night": f"{night_name} by night (and by day: none for the day)",
    "day": f"{day_name} by day (and by night: none for the night)",
}[sides]
index.write_text(index.read_text().rstrip("\n") + f"\n| [{display}]({ident}/) | `theme-{ident}` | {listed} |\n")

# ==============================================================================
# Its colour checks
# ==============================================================================

tests = Path("host-tests") / ident
shutil.copytree("host-tests/theme", tests, ignore=shutil.ignore_patterns("target"))
main = tests / "src" / "main.rs"
text = main.read_text()
# ARCADE's own collision, which the placeholders' colours share but the
# theme's won't.
text = re.sub(r"\n    /// The measure itself: the colours Last night first drew.*?\n    }\n", "\n", text, count=1, flags=re.S)
main.write_text(rename(text))
for path in [tests / "Cargo.toml", tests / "Cargo.lock"]:
    edit(path, ('name = "theme-test"', f'name = "{dashed}-theme-test"'))

check = Path(".github/workflows/check.yml")
edit(check, ("host-tests/theme, ", f"host-tests/theme, host-tests/{ident}, "))
test = Path(".github/workflows/test.yml")
text, found = re.subn(r"^(\s+crate: \[.*\btheme), ", rf"\1, {ident}, ", test.read_text(), count=1, flags=re.M)
if not found:
    sys.exit(f"{test}: no host tests' matrix with theme in it")
test.write_text(text)

print(f"src/themes/{ident}/ and host-tests/{ident}/ written, and {display} listed.")
EOF

# As CI's fmt check wants it (the `pub mod` lines sorted, say).
cargo fmt
(cd "host-tests/$ident" && cargo fmt)

cat >&2 <<EOF
Next, from sim/, take its screenshots, and look at them:

    UPDATE_SHOTS=1 cargo test --release --no-default-features --features themes-all

then build it alone, as its package is:

    cargo clippy --release --no-default-features --features theme-$ident -- -D warnings

and make it its own (AUTHORING-THEMES.md).
EOF
