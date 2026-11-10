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
//! Pairing is also what an attacker in range would go for, so: a pairing
//! that didn't use the passkey ("just works") leaves no bond behind; one
//! that fails puts back any bond it displaced; and after each failure the
//! next pairing waits longer (see [`pairing_backoff`]).
//!
//! The stack is Nordic's SoftDevice Controller (link layer, a binary blob
//! wrapped by `nrf-sdc`) under TrouBLE (host, in Rust), with Nordic's MPSL
//! scheduling the radio. MPSL owns RTC0, TIMER0, the RADIO, TEMP, some PPI
//! channels, and interrupt priority 0; see `main` for the setup.
//!
//! Besides serving the phone, the watch now and then listens for nearby
//! room sensors' advertisements, Aranet4 air monitors' and BTHome
//! thermometers' (see [`scan`], [`crate::aranet`] and
//! [`crate::bthome_sensor`] for what they say, and when to listen). That takes the
//! controller's multirole library (peripheral and scanner) rather than the
//! peripheral-only one.

use core::cell::{Cell, RefCell};
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::task::{Context, Poll};

use defmt::{debug, info, warn};
use embassy_futures::join::join4;
use embassy_futures::select::{Either, Either3, Either4, select, select3, select4};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::channel::{Channel, TrySendError};
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use heapless::Vec;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use static_cell::StaticCell;
use trouble_host::prelude::*;

use crate::aranet::{self, Nearby, Reading};
use crate::gb::{self, Incoming, LineBuffer, Outgoing};
use crate::geo::{self, GpsPolicy, Location};
use crate::link::{self, Link, Verdict};
use crate::packet_pool::Pool;
use crate::phone::{PhoneEvent, Unpaired};
use crate::{activity, clock, dfu, gb_buckets, storage};

/// The advertised and GAP device name. Gadgetbridge's Bangle.js support
/// recognises `Bangle.js.*` (among a few other Espruino names).
const NAME: &str = "Bangle.js ur";

/// Whether a phone is connected, for the status bar.
pub static CONNECTED: AtomicBool = AtomicBool::new(false);
/// Whether the last phone judged by [`link::Watch`] was disconnected for
/// not listening to the UART, and none has worked since: so that one that
/// still doesn't, once back, isn't disconnected again (and again).
static RECONNECTED: AtomicBool = AtomicBool::new(false);
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

/// A BTHome advertisement for the button grid (see [`burst`]): its data
/// and length, and until when to send it.
#[derive(Clone, Copy)]
struct Burst {
    data: [u8; 31],
    len: u8,
    until: Instant,
}

static BURST: Mutex<CriticalSectionRawMutex, Cell<Option<Burst>>> = Mutex::new(Cell::new(None));
/// A new [`BURST`], for whichever slot is advertising to switch to it.
static BURST_NEW: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// How long each grid press is advertised for. Home Assistant drops
/// repeats of a press by its packet id, but only for 4 s, so a burst must
/// end well before; scanners that listen a tenth of the time (an ESPHome
/// proxy, sharing its radio with Wi-Fi) still catch one of its ~50
/// advertisements almost surely.
const BURST_TIME: Duration = Duration::from_millis(1200);
/// How often a burst advertises: the fastest Bluetooth allows.
const BURST_INTERVAL: Duration = Duration::from_millis(20);
/// How long a burst keeps the air before another may replace it: each
/// advertisement carries one press, so one cut short after a few
/// advertisements would likely be missed. Two presses can come together
/// (one touch can complete two; a hold's release can follow it within
/// moments), and the second then waits. ~32 advertisements: a scanner
/// listening a tenth of the time misses them all 3% of the time, against
/// 0.6% for a whole burst; a hold's release comes at most this late,
/// where it'd stop a light dimming.
///
/// TODO: tune from what Home Assistant misses, once there's a record.
const BURST_FIRST: Duration = Duration::from_millis(800);

/// Advertises `data` (a BTHome advertisement from [`crate::bthome`]) for
/// [`BURST_TIME`], instead of the usual advertising data, replacing any
/// burst under way once that's had [`BURST_FIRST`]. Only the latest
/// waits: a third press within that time replaces the second, unsent.
///
/// It rides on the connectable advertising the watch does anyway
/// (whichever slot is advertising; see `host_task`), only faster, and with
/// the name moved to the scan response: so a phone can still connect
/// mid-burst, and there's no second advertising set, which our
/// SoftDevice Controller library hasn't got.
///
/// So while both slots serve a connection (the phone and an update, or a
/// computer fetching the field log), neither advertises, and there's
/// nothing to send it with: then it's not set at all, and this returns
/// `false`, for the grid to show it went nowhere (README FAQ B05). Not
/// set, rather than left for a slot that frees up within the burst: the
/// grid would have shown it unsent by then.
///
/// TODO: a burst under way, or waiting its turn, when a connection takes
/// the last free slot stops there, though it reported itself sent; it
/// may have been heard already. Telling the grid later would need a way
/// back to it from here.
pub fn burst(data: [u8; 31], len: usize) -> bool {
    if ACTIVE.lock(|active| active.borrow().len()) >= CONNECTIONS {
        warn!("ble: every slot busy; burst not sent");
        return false;
    }
    let now = Instant::now();
    let len = len as u8;
    BURST.lock(|b| {
        // When this one may start: when the one under way (or waiting)
        // has had its share, if it hasn't yet. A burst's start is its end
        // less `BURST_TIME`, so that's all a waiting one needs to say.
        let start = b
            .get()
            .filter(|b| b.until > now)
            .map_or(now, |b| (b.until - BURST_TIME + BURST_FIRST).max(now));
        let until = start + BURST_TIME;
        b.set(Some(Burst { data, len, until }));
    });
    BURST_NEW.signal(());
    true
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

/// The longest link-layer payload we ask for (data length extension): what
/// the controller's buffers hold (`buffer_cfg` in `main` sizes them by the
/// packet pool's MTU, 251 bytes, the most there is).
const MAX_DATA_LENGTH: u16 = <Pool as PacketPool>::MTU as u16;
/// How long such a packet takes on air on the 1M PHY: 14 bytes of
/// overhead besides the payload, 8 µs a byte (2120 µs for 251 bytes).
const MAX_DATA_TIME_US: u16 = (MAX_DATA_LENGTH + 14) * 8;

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
/// Gadgetbridge carries on as if a refused subscription had worked, which
/// [`link`] notices.
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
/// it was doing. Built here, in functions of their own, they take the stack
/// once, at boot, and the statics cost no more RAM than the same values did
/// inside the task's future.
///
/// The server and the host are built one after the other, each in its own
/// frame (`#[inline(never)]`), rather than together: building the host
/// also makes its first P-256 key pair (`SecurityManager::new`), ~3.3 KB of
/// stack, and with the server's temporaries still beneath that, booting
/// went deeper than anything else the watch does.
#[inline(never)]
fn build(
    controller: Controller,
    address: [u8; 6],
) -> (&'static BleStack, &'static Server<'static>) {
    let server = build_server();
    (build_host(controller, address), server)
}

/// The GATT server, for [`build`].
#[inline(never)]
fn build_server() -> &'static Server<'static> {
    static SERVER: StaticCell<Server<'static>> = StaticCell::new();
    SERVER.init(
        Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: NAME,
            appearance: &appearance::watch::GENERIC_WATCH,
        }))
        // Not `expect`: that would format the error with `Debug`, and `&str`'s
        // `Debug` escapes characters with ~3.5 KB of Unicode tables.
        .unwrap_or_else(|e| panic!("the attribute table is sized for this server: {e}")),
    )
}

