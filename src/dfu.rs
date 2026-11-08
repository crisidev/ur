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
//! 5. Validate (`04`): we read the slot back and check its CRC.
//! 6. Activate (`05`): we mark the slot for the bootloader, save what a
//!    reset would lose (see [`reset`]), and reset. The bootloader swaps the
//!    slots and boots the update on probation (see
//!    [`crate::firmware::confirm`]).
//!
//! Only bonded, authenticated (passkey-paired) connections may use the
//! service: otherwise anyone within Bluetooth range could install whatever
//! they like, since the bootloader checks a hash but not a signature.

use core::cell::Cell;

use defmt::{info, warn};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use heapless::Vec;

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
/// MCUBoot's trailer magic (`boot_img_magic` in MCUBoot 1.5), as bytes:
/// written to the last 16 bytes of the secondary slot, it asks for a swap.
/// Its "image OK" flag stays unset, so the swap is a test: the new image
/// must confirm itself or the next reset swaps back.
const BOOT_MAGIC: [u32; 4] = [0xf395c277, 0x7fefd260, 0x0f505235, 0x8079b62c];
/// The image header's magic, checked before we'll activate anything.
const IMAGE_MAGIC: u32 = 0x96f3b83d;

/// How far an update has got (percent), while one is in progress; the UI
/// shows it.
pub static PROGRESS: BlockingMutex<CriticalSectionRawMutex, Cell<Option<u8>>> =
    BlockingMutex::new(Cell::new(None));

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
        }
    }

    /// Whether an update is under way (for the UI, and to keep the phone's
    /// other traffic out of the way).
    pub fn active(&self) -> bool {
        self.state != State::Idle
    }

    /// Handles a control point write; returns the notification to send, if
    /// any. Activation doesn't return: it resets the chip.
    pub async fn on_control(&mut self, request: &[u8]) -> Option<Notification> {
        let (&op, arguments) = request.split_first()?;
        match (op, self.state) {
            (opcode::START, _) => {
                // A client may restart at any point (e.g. after a failed
                // attempt on the same connection).
                if arguments.first() != Some(&APPLICATION) {
                    return Some(response(op, status::NOT_SUPPORTED));
                }
                *self = Self::new();
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

    /// Called when the client disconnects.
    pub async fn abandon(&mut self) {
        if self.active() {
            warn!("dfu: client left mid-update ({}); abandoned", self.state);
            self.fail();
            if let Some(flash) = FLASH.lock().await.as_mut() {
                flash.sleep().await;
            }
        }
    }

    fn fail(&mut self) {
        *self = Self::new();
        PROGRESS.lock(|p| p.set(None));
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
        if application == 0 || application > SLOT_SIZE - TRAILER_SIZE {
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
        let mut erased = flash.erase(SLOT_START..image_end.min(trailer)).await;
        if erased.is_ok() {
            erased = flash.erase(trailer..trailer + spi_flash::SECTOR).await;
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
            if (self.page.is_full() || complete)
                && let Err(e) = self.program_page().await
            {
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
        let mut flash = FLASH.lock().await;
        let result = flash
            .as_mut()
            .expect("set at boot")
            .program(address, &self.page)
            .await;
        self.page.clear();
        result
    }

    /// Reads the slot back (proving what's in flash, not just what we were
    /// sent) and checks it against the init packet's CRC, and that it's an
    /// MCUBoot image at all.
    async fn validate(&mut self) -> Notification {
        let mut flash = FLASH.lock().await;
        let flash = flash.as_mut().expect("set at boot");
        let mut crc = Crc16::new();
        let mut chunk = [0u8; 256];
        let mut magic = 0;
        let mut address = SLOT_START;
        let end = SLOT_START + self.image_size;
        while address < end {
            let chunk = &mut chunk[..(end - address).min(256) as usize];
            flash.read(address, chunk).await;
            if address == SLOT_START {
                magic = u32::from_le_bytes(chunk[..4].try_into().expect("a full first chunk"));
            }
            crc.update(chunk);
            address += chunk.len() as u32;
        }
        let crc = crc.finish();
        if crc != self.expected_crc {
            warn!("dfu: CRC {:#06x}, expected {:#06x}", crc, self.expected_crc);
            self.fail();
            return response(opcode::VALIDATE, status::CRC_ERROR);
        }
        if magic != IMAGE_MAGIC {
            warn!("dfu: not an MCUBoot image (magic {:#010x})", magic);
            self.fail();
            return response(opcode::VALIDATE, status::OPERATION_FAILED);
        }
        info!("dfu: image validated");
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
