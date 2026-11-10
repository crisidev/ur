//! Host tests for the firmware's `bthome` module: the button grid's
//! presses as encrypted BTHome advertisements. `cargo test` runs them.
//!
//! `main` prints an advertisement for each gesture, in each layout of the
//! grid, which `check.py`
//! decodes with Home Assistant's own parser (`bthome-ble`), as Home
//! Assistant would:
//!
//! ```sh
//! cargo run --release | uv run --with bthome-ble python -I check.py
//! ```

#[path = "../../../src/bthome.rs"]
#[allow(dead_code)]
mod bthome;
#[path = "../../../src/buttons.rs"]
#[allow(dead_code)]
mod buttons;

use bthome::Identity;
use buttons::{Gesture, Layout, Press};

/// bthome.io's example key.
const KEY: [u8; 16] = [
    0x23, 0x1d, 0x39, 0xc1, 0xd7, 0xcc, 0x1a, 0xb1, 0xae, 0xe2, 0x24, 0xcd, 0x09, 0x6d, 0xb9, 0x32,
];

fn identity(display: &str) -> Identity {
    let mut address = [0u8; 6];
    // On the air, least significant byte first.
    for (i, byte) in display.split(':').rev().enumerate() {
        address[i] = u8::from_str_radix(byte, 16).expect("hex");
    }
    Identity { key: KEY, address }
}

