//! Bluetooth LE: a Nordic UART service speaking Gadgetbridge's Bangle.js
//! protocol (see [`crate::gb`] for the protocol itself).
//!
//! Gadgetbridge recognises a Bangle.js by its advertised name alone (see
//! [`NAME`]), then subscribes to the UART's TX characteristic and writes
//! lines to its RX characteristic. Everything else (time, notifications,
//! GPS, activity) travels as text over that pair.
//!
//! Pairing is required: every service but GAP needs a link encrypted with a
//! bond made with the passkey (LE Secure Connections, the passkey shown on
//! the watch), and the watch asks each client for that as it connects (an
//! SMP Security Request). A bonded phone then encrypts at once; a new one
//! pairs (Android prompts for the passkey, and gives up after 30 s). Bonds
//! are kept in flash so the phone needn't pair again. A client that stays
//! unencrypted gets a banner on the watch pointing at the README's FAQ (see
//! `not_paired`).
//!
//! The stack is Nordic's SoftDevice Controller (link layer, a binary blob
//! wrapped by `nrf-sdc`) under TrouBLE (host, in Rust), with Nordic's MPSL
//! scheduling the radio. MPSL owns RTC0, TIMER0, the RADIO, TEMP, some PPI
//! channels, and interrupt priority 0; see `main` for the setup.

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{debug, info, warn};
use embassy_futures::join::join3;
use embassy_futures::select::{Either4, select, select4};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use heapless::Vec;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use static_cell::StaticCell;
use trouble_host::prelude::*;

use crate::gb::{self, Incoming, LineBuffer, Outgoing};
use crate::geo::{self, GpsPolicy, Location};
use crate::packet_pool::Pool;
use crate::ui::{PhoneEvent, Unpaired};
use crate::{activity, clock, dfu, route, storage};

/// The advertised and GAP device name. Gadgetbridge's Bangle.js support
/// recognises `Bangle.js.*` (among a few other Espruino names).
const NAME: &str = "Bangle.js ur";

/// Whether a phone is connected, for the status bar.
pub static CONNECTED: AtomicBool = AtomicBool::new(false);
/// Whether a client is connected over a link that never became encrypted,
/// and why, for the Info screen (see [`not_paired`]).
pub static UNPAIRED: Mutex<CriticalSectionRawMutex, Cell<Option<Unpaired>>> =
    Mutex::new(Cell::new(None));
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

/// Two connections: the phone, plus one more for tools that can't share the
/// phone's (a firmware updater app, or a computer fetching the field log; see
/// [`crate::dfu`] and [`crate::field_log`]). Each needs two L2CAP channels
/// (signalling and ATT), and its own GATT client state in [`Server`].
pub const CONNECTIONS: usize = 2;
const L2CAP_CHANNELS: usize = 2 * CONNECTIONS;
/// The phone, plus room for one more (e.g. a re-pair before the old bond is
/// cleared). Must match the number of bond slots in [`storage::key::BONDS`].
const BONDS: usize = 2;

pub type Controller = nrf_sdc::SoftdeviceController<'static>;

// ==============================================================================
// GATT server
// ==============================================================================

#[gatt_server(connections_max = CONNECTIONS, packet_type = Pool)]
struct Server {
    uart: UartService,
    dfu: DfuService,
    log: LogService,
}

/// The Nordic UART service: a byte pipe each way. "RX" and "TX" are from the
/// watch's point of view.
///
/// Only for clients that paired with the passkey (`authenticated`), like
/// the rest of our services: the phone's link carries notifications (who
/// messaged you) and sets the clock, so it must be the bonded phone's, and
/// encrypted. A client that hasn't paired gets "insufficient
/// authentication" for its writes and subscriptions (writes without
/// response are dropped silently), which tells Android to pair; our
/// Security Request at connect (see `host_task`) asks for that up front.
#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
struct UartService {
    /// Phone to watch. Writes arrive in pieces of at most MTU − 3 bytes.
    #[characteristic(
        uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e",
        write,
        write_without_response,
        permissions(authenticated)
    )]
    rx: Vec<u8, 244>,
    /// Watch to phone, as notifications of up to MTU − 3 bytes. Sent with
    /// `notify_raw` and never stored, so the type only sizes a stored value
    /// that's never used: 20 bytes or less keeps it inside the attribute
    /// table, rather than in a static of its own (245 bytes for a
    /// `Vec<u8, 244>`).
    #[characteristic(
        uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e",
        notify,
        permissions(authenticated)
    )]
    tx: Vec<u8, 20>,
}

