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
//! This milestone covers time sync and battery level. Notifications (Alert
//! Notification Service) and media control (InfiniTime's music service) come
//! next, as more services on the same server.
//!
//! The stack is Nordic's SoftDevice Controller (link layer, a binary blob
//! wrapped by `nrf-sdc`) under TrouBLE (host, in Rust), with Nordic's MPSL
//! scheduling the radio. MPSL owns RTC0, TIMER0, the RADIO, TEMP, some PPI
//! channels, and interrupt priority 0; see `main` for the setup.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use defmt::{info, warn};
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
use heapless::Vec;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use trouble_host::prelude::*;

use crate::clock::{self, DateTime};

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

/// One connection (the phone), and the two L2CAP channels it needs
/// (signalling and ATT).
const CONNECTIONS: usize = 1;
const L2CAP_CHANNELS: usize = 2;

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
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS, L2CAP_CHANNELS> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(Address::random(address))
        .build();
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
            // Serve the phone until it disconnects; the battery reporter
            // never finishes on its own.
            select(serve(&server, &conn), report_battery(&server, &conn)).await;
            CONNECTED.store(false, Ordering::Relaxed);
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

/// Handles the phone's GATT requests until it disconnects.
async fn serve(server: &Server<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>) {
    let current_time = server.current_time.current_time.handle;
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
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
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("ble: failed to reply: {}", defmt::Debug2Format(&e)),
                }
            }
            _ => {}
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
