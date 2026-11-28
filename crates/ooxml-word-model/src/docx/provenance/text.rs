//! Owns: the values written into paragraphs by reference and the labels written in place
//! of values, as a version keeps them (docx-agent's `ProvenanceOps.text_facts` and
//! `text_labels`): each text entry whose paragraph the version has (renames followed),
//! its facts and labels followed from the text they were kept with to the paragraph's
//! text now, under the id the paragraph has now.
//!
//! Must not: change an entry (an edit's follow-up does).

use super::facts::{facts_after_edit, labels_after_edit};
use super::{entries, entry_text, item, text_paragraph, TextEntry};
use crate::docx::text::paragraph_text;
use crate::docx::Docx;
use crate::view::View;
use ooxml_opc::OpcError;
use serde_json::Value;
use std::collections::BTreeMap;

/// `{paragraph id: facts}` (or labels), in the order the entries give them.
pub type ByParagraph = Vec<(String, Vec<Value>)>;

/// A version's text facts and labels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextRecords {
    /// The facts still their paragraph's text, by paragraph.
    pub facts: ByParagraph,
    /// The labels still in their paragraph, by paragraph.
    pub labels: ByParagraph,
}

/// `docx`'s text facts and labels, its renames `aliases`.
///
/// # Errors
/// When the main part's relationships are not XML.
pub fn text_records(docx: &Docx, aliases: &BTreeMap<String, String>) -> Result<TextRecords, OpcError> {
    let mut out = TextRecords::default();
    let Some(name) = item(docx)? else {
        return Ok(out);
    };
    let Some(tree) = docx.package.tree(&name)? else {
        return Ok(out);
    };
    for entry in entries(tree.root()) {
        if entry.attribute(None, "kind") != Some("text") {
            continue;
        }
        let key = entry.attribute(None, "key").unwrap_or_default();
        let Some(found) = TextEntry::read(&entry_text(entry)) else {
            continue;
        };
        let Some((paragraph, id)) = text_paragraph(docx, aliases, key) else {
            continue;
        };
        let now = paragraph_text(&paragraph, View::Final);
        add(
            &mut out.facts,
            &id,
            facts_after_edit(&found.facts, &found.paragraph, &now),
        );
        if !found.labels.is_empty() {
            add(
                &mut out.labels,
                &id,
                labels_after_edit(&found.labels, &found.paragraph, &now),
            );
        }
    }
    Ok(out)
}

/// `found` added to `id`'s list (Python's `setdefault(id, []).extend(found)`), when there
/// are any.
fn add(out: &mut ByParagraph, id: &str, found: Vec<Value>) {
    if found.is_empty() {
        return;
    }
    match out.iter_mut().find(|(k, _)| k == id) {
        Some((_, list)) => list.extend(found),
        None => out.push((id.to_owned(), found)),
    }
}