/// Nordic's legacy DFU service, for firmware updates; see [`dfu`]. Only for
/// clients that paired with the passkey (`authenticated`), since it can
/// replace the firmware.
#[gatt_service(uuid = "00001530-1212-efde-1523-785feabcd123")]
struct DfuService {
    /// Requests in, responses out (as notifications).
    #[characteristic(
        uuid = "00001531-1212-efde-1523-785feabcd123",
        write,
        notify,
        permissions(authenticated)
    )]
    control_point: Vec<u8, 20>,
    /// Image data, and the start and init parameters.
    #[characteristic(
        uuid = "00001532-1212-efde-1523-785feabcd123",
        write_without_response,
        permissions(authenticated)
    )]
    packet: Vec<u8, 244>,
    /// DFU protocol revision 0.8, as InfiniTime reports: tells clients to
    /// go ahead with the update right away rather than first asking us to
    /// restart into a separate bootloader.
    #[characteristic(uuid = "00001534-1212-efde-1523-785feabcd123", read, value = 0x0008)]
    revision: u16,
}

/// Fetching the field log ([`crate::field_log`]), one 4 KiB sector at a
/// time: write a sector index (u16, little-endian) to `control`, and `data`
/// notifies, in order:
///
/// - `00 <index: u16> <present: u8> <sequence: u32> <version: u32>
///   <length: u32>`: the sector's header (the rest is meaningless if
///   `present` is 0),
/// - `01 <offset: u16> <bytes>`: its frames, in pieces,
/// - `02 <index: u16>`: the end.
///
/// For paired clients only: logs show who messaged you.
#[gatt_service(uuid = "75726c00-6f67-4c6f-8000-000000000001")]
struct LogService {
    #[characteristic(
        uuid = "75726c00-6f67-4c6f-8000-000000000002",
        write,
        permissions(authenticated)
    )]
    control: Vec<u8, 4>,
    /// Up to 243 bytes per notification; sized like `UartService::tx`.
    #[characteristic(
        uuid = "75726c00-6f67-4c6f-8000-000000000003",
        notify,
        permissions(authenticated)
    )]
    data: Vec<u8, 20>,
}

// ==============================================================================
// Tasks
// ==============================================================================

/// Runs MPSL's low-priority work (radio scheduling housekeeping).
#[embassy_executor::task]
pub async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

type BleStack = Stack<'static, Controller, Pool>;

/// Builds the host and the GATT server, in statics.
///
/// The stack's depth is what this arrangement is about. The host's state,
/// the attribute table, and the stack itself are big values that their
/// constructors build on the stack and then move into place; built inside
/// `host_task`'s future, those temporaries reserved space in its poll
/// function's frame (4.4 KB of it), on the stack for every poll, whatever
/// it was doing. Built here, in a function of its own, they take the stack
/// once, at boot, and the statics cost no more RAM than the same values did
/// inside the task's future.
#[inline(never)]
fn build(
    controller: Controller,
    address: [u8; 6],
) -> (&'static BleStack, &'static Server<'static>) {
    // The host's resources and the server share a cell: each cell costs a
    // word for its "taken" flag.
    type Resources = HostResources<Pool, CONNECTIONS, L2CAP_CHANNELS, 1, BONDS>;
    static PARTS: StaticCell<(Resources, Server<'static>)> = StaticCell::new();
    static STACK: StaticCell<BleStack> = StaticCell::new();
    let (resources, server) = PARTS.init((
        HostResources::new(),
        Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: NAME,
            appearance: &appearance::watch::GENERIC_WATCH,
        }))
        .expect("the attribute table is sized for this server"),
    ));
    let stack = STACK.init(
        trouble_host::new(controller, resources)
            .set_random_address(Address::random(address))
            // We can show a passkey but not enter one: the phone's user
            // types the number shown on the watch, which protects pairing
            // against man-in-the-middle attacks (unlike "just works").
            .set_io_capabilities(IoCapabilities::DisplayOnly)
            .build(),
    );
    (stack, &*server)
}

