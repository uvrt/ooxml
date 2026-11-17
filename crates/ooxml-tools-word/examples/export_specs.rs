//! Writes the Word tools' exports to `tests/golden/<tool>.json`: the files the Rust tests and
//! docx-agent's Python tests read. Run after changing a definition:
//! `cargo run -p ooxml-tools-word --example export_specs`.

use ooxml_tools::ToolArgs;
use ooxml_tools_word::{Describe, WordRead, KIND};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for (name, spec) in [("describe", Describe::spec()), ("word_read", WordRead::spec())] {
        let path = folder.join(format!("{name}.json"));
        std::fs::write(&path, format!("{}\n", spec.export(Some(&[KIND]))))?;
        println!("{}", path.display());
    }
    Ok(())
}
