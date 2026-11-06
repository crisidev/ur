//! Bluetooth LE: a Nordic UART service speaking Gadgetbridge's Bangle.js
//! protocol (see [`crate::gb`] for the protocol itself).
//!
//! Gadgetbridge recognises a Bangle.js by its advertised name alone (see
//! [`NAME`]), then subscribes to the UART's TX characteristic and writes
//! lines to its RX characteristic. Everything else (time, notifications,
//! GPS, activity) travels as text over that pair.
//!
//! Pairing is required in practice: Android starts it on connect (a request
//! the watch must answer within 30 s, or the phone gives up and disconnects),
//! so we support LE Secure Connections with a passkey shown on the watch, and
//! keep the resulting bonds in flash so the phone needn't pair again.
//!
//! The stack is Nordic's SoftDevice Controller (link layer, a binary blob
//! wrapped by `nrf-sdc`) under TrouBLE (host, in Rust), with Nordic's MPSL
//! scheduling the radio. MPSL owns RTC0, TIMER0, the RADIO, TEMP, some PPI
//! channels, and interrupt priority 0; see `main` for the setup.

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{debug, info, warn};
use embassy_futures::join::join;
use embassy_futures::select::{Either3, select3};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use trouble_host::prelude::*;

use crate::gb::{self, Incoming, LineBuffer, Outgoing};
use crate::geo::{self, GpsPolicy, Location};
use crate::ui::PhoneEvent;
use crate::{clock, storage};

/// The advertised and GAP device name. Gadgetbridge's Bangle.js support
/// recognises `Bangle.js.*` (among a few other Espruino names).
const NAME: &str = "Bangle.js ur";

/// Whether a phone is connected, for the status bar.
pub static CONNECTED: AtomicBool = AtomicBool::new(false);
/// The passkey to show while pairing, for the user to type on the phone.
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

/// Messages for the phone, from anywhere in the firmware; sent while
/// connected, dropped otherwise.
pub static OUTBOX: Channel<CriticalSectionRawMutex, Outgoing, 8> = Channel::new();

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
    uart: UartService,
}

/// The Nordic UART service: a byte pipe each way. "RX" and "TX" are from the
/// watch's point of view.
#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
struct UartService {
    /// Phone to watch. Writes arrive in pieces of at most MTU − 3 bytes.
    #[characteristic(
        uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e",
        write,
        write_without_response
    )]
    rx: Vec<u8, 244>,
    /// Watch to phone, as notifications.
    #[characteristic(uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e", notify)]
    tx: Vec<u8, 244>,
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

    let host = async {
        loop {
            if let Err(e) = runner.run().await {
                warn!("ble: host runner failed: {}", defmt::Debug2Format(&e));
            }
        }
    };
    let connections = async {
        let mut gps = GpsPolicy::default();
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
            // Anything queued while disconnected is stale.
            OUTBOX.clear();
            gps.on_connect(Instant::now());
            let _ = OUTBOX.try_send(Outgoing::Version);
            // Serve the phone until it disconnects; the status reporter
            // never finishes on its own.
            embassy_futures::select::select(
                serve(&stack, &server, &conn, &mut gps),
                report_status(),
            )
            .await;
            CONNECTED.store(false, Ordering::Relaxed);
            gps.on_disconnect(Instant::now());
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

/// What a received line asks us to do; collected while parsing (which
/// happens inside a non-async callback) and acted on afterwards.
enum Action {
    SetTime {
        utc_seconds: i64,
        offset_minutes: i16,
    },
    Fix(Location),
    ReportGps,
    ActivityFetch,
}

/// Handles the phone's GATT traffic, pairing, and our outgoing messages
/// until it disconnects.
async fn serve<C: trouble_host::Controller>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    gps: &mut GpsPolicy,
) {
    let rx = server.uart.rx.handle;
    let mut lines = LineBuffer::new();
    // While we've asked the phone for GPS: when to give up waiting.
    let mut gps_deadline: Option<Instant> = None;
    let request_gps = |deadline: &mut Option<Instant>| {
        info!("ble: asking the phone for a location");
        *deadline = Some(Instant::now() + geo::FIX_TIMEOUT);
        let _ = OUTBOX.try_send(Outgoing::GpsPower(true));
    };
    if gps.take_request() {
        request_gps(&mut gps_deadline);
    }

    let reason = loop {
        let gps_timeout = async {
            match gps_deadline {
                Some(deadline) => Timer::at(deadline).await,
                None => core::future::pending().await,
            }
        };
        let event = match select3(conn.next(), OUTBOX.receive(), gps_timeout).await {
            Either3::First(event) => event,
            Either3::Second(message) => {
                send(server, conn, message).await;
                continue;
            }
            Either3::Third(()) => {
                warn!("ble: no location from the phone; is GPS forwarding enabled?");
                gps_deadline = None;
                send(server, conn, Outgoing::GpsPower(false)).await;
                continue;
            }
        };
        match event {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                let mut actions: Vec<Action, 4> = Vec::new();
                if let GattEvent::Write(write) = &event
                    && write.handle() == rx
                {
                    write.with_data(|_offset, data| {
                        lines.push(data, |line| interpret(line, &mut actions))
                    });
                }
                // The attribute table stores writes and answers reads.
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("ble: failed to reply: {}", defmt::Debug2Format(&e)),
                }
                for action in actions {
                    match action {
                        Action::SetTime {
                            utc_seconds,
                            offset_minutes,
                        } => {
                            clock::set(utc_seconds, offset_minutes);
                            info!("ble: time set (UTC offset {} min)", offset_minutes);
                            gps.on_time_sync(offset_minutes);
                            if gps.take_request() && gps_deadline.is_none() {
                                request_gps(&mut gps_deadline);
                            }
                        }
                        Action::Fix(location) => {
                            // The phone keeps sending fixes until told to
                            // stop; one is all we wanted.
                            if gps_deadline.take().is_some() {
                                send(server, conn, Outgoing::GpsPower(false)).await;
                            }
                            gps.on_fix(Instant::now());
                            update_location(location).await;
                        }
                        Action::ReportGps => {
                            send(server, conn, Outgoing::GpsPower(gps_deadline.is_some())).await
                        }
                        Action::ActivityFetch => {
                            // TODO: send samples once the accelerometer is
                            // tracked (milestone 2). An empty transfer keeps
                            // Gadgetbridge from waiting forever.
                            send(server, conn, Outgoing::ActivityFetch { done: false }).await;
                            send(server, conn, Outgoing::ActivityFetch { done: true }).await;
                        }
                    }
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
            _ => {}
        }
    };
    info!("ble: disconnected: {}", defmt::Debug2Format(&reason));
}

