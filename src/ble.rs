//! Bluetooth LE: a GATT server that Gadgetbridge recognises as InfiniTime.
//!
//! We don't patch Gadgetbridge, so the watch speaks the protocol of the
//! firmware Gadgetbridge already supports for the PineTime, InfiniTime. The
//! relevant facts, from reading both projects' source:
//!
//! - Gadgetbridge identifies the watch *only* by its advertised name, which
//!   must match `InfiniTime.*` (see [`NAME`]).
//! - It gates protocol details on the Device Information firmware revision
//!   (see [`FIRMWARE_REVISION`]).
//! - Every characteristic it uses is optional: it skips ones that don't exist.
//!   But one that exists and *errors* aborts its whole connection setup, so
//!   everything we expose must accept reads, writes, and subscriptions.
//! - On every connect it writes the local time to the Current Time Service,
//!   which is how the watch learns the time at all (see [`crate::clock`]).
//!
//! Pairing is required in practice: Android starts it on connect (a request
//! the watch must answer within 30 s, or the phone gives up and disconnects),
//! so we support LE Secure Connections with a passkey shown on the watch, and
//! keep the resulting bonds in flash so the phone needn't pair again.
//!
//! This milestone covers time sync and battery level. Notifications (Alert
//! Notification Service) and media control (InfiniTime's music service) come
//! next, as more services on the same server.
//!
//! The stack is Nordic's SoftDevice Controller (link layer, a binary blob
//! wrapped by `nrf-sdc`) under TrouBLE (host, in Rust), with Nordic's MPSL
//! scheduling the radio. MPSL owns RTC0, TIMER0, the RADIO, TEMP, some PPI
//! channels, and interrupt priority 0; see `main` for the setup.

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use defmt::{debug, info, warn};
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};
use heapless::Vec;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use trouble_host::prelude::*;

use crate::clock::{self, DateTime};
use crate::storage;

/// The advertised and GAP device name. Gadgetbridge's PineTime support only
/// recognises devices named `InfiniTime.*` (or `Pinetime-JF.*`).
const NAME: &str = "InfiniTime ur";

/// The Device Information firmware revision we report.
///
/// Gadgetbridge parses this as exactly three numeric parts and changes
/// protocol based on it. Notably, from 0.15.0 it sends notifications as
/// `"<app>: <sender>\0<body>"` rather than just the body, and we need the app
/// and sender. Anything that isn't `N.N.N` silently gets the old format.
const FIRMWARE_REVISION: &str = "1.15.0";

/// Whether a phone is connected, for the status bar.
pub static CONNECTED: AtomicBool = AtomicBool::new(false);
/// Battery percentage, published by the UI loop for the Battery Service.
pub static BATTERY_PERCENT: AtomicU8 = AtomicU8::new(0);
/// The passkey to show while pairing, for the user to type on the phone.
pub static PASSKEY: Mutex<CriticalSectionRawMutex, Cell<Option<u32>>> = Mutex::new(Cell::new(None));

/// One connection (the phone), and the two L2CAP channels it needs
/// (signalling and ATT).
const CONNECTIONS: usize = 1;
const L2CAP_CHANNELS: usize = 2;
/// The phone, plus room for one more (e.g. a re-pair before the old bond is
/// cleared). Must match the number of bond slots in [`storage::key::BONDS`].
const BONDS: usize = 2;

pub type Controller = nrf_sdc::SoftdeviceController<'static>;

// ==============================================================================
// GATT server
// ==============================================================================

#[gatt_server]
struct Server {
    device_information: DeviceInformation,
    current_time: CurrentTimeService,
    battery: BatteryService,
}

#[gatt_service(uuid = service::DEVICE_INFORMATION)]
struct DeviceInformation {
    #[characteristic(uuid = characteristic::MANUFACTURER_NAME_STRING, read)]
    manufacturer: Vec<u8, 16>,
    #[characteristic(uuid = characteristic::MODEL_NUMBER_STRING, read)]
    model: Vec<u8, 16>,
    #[characteristic(uuid = characteristic::FIRMWARE_REVISION_STRING, read)]
    firmware_revision: Vec<u8, 16>,
}

