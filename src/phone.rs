//! What the phone tells the UI: notifications, calls, music, alarms,
//! directions, and trouble with the link.
//!
//! [`crate::gb`] decodes these from Gadgetbridge's messages, [`crate::ble`]
//! queues them for the UI loop, and [`crate::ui`] acts on them. They live
//! here, rather than in `ui.rs` where they're used, so that the protocol
//! code builds without the UI (and its canvas, fonts, and motor): in its
//! host tests (`host-tests/gb`), and in the simulator's replay of raw
//! Gadgetbridge lines. Hardware-free.

use heapless::String;

use crate::{alarm, route};

/// Things the phone tells the UI, via [`crate::ble`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PhoneEvent {
    /// The app it came from, and who it's from: Gadgetbridge's `sender`,
    /// or without one, the title (see `gb`). Never its body or subject.
    Notification {
        app: Text<24>,
        who: Text<40>,
    },
    IncomingCall {
        caller: String<40>,
    },
    /// The call was answered, rejected, or ended, on either side.
    CallOver,
    MusicInfo {
        artist: String<40>,
        track: String<40>,
    },
    MusicState {
        playing: bool,
    },
    /// "Find my watch" from the phone: ring until told to stop.
    Find {
        on: bool,
    },
    /// A client connected but never secured the link (see
    /// `ble::not_paired`). Shown like a notification.
    Unpaired(Unpaired),
    /// The phone sent text with Gadgetbridge's "Text as Bitmaps" images in
    /// it (see `gb::Images`), which we can't show: tell the wearer how to
    /// turn it off. Shown like a notification, once a connection.
    TextAsBitmaps,
    /// The alarms set in Gadgetbridge, all of them.
    Alarms(alarm::Phone),
    /// The next turn while the phone navigates; `None` once it stops.
    Route(Option<route::Step>),
    /// Gadgetbridge didn't pass a button grid press on, as its "Allow
    /// Intents" setting is off. Shown like a notification.
    IntentsOff,
    /// A firmware update arrived, but the watch refused to install it (see
    /// `crate::mcuboot`), and keeps the firmware it has. Not from the phone
    /// as such, but shown the same way: like a notification.
    UpdateRefused,
}

/// Why a connected client can't use the watch: its link never became
/// encrypted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, defmt::Format)]
pub enum Unpaired {
    /// Not a device we have a bond with: it has to pair.
    NotPaired,
    /// A device we have a bond with, which didn't use it: the phone forgot
    /// its side of the bond, and has to forget the watch and pair again.
    PairingLost,
}

impl Unpaired {
    pub fn title(self) -> &'static str {
        match self {
            Self::NotPaired => "NOT PAIRED",
            Self::PairingLost => "PAIRING LOST",
        }
    }

    /// What to do about it: README.md's FAQ entry, by its ID only, as the
    /// steps don't fit on the screen.
    pub fn hint(self) -> &'static str {
        match self {
            Self::NotPaired => "SEE FAQ C01",
            Self::PairingLost => "SEE FAQ C02",
        }
    }
}

/// Logged without contents: names and titles stay out of the logs.
impl defmt::Format for PhoneEvent {
    fn format(&self, f: defmt::Formatter) {
        let kind = match self {
            Self::Notification { .. } => "Notification",
            Self::IncomingCall { .. } => "IncomingCall",
            Self::CallOver => "CallOver",
            Self::MusicInfo { .. } => "MusicInfo",
            Self::MusicState { .. } => "MusicState",
            Self::Find { .. } => "Find",
            Self::Unpaired(_) => "Unpaired",
            Self::TextAsBitmaps => "TextAsBitmaps",
            Self::Alarms(_) => "Alarms",
            Self::Route(_) => "Route",
            Self::IntentsOff => "IntentsOff",
            Self::UpdateRefused => "UpdateRefused",
        };
        defmt::write!(f, "{=str}", kind)
    }
}

// ==============================================================================
// A notification's text, as shown
// ==============================================================================

/// What [`Text`] keeps for an ellipsis: U+0085, a control character in
/// Latin-1 (never kept as itself: [`TextBuilder`] makes those spaces), and
/// the ellipsis in Windows-1252, Latin-1's best-known extension.
const ELLIPSIS: u8 = 0x85;

