//! The chunk writer against the probe side's four test vectors (MS-FSSHTTPD 2.4.1; made-up
//! files, `tests/vectors`): every chunk's offset, size and deterministic signature, and the
//! subchunk sizes, byte for byte.
//!
//! v3's 2,500,000-byte stored entry is not kept: the file is its first 43 bytes (the local
//! header), 2,500,000 zero bytes, and its last 197 (the small entry and the central
//! directory). The analysis signs a data chunk with the CRC-32 and sizes its header states,
//! never with the data, so every offset, size and signature the vector lists is the file's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_zip::chunks::{ChunkKind, Chunking, Leaf, Reuse, Signature, MIB};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/");

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}{name}")).unwrap()
}

fn v3() -> Vec<u8> {
    let mut data = read("v3-over-1mb.head");
    data.resize(data.len() + 2_500_000, 0);
    data.extend(read("v3-over-1mb.tail"));
    data
}

/// A source that writes 0xA0, 0xA1, ... so random signatures are known.
fn counting() -> impl FnMut(&mut [u8]) {
    let mut next = 0xA0u8;
    move |out: &mut [u8]| {
        for b in out {
            *b = next;
            next = next.wrapping_add(1);
        }
    }
}

fn chunk(data: &[u8]) -> Chunking {
    Chunking::of(data, &mut counting())
}

/// (offset, size, signature in hex, or "random").
fn found(chunking: &Chunking) -> Vec<(usize, usize, String)> {
    chunking
        .chunks()
        .iter()
        .map(|c| {
            let signature = if c.random {
                "random".to_owned()
            } else {
                c.signature.to_hex()
            };
            (c.range.start, c.size(), signature)
        })
        .collect()
}

fn expected(rows: &[(usize, usize, &str)]) -> Vec<(usize, usize, String)> {
    rows.iter().map(|&(o, s, h)| (o, s, h.to_owned())).collect()
}

#[test]
fn v1_small_entries_are_xor_signed_chunks() {
    let data = read("v1-small-entries.zip");
    assert_eq!(data.len(), 478);
    let chunking = chunk(&data);
    assert!(chunking.zip_analysed());
    assert_eq!(
        found(&chunking),
        expected(&[
            (0, 127, "44cdf8a1a7f18de0fbd638ee8db8d8903fdf278d"),
            (127, 77, "bfb179fbe8f29fad7538554f0b81374a62ac7922"),
            (204, 69, "3dc9fa9c6924c4cd843e7763688af1a46d67ee8b"),
            (273, 205, "a81cf3bb28c4cdd712dde93ccd498740f636d4df"),
        ])
    );
    let kinds: Vec<_> = chunking.chunks().iter().map(|c| (c.kind, c.name.as_str())).collect();
    assert_eq!(
        kinds,
        [
            (ChunkKind::Entry, "[Content_Types].xml"),
            (ChunkKind::Entry, "doc/part1.xml"),
            (ChunkKind::Entry, "doc/part2.xml"),
            (ChunkKind::Final, ""),
        ]
    );
}

#[test]
fn v2_large_entries_are_a_header_and_a_data_chunk() {
    let data = read("v2-large-entries.zip");
    assert_eq!(data.len(), 13_734);
    let chunking = chunk(&data);
    assert!(chunking.zip_analysed());
    assert_eq!(
        found(&chunking),
        expected(&[
            (0, 42, "7f9f6fb7c6c50ce7da589e3b970c24edd69d4f57"),
            (42, 7394, "7a7a4136e21c0000000000002f23000000000000"),
            (7436, 43, "51c1202a5a29560402169042bac8a04ac1b01624"),
            (7479, 6000, "4b380ec370170000000000007017000000000000"),
            (13_479, 57, "f6f8d9f410f86b618f9bee4a2756204c9a9d3f2c"),
            (13_536, 198, "bf129aee882462978ad909ca7c29237e20c72ff0"),
        ])
    );
    let kinds: Vec<_> = chunking.chunks().iter().map(|c| c.kind).collect();
    use ChunkKind::*;
    assert_eq!(kinds, [Header, Data, Header, Data, Entry, Final]);
    assert!(chunking.chunks().iter().all(|c| c.subchunks.is_empty() && !c.random));
}

