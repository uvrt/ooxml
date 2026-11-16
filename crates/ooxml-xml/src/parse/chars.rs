//! Owns: XML 1.0 (fifth edition) character classes for names, as libxml2 applies them.
//! Must not: scan anything but names.

/// `NameStartChar`.
pub(crate) fn is_name_start(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}'
        | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}'
        | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}'
        | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// `NameChar`.
pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// ASCII name characters: 1 may start a name, 2 may continue one (3 both).
fn ascii_name(b: u8) -> u8 {
    match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'_' | b':' => 3,
        b'0'..=b'9' | b'-' | b'.' => 2,
        _ => 0,
    }
}

/// The length in bytes of the name at the start of `s` (0 when there is none).
pub(crate) fn name_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    // ASCII names (nearly all in OOXML) without decoding characters
    match bytes.first() {
        Some(&b) if b < 0x80 => {
            if ascii_name(b) & 1 == 0 {
                return 0;
            }
        }
        _ => return name_len_chars(s),
    }
    let mut i = 1;
    while let Some(&b) = bytes.get(i) {
        if b >= 0x80 {
            return name_len_chars(s);
        }
        if ascii_name(b) & 2 == 0 {
            return i;
        }
        i += 1;
    }
    i
}

fn name_len_chars(s: &str) -> usize {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, c)) if is_name_start(c) => {}
        _ => return 0,
    }
    for (i, c) in chars {
        if !is_name_char(c) {
            return i;
        }
    }
    s.len()
}

/// Whether `name` is a QName libxml2 accepts: at most one colon, not first or last.
pub(crate) fn is_qname(name: &str) -> bool {
    match name.split_once(':') {
        None => true,
        Some((prefix, local)) => !prefix.is_empty() && !local.is_empty() && !local.contains(':'),
    }
}

/// XML whitespace.
pub(crate) fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(name_len("w:p w:x"), 3);
        assert_eq!(name_len("1a"), 0);
        assert_eq!(name_len("\u{e9}t\u{e9}>"), 5);
        assert!(is_qname("w:p") && is_qname("p") && !is_qname("a:b:c") && !is_qname(":a") && !is_qname("a:"));
    }
}