/// The host, for [`build`].
#[inline(never)]
fn build_host(controller: Controller, address: [u8; 6]) -> &'static BleStack {
    type Resources = HostResources<Pool, CONNECTIONS, L2CAP_CHANNELS, 1, BONDS>;
    static RESOURCES: StaticCell<Resources> = StaticCell::new();
    static STACK: StaticCell<BleStack> = StaticCell::new();
    STACK.init(
        trouble_host::new(controller, RESOURCES.init(HostResources::new()))
            .set_random_address(Address::random(address))
            // We can show a passkey but not enter one: the phone's user
            // types the number shown on the watch, which protects pairing
            // against man-in-the-middle attacks (unlike "just works").
            .set_io_capabilities(IoCapabilities::DisplayOnly)
            .build(),
    )
}

/// Polls a future from a function of its own (`#[inline(never)]`), so
/// that its locals take a stack frame of their own instead of swelling the
/// frame of whatever polls it.
///
/// A task's `async` body is polled by one function, and the futures it
/// awaits or joins are mostly inlined into it, so its frame holds all
/// their locals at once, on the stack under every poll of any one of them.
/// Wrapping the ones that sit beside a deep path keeps them off it.
struct OwnFrame<F>(F);

impl<F: Future> Future for OwnFrame<F> {
    type Output = F::Output;

    #[inline(never)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        // SAFETY: Structural pinning: the future is only ever reached
        // through this pinned projection, never moved out, and `OwnFrame`
        // has no `Drop` (nor an `Unpin` beyond the future's own).
        unsafe { self.map_unchecked_mut(|own| &mut own.0) }.poll(cx)
    }
}

