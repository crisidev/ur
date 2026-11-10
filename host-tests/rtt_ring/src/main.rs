//! Host tests for the firmware's `rtt_ring` module: which bytes of
//! defmt's RTT buffer the field log reads. `cargo test` runs them.

// Used only by the tests (and the proofs).
#[path = "../../../src/rtt_ring.rs"]
#[allow(dead_code)]
mod rtt_ring;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::rtt_ring::*;

    /// A ring as defmt-rtt keeps one, with `field_log`'s side of it:
    /// `frames` and `consume`, over a `Vec` instead of raw pointers.
    struct Ring {
        buffer: Vec<u8>,
        read: usize,
        write: usize,
    }

    impl Ring {
        fn new(size: usize, at: usize) -> Self {
            Self {
                buffer: vec![0xEE; size],
                read: at,
                write: at,
            }
        }

        /// As defmt-rtt writes: up to one short of the read cursor.
        fn log(&mut self, bytes: &[u8]) {
            let size = self.buffer.len();
            assert!(waiting(size, self.read, self.write) + bytes.len() < size);
            for &byte in bytes {
                self.buffer[self.write] = byte;
                self.write = (self.write + 1) % size;
            }
        }

        /// What `field_log::frames` gives: the two pieces, joined.
        fn frames(&self, room: usize) -> Option<Vec<u8>> {
            let size = self.buffer.len();
            let waiting = waiting(size, self.read, self.write);
            let fits = whole_frames(size, self.read, waiting, room, |at| self.buffer[at])?;
            let (first, second) = pieces(size, self.read, fits);
            let mut frames = self.buffer[self.read..self.read + first].to_vec();
            frames.extend(&self.buffer[..second]);
            Some(frames)
        }

        fn consume(&mut self, n: usize) {
            self.read = after(self.buffer.len(), self.read, n);
        }
    }

    #[test]
    fn nothing_waits_in_an_empty_ring() {
        let ring = Ring::new(16, 5);
        assert_eq!(waiting(16, 5, 5), 0);
        assert_eq!(ring.frames(usize::MAX), None);
    }

    #[test]
    fn only_whole_frames_are_taken() {
        let mut ring = Ring::new(16, 0);
        ring.log(&[1, 2, 0, 3, 0, 4, 5]);
        assert_eq!(ring.frames(usize::MAX), Some(vec![1, 2, 0, 3, 0]));
        // Room for the first only; for neither (which isn't "none").
        assert_eq!(ring.frames(4), Some(vec![1, 2, 0]));
        assert_eq!(ring.frames(3), Some(vec![1, 2, 0]));
        assert_eq!(ring.frames(2), Some(vec![]));
        // A frame still being written is no frame yet.
        ring.consume(5);
        assert_eq!(ring.frames(usize::MAX), None);
        ring.log(&[0]);
        assert_eq!(ring.frames(usize::MAX), Some(vec![4, 5, 0]));
    }

    #[test]
    fn frames_are_followed_around_the_end() {
        let mut ring = Ring::new(8, 6);
        ring.log(&[1, 2, 3, 0, 4, 0]);
        assert_eq!((ring.read, ring.write), (6, 4));
        assert_eq!(waiting(8, 6, 4), 6);
        assert_eq!(pieces(8, 6, 6), (2, 4));
        assert_eq!(ring.frames(usize::MAX), Some(vec![1, 2, 3, 0, 4, 0]));
        assert_eq!(ring.frames(5), Some(vec![1, 2, 3, 0]));
        ring.consume(4);
        assert_eq!(ring.read, 2);
        assert_eq!(ring.frames(usize::MAX), Some(vec![4, 0]));
        ring.consume(2);
        assert_eq!(ring.read, ring.write);
    }

    /// Logging and draining in turn, with frames of every length the
    /// ring takes, wherever the cursors are: what comes out is what went
    /// in, in order, in whole frames.
    #[test]
    fn what_is_logged_is_what_is_read() {
        let mut ring = Ring::new(32, 0);
        let (mut logged, mut read) = (Vec::new(), Vec::new());
        for round in 0..200usize {
            let frame: Vec<u8> = (0..round % 13)
                .map(|i| 1 + ((round + i) % 250) as u8)
                .chain([0])
                .collect();
            ring.log(&frame);
            logged.extend(&frame);
            // Now and then, with little room: a frame at a time.
            let room = if round % 3 == 0 { 14 } else { usize::MAX };
            if let Some(frames) = ring.frames(room) {
                assert!(frames.len() <= room);
                assert!(frames.last().is_none_or(|&last| last == 0));
                ring.consume(frames.len());
                read.extend(frames);
            }
        }
        while let Some(frames) = ring.frames(usize::MAX) {
            ring.consume(frames.len());
            read.extend(frames);
        }
        assert_eq!(read, logged);
    }
}

