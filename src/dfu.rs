//! Receiving firmware updates over Bluetooth: Nordic's legacy DFU protocol,
//! as InfiniTime implements it, writing into MCUBoot's secondary slot.
//!
//! Speaking InfiniTime's dialect means the same `*-dfu.zip` (made by
//! `scripts/package.py`) installs us over InfiniTime and updates us
//! afterwards, using the same tools: `scripts/dfu.py`, nRF Connect, or the
//! nRF Device Firmware Update app. (Gadgetbridge can't: it knows us as a
//! Bangle.js, and its Bangle.js support doesn't do firmware.)
//!
//! The exchange, with the phone writing to the control point and packet
//! characteristics and the watch answering with control point notifications
//! (`10 <opcode> <status>`):
//!
//! 1. Start (`01 04`, "application"), then the packet: three little-endian
//!    u32 sizes (SoftDevice, bootloader, application; only the last may be
//!    non-zero).
//! 2. Init (`02 00`), the packet: the `.dat` init packet, whose last two
//!    bytes are the image's CRC-16, then `02 01`. We erase the slot here;
//!    clients expect the "init done" response to take a while.
//! 3. Optionally, packet receipt notifications every N packets (`08 <N>`):
//!    the client's flow control.
//! 4. Receive (`03`), then the image as packets of up to 20 bytes, with a
//!    receipt (`11 <bytes so far>`) every N packets and a response once all
//!    of it is in.
//! 5. Validate (`04`): we read the slot back and check its CRC, and that
//!    it's an image the bootloader will install and that will run here:
//!    header, sizes, SHA-256, vector table (see [`crate::mcuboot`]).
//! 6. Activate (`05`): we mark the slot for the bootloader, save what a
//!    reset would lose (see [`reset`]), and reset. The bootloader swaps the
//!    slots and boots the update on probation (see
//!    [`crate::firmware::confirm`]).
//!
//! Only authenticated (passkey-paired) connections may use the service,
//! every characteristic of it, from the first write (see
//! `ble::DfuService`): otherwise anyone within Bluetooth range could
//! install whatever they like, since the bootloader checks a hash but not
//! a signature.
//!
//! Anything wrong with an image refuses it before the swap is asked for,
//! and the watch says so (`UPDATE REFUSED`, README FAQ U02) and keeps
//! running what it has. So do a sender that goes quiet for a minute
//! ([`STALL`]) and one that leaves: the slot is left without the swap
//! request, which the bootloader ignores, and the next update erases it.
//! One connection at a time may update; another's Start is refused.

use core::cell::Cell;
use core::ops::Range;
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{info, warn};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant};
use heapless::Vec;

use crate::mcuboot::{self, Problem};
use crate::phone::PhoneEvent;
use crate::spi_flash::{self, FLASH};

// ==============================================================================
// The secondary slot
// ==============================================================================

/// Where the bootloader looks for updates in the SPI flash, and how big the
/// slot is (the same size as the primary; see `memory.x`).
const SLOT_START: u32 = 0x40000;
const SLOT_SIZE: u32 = 0x74000;
/// The end of the slot holds MCUBoot's swap-state trailer.
const TRAILER_SIZE: u32 = 0x1B0;
/// The most image the slot takes.
const CAPACITY: u32 = SLOT_SIZE - TRAILER_SIZE;
/// MCUBoot's trailer magic (`boot_img_magic` in MCUBoot 1.5), as bytes:
/// written to the last 16 bytes of the secondary slot, it asks for a swap.
/// Its "image OK" flag stays unset, so the swap is a test: the new image
/// must confirm itself or the next reset swaps back.
const BOOT_MAGIC: [u32; 4] = [0xf395c277, 0x7fefd260, 0x0f505235, 0x8079b62c];

/// Whether `range` lies within the secondary slot. Every erase and program
/// here checks it first: below the slot are the bootloader's boot logo and
/// recovery firmware, above it our saved activity and field log, and no
/// slip in the arithmetic should reach them.
fn in_slot(range: Range<u32>) -> bool {
    SLOT_START <= range.start && range.start <= range.end && range.end <= SLOT_START + SLOT_SIZE
}

/// How far an update has got (percent), while one is in progress; the UI
/// shows it.
pub static PROGRESS: BlockingMutex<CriticalSectionRawMutex, Cell<Option<u8>>> =
    BlockingMutex::new(Cell::new(None));

