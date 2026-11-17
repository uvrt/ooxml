//! Owns: the standard base64 alphabet with padding, for `data:` URIs.
//!
//! Must not: decode (nothing in the scene reads base64).

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// How many characters [`encode_into`] writes for `len` bytes.
pub fn encoded_len(len: usize) -> usize {
    len.div_ceil(3) * 4
}

/// `bytes` in base64 (RFC 4648, padded), appended to `out`.
pub fn encode_into(bytes: &[u8], out: &mut String) {
    out.reserve(encoded_len(bytes.len()));
    for chunk in bytes.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let sextets = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        let keep = chunk.len() + 1;
        for (i, s) in sextets.into_iter().enumerate() {
            if i < keep {
                out.push(char::from(ALPHABET.get(s as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            let mut out = String::new();
            encode_into(input.as_bytes(), &mut out);
            assert_eq!(out, expected);
            assert_eq!(encoded_len(input.len()), expected.len());
        }
    }
}