/// Runs the BLE host: advertise, serve one phone, repeat.
#[embassy_executor::task]
pub async fn host_task(controller: Controller, address: [u8; 6]) {
    let (stack, server) = build(controller, address);
    for slot in storage::key::BONDS {
        if let Some(bond) = storage::load(slot).await.as_ref().and_then(decode_bond) {
            info!("ble: restored bond with {}", bond.identity.addr);
            if let Err(e) = stack.add_bond_information(bond) {
                warn!("ble: restoring bond failed: {}", e);
            }
        }
    }
    let mut runner = stack.runner();
    // Shared by the connection slots below: one advertises at a time.
    let peripheral = embassy_sync::mutex::Mutex::<NoopRawMutex, _>::new(stack.peripheral());

    let host = async {
        loop {
            if let Err(e) = runner.run().await {
                warn!("ble: host runner failed: {}", e);
            }
        }
    };
    let gps = RefCell::new(GpsPolicy::default());
    // The phone's incoming lines, shared by the connection slots rather than
    // one per slot: it's 1 KB, and only the phone writes to the UART. (Were
    // two clients to write at once, their lines would mingle and be
    // dropped as unparseable.) Borrowed only while a write is parsed, never
    // across an await.
    let lines = RefCell::new(LineBuffer::new());
    // When the phone last left, for `advertising_interval`. Booting counts:
    // the phone is waiting to reconnect then too.
    let last_disconnect: Cell<Option<Instant>> = Cell::new(Some(Instant::now()));
    // Each slot advertises until something connects, serves it until it
    // disconnects, and repeats. While one slot serves, the other advertises,
    // so a second client can always find us.
    let slot = async || {
        loop {
            let conn = {
                let mut peripheral = peripheral.lock().await;
                match advertise(
                    &mut peripheral,
                    server,
                    advertising_interval(last_disconnect.get()),
                )
                .await
                {
                    Ok(Some(conn)) => conn,
                    // Time to review the interval (see `ADVERTISING_REVIEW`).
                    Ok(None) => continue,
                    Err(e) => {
                        warn!("ble: advertising failed: {}", e);
                        Timer::after_secs(5).await;
                        continue;
                    }
                }
            };
            info!("ble: connected");
            if let Err(e) = conn.raw().set_bondable(true) {
                warn!("ble: enabling bonding failed: {}", e);
            }
            // Ask the client to secure the link (an SMP Security Request):
            // one with our bond encrypts with it at once, one without pairs,
            // which shows the passkey. Everything but the GAP service needs
            // that (see `UartService`), and some clients never ask on their
            // own: Gadgetbridge's Bangle.js support doesn't.
            if let Err(e) = conn.raw().request_security() {
                warn!("ble: requesting security failed: {}", e);
            }
            // Whether this is the phone; see `serve`. The status reporter
            // waits for that, and never finishes on its own.
            let phone = Cell::new(false);
            let since = Instant::now();
            let peer = conn.raw().peer_identity();
            ACTIVE.lock(|active| {
                let _ = active.borrow_mut().push(peer);
            });
            select(
                serve(stack, server, &conn, &gps, &lines, &phone),
                report_status(&phone),
            )
            .await;
            ACTIVE.lock(|active| active.borrow_mut().retain(|p| !p.match_identity(&peer)));
            crate::power::add(crate::power::Meter::Connected, since);
            crate::power::count(crate::power::Count::Disconnects);
            if phone.get() {
                CONNECTED.store(false, Ordering::Relaxed);
                gps.borrow_mut().on_disconnect(Instant::now());
                last_disconnect.set(Some(Instant::now()));
            }
            // Only if a pairing was cut short: a plain disconnect has nothing
            // to show, and waking the screen for it costs ~10 s of panel,
            // backlight, and heart-rate LED.
            if PASSKEY.lock(Cell::get).is_some() {
                show_passkey(None);
            }
        }
    };
    join3(host, slot(), slot()).await;
}

/// How long one advertising run lasts before its interval is reconsidered
/// (the phone may have connected or left on the other slot meanwhile).
const ADVERTISING_REVIEW: Duration = Duration::from_secs(30);

/// How often to advertise, given when the phone last disconnected.
///
/// Each advertising event costs about as much as several connection events
/// (three channels, each transmit followed by a listen), so this is most of
/// what Bluetooth costs while the phone is away:
///
/// - While the phone is connected, the other slot only serves an updater or
///   a log fetcher, whose active scans find even a slow advertiser in a few
///   seconds.
/// - For five minutes after the phone leaves (or after boot), advertise
///   quickly: it's usually trying to come straight back.
/// - Otherwise, at a moderate pace. Android reconnects with a background
///   scan that only listens in short bursts, so a slow advertiser is found
///   late: at 1022.5 ms it took the phone *four minutes* to reconnect,
///   where the old fixed 500 ms always took seconds. 417.5 ms is one of
///   Apple's recommended intervals, and about the old pace.
fn advertising_interval(last_disconnect: Option<Instant>) -> Duration {
    if CONNECTED.load(Ordering::Relaxed) {
        Duration::from_millis(2000)
    } else if last_disconnect.is_some_and(|at| at.elapsed() < Duration::from_secs(5 * 60)) {
        Duration::from_micros(152_500)
    } else {
        Duration::from_micros(417_500)
    }
}

/// Advertises at `interval` until something connects, or until
/// [`ADVERTISING_REVIEW`] passes (`None`).
async fn advertise<'values, 'server, C: trouble_host::Controller>(
    peripheral: &mut Peripheral<'values, C, Pool>,
    server: &'server Server<'values>,
    interval: Duration,
) -> Result<Option<GattConnection<'values, 'server, Pool>>, BleHostError<C::Error>> {
    let mut data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(NAME.as_bytes()),
        ],
        &mut data,
    )?;
    let parameters = AdvertisementParameters {
        interval_min: interval,
        interval_max: interval,
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
    // Dropping the advertiser (on timeout) stops advertising.
    match with_timeout(ADVERTISING_REVIEW, advertiser.accept()).await {
        Ok(conn) => Ok(Some(conn?.with_attribute_server(server)?)),
        Err(_) => Ok(None),
    }
}

