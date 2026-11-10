//! The PineTime's external 4 MiB SPI NOR flash.
//!
//! PINE64 has fitted different chips over time (XTX XT25F32B per the wiki;
//! the dev kit has a Boya BY25Q32, JEDEC ID `68 40 16`), all with the same
//! standard command set, so any 32 Mbit chip is accepted.
//!
//! Its layout is the bootloader's (see `memory.x` for the internal side):
//!
//!   0x000000  the boot logo and a recovery firmware (InfiniTime's), 256 KiB
//!   0x040000  MCUBoot's secondary slot: where updates land, and where the
//!             previous image goes after a swap (464 KiB)
//!   0x0B4000  free (InfiniTime keeps its file system here)
//!   0x1FD000  what's saved before System OFF (12 KiB)
//!   0x200000  our field log (2 MiB)
//!
//! [`crate::dfu`] writes updates into the secondary slot,
//! [`crate::system_off`] saves unsynced activity before switching the watch
//! off, and [`crate::field_log`] keeps logs in the top 2 MiB.
//!
//! The flash shares SPIM2 with the LCD (see [`crate::spi`]). Every operation
//! here locks the bus only for the length of one command, so the screen keeps
//! updating between them; erases (up to seconds) are polled with the bus
//! released.
//!
//! Between uses the chip sits in deep power-down, where it draws ~1 µA
//! instead of ~25 µA in standby.

use embassy_nrf::gpio::Output;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Timer};

use crate::spi;

// The standard SPI NOR commands (e.g. XT25F32B datasheet section 8).
const WRITE_ENABLE: u8 = 0x06;
const READ_STATUS: u8 = 0x05;
const READ_DATA: u8 = 0x03;
const PAGE_PROGRAM: u8 = 0x02;
const SECTOR_ERASE: u8 = 0x20;
const BLOCK_ERASE: u8 = 0xD8;
const READ_JEDEC_ID: u8 = 0x9F;
const DEEP_POWER_DOWN: u8 = 0xB9;
const RELEASE_POWER_DOWN: u8 = 0xAB;

/// Status register bit 0: an erase or program is still running.
const WRITE_IN_PROGRESS: u8 = 0x01;

pub const PAGE: u32 = 256;
pub const SECTOR: u32 = 4096;
pub const BLOCK: u32 = 64 * 1024;
/// The chip's size: 32 Mbit (which [`SpiFlash::wake`] checks it is). It
/// decodes 22 bits of address, so one past this lands back at the start.
pub const SIZE: u32 = 4 * 1024 * 1024;

#[derive(Debug, defmt::Format)]
pub enum Error {
    /// The chip didn't finish an erase or program within its datasheet
    /// maximum (with margin).
    Timeout,
    /// The JEDEC ID wasn't a 32 Mbit SPI NOR (or nothing answered); nothing
    /// is written to an unknown chip.
    UnknownChip([u8; 3]),
}

/// The chip, set at boot. Users ([`crate::dfu`], [`crate::field_log`]) hold
/// the lock for a whole operation, and wake the chip first: whoever used it
/// last may have put it to sleep.
pub static FLASH: Mutex<CriticalSectionRawMutex, Option<SpiFlash>> = Mutex::new(None);

pub struct SpiFlash {
    cs: Output<'static>,
    /// Whether the chip is out of deep power-down. In deep power-down it
    /// ignores everything but the wake-up command, *silently*: a program
    /// sent to a sleeping chip is lost, and its status reads as idle. Since
    /// several users share the chip and each sleeps it when done, every
    /// operation wakes it first if needed.
    awake: bool,
}

impl SpiFlash {
    /// Takes the chip-select (P0.05) and puts the chip to sleep. The
    /// bootloader may have left it awake.
    pub async fn new(cs: Output<'static>) -> Self {
        // Awake as far as `power` knows too, so `sleep` balances it.
        crate::power::on(crate::power::Meter::SpiFlash);
        let mut flash = Self { cs, awake: true };
        flash.sleep().await;
        flash
    }

    /// Wakes the chip from deep power-down and checks it's the one we know.
    pub async fn wake(&mut self) -> Result<(), Error> {
        // Woken below even if it's awake: `power` must hear it's off first,
        // or it counts the chip as awake twice over from here.
        if self.awake {
            crate::power::off(crate::power::Meter::SpiFlash);
        }
        self.awake = false;
        self.ensure_awake().await;
        let mut id = [0; 3];
        self.command(&[READ_JEDEC_ID], &mut id).await;
        // Any manufacturer (0x00/0xFF: nothing answered), SPI NOR (0x40),
        // 32 Mbit (2^0x16 bytes).
        if matches!(id[0], 0x00 | 0xFF) || id[1..] != [0x40, 0x16] {
            return Err(Error::UnknownChip(id));
        }
        Ok(())
    }