#[gatt_service(uuid = service::CURRENT_TIME)]
struct CurrentTimeService {
    /// Year (u16 LE), month, day, hour, minute, second, weekday (Mon = 1),
    /// 1/256 s fractions, adjust reason. Gadgetbridge writes *local* time.
    #[characteristic(uuid = characteristic::CURRENT_TIME, read, write, notify)]
    current_time: [u8; 10],
    /// UTC offset and DST offset, each in 15-minute units. Gadgetbridge
    /// writes it alongside the current time; we accept and ignore it.
    #[characteristic(uuid = characteristic::LOCAL_TIME_INFORMATION, read, write)]
    local_time_information: [u8; 2],
}

#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify)]
    level: u8,
}

// ==============================================================================
// Tasks
// ==============================================================================

/// Runs MPSL's low-priority work (radio scheduling housekeeping).
#[embassy_executor::task]
pub async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

/// Runs the BLE host: advertise, serve one phone, repeat.
#[embassy_executor::task]
pub async fn host_task(controller: Controller, address: [u8; 6]) {
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS, L2CAP_CHANNELS, 1, BONDS> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(Address::random(address))
        // We can show a passkey but not enter one: the phone's user types
        // the number shown on the watch, which protects pairing against
        // man-in-the-middle attacks (unlike "just works").
        .set_io_capabilities(IoCapabilities::DisplayOnly)
        .build();
    for slot in storage::key::BONDS {
        if let Some(bond) = storage::load(slot).await.as_ref().and_then(decode_bond) {
            info!("ble: restored bond with {}", bond.identity.addr);
            if let Err(e) = stack.add_bond_information(bond) {
                warn!("ble: restoring bond failed: {}", defmt::Debug2Format(&e));
            }
        }
    }
    let mut runner = stack.runner();
    let mut peripheral = stack.peripheral();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: NAME,
        appearance: &appearance::watch::GENERIC_WATCH,
    }))
    .expect("the attribute table is sized for this server");
    let text = |s: &str| Vec::from_slice(s.as_bytes()).expect("DIS strings fit in 16 bytes");
    let dis = &server.device_information;
    for (characteristic, value) in [
        (&dis.manufacturer, text("ur")),
        (&dis.model, text("PineTime")),
        (&dis.firmware_revision, text(FIRMWARE_REVISION)),
    ] {
        server
            .set(characteristic, &value)
            .expect("characteristic belongs to this server");
    }

    let host = async {
        loop {
            if let Err(e) = runner.run().await {
                warn!("ble: host runner failed: {}", defmt::Debug2Format(&e));
            }
        }
    };
    let connections = async {
        loop {
            let conn = match advertise(&mut peripheral, &server).await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!("ble: advertising failed: {}", defmt::Debug2Format(&e));
                    Timer::after_secs(5).await;
                    continue;
                }
            };
            info!("ble: connected");
            CONNECTED.store(true, Ordering::Relaxed);
            if let Err(e) = conn.raw().set_bondable(true) {
                warn!("ble: enabling bonding failed: {}", defmt::Debug2Format(&e));
            }
            // Serve the phone until it disconnects; the battery reporter
            // never finishes on its own.
            select(
                serve(&stack, &server, &conn),
                report_battery(&server, &conn),
            )
            .await;
            CONNECTED.store(false, Ordering::Relaxed);
            show_passkey(None);
        }
    };
    join(host, connections).await;
}

/// Advertises until the phone connects.
async fn advertise<'values, 'server, C: trouble_host::Controller>(
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(NAME.as_bytes()),
        ],
        &mut data,
    )?;
    let parameters = AdvertisementParameters {
        // Slower than the 160 ms default, to save power while the phone is
        // away. A reconnecting phone still finds us within a second or two.
        // TODO: advertise quickly for a while after disconnect, then back off.
        interval_min: Duration::from_millis(500),
        interval_max: Duration::from_millis(500),
        ..Default::default()
    };
    let advertiser = peripheral
        .advertise(
            &parameters,
            Advertisement::ConnectableScannableUndirected {
                adv_data: &data[..len],
                scan_data: &[],
            },
        )
        .await?;
    Ok(advertiser.accept().await?.with_attribute_server(server)?)
}

