//! The readers of this crate that took an address's text before the commands took it read
//! ([`Address`], lane ph2-types8b), frozen as the reference the typed ones are compared with:
//! every address of the gates' cases (`crates/ooxml-oracle/cases`), on its document, holds
//! the same paragraphs or is refused the same, is kept as blocks the same, and follows its
//! renames to the same text.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::doc::WordDoc;
use crate::{EditError, Result};
use ooxml_word_model::address::{is_text_range, resolve, scope_paragraphs, span_ends, Address};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---- the readers as they were ----

/// `lists::paragraphs_of` as it read the text.
fn old_paragraphs_of(doc: &WordDoc, docx: &Docx, address: &str) -> Result<Vec<(String, usize)>> {
    let followed = doc.ids().follow_address(address);
    if let Some(found) = old_table_paragraphs(docx, &followed) {
        return Ok(found);
    }
    let found = scope_paragraphs(docx, &Address::parse(&followed)).or_else(|e| {
        if followed == address {
            Err(e)
        } else {
            scope_paragraphs(docx, &Address::parse(address))
        }
    });
    let (part, indexes) = found.map_err(EditError::Address)?;
    if indexes.is_empty() {
        return Err(EditError::Address(ooxml_word_model::address::AddressError::invalid(
            format!("{address} holds no paragraphs"),
            Vec::new(),
        )));
    }
    Ok(indexes.into_iter().map(|k| (part.part.clone(), k)).collect())
}

/// `lists::table_paragraphs` as it read the text.
fn old_table_paragraphs(docx: &Docx, address: &str) -> Option<Vec<(String, usize)>> {
    if is_text_range(address) || !matches!(span_ends(address), Ok(None)) {
        return None;
    }
    let (story, entry) = resolve(docx, address).ok()?;
    let Entry::Table(t) = entry else {
        return None;
    };
    let table = story.ids.tables().get(t)?;
    let grid = ooxml_word_model::docx::tables::TableGrid::new(story, &table);
    let mut out = Vec::new();
    for row in 0..grid.row_count() {
        for column in 0..grid.column_count() {
            out.extend(
                grid.cell_paragraphs(row, column)
                    .into_iter()
                    .map(|k| (story.part.clone(), k)),
            );
        }
    }
    Some(out)
}

/// `format::keeps_blocks` as it read the text.
fn old_keeps_blocks(doc: &WordDoc, docx: &Docx, address: &str) -> bool {
    if is_text_range(address) || address.contains('/') {
        return false;
    }
    if !matches!(span_ends(address), Ok(None)) {
        return true;
    }
    let followed = doc.ids().follow(address);
    !matches!(resolve(docx, &followed), Ok((_, Entry::Paragraph(_))))
}

// ---- the corpus ----

/// Every address-taking argument of the cases, a place's address too, by the document its
/// case runs on.
fn corpus_addresses() -> BTreeMap<PathBuf, BTreeSet<String>> {
    const KEYS: [&str; 17] = [
        "target", "targets", "at", "table", "range", "after", "before", "section", "scope", "block", "blocks",
        "within", "id", "bookmark", "ref", "start", "address",
    ];
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files(&path, out);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    fn strings(value: &serde_json::Value, key: &str, out: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => map.iter().for_each(|(k, v)| strings(v, k, out)),
            serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, key, out)),
            serde_json::Value::String(text) if KEYS.contains(&key) => {
                out.insert(text.clone());
                for kind in ["after:", "before:", "replace:", "end:"] {
                    if let Some(rest) = text.strip_prefix(kind) {
                        out.insert(rest.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    let oracle = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ooxml-oracle");
    let cases = oracle.join("cases");
    let mut paths = Vec::new();
    files(&cases, &mut paths);
    let mut by_document: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for path in paths {
        let Ok(relative) = path.strip_prefix(&cases) else {
            continue;
        };
        let own = relative
            .to_str()
            .and_then(|r| r.strip_suffix(".jsonl"))
            .filter(|r| !r.ends_with("_made"))
            .map(|r| oracle.join(r));
        for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
            let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let case = &entry["case"];
            let input = case["input"]
                .as_str()
                .and_then(|i| i.strip_prefix("$INPUT/"))
                .map(|i| cases.join("inputs").join(i));
            let Some(document) = input.or_else(|| own.clone()) else {
                continue;
            };
            strings(case, "", by_document.entry(document).or_default());
        }
    }
    by_document
}

#[test]
fn every_address_of_the_cases_is_read_by_the_commands_as_before() {
    let mut compared = 0;
    let mut differences = Vec::new();
    for (document, addresses) in corpus_addresses() {
        let Ok(bytes) = std::fs::read(&document) else { continue };
        let Ok(package) = ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()) else {
            continue;
        };
        let Ok(doc) = WordDoc::open(package, Arc::new(crate::validate::Unlimited)) else {
            continue;
        };
        let Ok(docx) = doc.docx() else { continue };
        for raw in &addresses {
            let address = Address::parse(raw);
            let mut differ = |what: &str| differences.push(format!("{}: {raw:?}: {what}", document.display()));
            let old = format!("{:?}", old_paragraphs_of(&doc, &docx, raw));
            let new = format!("{:?}", crate::lists::paragraphs_of(&doc, &docx, &address));
            if old != new {
                differ("paragraphs_of");
            }
            if old_keeps_blocks(&doc, &docx, raw) != crate::format::keeps_blocks(&doc, &docx, &address) {
                differ("keeps_blocks");
            }
            if doc.ids().follow_read(&address).as_str() != doc.ids().follow_address(raw) {
                differ("follow_read");
            }
            compared += 1;
        }
    }
    assert!(
        differences.is_empty(),
        "{} differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert!(compared > 1000, "{compared} addresses compared");
}
