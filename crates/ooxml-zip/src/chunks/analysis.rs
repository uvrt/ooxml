//! Owns: part A of the chunking -- walking a file's local headers into chunks and signing
//! them -- and the simple chunking it falls back to; part B's subchunks.
//!
//! Must not: fail or panic on any input: every read is bounds-checked, and what the
//! description says stops the walk stops it.

use super::{
    Chunk, ChunkKind, RandomSource, Signature, Subchunk, FINAL_RANDOM_LEN, MIB, SMALL_ENTRY, SUBCHUNK_RANDOM_LEN,
};
use crate::cp437;
use ooxml_base::sha1;
use std::ops::Range;

const LOCAL_SIG: [u8; 4] = *b"PK\x03\x04";
const LOCAL_LEN: usize = 30;
const ZIP64_EXTRA: u16 = 0x0001;
const FULL: u32 = 0xFFFF_FFFF;
const DATA_DESCRIPTOR: u16 = 0x0008;
const UTF8_NAME: u16 = 0x0800;

/// A chunk cut but not yet given its random parts: `signature` is `None` where it is random.
pub(super) struct Cut {
    kind: ChunkKind,
    name: String,
    range: Range<usize>,
    signature: Option<Signature>,
}

impl Cut {
    /// The chunk, its random signature and subchunks drawn from `random`.
    pub(super) fn signed(self, random: &mut dyn RandomSource) -> Chunk {
        let random_signature = self.signature.is_none();
        let signature = self
            .signature
            .unwrap_or_else(|| random_signature_of(random, FINAL_RANDOM_LEN));
        let subchunks = if self.range.len() > MIB {
            subchunks(self.range.clone(), random)
        } else {
            Vec::new()
        };
        Chunk {
            kind: self.kind,
            name: self.name,
            range: self.range,
            signature,
            random: random_signature,
            subchunks,
        }
    }
}

fn random_signature_of(random: &mut dyn RandomSource, len: usize) -> Signature {
    let mut bytes = [0u8; Signature::MAX];
    let slot = bytes.get_mut(..len).unwrap_or_default();
    random.fill(slot);
    Signature::new(slot).unwrap_or_default()
}

/// Part B: 1 MiB pieces of `range`, the last shorter, each with 8 random bytes.
fn subchunks(range: Range<usize>, random: &mut dyn RandomSource) -> Vec<Subchunk> {
    range
        .clone()
        .step_by(MIB)
        .map(|start| Subchunk {
            range: start..start.saturating_add(MIB).min(range.end),
            signature: random_signature_of(random, SUBCHUNK_RANDOM_LEN),
        })
        .collect()
}

/// The file in 1 MiB pieces, each signed with its SHA-1.
pub(super) fn simple_chunks(data: &[u8]) -> Vec<Cut> {
    data.chunks(MIB)
        .scan(0usize, |at, piece| {
            let start = *at;
            *at += piece.len();
            Some(Cut {
                kind: ChunkKind::Simple,
                name: String::new(),
                range: start..*at,
                signature: Some(Signature::from_array(sha1::digest(piece))),
            })
        })
        .collect()
}

/// A local header's fields the analysis reads.
struct Local {
    flags: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    name: Range<usize>,
    /// The header's length: 30, the name and the extra field.
    len: usize,
}

/// What reading the entry at an offset gives.
enum Step {
    /// An entry, its header and data within the file.
    Entry(Local),
    /// The walk stops here.
    Stop,
    /// The file is not analysed as ZIP at all.
    NotZip,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    let b = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes(b.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(b.try_into().ok()?))
}

fn u64_at(data: &[u8], at: usize) -> Option<u64> {
    let b = data.get(at..at.checked_add(8)?)?;
    Some(u64::from_le_bytes(b.try_into().ok()?))
}

/// The sizes from the ZIP64 extended information field in `extra`: its u64s are the size,
/// then the compressed size, each only where the header's is 0xFFFFFFFF. `None` without the
/// field, or when it is too short for the values it must hold.
fn zip64_sizes(extra: &[u8], compressed: u32, size: u32) -> Option<(u64, u64)> {
    let mut at = 0usize;
    while let (Some(tag), Some(len)) = (u16_at(extra, at), u16_at(extra, at + 2)) {
        let body = at + 4;
        if tag == ZIP64_EXTRA {
            let field = extra.get(body..body.checked_add(usize::from(len))?)?;
            let mut next = 0usize;
            let mut take = |stated: u32| -> Option<u64> {
                if stated != FULL {
                    return Some(u64::from(stated));
                }
                let v = u64_at(field, next)?;
                next += 8;
                Some(v)
            };
            let size = take(size)?;
            let compressed = take(compressed)?;
            return Some((compressed, size));
        }
        at = body.checked_add(usize::from(len))?;
    }
    None
}

