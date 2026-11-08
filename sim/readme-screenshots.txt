# The README's screenshots (docs/screenshots/), made in the simulator:
#
#     cd sim
#     UR_VERSION=0.9.0+$(git rev-parse --short=7 HEAD) \
#         cargo run --release -- --out ../docs/screenshots readme-screenshots.txt
#
# (UR_VERSION, for the version and build the Info screen shows; see
# build.rs.) Each section sets up what its screenshot shows, the way the
# dev kit's were taken.

# ==============================================================================
# Info: first, as it shows the uptime, a few seconds after booting
# ==============================================================================

battery 4040
down
wait 6.8s
shot info.png

# ==============================================================================
# The clock face, in each theme
# ==============================================================================
#
# One frame, in each theme's colours (as `screenshot.py --theme`), so the
# four differ only in colour: early afternoon, the phone connected, no
# heart rate yet, no steps, and the watch at 23 °C off the wrist.

time 2026-11-08 13:18:50 +01:00
location 59.91 10.75
connect
temperature 23
theme arcade
until 13:19
button
wait 2.5s
shot clock-arcade.png
shot clock-terminal.png terminal
shot clock-handheld.png handheld
shot clock-parchment.png parchment

# ==============================================================================
# Alarms, by day
# ==============================================================================

left
wait 1s
shot alarms.png handheld
theme auto

# ==============================================================================
# A notification arriving with the screen off
# ==============================================================================

wait 15s
notify 0
wait 1s
shot notification.png

# ==============================================================================
# A glance, late in the evening
# ==============================================================================

wait 10s
until 22:47
glance
wait 1s
shot glance.png
