//! Owns: name-based UUIDs as Python's `uuid.uuid5` makes them (RFC 4122 version 5: the
//! first 16 bytes of the SHA-1 digest of the namespace's bytes and the name's UTF-8, its
//! version and variant bits set), and the URL namespace (`uuid.NAMESPACE_URL`).
//!
//! Must not: draw random ids (an id made here is the same for the same name, as every id
//! the tools write is).

use crate::sha1::digest;

/// `uuid.NAMESPACE_URL`: `6ba7b811-9dad-11d1-80b4-00c04fd430c8`.
pub const NAMESPACE_URL: [u8; 16] = [
    0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
];

/// `uuid.uuid5(namespace, name)`'s bytes.
#[must_use]
pub fn uuid5(namespace: &[u8; 16], name: &str) -> [u8; 16] {
    let mut data = namespace.to_vec();
    data.extend_from_slice(name.as_bytes());
    let hash = digest(&data);
    let mut out = [0u8; 16];
    out.copy_from_slice(hash.get(..16).unwrap_or(&[0; 16]));
    if let Some(byte) = out.get_mut(6) {
        *byte = (*byte & 0x0f) | 0x50;
    }
    if let Some(byte) = out.get_mut(8) {
        *byte = (*byte & 0x3f) | 0x80;
    }
    out
}

/// `str(uuid)` in capitals and braces (`{6BA7B811-9DAD-...}`): the form a GUID takes in
/// Office's parts, as the tools write one (`"{%s}" % str(uuid).upper()`).
#[must_use]
pub fn braced_upper(uuid: &[u8; 16]) -> String {
    let hex: String = uuid.iter().map(|b| format!("{b:02X}")).collect();
    let part = |range: std::ops::Range<usize>| hex.get(range).unwrap_or_default();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        part(0..8),
        part(8..12),
        part(12..16),
        part(16..20),
        part(20..32)
    )
}

#[cfg(test)]
mod tests {
    use super::{braced_upper, uuid5, NAMESPACE_URL};

    #[test]
    fn as_python_makes_them() {
        // python3 -c 'import uuid; print(uuid.uuid5(uuid.NAMESPACE_URL, "urn:uvrt:docx-agent:provenance:v1#p:1A2B3C4D"))'
        let made = uuid5(&NAMESPACE_URL, "urn:uvrt:docx-agent:provenance:v1#p:1A2B3C4D");
        assert_eq!(braced_upper(&made), "{1528803C-6B21-5A35-AE39-7B503BA5D70F}");
        assert_eq!(
            braced_upper(&uuid5(&NAMESPACE_URL, "python.org")),
            "{7AF94E2B-4DD9-50F0-9C9A-8A48519BDEF0}"
        );
    }
}
