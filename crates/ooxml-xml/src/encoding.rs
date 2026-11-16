//! Owns: turning a part's bytes into UTF-8 text as libxml2 reads them -- the byte-order
//! mark, UTF-16 in either byte order (with or without a mark), and the declared encoding
//! for UTF-8, US-ASCII and ISO-8859-1 -- and the check that every character is one XML 1.0
//! allows.
//!
//! Must not: parse markup beyond the XML declaration's `encoding`. Other encodings are
//! refused ([`XmlError::Encoding`]) rather than guessed.

use crate::XmlError;
use std::borrow::Cow;

/// The part's text as UTF-8, its byte-order mark removed.
pub(crate) fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, XmlError> {
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => utf8(rest),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes).map(Cow::Owned),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes).map(Cow::Owned),
        [0x3C, 0x00, 0x3F, 0x00, ..] => utf16(bytes, u16::from_le_bytes).map(Cow::Owned),
        [0x00, 0x3C, 0x00, 0x3F, ..] => utf16(bytes, u16::from_be_bytes).map(Cow::Owned),
        _ => match declared_encoding(bytes).map(|e| e.to_ascii_lowercase()).as_deref() {
            None | Some("utf-8" | "utf8") => utf8(bytes),
            Some("us-ascii" | "ascii") => match bytes.iter().position(|b| !b.is_ascii()) {
                Some(at) => Err(XmlError::Encoding(format!("byte {at} is not US-ASCII"))),
                None => utf8(bytes),
            },
            Some("iso-8859-1" | "latin1" | "iso_8859-1" | "latin-1") => Ok(Cow::Owned(bytes.iter().map(|&b| char::from(b)).collect())),
            Some(other) => Err(XmlError::Encoding(format!("encoding {other:?} is not supported"))),
        },
    }
}

fn utf8(bytes: &[u8]) -> Result<Cow<'_, str>, XmlError> {
    std::str::from_utf8(bytes)
        .map(Cow::Borrowed)
        .map_err(|e| XmlError::Encoding(format!("invalid UTF-8 at byte {}", e.valid_up_to())))
}

fn utf16(bytes: &[u8], word: fn([u8; 2]) -> u16) -> Result<String, XmlError> {
    let (pairs, odd) = bytes.as_chunks::<2>();
    if !odd.is_empty() {
        return Err(XmlError::Encoding("UTF-16 text of an odd number of bytes".into()));
    }
    char::decode_utf16(pairs.iter().map(|p| word(*p)))
        .collect::<Result<String, _>>()
        .map_err(|e| XmlError::Encoding(format!("unpaired UTF-16 surrogate {:#06x}", e.unpaired_surrogate())))
}

/// The `encoding` of an XML declaration at the very start of ASCII-compatible bytes.
fn declared_encoding(bytes: &[u8]) -> Option<&str> {
    let decl = bytes.strip_prefix(b"<?xml")?;
    let end = decl.windows(2).position(|w| w == b"?>")?;
    let decl = std::str::from_utf8(decl.get(..end)?).ok()?;
    let after = decl.split_once("encoding")?.1.trim_start().strip_prefix('=')?.trim_start();
    let quote = after.chars().next().filter(|q| *q == '"' || *q == '\'')?;
    let value = after.get(1..)?;
    value.split(quote).next()
}

const ONES: u64 = 0x0101_0101_0101_0101;
const HIGHS: u64 = 0x8080_8080_8080_8080;

/// Whether any byte of `word` is below `n` (n <= 128).
pub(crate) fn any_below(word: u64, n: u8) -> bool {
    word.wrapping_sub(ONES * u64::from(n)) & !word & HIGHS != 0
}

/// Whether any byte of `word` is `b`.
pub(crate) fn any_is(word: u64, b: u8) -> bool {
    let y = word ^ (ONES * u64::from(b));
    y.wrapping_sub(ONES) & !y & HIGHS != 0
}

/// The offset of the first character XML 1.0 does not allow: a control character other than
/// tab, line feed and carriage return, or U+FFFE / U+FFFF. Eight bytes at a time where none
/// can be one.
pub(crate) fn first_invalid_char(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let (words, _) = bytes.as_chunks::<8>();
    let mut i = 0;
    for w in words {
        let word = u64::from_le_bytes(*w);
        if any_below(word, 0x20) || any_is(word, 0xEF) {
            if let Some(at) = first_invalid_byte(bytes, i, i + 8) {
                return Some(at);
            }
        }
        i += 8;
    }
    first_invalid_byte(bytes, i, bytes.len())
}

fn first_invalid_byte(bytes: &[u8], from: usize, to: usize) -> Option<usize> {
    (from..to).find(|&i| match bytes.get(i) {
        Some(&b) if b < 0x20 => !matches!(b, b'\t' | b'\n' | b'\r'),
        // U+FFFE and U+FFFF are EF BF BE and EF BF BF
        Some(0xEF) => bytes.get(i + 1) == Some(&0xBF) && matches!(bytes.get(i + 2), Some(0xBE | 0xBF)),
        _ => false,
    })
}

/// The offset in `bytes` of the first `a` or `b` (or the length).
pub(crate) fn find_either(bytes: &[u8], a: u8, b: u8) -> usize {
    let (words, _) = bytes.as_chunks::<8>();
    let mut i = 0;
    for w in words {
        let word = u64::from_le_bytes(*w);
        if any_is(word, a) || any_is(word, b) {
            break;
        }
        i += 8;
    }
    bytes.get(i..).and_then(|rest| rest.iter().position(|&c| c == a || c == b)).map_or(bytes.len(), |k| i + k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_what_libxml2_reads() {
        assert_eq!(decode(b"\xEF\xBB\xBF<a/>").unwrap(), "<a/>");
        let le: Vec<u8> = "\u{feff}<a>\u{e9}</a>".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&le).unwrap(), "<a>\u{e9}</a>");
        let be: Vec<u8> = "<?xml version='1.0'?><a/>".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode(&be).unwrap(), "<?xml version='1.0'?><a/>");
        assert_eq!(decode(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\xe9</a>").unwrap(), "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\u{e9}</a>");
        assert!(decode(b"<?xml version=\"1.0\" encoding=\"US-ASCII\"?><a>\xc3</a>").is_err());
        assert!(decode(b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><a/>").is_err());
        assert!(decode(b"<a>\xff</a>").is_err());
    }

    #[test]
    fn finds_characters_xml_does_not_allow() {
        assert_eq!(first_invalid_char("<a>\t\r\n</a>"), None);
        assert_eq!(first_invalid_char("<a>\u{1}</a>"), Some(3));
        assert_eq!(first_invalid_char("<a>\u{fffe}</a>"), Some(3));
        assert_eq!(first_invalid_char("<a>\u{85}\u{fffd}</a>"), None);
        let long = format!("{}\u{1}", "x".repeat(37));
        assert_eq!(first_invalid_char(&long), Some(37));
        assert_eq!(first_invalid_char(&"\u{fffd}".repeat(20)), None);
        assert_eq!(find_either(b"0123456789abcdef&<", b'<', b'&'), 16);
        assert_eq!(find_either(b"abc", b'<', b'&'), 3);
    }
}
