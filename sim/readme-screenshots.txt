# The README's screenshots (docs/screenshots/), made in the simulator:
#
#     cd sim
#     UR_VERSION=0.9.0+$(git rev-parse --short=7 HEAD) \
#         cargo run --release -- --out ../docs/screenshots readme-screenshots.txt
#
# (UR_VERSION, for the version and build the Info screen shows; see
# build.rs.) They're also approved screenshots (see README.md here):
# `UPDATE_SHOTS=1 cargo test` updates all but info.png, which only this
# makes, as only it has a release's version. Each section sets up what
# its screenshot shows. Every name, track, and street is fictional: the
# same shared senders as the dev kit's test notifications (src/debug.rs).

# ==============================================================================
# Info: first, as it shows the uptime, a few seconds after booting
# ==============================================================================

battery 4040
connect
left
left
left
wait 6.8s
shot info.png

# ==============================================================================
# A Sunday evening, on the wrist
# ==============================================================================
#
# After sunset in Oslo, so the dark themes are in use: the day so far
# (steps and heart rates), last night's sleep, music playing on the
# phone, and three notifications, from yesterday, this afternoon, and
# just now. On the wrist, the watch reads close to the skin's
# temperature.

time 2026-11-08 17:38:20 +01:00
location 59.91 10.75
temperature 31
heart 72
demo-day
last-night 23:20 06:55
music "Ida Moreau" "Night Signals" playing
theme arcade

# With the screen off, each arrives full-screen; the screen sleeps again
# after.
notify 4 26h
wait 1.5s
notify 3 3h
wait 1.5s
notify 0
wait 15s

# ==============================================================================
# The clock face, in each theme
# ==============================================================================
#
# One frame, in each theme's colours (as `screenshot.py --theme`), so the
# four differ only in colour: the note for the music, the temperature,
# and, on the top edge, the handle that says notifications came in that
# haven't been seen.

until 17:42
button
wait 2.5s
expect clock
shot clock-arcade.png
shot clock-terminal.png terminal
shot clock-handheld.png handheld
shot clock-parchment.png parchment

# ==============================================================================
# Around the clock
# ==============================================================================

# Above: the notifications, newest first; the second page, with handles
# both ways.
down
wait 1s
down
wait 1s
shot history.png
button
wait 1s

# Left: today, then last night.
right
wait 1s
shot today.png
right
wait 1s
shot last-night.png
button
wait 1s

# Right: the alarms, smart wake on, in a day theme's colours.
smart on 06:30 08:30
left
wait 1s
shot alarms.png handheld
button
wait 1s

# Right, past the alarms: the settings, turning on the button grid
# (docs/buttons.md). Then below: the grid, just after 2, then 3
# double-tapped.
left
left
wait 1s
tap 40,54
wait 1s
shot settings.png
button
wait 1s
up
wait 1s
tap 40,13
wait 300ms
tap 67,13
wait 150ms
tap 67,13
wait 100ms
shot buttons.png
button
wait 1s

# ==============================================================================
# Directions on a glance
# ==============================================================================

button
wait 1s
nav right "150 m" "Mill Lane toward Harbour Road"
glance
wait 1s
expect dim
shot directions.png
nav off
