//! The ported tools' generated definitions equal the golden exports byte for byte, and the
//! definitions have no problems. The goldens are checked against today's Python definitions,
//! and by the Python checker, in `python/docx-agent/tests/test_rust_tool_specs.py`; together
//! the two tests show the generated schemas equal today's.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::ported;

#[test]
fn the_generated_definitions_equal_the_goldens() {
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for (spec, _, served) in ported::ported() {
        let path = golden.join(format!("{}.json", spec.name));
        let want = std::fs::read_to_string(&path).unwrap();
        let got = format!("{}\n", spec.export(Some(served)));
        assert_eq!(
            got, want,
            "{}: regenerate with `cargo run -p ooxml-tools --example export_specs`",
            spec.name
        );
    }
}

#[test]
fn the_ported_definitions_have_no_problems() {
    for (spec, problems, _) in ported::ported() {
        assert_eq!(problems, Vec::<String>::new(), "{}", spec.name);
    }
}