fn main() {
    let address = std::env::args()
        .nth(1)
        .unwrap_or("E5:12:34:56:78:9A".into());
    let identity = identity(&address);
    println!("{address} {}", hex(&KEY));
    // Each line: the layout, the code, the gesture, and the advertisement.
    let two = [
        (23, Gesture::Tap),
        (23, Gesture::DoubleTap),
        (5, Gesture::Tap),
        (5, Gesture::Hold),
        (5, Gesture::LongPress),
        (5, Gesture::Release),
        (47, Gesture::Hold),
        (47, Gesture::LongPress),
        (47, Gesture::Release),
    ]
    .map(|(code, gesture)| (Layout::TwoLayers, code, gesture));
    // Every button, with every gesture.
    let six = (1..=6).flat_map(|code| {
        [
            Gesture::Tap,
            Gesture::DoubleTap,
            Gesture::Hold,
            Gesture::LongPress,
            Gesture::Release,
        ]
        .map(|gesture| (Layout::Six, code, gesture))
    });
    // The battery on its own first, as six buttons send it soon after
    // they're turned on: 82%. Then the presses, and 77%, with the counter
    // only ever going up, as on the watch, so one parser can take them all
    // in order, as Home Assistant's does.
    let (data, len) = bthome::advert(None, Layout::Six, 82, 255, &identity);
    println!("battery 82 - {}", hex(&data[..len]));
    for (counter, (layout, code, gesture)) in (256..).zip(two.into_iter().chain(six)) {
        let (data, len) = bthome::advert(
            Some(Press { code, gesture }),
            layout,
            82,
            counter,
            &identity,
        );
        let layout = match layout {
            Layout::Six => "six",
            Layout::TwoLayers => "two",
        };
        println!("{layout} {code} {} {}", gesture.name(), hex(&data[..len]));
    }
    let (data, len) = bthome::advert(None, Layout::Six, 77, 400, &identity);
    println!("battery 77 - {}", hex(&data[..len]));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn sealing_matches_bthomes_example() {
        // https://bthome.io/encryption/: MAC 54:48:E6:8F:80:A5, info 0x41,
        // counter 0x00112233, temperature and humidity.
        let nonce: [u8; 13] = hex("5448e68f80a5d2fc4133221100").try_into().expect("13");
        let mut data = hex("02ca0903bf13");
        let mic = bthome::seal_for_test(&KEY, &nonce, &mut data);
        assert_eq!(data, hex("e445f3c9962b"));
        assert_eq!(mic.to_vec(), hex("6c7c4519"));
    }

    /// The advertisement's parts, with the objects decrypted.
    fn open(data: &[u8], identity: &Identity) -> (Vec<u8>, u32) {
        assert_eq!(&data[..3], [0x02, 0x01, 0x06], "flags");
        let length = usize::from(data[3]);
        assert_eq!(data.len(), 4 + length, "one service data structure after");
        assert_eq!(&data[4..8], [0x16, 0xD2, 0xFC, 0x45]);
        let body = &data[8..];
        let (objects, rest) = body.split_at(body.len() - 8);
        let counter = u32::from_le_bytes(rest[..4].try_into().expect("4"));
        // CTR is its own inverse: sealing the ciphertext gives back the
        // plaintext (and a tag for that, which must differ).
        let mut nonce = [0u8; 13];
        for (out, byte) in nonce[..6].iter_mut().zip(identity.address.iter().rev()) {
            *out = *byte;
        }
        nonce[6..9].copy_from_slice(&[0xD2, 0xFC, 0x45]);
        nonce[9..].copy_from_slice(&rest[..4]);
        let mut plain = objects.to_vec();
        bthome::seal_for_test(&identity.key, &nonce, &mut plain);
        // And sealing that again gives the tag that was sent.
        let mut again = plain.clone();
        let mic = bthome::seal_for_test(&identity.key, &nonce, &mut again);
        assert_eq!(again, objects);
        assert_eq!(&mic[..], &rest[4..], "the tag");
        (plain, counter)
    }

    /// Each gesture's button event, and its dimmer turning by the code:
    /// right for its own, left on the hold's for a long press (a single's
    /// and a sequence's alike), none for a release.
    #[test]
    fn each_gesture_is_a_button_event_and_turns_its_own_dimmer() {
        let identity = identity("E5:12:34:56:78:9A");
        for (code, (event, dimmer, gesture)) in [7, 47].into_iter().flat_map(|code| {
            [
                (0x01, Some((0, 0x02)), Gesture::Tap),
                (0x02, Some((1, 0x02)), Gesture::DoubleTap),
                (0x80, Some((2, 0x02)), Gesture::Hold),
                (0x04, Some((2, 0x01)), Gesture::LongPress),
                (0x81, None, Gesture::Release),
            ]
            .map(|gesture| (code, gesture))
        }) {
            let (data, len) = bthome::advert(
                Some(Press { code, gesture }),
                Layout::TwoLayers,
                82,
                0x1234,
                &identity,
            );
            assert_eq!(len, 31, "all of a legacy advertisement");
            let (plain, counter) = open(&data[..len], &identity);
            assert_eq!(counter, 0x1234);
            // The packet id is the counter's low byte; the battery; the
            // button's event.
            assert_eq!(&plain[..6], [0x00, 0x34, 0x01, 82, 0x3A, event]);
            let mut dimmers = vec![[0x3C, 0, 0]; 3];
            if let Some((slot, rotate)) = dimmer {
                dimmers[slot] = [0x3C, rotate, code];
            }
            assert_eq!(plain[6..].to_vec(), dimmers.concat(), "{code} {gesture:?}");
        }
    }

    #[test]
    fn six_buttons_are_six_button_objects() {
        let identity = identity("E5:12:34:56:78:9A");
        for (event, gesture) in [
            (0x01, Gesture::Tap),
            (0x02, Gesture::DoubleTap),
            (0x80, Gesture::Hold),
            (0x04, Gesture::LongPress),
            (0x81, Gesture::Release),
        ] {
            for code in 1..=6 {
                let press = Press { code, gesture };
                let (data, len) = bthome::advert(Some(press), Layout::Six, 82, 0x1234, &identity);
                assert_eq!(len, 30, "a byte short of a whole legacy advertisement");
                let (plain, counter) = open(&data[..len], &identity);
                assert_eq!(counter, 0x1234);
                // The packet id, then a button each, in order: the pressed
                // one's with the event, the rest "none". No battery.
                let mut buttons = vec![[0x3A, 0x00]; 6];
                buttons[usize::from(code) - 1][1] = event;
                assert_eq!(plain[..2], [0x00, 0x34]);
                assert_eq!(plain[2..].to_vec(), buttons.concat(), "{code} {gesture:?}");
            }
        }
    }

    #[test]
    fn the_battery_alone() {
        let identity = identity("E5:12:34:56:78:9A");
        let (data, len) = bthome::advert(None, Layout::Six, 140, 0x1234, &identity);
        assert_eq!(len, 20);
        let (plain, _) = open(&data[..len], &identity);
        assert_eq!(
            plain,
            [0x00, 0x34, 0x01, 100],
            "the packet id, then 100% at most"
        );
    }

    #[test]
    fn the_battery_goes_at_each_5_percent_step() {
        use bthome::battery_due;
        // At first, whatever it is.
        assert!(battery_due(None, 57));
        // Sent at 90 (the step 90–94): not within it, nor a point either
        // side, where a reading wobbles.
        for percent in 89..=95 {
            assert!(!battery_due(Some(90), percent), "{percent}");
        }
        assert!(battery_due(Some(90), 88));
        assert!(battery_due(Some(90), 96), "charging");
        // Sent at 88 (85–89) on the way down: a wobble back to 90 doesn't
        // send it again, 83 or 91 does.
        assert!(!battery_due(Some(88), 90));
        assert!(battery_due(Some(88), 91));
        assert!(battery_due(Some(88), 83));
        // The ends.
        assert!(!battery_due(Some(100), 99));
        assert!(battery_due(Some(100), 98));
        assert!(!battery_due(Some(0), 5));
        assert!(battery_due(Some(0), 6));
    }

    #[test]
    fn a_new_counter_changes_everything() {
        let identity = identity("E5:12:34:56:78:9A");
        let press = Press {
            code: 5,
            gesture: Gesture::Tap,
        };
        let (a, len) = bthome::advert(Some(press), Layout::TwoLayers, 82, 300, &identity);
        let (b, _) = bthome::advert(Some(press), Layout::TwoLayers, 82, 301, &identity);
        assert_ne!(a[8..len - 8], b[8..len - 8], "the ciphertext");
    }
}

