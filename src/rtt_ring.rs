//! The arithmetic of reading defmt's RTT buffer in place: a ring of
//! `size` bytes, written at one cursor (by defmt-rtt) and read at another
//! (by [`crate::field_log`], in field builds).
//!
//! Apart from the buffer itself, which `field_log` reaches through raw
//! pointers: what's here decides which bytes those pointers read, and how
//! long the slices made from them are. So it's hardware-free, with host
//! tests and proofs (`host-tests/rtt_ring`) that every index stays inside
//! the buffer, between the cursors.
//!
//! Each is `#[inline(always)]`: so `field_log`'s code is what it was with
//! this written out in it (the field build's `.text` is the same size),
//! where left to itself the compiler made it 8 bytes bigger.
//!
//! Both cursors are below `size` (SEGGER's convention: the writer wraps
//! its own, and [`after`] ours), and the ring is full at `size - 1` bytes,
//! so that equal cursors mean empty.

/// How many bytes are waiting: from the read cursor up to the write
/// cursor, around the end.
#[inline(always)]
pub fn waiting(size: usize, read: usize, write: usize) -> usize {
    debug_assert!(read < size && write < size, "cursors are in the ring");
    (write + size - read) % size
}

/// How many of the `waiting` bytes after the read cursor are whole frames
/// that fit in `room` bytes, given the byte at each place in the ring
/// (`byte`, which is only asked for places between the cursors). Frames
/// end in a zero byte; anything after the last one is a frame still being
/// written. `None` if no whole frame is waiting; 0 if the first doesn't
/// fit in `room`.
///
/// A plain loop over the ring, rather than iterator adapters over its two
/// pieces: ~250 bytes less flash.
#[inline(always)]
pub fn whole_frames(
    size: usize,
    read: usize,
    waiting: usize,
    room: usize,
    byte: impl Fn(usize) -> u8,
) -> Option<usize> {
    debug_assert!(read < size && waiting < size, "no more waits than fits");
    let mut whole = false;
    let mut fits = 0;
    for i in 0..waiting {
        if byte((read + i) % size) == 0 {
            whole = true;
            if i >= room {
                break;
            }
            fits = i + 1;
        }
    }
    whole.then_some(fits)
}

/// The first `fits` bytes after the read cursor, as the lengths of the (at
/// most two) pieces of the buffer they're in: from the read cursor to the
/// buffer's end at most, then on from its start.
#[inline(always)]
pub fn pieces(size: usize, read: usize, fits: usize) -> (usize, usize) {
    debug_assert!(read < size && fits < size, "no more fits than waits");
    let in_first = fits.min(size - read);
    (in_first, fits - in_first)
}

/// The read cursor once `n` bytes after it are read.
#[inline(always)]
pub fn after(size: usize, read: usize, n: usize) -> usize {
    (read + n) % size
}
