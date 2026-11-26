//! Every Word and PowerPoint fixture of the repository chunks by ZIP analysis, entry by
//! entry up to the central directory (the libraries save them byte for byte, so a save
//! chunks the same; their Python tests hold the saves and one-edit deltas). The one
//! exception is a deck whose writer put a data descriptor on its first entry: the
//! description says such a file is simple-chunked, and the library keeps its layout.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_zip::chunks::{ChunkKind, Chunking};
use ooxml_zip::{Archive, Limits};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
/// Fixtures whose input is laid out so that MS-FSSHTTPD does not analyse it as ZIP.
const NOT_ANALYSED: &[&str] = &["real-basic-theme.pptx"];

fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            packages(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "docx" || e == "pptx" || e == "xlsx")
        {
            out.push(path);
        }
    }
}

#[test]
fn every_fixture_chunks_entry_by_entry() {
    let mut paths = Vec::new();
    for dir in ["crates/ooxml-oracle/fixtures", "python/pptx-agent/tests/fixtures"] {
        packages(&Path::new(ROOT).join(dir), &mut paths);
    }
    assert!(paths.len() >= 40, "{} fixtures", paths.len());
    let mut skipped = Vec::new();
    for path in &paths {
        let data = std::fs::read(path).unwrap();
        let chunking = Chunking::of(&data, &mut |out: &mut [u8]| out.fill(0));
        let name = path.file_name().unwrap().to_string_lossy();
        if NOT_ANALYSED.contains(&name.as_ref()) {
            assert!(!chunking.zip_analysed(), "{name}");
            skipped.push(name.into_owned());
            continue;
        }
        assert!(chunking.zip_analysed(), "{}", path.display());
        let archive = Archive::read(Arc::from(data), &Limits::default()).unwrap();
        let walked = chunking
            .chunks()
            .iter()
            .filter(|c| matches!(c.kind, ChunkKind::Entry | ChunkKind::Header))
            .count();
        assert_eq!(walked, archive.entries().len(), "{}", path.display());
        assert_eq!(chunking.chunks().last().unwrap().kind, ChunkKind::Final);
    }
    assert_eq!(skipped, NOT_ANALYSED);
}