/// Parses one line from the phone into actions; logs what we ignore.
fn interpret(line: &[u8], actions: &mut Vec<Action, 4>) {
    let action = match gb::parse(line) {
        Some(Incoming::SetTime {
            utc_seconds,
            offset_minutes,
        }) => Action::SetTime {
            utc_seconds,
            offset_minutes,
        },
        Some(Incoming::Gps {
            latitude,
            longitude,
        }) => match Location::from_degrees(latitude, longitude) {
            Some(location) => Action::Fix(location),
            None => return,
        },
        Some(Incoming::IsGpsActive) => Action::ReportGps,
        Some(Incoming::ActivityFetch) => Action::ActivityFetch,
        Some(Incoming::Other { kind, object }) => {
            match phone_event(kind, object) {
                Some(event) => {
                    if crate::INPUT
                        .try_send(crate::InputEvent::Phone(event))
                        .is_err()
                    {
                        warn!("ble: UI busy; dropped a {=str} message", kind);
                    }
                }
                None => debug!("ble: ignoring {=str} message", kind),
            }
            return;
        }
        None => {
            debug!("ble: ignoring line {=[u8]:a}", line);
            return;
        }
    };
    if actions.push(action).is_err() {
        warn!("ble: too many messages in one write; dropping one");
    }
}

/// Translates the phone's notification, call, music, and find-my-watch
/// messages for the UI. Only the fields we show are copied; in particular a
/// notification's body never leaves this function.
fn phone_event(kind: &str, object: gb::Object<'_>) -> Option<PhoneEvent> {
    fn text<const N: usize>(object: gb::Object<'_>, key: &str) -> heapless::String<N> {
        let mut out = heapless::String::new();
        if let Some(value) = object.get(key) {
            let _ = value.decode_into(&mut out);
        }
        out
    }
    Some(match kind {
        "notify" => {
            // Messaging apps usually put the sender's name in the title and
            // leave `sender` empty.
            let sender: heapless::String<40> = text(object, "sender");
            let who = if sender.is_empty() {
                text(object, "title")
            } else {
                sender
            };
            PhoneEvent::Notification {
                app: text(object, "src"),
                who,
            }
        }
        "call" => match object.get("cmd").and_then(|v| v.as_raw_str()) {
            Some("incoming") => {
                let name: heapless::String<40> = text(object, "name");
                let caller = if name.is_empty() {
                    text(object, "number")
                } else {
                    name
                };
                PhoneEvent::IncomingCall { caller }
            }
            // Answered, rejected, ended, …: the call no longer needs us.
            _ => PhoneEvent::CallOver,
        },
        "musicinfo" => PhoneEvent::MusicInfo {
            artist: text(object, "artist"),
            track: text(object, "track"),
        },
        "musicstate" => PhoneEvent::MusicState {
            playing: object.get("state").and_then(|v| v.as_raw_str()) == Some("play"),
        },
        "find" => PhoneEvent::Find {
            on: object.get("n").and_then(|v| v.as_bool()) == Some(true),
        },
        _ => return None,
    })
}

/// Adopts a new location, saving it only when it moved noticeably (about a
/// kilometre), since the phone may send several fixes in a row.
async fn update_location(location: Location) {
    let previous = geo::has_fix().then(geo::location);
    geo::set_location(location);
    let moved = previous.is_none_or(|p| {
        p.latitude_micro.abs_diff(location.latitude_micro)
            + p.longitude_micro.abs_diff(location.longitude_micro)
            > 10_000
    });
    if moved {
        info!("ble: location now {}", location);
        storage::save(storage::key::LOCATION, &location.to_bytes()).await;
    }
}

/// Sends one message, split into notifications that fit the MTU.
async fn send(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    message: Outgoing,
) {
    let line = message.to_line();
    let chunk = usize::from(conn.raw().att_mtu().saturating_sub(3)).clamp(20, 244);
    for piece in line.as_bytes().chunks(chunk) {
        let value = Vec::from_slice(piece).expect("chunks are at most 244 bytes");
        if let Err(e) = server.uart.tx.notify(conn, &value, false).await {
            warn!("ble: send failed: {}", defmt::Debug2Format(&e));
            return;
        }
    }
}

/// Reports the battery to the phone at connect and whenever its percentage
/// or charging state changes. (Not the voltage: it jitters by millivolts
/// from sample to sample, which made this report every minute.)
async fn report_status() {
    let mut reported = None;
    loop {
        let battery = BATTERY.lock(Cell::get);
        let key = (battery.percent, battery.charging);
        if reported != Some(key) {
            let _ = OUTBOX.try_send(Outgoing::Status {
                percent: battery.percent,
                charging: battery.charging,
                millivolts: battery.millivolts,
            });
            reported = Some(key);
        }
        Timer::after_secs(60).await;
    }
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