/// Handles the phone's GATT requests and pairing until it disconnects.
async fn serve<C: trouble_host::Controller>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) {
    let current_time = server.current_time.current_time.handle;
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                match &event {
                    GattEvent::Read(e) => debug!("ble: read handle {}", e.handle()),
                    GattEvent::Write(e) => debug!("ble: write handle {}", e.handle()),
                    GattEvent::Other(e) => debug!("ble: other gatt request {}", e.payload()),
                    GattEvent::NotAllowed(_) => debug!("ble: disallowed gatt request"),
                }
                if let GattEvent::Write(write) = &event
                    && write.handle() == current_time
                {
                    write.with_data(|_offset, data| match parse_current_time(data) {
                        Some(now) => {
                            info!("ble: time set to {}", now);
                            clock::set(now);
                        }
                        None => warn!("ble: ignoring malformed current time {=[u8]:x}", data),
                    });
                }
                // Accept everything else as-is: the attribute table stores
                // writes and answers reads.
                match event.accept() {
                    Ok(reply) => {
                        reply.send().await;
                        debug!("ble: replied");
                    }
                    Err(e) => warn!("ble: failed to reply: {}", defmt::Debug2Format(&e)),
                }
            }
            GattConnectionEvent::PassKeyDisplay(passkey) => {
                info!("ble: pairing; showing passkey");
                show_passkey(Some(passkey.value()));
            }
            GattConnectionEvent::PairingComplete {
                security_level,
                bond,
            } => {
                info!(
                    "ble: paired ({}), bonded: {}",
                    security_level,
                    bond.is_some()
                );
                show_passkey(None);
                if bond.is_some() {
                    save_bonds(stack).await;
                }
            }
            GattConnectionEvent::PairingFailed(e) => {
                warn!("ble: pairing failed: {}", defmt::Debug2Format(&e));
                show_passkey(None);
            }
            GattConnectionEvent::BondLost => {
                // The phone forgot us (e.g. "forget device"); it will pair
                // afresh, and the new bond replaces the stale one.
                warn!("ble: phone no longer has our bond");
            }
            GattConnectionEvent::Encrypted { security_level, .. } => {
                info!("ble: link encrypted ({})", security_level);
            }
            _ => debug!("ble: other connection event"),
        }
    };
    info!("ble: disconnected: {}", defmt::Debug2Format(&reason));
}

/// Keeps the Battery Service value current, notifying the phone on change.
async fn report_battery(server: &Server<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>) {
    let level = &server.battery.level;
    let mut reported = None;
    loop {
        let percent = BATTERY_PERCENT.load(Ordering::Relaxed);
        if reported != Some(percent) {
            // Stores the value for reads, then notifies if the phone has
            // subscribed (silently skipped if not).
            if let Err(e) = level.notify(conn, &percent, true).await {
                warn!(
                    "ble: battery notification failed: {}",
                    defmt::Debug2Format(&e)
                );
            }
            reported = Some(percent);
        }
        Timer::after_secs(60).await;
    }
}

