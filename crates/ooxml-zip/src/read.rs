//! Owns: reading an archive's structure -- the end of central directory record (ZIP64's
//! too), the central directory, and each entry's local record -- within [`ZipLimits`], with
//! the checks Python's zipfile makes (local and central names agree, entries do not
//! overlap) and the zip-bomb limits it does not.
//!
//! Must not: inflate (see `inflate`), or write.

use crate::{cp437, ZipError};
use ooxml_base::ZipLimits;
use std::ops::Range;
use std::sync::Arc;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
const END64_SIG: u32 = 0x0606_4b50;
const END64_LOCATOR_SIG: u32 = 0x0706_4b50;
const DESCRIPTOR_SIG: u32 = 0x0807_4b50;

/// How an entry's data is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// As it is (0).
    Stored,
    /// Deflated (8).
    Deflated,
    /// Another method, by its number (raw copy only).
    Other(u16),
}

impl Method {
    /// The method's number in a header.
    pub fn code(self) -> u16 {
        match self {
            Method::Stored => 0,
            Method::Deflated => 8,
            Method::Other(c) => c,
        }
    }

    fn from_code(code: u16) -> Method {
        match code {
            0 => Method::Stored,
            8 => Method::Deflated,
            c => Method::Other(c),
        }
    }
}

/// What the central directory says about an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMeta {
    /// The name, decoded (UTF-8 when flagged so, else code page 437).
    pub name: String,
    /// The name's bytes as stored.
    pub raw_name: Box<[u8]>,
    /// "Version made by": the creating system (high byte) and specification version.
    pub version_made_by: u16,
    /// "Version needed to extract".
    pub version_needed: u16,
    /// General-purpose flags.
    pub flags: u16,
    /// The compression method.
    pub method: Method,
    /// MS-DOS time.
    pub dos_time: u16,
    /// MS-DOS date.
    pub dos_date: u16,
    /// CRC-32 of the uncompressed data.
    pub crc32: u32,
    /// Size of the stored data.
    pub compressed_size: u64,
    /// Size of the uncompressed data.
    pub size: u64,
    /// Internal file attributes.
    pub internal_attr: u16,
    /// External file attributes.
    pub external_attr: u32,
    /// The central record's extra field.
    pub extra: Box<[u8]>,
    /// The entry's comment.
    pub comment: Box<[u8]>,
}

impl EntryMeta {
    /// Whether this is a directory entry (its name ends with `/`).
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
}

/// An entry: its metadata and where its records are in the archive.
#[derive(Debug, Clone)]
pub struct Entry {
    meta: EntryMeta,
    /// The whole local record: header, name, extra, data and any data descriptor.
    local: Range<usize>,
    /// The stored data within the archive.
    data: Range<usize>,
    /// The central directory record.
    central: Range<usize>,
}

impl Entry {
    /// The metadata.
    pub fn meta(&self) -> &EntryMeta {
        &self.meta
    }
    pub(crate) fn local(&self) -> Range<usize> {
        self.local.clone()
    }
    pub(crate) fn central(&self) -> Range<usize> {
        self.central.clone()
    }
}

/// A read archive: its bytes (shared, never copied) and its entries in central-directory
/// order.
#[derive(Debug, Clone)]
pub struct Archive {
    bytes: Arc<[u8]>,
    entries: Vec<Entry>,
    comment: Range<usize>,
}

/// Little-endian fields read from a slice, refusing to read past its end.
struct Fields<'a> {
    s: &'a [u8],
    at: usize,
}

impl<'a> Fields<'a> {
    fn new(s: &'a [u8], at: usize) -> Self {
        Fields { s, at }
    }
    fn take<const N: usize>(&mut self) -> Result<[u8; N], ZipError> {
        let b = self
            .s
            .get(self.at..self.at + N)
            .and_then(|b| <[u8; N]>::try_from(b).ok())
            .ok_or_else(|| malformed("a record runs past the end of the archive"))?;
        self.at += N;
        Ok(b)
    }
    fn u16(&mut self) -> Result<u16, ZipError> {
        self.take::<2>().map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Result<u32, ZipError> {
        self.take::<4>().map(u32::from_le_bytes)
    }
    fn u64(&mut self) -> Result<u64, ZipError> {
        self.take::<8>().map(u64::from_le_bytes)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], ZipError> {
        let b = self
            .s
            .get(self.at..self.at + n)
            .ok_or_else(|| malformed("a field runs past the end of the archive"))?;
        self.at += n;
        Ok(b)
    }
}

fn malformed(message: &str) -> ZipError {
    ZipError::Malformed(message.to_owned())
}

fn to_usize(v: u64) -> Result<usize, ZipError> {
    usize::try_from(v).map_err(|_| malformed("an offset does not fit this platform"))
}

/// Where the central directory is, and how many entries it holds.
struct Directory {
    offset: u64,
    size: u64,
    count: u64,
    comment: Range<usize>,
}

impl Archive {
    /// Read an archive's structure within `limits`. Nothing is inflated.
    pub fn read(bytes: Arc<[u8]>, limits: &ZipLimits) -> Result<Archive, ZipError> {
        let dir = end_of_directory(&bytes)?;
        if dir.count > limits.max_entries as u64 {
            return Err(ZipError::Limit {
                what: "entries",
                limit: limits.max_entries as u64,
            });
        }
        let mut entries = Vec::with_capacity(to_usize(dir.count)?.min(1 << 16));
        let mut at = to_usize(dir.offset)?;
        let end = at
            .checked_add(to_usize(dir.size)?)
            .ok_or_else(|| malformed("central directory size"))?;
        let mut total: u64 = 0;
        for _ in 0..dir.count {
            let entry = central_record(&bytes, at)?;
            check_limits(&entry.meta, limits, &mut total)?;
            at = entry.central.end;
            entries.push(entry);
        }
        if at > end {
            return Err(malformed("the central directory is longer than its stated size"));
        }
        check_overlaps(&entries, dir.offset)?;
        Ok(Archive {
            bytes,
            entries,
            comment: dir.comment,
        })
    }

