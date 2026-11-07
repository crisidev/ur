//! Living under the PineTime bootloader: what our own firmware image needs to
//! do so it can be installed, updated, and rolled back.
//!
//! The stock bootloader (MCUBoot, plus PINE64's boot logo and button
//! handling; layout in `memory.x`) runs on every reset. Updates arrive over
//! Bluetooth into the *secondary* slot in the external SPI flash (see
//! [`crate::dfu`]). On the next reset MCUBoot swaps the slots and boots the
//! new image on probation: unless the new image marks itself OK ("confirms"),
//! the reset after that swaps back. Three things here make that work:
//!
//! - [`clean_up_after_bootloader`]: the bootloader leaves peripherals running
//!   that our drivers don't expect.
//! - [`confirm`]: once this image has shown it can do the one thing that
//!   matters most (talk to the phone, so it can receive the *next* update),
//!   it's kept.
//! - The UI loop stops feeding the watchdog while the button is held (see
//!   `main`), so holding it for ~7 s always resets into the bootloader, where
//!   continuing to hold it selects rollback. That's the escape hatch for a
//!   sealed watch whose firmware runs but is broken.

use defmt::{info, warn};

// ==============================================================================
// Bootloader leftovers
// ==============================================================================

/// The peripherals the bootloader may leave enabled or mid-operation. It
/// drives the LCD and SPI flash through SPI0 on the same pins as our SPIM2,
/// and contending for those pins left our first display write hanging
/// forever. The rest are here because the bootloader's HAL may use them and
/// our drivers (or MPSL, which asserts on unexpected state) assume reset
/// values.
const BOOTLOADER_PERIPHERALS: [usize; 11] = [
    0x4000_2000, // UARTE0 (console)
    0x4000_3000, // SPI(M)0 / TWI(M)0
    0x4000_4000, // SPI(M)1 / TWI(M)1
    0x4002_3000, // SPIM2
    0x4000_6000, // GPIOTE
    0x4000_7000, // SAADC
    0x4000_8000, // TIMER0
    0x4000_9000, // TIMER1
    0x4000_A000, // TIMER2
    0x4000_B000, // RTC0
    0x4001_1000, // RTC1
];

/// Puts everything the bootloader may have touched back into its reset
/// state. Call first thing at boot, before any driver is set up.
///
/// Watchdog excepted: nothing stops it once started, which is the point.
pub fn clean_up_after_bootloader() {
    // No interrupt the bootloader enabled should fire into our vector table.
    let nvic = cortex_m::peripheral::NVIC::PTR;
    for register in 0..2 {
        // SAFETY: Writing all-ones to ICER/ICPR only disables and un-pends
        // interrupts, which nothing has enabled yet at this point in boot.
        unsafe {
            (*nvic).icer[register].write(u32::MAX);
            (*nvic).icpr[register].write(u32::MAX);
        }
    }

    // Each nRF52 peripheral has an undocumented power register at offset
    // 0xFFC; clearing and setting it resets the peripheral completely. Nordic
    // uses this in their own errata workarounds (e.g. anomaly 89 for TWIM),
    // and it's the only way to fully reset a peripheral short of a chip
    // reset.
    for base in BOOTLOADER_PERIPHERALS {
        let power = (base + 0xFFC) as *mut u32;
        // SAFETY: These are the peripherals' power registers; nothing is
        // using the peripherals yet.
        unsafe {
            power.write_volatile(0);
            power.read_volatile();
            power.write_volatile(1);
        }
    }

    // Disconnect every PPI channel the bootloader may have wired up; MPSL and
    // the Bluetooth controller connect the ones they own.
    embassy_nrf::pac::PPI
        .chenclr()
        .write_value(embassy_nrf::pac::ppi::regs::Chen(u32::MAX));
}

/// The running image's version, from its MCUBoot header (`ih_ver`: major,
/// minor, revision), as `scripts/package.py --version` set it.
pub fn version() -> (u8, u8, u16) {
    // SAFETY: Internal flash is memory-mapped and always readable; the
    // header's version starts 20 bytes into the slot.
    let [major, minor, low, high] = unsafe { (0x8014 as *const [u8; 4]).read_volatile() };
    (major, minor, u16::from_le_bytes([low, high]))
}

// ==============================================================================
// Confirming the image
// ==============================================================================

/// MCUBoot's "image OK" flag for the primary slot: in the slot's trailer,
/// 24 bytes before its end (after the 16-byte magic and 8 bytes of
/// alignment). A 1 here means "keep this image".
const IMAGE_OK_ADDRESS: u32 = 0x7C000 - 16 - 8;
const IMAGE_OK: u32 = 1;

fn is_confirmed() -> bool {
    // SAFETY: Internal flash is memory-mapped and always readable.
    let flag = unsafe { (IMAGE_OK_ADDRESS as *const u32).read_volatile() };
    flag & 0xFF == IMAGE_OK
}

/// Marks this image as good, so the bootloader keeps it. Idempotent.
///
/// We confirm once the phone has synced the time with us: that proves the
/// Bluetooth stack, pairing, and the Gadgetbridge protocol work, i.e. that
/// this image can receive the next update if it turns out to be bad in
/// other ways. Anything less (e.g. "booted and ran for a minute") could
/// keep an image that can never be replaced without opening the watch.
///
/// TODO: if the phone is out of reach when a new image first boots, a reset
/// before it reconnects rolls the update back. That's the safe direction;
/// an "OK" button on the watch could complement it.
pub async fn confirm() {
    if is_confirmed() {
        return;
    }
    if crate::storage::write_word_elsewhere(IMAGE_OK_ADDRESS, IMAGE_OK).await && is_confirmed() {
        info!("firmware: image confirmed; the bootloader will keep it");
    } else {
        warn!("firmware: confirming the image failed; the next reset rolls back");
    }
}
