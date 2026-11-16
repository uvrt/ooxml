//! ZIP archives as OOXML packages use them (docs/architecture/README.md, sections 3, 4, 11).
//!
//! Owns: reading an archive's entries within the zip-bomb limits ([`Archive`]), an entry's
//! uncompressed bytes checked by size and CRC-32 ([`Archive::read_entry`]), and writing an
//! archive ([`Writer`]): untouched entries copied raw, compressed bytes and all; new or
//! changed ones deflated (zlib-rs) in the form Python's zipfile writes.
//!
//! Must not: parse XML or know what a part is.
#![forbid(unsafe_code)]

mod cp437;
mod error;
mod inflate;
mod read;
mod write;

pub use error::ZipError;
pub use ooxml_base::ZipLimits as Limits;
pub use read::{Archive, Entry, EntryMeta, Method};
pub use write::{NewEntry, Writer};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn entry(name: &str, method: Method) -> NewEntry {
        NewEntry {
            name: name.into(),
            method,
            dos_time: 0,
            dos_date: 33,
            external_attr: 0,
            create_system: 3,
        }
    }

    fn archive(parts: &[(&str, &[u8], Method)]) -> Vec<u8> {
        let mut w = Writer::new();
        for (name, data, method) in parts {
            w.add(&entry(name, *method), data).unwrap();
        }
        w.finish(b"note").unwrap()
    }

    #[test]
    fn written_entries_read_back() {
        let xml = b"<a>".repeat(1000);
        let bytes = archive(&[
            ("[Content_Types].xml", &xml, Method::Deflated),
            ("media/\u{e9}.png", b"\x89PNG", Method::Stored),
        ]);
        let a = Archive::read(bytes.into(), &Limits::default()).unwrap();
        assert_eq!(a.entries().len(), 2);
        let [first, second] = a.entries() else { panic!() };
        assert_eq!(a.read_entry(first).unwrap(), xml);
        assert!(first.meta().compressed_size < 100);
        assert_eq!(second.meta().name, "media/\u{e9}.png");
        assert_eq!(second.meta().flags, 0x800);
        assert_eq!(second.meta().external_attr, 0o600 << 16);
        assert_eq!(second.meta().version_made_by, 3 << 8 | 20);
        assert_eq!(a.comment(), b"note");
    }

    #[test]
    fn raw_copies_are_byte_identical() {
        let bytes = archive(&[("a.xml", b"<a/>", Method::Deflated), ("b.bin", b"0123", Method::Stored)]);
        let a = Archive::read(Arc::from(bytes.clone()), &Limits::default()).unwrap();
        let mut w = Writer::new();
        for e in a.entries() {
            w.copy(&a, e).unwrap();
        }
        assert_eq!(w.finish(a.comment()).unwrap(), bytes);
    }

    #[test]
    fn limits_refuse_bombs_and_damage() {
        let big = vec![0u8; 3 << 20];
        let bytes = archive(&[("zeros", &big, Method::Deflated)]);
        let tight = Limits {
            max_ratio: 100,
            ..Limits::default()
        };
        assert!(matches!(
            Archive::read(Arc::from(bytes.clone()), &tight),
            Err(ZipError::Limit { .. })
        ));
        let small = Limits {
            max_entry_size: 1 << 20,
            ..Limits::default()
        };
        assert!(matches!(
            Archive::read(Arc::from(bytes.clone()), &small),
            Err(ZipError::Limit { .. })
        ));
        assert!(matches!(
            Archive::read(Arc::from(&b"not a zip"[..]), &Limits::default()),
            Err(ZipError::Malformed(_))
        ));
        // a flipped byte in the data: the CRC-32 check refuses it
        let mut damaged = archive(&[("a.bin", b"0123456789", Method::Stored)]);
        if let Some(b) = damaged.get_mut(30 + 5 + 3) {
            *b ^= 1;
        }
        let a = Archive::read(Arc::from(damaged), &Limits::default()).unwrap();
        assert!(matches!(a.read_entry(&a.entries()[0]), Err(ZipError::Corrupt { .. })));
    }
}
