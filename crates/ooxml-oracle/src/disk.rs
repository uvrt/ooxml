//! Owns: nothing of its own any more -- [`DiskFonts`], the font store over the file system,
//! moved to `ooxml-word-pages` (lane L4), where the layout in production reads the
//! machine's fonts through it; re-exported for the harnesses that name it here.
//!
//! Must not: hold a store of its own.

pub use ooxml_word_pages::DiskFonts;
