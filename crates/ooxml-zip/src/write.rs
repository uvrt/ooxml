//! Owns: writing an archive -- an entry copied raw (its local record byte for byte, its
//! central record with only its offset changed), or a new entry written as Python's zipfile
//! writes one with `writestr` (local header without extra field or data descriptor,
//! versions 20, UTF-8 flag only for a non-ASCII name, `rw-------` when no attributes) --
//! then the end of central directory record.
//!
//! Must not: write ZIP64 records (an archive that would need them is refused; the limits
//! keep archives far below), or change a raw-copied entry's bytes.

use crate::inflate::{crc32, deflate};
use crate::read::{Archive, Entry, Method};
use crate::ZipError;

/// What a new entry is written with (Python's `ZipInfo` fields that a save sets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    /// The name.
    pub name: String,
    /// Stored or deflated.
    pub method: Method,
    /// MS-DOS time.
    pub dos_time: u16,
    /// MS-DOS date.
    pub dos_date: u16,
    /// External attributes; 0 is written as `rw-------` (`0o600 << 16`), as zipfile does.
    pub external_attr: u32,
    /// The creating system (the high byte of "version made by").
    pub create_system: u8,
}

/// An archive being written.
#[derive(Debug, Default)]
pub struct Writer {
    out: Vec<u8>,
    central: Vec<u8>,
    count: usize,
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn small<T: TryFrom<usize>>(n: usize, what: &str) -> Result<T, ZipError> {
    T::try_from(n).map_err(|_| ZipError::Unsupported(format!("{what} needs ZIP64")))
}

impl Writer {
    /// An empty archive.
    pub fn new() -> Writer {
        Writer::default()
    }

    /// Copy `entry` of `archive` as it is stored: the local record's bytes unchanged, the
    /// central record's with the new offset.
    pub fn copy(&mut self, archive: &Archive, entry: &Entry) -> Result<(), ZipError> {
        let offset: u32 = small(self.out.len(), "an offset")?;
        let local = archive
            .bytes()
            .get(entry.local())
            .ok_or_else(|| ZipError::Malformed("entry bounds".into()))?;
        let central = archive
            .bytes()
            .get(entry.central())
            .ok_or_else(|| ZipError::Malformed("entry bounds".into()))?;
        self.out.extend_from_slice(local);
        let start = self.central.len();
        self.central.extend_from_slice(central);
        // the offset field is at byte 42 of a central record; 0xFFFFFFFF means ZIP64's
        if let Some(field) = self.central.get_mut(start + 42..start + 46) {
            if field == u32::MAX.to_le_bytes() {
                return Err(ZipError::Unsupported(
                    "copying an entry whose offset is in a ZIP64 field".into(),
                ));
            }
            field.copy_from_slice(&offset.to_le_bytes());
        }
        self.count += 1;
        Ok(())
    }

    /// Add an entry of `data` (compressed here when `entry.method` is deflate).
    pub fn add(&mut self, entry: &NewEntry, data: &[u8]) -> Result<(), ZipError> {
        let stored;
        let body: &[u8] = match entry.method {
            Method::Stored => data,
            Method::Deflated => {
                stored = deflate(data)?;
                &stored
            }
            Method::Other(m) => return Err(ZipError::Unsupported(format!("writing compression method {m}"))),
        };
        let offset: u32 = small(self.out.len(), "an offset")?;
        let (size, compressed): (u32, u32) = (
            small(data.len(), "an entry's size")?,
            small(body.len(), "an entry's size")?,
        );
        let name = entry.name.as_bytes();
        let name_len: u16 = small(name.len(), "a name")?;
        let flags: u16 = if entry.name.is_ascii() { 0 } else { 0x800 };
        let crc = crc32(data);
        let external = if entry.external_attr == 0 {
            0o600 << 16
        } else {
            entry.external_attr
        };
        // local header
        let o = &mut self.out;
        put32(o, 0x0403_4b50);
        put16(o, 20);
        for v in [flags, entry.method.code(), entry.dos_time, entry.dos_date] {
            put16(o, v);
        }
        for v in [crc, compressed, size] {
            put32(o, v);
        }
        put16(o, name_len);
        put16(o, 0);
        o.extend_from_slice(name);
        o.extend_from_slice(body);
        // central record
        let c = &mut self.central;
        put32(c, 0x0201_4b50);
        put16(c, u16::from(entry.create_system) << 8 | 20);
        for v in [20, flags, entry.method.code(), entry.dos_time, entry.dos_date] {
            put16(c, v);
        }
        for v in [crc, compressed, size] {
            put32(c, v);
        }
        for v in [name_len, 0, 0, 0, 0] {
            put16(c, v);
        }
        put32(c, external);
        put32(c, offset);
        c.extend_from_slice(name);
        self.count += 1;
        Ok(())
    }

    /// The archive's bytes, with `comment` as the archive comment.
    pub fn finish(mut self, comment: &[u8]) -> Result<Vec<u8>, ZipError> {
        let offset: u32 = small(self.out.len(), "the central directory's offset")?;
        let size: u32 = small(self.central.len(), "the central directory's size")?;
        let count: u16 = small(self.count, "the entry count")?;
        let comment_len: u16 = small(comment.len(), "the archive comment")?;
        self.out.append(&mut self.central);
        let o = &mut self.out;
        put32(o, 0x0605_4b50);
        for v in [0, 0, count, count] {
            put16(o, v);
        }
        put32(o, size);
        put32(o, offset);
        put16(o, comment_len);
        o.extend_from_slice(comment);
        Ok(self.out)
    }
}
