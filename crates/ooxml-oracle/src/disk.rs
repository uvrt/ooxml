//! Owns: [`DiskFonts`], the font store over the file system.
//!
//! Must not: cache file contents beyond what the fonts layer asks it to read.

use ooxml_fonts::{FileStamp, FontError, FontFile, FontStore};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// Font files read from the file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskFonts;

struct DiskFile(std::fs::File);

fn store_error(e: std::io::Error) -> FontError {
    FontError::Store(e.to_string())
}

impl FontFile for DiskFile {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, FontError> {
        let mut file = &self.0;
        file.seek(SeekFrom::Start(offset)).map_err(store_error)?;
        let mut out = Vec::with_capacity(len.min(1 << 20));
        file.take(len as u64).read_to_end(&mut out).map_err(store_error)?;
        Ok(out)
    }
}

impl FontStore for DiskFonts {
    fn entries(&self, folder: &str) -> Option<Vec<String>> {
        let listing = std::fs::read_dir(folder).ok()?;
        Some(listing.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect())
    }

    fn open(&self, path: &str) -> Result<Box<dyn FontFile + '_>, FontError> {
        Ok(Box::new(DiskFile(std::fs::File::open(path).map_err(store_error)?)))
    }

    fn read(&self, path: &str) -> Result<Arc<[u8]>, FontError> {
        std::fs::read(path).map(Arc::from).map_err(store_error)
    }

    fn stamp(&self, path: &str) -> Option<FileStamp> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(FileStamp {
            modified_ns: i128::try_from(modified.as_nanos()).ok()?,
            size: meta.len(),
        })
    }
}