    pub async fn sleep(&mut self) {
        self.command(&[DEEP_POWER_DOWN], &mut []).await;
        if self.awake {
            crate::power::off(crate::power::Meter::SpiFlash);
        }
        self.awake = false;
    }

    async fn ensure_awake(&mut self) {
        if !self.awake {
            self.command(&[RELEASE_POWER_DOWN], &mut []).await;
            // tRES1: at most 20 µs before it accepts commands again.
            Timer::after_micros(30).await;
            self.awake = true;
            crate::power::on(crate::power::Meter::SpiFlash);
        }
    }

    pub async fn read(&mut self, address: u32, data: &mut [u8]) {
        self.ensure_awake().await;
        let [_, a2, a1, a0] = address.to_be_bytes();
        self.command(&[READ_DATA, a2, a1, a0], data).await;
    }

    /// Erases `range`, which must be sector-aligned: whole 64 KiB blocks
    /// where it covers them (one block erase is ~5× faster than its 16
    /// sector erases), single 4 KiB sectors at the edges.
    pub async fn erase(&mut self, range: core::ops::Range<u32>) -> Result<(), Error> {
        assert!(
            range.start.is_multiple_of(SECTOR)
                && range.end.is_multiple_of(SECTOR)
                && range.end <= SIZE,
            "erase ranges are sector-aligned"
        );
        self.ensure_awake().await;
        let mut address = range.start;
        while address < range.end {
            let [_, a2, a1, a0] = address.to_be_bytes();
            if address.is_multiple_of(BLOCK) && address + BLOCK <= range.end {
                self.command(&[WRITE_ENABLE], &mut []).await;
                self.command(&[BLOCK_ERASE, a2, a1, a0], &mut []).await;
                // Datasheet maximum: 1.2 s.
                self.wait_idle(Duration::from_millis(2000)).await?;
                address += BLOCK;
            } else {
                self.command(&[WRITE_ENABLE], &mut []).await;
                self.command(&[SECTOR_ERASE, a2, a1, a0], &mut []).await;
                // Datasheet maximum: 300 ms.
                self.wait_idle(Duration::from_millis(500)).await?;
                address += SECTOR;
            }
        }
        Ok(())
    }

    /// Programs `data` at `address`; the range must lie within one 256-byte
    /// page (the chip wraps around within the page otherwise), and be erased.
    pub async fn program(&mut self, address: u32, data: &[u8]) -> Result<(), Error> {
        assert!(
            !data.is_empty() && address % PAGE + data.len() as u32 <= PAGE && address < SIZE,
            "a program stays within one page"
        );
        self.ensure_awake().await;
        let [_, a2, a1, a0] = address.to_be_bytes();
        let header = [PAGE_PROGRAM, a2, a1, a0];
        self.command(&[WRITE_ENABLE], &mut []).await;
        {
            // Header and data in one chip-select cycle, but two DMA writes,
            // to avoid copying the data into a combined buffer.
            let mut bus = spi::BUS.get().await.lock().await;
            self.cs.set_low();
            spi::write(&mut bus, &header).await;
            spi::write(&mut bus, data).await;
            self.cs.set_high();
        }
        // Datasheet maximum: 2.4 ms.
        self.wait_idle(Duration::from_millis(10)).await
    }

    /// Programs `data` at `address`, across page boundaries as needed; the
    /// range must be erased.
    pub async fn write(&mut self, mut address: u32, mut data: &[u8]) -> Result<(), Error> {
        while !data.is_empty() {
            let room = (PAGE - address % PAGE) as usize;
            let (now, later) = data.split_at(room.min(data.len()));
            self.program(address, now).await?;
            address += now.len() as u32;
            data = later;
        }
        Ok(())
    }

    /// Polls the status register until an erase/program finishes. The bus is
    /// free between polls.
    async fn wait_idle(&mut self, timeout: Duration) -> Result<(), Error> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut status = [0];
            self.command(&[READ_STATUS], &mut status).await;
            if status[0] & WRITE_IN_PROGRESS == 0 {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(Error::Timeout);
            }
            Timer::after_millis(1).await;
        }
    }

    /// One command: `command` out, then `response.len()` bytes in, within a
    /// single chip-select cycle.
    async fn command(&mut self, command: &[u8], response: &mut [u8]) {
        let mut bus = spi::BUS.get().await.lock().await;
        self.cs.set_low();
        spi::write(&mut bus, command).await;
        if !response.is_empty() {
            spi::read(&mut bus, response).await;
        }
        self.cs.set_high();
    }
}
