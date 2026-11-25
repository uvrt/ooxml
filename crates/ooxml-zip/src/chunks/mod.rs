//! Owns: a file's chunks as MS-FSSHTTPD 2.4.1 cuts and signs them for a co-authoring host
//! (Office's "ZIP" chunking scheme), their leaves, and which leaves a new version must send
//! to a host that holds the previous version's ([`Delta`]).
//!
//! Written from the probe side's clean-room description of the analysis (23 November 2026)
//! and checked against its four test vectors (`tests/chunk_vectors.rs`):
//!
//! - **A.** Walk the local headers from the start of the file. Each entry is one chunk
//!   signed SHA-1(header) XOR (CRC-32, compressed size, size) when header and data take at
//!   most 4,096 bytes, else a header chunk signed with its SHA-1 and a data chunk signed with
//!   the CRC-32 and sizes. Everything after the last entry walked (the central directory and
//!   end records) is the final chunk, signed with its SHA-1 up to 1 MiB, else with 12 random
//!   bytes. A file whose first entry has a data descriptor and no compressed size, or whose
//!   walk finds no entry, is cut into 1 MiB pieces each signed with its SHA-1 instead
//!   ("simple" chunking). A header that is truncated, whose sizes need a ZIP64 field it lacks,
//!   or whose data runs past the end stops the walk there.
//! - **B.** A chunk over 1 MiB keeps its signature and size as one leaf, with subchunks of
//!   1 MiB (the last shorter) under it, each signed with 8 random bytes.
//! - **C.** Each chunk is a leaf (signature and size); the file's root has an empty signature
//!   and the file's size. A new version reuses a held leaf whenever signature and size match.
//!
//! Random signatures come from a [`RandomSource`]; [`OsRandom`] in production, any closure in
//! tests. The walk only reads: it never fails, and malformed input falls back as described.
//!
//! Must not: know what a part is, or encode the FSSHTTPB wire format (the FSSHTTP client's).

mod analysis;
mod delta;
mod random;

pub use delta::{Delta, Reuse};
pub use random::{OsRandom, RandomSource};

use crate::ZipError;
use std::fmt;
use std::ops::Range;

/// The largest chunk stored as one piece, the subchunks' size and the simple chunks' size
/// (1 MiB).
pub const MIB: usize = 1 << 20;
/// The most bytes an entry's header and data take as one XOR-signed chunk.
pub const SMALL_ENTRY: usize = 4096;
/// The length of a final chunk's random signature.
pub const FINAL_RANDOM_LEN: usize = 12;
/// The length of a subchunk's random signature.
pub const SUBCHUNK_RANDOM_LEN: usize = 8;

/// A chunk signature: at most 20 bytes (a SHA-1, the CRC-32 and sizes, their XOR, or random
/// bytes); the root's is empty.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Signature {
    bytes: [u8; Signature::MAX],
    len: u8,
}

impl Signature {
    /// The longest signature.
    pub const MAX: usize = 20;

    /// The empty signature (the root's).
    pub const fn empty() -> Self {
        Signature {
            bytes: [0; Signature::MAX],
            len: 0,
        }
    }

    /// A signature of these bytes; refused when longer than [`Signature::MAX`].
    pub fn new(bytes: &[u8]) -> Result<Self, ZipError> {
        let mut out = Signature::empty();
        let slot = out
            .bytes
            .get_mut(..bytes.len())
            .ok_or(ZipError::Signature { len: bytes.len() })?;
        slot.copy_from_slice(bytes);
        out.len = u8::try_from(bytes.len()).map_err(|_| ZipError::Signature { len: bytes.len() })?;
        Ok(out)
    }

    /// A full-length signature.
    pub const fn from_array(bytes: [u8; Signature::MAX]) -> Self {
        Signature {
            bytes,
            len: Signature::MAX as u8,
        }
    }

