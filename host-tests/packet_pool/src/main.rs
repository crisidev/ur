//! Host tests for the firmware's `packet_pool` module: the Bluetooth
//! host's packet buffers. `cargo test` runs them.
//!
//! The pool hands out raw pointers into one static, on the promise that
//! each buffer has one owner at a time. These tests hold it to that; run
//! under Miri (`cargo +nightly miri test`), they also check that no two
//! references to a buffer ever overlap.

// Used only by the tests.
#[path = "../../../src/packet_pool.rs"]
#[allow(dead_code)]
mod packet_pool;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use trouble_host::PacketPool;

    use super::packet_pool::{Packet, Pool};

    /// The pool is one static: the tests take turns with it.
    static POOL: Mutex<()> = Mutex::new(());

    /// Every buffer there is, or as many as are free.
    fn all() -> Vec<Packet> {
        std::iter::from_fn(Pool::allocate).collect()
    }

    fn address(packet: &Packet) -> usize {
        packet.as_ref().as_ptr() as usize
    }

    #[test]
    fn the_pool_is_as_big_as_the_host_expects() {
        let _turn = POOL.lock().unwrap_or_else(|e| e.into_inner());
        // As the firmware's features size it (`main` sizes the
        // controller's buffers by the same MTU).
        assert_eq!(Pool::MTU, 251);
        assert_eq!(Pool::capacity(), 16);
        let packets = all();
        assert_eq!(packets.len(), Pool::capacity());
        assert!(Pool::allocate().is_none(), "no more than its capacity");
    }

    #[test]
    fn each_packet_has_a_buffer_of_its_own() {
        let _turn = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let mut packets = all();
        // Each a whole MTU, none overlapping another: `MTU` bytes apart
        // at least.
        let mut starts: Vec<usize> = packets.iter().map(address).collect();
        starts.sort_unstable();
        assert!(starts.windows(2).all(|pair| pair[1] - pair[0] >= Pool::MTU));
        for (i, packet) in packets.iter_mut().enumerate() {
            assert_eq!(packet.as_ref().len(), Pool::MTU);
            packet.as_mut().fill(i as u8 + 1);
        }
        // What one wrote, no other overwrote.
        for (i, packet) in packets.iter().enumerate() {
            assert!(packet.as_ref().iter().all(|&byte| byte == i as u8 + 1));
        }
    }

    #[test]
    fn a_dropped_packets_buffer_comes_back_zeroed() {
        let _turn = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let mut packets = all();
        for packet in &mut packets {
            packet.as_mut().fill(0xA5);
        }
        // One back: the next allocation is that buffer, and no other,
        // with nothing of the last packet in it.
        let freed = packets.swap_remove(5);
        let freed_at = address(&freed);
        drop(freed);
        let again = Pool::allocate().expect("one is free");
        assert_eq!(address(&again), freed_at);
        assert!(again.as_ref().iter().all(|&byte| byte == 0));
        assert!(Pool::allocate().is_none());
        // The others are untouched by its zeroing.
        for packet in &packets {
            assert!(packet.as_ref().iter().all(|&byte| byte == 0xA5));
        }
    }

    /// Through many rounds of taking and giving back, in a mixed order:
    /// the buffers handed out are always distinct, and never more or
    /// fewer than the pool has free, so none is leaked and none is given
    /// to two packets.
    #[test]
    fn nothing_is_leaked_or_handed_out_twice() {
        let _turn = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let mut held: Vec<Packet> = Vec::new();
        // A small generator, so the order is mixed but the same each run.
        let mut state = 0x2545_F491u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..2_000 {
            if next() % 3 != 0 {
                match Pool::allocate() {
                    Some(packet) => held.push(packet),
                    None => assert_eq!(held.len(), Pool::capacity(), "none only when all are out"),
                }
            } else if !held.is_empty() {
                let which = next() as usize % held.len();
                drop(held.swap_remove(which));
            }
            assert!(held.len() <= Pool::capacity());
            let distinct: HashSet<usize> = held.iter().map(address).collect();
            assert_eq!(distinct.len(), held.len(), "a buffer has one packet");
        }
        drop(held);
        assert_eq!(all().len(), Pool::capacity(), "all of them back");
    }
}
