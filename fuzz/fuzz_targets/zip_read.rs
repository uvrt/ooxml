//! ZIP reading within limits: no input panics or inflates past an entry's stated size, and
//! an archive copied raw entry by entry reads back with the same entries; the MS-FSSHTTPD
//! chunks of any input cover it in order, without gaps.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_zip::chunks::Chunking;
use ooxml_zip::{Archive, Limits, Writer};
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let chunking = Chunking::of(data, &mut |out: &mut [u8]| out.fill(0));
    let mut at = 0;
    for chunk in chunking.chunks() {
        assert_eq!(chunk.range.start, at);
        at = chunk.range.end;
    }
    assert_eq!(at, data.len());
    let limits = Limits { max_total_size: 1 << 24, max_entry_size: 1 << 22, ..Limits::default() };
    let Ok(archive) = Archive::read(Arc::from(data), &limits) else { return };
    let mut copy = Writer::new();
    for entry in archive.entries() {
        if let Ok(bytes) = archive.read_entry(entry) {
            assert_eq!(bytes.len() as u64, entry.meta().size);
        }
        if copy.copy(&archive, entry).is_err() {
            return;
        }
    }
    let Ok(copied) = copy.finish(archive.comment()) else { return };
    let again = Archive::read(Arc::from(copied), &limits).expect("a raw copy reads back");
    assert_eq!(again.entries().len(), archive.entries().len());
});