/// Runs the BLE host: advertise, serve one phone, repeat.
#[embassy_executor::task]
pub async fn host_task(controller: Controller, address: [u8; 6]) {
    let (stack, server) = build(controller, address);
    OwnFrame(restore_bonds(stack)).await;
    let (mut rx, mut control, mut tx) = stack.runner().split();
    // Shared by the connection slots below: one advertises at a time.
    let peripheral = embassy_sync::mutex::Mutex::<NoopRawMutex, _>::new(stack.peripheral());

    // The host's three loops, as `Runner::run` runs them (the first to
    // stop stops the others), but each polled in its own frame: `run`'s
    // own poll holds the control and transmit loops' locals (0.8 KB), on
    // the stack beneath the receive loop, whose pairing maths is the
    // deepest the stack goes.
    let host = async {
        loop {
            let (Either3::First(result) | Either3::Second(result) | Either3::Third(result)) =
                select3(
                    OwnFrame(tx.run()),
                    OwnFrame(rx.run_with_handler(&Reports)),
                    OwnFrame(control.run()),
                )
                .await;
            if let Err(e) = result {
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
            // Ask for the longest link-layer packets (data length
            // extension): without them, a link stays at 27 bytes a packet,
            // so a line from the phone (one or two 244-byte writes) takes
            // 6–13 packets, and at the idle parameters the phone gets about
            // one through a second. Neither the controller nor (it seems)
            // the phone asks on its own. The controller's buffers are sized
            // for it already (`buffer_cfg` in `main`). The client may offer
            // less; what it settles on is logged (`DataLengthUpdated`).
            // The HCI command itself rather than trouble-host's
            // `update_data_length`, which first asks the controller whether
            // it supports this (it does: `support_dle_peripheral` in
            // `main`), for ~0.45 KB of flash.
            let longer = bt_hci::cmd::le::LeSetDataLength::new(
                conn.raw().handle(),
                MAX_DATA_LENGTH,
                MAX_DATA_TIME_US,
            );
            if let Err(e) = stack.command(longer).await {
                warn!("ble: asking for longer packets failed: {}", e);
            }
            // Whether this is the phone; see `serve`. The status reporter
            // waits for that, and never finishes on its own.
            let phone = Cell::new(false);
            crate::power::on(crate::power::Meter::Connected);
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
            crate::power::off(crate::power::Meter::Connected);
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
    // The slots poll in frames of their own too, off the host's path, and
    // so does the scanner.
    join4(
        host,
        OwnFrame(slot()),
        OwnFrame(slot()),
        OwnFrame(scan(stack)),
    )
    .await;
}

/// Gives the host the bonds kept in flash that it hasn't got: all of them
/// at boot, and after a pairing that failed, those it dropped (see
/// [`pairing_failed`]).
async fn restore_bonds<C: trouble_host::Controller>(stack: &Stack<'_, C, Pool>) {
    for slot in storage::key::BONDS {
        if let Some(bond) = storage::load(slot).await.as_ref().and_then(decode_bond) {
            let known = stack.with_bond_information(|bonds| {
                bonds
                    .iter()
                    .any(|b| b.identity.match_identity(&bond.identity))
            });
            if known {
                continue;
            }
            info!("ble: restored bond with {}", bond.identity.addr);
            if let Err(e) = stack.add_bond_information(bond) {
                warn!("ble: restoring bond failed: {}", e);
            }
        }
    }
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
/// [`ADVERTISING_REVIEW`] passes (`None`). During a [`burst`], its data
/// instead, until it ends, or another starts (`None` either way).
async fn advertise<'values, 'server, C: trouble_host::Controller>(
    peripheral: &mut Peripheral<'values, C, Pool>,
    server: &'server Server<'values>,
    interval: Duration,
) -> Result<Option<GattConnection<'values, 'server, Pool>>, BleHostError<C::Error>> {
    // Only a burst set from here on interrupts this one.
    BURST_NEW.reset();
    let now = Instant::now();
    let burst = BURST.lock(Cell::get).filter(|b| b.until > now);
    let mut data = [0; 31];
    let mut name = [0; 31];
    let name_len = AdStructure::encode_slice(
        &[AdStructure::CompleteLocalName(NAME.as_bytes())],
        &mut name,
    )?;
    let (adv_data, scan_data, interval, review) = match &burst {
        Some(burst) => (
            &burst.data[..usize::from(burst.len)],
            &name[..name_len],
            BURST_INTERVAL,
            burst.until - now,
        ),
        None => {
            let len = AdStructure::encode_slice(
                &[
                    AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                    AdStructure::CompleteLocalName(NAME.as_bytes()),
                ],
                &mut data,
            )?;
            (&data[..len], &[][..], interval, ADVERTISING_REVIEW)
        }
    };
    let parameters = AdvertisementParameters {
        interval_min: interval,
        interval_max: interval,
        ..Default::default()
    };
    let advertiser = peripheral
        .advertise(
            &parameters,
            Advertisement::ConnectableScannableUndirected {
                adv_data,
                scan_data,
            },
        )
        .await?;
    // Dropping the advertiser (on timeout, or for a new burst) stops
    // advertising. A new burst that must wait (see `burst`) leaves this
    // one on the air until it may start.
    let new_burst = async {
        BURST_NEW.wait().await;
        if let Some(next) = BURST.lock(Cell::get) {
            Timer::at(next.until - BURST_TIME).await;
        }
    };
    match select(with_timeout(review, advertiser.accept()), new_burst).await {
        Either::First(Ok(conn)) => Ok(Some(conn?.with_attribute_server(server)?)),
        Either::First(Err(_)) | Either::Second(()) => Ok(None),
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
/// idle connection events. With 7, it's ~1 event a second. What the watch
/// sends doesn't wait, since a peripheral may use any connection event.
/// What the phone sends does: we only hear it when we listen, so the
/// first packet of a burst waits up to ~1 s, and, were we to stay that
/// deaf, each further packet as long again (a 27-byte packet a second, a
/// notification's line in 6 s). So a write from the phone has us
/// [`listen`] at every event until the burst is over.
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

/// How long the phone has been quiet when we stop [`listen`]ing at every
/// connection event. Gadgetbridge sends a burst's lines (a notification,
/// the music playing, the alarms) back to back, well within this.
const BURST_OVER_AFTER: Duration = Duration::from_secs(5);

/// Has the controller wake at every connection event (`on`), or only when
/// it has something to send or the peripheral latency is up (`!on`, as it
/// starts). Peripheral latency is what we *may* skip, not what we must, so
/// this is ours alone to choose: unlike asking the phone for latency 0 (a
/// connection update, which the phone may refuse and which takes effect
/// several events later, then another to go back), it costs no radio
/// traffic, and the very next event hears the phone. At the idle
/// parameters that's ~9 events a second instead of ~1, for the
/// [`BURST_OVER_AFTER`] after a write: ~40 events, a fraction of a
/// millicoulomb, per burst.
async fn listen(
    stack: &Stack<'_, Controller, Pool>,
    conn: &GattConnection<'_, '_, Pool>,
    on: bool,
) {
    use nrf_sdc::vendor::{NordicPeripheralLatencyModeSet, NordicPeripheralLatencyModeSetParams};
    // SDC_HCI_VS_PERIPHERAL_LATENCY_MODE_DISABLE and _ENABLE.
    let mode = if on { 0x01 } else { 0x00 };
    let command = NordicPeripheralLatencyModeSet::new(NordicPeripheralLatencyModeSetParams {
        handle: conn.raw().handle(),
        mode,
    });
    if let Err(e) = stack.command(command).await {
        warn!("ble: setting the peripheral latency mode failed: {}", e);
    }
}

/// What a received line asks us to do; collected while parsing (which
/// happens inside a non-async callback) and acted on afterwards.
enum Action {
    SetTime {
        utc_seconds: i64,
        offset_minutes: i16,
    },
    /// A location, and whether it's precise (see `aranet::precise`).
    Fix(Location, bool),
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
///
/// One that reaches for the UART (writes to it, or is refused that or its
/// subscription) must also be subscribed to it, and writing, by
/// [`link::GRACE_MS`] after its link is paired, or it's disconnected, once
/// (see [`link`] and [`judge_uart`]). The other clients never touch the
/// UART, so this never cuts short a log fetch or an update.
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
    // Whether a write from the phone has us `listen`ing at every connection
    // event, until `BURST_OVER_AFTER` after `last_traffic`. Regardless of
    // `slowed`: Android picks latency 7 on its own, seconds into a
    // connection.
    let mut listening = false;
    // Whether the link's security is still to be judged (see
    // `not_paired`), at `last_traffic + SECURITY_GRACE`: while it is,
    // `last_traffic` stays at the connection's start. (It only matters once
    // the phone is here, which takes an encrypted link.) That saves an
    // `Instant` in this future, which is in RAM twice (once per slot), and
    // RAM is that scarce.
    let mut security = Security::Pending;
    // Whether a pairing has shown its passkey and not yet ended: one the
    // client cuts short by leaving counts as failed.
    let mut pairing = false;
    // Whether the phone listens to the UART: see `link`, and `judge_uart`.
    let mut uart = link::Watch::default();

    let reason = loop {
        // The end of a burst comes first (it's sooner), then the slowing.
        let idle = async {
            if listening {
                Timer::at(last_traffic + BURST_OVER_AFTER).await
            } else if phone.get() && !slowed && !update.active() {
                Timer::at(last_traffic + IDLE_AFTER).await
            } else {
                core::future::pending().await
            }
        };
        // The GPS, security, and update deadlines share a branch
        // (`select4` has only four); which one passed is sorted out below.
        let deadline = async {
            let security_deadline =
                (security == Security::Pending).then_some(last_traffic + SECURITY_GRACE);
            let uart_deadline = uart.due().map(Instant::from_millis);
            let deadlines = gps_deadline.into_iter().chain(security_deadline);
            match deadlines
                .chain(uart_deadline)
                .chain(update.stalls_at())
                .min()
            {
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
                // With no deadline set (as `request_gps` does), a request
                // for the GPS is a room sensor scan's (see `scan`): asked
                // for if it's not too soon, and it too ends at the
                // deadline, if no fix does.
                if matches!(message, Outgoing::GpsPower(true)) && gps_deadline.is_none() {
                    if !gps.borrow_mut().for_sensors(Instant::now()) {
                        continue;
                    }
                    info!("ble: asking the phone for a location, for the room sensors");
                    gps_deadline = Some(Instant::now() + geo::FIX_TIMEOUT);
                }
                send(server, conn, message).await;
                continue;
            }
            Either4::Fourth(()) => {
                if listening {
                    listen(stack, conn, false).await;
                    listening = false;
                } else {
                    slow_down(stack, conn).await;
                    slowed = true;
                }
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
                if update.stalls_at().is_some_and(|at| at <= now) {
                    update.abandon("stalled").await;
                }
                if gps_deadline.is_some_and(|at| at <= now) {
                    warn!("ble: no location from the phone (see FAQ C03)");
                    gps_deadline = None;
                    send(server, conn, Outgoing::GpsPower(false)).await;
                }
                if uart.due().is_some_and(|at| at <= now.as_millis()) {
                    judge_uart(server, conn, &mut uart, phone.get(), update.active());
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
                // async; this callback isn't), so keep a copy: which
                // characteristic, and the data. Copied into place, rather
                // than built and moved in as an `Option<(u16, Vec)>`,
                // which put two more copies of its 0.25 KB on the stack.
                let mut dfu_handle: Option<u16> = None;
                let mut dfu_data: Vec<u8, 244> = Vec::new();
                let mut log_request: Option<u16> = None;
                // A write to one of our subscriptions (CCCDs), logged
                // below, once the attribute table has taken it.
                let mut subscription: Option<u16> = None;
                // A write to the UART: more of the phone's burst may follow.
                let mut from_phone = false;
                if let GattEvent::NotAllowed(refused) = &event {
                    // Usually a client that hasn't paired (yet): worth
                    // seeing in a field log, as the client may well carry
                    // on as if it had worked (see `link`).
                    let handle = refused.handle();
                    info!("ble: refused a request for handle {}", handle);
                    if handle == rx || Some(handle) == server.uart.tx.cccd_handle {
                        uart.reached_for_uart(Instant::now().as_millis());
                    }
                }
                if let GattEvent::Write(write) = &event {
                    let handle = write.handle();
                    if handle == rx {
                        uart.reached_for_uart(Instant::now().as_millis());
                        from_phone = true;
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
                            if dfu_data.extend_from_slice(data).is_ok() {
                                dfu_handle = Some(handle);
                            }
                        });
                    } else if [
                        server.uart.tx.cccd_handle,
                        server.dfu.control_point.cccd_handle,
                        server.log.data.cccd_handle,
                    ]
                    .contains(&Some(handle))
                    {
                        subscription = Some(handle);
                    }
                }
                // The attribute table stores writes and answers reads, and
                // refuses what the attribute's permissions don't allow.
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("ble: failed to reply: {}", e),
                }
                // Even a failed one counts: a controller refusing it would
                // otherwise be asked (and a warning logged) at every write.
                if from_phone && !listening {
                    listen(stack, conn, true).await;
                    listening = true;
                }
                if let Some(handle) = subscription {
                    subscribed(server, conn, handle, phone.get());
                }
                if let Some(index) = log_request {
                    // A fetch is hundreds of requests; make them quick.
                    if index == 0 {
                        speed_up(stack, conn).await;
                        slowed = false;
                    }
                    send_log_sector(server, conn, index).await;
                }
                if let Some(handle) = dfu_handle {
                    let was_updating = update.active();
                    let notification = if handle == dfu_control {
                        update.on_control(&dfu_data).await
                    } else {
                        update.on_packet(&dfu_data).await
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
                        Action::Fix(location, precise) => {
                            // The phone keeps sending fixes until told to
                            // stop; a precise one is all we wanted. A
                            // coarse one does for the sunrise, but not for
                            // the room sensors: the phone may yet do better
                            // before the deadline (see `geo::GpsPolicy`).
                            // Once we've stopped, a coarse one still on its
                            // way mustn't replace the precise one.
                            let asked = gps_deadline.is_some();
                            if precise && gps_deadline.take().is_some() {
                                send(server, conn, Outgoing::GpsPower(false)).await;
                            }
                            if precise || asked {
                                gps.borrow_mut().on_fix(Instant::now());
                                update_location(location, precise).await;
                            }
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
                if let Some(wait) = pairing_backoff() {
                    warn!(
                        "ble: pairing {} ms after a failed one; refused",
                        wait.as_millis()
                    );
                    conn.raw().disconnect();
                    continue;
                }
                info!("ble: pairing; showing passkey");
                pairing = true;
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
                pairing = false;
                if security_level == SecurityLevel::EncryptedAuthenticated {
                    PAIRING_FAILURES.lock(|cell| cell.set((0, Instant::now())));
                    uart.authenticated(Instant::now().as_millis());
                    if bond.is_some() {
                        save_bonds(stack).await;
                    }
                } else {
                    // "Just works", from a client that can neither show
                    // nor type a passkey: none of our services take its
                    // link, and its bond mustn't stay either. Kept, it
                    // takes a slot (two make the table full), and a client
                    // claiming a bonded phone's identity address replaces
                    // that phone's bond with its own, so the phone can't
                    // reconnect: our bonds go back to what's in flash.
                    warn!("ble: paired without the passkey; not kept");
                    if let Some(bond) = bond {
                        let _ = stack.remove_bond_information(bond.identity);
                    }
                    pairing_failed(stack).await;
                    not_paired(conn, &mut security);
                }
            }
            GattConnectionEvent::PairingFailed(e) => {
                warn!("ble: pairing failed: {}", e);
                show_passkey(None);
                pairing = false;
                pairing_failed(stack).await;
                not_paired(conn, &mut security);
            }
            GattConnectionEvent::BondLost => {
                // The phone forgot us (e.g. "forget device"); it will pair
                // afresh, and the new bond replaces the stale one.
                warn!("ble: phone no longer has our bond");
            }
            GattConnectionEvent::Encrypted { security_level, .. } => {
                info!("ble: link encrypted ({})", security_level);
                if security_level == SecurityLevel::EncryptedAuthenticated {
                    uart.authenticated(Instant::now().as_millis());
                    if security == Security::Pending {
                        security = Security::Settled;
                    }
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
            // What the client made of our `LeSetDataLength`: the
            // longest packets each way.
            GattConnectionEvent::DataLengthUpdated {
                max_tx_octets,
                max_rx_octets,
                ..
            } => {
                info!(
                    "ble: packets up to {} B out, {} B in",
                    max_tx_octets, max_rx_octets
                );
            }
            _ => {}
        }
    };
    update.abandon("left").await;
    if pairing {
        pairing_failed(stack).await;
    }
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

/// Judges, at its deadline, whether the phone listens to the UART, and
/// disconnects one that doesn't, once (see [`link`]): Gadgetbridge then
/// reconnects, and subscribes afresh over a link that's paired by now.
/// `talking` is whether it's written to the UART on this connection (it's
/// the phone; see [`serve`]).
fn judge_uart(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, Pool>,
    uart: &mut link::Watch,
    talking: bool,
    updating: bool,
) {
    let link = Link {
        authenticated: conn
            .raw()
            .security_level()
            .is_ok_and(|l| l == SecurityLevel::EncryptedAuthenticated),
        subscribed: server.uart.tx.should_notify(conn),
        talking,
        updating,
    };
    match uart.check(link, RECONNECTED.load(Ordering::Relaxed)) {
        Verdict::Working => RECONNECTED.store(false, Ordering::Relaxed),
        Verdict::Reconnect => {
            warn!(
                "ble: the phone isn't listening (subscribed: {}, writing: {}); reconnecting",
                link.subscribed, link.talking
            );
            RECONNECTED.store(true, Ordering::Relaxed);
            conn.raw().disconnect();
        }
        Verdict::Broken => warn!(
            "ble: the phone still isn't listening (subscribed: {}, writing: {}); see FAQ C05",
            link.subscribed, link.talking
        ),
        Verdict::Wait => {}
    }
}

/// A client turned notifications on or off, with a write to the CCCD at
/// `handle` (one of ours, which the attribute table has taken). Logged,
/// and if it's the phone subscribing to the UART after it first wrote to
/// it, it's told the version and the battery again: what was sent before
/// it subscribed went nowhere (see [`send`]), and Gadgetbridge only hears
/// those when they change.
fn subscribed(server: &Server<'_>, conn: &GattConnection<'_, '_, Pool>, handle: u16, phone: bool) {
    // Read back from the table, rather than parsed from the write.
    let (name, on) = if Some(handle) == server.uart.tx.cccd_handle {
        ("uart", server.uart.tx.should_notify(conn))
    } else if Some(handle) == server.dfu.control_point.cccd_handle {
        ("dfu", server.dfu.control_point.should_notify(conn))
    } else {
        ("log", server.log.data.should_notify(conn))
    };
    info!("ble: {=str} notifications {=bool}", name, on);
    if name == "uart" && on && phone {
        let _ = OUTBOX.try_send(Outgoing::Version);
        let _ = OUTBOX.try_send(status(BATTERY.lock(Cell::get)));
    }
}

/// Dispatches one line from the phone, as [`gb::parse`] reads it: the UI's
/// events go straight to the UI, what needs the connection into `actions`
/// (see [`Action`]). Logs what we ignore.
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
            latitude_micro,
            longitude_micro,
            accuracy,
        }) => match Location::from_micro(latitude_micro, longitude_micro) {
            Some(location) => Action::Fix(location, aranet::precise(accuracy)),
            None => return,
        },
        Some(Incoming::IsGpsActive) => Action::ReportGps,
        Some(Incoming::ActivityFetch { since_ms }) => Action::ActivityFetch { since_ms },
        // The UI loop runs the session (see `live`), and checks the
        // connection.
        Some(Incoming::Live {
            heart_rate,
            steps,
            interval,
        }) => {
            crate::live::ask(crate::live::Request::new(heart_rate, steps, interval));
            return;
        }
        Some(Incoming::Phone {
            event,
            text_as_bitmaps,
        }) => {
            // The warning goes after the message, to take over its banner
            // (the UI shows it once a connection).
            let warning = text_as_bitmaps.then_some(PhoneEvent::TextAsBitmaps);
            for event in [Some(event), warning].into_iter().flatten() {
                if let Err(TrySendError::Full(event)) =
                    crate::INPUT.try_send(crate::InputEvent::Phone(event))
                {
                    warn!("ble: UI busy; dropped {}", event);
                }
            }
            return;
        }
        Some(Incoming::Ignored(kind)) => {
            debug!("ble: ignoring {=str} message", kind);
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

/// Adopts a new location, saving it only when it moved noticeably (about a
/// kilometre), since the phone may send several fixes in a row.
async fn update_location(location: Location, precise: bool) {
    let previous = geo::has_fix().then(geo::location);
    geo::set_location(location, precise);
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

/// How often a fetch ends with the empty track list that has Gadgetbridge
/// pass its new data on to Health Connect (see `gb::FETCH_END`).
/// Gadgetbridge shows an "End : Fetching sports details" toast for each,
/// and with auto-fetch on that's every few minutes, so the export lags
/// the watch by up to this much instead. Nothing is lost by skipping one:
/// the export picks up from where its last run stopped. The first fetch
/// after a boot always sends it, and so does the first that sends a sleep
/// held back until its stages settled (see [`SLEEP_HELD`]).
const HEALTH_CONNECT_EVERY: Duration = Duration::from_secs(2 * 60 * 60);

/// Whether the last fetch completed held back a sleep (see
/// [`crate::sleep_hold`]). The first after it that doesn't sends the
/// night, which should reach Health Connect then, not up to
/// [`HEALTH_CONNECT_EVERY`] later: by then, the empty track list has likely
/// gone out in the night, with nothing new to export.
static SLEEP_HELD: AtomicBool = AtomicBool::new(false);

/// When the empty track list last went out: uptime in seconds plus one,
/// 0 if not since boot. Shared by both connection slots, since it's the
/// same phone's export either way.
static LAST_NO_TRACKS: AtomicU32 = AtomicU32::new(0);

/// Sends the activity Gadgetbridge asked for, in buckets of a few minutes
/// (see [`gb_buckets`]: a sample a minute, Gadgetbridge would merge).
async fn send_activity(server: &Server<'_>, conn: &GattConnection<'_, '_, Pool>, since_ms: i64) {
    let now = clock::now_utc_seconds();
    let from = gb::fetch_from(since_ms, now);
    if from != since_ms {
        info!("ble: fetch from the phone's future; sending all");
    }
    send(server, conn, Outgoing::ActivityFetch { done: false }).await;
    // Start early: the recent past's sleep classes and stages may have been
    // revised since the last sync (sleep is only recognised after the fact,
    // and stages need the hour after), and Gadgetbridge overwrites samples
    // it already has. (Their steps, heart rate and movement, that is: it
    // keeps the kind it first got, which is why the minutes whose kind
    // may still change aren't sent at all; see `sleep_hold`.)
    //
    // And from where the watch last stopped sending, if that's earlier:
    // Gadgetbridge asks from when it last fetched, which is past what the
    // watch held back then.
    //
    // Without the time there's nothing to send, as the history is filed by
    // it; the phone sets it before fetching.
    //
    // The history's oldest minutes drop out as new ones come in, which may
    // happen while the fetch goes on: so it's as if a few more had already.
    let since = from.div_euclid(60_000) - crate::stages::REVISED as i64;
    let since = crate::system_off::synced_through().map_or(since, |synced| since.min(synced + 1));
    let hold = activity::gadgetbridge_hold();
    let mut fetch = now.map(|now| {
        let oldest = activity::newest_minute().map_or(i64::MIN / 2, |newest| {
            newest - activity::HISTORY_MINUTES as i64 + 1 + gb_buckets::MINUTES
        });
        let mut fetch = gb_buckets::Fetch::new(since, oldest, now.div_euclid(60));
        if let Some(hold) = hold {
            fetch.hold(hold.from);
        }
        fetch
    });
    let mut sent = 0;
    // Whether every sample went out (as far as we can tell: notifications
    // aren't acknowledged).
    let mut complete = fetch.is_some();
    // A bucket at a time: the history can't be borrowed across the awaits
    // while sending.
    while let Some(bucket) = fetch.as_mut().and_then(|fetch| {
        activity::HISTORY.lock(|history| {
            let history = history.borrow();
            let minutes = history.since(fetch.from()).map(|(utc_minute, sample)| {
                let minute = gb_buckets::Minute {
                    steps: sample.steps,
                    movement: sample.movement,
                    heart_rate: sample.heart_rate,
                    seen: sample.seen(),
                    kind: sample.sleep().gadgetbridge_name(),
                };
                (utc_minute, minute)
            });
            fetch.next(minutes)
        })
    }) {
        let message = Outgoing::Activity {
            timestamp_ms: bucket.timestamp_ms(),
            steps: bucket.steps,
            movement: bucket.movement,
            heart_rate: bucket.heart_rate,
            kind: bucket.kind,
        };
        complete &= send(server, conn, message).await;
        sent += 1;
    }
    // The end, and the empty track list that gets Gadgetbridge to pass
    // the samples on to Health Connect (see `gb::FETCH_END`), at most
    // every `HEALTH_CONNECT_EVERY`, or when a held-back sleep went out.
    // Only the end counts towards `complete`: without the list, the phone
    // still has every sample.
    let [end, no_tracks] = gb::FETCH_END;
    complete &= send(server, conn, end).await;
    let held_sleep = hold.is_some_and(|hold| hold.sleep);
    let released = complete && !held_sleep && SLEEP_HELD.load(Ordering::Relaxed);
    if complete {
        SLEEP_HELD.store(held_sleep, Ordering::Relaxed);
    }
    // Uptime in seconds, plus one so that 0 means "not since boot".
    let uptime = Instant::now().as_secs() as u32 + 1;
    let last = LAST_NO_TRACKS.load(Ordering::Relaxed);
    if released || last == 0 || uptime - last >= HEALTH_CONNECT_EVERY.as_secs() as u32 {
        send(server, conn, no_tracks).await;
        LAST_NO_TRACKS.store(uptime, Ordering::Relaxed);
    }
    match hold {
        Some(hold) if held_sleep => info!(
            "ble: sent {} buckets of activity; sleep held from minute {}",
            sent, hold.from
        ),
        _ => info!("ble: sent {} buckets of activity", sent),
    }
    if let Some(fetch) = fetch.filter(|_| complete) {
        // The phone now has everything before the bucket under way, or the
        // first one held back.
        crate::system_off::on_synced(fetch.sent_until() - 1);
    }
}

/// How many pairings failed in a row, for [`pairing_backoff`], and when
/// the last one did.
static PAIRING_FAILURES: Mutex<CriticalSectionRawMutex, Cell<(u8, Instant)>> =
    Mutex::new(Cell::new((0, Instant::from_ticks(0))));

/// After a pairing fails, the next waits 2 s, then 4 s, … up to a minute
/// for each further failure in a row, as the Bluetooth core spec suggests
/// ("Repeated Attempts", Vol 3, Part H, 2.3.6): `Some(how much longer)`
/// while that wait lasts.
///
/// Anyone in range can start pairing, and each attempt wakes the screen
/// to show a passkey (and could evict a bond; see [`make_room_for_bond`]).
/// Unchecked, that's a battery drained by a screen kept lit, and a passkey
/// guessed (one in a million each time) at a few tries a second instead of
/// one a minute. It's one wait for everyone, since an attacker's address
/// can change at will; so a wearer who mistypes the passkey waits a few
/// seconds too, and one pairing under an attack waits up to a minute.
fn pairing_backoff() -> Option<Duration> {
    let (failures, at) = PAIRING_FAILURES.lock(Cell::get);
    if failures == 0 {
        return None;
    }
    let wait = Duration::from_secs(1 << failures.min(6)).min(Duration::from_secs(60));
    let until = at + wait;
    let now = Instant::now();
    (now < until).then(|| until - now)
}

/// After a pairing that failed, or was refused for not using the passkey:
/// starts [`pairing_backoff`]'s next wait, and puts back the bonds it
/// dropped or replaced, from flash. (`make_room_for_bond` may have evicted
/// one for it, which only flash still has until a pairing succeeds.)
async fn pairing_failed<C: trouble_host::Controller>(stack: &Stack<'_, C, Pool>) {
    PAIRING_FAILURES.lock(|cell| {
        let (failures, _) = cell.get();
        cell.set((failures.saturating_add(1), Instant::now()));
    });
    restore_bonds(stack).await;
}

/// Who's connected right now, so pairing never evicts their bonds, and
/// [`burst`] knows whether a slot is left to advertise. Each slot is in it
/// from the moment it has a connection until it's done serving it.
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
///
/// Not if the client hasn't subscribed to the UART: `notify_raw` would
/// drop each piece and still say it went out. (Until it subscribes, see
/// [`subscribed`] and [`judge_uart`].)
async fn send(server: &Server<'_>, conn: &GattConnection<'_, '_, Pool>, message: Outgoing) -> bool {
    if !server.uart.tx.should_notify(conn) {
        debug!("ble: not subscribed; dropped {}", message);
        return false;
    }
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

/// Reports the battery to the phone at connect, whenever its percentage
/// or charging state changes, and every half hour regardless (see
/// [`link::StatusReports`]).
async fn report_status(phone: &Cell<bool>) {
    while !phone.get() {
        Timer::after_secs(1).await;
    }
    let mut reports = link::StatusReports::default();
    loop {
        let battery = BATTERY.lock(Cell::get);
        if reports.due(
            battery.percent,
            battery.charging,
            Instant::now().as_millis(),
        ) {
            let _ = OUTBOX.try_send(status(battery));
        }
        Timer::after_secs(60).await;
    }
}

/// The battery, as a message for the phone.
fn status(battery: BatteryReport) -> Outgoing {
    Outgoing::Status {
        percent: battery.percent,
        charging: battery.charging,
        millivolts: battery.millivolts,
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

// ==============================================================================
// Listening for room sensors
// ==============================================================================

/// What the scans have heard, and when the next is due; see
/// [`aranet::Nearby`]. The UI loop asks for scans (with [`SCAN`]) and reads
/// the readings; [`Reports`] fills it in.
pub static NEARBY: Mutex<CriticalSectionRawMutex, RefCell<Nearby>> =
    Mutex::new(RefCell::new(Nearby::new()));

/// Starts a scan. The asker calls [`Nearby::begin`] first, so that it
/// doesn't ask again while this one runs.
pub static SCAN: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Raised when a scan has heard every sensor it waits for, to stop early.
static HEARD_EVERYONE: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Advertising reports seen by the scan under way, of any device, for
/// its log line: what the radio heard, Aranet4 or not.
static REPORTS: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

/// Runs a scan for each [`SCAN`]: passive (the readings are in the
/// advertisements themselves, so we never ask for more, and never
/// transmit), for at most [`Nearby::limit`].
///
/// The scanner listens with its window as long as its interval, which
/// makes it the controller's lowest-priority radio activity: the phone's
/// connection events, the free slot's advertising, and flash writes all
/// pre-empt it, and it picks up again after (Nordic's "SoftDevice
/// Controller scheduling"). So listening can't disturb the phone; it only
/// misses a few percent of what it could hear, which the Aranet4's next
/// advertisement, a few seconds later, makes up for. A firmware update is
/// the exception, at a connection interval fast enough to starve the
/// scanner: it doesn't scan then at all.
async fn scan(stack: &BleStack) {
    let mut central = stack.central();
    let config = ScanConfig {
        active: false,
        interval: Duration::from_millis(100),
        window: Duration::from_millis(100),
        // `timeout` is ignored for legacy scans: we time it ourselves.
        ..Default::default()
    };
    loop {
        SCAN.wait().await;
        let started = Instant::now();
        // The UI loop doesn't ask during an update, but one may have
        // started since it asked.
        let updating = dfu::PROGRESS.lock(Cell::get).is_some();
        if updating {
            info!("aranet: not scanning during an update");
        } else {
            HEARD_EVERYONE.reset();
            REPORTS.store(0, Ordering::Relaxed);
            match Scanner::new(&mut central).scan(&config).await {
                Ok(session) => {
                    crate::power::on(crate::power::Meter::Scanning);
                    let limit = NEARBY.lock(|n| n.borrow().limit(started.as_millis()));
                    let limit = Duration::from_millis(limit);
                    let _ = with_timeout(limit, HEARD_EVERYONE.wait()).await;
                    session.stop().await;
                    crate::power::off(crate::power::Meter::Scanning);
                    crate::power::count(crate::power::Count::Scans);
                }
                Err(e) => warn!("aranet: scanning failed: {}", e),
            }
        }
        let now = Instant::now().as_millis();
        let (ended, best) = NEARBY.lock(|nearby| {
            let mut nearby = nearby.borrow_mut();
            let ended = nearby.end(now);
            (ended, nearby.best().filter(|_| ended.heard))
        });
        // The sensors went (or came): where the phone is says whether the
        // wearer left them (see `aranet::Ask`). Through the phone's
        // connection, which decides whether to ask (`serve`), and turns
        // the GPS off again. Not with the phone away: it would only
        // queue, and be dropped.
        if ended.ask.is_some() && CONNECTED.load(Ordering::Relaxed) {
            let _ = OUTBOX.try_send(Outgoing::GpsPower(true));
        }
        let ms = started.elapsed().as_millis() as u32;
        let reports = REPORTS.load(Ordering::Relaxed);
        match best {
            _ if updating => {}
            Some(s) => info!(
                "aranet: {} ms, {} reports; nearest {} {=[u8]:x} at {} dBm: {} ppm, {} dC, {}%, {} s old",
                ms,
                reports,
                s.reading.kind,
                s.address,
                s.rssi,
                s.reading.co2,
                s.reading.tenths(),
                s.reading.humidity,
                s.reading.age
            ),
            None => info!("aranet: {} ms, {} reports; none heard", ms, reports),
        }
    }
}

/// Takes the scans' advertising reports, as the host's receive loop gets
/// them: keeps what Aranet4s and BTHome thermometers say, and drops
/// everything else at once (in a busy place, there are hundreds a
/// second).
struct Reports;

impl EventHandler for Reports {
    fn on_adv_reports(&self, reports: LeAdvReportsIter<'_>) {
        for report in reports.flatten() {
            REPORTS.fetch_add(1, Ordering::Relaxed);
            let Ok(address) = <aranet::Address>::try_from(report.addr.raw()) else {
                continue;
            };
            let reading = if let Some(payload) = aranet::manufacturer_payload(report.data) {
                match aranet::parse(payload) {
                    Ok(reading) => reading,
                    // Once or twice a scan, and only from an Aranet:
                    // worth knowing why it's not shown.
                    Err(skip) => {
                        info!("aranet: {=[u8]:x} has no reading: {}", address, skip);
                        continue;
                    }
                }
            } else if let Some(Ok(reading)) =
                crate::bthome_sensor::service_data(report.data).map(crate::bthome_sensor::parse)
            {
                reading
            } else {
                // Not logged when BTHome service data has no temperature
                // (encrypted, another kind of sensor, or a thermometer's
                // other advert): buttons, and other watches, send it all
                // the time.
                continue;
            };
            hear(address, report.rssi, reading);
        }
    }
}

/// A room sensor was heard: into [`NEARBY`], and stops the scan if that
/// was everyone it waits for.
fn hear(address: aranet::Address, rssi: i8, reading: Reading) {
    let now = Instant::now().as_millis();
    let (counts, everyone) = NEARBY.lock(|nearby| {
        let mut nearby = nearby.borrow_mut();
        let counts = nearby.hear(address, rssi, reading, now);
        (counts, nearby.heard_everyone(now))
    });
    if !counts {
        debug!("aranet: {=[u8]:x} at {} dBm: too weak", address, rssi);
    }
    if everyone {
        HEARD_EVERYONE.signal(());
    }
}
