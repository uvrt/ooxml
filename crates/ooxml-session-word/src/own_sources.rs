//! Owns: what a Word document says of a source value that names no rows (round 56, R120;
//! docx-agent's `table_data.own_source` and `source_hint`, ported with their words): a value
//! naming one of its own blocks -- `"a table"` (`t:60D6847F`), `"a paragraph"`, `"a content
//! control"`, or `"part of a table"` for a table's id with more after it -- and a drawing's id
//! given for a dataset (`d:3`, the form Word names drawings by), here or not.
//!
//! Must not: resolve rows (a Word document gives none) or change the document.

use crate::host_reads::{block_kind_of, has_drawing_of};
use ooxml_word_edit::WordDoc;

/// What `address` names in `doc` that gives no rows; `None` when it names none of its blocks.
pub(crate) fn own_source(doc: &WordDoc, address: &str) -> Option<String> {
    if !["t:", "p:", "p@", "cc:", "cc@"].iter().any(|p| address.starts_with(p)) {
        return None;
    }
    let base = address.split_once('/').map(|(b, _)| b);
    let kind = |id: &str| -> Option<&'static str> {
        match block_kind_of(doc, id).ok().flatten()? {
            "table" => Some("a table"),
            "paragraph" => Some("a paragraph"),
            "content control" => Some("a content control"),
            _ => None,
        }
    };
    if let Some(what) = kind(address) {
        return Some(what.to_owned());
    }
    let what = kind(base?)?;
    Some(format!("part of {what}"))
}

/// What `value`, given as a source, is in `doc` when it is a drawing's id; `None` otherwise.
pub(crate) fn source_hint(doc: &WordDoc, value: &str) -> Option<String> {
    let drawing = value.strip_prefix("d:").is_some_and(|rest| {
        let (number, extra) = rest.split_once('#').unwrap_or((rest, "0"));
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        digits(number) && digits(extra)
    });
    if !drawing {
        return None;
    }
    let tail = "not a dataset: a source is a dataset d:<id>, its id starting with a letter, or a table t:<name>";
    if has_drawing_of(doc, value).unwrap_or(false) {
        Some(format!("{value} is a drawing in this document, {tail}"))
    } else {
        Some(format!("{value} is a drawing's id (none here), {tail}"))
    }
}
