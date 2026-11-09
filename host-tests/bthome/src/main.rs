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

    #[test]
    fn each_gesture_is_a_button_event_and_turns_its_own_dimmer() {
        let identity = identity("E5:12:34:56:78:9A");
        for (event, slot, gesture) in [
            (0x01, Some(0), Gesture::Tap),
            (0x02, Some(1), Gesture::DoubleTap),
            (0x80, Some(2), Gesture::Hold),
            (0x04, None, Gesture::LongPress),
            (0x81, None, Gesture::Release),
        ] {
            let (data, len) = bthome::advert(
                Some(Press { code: 47, gesture }),
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
            if let Some(slot) = slot {
                dimmers[slot] = [0x3C, 0x02, 47];
            }
            assert_eq!(plain[6..].to_vec(), dimmers.concat(), "{gesture:?}");
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
