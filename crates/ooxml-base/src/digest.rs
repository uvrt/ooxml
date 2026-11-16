//! Owns: content digests -- 128-bit keyed hashes of bytes, what content-keyed caches
//! (`ooxml-derive`'s memo, layout's measures) key on.
//!
//! The function is SipHash-2-4 with 128-bit output (Aumasson and Bernstein), written here
//! so the base layer needs no dependency. It is a keyed PRF: with a key the input's author
//! does not know, finding two inputs with one digest is infeasible; with a known key it is
//! not. A session that keys caches on untrusted content takes a [`DigestKey`] from a random
//! source (a leaf crate's job: this crate does no I/O); tests may use a fixed one.
//!
//! Must not: be used where a cryptographic hash is required (signatures, storage keys across
//! users).

use std::fmt;

/// The 128-bit key of a [`Digester`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DigestKey {
    k0: u64,
    k1: u64,
}

impl DigestKey {
    /// A key from its two halves (little-endian words of the 16 key bytes).
    pub const fn new(k0: u64, k1: u64) -> Self {
        DigestKey { k0, k1 }
    }
}

/// A 128-bit content digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Digest(pub [u8; 16]);

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({self})")
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl Digest {
    /// The digest of `bytes` under `key`.
    pub fn of(key: DigestKey, bytes: &[u8]) -> Digest {
        let mut d = Digester::new(key);
        d.update(bytes);
        d.finish()
    }
}

/// An incremental SipHash-2-4-128: feed bytes with [`Digester::update`], then
/// [`Digester::finish`]. Feeding in pieces gives the digest of their concatenation.
#[derive(Debug, Clone)]
pub struct Digester {
    v: [u64; 4],
    tail: u64,
    tail_len: usize,
    len: u64,
}

impl Digester {
    /// A digester under `key`.
    pub fn new(key: DigestKey) -> Self {
        Digester {
            v: [
                key.k0 ^ 0x736f_6d65_7073_6575,
                key.k1 ^ 0x646f_7261_6e64_6f6d ^ 0xee,
                key.k0 ^ 0x6c79_6765_6e65_7261,
                key.k1 ^ 0x7465_6462_7974_6573,
            ],
            tail: 0,
            tail_len: 0,
            len: 0,
        }
    }

    fn round(v: &mut [u64; 4]) {
        let [mut v0, mut v1, mut v2, mut v3] = *v;
        v0 = v0.wrapping_add(v1);
        v1 = v1.rotate_left(13) ^ v0;
        v0 = v0.rotate_left(32);
        v2 = v2.wrapping_add(v3);
        v3 = v3.rotate_left(16) ^ v2;
        v0 = v0.wrapping_add(v3);
        v3 = v3.rotate_left(21) ^ v0;
        v2 = v2.wrapping_add(v1);
        v1 = v1.rotate_left(17) ^ v2;
        v2 = v2.rotate_left(32);
        *v = [v0, v1, v2, v3];
    }

    fn compress(&mut self, m: u64) {
        self.v[3] ^= m;
        Self::round(&mut self.v);
        Self::round(&mut self.v);
        self.v[0] ^= m;
    }

    /// Feed more bytes.
    pub fn update(&mut self, bytes: &[u8]) {
        self.len = self.len.wrapping_add(bytes.len() as u64);
        let mut rest = bytes;
        while let Some((&b, more)) = rest.split_first() {
            if self.tail_len == 0 {
                if let Some((word, after)) = rest.split_first_chunk::<8>() {
                    self.compress(u64::from_le_bytes(*word));
                    rest = after;
                    continue;
                }
            }
            self.tail |= u64::from(b) << (8 * self.tail_len);
            self.tail_len += 1;
            if self.tail_len == 8 {
                let m = self.tail;
                self.compress(m);
                self.tail = 0;
                self.tail_len = 0;
            }
            rest = more;
        }
    }

    /// Feed a length-prefixed field, so that fields cannot run into each other
    /// (`["ab", "c"]` and `["a", "bc"]` differ).
    pub fn field(&mut self, bytes: &[u8]) {
        self.update(&(bytes.len() as u64).to_le_bytes());
        self.update(bytes);
    }

    /// The digest of everything fed.
    pub fn finish(mut self) -> Digest {
        let b = ((self.len & 0xff) << 56) | self.tail;
        self.compress(b);
        self.v[2] ^= 0xee;
        for _ in 0..4 {
            Self::round(&mut self.v);
        }
        let lo = self.v[0] ^ self.v[1] ^ self.v[2] ^ self.v[3];
        self.v[1] ^= 0xdd;
        for _ in 0..4 {
            Self::round(&mut self.v);
        }
        let hi = self.v[0] ^ self.v[1] ^ self.v[2] ^ self.v[3];
        let mut out = [0u8; 16];
        for (o, b) in out.iter_mut().zip(lo.to_le_bytes().into_iter().chain(hi.to_le_bytes())) {
            *o = b;
        }
        Digest(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_key() -> DigestKey {
        // the reference vectors' key: bytes 00..0f
        DigestKey::new(0x0706_0504_0302_0100, 0x0f0e_0d0c_0b0a_0908)
    }

    #[test]
    fn matches_the_reference_vectors() {
        // vectors_sip128 from the SipHash reference implementation: messages 00, 00 01, ...
        assert_eq!(
            Digest::of(reference_key(), b"").to_string(),
            "a3817f04ba25a8e66df67214c7550293"
        );
        let fifteen: Vec<u8> = (0..15).collect();
        assert_eq!(
            Digest::of(reference_key(), &fifteen).to_string(),
            "5493e99933b0a8117e08ec0f97cfc3d9"
        );
    }

    #[test]
    fn pieces_digest_as_the_whole() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let whole = Digest::of(reference_key(), &data);
        for cut in [0, 1, 7, 8, 9, 63, 500, 999] {
            let mut d = Digester::new(reference_key());
            d.update(&data[..cut]);
            d.update(&data[cut..]);
            assert_eq!(d.finish(), whole, "cut at {cut}");
        }
    }

    #[test]
    fn fields_do_not_run_together() {
        let mut a = Digester::new(reference_key());
        a.field(b"ab");
        a.field(b"c");
        let mut b = Digester::new(reference_key());
        b.field(b"a");
        b.field(b"bc");
        assert_ne!(a.finish(), b.finish());
    }
}
