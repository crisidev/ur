# The README's animated GIFs (docs/screenshots/*.gif), made in the
# simulator, all with one command:
#
#     cd sim
#     cargo run --release -- --out ../docs/screenshots readme-gifs.txt
#
# Rerun it when a screen they show changes. Like readme-screenshots.txt,
# everything is driven through the watch's own inputs, and every name is
# fictional (the dev kit's test notifications, src/debug.rs). Each
# recording starts on the moment it's about (its first frame is what a
# still copy of it, or a page still loading it, shows), and any dark
# stretch before that moment comes at its end instead, which the loop
# then plays before it. See sim/README.md for how recordings time their frames:
# a still screen is held 2 s at most, unless `--still` says longer (as
# for the screens below that are meant to be read).

# ==============================================================================
# A Sunday evening, on the wrist
# ==============================================================================
#
# As readme-screenshots.txt sets it up: a phone connected (as a watch
# usually is, so the status bar shows no "no phone" rune), after sunset
# in Oslo (the Arcade theme), the day so far, last night's sleep, music
# playing, and a couple of notifications from earlier.

connect
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
# and down to the notifications. (Each sweep takes ~0.5 s, as on the
# watch, and a GIF's frames: more of them, and swipe.gif outgrows
# tests/record.rs's budget. The alarms, left of the clock, are left out
# for that.)

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
# unseen; then the button puts it out again, to loop back to the arrival.

notify 0
wait 0.1s
record notification.gif --still 3s
wait 2.9s
tap
wait 2s
button
wait 0.8s
stop
expect not screen-on

# ==============================================================================
# glance.gif: a turn of the wrist
# ==============================================================================
#
# The time, dimly, for a few seconds, and dark again.

until 17:45
glance
wait 0.1s
record glance.gif --still 6s
wait 6.5s
expect not screen-on
wait 0.8s
stop

# ==============================================================================
# alarm.gif: the morning's alarm, snoozed
# ==============================================================================
#
# Overnight, an alarm at 07:30. It rings with the screen dim, brightening
# (which the screen's colours alone don't show); a tap snoozes it. (The
# dark minute before it rang isn't recorded: the loop goes straight from
# the snoozed clock back to the ringing.)

alarm 1 07:30 on
until 07:29
wait 58s
until 07:30
expect buzzing
wait 0.1s
record alarm.gif --still 4s
wait 2.9s
tap
expect not buzzing
wait 2.5s
stop
