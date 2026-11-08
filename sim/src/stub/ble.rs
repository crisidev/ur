//! A stand-in for `src/ble.rs`: what the Bluetooth task shares with the UI
//! loop, with the same names and types. The simulator sets what the task
//! would (whether the phone is connected, a passkey, the Aranet4s a scan
//! heard), from its commands, and prints the messages the UI queues for
//! the phone.

use core::cell::{Cell, RefCell};
use core::sync::atomic::AtomicBool;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

use crate::aranet::Nearby;
use crate::gb::Outgoing;
use crate::ui::Unpaired;

/// Whether a phone is connected, for the status bar.
pub static CONNECTED: AtomicBool = AtomicBool::new(false);
/// Whether a client is connected but not paired, and why.
pub static UNPAIRED: Mutex<CriticalSectionRawMutex, Cell<Option<Unpaired>>> =
    Mutex::new(Cell::new(None));
/// The passkey to show while pairing.
pub static PASSKEY: Mutex<CriticalSectionRawMutex, Cell<Option<u32>>> = Mutex::new(Cell::new(None));
/// Battery state, published by the UI loop for status reports to the phone.
pub static BATTERY: Mutex<CriticalSectionRawMutex, Cell<BatteryReport>> =
    Mutex::new(Cell::new(BatteryReport {
        percent: 0,
        charging: false,
        millivolts: 0,
    }));

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BatteryReport {
    pub percent: u8,
    pub charging: bool,
    pub millivolts: u32,
}

/// Messages for the phone, which [`crate::watch`] empties and prints.
pub static OUTBOX: Channel<CriticalSectionRawMutex, Outgoing, 8> = Channel::new();

/// What the scans have heard, and when the next is due; the simulator's
/// `aranet` fills it in as a scan would.
pub static NEARBY: Mutex<CriticalSectionRawMutex, RefCell<Nearby>> =
    Mutex::new(RefCell::new(Nearby::new()));

/// Asks for a scan, which the simulator doesn't run (see `aranet`).
pub static SCAN: Signal<CriticalSectionRawMutex, ()> = Signal::new();