/// Decodes a Current Time characteristic value (Bluetooth CTS, "Exact Time
/// 256" plus adjust reason). Weekday and fractions are ignored: we derive the
/// weekday ourselves and don't need sub-second precision.
fn parse_current_time(data: &[u8]) -> Option<DateTime> {
    let &[year_lo, year_hi, month, day, hour, minute, second, ..] = data else {
        return None;
    };
    let time = DateTime {
        year: u16::from_le_bytes([year_lo, year_hi]),
        month,
        day,
        hour,
        minute,
        second,
    };
    let valid = (2000..=2199).contains(&time.year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60;
    valid.then_some(time)
}

// ==============================================================================
// Pairing and bonds
// ==============================================================================

/// Shows (or with `None`, hides) the pairing passkey on the watch, waking the
/// screen if needed.
fn show_passkey(passkey: Option<u32>) {
    PASSKEY.lock(|cell| cell.set(passkey));
    // Not awaited-for space: if the input queue is full, the UI is awake and
    // busy anyway and will pick up the passkey on its next frame.
    let _ = crate::INPUT.try_send(crate::InputEvent::Attention);
}

/// Writes the stack's current bonds to flash, one per slot, and clears
/// unused slots.
async fn save_bonds<C: trouble_host::Controller>(stack: &Stack<'_, C, DefaultPacketPool>) {
    let encoded: Vec<[u8; BOND_LEN], BONDS> = stack.with_bond_information(|bonds| {
        bonds
            .iter()
            .filter(|b| b.is_bonded)
            .take(BONDS)
            .map(encode_bond)
            .collect()
    });
    let mut ok = true;
    for (i, slot) in storage::key::BONDS.into_iter().enumerate() {
        ok &= match encoded.get(i) {
            Some(bond) => storage::save(slot, bond).await,
            None => storage::remove(slot).await,
        };
    }
    if ok {
        info!("ble: saved {} bond(s)", encoded.len());
    } else {
        warn!("ble: bonds not fully saved; the phone may need to pair again after a reboot");
    }
}

/// A bond in flash: a format version, the peer's identity address, the
/// long-term key, the peer's identity resolving key (if it sent one; phones
/// use it to recognise us behind their rotating private addresses), and how
/// the pairing went.
///
/// | bytes  | contents                                  |
/// |--------|-------------------------------------------|
/// | 0      | format version (1)                        |
/// | 1      | address kind                              |
/// | 2..8   | address                                   |
/// | 8..24  | LTK, little-endian                        |
/// | 24     | 1 if an IRK follows                       |
/// | 25..41 | IRK, little-endian                        |
/// | 41     | bonded flag                               |
/// | 42     | security level: 0 none, 1 encrypted, 2 authenticated |
const BOND_LEN: usize = 43;
const BOND_FORMAT: u8 = 1;

fn encode_bond(bond: &BondInformation) -> [u8; BOND_LEN] {
    let mut out = [0u8; BOND_LEN];
    out[0] = BOND_FORMAT;
    out[1] = bond.identity.addr.kind.as_raw();
    out[2..8].copy_from_slice(&bond.identity.addr.addr.into_inner());
    out[8..24].copy_from_slice(&bond.ltk.0.to_le_bytes());
    if let Some(irk) = &bond.identity.irk {
        out[24] = 1;
        out[25..41].copy_from_slice(&irk.0.get().to_le_bytes());
    }
    out[41] = u8::from(bond.is_bonded);
    out[42] = match bond.security_level {
        SecurityLevel::NoEncryption => 0,
        SecurityLevel::Encrypted => 1,
        SecurityLevel::EncryptedAuthenticated => 2,
    };
    out
}

/// The inverse of [`encode_bond`]; `None` for anything we didn't write.
fn decode_bond(data: &[u8; BOND_LEN]) -> Option<BondInformation> {
    if data[0] != BOND_FORMAT {
        return None;
    }
    let u128_at =
        |at: usize| u128::from_le_bytes(data[at..at + 16].try_into().expect("16-byte slice"));
    let address = Address {
        kind: AddrKind::new(data[1]),
        addr: BdAddr::new(data[2..8].try_into().expect("6-byte slice")),
    };
    let irk = if data[24] == 1 {
        IdentityResolvingKey::new(u128_at(25))
    } else {
        None
    };
    let security_level = match data[42] {
        0 => SecurityLevel::NoEncryption,
        1 => SecurityLevel::Encrypted,
        2 => SecurityLevel::EncryptedAuthenticated,
        _ => return None,
    };
    Some(BondInformation::new(
        Identity { addr: address, irk },
        LongTermKey::new(u128_at(8)),
        security_level,
        data[41] == 1,
    ))
}
