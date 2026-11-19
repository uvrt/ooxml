//! The registry is the one list of the tools: every other view of the tool set derives from
//! it, and each registered tool's definition is its golden export (written beside the
//! tool, `ooxml-tools-word/tests/golden/<name>.json`, from docx-agent's definition).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_session::{DocKind, Registry};
use std::path::Path;

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../ooxml-tools-word/tests/golden/{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} has no golden export {}: {e}", path.display()))
}

#[test]
fn every_tool_is_defined_as_its_golden_export() {
    let registry = Registry::standard();
    let exports = registry.export(DocKind::Docx);
    assert!(!exports.is_empty());
    for export in &exports {
        let name = export["name"].as_str().unwrap();
        assert_eq!(format!("{export}\n"), golden(name), "{name}");
    }
}

#[test]
fn the_names_are_the_exports_sorted_each_once() {
    let registry = Registry::standard();
    let mut exported: Vec<String> = registry
        .export(DocKind::Docx)
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect();
    let first = exported.first().cloned();
    let last = exported.last().cloned();
    exported.sort();
    let count = exported.len();
    exported.dedup();
    assert_eq!(exported.len(), count, "a tool registered twice for one kind");
    assert_eq!(registry.names(), exported);
    // describe is the first a model is shown; read_batch, over the others, comes last
    assert_eq!(first.as_deref(), Some("describe"));
    assert_eq!(last.as_deref(), Some("read_batch"));
    for name in registry.names() {
        let spec = match registry.named(&name) {
            Some(tool) => tool.spec(),
            None => registry.batch_spec(),
        };
        assert!(spec.problems().is_empty(), "{name}: {:?}", spec.problems());
    }
    assert!(registry.export(DocKind::Pptx).is_empty());
}