    /// Its bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..usize::from(self.len)).unwrap_or(&[])
    }

    /// Its bytes in lowercase hexadecimal.
    pub fn to_hex(&self) -> String {
        self.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({})", self.to_hex())
    }
}

/// What a chunk holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// A whole entry, header and data (at most [`SMALL_ENTRY`] bytes), signed H XOR D.
    Entry,
    /// An entry's local header, signed with its SHA-1 (H).
    Header,
    /// An entry's data, signed with its CRC-32 and sizes (D).
    Data,
    /// Everything after the last entry walked: the central directory and end records.
    Final,
    /// A piece of a file not analysed as ZIP, signed with its SHA-1.
    Simple,
}

impl ChunkKind {
    /// Its name: `entry`, `header`, `data`, `final` or `simple`.
    pub fn name(self) -> &'static str {
        match self {
            ChunkKind::Entry => "entry",
            ChunkKind::Header => "header",
            ChunkKind::Data => "data",
            ChunkKind::Final => "final",
            ChunkKind::Simple => "simple",
        }
    }
}

/// A piece of a chunk over 1 MiB, with its random signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subchunk {
    /// Its bytes in the file.
    pub range: Range<usize>,
    /// Its signature ([`SUBCHUNK_RANDOM_LEN`] random bytes).
    pub signature: Signature,
}

/// One chunk of a file: one leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// What it holds.
    pub kind: ChunkKind,
    /// The entry's name for [`ChunkKind::Entry`], [`ChunkKind::Header`] and
    /// [`ChunkKind::Data`] (UTF-8 when flagged so, else code page 437); empty otherwise.
    pub name: String,
    /// Its bytes in the file.
    pub range: Range<usize>,
    /// Its signature.
    pub signature: Signature,
    /// Whether the signature is random bytes (a final chunk over 1 MiB): it never matches
    /// another writer's.
    pub random: bool,
    /// Its subchunks when it is over 1 MiB, in file order; empty otherwise.
    pub subchunks: Vec<Subchunk>,
}

impl Chunk {
    /// Its size in bytes.
    pub fn size(&self) -> usize {
        self.range.len()
    }

    /// Its leaf: signature and size.
    pub fn leaf(&self) -> Leaf {
        Leaf {
            signature: self.signature,
            size: self.range.len() as u64,
        }
    }
}

/// A leaf as a host holds it: a signature and a data size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Leaf {
    /// The chunk's signature.
    pub signature: Signature,
    /// The chunk's size in bytes.
    pub size: u64,
}

/// A file cut into chunks, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunking {
    analysed: bool,
    size: usize,
    chunks: Vec<Chunk>,
}

impl Chunking {
    /// `data` cut and signed (part A and B), random signatures drawn from `random`.
    pub fn of(data: &[u8], random: &mut dyn RandomSource) -> Self {
        let (analysed, chunks) = match analysis::zip_chunks(data) {
            Some(chunks) => (true, chunks),
            None => (false, analysis::simple_chunks(data)),
        };
        let chunks = chunks.into_iter().map(|cut| cut.signed(random)).collect();
        Chunking {
            analysed,
            size: data.len(),
            chunks,
        }
    }

    /// Whether the file was analysed as ZIP (else it was cut into simple chunks).
    pub fn zip_analysed(&self) -> bool {
        self.analysed
    }

    /// The file's size.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The chunks in file order.
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    /// The leaves in file order: what a host holds of this version.
    pub fn leaves(&self) -> Vec<Leaf> {
        self.chunks.iter().map(Chunk::leaf).collect()
    }

    /// The root: an empty signature and the file's size (part C).
    pub fn root(&self) -> Leaf {
        Leaf {
            signature: Signature::empty(),
            size: self.size as u64,
        }
    }

    /// Which chunks a host holding `previous` (the previous version's leaves) must be sent.
    pub fn delta(&self, previous: &[Leaf]) -> Delta {
        Delta::new(self, previous)
    }
}
