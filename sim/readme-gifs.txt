# The README's animated GIFs (docs/screenshots/*.gif), made in the
# simulator, all with one command:
#
#     cd sim
#     cargo run --release -- --out ../docs/screenshots readme-gifs.txt
#
# Rerun it when a screen they show changes. Like readme-screenshots.txt,
# everything is driven through the watch's own inputs, and every name is
# fictional (the dev kit's test notifications, src/debug.rs). Each
# recording starts once its scene is set, and ends after a pause, as a
# GIF loops. See sim/README.md for how recordings time their frames:
# a still screen is held 2 s at most, unless `--still` says longer (as
# for the screens below that are meant to be read).

# ==============================================================================
# A Sunday evening, on the wrist
# ==============================================================================
#
# As readme-screenshots.txt sets it up: after sunset in Oslo (the Arcade
# theme), the day so far, last night's sleep, a phone with music playing,
# and a couple of notifications from earlier.

time 2026-11-08 17:38:20 +01:00
location 59.91 10.75
temperature 31
heart 72
demo-day
last-night 23:20 06:55
music "Ida Moreau" "Night Signals" playing
theme arcade
notify 4 26h
wait 1.5s
notify 3 3h
wait 15s
expect not screen-on

# ==============================================================================
# swipe.gif: around the clock face
# ==============================================================================
#
# The button wakes the screen; then right to today and last night, back,
# left to the alarms, back, and down to the notifications.

button
wait 0.5s
record swipe.gif
wait 1s
right
wait 1.5s
right
wait 1.5s
left
wait 0.6s
left
wait 0.6s
left
wait 1.5s
right
wait 0.6s
down
wait 2s
stop
button
wait 15s
expect not screen-on

# ==============================================================================
# notification.gif: one arriving with the screen off
# ==============================================================================
#
# Full-screen and dim, as the screen was off; a firm tap then wakes the
# screen fully, to the clock face, whose top edge says there's something
# unseen.

record notification.gif --still 3s
wait 0.8s
notify 0
wait 3s
tap
wait 2s
stop
wait 15s

# ==============================================================================
# glance.gif: a turn of the wrist
# ==============================================================================
#
# The time, dimly, for a few seconds, and dark again.

until 17:45
record glance.gif --still 6s
wait 0.8s
glance
wait 6.5s
stop

# ==============================================================================
# alarm.gif: the morning's alarm, snoozed
# ==============================================================================
#
# Overnight, an alarm at 07:30. It rings with the screen dim, brightening
# (which the screen's colours alone don't show); a tap snoozes it.

alarm 1 07:30 on
until 07:29
wait 58s
record alarm.gif --still 4s
until 07:30
expect buzzing
wait 3s
tap
expect not buzzing
wait 2.5s
stop
