//! What Gadgetbridge does to text before it sends it to a Bangle.js:
//! cropping, emoji to ASCII, and "Text as Bitmaps".
//!
//! Java's strings are UTF-16 and this code indexes them by unit, so the
//! work here is on `u16`s too: a crop can cut an emoji in half, as theirs
//! does.

/// `cropToLength` (`BangleJSDeviceSupport.java:1461-1465`): at most `len`
/// UTF-16 units, the last three of them dots if any were cut. A `null` is
/// an empty string, so cropped fields are always sent.
pub fn crop_to_length(text: Option<&str>, len: usize) -> String {
    let Some(text) = text else {
        return String::new();
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() <= len {
        return text.into();
    }
    // Lossy: half an emoji left at the cut becomes U+FFFD here, where Java
    // keeps the lone surrogate. Both go out as one `\uXXXX`.
    String::from_utf16_lossy(&units[..len - 3]) + "..."
}

/// `EmojiConverter.simpleEmojiMapping` (`util/EmojiConverter.java:40-79`).
const SIMPLE_EMOJI: &[(&str, &str)] = &[
    ("\u{1F600}", ":-D"),
    ("\u{1F601}", ":-D"),
    ("\u{1F602}", ":'D"),
    ("\u{1F603}", ":-D"),
    ("\u{1F604}", ":-D"),
    ("\u{1F605}", ":'D"),
    ("\u{1F606}", "X-D"),
    ("\u{1F607}", "O:-)"),
    ("\u{1F609}", ";-)"),
    ("\u{1F60A}", ":-)"),
    ("\u{1F60B}", ":-p"),
    ("\u{1F60E}", "B-)"),
    ("\u{1F615}", ":-/"),
    ("\u{1F616}", ":-S"),
    ("\u{1F617}", ":*"),
    ("\u{1F618}", ";-*"),
    ("\u{1F619}", ":-*"),
    ("\u{1F61A}", ":-*"),
    ("\u{1F61B}", ":-P"),
    ("\u{1F61C}", ";-P"),
    ("\u{1F61D}", "X-P"),
    ("\u{1F61E}", ":-S"),
    ("\u{1F620}", ":-@"),
    ("\u{1F621}", ":-@"),
    ("\u{1F622}", ":'("),
    ("\u{1F623}", ":-("),
    ("\u{1F625}", ":'("),
    ("\u{1F62D}", ":'("),
    ("\u{1F62E}", ":-O"),
    ("\u{1F632}", "X-o"),
    ("\u{1F642}", ":)"),
    ("\u{1F643}", "(-:"),
    ("\u{2639}", ":-("),
    ("\u{2764}", "<3"),
    ("\u{27A1}", "->"),
    ("\u{2B05}", "<-"),
    ("\u{1F448}", "<-@"),
    ("\u{1F449}", "@->"),
];

/// A few of the aliases the emoji library Gadgetbridge uses
/// (`com.github.wax911.android-emojify` 1.9.7) gives: `convertAdvancedEmojiToAscii`
/// (`util/EmojiConverter.java:119-126`) writes every emoji the library
/// knows as `:alias:`.
///
/// TODO: only these few. The library's table is thousands of entries and
/// not in Gadgetbridge's tree; any other emoji passes through here as it
/// is, which is also what Gadgetbridge does with one its library doesn't
/// know (and with everything, should the library throw). So both shapes
/// reach the watch from this mock, but not for the same emoji as from a
/// phone.
const EMOJI_ALIASES: &[(&str, &str)] = &[
    ("\u{1F44D}", ":thumbsup:"),
    ("\u{1F3E1}", ":house_with_garden:"),
    ("\u{1F389}", ":tada:"),
    ("\u{1F525}", ":fire:"),
    ("\u{2705}", ":white_check_mark:"),
    ("\u{1F64F}", ":pray:"),
];

/// `EmojiConverter.convertUnicodeEmojiToAscii`
/// (`util/EmojiConverter.java:128-134`): what a Bangle.js gets in place of
/// emoji unless "Text as Bitmaps" is on.
pub fn emoji_to_ascii(text: &str) -> String {
    let mut text = text.to_owned();
    for (emoji, ascii) in SIMPLE_EMOJI.iter().chain(EMOJI_ALIASES) {
        text = text.replace(emoji, ascii);
    }
    text
}

/// `isCharCodeEmoji` (`BangleJSDeviceSupport.java:1369-1379`): by UTF-16
/// unit, so anything beyond U+FFFF counts, by its surrogates.
fn is_char_code_emoji(ch: u16) -> bool {
    matches!(ch,
        0x2190..=0x21FF | 0x2600..=0x26FF | 0x2700..=0x27BF | 0x3000..=0x303F
        | 0xD800..=0xDFFF | 0xFE00..=0xFE0F)
}

/// `renderUnicodeAsImage` (`BangleJSDeviceSupport.java:1381-1458`): the
/// text as a Bangle.js is to show it. `…` always becomes three dots; then
/// either emoji become ASCII (`bitmaps` off, the default), or every word
/// with anything beyond U+00FF in it becomes an image in the text.
///
/// The result is one `char` per UTF-16 unit Gadgetbridge would send where
/// images are involved (their bytes are U+0000–U+00FF), so
/// [`crate::json::string_to_wire`] spells it as Gadgetbridge would.
pub fn render_unicode_as_image(text: Option<&str>, bitmaps: bool) -> Option<String> {
    let text = text?.replace('\u{2026}', "...");
    if !bitmaps {
        return Some(emoji_to_ascii(&text));
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut word: Vec<u16> = Vec::new();
    let mut result: Vec<u16> = Vec::new();
    let mut needs_translate = false;
    let mut word_is_all_emoji = true;
    for mut ch in units {
        // `:1400-1407`: what the Bangle.js's own font has a look-alike for.
        ch = match ch {
            0x2013 | 0x2010 | 0x2014 => u16::from(b'-'),
            0x201A | 0xFF0C | 0x3001 => u16::from(b','),
            0x3002 => u16::from(b'.'),
            0x3010 => u16::from(b'['),
            0x3011 => u16::from(b']'),
            0x2018 | 0x2019 | 0x201B | 0x2032 | 0x02B9 => u16::from(b'\''),
            0x201C | 0x201D | 0x201E | 0x201F | 0x2033 => u16::from(b'"'),
            0xFEFF => continue,
            ch => ch,
        };
        if is_char_code_emoji(ch) {
            if !word_is_all_emoji {
                if needs_translate {
                    result.extend(render_word(&word));
                } else {
                    result.extend(&word);
                }
                word.clear();
                word_is_all_emoji = true;
            }
            needs_translate = true;
        } else {
            if !word.is_empty() && word_is_all_emoji {
                result.extend(render_word(&word));
                word.clear();
                needs_translate = false;
            }
            word_is_all_emoji = ch >= 0xFE00;
        }
        if ch < 128 && b" -_/:.,?!'\"&*()[]".contains(&(ch as u8)) {
            if needs_translate {
                result.extend(render_word(&word));
            } else {
                result.extend(&word);
            }
            result.push(ch);
            word.clear();
            needs_translate = false;
            word_is_all_emoji = true;
        } else {
            if ch > 255 {
                needs_translate = true;
            }
            word.push(ch);
        }
    }
    if needs_translate {
        result.extend(render_word(&word));
    } else {
        result.extend(&word);
    }
    Some(String::from_utf16_lossy(&result))
}

/// `renderUnicodeWordAsImage` (`BangleJSDeviceSupport.java:1343-1364`): a
/// word with CJK ideographs in it goes out two units to an image.
fn render_word(word: &[u16]) -> Vec<u16> {
    if word.iter().any(|ch| (0x4E00..=0x9FFF).contains(ch)) {
        word.chunks(2).flat_map(render_word_part).collect()
    } else {
        render_word_part(word)
    }
}

/// `renderUnicodeWordPartAsImage` (`BangleJSDeviceSupport.java:1334-1341`):
/// a NUL, then the word drawn as an Espruino image
/// (`bitmapToEspruinoArray`, `:2237-2328`): width, height, bits per pixel
/// plus 128 for "has a transparent colour", that colour, and the pixels
/// packed, most significant bit first.
///
/// The pixels are Android's text rendering, which this can't do; what
/// matters to whoever reads the text is the image's shape, so they are
/// noise here, seeded by the word, at a size like a phone's: 21 pixels
/// high (an 18 px font, `:2178`) and 10 wide a unit, cropped at 255
/// (`:2240-2247`). One bit a pixel, black transparent
/// (`MONOCHROME_TRANSPARENT`); three bits with emoji in it
/// (`RGB_3BPP_TRANSPARENT`, taken as always having something
/// transparent).
fn render_word_part(word: &[u16]) -> Vec<u16> {
    let has_emoji = word.iter().any(|&ch| is_char_code_emoji(ch));
    let width = (word.len() * 10).clamp(1, 255);
    let height = 21;
    let bpp = if has_emoji { 3 } else { 1 };
    let mut seed = word.iter().fold(0x9E37_79B9u32, |seed, &ch| {
        seed.rotate_left(5) ^ u32::from(ch)
    });
    let mut out = vec![0, width as u16, height as u16, bpp as u16 + 128];
    // The transparent colour: black, or with emoji the least used one.
    out.push(if has_emoji { (seed % 8) as u16 } else { 0 });
    for _ in 0..(width * height * bpp).div_ceil(8) {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push((seed >> 24) as u16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_count_utf16_units() {
        assert_eq!(crop_to_length(None, 40), "");
        assert_eq!(crop_to_length(Some("Kari"), 40), "Kari");
        assert_eq!(crop_to_length(Some("abcdefgh"), 6), "abc...");
        assert_eq!(crop_to_length(Some("abcdef"), 6), "abcdef");
        // An emoji is two units.
        assert_eq!(crop_to_length(Some("ab😀cd"), 5), "ab...");
    }

    #[test]
    fn emoji_become_ascii_by_default() {
        let render = |text| render_unicode_as_image(Some(text), false).unwrap();
        assert_eq!(
            render("Familien 🏡: Nils"),
            "Familien :house_with_garden:: Nils"
        );
        assert_eq!(render("Takk ❤ 😀"), "Takk <3 :-D");
        assert_eq!(render("Vent…"), "Vent...");
        // Other text beyond Latin-1 is left alone.
        assert_eq!(render("“Привет” – 中文"), "“Привет” – 中文");
        assert_eq!(render_unicode_as_image(None, false), None);
    }

    #[test]
    fn words_beyond_latin1_become_images() {
        let render = |text| render_unicode_as_image(Some(text), true).unwrap();
        assert_eq!(render("Kari på tur"), "Kari på tur");
        assert_eq!(render("“Don’t” – ok…"), "\"Don't\" - ok...");
        let image = render("Привет, Kari");
        let units: Vec<u32> = image.chars().map(u32::from).collect();
        // NUL, 60 x 21, one bit a pixel with black transparent, 158 bytes.
        assert_eq!(units[..5], [0, 60, 21, 129, 0]);
        assert_eq!(units.len(), 5 + (60usize * 21).div_ceil(8) + ", Kari".len());
        assert!(image.ends_with(", Kari"));
        assert!(units.iter().all(|&u| u < 256));
        // CJK: two characters to an image.
        let image = render("中文字");
        assert!(image.chars().filter(|&c| c == '\0').count() >= 2);
    }
}
