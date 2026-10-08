// SPDX-License-Identifier: Apache-2.0
//! Realtime-safe single-producer single-consumer sample ring, one per track.
//!
//! The producer runs on the Core Audio callback thread: [`RingProducer::push`]
//! never blocks, locks or allocates. A block is stored as a small header
//! (frame count, rate, host time) followed by its samples, all stored as raw `u32` bit
//! patterns in one `rtrb` queue, so a block is either fully in or fully dropped. When the ring
//! is full the block is dropped and counted; the consumer sees the resulting
//! host-time jump and the resampler fills the hole with silence.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

/// Header size in `u32` slots: frames, rate, host time (2 slots).
const HEADER: usize = 4;

#[derive(Default)]
struct Counters {
    dropped_samples: AtomicU64,
    dropped_blocks: AtomicU64,
}

/// Creates a ring that can hold about `capacity_samples` samples of audio.
pub fn ring(capacity_samples: usize) -> (RingProducer, RingConsumer) {
    // Room for the samples plus headers of typical (>= 256 frame) blocks.
    let slots = capacity_samples + capacity_samples / 64 + 4 * HEADER;
    let (p, c) = RingBuffer::<u32>::new(slots);
    let counters = Arc::new(Counters::default());
    (
        RingProducer {
            inner: p,
            counters: counters.clone(),
        },
        RingConsumer {
            inner: c,
            counters,
            reported: 0,
        },
    )
}

