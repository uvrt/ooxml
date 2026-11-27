//! Owns: docx-agent's records of where placed values came from, kept in the document
//! (`edit/provenance.py`): the custom XML item whose root is `prov:sources`
//! ([`NS`]), found from the main part's `customXml` relationships by its root's namespace
//! ([`item`]); its entries ([`entries`]: `prov:source` with a `kind`, a `key` and a
//! record as text); what a key names ([`kind_of`]) and whether its table, chart or
//! paragraph is in a version ([`target_in_file`], renamed paragraphs followed); a text
//! entry's paragraph text, facts and labels ([`TextEntry`]); and the rules facts and labels
//! follow an edit by ([`facts`]); what an entry holds once facts or labels are kept on it
//! ([`keep`]); the tables' and charts' records as docx-agent reads
//! them ([`provenance`]), and the paragraphs' text facts and labels ([`text_records`]).
//!
//! Must not: change the document (the editing crate writes the item), or decide what a
//! tool reports about a record.

pub mod facts;
mod json;
pub mod keep;
mod read;
mod text;

pub use json::dumps;
pub use read::{provenance, Provenance};
pub use text::{text_records, ByParagraph, TextRecords};

use crate::address::resolve::direct;
use crate::docx::drawings::graphics;
use crate::docx::Docx;
use crate::ids::{valid_long_hex, Entry};
use ooxml_opc::OpcError;
use ooxml_xml::{Element, Node};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// The item's namespace, and the schema its properties name.
pub const NS: &str = "urn:uvrt:docx-agent:provenance:v1";
/// The relationship from the main part to a custom XML item.
pub const REL_CUSTOM_XML: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
/// The relationship from a custom XML item to its properties.
pub const REL_CUSTOM_XML_PROPS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps";
/// The namespace of a custom XML item's properties (`ds:datastoreItem`).
pub const DS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/customXml";
/// A custom XML item's content type (Word's `Default` for `.xml`), and its properties'.
pub const ITEM_TYPE: &str = "application/xml";
/// A custom XML item's properties' content type.
pub const PROPS_TYPE: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";
/// The record format's version, `v` in every record.
pub const VERSION: u64 = 1;

/// What an entry's key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A table, by its durable id (`t:<paraId>`, a story before it outside the body).
    Table,
    /// A chart, by its drawing's id (`d:<docPr id>`).
    Chart,
    /// A paragraph's text facts, by its durable id (`p:<paraId>`).
    Text,
}

/// What `key` names (docx-agent's `kind_of`): `None` for another form.
#[must_use]
pub fn kind_of(key: &str) -> Option<Kind> {
    let local = if key.starts_with("d:") {
        key
    } else {
        key.rsplit_once('/').map_or(key, |(_, l)| l)
    };
    if let Some(hex) = local.strip_prefix("t:") {
        if !local.contains('#') && valid_long_hex(hex) {
            return Some(Kind::Table);
        }
    }
    if key.starts_with("d:") && !key.contains('#') {
        return Some(Kind::Chart);
    }
    match local.strip_prefix("p:") {
        Some(hex) if valid_long_hex(hex) => Some(Kind::Text),
        _ => None,
    }
}

/// The custom XML item holding the records: the first `customXml` target of the main
/// part whose root is `prov:sources` (an item that is missing or not XML is passed over).
///
/// # Errors
/// When the main part's relationships are not XML.
pub fn item(docx: &Docx) -> Result<Option<String>, OpcError> {
    let relationships = docx.package.relationships(&docx.main)?;
    for target in relationships.targets_of_type(REL_CUSTOM_XML) {
        let Ok(Some(tree)) = docx.package.tree(target) else {
            continue;
        };
        if tree.root().name().is(Some(NS), "sources") {
            return Ok(Some(target.to_owned()));
        }
    }
    Ok(None)
}

/// The item's entries (`prov:source` children of its root), in order.
pub fn entries(root: &Element) -> impl Iterator<Item = &Arc<Element>> {
    root.child_elements().filter(|e| e.name().is(Some(NS), "source"))
}