/// The entry whose local header starts at `at`, or why the walk ends there.
fn read_local(data: &[u8], at: usize, first: bool) -> Step {
    let Some(header) = data.get(at..).filter(|rest| rest.len() >= LOCAL_LEN) else {
        return Step::Stop;
    };
    if header.get(..4) != Some(&LOCAL_SIG[..]) {
        return Step::Stop;
    }
    let field = |offset: usize| u16_at(header, offset).unwrap_or(0);
    let word = |offset: usize| u32_at(header, offset).unwrap_or(0);
    let (flags, crc, compressed, size) = (field(6), word(14), word(18), word(22));
    let (name_len, extra_len) = (usize::from(field(26)), usize::from(field(28)));
    let len = LOCAL_LEN + name_len + extra_len;
    let (compressed, size) = if compressed == FULL || size == FULL {
        let extra = header.get(LOCAL_LEN + name_len..len).unwrap_or(&[]);
        match zip64_sizes(extra, compressed, size) {
            Some(sizes) => sizes,
            None => return Step::Stop,
        }
    } else {
        (u64::from(compressed), u64::from(size))
    };
    if first && flags & DATA_DESCRIPTOR != 0 && compressed == 0 {
        return Step::NotZip;
    }
    let name = at + LOCAL_LEN..at + LOCAL_LEN + name_len;
    Step::Entry(Local {
        flags,
        crc,
        compressed,
        size,
        name,
        len,
    })
}

fn entry_name(data: &[u8], local: &Local) -> String {
    let raw = data.get(local.name.clone()).unwrap_or(&[]);
    if local.flags & UTF8_NAME != 0 {
        String::from_utf8_lossy(raw).into_owned()
    } else {
        cp437::decode(raw)
    }
}

/// The entry's chunks: one XOR-signed chunk, or a header and a data chunk.
fn entry_cuts(data: &[u8], at: usize, end: usize, local: &Local) -> Vec<Cut> {
    let name = entry_name(data, local);
    let header_end = at + local.len;
    let h = sha1::digest(data.get(at..header_end).unwrap_or(&[]));
    let mut d = [0u8; Signature::MAX];
    let (crc, rest) = d.split_at_mut(4);
    let (compressed, size) = rest.split_at_mut(8);
    crc.copy_from_slice(&local.crc.to_le_bytes());
    compressed.copy_from_slice(&local.compressed.to_le_bytes());
    size.copy_from_slice(&local.size.to_le_bytes());
    if end - at <= SMALL_ENTRY {
        let mut xor = h;
        for (x, b) in xor.iter_mut().zip(d) {
            *x ^= b;
        }
        return vec![Cut {
            kind: ChunkKind::Entry,
            name,
            range: at..end,
            signature: Some(Signature::from_array(xor)),
        }];
    }
    vec![
        Cut {
            kind: ChunkKind::Header,
            name: name.clone(),
            range: at..header_end,
            signature: Some(Signature::from_array(h)),
        },
        Cut {
            kind: ChunkKind::Data,
            name,
            range: header_end..end,
            signature: Some(Signature::from_array(d)),
        },
    ]
}

/// Part A: the file's chunks by its local headers, the final chunk last; `None` when the
/// file is not analysed as ZIP (simple chunking applies).
pub(super) fn zip_chunks(data: &[u8]) -> Option<Vec<Cut>> {
    let mut cuts = Vec::new();
    let mut at = 0usize;
    loop {
        let local = match read_local(data, at, cuts.is_empty()) {
            Step::Entry(local) => local,
            Step::Stop => break,
            Step::NotZip => return None,
        };
        let end = usize::try_from(local.compressed)
            .ok()
            .and_then(|compressed| at.checked_add(local.len)?.checked_add(compressed))
            .filter(|&end| end <= data.len());
        let Some(end) = end else { break };
        cuts.extend(entry_cuts(data, at, end, &local));
        at = end;
    }
    if cuts.is_empty() {
        return None;
    }
    let rest = data.get(at..).unwrap_or(&[]);
    let signature = (rest.len() <= MIB).then(|| Signature::from_array(sha1::digest(rest)));
    cuts.push(Cut {
        kind: ChunkKind::Final,
        name: String::new(),
        range: at..data.len(),
        signature,
    });
    Some(cuts)
}
