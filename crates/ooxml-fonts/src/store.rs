//! Owns: the ports the font lookup reads files through -- [`FontStore`] (folders, files,
//! their stamps) and [`FontFile`] (reads at an offset, for a scan that reads a few
//! kilobytes of each file rather than the whole of it) -- and [`FileStamp`].
//!
//! Must not: implement a port. Adapters (the file system, an in-memory store for tests)
//! live in leaf crates and tests (design section 2.3).

use crate::FontError;
use std::sync::Arc;

/// What a file's identity is checked by: its modification time and size. A file whose
/// stamp changed is read again; the fonts epoch ([`crate::WordFonts::epoch`]) covers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FileStamp {
    /// Modification time, nanoseconds since the epoch.
    pub modified_ns: i128,
    /// Size in bytes.
    pub size: u64,
}

/// One font file opened for reads at offsets.
pub trait FontFile {
    /// Up to `len` bytes from `offset`: fewer at the end of the file, none past it.
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, FontError>;
}

/// Where font files are listed and read.
pub trait FontStore: Send + Sync {
    /// The names of the entries of `folder` (not paths), in any order; `None` where it is
    /// not a folder.
    fn entries(&self, folder: &str) -> Option<Vec<String>>;
    /// The path of entry `name` of `folder`.
    fn join(&self, folder: &str, name: &str) -> String {
        format!("{}/{}", folder.trim_end_matches('/'), name)
    }
    /// `path` opened for reads at offsets.
    fn open(&self, path: &str) -> Result<Box<dyn FontFile + '_>, FontError>;
    /// The whole of `path`.
    fn read(&self, path: &str) -> Result<Arc<[u8]>, FontError>;
    /// `path`'s stamp, or `None` where it cannot be stated.
    fn stamp(&self, path: &str) -> Option<FileStamp>;
}
