//! Owns: the inputs an application registers for a session (design phase 3, section 5;
//! ooxml-edit's `Session.add_blob` and `limits.py`, ported exactly): each checked before
//! any library sees it -- its size, what its first bytes say it is against what the caller
//! declared ([`sniff`], [`check_declared_type`]), an image's decoded size read from its
//! header ([`image_size`], [`check_image`]), a ZIP's central directory for the signs of a
//! zip bomb ([`check_package`]) -- and kept under a handle (`b1`) within the session's total
//! ([`BlobStore`]); a blob looked up by a tool ([`BlobStore::blob`]).
//!
//! Nothing is decompressed to check it.
//!
//! Must not: open a blob as a document, or decode it.

use ooxml_base::pyfmt::repr_str;
use ooxml_base::ZipLimits;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use serde_json::{Map, Value};
use std::sync::Arc;

const MB: u64 = 1024 * 1024;

/// The limits on what an application registers (ooxml-edit's `Limits`' inputs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobLimits {
    /// One blob's bytes.
    pub max_blob_bytes: u64,
    /// Every blob of the session together.
    pub max_total_blob_bytes: u64,
    /// One image's bytes.
    pub max_image_bytes: u64,
    /// One image's decoded pixels.
    pub max_image_pixels: u64,
    /// A text blob's characters.
    pub max_markdown_chars: usize,
    /// One package's bytes.
    pub max_package_bytes: u64,
    /// One package's parts.
    pub max_parts: usize,
    /// One package's parts expanded, together.
    pub max_uncompressed_bytes: u64,
    /// How many times a part over 1 MB uncompressed may expand.
    pub max_compression_ratio: u64,
}

impl Default for BlobLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 50 * MB,
            max_total_blob_bytes: 100 * MB,
            max_image_bytes: 20 * MB,
            max_image_pixels: 40_000_000,
            max_markdown_chars: 200_000,
            max_package_bytes: 50 * MB,
            max_parts: 2000,
            max_uncompressed_bytes: 500 * MB,
            max_compression_ratio: 200,
        }
    }
}

/// Bytes the application registered that break a limit or are not what they claim: raised
/// to the application, which registered them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LimitError(pub String);

fn limit<T>(message: String) -> Result<T, LimitError> {
    Err(LimitError(message))
}

/// First bytes and the type they announce.
const MAGIC: [(&[u8], &str); 11] = [
    (b"\x89PNG\r\n\x1a\n", "image/png"),
    (b"\xff\xd8\xff", "image/jpeg"),
    (b"GIF87a", "image/gif"),
    (b"GIF89a", "image/gif"),
    (b"BM", "image/bmp"),
    (b"II*\x00", "image/tiff"),
    (b"MM\x00*", "image/tiff"),
    (b"PK\x03\x04", "application/zip"),
    (b"PK\x05\x06", "application/zip"),
    (b"%PDF-", "application/pdf"),
    (b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", "application/x-ole-storage"),
];

/// The text types, by a name's extension.
const TEXT_TYPES: [(&str, &str); 6] = [
    (".md", "text/markdown"),
    (".markdown", "text/markdown"),
    (".txt", "text/plain"),
    (".csv", "text/csv"),
    (".json", "application/json"),
    (".svg", "image/svg+xml"),
];

/// The type `data` is, from its first bytes; text types from the name's extension.
#[must_use]
pub fn sniff(data: &[u8], name: &str) -> &'static str {
    for (magic, mime) in MAGIC {
        if data.starts_with(magic) {
            return mime;
        }
    }
    if data.get(..4) == Some(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        return "image/webp";
    }
    if matches!(data.get(..4), Some(b"\x01\x00\x00\x00" | b"\xd7\xcd\xc6\x9a")) {
        return if data.get(40..44) == Some(b" EMF") {
            "image/x-emf"
        } else {
            "image/x-wmf"
        };
    }
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| format!(".{}", ext.to_lowercase()))
        .unwrap_or_default();
    match TEXT_TYPES.iter().find(|(ext, _)| *ext == extension) {
        Some((_, mime)) if std::str::from_utf8(data).is_ok() => mime,
        _ => "application/octet-stream",
    }
}

/// The blob's type: what its bytes say, which must agree with what the caller declared. A
/// declared ZIP-based type (an Office package) is accepted for ZIP bytes; a declared type
/// the bytes cannot confirm is refused, so a renamed executable is not an image.
///
/// # Errors
/// A declared type the bytes are not.
pub fn check_declared_type(data: &[u8], name: &str, declared: Option<&str>) -> Result<String, LimitError> {
    let found = sniff(data, name);
    let Some(declared) = declared else {
        return Ok(found.to_owned());
    };
    let agrees = declared == found
        || (found == "application/zip"
            && (declared.ends_with("+zip")
                || declared.contains("officedocument")
                || declared.starts_with("application/vnd.")))
        || (found == "image/x-emf" && matches!(declared, "image/emf" | "image/x-emf"))
        || (found == "image/x-wmf" && matches!(declared, "image/wmf" | "image/x-wmf"));
    if agrees {
        return Ok(declared.to_owned());
    }
    let label = if name.is_empty() { "the blob" } else { name };
    limit(format!("{label} is declared {declared} but its bytes are {found}"))
}

