//! Owns: what `check`'s `fields` says of a version (docx-agent's `_field_facts`): how many
//! fields of each kind there are, the tables of contents that list nothing, and the
//! cross references (`REF`, `PAGEREF`, `NOTEREF`) to a bookmark no story holds.
//!
//! Must not: change the document, or lay it out (a field's result is a cache: the facts say
//! so, they do not compute it).

use super::scan::all_fields;
use super::update::empty_tocs;
use crate::{Result, WordDoc};
use ooxml_word_model::docx::{casefold, Docx};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use std::collections::HashSet;

/// The facts of a version's fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FieldFacts {
    /// Each field kind and how many fields are of it, in the order each kind is first met.
    pub counts: Vec<(String, usize)>,
    /// The tables of contents that list nothing, each with what to do.
    pub empty_toc: Vec<String>,
    /// The cross references to a bookmark no story holds.
    pub missing_bookmarks: Vec<String>,
}

/// The field kinds whose first argument names a bookmark.
const TO_BOOKMARKS: [&str; 3] = ["REF", "PAGEREF", "NOTEREF"];

/// The facts of `doc`'s fields, story by story in document order.
///
/// # Errors
/// When the version cannot be read.
pub fn field_facts(doc: &WordDoc) -> Result<FieldFacts> {
    let docx = doc.docx()?;
    let fields = all_fields(doc, &docx);
    let mut facts = FieldFacts::default();
    for located in fields.iter() {
        let keyword = located.found.keyword();
        match facts.counts.iter_mut().find(|(k, _)| *k == keyword) {
            Some((_, count)) => *count += 1,
            None => facts.counts.push((keyword, 1)),
        }
    }
    facts.empty_toc = empty_tocs(doc, &docx, None);
    let names = bookmark_names(&docx);
    let folded: HashSet<String> = names.iter().map(|n| casefold(n)).collect();
    for located in fields.iter() {
        let keyword = located.found.keyword();
        if !TO_BOOKMARKS.contains(&keyword.as_str()) {
            continue;
        }
        let name = located.found.instruction.split_whitespace().nth(1).unwrap_or("");
        let bare = name.strip_prefix("bm:").unwrap_or(name);
        if !names.contains(bare) && !folded.contains(&casefold(bare)) {
            facts.missing_bookmarks.push(format!(
                "{}: {keyword} to a missing bookmark {}",
                located.id(),
                ooxml_base::pyfmt::repr_str(name)
            ));
        }
    }
    Ok(facts)
}

/// Every bookmark's name in every story (`_bookmark_markers`' keys; a start without a name
/// is `""`).
fn bookmark_names(docx: &Docx) -> HashSet<String> {
    let mut out = HashSet::new();
    for part in &docx.parts {
        starts(part.root(), &mut out);
    }
    out
}

fn starts(element: &Element, out: &mut HashSet<String>) {
    if element.name().is(Some(W), "bookmarkStart") {
        out.insert(element.attribute(Some(W), "name").unwrap_or("").to_owned());
    }
    for child in element.child_elements() {
        starts(child, out);
    }
}
