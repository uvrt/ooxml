//! OPC packages: no input panics; every part's tree and bytes can be asked for; an unchanged
//! save is the input, and a save with every XML part touched opens again with the same parts.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_opc::{Limits, Package};
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let mut limits = Limits::default();
    limits.zip.max_total_size = 1 << 24;
    let Ok(package) = Package::open(Arc::from(data), &limits) else { return };
    let mut touched = package.clone();
    for name in package.part_names() {
        let _ = package.read(&name);
        if let Ok(Some(tree)) = package.tree(&name) {
            touched = touched.with_tree(&name, tree).expect("the part exists");
        }
    }
    if let Ok(saved) = package.save() {
        assert_eq!(&saved[..], data, "an unchanged package saves as it was read");
    }
    if let Ok(saved) = touched.save() {
        let again = Package::open(Arc::from(saved), &limits).expect("a saved package opens");
        assert_eq!(again.part_names(), package.part_names());
    }
});
