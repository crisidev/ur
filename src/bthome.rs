//! The button grid's presses as BTHome v2 advertisements, for Home
//! Assistant's BTHome integration (see docs/buttons.md for setting it up,
//! and docs/bthome-design.md for the reasoning).
//!
//! BTHome is one-way: the watch advertises service data under UUID
//! 0xFCD2, and Home Assistant, listening through any of its Bluetooth
//! adapters or proxies, decodes it. No pairing, no phone.
//!
//! **What a press looks like.** Home Assistant fires *events* (rather
//! than updating a sensor's state, which a second identical press
//! wouldn't change) for BTHome's button (`0x3A`: press, double press,
//! hold, release, … and nothing more) and dimmer (`0x3C`: rotate left or
//! right, by a number of steps) objects. It tells repeated objects apart
//! only by their position in the advertisement (`button_1`, `button_2`,
//! …), and an encrypted legacy advertisement has room for 15 bytes of
//! objects: the packet id (`00`, which Home Assistant uses to drop the
//! repeats of one press), and six two-byte objects. Every advertisement
//! of a layout has the same objects, in the same order, so their
//! positions, and with them Home Assistant's names, never change. What
//! goes in them depends on the grid's [`Layout`] (see docs/bthome-design.md
//! for the budget, and why not extended advertising):
//!
//! **Six buttons:** six button objects, one per button, in order, so Home
//! Assistant names them `button_1` to `button_6`. The pressed one's says
//! what it did: `press` (`01`), `double_press` (`02`), `hold_press`
//! (`80`), `long_press` (`04`, the same hold, held on), or `hold_release`
//! (`81`, on the lift after a hold); the rest
//! say "none" (`00`), which fires nothing. Each button and gesture is a
//! device trigger in Home Assistant's UI. 14 bytes: no room for the
//! battery (two more), which goes in an advertisement of its own instead,
//! the packet id and the battery alone, which fires no event: soon after
//! boot (or turning six buttons for Home Assistant on), then as it moves
//! by a 5% step (see [`battery_due`]).
//!
//! **Two layers** (codes 1–99, none ending in 0): the buttons can't each
//! have an object (nine, let alone a code each), so each advertisement
//! says what happened twice over, for two kinds of automation:
//!
//! | Object     | Home Assistant's event class | Fires                         |
//! |------------|------------------------------|-------------------------------|
//! | `3A ..`    | `button`                     | the gesture, as BTHome names it |
//! | `3C ...`   | `dimmer_1`                   | a tap, turning right by the code |
//! | `3C ...`   | `dimmer_2`                   | a double tap, by the code     |
//! | `3C ...`   | `dimmer_3`                   | a hold right, a long press left, by the code |
//!
//! The button is the gesture alone, as above. The dimmers carry which
//! code it was: the one for the gesture turns right (`02`) by the code
//! (`23` for 2 then 3), so a single event says both, and one trigger can
//! match it. A long press (the same hold, held on) turns the hold's
//! dimmer the other way, left (`01`), by the code: there's no room for a
//! fourth dimmer, and it's a hold's, so a hold's dimmer says both, by
//! direction. An automation must then match the direction (Home
//! Assistant's `event_type`, `rotate_right` or `rotate_left`) as well as
//! the code, or a hold fires it twice; the docs' examples all do. A
//! release turns no dimmer: only one hold can be down at a time, so its
//! end needs no code. The dimmers that don't turn carry `00 00`, which
//! fires nothing. With the packet id, and the battery
//! (`01`, a battery sensor that updates with each press), that's 15
//! bytes, all there's room for.
//!
//! **Encrypted**, with AES-CCM and a key made on the watch at first boot
//! (shown on the Settings screen, for typing into Home Assistant). Home
//! Assistant then ignores anything from the watch's address it can't
//! decrypt, and drops replays of a press, and stale copies of one (which
//! Bluetooth stacks re-report from their caches) rather than firing again.
//! The counter in the nonce never repeats: it's reserved in flash ahead of
//! use (see `device::send`).
//!
//! The encoding and encryption are hardware-free, and run in host tests
//! (`host-tests/bthome`, against BTHome's published test vector) and the
//! simulator, which prints what the watch would send. `device` keeps the
//! key and counter, and hands each advertisement to [`crate::ble`].