/// Sends one field log sector; see [`LogService`].
async fn send_log_sector(server: &Server<'_>, conn: &GattConnection<'_, '_, Pool>, index: u16) {
    let notify = async |bytes: &[u8]| {
        if let Err(e) = server.log.data.notify_raw(conn, bytes, false).await {
            warn!("ble: sending the field log failed: {}", e);
        }
    };
    let info = crate::field_log::sector_info(u32::from(index)).await;
    let mut header: Vec<u8, 20> = Vec::new();
    let _ = header.push(0x00);
    let _ = header.extend_from_slice(&index.to_le_bytes());
    let _ = header.push(u8::from(info.is_some()));
    let (sequence, version, length) = info.map_or((0, 0, 0), |i| (i.sequence, i.version, i.length));
    for word in [sequence, version, length] {
        let _ = header.extend_from_slice(&word.to_le_bytes());
    }
    notify(&header).await;
    let mut offset = 0;
    while offset < length {
        let mut piece = [0u8; 243];
        let n = (length - offset).min(240) as usize;
        piece[0] = 0x01;
        piece[1..3].copy_from_slice(&(offset as u16).to_le_bytes());
        crate::field_log::read(u32::from(index), offset, &mut piece[3..3 + n]).await;
        notify(&piece[..3 + n]).await;
        offset += n as u32;
    }
    let [low, high] = index.to_le_bytes();
    notify(&[0x02, low, high]).await;
}

/// Marks this connection as the phone's (see [`serve`]).
fn become_phone(
    phone: &Cell<bool>,
    gps: &RefCell<GpsPolicy>,
    gps_deadline: &mut Option<Instant>,
    request_gps: impl Fn(&mut Option<Instant>),
) {
    info!("ble: the phone is here");
    phone.set(true);
    CONNECTED.store(true, Ordering::Relaxed);
    // Anything queued while it was away is stale.
    OUTBOX.clear();
    let _ = OUTBOX.try_send(Outgoing::Version);
    let mut gps = gps.borrow_mut();
    gps.on_connect(Instant::now());
    if gps.take_request(Instant::now()) {
        request_gps(gps_deadline);
    }
}

/// Asks the client for the shortest connection interval, for an update or
/// a field log fetch:
/// throughput is a few packets per connection event, so the interval is the
/// limit. Phones and BlueZ default to 30–50 ms; 7.5 ms is the minimum. The
/// client may refuse, and the phone's usual parameters return with the next
/// connection (an update ends in a reset), so nothing to undo.
async fn speed_up(stack: &Stack<'_, Controller, Pool>, conn: &GattConnection<'_, '_, Pool>) {
    let params = RequestedConnParams {
        min_connection_interval: Duration::from_micros(7_500),
        max_connection_interval: Duration::from_millis(15),
        max_latency: 0,
        supervision_timeout: Duration::from_secs(4),
        ..Default::default()
    };
    if let Err(e) = conn.raw().update_connection_params(stack, &params).await {
        warn!("dfu: asking for a faster connection failed: {}", e);
    }
}

/// After this long without traffic, ask the phone to [`slow_down`].
const IDLE_AFTER: Duration = Duration::from_secs(30);

/// Asks for connection parameters for a phone that's just keeping in
/// touch: 100–125 ms, and we may skip up to seven events in eight when we
/// have nothing to send. Gadgetbridge asks for "balanced" (30–50 ms, no
/// skipping) at connect, i.e. ~25 connection events a second, but field
/// logs show Android itself settling on 112.5–123.8 ms with latency 7 a
/// few seconds later; asking for latency 2 here (AOSP's
/// `CONNECTION_PRIORITY_LOW_POWER`) undid that and roughly tripled the
/// idle connection events. With 7, it's ~1 event a second. Anything from
/// the phone waits at most ~1 s longer; what the watch sends doesn't wait,
/// since a peripheral may use any connection event.
async fn slow_down(stack: &Stack<'_, Controller, Pool>, conn: &GattConnection<'_, '_, Pool>) {
    let params = RequestedConnParams {
        min_connection_interval: Duration::from_millis(100),
        max_connection_interval: Duration::from_millis(125),
        max_latency: 7,
        supervision_timeout: Duration::from_secs(6),
        ..Default::default()
    };
    if let Err(e) = conn.raw().update_connection_params(stack, &params).await {
        warn!("ble: asking for a slower connection failed: {}", e);
    }
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
    ActivityFetch {
        since_ms: i64,
    },
}

