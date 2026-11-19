//! Writes the ported tools' exports to `tests/golden/` (or the folder given): the files the
//! Rust equality test and the Python checker read. Run after changing a ported tool:
//! `cargo run -p ooxml-tools --example export_specs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

#[path = "../tests/integration/ported/mod.rs"]
mod ported;

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out = std::env::args().nth(1).map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&out)?;
    for (spec, _, served) in ported::ported() {
        let path = out.join(format!("{}.json", spec.name));
        std::fs::write(&path, format!("{}\n", spec.export(Some(served))))?;
        println!("{}", path.display());
    }
    Ok(())
}