use aes::Aes128;
use aes::cipher::{BlockEncrypt, KeyInit};

use crate::buttons::{Gesture, Layout, Press};

/// BTHome's 16-bit service UUID, as sent (little-endian).
const UUID: [u8; 2] = [0xD2, 0xFC];
/// The device information byte: encrypted (bit 0), sent irregularly, on
/// presses (bit 2: Home Assistant treats the watch as a "sleepy" device,
/// whose entities don't go unavailable between presses), BTHome v2 (bits
/// 5–7).
const INFO: u8 = 0x45;

/// What identifies the watch to Home Assistant: the key, and the address
/// it advertises from (the nonce includes it).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub key: [u8; 16],
    /// As on the air, least significant byte first (as `main` builds it).
    pub address: [u8; 6],
}

/// The advertising data for `press` on a grid laid out as `layout`, or,
/// with no press, the battery alone (see [`battery_due`]): flags, then the
/// service data, and its length. `counter` must never repeat for one key.
pub fn advert(
    press: Option<Press>,
    layout: Layout,
    battery: u8,
    counter: u32,
    identity: &Identity,
) -> ([u8; 31], usize) {
    let battery = battery.min(100);
    // The objects, in ascending id order, as BTHome asks, after the
    // packet id.
    let mut objects = [0u8; 15];
    objects[..2].copy_from_slice(&[0x00, counter as u8]);
    let len = match press {
        None => {
            objects[2..4].copy_from_slice(&[0x01, battery]);
            4
        }
        Some(press) => {
            // BTHome's button event, and which dimmer turns which way:
            // right (`02`) for the gesture's own, left (`01`) for a long
            // press on the hold's, none for a release (see the module
            // docs).
            let (event, dimmer) = match press.gesture {
                Gesture::Tap => (0x01, Some((0, 0x02))),
                Gesture::DoubleTap => (0x02, Some((1, 0x02))),
                Gesture::Hold => (0x80, Some((2, 0x02))),
                Gesture::LongPress => (0x04, Some((2, 0x01))),
                Gesture::Release => (0x81, None),
            };
            match layout {
                // A button each, the pressed one's with the event: "none"
                // (`00`) for the rest.
                Layout::Six => {
                    for (i, object) in objects[2..14].as_chunks_mut::<2>().0.iter_mut().enumerate()
                    {
                        let pressed = usize::from(press.code) == i + 1;
                        *object = [0x3A, if pressed { event } else { 0x00 }];
                    }
                    14
                }
                Layout::TwoLayers => {
                    objects[2..6].copy_from_slice(&[0x01, battery, 0x3A, event]);
                    for i in 0..3 {
                        let (rotate, steps) = match dimmer {
                            // Turning, by the code.
                            Some((slot, rotate)) if slot == i => (rotate, press.code),
                            _ => (0x00, 0x00),
                        };
                        objects[6 + 3 * i..9 + 3 * i].copy_from_slice(&[0x3C, rotate, steps]);
                    }
                    15
                }
            }
        }
    };
    let objects = &mut objects[..len];

    // The nonce: the address as people write it (most significant byte
    // first, unlike on the air), the UUID, the info byte, and the counter.
    let counter = counter.to_le_bytes();
    let mut nonce = [0u8; 13];
    for (out, byte) in nonce[..6].iter_mut().zip(identity.address.iter().rev()) {
        *out = *byte;
    }
    nonce[6..8].copy_from_slice(&UUID);
    nonce[8] = INFO;
    nonce[9..].copy_from_slice(&counter);
    let mic = seal(&identity.key, &nonce, objects);

    let mut data = [0u8; 31];
    let service_data = 1 + UUID.len() + 1 + objects.len() + counter.len() + mic.len();
    let mut len = 0;
    for part in [
        // Flags: LE general discoverable, no BR/EDR. Home Assistant's
        // Bluetooth stack (BlueZ) wants them before it reads BTHome.
        &[0x02, 0x01, 0x06][..],
        &[service_data as u8, 0x16],
        &UUID,
        &[INFO],
        objects,
        &counter,
        &mic,
    ] {
        data[len..len + part.len()].copy_from_slice(part);
        len += part.len();
    }
    (data, len)
}