    /// The entries, in central-directory order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The archive's bytes.
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    /// The archive comment.
    pub fn comment(&self) -> &[u8] {
        self.bytes.get(self.comment.clone()).unwrap_or_default()
    }

    /// An entry's stored (compressed) data.
    pub fn stored_data(&self, entry: &Entry) -> &[u8] {
        self.bytes.get(entry.data.clone()).unwrap_or_default()
    }
}

fn check_limits(meta: &EntryMeta, limits: &ZipLimits, total: &mut u64) -> Result<(), ZipError> {
    if meta.size > limits.max_entry_size {
        return Err(ZipError::Limit {
            what: "uncompressed size of an entry",
            limit: limits.max_entry_size,
        });
    }
    *total = total.saturating_add(meta.size);
    if *total > limits.max_total_size {
        return Err(ZipError::Limit {
            what: "uncompressed size of all entries",
            limit: limits.max_total_size,
        });
    }
    if meta.size > limits.ratio_floor && meta.size / meta.compressed_size.max(1) > limits.max_ratio {
        return Err(ZipError::Limit {
            what: "compression ratio of an entry",
            limit: limits.max_ratio,
        });
    }
    Ok(())
}

/// Local records must not overlap one another or the central directory (zip bombs that
/// share data between entries).
fn check_overlaps(entries: &[Entry], directory: u64) -> Result<(), ZipError> {
    let mut spans: Vec<Range<usize>> = entries.iter().map(|e| e.local.clone()).collect();
    spans.sort_by_key(|r| r.start);
    let directory = to_usize(directory)?;
    for pair in spans.windows(2) {
        if let [a, b] = pair {
            if a.end > b.start {
                return Err(malformed("overlapped entries"));
            }
        }
    }
    if spans.last().is_some_and(|r| r.end > directory) {
        return Err(malformed("an entry overlaps the central directory"));
    }
    Ok(())
}

fn end_of_directory(bytes: &[u8]) -> Result<Directory, ZipError> {
    // the record is 22 bytes plus a comment of at most 65,535
    let floor = bytes.len().saturating_sub(22 + 65_535);
    let at = (floor..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| bytes.get(i..i + 4) == Some(&END_SIG.to_le_bytes()[..]))
        .ok_or_else(|| malformed("no end of central directory record"))?;
    let mut f = Fields::new(bytes, at + 4);
    let (disk, dir_disk) = (f.u16()?, f.u16()?);
    let (_count_here, count) = (f.u16()?, f.u16()?);
    let (size, offset) = (f.u32()?, f.u32()?);
    let comment_len = usize::from(f.u16()?);
    let comment = at + 22..(at + 22 + comment_len).min(bytes.len());
    if disk != 0 || dir_disk != 0 {
        return Err(ZipError::Unsupported("archives split over several disks".into()));
    }
    let mut dir = Directory {
        offset: u64::from(offset),
        size: u64::from(size),
        count: u64::from(count),
        comment,
    };
    if count == 0xFFFF || size == u32::MAX || offset == u32::MAX {
        zip64_directory(bytes, at, &mut dir)?;
    }
    Ok(dir)
}

fn zip64_directory(bytes: &[u8], end_at: usize, dir: &mut Directory) -> Result<(), ZipError> {
    let locator = end_at.checked_sub(20).ok_or_else(|| malformed("no ZIP64 locator"))?;
    let mut f = Fields::new(bytes, locator);
    if f.u32()? != END64_LOCATOR_SIG {
        return Ok(());
    }
    let _disk = f.u32()?;
    let mut g = Fields::new(bytes, to_usize(f.u64()?)?);
    if g.u32()? != END64_SIG {
        return Err(malformed("the ZIP64 end of central directory record is missing"));
    }
    let _record_size = g.u64()?;
    let (_made_by, _needed, _disk, _dir_disk) = (g.u16()?, g.u16()?, g.u32()?, g.u32()?);
    let (_count_here, count, size, offset) = (g.u64()?, g.u64()?, g.u64()?, g.u64()?);
    *dir = Directory {
        offset,
        size,
        count,
        comment: dir.comment.clone(),
    };
    Ok(())
}

/// The central record at `at`, its local record located and checked.
fn central_record(bytes: &[u8], at: usize) -> Result<Entry, ZipError> {
    let mut f = Fields::new(bytes, at);
    if f.u32()? != CENTRAL_SIG {
        return Err(malformed("a central directory record has a bad signature"));
    }
    let (version_made_by, version_needed, flags, method) = (f.u16()?, f.u16()?, f.u16()?, f.u16()?);
    let (dos_time, dos_date, crc32) = (f.u16()?, f.u16()?, f.u32()?);
    let (mut compressed_size, mut size) = (u64::from(f.u32()?), u64::from(f.u32()?));
    let (name_len, extra_len, comment_len) = (usize::from(f.u16()?), usize::from(f.u16()?), usize::from(f.u16()?));
    let (_disk, internal_attr, external_attr) = (f.u16()?, f.u16()?, f.u32()?);
    let mut local_offset = u64::from(f.u32()?);
    let raw_name: Box<[u8]> = f.bytes(name_len)?.into();
    let extra: Box<[u8]> = f.bytes(extra_len)?.into();
    let comment: Box<[u8]> = f.bytes(comment_len)?.into();
    zip64_extra(&extra, &mut size, &mut compressed_size, &mut local_offset)?;
    let name = if flags & 0x800 != 0 {
        String::from_utf8(raw_name.to_vec()).map_err(|_| malformed("an entry name flagged UTF-8 is not"))?
    } else {
        cp437::decode(&raw_name)
    };
    let meta = EntryMeta {
        name,
        raw_name,
        version_made_by,
        version_needed,
        flags,
        method: Method::from_code(method),
        dos_time,
        dos_date,
        crc32,
        compressed_size,
        size,
        internal_attr,
        external_attr,
        extra,
        comment,
    };
    let (local, data) = local_record(bytes, to_usize(local_offset)?, &meta)?;
    Ok(Entry {
        meta,
        local,
        data,
        central: at..f.at,
    })
}

/// The ZIP64 extended information extra field's values for the fields stored as 0xFFFFFFFF.
fn zip64_extra(extra: &[u8], size: &mut u64, compressed: &mut u64, offset: &mut u64) -> Result<(), ZipError> {
    let mut f = Fields::new(extra, 0);
    while f.at + 4 <= extra.len() {
        let (id, len) = (f.u16()?, usize::from(f.u16()?));
        let body = f.bytes(len)?;
        if id != 0x0001 {
            continue;
        }
        let mut g = Fields::new(body, 0);
        for field in [size, compressed, offset] {
            if *field == u64::from(u32::MAX) {
                *field = g.u64()?;
            }
        }
        break;
    }
    Ok(())
}

/// The local record of `meta` at `at`: its whole span and its data's span.
fn local_record(bytes: &[u8], at: usize, meta: &EntryMeta) -> Result<(Range<usize>, Range<usize>), ZipError> {
    let mut f = Fields::new(bytes, at);
    if f.u32()? != LOCAL_SIG {
        return Err(malformed("a local header has a bad signature"));
    }
    f.at += 22;
    let (name_len, extra_len) = (usize::from(f.u16()?), usize::from(f.u16()?));
    if f.bytes(name_len)? != &*meta.raw_name {
        return Err(malformed("an entry's local and central names differ"));
    }
    f.at += extra_len;
    let start = f.at;
    let end = start
        .checked_add(to_usize(meta.compressed_size)?)
        .ok_or_else(|| malformed("entry size"))?;
    if end > bytes.len() {
        return Err(malformed("an entry's data is cut short"));
    }
    let mut record_end = end;
    if meta.flags & 0x8 != 0 {
        // a data descriptor: optional signature, CRC-32, then two sizes (8 bytes each with ZIP64)
        let mut d = Fields::new(bytes, end);
        let signed = d.u32()? == DESCRIPTOR_SIG;
        let sizes = if meta.extra_has_zip64() { 16 } else { 8 };
        record_end = end + usize::from(signed) * 4 + 4 + sizes;
        if record_end > bytes.len() {
            return Err(malformed("an entry's data descriptor is cut short"));
        }
    }
    Ok((at..record_end, start..end))
}

impl EntryMeta {
    fn extra_has_zip64(&self) -> bool {
        let mut f = Fields::new(&self.extra, 0);
        while let (Ok(id), Ok(len)) = (f.u16(), f.u16()) {
            if id == 0x0001 {
                return true;
            }
            f.at += usize::from(len);
        }
        false
    }
}
