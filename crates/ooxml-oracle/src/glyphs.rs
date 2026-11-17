//! Owns: nothing of its own any more -- [`WordGlyphs`], Word's fonts as the text engine's
//! port, moved to the layout (`ooxml-word-layout`, lane L3a), which adapts the fonts layer
//! to the text engine; re-exported for the harnesses that name it here.
//!
//! Must not: hold an adapter of its own.

pub use ooxml_word_layout::faces::{Coverage, WordGlyphs};