/// How far the battery must go past a 5% step's edge before its own
/// advertisement goes again: a reading that wobbles across an edge (they
/// move by a point or so) mustn't send each time.
const HYSTERESIS: u8 = 2;

/// Whether to advertise the battery on its own, at `percent`, having last
/// sent `sent` (`None`: not since boot, or since six buttons with Home
/// Assistant were turned on). Six buttons' presses have no room for it
/// (see the module docs), so it goes now and then: first so Home
/// Assistant has a value, then each time it leaves the 5% step it was
/// last sent in (85–89, say), by [`HYSTERESIS`], either way, so charging
/// shows too.
pub fn battery_due(sent: Option<u8>, percent: u8) -> bool {
    let Some(sent) = sent else {
        return true;
    };
    let step = sent / 5 * 5;
    percent + HYSTERESIS <= step || percent >= step + 5 - 1 + HYSTERESIS
}

/// Encrypts `data` (at most a block) in place with AES-128-CCM, as BTHome
/// does: a 13-byte nonce (so a 2-byte length), no associated data, and a
/// 4-byte tag, which it returns.
///
/// By hand rather than through the `ccm` crate: for a single block, CCM is
/// four AES blocks, two for the tag (CBC-MAC) and two for the keystream
/// (CTR), and `aes` is already in the firmware, for Bluetooth pairing.
/// Not async, so the expanded key lives on the stack, briefly, rather than
/// in a task's future (static RAM).
fn seal(key: &[u8; 16], nonce: &[u8; 13], data: &mut [u8]) -> [u8; 4] {
    assert!(data.len() <= 16, "one block");
    let cipher = Aes128::new(key.into());
    let encrypt = |mut block: [u8; 16]| {
        cipher.encrypt_block((&mut block).into());
        block
    };
    // A block of the nonce, with flags first and a two-byte number last:
    // the message's length (for B0), or the counter (for A_i).
    let block = |flags: u8, number: u16| {
        let mut block = [0u8; 16];
        block[0] = flags;
        block[1..14].copy_from_slice(nonce);
        block[14..].copy_from_slice(&number.to_be_bytes());
        block
    };
    // CBC-MAC over B0 (flags: a 4-byte tag, (4 - 2) / 2 << 3, and a
    // 2-byte length, 2 - 1) and the data, zero-padded.
    let mut mac = encrypt(block(0x09, data.len() as u16));
    for (m, d) in mac.iter_mut().zip(data.iter()) {
        *m ^= d;
    }
    let mac = encrypt(mac);
    // CTR: A1's keystream encrypts the data, A0's the tag.
    let stream = encrypt(block(0x01, 1));
    for (d, s) in data.iter_mut().zip(stream) {
        *d ^= s;
    }
    let s0 = encrypt(block(0x01, 0));
    core::array::from_fn(|i| mac[i] ^ s0[i])
}

/// For the host tests: [`seal`] on its own, against BTHome's example.
#[cfg(not(target_os = "none"))]
pub fn seal_for_test(key: &[u8; 16], nonce: &[u8; 13], data: &mut [u8]) -> [u8; 4] {
    seal(key, nonce, data)
}

// ==============================================================================
// On the watch: the key, the counter, and sending
// ==============================================================================

#[cfg(target_os = "none")]
pub use device::{identity, init, send};

#[cfg(target_os = "none")]
mod device {
    use core::cell::Cell;

    use defmt::{info, warn};
    use embassy_nrf::rng::Rng;
    use embassy_sync::blocking_mutex::Mutex;
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

    use super::{Identity, advert};
    use crate::buttons::{Layout, Press};
    use crate::storage;

    /// The key and address, once [`init`] has loaded or made them.
    static IDENTITY: Mutex<CriticalSectionRawMutex, Cell<Option<Identity>>> =
        Mutex::new(Cell::new(None));
    /// The next counter to use, and how far flash has reserved (see
    /// [`send`]).
    static COUNTER: Mutex<CriticalSectionRawMutex, Cell<(u32, u32)>> =
        Mutex::new(Cell::new((0, 0)));

