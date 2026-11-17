//! Owns: the faces a Word document embeds (ECMA-376 17.8.1): undoing a part's obfuscation
//! -- its first 32 bytes XORed with the 16 bytes of `w:fontKey`, read from the GUID's last
//! hex pair to its first -- and reading the face, keyed as the catalogue keys installed
//! ones (docx2svg's `embedded_faces`).
//!
//! Must not: parse the font table part (the Word model's job: it hands each `w:embed*`
//! here with its family, style, key and part bytes), or write a face anywhere.

use crate::sfnt::Face;
use crate::FontError;
use std::sync::Arc;

/// One `w:embedRegular` (or bold, italic, bold italic) of a document's font table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedPart {
    /// `w:font/@w:name`.
    pub family: String,
    /// The element's style: `Regular`, `Bold`, `Italic` or `BoldItalic`.
    pub style: String,
    /// `w:fontKey`, the GUID with or without its braces.
    pub key: String,
    /// The part's bytes, as stored (obfuscated).
    pub bytes: Vec<u8>,
}

/// `(bold, italic)` of an embed element's style; another style counts as regular.
pub fn embed_style(style: &str) -> (bool, bool) {
    match style {
        "Bold" => (true, false),
        "Italic" => (false, true),
        "BoldItalic" => (true, true),
        _ => (false, false),
    }
}

/// The 16 key bytes of a `w:fontKey` GUID, last hex pair first; `None` for a key that is
/// not at least 32 hex digits (the first 32 are read).
fn key_bytes(key: &str) -> Option<[u8; 16]> {
    let digits: Vec<u8> = key.bytes().filter(|b| !matches!(b, b'-' | b'{' | b'}')).collect();
    if digits.len() < 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (k, slot) in out.iter_mut().enumerate() {
        let at = 30 - 2 * k;
        let pair = std::str::from_utf8(digits.get(at..at + 2)?).ok()?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

/// An embedded part's font bytes, its obfuscation undone.
pub fn deobfuscate(key: &str, bytes: &[u8]) -> Result<Vec<u8>, FontError> {
    let mask = key_bytes(key).ok_or_else(|| FontError::malformed("embedded", format!("not a font key: {key}")))?;
    let mut out = bytes.to_vec();
    for (i, byte) in out.iter_mut().take(32).enumerate() {
        *byte ^= mask.get(i % 16).copied().unwrap_or_default();
    }
    Ok(out)
}

/// The face of an embedded part, or why it cannot be read.
pub fn embedded_face(part: &EmbeddedPart) -> Result<Face, FontError> {
    let data = deobfuscate(&part.key, &part.bytes)?;
    Face::new(Arc::from(data), 0, format!("embedded:{}:{}", part.family, part.style))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_read_from_its_last_pair_to_its_first() {
        let key = "{00112233-4455-6677-8899-AABBCCDDEEFF}";
        let mask = key_bytes(key).unwrap();
        assert_eq!(mask[0], 0xFF);
        assert_eq!(mask[15], 0x00);
        let clear = vec![0u8; 40];
        let hidden = deobfuscate(key, &clear).unwrap();
        assert_eq!(hidden[0], 0xFF);
        assert_eq!(hidden[16], 0xFF);
        assert_eq!(hidden[32], 0);
        assert_eq!(deobfuscate(key, &hidden).unwrap(), clear);
        assert!(deobfuscate("{xyz}", &clear).is_err());
    }
}
