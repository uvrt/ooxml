//! Owns: an entry's uncompressed bytes -- inflating deflated data with no more output than
//! the entry states (zip bombs), and checking its size and CRC-32 -- and deflating data for
//! a written entry (zlib's default level, as Python's zipfile writes).
//!
//! Must not: read the archive's structure (see `read`).

use crate::read::{Archive, Entry, Method};
use crate::ZipError;
use flate2::{Compress, Compression, Crc, Decompress, FlushCompress, FlushDecompress, Status};

impl Archive {
    /// An entry's uncompressed bytes, checked against its stated size and CRC-32.
    pub fn read_entry(&self, entry: &Entry) -> Result<Vec<u8>, ZipError> {
        let meta = entry.meta();
        let corrupt = |message: &str| ZipError::Corrupt {
            name: meta.name.clone(),
            message: message.to_owned(),
        };
        if meta.flags & 0x1 != 0 {
            return Err(ZipError::Unsupported(format!("{}: encrypted", meta.name)));
        }
        let stored = self.stored_data(entry);
        let size = usize::try_from(meta.size).map_err(|_| corrupt("its size does not fit this platform"))?;
        let data = match meta.method {
            Method::Stored => stored.to_vec(),
            Method::Deflated => {
                inflate(stored, size).ok_or_else(|| corrupt("its data does not inflate to its size"))?
            }
            Method::Other(m) => return Err(ZipError::Unsupported(format!("{}: compression method {m}", meta.name))),
        };
        if data.len() != size {
            return Err(corrupt("its data is not its stated size"));
        }
        let mut crc = Crc::new();
        crc.update(&data);
        if crc.sum() != meta.crc32 {
            return Err(corrupt("bad CRC-32"));
        }
        Ok(data)
    }
}

/// Raw deflate data inflated to exactly `size` bytes, or `None` (more, less, or not
/// deflate).
fn inflate(stored: &[u8], size: usize) -> Option<Vec<u8>> {
    // one byte of room past the stated size shows an entry that would inflate to more
    let mut out = Vec::with_capacity(size.saturating_add(1));
    let mut d = Decompress::new(false);
    loop {
        let consumed = usize::try_from(d.total_in()).ok()?;
        let status = d
            .decompress_vec(stored.get(consumed..)?, &mut out, FlushDecompress::Finish)
            .ok()?;
        match status {
            Status::StreamEnd => return (out.len() == size).then_some(out),
            _ if out.len() > size => return None,
            _ if out.len() == out.capacity() => out.reserve(1),
            Status::BufError if usize::try_from(d.total_in()).ok()? >= stored.len() => return None,
            _ => {}
        }
    }
}

/// `data` deflated (raw, zlib's default level 6, which zipfile's `ZIP_DEFLATED` uses).
pub(crate) fn deflate(data: &[u8]) -> Result<Vec<u8>, ZipError> {
    let mut c = Compress::new(Compression::new(6), false);
    let mut out = Vec::with_capacity(data.len() / 4 + 64);
    loop {
        let consumed = usize::try_from(c.total_in()).map_err(|_| ZipError::Unsupported("size".into()))?;
        let rest = data.get(consumed..).unwrap_or_default();
        if out.len() == out.capacity() {
            out.reserve(out.capacity().max(4096));
        }
        match c.compress_vec(rest, &mut out, FlushCompress::Finish) {
            Ok(Status::StreamEnd) => return Ok(out),
            Ok(_) => {}
            Err(e) => return Err(ZipError::Unsupported(format!("deflate: {e}"))),
        }
    }
}

/// CRC-32 of `data`.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(data);
    crc.sum()
}