/// Whether a connection's update holds the slot. Two at once (each
/// connection has a [`Session`]) would erase and write over each other,
/// and one could ask for a swap of what the other half-wrote.
static SLOT_TAKEN: AtomicBool = AtomicBool::new(false);

/// How long an update may go without a word from its sender before it's
/// abandoned. Otherwise a sender that stalls (an app sent to the
/// background, say) would hold the slot, keep the progress screen lit, and
/// keep the field log paused, for as long as it stayed connected. The
/// longest pause in a working update is ours (erasing the slot, or reading
/// it back: seconds), during which nothing waits on this.
pub const STALL: Duration = Duration::from_secs(60);

// ==============================================================================
// Protocol
// ==============================================================================

mod opcode {
    pub const START: u8 = 0x01;
    pub const INIT: u8 = 0x02;
    pub const RECEIVE: u8 = 0x03;
    pub const VALIDATE: u8 = 0x04;
    pub const ACTIVATE_AND_RESET: u8 = 0x05;
    pub const RESET: u8 = 0x06;
    pub const REPORT_SIZE: u8 = 0x07;
    pub const RECEIPT_REQUEST: u8 = 0x08;
    pub const RESPONSE: u8 = 0x10;
    pub const RECEIPT: u8 = 0x11;
}

mod status {
    pub const SUCCESS: u8 = 0x01;
    pub const INVALID_STATE: u8 = 0x02;
    pub const NOT_SUPPORTED: u8 = 0x03;
    pub const TOO_BIG: u8 = 0x04;
    pub const CRC_ERROR: u8 = 0x05;
    pub const OPERATION_FAILED: u8 = 0x06;
}

/// The "application" image type; we take no SoftDevice or bootloader.
const APPLICATION: u8 = 0x04;

/// A control point notification for the client.
pub type Notification = Vec<u8, 8>;

fn response(request: u8, status: u8) -> Notification {
    Vec::from_slice(&[opcode::RESPONSE, request, status]).expect("fits")
}

#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
enum State {
    Idle,
    /// Start received; the next packet holds the image sizes.
    AwaitingSizes,
    /// Sizes accepted; waiting for the init packet.
    Started,
    /// Collecting the init packet.
    Init,
    /// Slot erased; waiting for Receive.
    Ready,
    Receiving,
    /// The whole image is in flash.
    Received,
    Validated,
}

