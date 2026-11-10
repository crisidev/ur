//! Retro's notifications, calls, and music.

use super::BACKGROUND;
use super::layout::{CALL_BUTTONS_Y, CALL_TEXT_WIDTH, MUSIC_CONTROLS_Y, MUSIC_TEXT_WIDTH};
use crate::canvas::{Canvas, Color};
use crate::font;
use crate::themes::helpers::fitted;
use crate::themes::{Call, Music};

// Notifications look like the watch's other screens: their app is the
// title, top left in the accent colour, as TODAY's or ALARMS' is, with
// when it arrived top right in the history, as MUSIC's state is. Then a
// rule, and who it's from, centred below, in a face that makes it the
// most important thing on the screen. The banner is the same in short,
// over the bottom of whatever shows.
//
// Who it's from is the 3×5 at 2× if it fits on one line (up to about 9
// letters), or on two of about the same width (see `large`): a poster,
// the size the watch's other big text is. Anything longer is in the
// medium face (see `font::medium_glyph`), balanced over as few lines as
// it takes (see `font::wrap`), never the small 1×, which is too small to
// read at a glance. Either way it's centred, as a name on a screen of its
// own is expected to be, and as it was before the medium face: the
// balanced lines read as a block, and a short name doesn't sit in a
// corner. (The banner, which has to be compact, is the exception: see
// `render_banner`.)

/// Text columns of the banner and the notification card, as of a call:
/// four columns (12 of the panel's pixels) clear of each edge, for a
/// little more air than the three there were. Four still leave room for
/// 18 letters at 1× and 9 at 2×, as three did; a fifth would take a
/// letter off each.
const BANNER_TEXT: core::ops::Range<i32> = 4..76;
const BANNER_TEXT_WIDTH: i32 = BANNER_TEXT.end - BANNER_TEXT.start;

/// Line pitch of the medium face: 7 px capitals, 2 accent rows, and a
/// row's gap, so an accent never touches the line above.
const MEDIUM_PITCH: i32 = font::MEDIUM_HEIGHT + font::ACCENT_ROWS + 1;

/// The most lines who it's from takes in the card: 40 letters at about 11
/// a line, with room to spare for long words.
const CARD_LINES: usize = 5;

/// How unequal two lines at 2× may be: the shorter at least 7/10 of the
/// longer, in pixels. In this all but monospaced font, a 5- and a 5-letter
/// word are equal, 4 and 5 make 79%, 5 and 7 just make it at 70%, and 4
/// and 6 (65%) or 3 and 8 (35%) don't. Below about this, the pair reads
/// as a long line with a stub hanging off it rather than as a block.
const BALANCE_TENTHS: i32 = 7;

