//! Owns: bounds-checked little-endian reads at an offset of a record, failing exactly where
//! the reference's `struct.unpack_from` fails (the value's whole size must lie inside), and
//! the text decodings records use (UTF-16LE and Windows-1252, bad units replaced).
//!
//! Must not: read anything without its bounds check.

use crate::error::ReadError;

/// A record's bytes.
#[derive(Debug, Clone, Copy)]
pub struct Bytes<'a>(pub &'a [u8]);

macro_rules! read_at {
    ($name:ident, $ty:ty, $n:expr) => {
        /// The value at `at`, little-endian.
        pub fn $name(&self, at: usize) -> Result<$ty, ReadError> {
            let end = at.checked_add($n).ok_or(ReadError::PastEnd)?;
            let slice = self.0.get(at..end).ok_or(ReadError::PastEnd)?;
            let mut raw = [0u8; $n];
            raw.copy_from_slice(slice);
            Ok(<$ty>::from_le_bytes(raw))
        }
    };
}

impl<'a> Bytes<'a> {
    read_at!(u8, u8, 1);
    read_at!(u16, u16, 2);
    read_at!(i16, i16, 2);
    read_at!(u32, u32, 4);
    read_at!(i32, i32, 4);
    read_at!(f32, f32, 4);

    /// `N` unsigned 32-bit values from `at`.
    pub fn u32s<const N: usize>(&self, at: usize) -> Result<[u32; N], ReadError> {
        let mut out = [0u32; N];
        for (k, slot) in out.iter_mut().enumerate() {
            *slot = self.u32(at + 4 * k)?;
        }
        Ok(out)
    }

    /// `N` signed 32-bit values from `at`.
    pub fn i32s<const N: usize>(&self, at: usize) -> Result<[i32; N], ReadError> {
        let mut out = [0i32; N];
        for (k, slot) in out.iter_mut().enumerate() {
            *slot = self.i32(at + 4 * k)?;
        }
        Ok(out)
    }

    /// The record's length.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `count` values from `at`, each read with `read` at its offset (`size` bytes apart); an
    /// empty read at an offset past the end fails too, as the reference's does.
    pub fn many<T>(
        &self,
        at: usize,
        count: usize,
        size: usize,
        read: impl Fn(&Self, usize) -> Result<T, ReadError>,
    ) -> Result<Vec<T>, ReadError> {
        if at > self.0.len() {
            return Err(ReadError::PastEnd);
        }
        let total = count.checked_mul(size).ok_or(ReadError::PastEnd)?;
        if at.checked_add(total).ok_or(ReadError::PastEnd)? > self.0.len() {
            return Err(ReadError::PastEnd);
        }
        (0..count).map(|k| read(self, at + k * size)).collect()
    }

    /// The bytes `[start, end)`, clamped to the record as a slice is.
    pub fn slice(&self, start: usize, end: usize) -> &'a [u8] {
        let len = self.0.len();
        let (s, e) = (start.min(len), end.min(len));
        self.0.get(s..e.max(s)).unwrap_or(&[])
    }
}

/// UTF-16LE, each unpaired surrogate and an odd last byte replaced by U+FFFD.
pub fn utf16le(raw: &[u8]) -> String {
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect();
    let mut out = String::from_utf16_lossy(&units);
    // An odd last byte after a lone high surrogate is one replacement with it.
    let lone_high_last = units.last().is_some_and(|u| (0xD800..=0xDBFF).contains(u));
    if raw.len() % 2 == 1 && !lone_high_last {
        out.push('\u{FFFD}');
    }
    out
}

/// Windows-1252's characters for 0x80 to 0x9F; `None` for the five it leaves undefined.
const CP1252_HIGH: [Option<char>; 32] = [
    Some('\u{20AC}'),
    None,
    Some('\u{201A}'),
    Some('\u{0192}'),
    Some('\u{201E}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02C6}'),
    Some('\u{2030}'),
    Some('\u{0160}'),
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    Some('\u{017D}'),
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02DC}'),
    Some('\u{2122}'),
    Some('\u{0161}'),
    Some('\u{203A}'),
    Some('\u{0153}'),
    None,
    Some('\u{017E}'),
    Some('\u{0178}'),
];

/// Windows-1252, its undefined bytes replaced by U+FFFD.
pub fn cp1252(raw: &[u8]) -> String {
    raw.iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH
                .get(usize::from(b - 0x80))
                .copied()
                .flatten()
                .unwrap_or('\u{FFFD}'),
            _ => char::from(b),
        })
        .collect()
}

/// Whether the reference's `str.isspace` holds for `ch`.
pub fn is_space(ch: char) -> bool {
    ch.is_whitespace() || ('\u{1C}'..='\u{1F}').contains(&ch)
}

/// `text` without leading and trailing whitespace, as the reference's `str.strip()`.
pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fail_past_the_end() {
        let b = Bytes(&[1, 0, 0, 0, 2]);
        assert_eq!(b.u32(0), Ok(1));
        assert_eq!(b.u32(2), Err(ReadError::PastEnd));
        assert!(b.many(6, 0, 4, Bytes::u32).is_err());
        assert_eq!(b.many(5, 0, 4, Bytes::u32), Ok(vec![]));
    }

    #[test]
    fn decodings() {
        assert_eq!(cp1252(&[0x41, 0x80, 0x81]), "A\u{20AC}\u{FFFD}");
        assert_eq!(utf16le(&[0x41, 0, 0x00, 0xD8, 0x42]), "A\u{FFFD}");
        assert_eq!(utf16le(&[0x41]), "\u{FFFD}");
        assert_eq!(strip("\u{1F} a \u{A0}"), "a");
    }
}