fn u16_at(data: &[u8], at: usize, big: bool) -> Option<u32> {
    let b: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(u32::from(if big {
        u16::from_be_bytes(b)
    } else {
        u16::from_le_bytes(b)
    }))
}

fn u32_at(data: &[u8], at: usize, big: bool) -> Option<u32> {
    let b: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(if big {
        u32::from_be_bytes(b)
    } else {
        u32::from_le_bytes(b)
    })
}

fn le_at(data: &[u8], at: usize, len: usize) -> Option<u32> {
    let bytes = data.get(at..at + len)?;
    Some(bytes.iter().rev().fold(0u32, |n, b| (n << 8) | u32::from(*b)))
}

/// An image's pixel size from its header (PNG, JPEG, GIF, BMP, WebP), or `None`.
#[must_use]
pub fn image_size(data: &[u8]) -> Option<(u64, u64)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.get(12..16) == Some(b"IHDR") {
        return Some((u64::from(u32_at(data, 16, true)?), u64::from(u32_at(data, 20, true)?)));
    }
    if matches!(data.get(..6), Some(b"GIF87a" | b"GIF89a")) {
        return Some((u64::from(u16_at(data, 6, false)?), u64::from(u16_at(data, 8, false)?)));
    }
    if data.starts_with(b"BM") {
        let width = i32::from_le_bytes(data.get(18..22)?.try_into().ok()?);
        let height = i32::from_le_bytes(data.get(22..26)?.try_into().ok()?);
        return Some((u64::from(width.unsigned_abs()), u64::from(height.unsigned_abs())));
    }
    if data.starts_with(b"\xff\xd8") {
        return jpeg_size(data);
    }
    if data.get(..4) == Some(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        return webp_size(data);
    }
    None
}

fn jpeg_size(data: &[u8]) -> Option<(u64, u64)> {
    let mut index = 2;
    while index + 9 < data.len() {
        if data.get(index) != Some(&0xFF) {
            index += 1;
            continue;
        }
        let marker = *data.get(index + 1)?;
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7 | 0xFF) {
            index += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        let length = usize::try_from(u16_at(data, index + 2, true)?).ok()?;
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = u16_at(data, index + 5, true)?;
            let width = u16_at(data, index + 7, true)?;
            return Some((u64::from(width), u64::from(height)));
        }
        index += 2 + length;
    }
    None
}

fn webp_size(data: &[u8]) -> Option<(u64, u64)> {
    match data.get(12..16)? {
        b"VP8X" => Some((u64::from(le_at(data, 24, 3)?) + 1, u64::from(le_at(data, 27, 3)?) + 1)),
        b"VP8 " => Some((
            u64::from(u16_at(data, 26, false)? & 0x3FFF),
            u64::from(u16_at(data, 28, false)? & 0x3FFF),
        )),
        b"VP8L" => {
            let bits = le_at(data, 21, 4)?;
            Some((u64::from(bits & 0x3FFF) + 1, u64::from((bits >> 14) & 0x3FFF) + 1))
        }
        _ => None,
    }
}

/// Refuse an image over the byte or decoded-pixel limit; its size, where known.
///
/// # Errors
/// Over either limit.
pub fn check_image(data: &[u8], limits: &BlobLimits, name: &str) -> Result<Option<(u64, u64)>, LimitError> {
    let label = if name.is_empty() { "the image" } else { name };
    if data.len() as u64 > limits.max_image_bytes {
        return limit(format!(
            "{label} is {} bytes; at most {}",
            data.len(),
            limits.max_image_bytes
        ));
    }
    let size = image_size(data);
    if let Some((width, height)) = size {
        if width * height > limits.max_image_pixels {
            return limit(format!(
                "{label} is {width}x{height} pixels; at most {} decoded",
                limits.max_image_pixels
            ));
        }
    }
    Ok(size)
}

