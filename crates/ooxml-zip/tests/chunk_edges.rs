//! The chunk writer on malformed and unusual input: it falls back exactly as the
//! description says (the walk stops, or simple chunking) and never panics; whatever the
//! input, the chunks tile the file and the subchunks their chunk.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::sha1;
use ooxml_zip::chunks::{ChunkKind, Chunking, Reuse, MIB};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/");

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}{name}")).unwrap()
}

fn chunk(data: &[u8]) -> Chunking {
    Chunking::of(data, &mut |out: &mut [u8]| out.fill(7))
}

fn kinds(chunking: &Chunking) -> Vec<(ChunkKind, usize, usize)> {
    chunking
        .chunks()
        .iter()
        .map(|c| (c.kind, c.range.start, c.range.end))
        .collect()
}

/// The chunks cover the file in order without gaps, and each subchunk list its chunk.
fn assert_tiles(data: &[u8], chunking: &Chunking) {
    let mut at = 0;
    for c in chunking.chunks() {
        assert_eq!(c.range.start, at);
        at = c.range.end;
        if c.size() > MIB {
            assert_eq!(c.subchunks.first().unwrap().range.start, c.range.start);
            assert_eq!(c.subchunks.last().unwrap().range.end, c.range.end);
            assert!(c.subchunks.windows(2).all(|w| w[0].range.end == w[1].range.start));
            assert!(c.subchunks.iter().all(|s| s.range.len() <= MIB));
        } else {
            assert!(c.subchunks.is_empty());
        }
    }
    assert_eq!(at, data.len());
}

