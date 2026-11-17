//! The XML parser and writer: no input panics, and what the writer writes parses back to a
//! tree that writes the same bytes (the write contract is a fixed point).
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_xml::{parse, Limits};

fuzz_target!(|data: &[u8]| {
    let limits = Limits::default();
    if let Ok(tree) = parse(data, &limits) {
        let written = tree.write();
        let again = parse(&written, &limits).expect("the writer's output parses");
        assert_eq!(again.write(), written, "the write contract is a fixed point");
        assert_eq!(again, tree, "the written tree is the parsed tree");
    }
});