/// Handles one client's GATT traffic and pairing, and, if it's the phone,
/// our outgoing messages, until it disconnects.
///
/// A connection counts as the phone once it writes to the UART (Gadgetbridge
/// sends the time first thing): only then does it get our messages, and set
/// `phone`. Other clients (an updater, a log fetcher) never see them, so
/// they can't swallow messages meant for the phone.
async fn serve(
    stack: &Stack<'_, Controller, Pool>,
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, Pool>,
    gps: &RefCell<GpsPolicy>,
    lines: &RefCell<LineBuffer>,
    phone: &Cell<bool>,
) {
    let rx = server.uart.rx.handle;
    let dfu_control = server.dfu.control_point.handle;
    let dfu_packet = server.dfu.packet.handle;
    let log_control = server.log.control.handle;
    let mut update = dfu::Session::new();
    // While we've asked the phone for GPS: when to give up waiting.
    let mut gps_deadline: Option<Instant> = None;
    let request_gps = |deadline: &mut Option<Instant>| {
        info!("ble: asking the phone for a location");
        *deadline = Some(Instant::now() + geo::FIX_TIMEOUT);
        let _ = OUTBOX.try_send(Outgoing::GpsPower(true));
    };

    // When the link last carried anything, and whether we've since asked for
    // the idle parameters (see `slow_down`).
    let mut last_traffic = Instant::now();
    let mut slowed = false;
    // Whether the link's security is still to be judged (see
    // `not_paired`), at `last_traffic + SECURITY_GRACE`: while it is,
    // `last_traffic` stays at the connection's start. (It only matters once
    // the phone is here, which takes an encrypted link.) That saves an
    // `Instant` in this future, which is in RAM twice (once per slot), and
    // RAM is that scarce.
    let mut security = Security::Pending;

    let reason = loop {
        let idle = async {
            if phone.get() && !slowed && !update.active() {
                Timer::at(last_traffic + IDLE_AFTER).await
            } else {
                core::future::pending().await
            }
        };
        // The GPS and security deadlines share a branch (`select4` has
        // only four); which one passed is sorted out below.
        let deadline = async {
            let security_deadline =
                (security == Security::Pending).then_some(last_traffic + SECURITY_GRACE);
            match gps_deadline.into_iter().chain(security_deadline).min() {
                Some(deadline) => Timer::at(deadline).await,
                None => core::future::pending().await,
            }
        };
        let outgoing = async {
            if phone.get() {
                OUTBOX.receive().await
            } else {
                core::future::pending().await
            }
        };
        let event = match select4(conn.next(), outgoing, deadline, idle).await {
            Either4::First(event) => event,
            Either4::Second(message) => {
                send(server, conn, message).await;
                continue;
            }
            Either4::Fourth(()) => {
                slow_down(stack, conn).await;
                slowed = true;
                continue;
            }
            Either4::Third(()) => {
                let now = Instant::now();
                if security == Security::Pending && last_traffic + SECURITY_GRACE <= now {
                    security = Security::Settled;
                    // A pairing under way (the passkey showing) gets as
                    // long as it needs: its failure tells the user instead.
                    // Encrypted without the passkey ("just works", from a
                    // client that can't show or type one) is no better:
                    // our services need `authenticated`.
                    let authenticated = conn
                        .raw()
                        .security_level()
                        .is_ok_and(|l| l == SecurityLevel::EncryptedAuthenticated);
                    if !authenticated && PASSKEY.lock(Cell::get).is_none() {
                        not_paired(conn, &mut security);
                    }
                }
                if gps_deadline.is_some_and(|at| at <= now) {
                    warn!("ble: no location from the phone (see FAQ C03)");
                    gps_deadline = None;
                    send(server, conn, Outgoing::GpsPower(false)).await;
                }
                continue;
            }
        };
        match event {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                if security != Security::Pending {
                    last_traffic = Instant::now();
                }
                let mut actions: Vec<Action, 4> = Vec::new();
                // DFU writes are acted on after replying (flash work is
                // async; this callback isn't), so keep a copy.
                let mut dfu_write: Option<(u16, Vec<u8, 244>)> = None;
                let mut log_request: Option<u16> = None;
                if let GattEvent::Write(write) = &event {
                    let handle = write.handle();
                    if handle == rx {
                        if !phone.get() {
                            become_phone(phone, gps, &mut gps_deadline, request_gps);
                            // Whatever an earlier connection left half-sent
                            // would garble this one's first line.
                            lines.borrow_mut().clear();
                        }
                        write.with_data(|_offset, data| {
                            lines
                                .borrow_mut()
                                .push(data, |line| interpret(line, &mut actions))
                        });
                    } else if handle == log_control {
                        write.with_data(|_offset, data| {
                            if let [low, high, ..] = *data {
                                log_request = Some(u16::from_le_bytes([low, high]));
                            }
                        });
                    } else if handle == dfu_control || handle == dfu_packet {
                        write.with_data(|_offset, data| {
                            dfu_write = Vec::from_slice(data).ok().map(|data| (handle, data));
                        });
                    }
                }
                // The attribute table stores writes and answers reads, and
                // refuses what the attribute's permissions don't allow.
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("ble: failed to reply: {}", e),
                }
                if let Some(index) = log_request {
                    // A fetch is hundreds of requests; make them quick.
                    if index == 0 {
                        speed_up(stack, conn).await;
                        slowed = false;
                    }
                    send_log_sector(server, conn, index).await;
                }
                if let Some((handle, data)) = &dfu_write {
                    let was_updating = update.active();
                    let notification = if *handle == dfu_control {
                        update.on_control(data).await
                    } else {
                        update.on_packet(data).await
                    };
                    if let Some(value) = notification
                        && let Err(e) = server
                            .dfu
                            .control_point
                            .notify_raw(conn, &value, false)
                            .await
                    {
                        warn!("dfu: notifying failed: {}", e);
                    }
                    if update.active() && !was_updating {
                        speed_up(stack, conn).await;
                        slowed = false;
                    }
                }
                for action in actions {
                    match action {
                        Action::SetTime {
                            utc_seconds,
                            offset_minutes,
                        } => {
                            clock::set(utc_seconds, offset_minutes);
                            info!("ble: time set (UTC offset {} min)", offset_minutes);
                            // Proof that this image can talk to the phone,
                            // and so receive the next update.
                            crate::firmware::confirm().await;
                            gps.borrow_mut().on_time_sync(offset_minutes);
                            if gps.borrow_mut().take_request(Instant::now())
                                && gps_deadline.is_none()
                            {
                                request_gps(&mut gps_deadline);
                            }
                        }
                        Action::Fix(location) => {
                            // The phone keeps sending fixes until told to
                            // stop; one is all we wanted.
                            if gps_deadline.take().is_some() {
                                send(server, conn, Outgoing::GpsPower(false)).await;
                            }
                            gps.borrow_mut().on_fix(Instant::now());
                            update_location(location).await;
                        }
                        Action::ReportGps => {
                            send(server, conn, Outgoing::GpsPower(gps_deadline.is_some())).await;
                        }
                        Action::ActivityFetch { since_ms } => {
                            // Hundreds of samples: a few per connection
                            // event, so not at the idle interval.
                            if slowed {
                                speed_up(stack, conn).await;
                                slowed = false;
                            }
                            send_activity(server, conn, since_ms).await;
                            last_traffic = Instant::now();
                        }
                    }
                }
            }
            GattConnectionEvent::PassKeyDisplay(passkey) => {
                info!("ble: pairing; showing passkey");
                make_room_for_bond(stack, conn);
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
                warn!("ble: pairing failed: {}", e);
                show_passkey(None);
                not_paired(conn, &mut security);
            }
            GattConnectionEvent::BondLost => {
                // The phone forgot us (e.g. "forget device"); it will pair
                // afresh, and the new bond replaces the stale one.
                warn!("ble: phone no longer has our bond");
            }
            GattConnectionEvent::Encrypted { security_level, .. } => {
                info!("ble: link encrypted ({})", security_level);
                if security == Security::Pending
                    && security_level == SecurityLevel::EncryptedAuthenticated
                {
                    security = Security::Settled;
                }
            }
            // What the phone actually gave us: the single biggest factor in
            // what the connection costs (see `docs/power-plan.md`).
            GattConnectionEvent::ConnectionParamsUpdated {
                conn_interval,
                peripheral_latency,
                supervision_timeout,
            } => {
                info!(
                    "ble: connection interval {} µs, latency {}, timeout {} ms",
                    conn_interval.as_micros(),
                    peripheral_latency,
                    supervision_timeout.as_millis()
                );
            }
            _ => {}
        }
    };
    update.abandon().await;
    if security == Security::Told {
        UNPAIRED.lock(|cell| cell.set(None));
    }
    info!("ble: disconnected: {}", reason);
}