    /// How many counters one flash write reserves: one small write per
    /// this many presses.
    const RESERVE: u32 = 64;
    /// Where the counter starts on a watch that never sent one. Home
    /// Assistant only rejects a counter that went backwards from 100 up, so
    /// starting here keeps that check on from the first press.
    const FIRST: u32 = 256;

    /// Loads the key and the counter, or, on first boot, makes a key
    /// (from the hardware RNG) and starts the counter. Before the Bluetooth
    /// controller takes the RNG.
    ///
    /// A key whose counter is gone (its flash entry lost, or unreadable)
    /// is replaced, as on first boot: the counter would start again, and
    /// repeat nonces under that key, which lets anyone who heard both
    /// presses decrypt them (CTR mode). Home Assistant then can't decrypt
    /// the watch, and asks for its key again: the price of never reusing
    /// a nonce. So a new key's counter is saved with it, not at its first
    /// press: a key without one then only ever means it was lost.
    pub async fn init(rng: &mut Rng<'static, embassy_nrf::mode::Async>, address: [u8; 6]) {
        // Whatever was left of the last reserved block is skipped: it may
        // have been used. 0 if there's none (it's never below `FIRST`):
        // plain values, as what's held across an await here takes static
        // RAM, in the main task's future.
        let reserved = storage::load::<4>(storage::key::BTHOME_COUNTER)
            .await
            .map_or(0, |r| u32::from_le_bytes(r).max(FIRST));
        let mut key = storage::load::<16>(storage::key::BTHOME_KEY).await;
        if key.is_some() && reserved == 0 {
            warn!("bthome: the counter's gone; a new key, for Home Assistant");
            key = None;
        }
        let key = match key {
            Some(key) => key,
            None => {
                let mut key = [0u8; 16];
                // Bias correction, for a key's worth of good bits; slower,
                // but this is rare.
                rng.set_bias_correction(true);
                rng.fill_bytes(&mut key).await;
                rng.set_bias_correction(false);
                // And a counter with it: a key in flash without one means
                // it was lost (or this didn't finish, which is as safe).
                if storage::save(storage::key::BTHOME_KEY, &key).await
                    && storage::save(storage::key::BTHOME_COUNTER, &const { FIRST.to_le_bytes() })
                        .await
                {
                    info!("bthome: made a key");
                }
                key
            }
        };
        let reserved = reserved.max(FIRST);
        IDENTITY.lock(|i| i.set(Some(Identity { key, address })));
        COUNTER.lock(|c| c.set((reserved, reserved)));
    }

    /// The key and address, for the Settings screen's key page.
    pub fn identity() -> Option<Identity> {
        IDENTITY.lock(Cell::get)
    }

    /// Advertises `press`, on a grid laid out as `layout` (or with none,
    /// the battery alone), for a moment (see `ble::burst`). The counter is
    /// reserved in flash before it's used, so a reset can never make it
    /// repeat with another press: a repeat would let anyone listening
    /// decrypt both (CTR mode). If flash can't reserve one, nothing is sent.
    ///
    /// Returns whether it's going out: not without a key, a counter, or a
    /// free connection slot to advertise it (see `ble::burst`).
    pub async fn send(press: Option<Press>, layout: Layout, battery: u8) -> bool {
        let Some(identity) = identity() else {
            warn!("bthome: no key");
            return false;
        };
        let (next, reserved) = COUNTER.lock(Cell::get);
        if next >= reserved {
            let reserve = next + RESERVE;
            if !storage::save(storage::key::BTHOME_COUNTER, &reserve.to_le_bytes()).await {
                warn!("bthome: couldn't reserve a counter; not sent");
                return false;
            }
            COUNTER.lock(|c| c.set((next, reserve)));
        }
        COUNTER.lock(|c| c.set((next + 1, c.get().1)));
        let (data, len) = advert(press, layout, battery, next, &identity);
        info!("bthome: {=u32} {=[u8]:02x}", next, data[..len]);
        crate::ble::burst(data, len)
    }
}
