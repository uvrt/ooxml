//! Owns: where the chunking's random signatures come from ([`RandomSource`]) and the
//! production source ([`OsRandom`]).
//!
//! Must not: be used for secrets. A random signature only has to differ from every other
//! writer's (MS-FSSHTTPD: random signatures never match), not to be unpredictable.

use std::hash::{BuildHasher, Hasher, RandomState};

/// Fills random signatures. Any `FnMut(&mut [u8])` is one, so a test passes a closure that
/// writes fixed bytes.
pub trait RandomSource {
    /// Fill `out` with random bytes.
    fn fill(&mut self, out: &mut [u8]);
}

impl<F: FnMut(&mut [u8])> RandomSource for F {
    fn fill(&mut self, out: &mut [u8]) {
        self(out);
    }
}

/// Random bytes from keys the operating system's random source seeds (the standard
/// library's `RandomState`: SipHash-1-3 keyed per process), hashed with a counter. Distinct
/// across processes and calls; no dependency beyond `std`.
#[derive(Debug, Clone, Default)]
pub struct OsRandom {
    keys: RandomState,
    counter: u64,
}

impl OsRandom {
    /// A source with fresh keys.
    pub fn new() -> Self {
        OsRandom::default()
    }
}

impl RandomSource for OsRandom {
    fn fill(&mut self, out: &mut [u8]) {
        for piece in out.chunks_mut(8) {
            let mut hasher = self.keys.build_hasher();
            hasher.write_u64(self.counter);
            self.counter = self.counter.wrapping_add(1);
            let bytes = hasher.finish().to_le_bytes();
            for (slot, b) in piece.iter_mut().zip(bytes) {
                *slot = b;
            }
        }
    }
}
