//! Owns: a changed part as the Word edits' harnesses report it (`word_tool_reference.py`'s
//! `part_texts` on docx-agent's side): its bytes as text, lossily -- or, for an embedded
//! package (a chart's workbook, a ZIP archive), each entry's content byte for byte under
//! `<part>!<entry>` (as text where it is UTF-8, else as hex) and the entries' names, order
//! and compression methods under the part's own name. Two writers' deflate streams differ
//! for the same content (CPython's zlib, the Rust core's zlib-rs), so an archive is compared
//! by what it holds, never by its compressed bytes; nothing else is loosened.
//!
//! Must not: report anything an archive's entries do not hold (their times and attributes
//! are the package's, which the Rust core keeps as it found them).

use ooxml_zip::{Archive, Limits};
use serde_json::{json, Map};
use std::sync::Arc;

/// Put `name`'s report into `parts`.
pub(crate) fn insert_part(parts: &mut Map<String, serde_json::Value>, name: String, bytes: &Arc<[u8]>) {
    if bytes.starts_with(b"PK\x03\x04") {
        if let Ok(archive) = Archive::read(Arc::clone(bytes), &Limits::default()) {
            let mut order = Vec::new();
            let mut inner = Vec::new();
            for entry in archive.entries().iter().filter(|e| !e.meta().is_dir()) {
                let Ok(data) = archive.read_entry(entry) else {
                    inner.clear();
                    break;
                };
                order.push(format!("{} ({})", entry.meta().name, entry.meta().method.code()));
                inner.push((format!("{name}!{}", entry.meta().name), exact(&data)));
            }
            if !order.is_empty() && order.len() == inner.len() {
                for (key, text) in inner {
                    parts.insert(key, json!(text));
                }
                parts.insert(name, json!(format!("archive: {}", order.join(", "))));
                return;
            }
        }
    }
    parts.insert(name, json!(String::from_utf8_lossy(bytes)));
}

/// An entry's content, exactly: its text where it is UTF-8, else `hex:` and its bytes.
fn exact(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(text) => text.to_owned(),
        Err(_) => {
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            format!("hex:{hex}")
        }
    }
}