/// Proofs, for `cargo kani`: what the tests above check for some presses,
/// for every press, battery level and counter.
///
/// AES is left out (`seal` is stubbed: CBMC would take hours over its
/// rounds, and BTHome's own example pins it down in the tests): the stub
/// leaves the objects as they are, so the proofs read them in the clear,
/// and gives any tag at all.
#[cfg(kani)]
mod proofs {
    use super::bthome::{self, Identity, advert, battery_due};
    use super::buttons::{Gesture, Layout, Press};

    /// In `seal`'s place: no encryption, any tag. Keeps its one check,
    /// that the objects fit a block.
    fn no_seal(_key: &[u8; 16], _nonce: &[u8; 13], data: &mut [u8]) -> [u8; 4] {
        assert!(data.len() <= 16, "one block");
        kani::any()
    }

    fn any_gesture() -> (Gesture, u8) {
        match kani::any::<u8>() % 5 {
            0 => (Gesture::Tap, 0x01),
            1 => (Gesture::DoubleTap, 0x02),
            2 => (Gesture::Hold, 0x80),
            3 => (Gesture::LongPress, 0x04),
            _ => (Gesture::Release, 0x81),
        }
    }

    /// The advertisement's objects, having checked what's around them: the
    /// flags, then one service data structure that takes up the rest, with
    /// BTHome's UUID and info byte first, and the counter last, before the
    /// tag.
    fn objects(data: &[u8; 31], len: usize, counter: u32) -> &[u8] {
        assert!(len <= 31);
        assert_eq!(data[..3], [0x02, 0x01, 0x06]);
        assert_eq!(4 + usize::from(data[3]), len);
        assert_eq!(data[4..8], [0x16, 0xD2, 0xFC, 0x45]);
        assert_eq!(data[len - 8..len - 4], counter.to_le_bytes());
        &data[8..len - 8]
    }