/// Proofs, for `cargo kani`: what the tests above check for some rings,
/// for every ring. `field_log` makes slices from raw pointers by these
/// numbers, so they're what stands between a slip in the arithmetic and
/// reading memory that isn't the buffer's.
#[cfg(kani)]
mod proofs {
    use super::rtt_ring::*;

    /// A ring of any size up to 2^31 bytes (the control block's sizes
    /// are 32 bits; defmt-rtt's buffer is 1 KiB), and cursors in it.
    fn any_ring() -> (usize, usize, usize) {
        let (size, read, write): (usize, usize, usize) = kani::any();
        kani::assume(0 < size && size <= 1 << 31);
        kani::assume(read < size && write < size);
        (size, read, write)
    }

    /// For every ring: fewer bytes wait than the ring is long, none
    /// when the cursors meet, and reading all of them brings the read
    /// cursor to the write cursor (so the ring is empty again); reading
    /// any fewer leaves it inside the ring, with the rest still waiting.
    #[kani::proof]
    fn what_waits_is_between_the_cursors() {
        let (size, read, write) = any_ring();
        let waits = waiting(size, read, write);
        assert!(waits < size);
        assert_eq!(waits == 0, read == write);
        assert_eq!(after(size, read, waits), write);
        let n: usize = kani::any();
        kani::assume(n <= waits);
        let next = after(size, read, n);
        assert!(next < size);
        assert_eq!(waiting(size, next, write), waits - n);
    }

    /// For every ring, and any count of the waiting bytes: the two
    /// pieces are that many bytes in all; the first runs from the read
    /// cursor and ends within the buffer; the second, if there is one,
    /// starts where the first reached the buffer's end, and stops at or
    /// before the write cursor. So both of `field_log::frames`' slices
    /// lie inside the buffer, over bytes defmt-rtt has written and won't
    /// touch until they're consumed.
    #[kani::proof]
    fn the_pieces_are_inside_the_buffer() {
        let (size, read, write) = any_ring();
        let fits: usize = kani::any();
        kani::assume(fits <= waiting(size, read, write));
        let (first, second) = pieces(size, read, fits);
        assert_eq!(first + second, fits);
        assert!(read + first <= size);
        // An empty first piece means nothing fits: what `field_log`'s
        // writer takes as its cue to start the next sector.
        assert_eq!(first == 0, fits == 0);
        if second > 0 {
            assert!(read + first == size && second <= write && write < read);
        } else if read <= write {
            assert!(read + first <= write);
        }
    }

    /// The longest ring the next proof walks.
    const SIZE: usize = 8;

    /// For every ring of up to [`SIZE`] bytes, holding any bytes, and
    /// any room: the scan only looks at bytes between the cursors (it
    /// never reads what defmt-rtt hasn't written); there are whole
    /// frames exactly when a zero byte waits; and then what fits is no
    /// more than the room or what waits, ends in a frame's end (or is
    /// nothing), and leaves out no frame that ends within the room.
    #[kani::proof]
    #[kani::unwind(9)]
    fn whole_frames_are_whole_and_fit() {
        let buffer: [u8; SIZE] = kani::any();
        let (size, read, write) = any_ring();
        kani::assume(size <= SIZE);
        let room: usize = kani::any();
        let waits = waiting(size, read, write);
        // The byte `i` places after the read cursor.
        let nth = |i: usize| buffer[(read + i) % size];
        let byte = |at: usize| {
            assert!(at < size);
            assert!(waiting(size, read, at) < waits);
            buffer[at]
        };
        let fits = whole_frames(size, read, waits, room, byte);

        let i: usize = kani::any();
        kani::assume(i < waits);
        match fits {
            None => assert!(nth(i) != 0),
            Some(fits) => {
                assert!(fits <= waits && fits <= room);
                assert!(fits == 0 || nth(fits - 1) == 0);
                // Any frame that ends within the room is within what fits.
                if nth(i) == 0 && i < room {
                    assert!(i < fits);
                }
            }
        }
    }
}