fn join_u64(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

pub struct RingProducer {
    inner: Producer<u32>,
    counters: Arc<Counters>,
}

impl RingProducer {
    /// Pushes one block. Returns `false` (and counts the drop) when the ring
    /// cannot hold it. Realtime-safe.
    pub fn push(&mut self, samples: &[f32], sample_rate: f64, host_time_ns: u64) -> bool {
        let need = HEADER + samples.len();
        if samples.is_empty() {
            return true;
        }
        let Ok(mut chunk) = self.inner.write_chunk_uninit(need) else {
            self.counters
                .dropped_samples
                .fetch_add(samples.len() as u64, Ordering::Relaxed);
            self.counters.dropped_blocks.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let header = [
            samples.len() as u32,
            (sample_rate as f32).to_bits(),
            (host_time_ns >> 32) as u32,
            host_time_ns as u32,
        ];
        // The header goes through the same iterator as the samples.
        let (a, b) = chunk.as_mut_slices();
        let mut it = header
            .into_iter()
            .chain(samples.iter().map(|s| s.to_bits()));
        for slot in a.iter_mut().chain(b.iter_mut()) {
            if let Some(v) = it.next() {
                slot.write(v);
            }
        }
        // SAFETY: all `need` slots were initialised above.
        unsafe { chunk.commit_all() };
        true
    }

    /// Whether a block of `samples` fits now. For a producer that may wait
    /// (a replay as fast as possible): waiting is not a drop, and `push`
    /// counts every refusal as one.
    pub fn has_room(&self, samples: usize) -> bool {
        self.inner.slots() >= HEADER + samples
    }

    /// Samples dropped so far because the ring was full.
    pub fn dropped_samples(&self) -> u64 {
        self.counters.dropped_samples.load(Ordering::Relaxed)
    }
}

/// One captured block, as popped from the ring.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub sample_rate: f64,
    pub host_time_ns: u64,
    pub samples: Vec<f32>,
}

pub struct RingConsumer {
    inner: Consumer<u32>,
    counters: Arc<Counters>,
    reported: u64,
}

impl RingConsumer {
    /// Pops the next block into `out` (cleared first, capacity reused).
    /// Returns `(sample_rate, host_time_ns)`, or `None` when empty.
    pub fn pop_into(&mut self, out: &mut Vec<f32>) -> Option<(f64, u64)> {
        out.clear();
        let avail = self.inner.slots();
        if avail < HEADER {
            return None;
        }
        let chunk = self.inner.read_chunk(HEADER).ok()?;
        let (a, b) = chunk.as_slices();
        let mut h = [0u32; HEADER];
        for (d, s) in h.iter_mut().zip(a.iter().chain(b.iter())) {
            *d = *s;
        }
        chunk.commit_all();
        let frames = h[0] as usize;
        let rate = f32::from_bits(h[1]) as f64;
        let host = join_u64(h[2], h[3]);
        // The producer commits header and samples together, so they are here.
        let chunk = self.inner.read_chunk(frames).ok()?;
        let (a, b) = chunk.as_slices();
        out.extend(a.iter().chain(b.iter()).map(|&w| f32::from_bits(w)));
        chunk.commit_all();
        Some((rate, host))
    }

    /// Allocating convenience over [`pop_into`](Self::pop_into).
    pub fn pop(&mut self) -> Option<Block> {
        let mut samples = Vec::new();
        let (sample_rate, host_time_ns) = self.pop_into(&mut samples)?;
        Some(Block {
            sample_rate,
            host_time_ns,
            samples,
        })
    }

    /// Total samples dropped by the producer so far.
    pub fn dropped_samples(&self) -> u64 {
        self.counters.dropped_samples.load(Ordering::Relaxed)
    }

    /// Samples dropped since the previous call (for overrun events).
    pub fn take_new_drops(&mut self) -> u64 {
        let now = self.dropped_samples();
        let d = now - self.reported;
        self.reported = now;
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_blocks_and_stamps() {
        let (mut p, mut c) = ring(4096);
        let a: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..37).map(|i| -(i as f32)).collect();
        assert!(p.push(&a, 48_000.0, 123_456_789_012));
        assert!(p.push(&b, 44_100.0, u64::MAX - 5));
        let x = c.pop().unwrap();
        assert_eq!(x.samples, a);
        assert_eq!(x.sample_rate, 48_000.0);
        assert_eq!(x.host_time_ns, 123_456_789_012);
        let y = c.pop().unwrap();
        assert_eq!(y.samples, b);
        assert_eq!(y.sample_rate, 44_100.0);
        assert_eq!(y.host_time_ns, u64::MAX - 5);
        assert!(c.pop().is_none());
    }

    #[test]
    fn header_bits_survive() {
        // Header words are raw bit patterns; they may look like NaNs.
        let (mut p, mut c) = ring(1024);
        assert!(p.push(&[0.5; 3], 16_000.0, 0x7fc0_0001_7fc0_0002));
        assert_eq!(c.pop().unwrap().host_time_ns, 0x7fc0_0001_7fc0_0002);
    }

    #[test]
    fn overflow_drops_whole_blocks_and_counts() {
        let (mut p, mut c) = ring(1000);
        let blk = vec![1.0f32; 300];
        let mut ok = 0;
        for i in 0..10 {
            if p.push(&blk, 48_000.0, i) {
                ok += 1;
            }
        }
        assert!((3..10).contains(&ok));
        assert_eq!(p.dropped_samples(), (10 - ok) as u64 * 300);
        assert_eq!(c.take_new_drops(), (10 - ok) as u64 * 300);
        assert_eq!(c.take_new_drops(), 0);
        // What was accepted is intact and in order.
        for i in 0..ok {
            let b = c.pop().unwrap();
            assert_eq!(b.host_time_ns, i as u64);
            assert_eq!(b.samples.len(), 300);
        }
        assert!(c.pop().is_none());
        // Space is reusable after draining.
        assert!(p.push(&blk, 48_000.0, 99));
    }

    #[test]
    fn wraps_around_the_end_of_the_buffer() {
        let (mut p, mut c) = ring(512);
        for round in 0..200u64 {
            let blk: Vec<f32> = (0..97).map(|i| (round * 1000 + i) as f32).collect();
            assert!(p.push(&blk, 16_000.0, round));
            let got = c.pop().unwrap();
            assert_eq!(got.samples, blk);
            assert_eq!(got.host_time_ns, round);
        }
    }

    #[test]
    fn threaded_spsc_stress() {
        let (mut p, mut c) = ring(1 << 16);
        let t = std::thread::spawn(move || {
            let mut sent = 0u64;
            for i in 0..5000u64 {
                let blk = vec![i as f32; 64 + (i % 5) as usize];
                while !p.push(&blk, 48_000.0, i) {
                    std::thread::yield_now();
                }
                sent += 1;
            }
            sent
        });
        let mut next = 0u64;
        while next < 5000 {
            if let Some(b) = c.pop() {
                assert_eq!(b.host_time_ns, next);
                assert!(b.samples.iter().all(|&s| s == next as f32));
                next += 1;
            } else {
                std::thread::yield_now();
            }
        }
        assert_eq!(t.join().unwrap(), 5000);
    }
}