/// One client's update, from Start to Activate. Lives for one connection;
/// a disconnect abandons it (the slot is left without the trailer magic, so
/// the bootloader ignores whatever was written).
pub struct Session {
    state: State,
    image_size: u32,
    received: u32,
    init_packet: Vec<u8, 32>,
    expected_crc: u16,
    receipt_interval: u16,
    packets_since_receipt: u16,
    /// Data waiting to be programmed: one flash page at a time.
    page: Vec<u8, { spi_flash::PAGE as usize }>,
    /// Whether this session holds [`SLOT_TAKEN`].
    owns_slot: bool,
    /// When the sender last wrote anything, for [`STALL`].
    last_heard: Instant,
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            image_size: 0,
            received: 0,
            init_packet: Vec::new(),
            expected_crc: 0,
            receipt_interval: 0,
            packets_since_receipt: 0,
            page: Vec::new(),
            owns_slot: false,
            last_heard: Instant::from_ticks(0),
        }
    }

    /// When to give up on a sender that's gone quiet (see [`STALL`]), while
    /// an update is under way.
    pub fn stalls_at(&self) -> Option<Instant> {
        self.active().then(|| self.last_heard + STALL)
    }

    /// Whether an update is under way (for the UI, and to keep the phone's
    /// other traffic out of the way).
    pub fn active(&self) -> bool {
        self.state != State::Idle
    }

    /// Handles a control point write; returns the notification to send, if
    /// any. Activation doesn't return: it resets the chip.
    pub async fn on_control(&mut self, request: &[u8]) -> Option<Notification> {
        self.last_heard = Instant::now();
        let (&op, arguments) = request.split_first()?;
        match (op, self.state) {
            (opcode::START, _) => {
                // A client may restart at any point (e.g. after a failed
                // attempt on the same connection).
                if arguments.first() != Some(&APPLICATION) {
                    return Some(response(op, status::NOT_SUPPORTED));
                }
                if !self.owns_slot && SLOT_TAKEN.swap(true, Ordering::Relaxed) {
                    warn!("dfu: refused; another connection is updating");
                    return Some(response(op, status::INVALID_STATE));
                }
                *self = Self::new();
                self.owns_slot = true;
                self.last_heard = Instant::now();
                self.state = State::AwaitingSizes;
                PROGRESS.lock(|p| p.set(Some(0)));
                // Wake the screen for the progress bar.
                let _ = crate::INPUT.try_send(crate::InputEvent::Attention);
                None
            }
            (opcode::INIT, State::Started) if arguments.first() != Some(&1) => {
                self.state = State::Init;
                None
            }
            // `02 01`: the init packet is complete. (Very old clients send a
            // bare `02` after the packet; treat that the same.)
            (opcode::INIT, State::Init) => Some(self.finish_init().await),
            (opcode::RECEIPT_REQUEST, _) => {
                if let [low, high, ..] = *arguments {
                    self.receipt_interval = u16::from_le_bytes([low, high]);
                }
                None
            }
            (opcode::RECEIVE, State::Ready) => {
                self.state = State::Receiving;
                None
            }
            (opcode::VALIDATE, State::Received) => Some(self.validate().await),
            (opcode::ACTIVATE_AND_RESET, State::Validated) => {
                self.activate().await;
                Some(response(op, status::OPERATION_FAILED))
            }
            (opcode::RESET, _) => reset("requested by the update client").await,
            (opcode::REPORT_SIZE, _) => {
                let mut reply = response(op, status::SUCCESS);
                reply
                    .extend_from_slice(&self.received.to_le_bytes())
                    .expect("fits");
                Some(reply)
            }
            (op, state) => {
                warn!("dfu: opcode {:#x} unexpected while {}", op, state);
                Some(response(op, status::INVALID_STATE))
            }
        }
    }

    /// Handles a packet characteristic write; returns the notification to
    /// send, if any.
    pub async fn on_packet(&mut self, data: &[u8]) -> Option<Notification> {
        self.last_heard = Instant::now();
        match self.state {
            State::AwaitingSizes => Some(self.accept_sizes(data)),
            State::Init => {
                if self.init_packet.extend_from_slice(data).is_err() {
                    warn!("dfu: init packet too long");
                    self.fail();
                    return Some(response(opcode::INIT, status::OPERATION_FAILED));
                }
                None
            }
            State::Receiving => self.receive(data).await,
            state => {
                warn!(
                    "dfu: {} bytes of data unexpected while {}",
                    data.len(),
                    state
                );
                None
            }
        }
    }

    /// Called when the client disconnects (`why`: "left"), or has gone
    /// quiet for [`STALL`] ("stalled").
    pub async fn abandon(&mut self, why: &'static str) {
        if self.active() {
            warn!(
                "dfu: client {=str} mid-update ({}); abandoned",
                why, self.state
            );
            self.fail();
            if let Some(flash) = FLASH.lock().await.as_mut() {
                flash.sleep().await;
            }
        }
    }

    fn fail(&mut self) {
        if self.owns_slot {
            SLOT_TAKEN.store(false, Ordering::Relaxed);
        }
        *self = Self::new();
        PROGRESS.lock(|p| p.set(None));
    }

    /// Gives up on an image that won't do (see [`mcuboot`]): logs why,
    /// tells the wearer, and returns the client's answer to `request`.
    fn refuse(&mut self, request: u8, problem: Problem) -> Notification {
        warn!("dfu: refused the image: {}", problem);
        self.fail();
        // Not awaited-for space: the UI is busy if its queue is full, and
        // the client hears about it regardless.
        let _ = crate::INPUT.try_send(crate::InputEvent::Phone(PhoneEvent::UpdateRefused));
        let status = match problem {
            Problem::Corrupt => status::CRC_ERROR,
            _ => status::OPERATION_FAILED,
        };
        response(request, status)
    }

    fn accept_sizes(&mut self, data: &[u8]) -> Notification {
        let size = |i: usize| {
            data.get(i * 4..i * 4 + 4)
                .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
        };
        let (Some(softdevice), Some(bootloader), Some(application)) = (size(0), size(1), size(2))
        else {
            self.fail();
            return response(opcode::START, status::OPERATION_FAILED);
        };
        if softdevice != 0 || bootloader != 0 {
            self.fail();
            return response(opcode::START, status::NOT_SUPPORTED);
        }
        if application == 0 || application > CAPACITY {
            warn!(
                "dfu: an image of {} bytes doesn't fit the slot",
                application
            );
            self.fail();
            return response(opcode::START, status::TOO_BIG);
        }
        info!("dfu: starting; image is {} bytes", application);
        self.image_size = application;
        self.state = State::Started;
        response(opcode::START, status::SUCCESS)
    }

    /// Parses the init packet and erases the slot.
    async fn finish_init(&mut self) -> Notification {
        // Device type, revision, application version, SoftDevice list, then
        // the CRC; we only need the CRC, which is last.
        let [.., low, high] = self.init_packet[..] else {
            self.fail();
            return response(opcode::INIT, status::OPERATION_FAILED);
        };
        self.expected_crc = u16::from_le_bytes([low, high]);

        let mut flash = FLASH.lock().await;
        let flash = flash.as_mut().expect("set at boot");
        if let Err(e) = flash.wake().await {
            warn!("dfu: SPI flash unavailable: {}", e);
            self.fail();
            return response(opcode::INIT, status::OPERATION_FAILED);
        }
        // The image's sectors, and the trailer's (which may hold state from a
        // past update; MCUBoot must see it erased until we activate).
        let image_end = (SLOT_START + self.image_size).next_multiple_of(spi_flash::SECTOR);
        let trailer = SLOT_START + SLOT_SIZE - spi_flash::SECTOR;
        let (image, trailer) = (
            SLOT_START..image_end.min(trailer),
            trailer..trailer + spi_flash::SECTOR,
        );
        if !in_slot(image.clone()) || !in_slot(trailer.clone()) {
            warn!("dfu: an erase outside the slot; refused");
            self.fail();
            return response(opcode::INIT, status::OPERATION_FAILED);
        }
        let mut erased = flash.erase(image).await;
        if erased.is_ok() {
            erased = flash.erase(trailer).await;
        }
        if let Err(e) = erased {
            warn!("dfu: erasing the slot failed: {}", e);
            self.fail();
            return response(opcode::INIT, status::OPERATION_FAILED);
        }
        info!("dfu: slot erased; expecting CRC {:#06x}", self.expected_crc);
        self.state = State::Ready;
        response(opcode::INIT, status::SUCCESS)
    }

    async fn receive(&mut self, mut data: &[u8]) -> Option<Notification> {
        if self.received + data.len() as u32 > self.image_size {
            warn!("dfu: more data than the announced size");
            self.fail();
            return Some(response(opcode::RECEIVE, status::TOO_BIG));
        }
        // Gather whole pages: programming is per page, so this halves the
        // flash traffic compared to programming each 20-byte packet.
        while !data.is_empty() {
            let room = self.page.capacity() - self.page.len();
            let (now, later) = data.split_at(room.min(data.len()));
            self.page.extend_from_slice(now).expect("sized to fit");
            self.received += now.len() as u32;
            data = later;
            let complete = self.received == self.image_size;
            if !(self.page.is_full() || complete) {
                continue;
            }
            // The header is in the first page: something that isn't an
            // image for this bootloader (another zip's contents, say) is
            // refused now, rather than after all of it has arrived.
            if self.received <= spi_flash::PAGE
                && let Err(problem) = mcuboot::Header::parse(&self.page, self.image_size, CAPACITY)
            {
                return Some(self.refuse(opcode::RECEIVE, problem));
            }
            if let Err(e) = self.program_page().await {
                warn!("dfu: programming the SPI flash failed: {}", e);
                self.fail();
                return Some(response(opcode::RECEIVE, status::OPERATION_FAILED));
            }
        }
        PROGRESS.lock(|p| p.set(Some((self.received * 100 / self.image_size) as u8)));

        if self.received == self.image_size {
            info!("dfu: received the whole image");
            self.state = State::Received;
            return Some(response(opcode::RECEIVE, status::SUCCESS));
        }
        self.packets_since_receipt += 1;
        if self.receipt_interval != 0 && self.packets_since_receipt >= self.receipt_interval {
            self.packets_since_receipt = 0;
            let mut receipt = Vec::from_slice(&[opcode::RECEIPT]).expect("fits");
            receipt
                .extend_from_slice(&self.received.to_le_bytes())
                .expect("fits");
            return Some(receipt);
        }
        None
    }

    async fn program_page(&mut self) -> Result<(), spi_flash::Error> {
        let address = SLOT_START + self.received - self.page.len() as u32;
        let result = if in_slot(address..address + self.page.len() as u32) {
            let mut flash = FLASH.lock().await;
            flash
                .as_mut()
                .expect("set at boot")
                .program(address, &self.page)
                .await
        } else {
            warn!("dfu: a write outside the slot; refused");
            // Not the flash's fault, but it ends the update all the same.
            Err(spi_flash::Error::Timeout)
        };
        self.page.clear();
        result
    }

    /// Reads the slot back (proving what's in flash, not just what we were
    /// sent) and checks it against the init packet's CRC, and that it's an
    /// image to install (see [`mcuboot`]), in one pass.
    async fn validate(&mut self) -> Notification {
        let mut flash = FLASH.lock().await;
        let flash = flash.as_mut().expect("set at boot");
        let mut chunk = [0u8; 256];
        let header = &mut chunk[..mcuboot::HEADER_LEN];
        flash.read(SLOT_START, header).await;
        let header = match mcuboot::Header::parse(header, self.image_size, CAPACITY) {
            Ok(header) => header,
            Err(problem) => return self.refuse(opcode::VALIDATE, problem),
        };
        let mut check = mcuboot::Check::new(header, self.image_size);
        let mut crc = Crc16::new();
        let mut address = SLOT_START;
        let end = SLOT_START + self.image_size;
        while address < end {
            let chunk = &mut chunk[..(end - address).min(256) as usize];
            flash.read(address, chunk).await;
            crc.update(chunk);
            check.update(chunk);
            address += chunk.len() as u32;
        }
        let crc = crc.finish();
        if crc != self.expected_crc {
            warn!("dfu: CRC {:#06x}, expected {:#06x}", crc, self.expected_crc);
            return self.refuse(opcode::VALIDATE, Problem::Corrupt);
        }
        match check.finish() {
            // Older or newer alike (see `mcuboot::Version`): the log says
            // which, next to the running version (`firmware::version_word`)
            // at boot.
            Ok(version) => info!("dfu: image validated; version {}", version),
            Err(problem) => return self.refuse(opcode::VALIDATE, problem),
        }
        self.state = State::Validated;
        response(opcode::VALIDATE, status::SUCCESS)
    }

    /// Asks the bootloader to swap in the new image, and resets into it.
    async fn activate(&mut self) {
        // In a block of its own, so the flash is free for `reset`'s saving
        // and flushing.
        {
            let mut flash = FLASH.lock().await;
            let flash = flash.as_mut().expect("set at boot");
            let magic: [u8; 16] = BOOT_MAGIC
                .map(u32::to_le_bytes)
                .as_flattened()
                .try_into()
                .expect("16 bytes");
            if let Err(e) = flash.program(SLOT_START + SLOT_SIZE - 16, &magic).await {
                warn!("dfu: writing the swap request failed: {}", e);
                self.fail();
                return;
            }
            flash.sleep().await;
        }
        reset("installing an update").await
    }
}

