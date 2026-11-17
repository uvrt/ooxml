//! Glyph outlines: no font file panics the outline reader. Each input is read as a font
//! (its first face), and a text's ink is read upright and slanted, through `glyf` or CFF and
//! `GPOS` pair kerning.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_fonts::Face;
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let Ok(face) = Face::new(Arc::from(data), 0, "fuzz") else { return };
    for slant in [0.0, 0.34] {
        let _ = face.ink("AVATAR Tyre", slant);
    }
    for glyph in 0..64 {
        let _ = face.glyph_ink(glyph, 0.34);
    }
});
