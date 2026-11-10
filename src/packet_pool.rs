//! The Bluetooth host's packet buffers: trouble-host's default pool, but
//! starting out all zeros, so that it costs no flash.
//!
//! trouble-host's `DefaultPacketPool` marks each of its buffers `free:
//! true` from the start. That makes its static (16 × 251 bytes) not all
//! zeros, so it lands in `.data`, whose initial image sits in flash for the
//! startup code to copy into RAM: 4 KB of flash, nearly all zeros. This
//! pool is the same (as many buffers, as big, handed out first-free and
//! zeroed) but marks buffers *in use* instead, so it starts all zeros, in
//! `.bss`. It takes the same RAM.
//!
//! TODO: upstream the inversion to trouble-host's pool, and drop this.

use core::cell::{Cell, UnsafeCell};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// The `default-packet-pool-*` features still size the pool: the host
// derives other limits from the MTU feature, which must agree with ours.
use trouble_host::config::{DEFAULT_PACKET_POOL_MTU as MTU, DEFAULT_PACKET_POOL_SIZE as SIZE};

/// The buffers. A buffer belongs to whichever [`Packet`] holds its index,
/// which [`USED`] hands out once at a time.
struct Buffers(UnsafeCell<[[u8; MTU]; SIZE]>);

// SAFETY: Each buffer is only touched through the one `Packet` that owns
// it (see `Pool::allocate`).
unsafe impl Sync for Buffers {}

static BUFFERS: Buffers = Buffers(UnsafeCell::new([[0; MTU]; SIZE]));
static USED: Mutex<CriticalSectionRawMutex, [Cell<bool>; SIZE]> =
    Mutex::new([const { Cell::new(false) }; SIZE]);

/// The pool, for trouble-host's types (`Stack<'_, _, Pool>`, …).
pub struct Pool;

impl trouble_host::PacketPool for Pool {
    type Packet = Packet;
    const MTU: usize = MTU;

    fn capacity() -> usize {
        SIZE
    }

    fn allocate() -> Option<Packet> {
        let index = USED.lock(|used| {
            let index = used.iter().position(|used| !used.get())?;
            used[index].set(true);
            Some(index)
        })?;
        let mut packet = Packet { index };
        // As trouble-host's pool does: no stale data from an earlier packet.
        packet.as_mut().fill(0);
        Some(packet)
    }
}

/// One buffer, until dropped.
pub struct Packet {
    index: usize,
}

impl Packet {
    fn buffer(&self) -> *mut u8 {
        // SAFETY: `index` < SIZE (from `allocate`), so this stays inside
        // `BUFFERS`.
        unsafe { BUFFERS.0.get().cast::<[u8; MTU]>().add(self.index).cast() }
    }
}

impl trouble_host::Packet for Packet {}

impl AsRef<[u8]> for Packet {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: This packet owns the buffer at `index` until it drops.
        unsafe { core::slice::from_raw_parts(self.buffer(), MTU) }
    }
}

impl AsMut<[u8]> for Packet {
    fn as_mut(&mut self) -> &mut [u8] {
        // SAFETY: As in `as_ref`, and `&mut self` makes this the only
        // reference.
        unsafe { core::slice::from_raw_parts_mut(self.buffer(), MTU) }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        USED.lock(|used| {
            // A packet is the only holder of its index (it's neither
            // `Clone` nor `Copy`), so its buffer is still marked. Were it
            // not, it had been handed to a second packet already, and
            // the two would write over each other.
            debug_assert!(used[self.index].get(), "a packet's buffer is in use");
            used[self.index].set(false);
        });
    }
}
