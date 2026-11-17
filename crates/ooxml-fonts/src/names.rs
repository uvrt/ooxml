//! Owns: the `name` table's records, decoded as fontTools' `toUnicode` decodes them
//! (ooxml-common's `office.decode_name`): UTF-16BE for Unicode and Windows Unicode
//! records, Mac Roman for Macintosh Roman ones.
//!
//! Must not: decide which name a face is found by (that is [`crate::sfnt`]'s
//! `families`, the lookup's business).
//!
//! Not decoded (a record of them is skipped, as an undecodable one is): Windows records in
//! Shift JIS, GB2312, Big5, Wansung and Johab (encodings 2 to 6). Python decodes them with
//! its codecs; no installed face measured so far names itself only in them.

use crate::be::{slice, u16_at};

/// One decoded record of a `name` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameRecord {
    /// The platform ID.
    pub platform: u16,
    /// The encoding ID.
    pub encoding: u16,
    /// The language ID.
    pub language: u16,
    /// The name ID.
    pub name_id: u16,
    /// The text.
    pub text: String,
}

/// Every decodable record of a `name` table, in table order.
pub fn name_records(table: &[u8]) -> Vec<NameRecord> {
    let (Some(count), Some(storage)) = (u16_at(table, 2), u16_at(table, 4)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..usize::from(count) {
        let at = 6 + 12 * i;
        if at + 12 > table.len() {
            break;
        }
        let field = |k: usize| u16_at(table, at + 2 * k).unwrap_or_default();
        let (platform, encoding, language, name_id, length, offset) =
            (field(0), field(1), field(2), field(3), field(4), field(5));
        let raw = slice(table, usize::from(storage) + usize::from(offset), usize::from(length));
        if let Some(text) = decode_name(platform, encoding, raw) {
            out.push(NameRecord {
                platform,
                encoding,
                language,
                name_id,
                text,
            });
        }
    }
    out
}

/// A record's text, or `None` where it cannot be decoded.
pub fn decode_name(platform: u16, encoding: u16, raw: &[u8]) -> Option<String> {
    if platform == 0 || (platform == 3 && matches!(encoding, 0 | 1 | 10)) {
        return utf16_be(raw);
    }
    if platform == 1 && encoding == 0 {
        return Some(raw.iter().map(|&b| mac_roman(b)).collect());
    }
    None
}

fn utf16_be(raw: &[u8]) -> Option<String> {
    let (pairs, rest) = raw.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
    String::from_utf16(&units).ok()
}

/// Mac OS Roman's upper half (0x80 to 0xFF), as Python's `mac_roman` codec maps it.
const MAC_ROMAN_HIGH: &str = "\u{00c4}\u{00c5}\u{00c7}\u{00c9}\u{00d1}\u{00d6}\u{00dc}\u{00e1}\u{00e0}\u{00e2}\u{00e4}\u{00e3}\u{00e5}\u{00e7}\u{00e9}\u{00e8}\u{00ea}\u{00eb}\u{00ed}\u{00ec}\u{00ee}\u{00ef}\u{00f1}\u{00f3}\u{00f2}\u{00f4}\u{00f6}\u{00f5}\u{00fa}\u{00f9}\u{00fb}\u{00fc}\u{2020}\u{00b0}\u{00a2}\u{00a3}\u{00a7}\u{2022}\u{00b6}\u{00df}\u{00ae}\u{00a9}\u{2122}\u{00b4}\u{00a8}\u{2260}\u{00c6}\u{00d8}\u{221e}\u{00b1}\u{2264}\u{2265}\u{00a5}\u{00b5}\u{2202}\u{2211}\u{220f}\u{03c0}\u{222b}\u{00aa}\u{00ba}\u{03a9}\u{00e6}\u{00f8}\u{00bf}\u{00a1}\u{00ac}\u{221a}\u{0192}\u{2248}\u{2206}\u{00ab}\u{00bb}\u{2026}\u{00a0}\u{00c0}\u{00c3}\u{00d5}\u{0152}\u{0153}\u{2013}\u{2014}\u{201c}\u{201d}\u{2018}\u{2019}\u{00f7}\u{25ca}\u{00ff}\u{0178}\u{2044}\u{20ac}\u{2039}\u{203a}\u{fb01}\u{fb02}\u{2021}\u{00b7}\u{201a}\u{201e}\u{2030}\u{00c2}\u{00ca}\u{00c1}\u{00cb}\u{00c8}\u{00cd}\u{00ce}\u{00cf}\u{00cc}\u{00d3}\u{00d4}\u{f8ff}\u{00d2}\u{00da}\u{00db}\u{00d9}\u{0131}\u{02c6}\u{02dc}\u{00af}\u{02d8}\u{02d9}\u{02da}\u{00b8}\u{02dd}\u{02db}\u{02c7}";

fn mac_roman(byte: u8) -> char {
    if byte < 0x80 {
        return char::from(byte);
    }
    MAC_ROMAN_HIGH
        .chars()
        .nth(usize::from(byte - 0x80))
        .unwrap_or('\u{fffd}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_as_python_does() {
        assert_eq!(decode_name(3, 1, &[0, b'A', 0, b'b']).as_deref(), Some("Ab"));
        assert_eq!(decode_name(3, 1, &[0, b'A', 0]), None);
        assert_eq!(decode_name(1, 0, &[b'C', 0x8e]).as_deref(), Some("C\u{e9}"));
        assert_eq!(decode_name(1, 0, &[0xff]).as_deref(), Some("\u{2c7}"));
        assert_eq!(decode_name(3, 2, &[0x82, 0xa0]), None);
        assert_eq!(MAC_ROMAN_HIGH.chars().count(), 128);
    }
}