/// A notification's app or title, as the watch keeps it to show: Latin-1,
/// a byte a character, as the font draws nothing beyond Latin-1 anyway.
/// So `N` bytes hold `N` characters, where UTF-8 held as few as `N / 2`
/// of æ, ø and å (`ÅSE BERGERSEN` lost its last letters to that). Unused
/// bytes are 0, which is never a character here, so it needs no length.
/// Made by a [`TextBuilder`], which drops emoji, collapses spaces, and ends
/// text that didn't fit with an ellipsis.
#[derive(Clone, PartialEq, Eq)]
pub struct Text<const N: usize>([u8; N]);

impl<const N: usize> Text<N> {
    pub const fn new() -> Self {
        Self([0; N])
    }

    pub fn is_empty(&self) -> bool {
        self.0[0] == 0
    }

    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.0.iter().take_while(|&&b| b != 0).map(|&b| match b {
            ELLIPSIS => '\u{2026}',
            b => char::from(b),
        })
    }

    /// The text as UTF-8, for drawing, in `out`, which holds it all if
    /// it has room for `2 * N + 1` bytes (an ellipsis takes 3).
    pub fn write_to<const M: usize>(&self, out: &mut String<M>) {
        out.clear();
        for c in self.chars() {
            if out.push(c).is_err() {
                break;
            }
        }
    }
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> From<&str> for Text<N> {
    fn from(text: &str) -> Self {
        let mut builder = TextBuilder::new();
        text.chars().for_each(|c| builder.push(c));
        builder.finish()
    }
}

impl<const N: usize> core::fmt::Display for Text<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.chars()
            .try_for_each(|c| core::fmt::Write::write_char(f, c))
    }
}

impl<const N: usize> core::fmt::Debug for Text<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "\"{self}\"")
    }
}

/// Builds a [`Text`], a character at a time, as the text is decoded.
///
/// - **Emoji and other pictographs are left out**, not drawn as the `?`
///   the font has for what it lacks: `?` reads as an error, or a
///   question. The spaces around one collapse into one, and none is left
///   before the punctuation that followed it, so `Familien 🏡: Nils`
///   becomes `Familien: Nils`, `🎉 Bursdag` becomes `Bursdag`, and a name
///   that was only emoji becomes empty.
/// - **Invisible characters are ignored:** joiners and variation
///   selectors (inside emoji), and the direction marks phones put around
///   names and phone numbers.
/// - **Typographic punctuation** (curly quotes, dashes, the minus sign) is
///   kept as the ASCII the font draws it as, and the ellipsis as itself.
///   Anything else beyond Latin-1 (Cyrillic, Greek, CJK, …) is `?`: a
///   letter is missing there, and the `?` says so.
/// - **Whitespace** (line breaks, tabs, no-break spaces, runs of spaces)
///   becomes one space, and none is kept at either end.
/// - **Text too long to keep** ends in an ellipsis, in place of its last
///   character (and any space before it), so a cut name never passes for
///   a whole one: `ÅSE BERGER` was a different, plausible surname.
pub struct TextBuilder<const N: usize> {
    text: Text<N>,
    len: usize,
    /// What's between the last character kept and the next.
    gap: Gap,
    full: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gap {
    None,
    Space,
    /// A pictograph, with or without spaces around it: a space, unless
    /// what follows is punctuation that closes what came before.
    Dropped,
}

impl<const N: usize> TextBuilder<N> {
    pub const fn new() -> Self {
        Self {
            text: Text::new(),
            len: 0,
            gap: Gap::None,
            full: false,
        }
    }

    pub fn push(&mut self, c: char) {
        if invisible(c) {
            return;
        }
        if c.is_whitespace() || c.is_control() {
            if self.gap == Gap::None {
                self.gap = Gap::Space;
            }
            return;
        }
        if pictograph(c) {
            self.gap = Gap::Dropped;
            return;
        }
        let closes = matches!(c, ':' | ',' | '.' | ';' | '!' | '?' | ')' | ']' | '}');
        let space = match self.gap {
            Gap::None => false,
            Gap::Space => true,
            Gap::Dropped => !closes,
        };
        self.gap = Gap::None;
        // Only between characters: none at the start.
        if space && self.len > 0 {
            self.keep(b' ');
        }
        self.keep(latin1(c));
    }

