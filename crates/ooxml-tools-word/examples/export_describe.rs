//! Writes `describe`'s export to `tests/golden/describe.json`: the file the Rust test and
//! docx-agent's Python test read. Run after changing the definition:
//! `cargo run -p ooxml-tools-word --example export_describe`.

use ooxml_tools::ToolArgs;
use ooxml_tools_word::{Describe, KIND};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/describe.json");
    std::fs::write(&path, format!("{}\n", Describe::spec().export(Some(&[KIND]))))?;
    println!("{}", path.display());
    Ok(())
}