/// An entry's text (lxml's `.text`: the text before its first child element).
#[must_use]
pub fn entry_text(entry: &Element) -> String {
    let mut out = String::new();
    for node in entry.children().iter() {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Element(_) => break,
            _ => {}
        }
    }
    out
}

/// The record a text holds (`ooxml_edit.provenance.loads`): a JSON object with a whole
/// number `v` of at least 1; `None` for anything else.
#[must_use]
pub fn record(text: &str) -> Option<Map<String, Value>> {
    let Ok(Value::Object(found)) = serde_json::from_str::<Value>(text) else {
        return None;
    };
    found.get("v").and_then(Value::as_u64).filter(|v| *v >= 1)?;
    Some(found)
}

/// A text entry's record: the paragraph's text when it was kept, its facts and labels.
#[derive(Debug, Clone, PartialEq)]
pub struct TextEntry {
    /// The paragraph's text the places count in.
    pub paragraph: String,
    /// Its facts.
    pub facts: Vec<Value>,
    /// Its labels.
    pub labels: Vec<Value>,
}

impl TextEntry {
    /// The record an entry's text holds (docx-agent's `_text_entry`), `None` when it holds
    /// no text record.
    #[must_use]
    pub fn read(text: &str) -> Option<TextEntry> {
        let found = record(text)?;
        let paragraph = found.get("paragraph")?.as_str()?.to_owned();
        let facts = found.get("facts")?.as_array()?.clone();
        let labels = match found.get("labels") {
            None => Vec::new(),
            Some(labels) => labels.as_array()?.clone(),
        };
        Some(TextEntry {
            paragraph,
            facts,
            labels,
        })
    }

    /// The entry's record as docx-agent writes it (`_text_record`): its text, facts and,
    /// when it has any, labels.
    #[must_use]
    pub fn record(&self) -> Value {
        let mut out = Map::new();
        out.insert("v".into(), Value::from(VERSION));
        out.insert("paragraph".into(), Value::from(self.paragraph.as_str()));
        out.insert("facts".into(), Value::Array(self.facts.clone()));
        if !self.labels.is_empty() {
            out.insert("labels".into(), Value::Array(self.labels.clone()));
        }
        Value::Object(out)
    }
}

/// The block `key` names now (docx-agent's `_resolve`): the id its renames lead to while
/// that names a block, else `key` as it stands.
#[must_use]
pub fn resolve_followed<'d>(
    docx: &'d Docx,
    aliases: &BTreeMap<String, String>,
    key: &str,
) -> Option<crate::address::resolve::Block<'d>> {
    let mut seen = HashSet::new();
    let mut target = key;
    while let Some(next) = aliases.get(target) {
        if !seen.insert(target) {
            break;
        }
        target = next;
    }
    if target != key {
        if let Some(found) = direct(docx, target) {
            return Some(found);
        }
    }
    direct(docx, key)
}

/// The paragraph a text entry's key names now, renames followed (docx-agent's
/// `_text_paragraph`): its element and its id.
#[must_use]
pub fn text_paragraph(docx: &Docx, aliases: &BTreeMap<String, String>, key: &str) -> Option<(Arc<Element>, String)> {
    let (part, entry) = resolve_followed(docx, aliases, key)?;
    let Entry::Paragraph(k) = entry else { return None };
    let paragraph = part.ids.paragraphs().get(k)?;
    Some((paragraph.element, paragraph.id))
}

/// Whether the table, chart or paragraph `key` is in the file (docx-agent's
/// `_provenance_target` with `accepted=False`).
#[must_use]
pub fn target_in_file(docx: &Docx, aliases: &BTreeMap<String, String>, key: &str) -> bool {
    match kind_of(key) {
        Some(Kind::Text) => text_paragraph(docx, aliases, key).is_some(),
        Some(Kind::Table) => matches!(direct(docx, key), Some((_, Entry::Table(_)))),
        Some(Kind::Chart) => graphics(docx)
            .into_iter()
            .find(|d| d.id == key)
            .is_some_and(|d| d.kind == "chart"),
        None => false,
    }
}

#[cfg(test)]
mod tests;