#[test]
fn v3_a_chunk_over_a_mebibyte_has_random_subchunks() {
    let data = v3();
    assert_eq!(data.len(), 2_500_240);
    let chunking = chunk(&data);
    assert!(chunking.zip_analysed());
    assert_eq!(
        found(&chunking),
        expected(&[
            (0, 43, "be52d8b150ffcc25ba65947b8e68c19db1c20f33"),
            (43, 2_500_000, "8644a576a025260000000000a025260000000000"),
            (2_500_043, 57, "f6f8d9f410f86b618f9bee4a2756204c9a9d3f2c"),
            (2_500_100, 140, "b2d5604c71bbdaaaecc60f3c84f471a3c0569c4d"),
        ])
    );
    let big = &chunking.chunks()[1];
    let sizes: Vec<_> = big.subchunks.iter().map(|s| s.range.len()).collect();
    assert_eq!(sizes, [1_048_576, 1_048_576, 402_848]);
    assert_eq!(big.subchunks[0].range.start, 43);
    assert_eq!(big.subchunks[2].range.end, 2_500_043);
    // 8 random bytes each, from the injected source
    let signatures: Vec<_> = big.subchunks.iter().map(|s| s.signature.to_hex()).collect();
    assert_eq!(signatures, ["a0a1a2a3a4a5a6a7", "a8a9aaabacadaeaf", "b0b1b2b3b4b5b6b7"]);
    assert!(chunking
        .chunks()
        .iter()
        .filter(|c| c.subchunks.is_empty())
        .all(|c| c.size() <= MIB));
}

#[test]
fn v4_a_first_entry_with_a_data_descriptor_is_simple_chunked() {
    let data = read("v4-data-descriptor.zip");
    assert_eq!(data.len(), 190);
    let chunking = chunk(&data);
    assert!(!chunking.zip_analysed());
    assert_eq!(
        found(&chunking),
        expected(&[(0, 190, "8a1a522de8c36378038c0ff8d0fd88548db3f793")])
    );
    assert_eq!(chunking.chunks()[0].kind, ChunkKind::Simple);
}

#[test]
fn the_root_is_an_empty_signature_and_the_file_size() {
    let data = read("v2-large-entries.zip");
    let chunking = chunk(&data);
    assert_eq!(
        chunking.root(),
        Leaf {
            signature: Signature::empty(),
            size: 13_734
        }
    );
    let total: u64 = chunking.leaves().iter().map(|l| l.size).sum();
    assert_eq!(total, 13_734);
}

#[test]
fn an_unchanged_large_entry_is_reused_through_its_parent_leaf() {
    let data = v3();
    let first = Chunking::of(&data, &mut counting());
    // another writer: other random bytes for the subchunks
    let second = Chunking::of(&data, &mut |out: &mut [u8]| out.fill(0x5A));
    assert_ne!(
        first.chunks()[1].subchunks[0].signature,
        second.chunks()[1].subchunks[0].signature
    );
    let delta = second.delta(&first.leaves());
    assert_eq!(delta.reuse(), [Reuse::Held; 4]);
    assert_eq!(delta.new_bytes(), 0);
    assert!(delta.new_chunks().is_empty());
}

#[test]
fn only_changed_chunks_travel() {
    let v1 = read("v1-small-entries.zip");
    let v2 = read("v2-large-entries.zip");
    let held = chunk(&v1).leaves();
    let delta = chunk(&v2).delta(&held);
    assert_eq!(delta.new_chunks(), [0, 1, 2, 3, 4, 5]);
    assert_eq!(delta.new_bytes(), 13_734);
    let same = chunk(&v1).delta(&held);
    assert_eq!(same.held_chunks(), [0, 1, 2, 3]);
}