/// Refuse a ZIP package that is too large, has too many parts, or looks like a bomb. Reads
/// only the central directory: the declared sizes are what the archive would expand to.
///
/// # Errors
/// Over a limit, or no readable ZIP package.
pub fn check_package(data: &Arc<[u8]>, limits: &BlobLimits, name: &str) -> Result<(), LimitError> {
    let label = if name.is_empty() { "the package" } else { name };
    if data.len() as u64 > limits.max_package_bytes {
        return limit(format!(
            "{label} is {} bytes; at most {}",
            data.len(),
            limits.max_package_bytes
        ));
    }
    let open = ZipLimits {
        max_entries: usize::MAX,
        max_entry_size: u64::MAX,
        max_total_size: u64::MAX,
        max_ratio: u64::MAX,
        ratio_floor: u64::MAX,
    };
    let archive = ooxml_zip::Archive::read(Arc::clone(data), &open)
        .map_err(|e| LimitError(format!("{label} is not a readable ZIP package: {e}")))?;
    let entries = archive.entries();
    if entries.len() > limits.max_parts {
        return limit(format!(
            "{label} has {} parts; at most {}",
            entries.len(),
            limits.max_parts
        ));
    }
    let total: u64 = entries.iter().map(|e| e.meta().size).sum();
    if total > limits.max_uncompressed_bytes {
        return limit(format!(
            "{label} expands to {total} bytes; at most {}",
            limits.max_uncompressed_bytes
        ));
    }
    for entry in entries {
        let meta = entry.meta();
        if meta.size > MB && meta.size > limits.max_compression_ratio.saturating_mul(meta.compressed_size.max(1)) {
            return limit(format!(
                "{label}: {} expands {} bytes from {}; over the ratio limit",
                meta.name, meta.size, meta.compressed_size
            ));
        }
    }
    Ok(())
}

/// An input the application registered, under a handle (`b1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// Its handle.
    pub handle: String,
    /// The name it was registered with.
    pub name: String,
    /// Its media type, as checked against the bytes.
    pub mime: String,
    /// Its bytes.
    pub data: Arc<[u8]>,
}

impl Blob {
    /// What the session's `describe` says of it: never its bytes.
    #[must_use]
    pub fn describe(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("handle".into(), Value::from(self.handle.as_str()));
        out.insert("name".into(), Value::from(self.name.as_str()));
        out.insert("mime".into(), Value::from(self.mime.as_str()));
        out.insert("size".into(), Value::from(self.data.len()));
        out
    }
}

/// The session's blobs, in the order registered.
#[derive(Debug, Clone, Default)]
pub struct BlobStore {
    blobs: Vec<Blob>,
    given: u64,
    /// Its limits.
    pub limits: BlobLimits,
}

impl BlobStore {
    /// An empty store within `limits`.
    #[must_use]
    pub fn new(limits: BlobLimits) -> BlobStore {
        BlobStore {
            blobs: Vec::new(),
            given: 0,
            limits,
        }
    }

    /// Register an input and return its handle (`b1`). The bytes are checked first: the
    /// size limits, that they are what `mime` claims (the type found from them when it is
    /// not given), an image's decoded size, and a ZIP package's central directory.
    ///
    /// # Errors
    /// A limit broken, or bytes that are not what they claim.
    pub fn add(&mut self, data: Arc<[u8]>, name: &str, mime: Option<&str>) -> Result<String, LimitError> {
        let mime = check_declared_type(&data, name, mime)?;
        let limits = self.limits;
        if data.len() as u64 > limits.max_blob_bytes {
            return limit(format!(
                "{name} is {} bytes; at most {}",
                data.len(),
                limits.max_blob_bytes
            ));
        }
        if mime.starts_with("image/") {
            check_image(&data, &limits, name)?;
        } else if data.starts_with(b"PK") {
            check_package(&data, &limits, name)?;
        } else if mime.starts_with("text/") {
            let text = std::str::from_utf8(&data)
                .map_err(|e| LimitError(format!("'utf-8' codec can't decode {name}: {e}")))?;
            if text.chars().count() > limits.max_markdown_chars {
                return limit(format!("{name} is over {} characters", limits.max_markdown_chars));
            }
        }
        let used: u64 = self.blobs.iter().map(|b| b.data.len() as u64).sum();
        let after = used + data.len() as u64;
        if after > limits.max_total_blob_bytes {
            return limit(format!(
                "the session's blobs would be {after} bytes; at most {}",
                limits.max_total_blob_bytes
            ));
        }
        self.given += 1;
        let handle = format!("b{}", self.given);
        self.blobs.push(Blob {
            handle: handle.clone(),
            name: name.to_owned(),
            mime,
            data,
        });
        Ok(handle)
    }

    /// The blob `handle` names; `not_found` lists `"handle (name)"` of every blob there is.
    ///
    /// # Errors
    /// No such blob.
    pub fn blob(&self, handle: &str, retry: &TryCall) -> Result<&Blob, ToolError> {
        self.blobs.iter().find(|b| b.handle == handle).ok_or_else(|| {
            let listed = self.blobs.iter().map(|b| format!("{} ({})", b.handle, b.name));
            ToolError::refusal(
                Refusal::NotFound,
                format!("no blob {}", repr_str(handle)),
                WayOut::names(listed).unwrap_or_else(|| WayOut::call(retry.clone())),
            )
        })
    }

    /// Every blob, in the order registered.
    #[must_use]
    pub fn blobs(&self) -> &[Blob] {
        &self.blobs
    }

    /// What the session's `describe` lists: each blob's [`Blob::describe`].
    #[must_use]
    pub fn describe(&self) -> Vec<Value> {
        self.blobs.iter().map(|b| Value::Object(b.describe())).collect()
    }
}
