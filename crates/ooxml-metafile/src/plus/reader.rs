//! Owns: reading one EMF+ record's data front to back -- integers, finite floats, counts that
//! must fit what follows, points (floats, 16-bit integers, or relative 7- and 15-bit
//! steps) and rectangles -- failing where the reference's reader fails.
//!
//! Must not: interpret what it reads.

use crate::bytes::Bytes;
use crate::error::ReadError;

/// A cursor over a record's data.
#[derive(Debug, Clone)]
pub(crate) struct Data<'a> {
    pub b: &'a [u8],
    pub at: usize,
}

impl<'a> Data<'a> {
    pub fn new(b: &'a [u8]) -> Data<'a> {
        Data { b, at: 0 }
    }

    pub fn left(&self) -> usize {
        self.b.len().saturating_sub(self.at)
    }

    pub fn u8(&mut self) -> Result<u8, ReadError> {
        let v = Bytes(self.b).u8(self.at)?;
        self.at += 1;
        Ok(v)
    }

    pub fn i16(&mut self) -> Result<i16, ReadError> {
        let v = Bytes(self.b).i16(self.at)?;
        self.at += 2;
        Ok(v)
    }

    pub fn u16(&mut self) -> Result<u16, ReadError> {
        let v = Bytes(self.b).u16(self.at)?;
        self.at += 2;
        Ok(v)
    }

    pub fn u32(&mut self) -> Result<u32, ReadError> {
        let v = Bytes(self.b).u32(self.at)?;
        self.at += 4;
        Ok(v)
    }

    pub fn i32(&mut self) -> Result<i32, ReadError> {
        let v = Bytes(self.b).i32(self.at)?;
        self.at += 4;
        Ok(v)
    }

    /// `N` unsigned values read at once (the whole group must fit).
    pub fn u32s<const N: usize>(&mut self) -> Result<[u32; N], ReadError> {
        let v = Bytes(self.b).u32s::<N>(self.at)?;
        self.at += 4 * N;
        Ok(v)
    }

    /// A float that must be finite.
    pub fn f32(&mut self) -> Result<f64, ReadError> {
        let v = f64::from(Bytes(self.b).f32(self.at)?);
        self.at += 4;
        if !v.is_finite() {
            return Err(ReadError::NotFinite);
        }
        Ok(v)
    }

    /// `n` floats that must all be finite.
    pub fn floats(&mut self, n: usize) -> Result<Vec<f64>, ReadError> {
        if n.checked_mul(4).is_none_or(|bytes| self.at + bytes > self.b.len()) {
            return Err(ReadError::PastEnd);
        }
        let out = Bytes(self.b).many(self.at, n, 4, Bytes::f32)?;
        self.at += 4 * n;
        let out: Vec<f64> = out.into_iter().map(f64::from).collect();
        if out.iter().any(|v| !v.is_finite()) {
            return Err(ReadError::NotFinite);
        }
        Ok(out)
    }

    /// `N` floats that must all be finite.
    pub fn floats_n<const N: usize>(&mut self) -> Result<[f64; N], ReadError> {
        let v = self.floats(N)?;
        <[f64; N]>::try_from(v).map_err(|_| ReadError::PastEnd)
    }

    /// `n` bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], ReadError> {
        let end = self.at.checked_add(n).ok_or(ReadError::PastEnd)?;
        let out = self.b.get(self.at..end).ok_or(ReadError::PastEnd)?;
        self.at = end;
        Ok(out)
    }

    /// `n` bytes from a signed size: a negative size fails as the reference's does.
    pub fn bytes_signed(&mut self, n: i32) -> Result<&'a [u8], ReadError> {
        let n = usize::try_from(n).map_err(|_| ReadError::PastEnd)?;
        self.bytes(n)
    }

    /// A count of items of `size` bytes, which must fit what is left.
    pub fn count(&mut self, size: usize) -> Result<usize, ReadError> {
        let n = self.u32()? as usize;
        if n.checked_mul(size).is_none_or(|b| b > self.left()) {
            return Err(ReadError::CountPastEnd);
        }
        Ok(n)
    }

    /// `EmfPlusInteger7` or `EmfPlusInteger15`.
    fn int_r(&mut self) -> Result<f64, ReadError> {
        let first = i32::from(self.u8()?);
        if first & 0x80 != 0 {
            let second = i32::from(self.u8()?);
            let value = ((first & 0x7F) << 8) | second;
            return Ok(f64::from(if value & 0x4000 != 0 { value - 0x8000 } else { value }));
        }
        Ok(f64::from(if first & 0x40 != 0 { first - 0x80 } else { first }))
    }

    /// `count` points: floats, 16-bit integers (`compressed`), or relative steps.
    pub fn points(&mut self, count: usize, compressed: bool, relative: bool) -> Result<Vec<(f64, f64)>, ReadError> {
        if relative {
            let (mut x, mut y) = (0.0, 0.0);
            let mut out = Vec::with_capacity(count.min(self.left()));
            for _ in 0..count {
                x += self.int_r()?;
                y += self.int_r()?;
                out.push((x, y));
            }
            return Ok(out);
        }
        let flat = self.numbers(2 * count, compressed)?;
        Ok(flat.as_chunks::<2>().0.iter().map(|[x, y]| (*x, *y)).collect())
    }

    fn numbers(&mut self, n: usize, compressed: bool) -> Result<Vec<f64>, ReadError> {
        if compressed {
            if n.checked_mul(2).is_none_or(|b| b > self.left()) {
                return Err(ReadError::PastEnd);
            }
            (0..n).map(|_| self.i16().map(f64::from)).collect()
        } else {
            self.floats(n)
        }
    }

    /// `count` rectangles (`x, y, width, height`): floats or 16-bit integers.
    pub fn rects(&mut self, count: usize, compressed: bool) -> Result<Vec<[f64; 4]>, ReadError> {
        let flat = self.numbers(4 * count, compressed)?;
        Ok(flat.as_chunks::<4>().0.to_vec())
    }
}
