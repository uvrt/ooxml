//! Owns: what a folder scan learns of a font file without reading it whole -- each face's
//! table directory and its `name`, `OS/2` and `head` tables, a few kilobytes -- which is
//! the face's style and the family names Word finds it by (docx2svg's `_scanned_faces` and
//! `_scanned_entries`).
//!
//! Must not: decide which face wins a name (that is [`crate::catalogue`]).

use crate::be::{u16_at, u32_at};
use crate::names::name_records;
use crate::sfnt::{families_of, Tag};
use crate::store::FontFile;
use crate::FontError;
use std::collections::BTreeSet;

/// One face of a scanned file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFace {
    /// Its number in the file (0 for a single font).
    pub number: usize,
    /// `(bold, italic, family names)`, or `None` where its style or names cannot be read.
    pub named: Option<(bool, bool, BTreeSet<String>)>,
}

const SCAN_TABLES: [&Tag; 3] = [b"name", b"OS/2", b"head"];

/// Every face of a file, from its header tables. None at all where the file's header or a
/// face's table directory cannot be read.
pub fn scan_file(file: &dyn FontFile) -> Vec<ScannedFace> {
    scan(file).unwrap_or_default()
}

fn scan(file: &dyn FontFile) -> Result<Vec<ScannedFace>, FontError> {
    let short = || FontError::Malformed {
        source_name: String::new(),
        message: "cut short".into(),
    };
    let head = file.read_at(0, 12)?;
    let offsets: Vec<u64> = if head.get(..4) == Some(b"ttcf".as_slice()) {
        let count = u32_at(&head, 8).ok_or_else(short)? as usize;
        let list = file.read_at(12, 4 * count)?;
        (0..count)
            .map(|i| u32_at(&list, 4 * i).map(u64::from).ok_or_else(short))
            .collect::<Result<_, _>>()?
    } else {
        vec![0]
    };
    let mut faces = Vec::new();
    for (number, offset) in offsets.into_iter().enumerate() {
        let header = file.read_at(offset, 12)?;
        let count = usize::from(u16_at(&header, 4).filter(|_| header.len() >= 12).ok_or_else(short)?);
        let directory = file.read_at(offset + 12, 16 * count)?;
        let mut tables: Vec<(Tag, u64, usize)> = Vec::new();
        for i in 0..count {
            let entry = directory.get(16 * i..16 * i + 16).ok_or_else(short)?;
            let tag: Tag = entry.get(..4).and_then(|t| t.try_into().ok()).ok_or_else(short)?;
            let (at, len) = (
                u32_at(entry, 8).ok_or_else(short)?,
                u32_at(entry, 12).ok_or_else(short)?,
            );
            tables.retain(|(t, _, _)| *t != tag);
            tables.push((tag, u64::from(at), len as usize));
        }
        let mut read: Vec<(Tag, Vec<u8>)> = Vec::new();
        for tag in SCAN_TABLES {
            if let Some((_, at, len)) = tables.iter().find(|(t, _, _)| t == tag) {
                read.push((*tag, file.read_at(*at, *len)?));
            }
        }
        faces.push(ScannedFace {
            number,
            named: named(&read),
        });
    }
    Ok(faces)
}

fn named(tables: &[(Tag, Vec<u8>)]) -> Option<(bool, bool, BTreeSet<String>)> {
    let table = |tag: &Tag| tables.iter().find(|(t, _)| t == tag).map(|(_, bytes)| bytes.as_slice());
    let head = table(b"head")?;
    let selection = table(b"OS/2")
        .filter(|t| t.len() >= 64)
        .and_then(|t| u16_at(t, 62))
        .unwrap_or(0);
    let mac_style = u16_at(head, 44)?;
    let names = table(b"name").map(name_records).unwrap_or_default();
    Some((
        selection & 0x20 != 0 || mac_style & 1 != 0,
        selection & 0x01 != 0 || mac_style & 2 != 0,
        families_of(&names),
    ))
}