    fn keep(&mut self, byte: u8) {
        if self.full {
            return;
        }
        if self.len < N {
            self.text.0[self.len] = byte;
            self.len += 1;
            return;
        }
        // Too long: an ellipsis in place of the last character, and of
        // any space before it.
        self.full = true;
        self.len = N - 1;
        while self.len > 0 && self.text.0[self.len - 1] == b' ' {
            self.len -= 1;
        }
        self.text.0[self.len] = ELLIPSIS;
        self.text.0[self.len + 1..].fill(0);
    }

    /// Whether nothing has been kept: nothing came, or only whitespace,
    /// invisible characters, or emoji, which all show nothing.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn finish(self) -> Text<N> {
        // What `Text` relies on to need no length, and what the builder
        // promises of what's shown: no 0 before the text's end (and only
        // 0s after), no space at either end, nor two in a row, and no
        // control character but the one that stands for an ellipsis.
        debug_assert!(
            {
                let text = &self.text.0;
                let end = text.iter().position(|&b| b == 0).unwrap_or(N);
                let shown = &text[..end];
                text[end..].iter().all(|&b| b == 0)
                    && shown.first() != Some(&b' ')
                    && shown.last() != Some(&b' ')
                    && !shown.windows(2).any(|pair| pair == b"  ")
                    && shown
                        .iter()
                        .all(|&b| b == ELLIPSIS || !char::from(b).is_control())
                    && (end == 0) == self.is_empty()
            },
            "text as `Text` keeps it"
        );
        self.text
    }
}

impl<const N: usize> Default for TextBuilder<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// `c` as Latin-1 (see [`TextBuilder`]).
fn latin1(c: char) -> u8 {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201b}' | '\u{2032}' | '\u{2b9}' => b'\'',
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' => b'"',
        '\u{201a}' => b',',
        '\u{2010}'..='\u{2015}' | '\u{2212}' => b'-',
        '\u{2026}' => ELLIPSIS,
        // Control characters were whitespace (see `push`).
        c => u8::try_from(u32::from(c)).unwrap_or(b'?'),
    }
}

/// Characters that show nothing themselves: zero-width spaces and
/// joiners (which glue emoji together), direction marks and isolates
/// (phones put them around names and phone numbers in right-to-left
/// settings), variation selectors (text or emoji style), the keycap
/// mark (`1️⃣`), the byte order mark, and the tags in subdivision flags.
fn invisible(c: char) -> bool {
    matches!(
        u32::from(c),
        0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2069
            | 0x20E3
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xE0000..=0xE007F
    )
}

/// Emoji, and the other pictographs and symbols phones draw as emoji, by
/// the Unicode blocks they're in rather than character by character (the
/// emoji list grows every year; the blocks don't move):
///
/// - Miscellaneous Technical (⌚, ⏰, ⏳, …), Geometric Shapes (▶, ◀,
///   ●, …), Miscellaneous Symbols (☀, ☎, ★, ♥, …), and Dingbats (✅, ✈,
///   ✨, ❤, …): U+2300–23FF and U+25A0–27BF.
/// - Miscellaneous Symbols and Arrows (⭐, ⬆, …): U+2B00–2BFF.
/// - The four CJK symbols that are emoji (〰, 〽, ㊗, ㊙).
/// - Everything from Mahjong Tiles to Symbols for Legacy Computing:
///   U+1F000–1FBFF, which holds nearly all emoji (faces, people, animals,
///   food, flags, skin tones, …).
///
/// Not arrows (U+2190–21FF), letterlike symbols (™, ℹ) or enclosed
/// letters (Ⓜ, ①): those are as often text, and stay `?` like any other
/// character the font lacks.
fn pictograph(c: char) -> bool {
    matches!(
        u32::from(c),
        0x2300..=0x23FF
            | 0x25A0..=0x27BF
            | 0x2B00..=0x2BFF
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0x1F000..=0x1FBFF
    )
}