/// Resets the chip on purpose, having said why, and saved what a reset
/// would lose, and then waits a moment for the last Bluetooth packets to
/// get out.
///
/// - The field log is paused during an update, and its backlog fills the
///   log's buffer, after which new lines are dropped: so it's flushed
///   first to make room for the reason, and again after, so that the
///   reason is in the field log before the reset. Otherwise the next boot's
///   reset reason ("soft") is all there is to explain it.
/// - Today's steps, and the activity the phone hasn't fetched yet, go into
///   the record that otherwise carries them across System OFF (see
///   [`crate::system_off::save_before_reset`]), for the next boot to
///   restore. The history doesn't survive any reset, and the bootloader's
///   swap can overwrite the RAM that today's steps survive resets in.
async fn reset(why: &'static str) -> ! {
    crate::field_log::flush().await;
    info!("reset: {=str}", why);
    crate::system_off::save_before_reset().await;
    crate::field_log::flush().await;
    embassy_time::Timer::after_millis(500).await;
    cortex_m::peripheral::SCB::sys_reset()
}

/// CRC-16/CCITT-FALSE (polynomial 0x1021, initial 0xFFFF), as Nordic's
/// `crc16_compute` and `scripts/package.py` compute it.
struct Crc16(u16);

impl Crc16 {
    fn new() -> Self {
        Self(0xFFFF)
    }

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.0 ^= u16::from(byte) << 8;
            for _ in 0..8 {
                self.0 = if self.0 & 0x8000 != 0 {
                    (self.0 << 1) ^ 0x1021
                } else {
                    self.0 << 1
                };
            }
        }
    }

    fn finish(self) -> u16 {
        self.0
    }
}