/// Where a connection stands on [`not_paired`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Security {
    /// Waiting for the link to become encrypted.
    Pending,
    /// Encrypted, or a pairing is under way: nothing to say (unless the
    /// pairing fails).
    Settled,
    /// The user has been told.
    Told,
}

/// How long a new connection has to become encrypted before we tell the
/// user it isn't (see [`not_paired`]). A bonded phone encrypts within a
/// second of our Security Request; a new one shows the passkey by then.
const SECURITY_GRACE: Duration = Duration::from_secs(10);

/// Tells the user, once per connection, that a client is connected but
/// the link isn't encrypted with a passkey bond (it ignored our Security
/// Request, or pairing failed), so it can't use anything of ours. A banner
/// points at the README's FAQ for the steps (see [`Unpaired`]), and the
/// Info screen's phone row says so for as long as the connection lasts.
///
/// Two cases, told apart by whether the client's address matches one of
/// our bonds (its identity address, or a private address that one of the
/// bonds' identity resolving keys resolves), which the host knows from the
/// moment it connects:
///
/// - It doesn't: a phone that never paired, e.g. one set up before ur
///   required pairing. It has to pair.
/// - It does, yet didn't encrypt with the bond: the phone has forgotten its
///   side (as a sealed watch's phone once had: Android had no bond, the
///   watch still did). It has to forget the watch too, and pair again.
fn not_paired(conn: &GattConnection<'_, '_, Pool>, security: &mut Security) {
    if core::mem::replace(security, Security::Told) == Security::Told {
        return;
    }
    let why = if conn.raw().is_bonded_peer() {
        Unpaired::PairingLost
    } else {
        Unpaired::NotPaired
    };
    warn!("ble: link not encrypted: {}", why);
    UNPAIRED.lock(|cell| cell.set(Some(why)));
    // Not awaited-for space: see `show_passkey`.
    let _ = crate::INPUT.try_send(crate::InputEvent::Phone(PhoneEvent::Unpaired(why)));
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
        Some(Incoming::ActivityFetch { since_ms }) => Action::ActivityFetch { since_ms },
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

/// Translates the phone's notification, call, music, find-my-watch, alarm,
/// and navigation messages for the UI. Only the fields we show are copied; in
/// particular a notification's body never leaves this function.
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
        // `{"t":"alarm","d":[{"h":7,"m":30,"rep":31,"on":true},…]}`: every
        // alarm set in Gadgetbridge's alarm screen, whenever one changes
        // there (see `onSetAlarms` in its driver). `rep` has a bit per
        // weekday, Monday first; 0 is once.
        "alarm" => {
            let mut alarms = crate::alarm::Phone::default();
            let items = object.get("d").map(|d| d.items()).into_iter().flatten();
            for alarm in items.filter_map(|v| v.as_object()) {
                let number = |key| alarm.get(key).and_then(|v| v.as_f64());
                let (Some(hour), Some(minute)) = (number("h"), number("m")) else {
                    continue;
                };
                let days = number("rep").unwrap_or(0.0) as u8;
                // A Bangle.js takes a missing `on` as on.
                let on = alarm.get("on").and_then(|v| v.as_bool()) != Some(false);
                if !alarms.push(hour as u16, minute as u16, days, on) {
                    warn!("ble: dropped an alarm ({}:{})", hour as u16, minute as u16);
                }
            }
            PhoneEvent::Alarms(alarms)
        }
        // `{"t":"nav","instr":"Storgata","distance":"150 m","action":"left",
        // "eta":"11:55"}`: the next turn while the phone navigates, every
        // field optional; `{"t":"nav"}` when navigation ends (see `route`).
        "nav" => {
            // Decoded at a size `text` is already used with (each size is
            // a copy of the decoder, ~1 KB of flash), then copied over, with
            // Google Maps' no-break spaces (between number and unit), which
            // the font lacks, as plain ones.
            fn spaced<const N: usize>(object: gb::Object<'_>, key: &str) -> heapless::String<N> {
                let decoded: heapless::String<40> = text(object, key);
                let mut out: heapless::String<N> = heapless::String::new();
                for c in decoded.chars() {
                    let c = if c == '\u{a0}' { ' ' } else { c };
                    // Trimmed, in case they come padded.
                    if c != ' ' || !out.is_empty() {
                        let _ = out.push(c);
                    }
                }
                while out.ends_with(' ') {
                    out.pop();
                }
                out
            }
            let street = spaced::<{ route::STREET_LEN }>(object, "instr");
            let distance = spaced::<{ route::DISTANCE_LEN }>(object, "distance");
            let action = object.get("action").and_then(|v| v.as_raw_str());
            let navigating = !street.is_empty() || !distance.is_empty() || action.is_some();
            PhoneEvent::Route(navigating.then(|| route::Step {
                turn: route::Turn::from_action(action),
                street,
                distance,
            }))
        }
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

/// Sends the per-minute activity samples Gadgetbridge asked for.
async fn send_activity(server: &Server<'_>, conn: &GattConnection<'_, '_, Pool>, since_ms: i64) {
    send(server, conn, Outgoing::ActivityFetch { done: false }).await;
    // Copy out in small batches: the history can't be borrowed across the
    // awaits while sending. Small, because the batch lives in the host
    // task's static storage (twice: once per connection slot); a few
    // samples per lock is plenty.
    //
    // Start early: the recent past's sleep classes and stages may have been
    // revised since the last sync (sleep is only recognised after the fact,
    // and stages need the hour after), and Gadgetbridge overwrites samples
    // it already has.
    let mut next = since_ms.div_euclid(60_000) - crate::stages::REVISED as i64;
    let mut sent = 0;
    // Whether every sample went out (as far as we can tell: notifications
    // aren't acknowledged).
    let mut complete = true;
    loop {
        let batch: Vec<(i64, activity::Sample), 4> =
            activity::HISTORY.lock(|history| history.borrow().since(next).take(4).collect());
        let Some(&(last, _)) = batch.last() else {
            break;
        };
        for (utc_minute, sample) in batch {
            let message = Outgoing::Activity {
                utc_minute,
                steps: sample.steps,
                movement: sample.movement,
                heart_rate: sample.heart_rate,
                kind: sample.sleep().gadgetbridge_name(),
            };
            complete &= send(server, conn, message).await;
            sent += 1;
        }
        next = last + 1;
    }
    complete &= send(server, conn, Outgoing::ActivityFetch { done: true }).await;
    info!("ble: sent {} minutes of activity", sent);
    if complete {
        // The phone now has everything up to the newest minute sent, if any
        // was, and up to what it asked from otherwise.
        crate::system_off::on_synced((next - 1).max(since_ms.div_euclid(60_000)));
    }
}

/// Who's connected right now, so pairing never evicts their bonds.
static ACTIVE: Mutex<CriticalSectionRawMutex, RefCell<Vec<Identity, CONNECTIONS>>> =
    Mutex::new(RefCell::new(Vec::new()));

/// Called as a pairing starts: if it will need a new bond and every slot
/// is taken, drops the bond of a device that isn't connected now, so the
/// new one fits. (The bond table has exactly [`BONDS`] slots, and a full
/// one fails the pairing with `OutOfMemory`.) A device re-pairing after
/// losing its bond needs no room: its new bond replaces its old one.
///
/// The eviction only reaches flash once the pairing completes (see
/// `save_bonds`), so a pairing that fails costs nothing after a reboot.
fn make_room_for_bond(stack: &Stack<'_, Controller, Pool>, conn: &GattConnection<'_, '_, Pool>) {
    let peer = conn.raw().peer_identity();
    let bonds: Vec<Identity, BONDS> =
        stack.with_bond_information(|bonds| bonds.iter().map(|b| b.identity).collect());
    if bonds.len() < BONDS || bonds.iter().any(|b| b.match_identity(&peer)) {
        return;
    }
    let active = ACTIVE.lock(|active| active.borrow().clone());
    let Some(&victim) = bonds
        .iter()
        .find(|b| !active.iter().any(|a| a.match_identity(b)))
    else {
        warn!("ble: every bonded device is connected; the new pairing won't be kept");
        return;
    };
    match stack.remove_bond_information(victim) {
        Ok(()) => info!("ble: dropped the bond with {} to make room", victim.addr),
        Err(e) => warn!("ble: dropping a bond failed: {}", e),
    }
}

/// Sends one message, split into notifications that fit the MTU.
/// Returns whether it went out (failures are logged).
async fn send(server: &Server<'_>, conn: &GattConnection<'_, '_, Pool>, message: Outgoing) -> bool {
    let line = message.to_line();
    let chunk = usize::from(conn.raw().att_mtu().saturating_sub(3)).clamp(20, 244);
    // `notify_raw` sends the bytes as they are, where `notify` would want
    // them copied into the characteristic's type (a 244-byte `Vec`) first,
    // which then sat in every future that sends.
    for piece in line.as_bytes().chunks(chunk) {
        if let Err(e) = server.uart.tx.notify_raw(conn, piece, false).await {
            warn!("ble: send failed: {}", e);
            return false;
        }
    }
    true
}

/// Reports the battery to the phone at connect and whenever its percentage
/// or charging state changes. (Not the voltage: it jitters by millivolts
/// from sample to sample, which made this report every minute.)
async fn report_status(phone: &Cell<bool>) {
    while !phone.get() {
        Timer::after_secs(1).await;
    }
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
    let before = PASSKEY.lock(|cell| cell.replace(passkey));
    // Only a change needs the screen: every disconnect hides the passkey,
    // and waking the screen each time (often while nobody's pairing) cost
    // power and lit the watch for no reason.
    if before != passkey {
        // Not awaited-for space: if the input queue is full, the UI is awake
        // and busy anyway and will pick up the passkey on its next frame.
        let _ = crate::INPUT.try_send(crate::InputEvent::Attention);
    }
}

/// Writes the stack's current bonds to flash, one per slot, and clears
/// unused slots.
async fn save_bonds<C: trouble_host::Controller>(stack: &Stack<'_, C, Pool>) {
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
