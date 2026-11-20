//! The metafile players' differential, as a test: every case in `data/metafile-cases.gz`
//! (ooxml-common's drawings of the metafiles and bitmaps its, pptx2svg's and docx2svg's
//! tests make, and seeded mutants of them; `python/metafile_record.py` writes the bundle)
//! is played in Rust and must equal Python's drawing -- byte for byte, or but for the
//! deflate bytes of embedded PNGs (the same pixels).

use flate2::read::MultiGzDecoder;
use ooxml_oracle::metafile::{compare, read_case, Verdict};
use std::io::Read;

#[test]
fn every_recorded_case_draws_as_ooxml_common_draws_it() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/metafile-cases.gz");
    let mut bytes = Vec::new();
    MultiGzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_end(&mut bytes)
        .unwrap();
    let (mut at, mut cases, mut pixels) = (0usize, 0usize, 0usize);
    let mut differences = Vec::new();
    while at < bytes.len() {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let case = read_case(&bytes[at + 4..at + 4 + len]).unwrap();
        match compare(&case) {
            Verdict::Same => {}
            Verdict::SamePixels => pixels += 1,
            Verdict::Different(cause, detail) => differences.push(format!("case {cases} [{cause}] {detail}")),
        }
        cases += 1;
        at += 4 + len;
    }
    assert!(cases > 1000, "the bundle holds {cases} cases");
    assert!(
        differences.is_empty(),
        "{} of {cases} differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
    eprintln!("{cases} cases equal ({pixels} but for PNG deflate bytes)");
}