/// A stored entry with ZIP64 sizes in its extra field (`zip64`: the u64s it holds).
fn zip64_entry(stated: (u32, u32), zip64: &[u64], data: &[u8]) -> Vec<u8> {
    let name = b"a.bin";
    let mut extra = Vec::new();
    extra.extend(0x0001u16.to_le_bytes());
    extra.extend(u16::try_from(zip64.len() * 8).unwrap().to_le_bytes());
    for v in zip64 {
        extra.extend(v.to_le_bytes());
    }
    let mut out = b"PK\x03\x04".to_vec();
    out.extend([45, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method, time, date
    out.extend(0x1234_5678u32.to_le_bytes()); // CRC-32 (never checked)
    out.extend(stated.0.to_le_bytes()); // compressed
    out.extend(stated.1.to_le_bytes()); // size
    out.extend(u16::try_from(name.len()).unwrap().to_le_bytes());
    out.extend(u16::try_from(extra.len()).unwrap().to_le_bytes());
    out.extend(name);
    out.extend(extra);
    out.extend(data);
    out.extend(b"PK\x05\x06rest");
    out
}

#[test]
fn nothing_or_not_a_zip_is_simple_chunked() {
    let empty = chunk(&[]);
    assert!(!empty.zip_analysed() && empty.chunks().is_empty());
    assert_eq!(empty.root().size, 0);
    let text = vec![b'x'; 2 * MIB + 5];
    let chunking = chunk(&text);
    assert!(!chunking.zip_analysed());
    assert_eq!(
        kinds(&chunking),
        [
            (ChunkKind::Simple, 0, MIB),
            (ChunkKind::Simple, MIB, 2 * MIB),
            (ChunkKind::Simple, 2 * MIB, 2 * MIB + 5)
        ]
    );
    let last = &chunking.chunks()[2];
    assert_eq!(last.signature.as_bytes(), sha1::digest(b"xxxxx"));
    assert!(chunking.chunks().iter().all(|c| c.subchunks.is_empty() && !c.random));
}

#[test]
fn a_truncated_first_header_is_not_analysed() {
    let v1 = read("v1-small-entries.zip");
    for len in [4, 29, 30, 60] {
        let chunking = chunk(&v1[..len]);
        assert!(!chunking.zip_analysed(), "{len}");
        assert_eq!(kinds(&chunking), [(ChunkKind::Simple, 0, len)]);
    }
}

#[test]
fn a_truncated_later_entry_ends_the_walk_in_the_final_chunk() {
    let v1 = read("v1-small-entries.zip");
    let cut = &v1[..150];
    let chunking = chunk(cut);
    assert!(chunking.zip_analysed());
    assert_eq!(
        kinds(&chunking),
        [(ChunkKind::Entry, 0, 127), (ChunkKind::Final, 127, 150)]
    );
    assert_eq!(chunking.chunks()[1].signature.as_bytes(), sha1::digest(&cut[127..]));
}

#[test]
fn sizes_past_the_end_stop_the_walk() {
    let mut v1 = read("v1-small-entries.zip");
    // the second entry's compressed size, far past the end
    v1[127 + 18..127 + 22].copy_from_slice(&0x7FFF_0000u32.to_le_bytes());
    let chunking = chunk(&v1);
    assert_eq!(
        kinds(&chunking),
        [(ChunkKind::Entry, 0, 127), (ChunkKind::Final, 127, 478)]
    );
    // the first entry's: nothing walked, so simple chunking
    let mut v1 = read("v1-small-entries.zip");
    v1[18..22].copy_from_slice(&0x7FFF_0000u32.to_le_bytes());
    assert!(!chunk(&v1).zip_analysed());
}

#[test]
fn zip64_sizes_without_their_extra_field_stop_the_walk() {
    let mut v1 = read("v1-small-entries.zip");
    v1[127 + 18..127 + 22].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        kinds(&chunk(&v1)),
        [(ChunkKind::Entry, 0, 127), (ChunkKind::Final, 127, 478)]
    );
}

#[test]
fn zip64_sizes_come_from_the_extra_field() {
    // both sizes: the size first, then the compressed size
    let data = zip64_entry((u32::MAX, u32::MAX), &[5000, 4999], &[1; 4999]);
    let chunking = chunk(&data);
    assert!(chunking.zip_analysed());
    let header_len = 30 + 5 + 20;
    assert_eq!(
        kinds(&chunking),
        [
            (ChunkKind::Header, 0, header_len),
            (ChunkKind::Data, header_len, header_len + 4999),
            (ChunkKind::Final, header_len + 4999, data.len())
        ]
    );
    let d = chunking.chunks()[1].signature.to_hex();
    assert_eq!(d, "7856341287130000000000008813000000000000");
    // only the compressed size is 0xFFFFFFFF: the field holds it alone
    let data = zip64_entry((u32::MAX, 10), &[8], &[2; 8]);
    let chunking = chunk(&data);
    assert_eq!(kinds(&chunking)[0], (ChunkKind::Entry, 0, 30 + 5 + 12 + 8));
    // a field too short for the values it must hold: no chunk walked
    let data = zip64_entry((u32::MAX, u32::MAX), &[8], &[2; 8]);
    assert!(!chunk(&data).zip_analysed());
}

#[test]
fn a_data_descriptor_after_the_first_entry_is_walked_as_stated() {
    let v2 = read("v2-large-entries.zip");
    let mut data = v2.clone();
    // the small entry (at 13,479) flagged with a data descriptor: its sizes are still read
    data[13_479 + 6] |= 0x08;
    let chunking = chunk(&data);
    assert!(chunking.zip_analysed());
    assert_eq!(chunking.chunks().len(), 6);
    // the first entry flagged, but with its compressed size stated: analysed
    let mut data = v2;
    data[6] |= 0x08;
    assert!(chunk(&data).zip_analysed());
}

#[test]
fn a_final_chunk_over_a_mebibyte_is_signed_with_random_bytes() {
    let v1 = read("v1-small-entries.zip");
    let mut data = v1[..127].to_vec();
    data.resize(127 + MIB + 1, 3);
    let chunking = chunk(&data);
    let last = chunking.chunks().last().unwrap();
    assert_eq!((last.kind, last.random), (ChunkKind::Final, true));
    assert_eq!(last.signature.as_bytes(), [7; 12]);
    assert_eq!(last.subchunks.len(), 2);
    assert_tiles(&data, &chunking);
    // a random leaf is never reused when the random source differs
    let other = Chunking::of(&data, &mut |out: &mut [u8]| out.fill(9));
    assert_eq!(other.delta(&chunking.leaves()).reuse(), [Reuse::Held, Reuse::New]);
}

#[test]
fn a_repeated_new_leaf_travels_once() {
    let v1 = read("v1-small-entries.zip");
    let entry = &v1[..127];
    let mut data = entry.to_vec();
    data.extend(entry);
    data.extend(&v1[127..]);
    let delta = chunk(&data).delta(&[]);
    assert_eq!(delta.reuse()[..2], [Reuse::New, Reuse::Earlier(0)]);
}

/// A small xorshift generator, so the mutations are the same every run.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn mutated_and_truncated_vectors_never_panic_and_tile() {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    for name in ["v1-small-entries.zip", "v2-large-entries.zip", "v4-data-descriptor.zip"] {
        let original = read(name);
        for len in 0..original.len().min(400) {
            let chunking = chunk(&original[..len]);
            assert_tiles(&original[..len], &chunking);
        }
        for _ in 0..2000 {
            let mut data = original.clone();
            for _ in 0..(rng.next() % 4 + 1) {
                let at = usize::try_from(rng.next() % data.len() as u64).unwrap();
                data[at] = u8::try_from(rng.next() & 0xFF).unwrap();
            }
            let len = data.len() - usize::try_from(rng.next() % 8).unwrap();
            data.truncate(len);
            assert_tiles(&data, &chunk(&data));
        }
    }
}