    /// For every code six buttons have, every gesture, battery level,
    /// counter, key and address: the advertisement is well-formed (see
    /// `objects`), 30 bytes, and its objects are the packet id (the
    /// counter's low byte) and six buttons, of which only the pressed one
    /// has an event, the gesture's. Nothing panics on the way.
    #[kani::proof]
    #[kani::stub(bthome::seal, no_seal)]
    #[kani::unwind(8)]
    fn six_buttons_say_which_one_and_what() {
        let (gesture, event) = any_gesture();
        let code: u8 = kani::any();
        kani::assume((1..=6).contains(&code));
        let identity = Identity {
            key: kani::any(),
            address: kani::any(),
        };
        let counter: u32 = kani::any();
        let press = Some(Press { code, gesture });
        let (data, len) = advert(press, Layout::Six, kani::any(), counter, &identity);
        assert_eq!(len, 30);
        let objects = objects(&data, len, counter);
        assert_eq!(objects[..2], [0x00, counter as u8]);
        for button in 1..=6u8 {
            let at = 2 * usize::from(button);
            let said = if button == code { event } else { 0x00 };
            assert_eq!(objects[at..at + 2], [0x3A, said]);
        }
    }

    /// For every code two layers have (1–99, none ending in 0), every
    /// gesture, battery level, counter, key and address: the
    /// advertisement is well-formed, all 31 bytes of one, and its objects
    /// are the packet id, the battery (100% at most), the button's event,
    /// and three dimmers, of which at most one turns, by the code: the
    /// gesture's own to the right, the hold's to the left for a long
    /// press, none for a release.
    #[kani::proof]
    #[kani::stub(bthome::seal, no_seal)]
    #[kani::unwind(8)]
    fn two_layers_say_the_gesture_and_the_code() {
        let (gesture, event) = any_gesture();
        let code: u8 = kani::any();
        kani::assume((1..=99).contains(&code) && (code < 10 || code % 10 != 0));
        let identity = Identity {
            key: kani::any(),
            address: kani::any(),
        };
        let (battery, counter): (u8, u32) = kani::any();
        let press = Some(Press { code, gesture });
        let (data, len) = advert(press, Layout::TwoLayers, battery, counter, &identity);
        assert_eq!(len, 31);
        let objects = objects(&data, len, counter);
        assert_eq!(
            objects[..6],
            [0x00, counter as u8, 0x01, battery.min(100), 0x3A, event]
        );
        let turns = match gesture {
            Gesture::Tap => Some((0, 0x02)),
            Gesture::DoubleTap => Some((1, 0x02)),
            Gesture::Hold => Some((2, 0x02)),
            Gesture::LongPress => Some((2, 0x01)),
            Gesture::Release => None,
        };
        for dimmer in 0..3 {
            let at = 6 + 3 * dimmer;
            let said = match turns {
                Some((slot, way)) if slot == dimmer => [0x3C, way, code],
                _ => [0x3C, 0x00, 0x00],
            };
            assert_eq!(objects[at..at + 3], said);
        }
    }

    /// For either layout, any battery level and counter: the battery's
    /// own advertisement is well-formed, 20 bytes, and holds the packet
    /// id and the battery, 100% at most.
    #[kani::proof]
    #[kani::stub(bthome::seal, no_seal)]
    #[kani::unwind(8)]
    fn the_battery_alone_is_the_battery() {
        let layout = if kani::any() {
            Layout::Six
        } else {
            Layout::TwoLayers
        };
        let identity = Identity {
            key: kani::any(),
            address: kani::any(),
        };
        let (battery, counter): (u8, u32) = kani::any();
        let (data, len) = advert(None, layout, battery, counter, &identity);
        assert_eq!(len, 20);
        let objects = objects(&data, len, counter);
        assert_eq!(objects, [0x00, counter as u8, 0x01, battery.min(100)]);
    }

    /// For every percentage last sent, and every one now: no sum wraps
    /// round; the battery isn't sent again at what was sent, nor a point
    /// either side of it (a reading wobbles by that much); and it is once
    /// it has moved 7 points, either way (5 for the step, 2 past its
    /// edge). The first one always goes.
    #[kani::proof]
    fn the_battery_goes_when_it_has_moved() {
        let (sent, percent): (u8, u8) = kani::any();
        kani::assume(sent <= 100 && percent <= 100);
        assert!(battery_due(None, percent));
        let due = battery_due(Some(sent), percent);
        let moved = sent.abs_diff(percent);
        if moved <= 1 {
            assert!(!due);
        }
        if moved >= 7 {
            assert!(due);
        }
    }
}