/// Who it's from at 2×, as `lines` of the small font: how many (one or
/// two), or 0 if it's not short enough. Two only split at a space, about
/// equal (see [`BALANCE_TENTHS`]): a first name and a surname, typically.
/// Never hyphenated: a broken word in large type is harder to read than
/// the same word whole in the medium face, and so is a big first line
/// over a one-word second.
fn large(who: &str, lines: &mut [font::Line; CARD_LINES]) -> usize {
    // `wrap` balances the lines already: what's left is to check how it
    // broke them. A break at a space is neither a hyphen nor a joint in a
    // link; an ellipsis at the end means it needed more than two.
    let count = font::wrap(
        who,
        font::Face::Small,
        BANNER_TEXT_WIDTH / 2,
        &mut lines[..2],
    );
    let [top, bottom, ..] = *lines;
    match count {
        1 => 1,
        2 if bottom.tail == font::Tail::None && who.as_bytes().get(top.end) == Some(&b' ') => {
            let (a, b) = (
                top.width(who, font::Face::Small),
                bottom.width(who, font::Face::Small),
            );
            if a.min(b) * 10 >= a.max(b) * BALANCE_TENTHS {
                2
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// A notification, with the whole screen (one that arrived with the screen
/// off, in the dim look, or a page of the history): the app top left in
/// the accent colour, and `time` top right (when it arrived, in the
/// history; nothing on a notification that has just come, as that's now),
/// a rule under them, as on Today, and who it's from below, centred, big
/// if it's short (see [`large`]).
pub(super) fn render_card(canvas: &mut Canvas, app: &str, who: &str, time: &str) {
    canvas.clear(BACKGROUND);
    // The app gives way to the time: 4 px between them, and an ellipsis.
    let time_width = font::small_text_width(time);
    let gap = if time.is_empty() { 0 } else { time_width + 4 };
    fitted(
        canvas,
        (BANNER_TEXT.start, 3),
        app,
        font::Face::Small,
        BANNER_TEXT_WIDTH - gap,
        Color::ACCENT,
    );
    canvas.text_right(BANNER_TEXT.end, 3, time, Color::TEXT_DIM);
    // Dashed, 2 px on and 1 off: Today's chart baselines, in a colour
    // that shows in every look.
    for x in (BANNER_TEXT.start..BANNER_TEXT.end).step_by(3) {
        canvas.fill_rect(x, 11, 2, 1, Color::TEXT_FAINT);
    }

    // At 2× (pitch, capitals' height, and accents' rows, all doubled),
    // or in the medium face.
    let mut lines = [font::Line::default(); CARD_LINES];
    let (count, face, scale, pitch, height) = match large(who, &mut lines) {
        0 => (
            font::wrap(who, font::Face::Medium, BANNER_TEXT_WIDTH, &mut lines),
            font::Face::Medium,
            1,
            MEDIUM_PITCH,
            font::MEDIUM_HEIGHT,
        ),
        count => (
            count,
            font::Face::Small,
            2,
            2 * (5 + font::ACCENT_ROWS + 2),
            2 * 5,
        ),
    };
    // The capitals centred on the screen, which is where the eye expects
    // a name (centring the space below the rule put them 6 px low), but
    // no nearer the rule than two clear rows above the accents. Accents
    // above the first line are left out of the centring, as most names
    // have none.
    let block = pitch * (count as i32 - 1) + height;
    let top = (40 - block / 2).max(12 + 2 + font::ACCENT_ROWS * scale);
    for (i, &line) in lines[..count].iter().enumerate() {
        let y = top + pitch * i as i32;
        // The tail (an ellipsis or a hyphen) is part of the width.
        let x = 40 - line.width(who, face) * scale / 2;
        if scale == 2 {
            canvas.text_scaled(x, y, line.of(who), Color::TEXT, 2);
        } else {
            canvas.line(x, y, who, line, face, Color::TEXT);
        }
    }
}

/// Line pitch of the small font in the banner: 5 px capitals, 2 accent
/// rows, and a row's gap, so an accent never touches the line above.
const SMALL_PITCH: i32 = 5 + font::ACCENT_ROWS + 1;

/// Over the bottom of the screen, as compact as it was before the
/// redesign: an accent rule, the app (or what happened) small in the
/// accent colour, and who it's from (or what to do) small too, on one
/// line or two, ending in an ellipsis if even two won't do. The history
/// has the rest. One line is 17 px, over the clock face's steps, leaving
/// the heart rate and the next alarm in view; two, 25, from just under
/// the date and the seconds, which a 4 s banner can afford; the time
/// always shows.
///
/// Small, not in the medium face that names are given on a screen of
/// their own: its 7 px capitals, their accents, and a row clear of the
/// app above them make 9 rows to the small font's 7, and the 17 px can't
/// spare them without the app crowding the rule or the text the bottom
/// edge. Rows, from the rule: 1 clear, the app's 5, 1 clear (so Å's ring
/// never touches it), 2 for accents, 5 for the capitals, and 2 clear at
/// the bottom.
pub(super) fn render_banner(canvas: &mut Canvas, app: &str, who: &str) {
    let mut lines = [font::Line::default(); 2];
    let count = font::wrap(who, font::Face::Small, BANNER_TEXT_WIDTH, &mut lines).max(1);
    let top = 63 - SMALL_PITCH * (count as i32 - 1);
    canvas.fill_rect(0, top, 80, 80 - top, BACKGROUND);
    canvas.fill_rect(0, top, 80, 1, Color::ACCENT);
    fitted(
        canvas,
        (BANNER_TEXT.start, top + 2),
        app,
        font::Face::Small,
        BANNER_TEXT_WIDTH,
        Color::ACCENT,
    );
    for (i, &line) in lines[..count].iter().enumerate() {
        let y = top + 2 + 5 + 1 + font::ACCENT_ROWS + SMALL_PITCH * i as i32;
        canvas.line(
            BANNER_TEXT.start,
            y,
            who,
            line,
            font::Face::Small,
            Color::TEXT,
        );
    }
}

/// The caller's name scrolls within columns 4..76 (see
/// [`CALL_TEXT_WIDTH`]); the decline and answer buttons are where the UI
/// takes taps on them.
pub(super) fn render_call(canvas: &mut Canvas, call: &Call) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 6, "INCOMING CALL", Color::ACCENT);

    // The caller at 2×, centred if it fits, scrolling if not.
    let width = font::small_text_width(call.caller) * 2;
    let left = if width <= CALL_TEXT_WIDTH {
        40 - width / 2
    } else {
        4
    };
    canvas.marquee(
        left..4 + CALL_TEXT_WIDTH,
        24,
        call.caller,
        Color::TEXT,
        2,
        call.elapsed,
    );

    let hint = if call.silenced {
        "SILENCED"
    } else {
        "BUTTON: SILENCE"
    };
    canvas.text_centered(40, 44, hint, Color::TEXT_FAINT);

    for (x, label, color) in [(2, "DECLINE", Color::BAD), (42, "ANSWER", Color::GOOD)] {
        canvas.fill_rect(x, CALL_BUTTONS_Y, 36, 18, color);
        canvas.text_centered(x + 18, CALL_BUTTONS_Y + 7, label, BACKGROUND);
    }
}

/// Artist and track scroll within columns 3..77 (see [`MUSIC_TEXT_WIDTH`]);
/// the controls are where the UI takes taps on them.
pub(super) fn render_music(canvas: &mut Canvas, music: &Music) {
    let elapsed = music.elapsed;
    canvas.clear(BACKGROUND);
    canvas.text(3, 4, "MUSIC", Color::ACCENT);
    canvas.text_right(
        77,
        4,
        match music.playing {
            Some(true) => "PLAYING",
            Some(false) => "PAUSED",
            None => "",
        },
        Color::TEXT_DIM,
    );

    canvas.marquee(
        3..3 + MUSIC_TEXT_WIDTH,
        17,
        music.artist,
        Color::TEXT_DIM,
        1,
        elapsed,
    );
    canvas.marquee(
        3..3 + MUSIC_TEXT_WIDTH,
        29,
        music.track,
        Color::TEXT,
        2,
        elapsed,
    );

    // Previous, play/pause, next: 5×5 icons at 3×, centred in thirds.
    let play_pause = if music.playing == Some(true) {
        &font::PAUSE
    } else {
        &font::PLAY
    };
    for (x, icon) in [(6, &font::PREVIOUS), (33, play_pause), (60, &font::NEXT)] {
        canvas.sprite(x, MUSIC_CONTROLS_Y + 6, 5, icon, 3, Color::TEXT);
    }
}
