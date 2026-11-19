//! Owns: the ids a placed copy carries, as docx-agent's `_stamp` gives them: in document
//! order, each paragraph's and row's paraId and textId and each drawing's anchorId and
//! editId kept where the document does not use them and never did, the rest drawn from the
//! part's sequences (rows and drawings from their own); every id made, and per copied
//! block its own id followed by its paragraphs' (as the source's are listed).
//!
//! Must not: write but through the sink.

use crate::names::{W, W14, WP, WP14};
use crate::placing::Place;
use crate::sink::EditSink;
use crate::stamping::{w14_prefix, wp14_prefix};
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::ids::{hex_value, valid_long_hex};
use ooxml_xml::QName;
use std::sync::Arc;

/// What stamping a copy gave.
pub(crate) struct Stamped {
    /// Every id made, in document order, each once.
    pub(crate) created: Vec<String>,
    /// Per copied block: its own id (`""` where none), then its paragraphs'.
    pub(crate) per_element: Vec<Vec<String>>,
    /// Whether a drawing was stamped (the part declares `wp14` then).
    pub(crate) drawings: bool,
}

/// What a node is stamped as.
enum Stampable {
    Paragraph,
    Row,
    Drawing,
}

/// Stamp the copies `elements`, placed in `work` at `place`.
///
/// # Errors
/// When the ledger cannot be written.
pub(crate) fn stamp(sink: &mut EditSink<'_, '_>, work: &mut Work, place: &Place, elements: &[Id]) -> Result<Stamped> {
    let part = &place.part;
    let tree = sink.doc().tree(part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let w14 = w14_prefix(tree.root(), &place.container);
    let wp14 = wp14_prefix(tree.root(), &place.container);
    let prefix = if place.story == "body" {
        String::new()
    } else {
        format!("{}/", place.story)
    };
    let mut out = Stamped {
        created: Vec::new(),
        per_element: Vec::new(),
        drawings: false,
    };
    for &element in elements {
        let mut first_row: Option<String> = None;
        let mut paragraphs: Vec<String> = Vec::new();
        let mut own: Option<String> = None;
        for node in work.descendants(element) {
            let Some(kind) = stampable(work, node) else { continue };
            let (seed, names) = match kind {
                Stampable::Paragraph => (part.clone(), (w14.as_str(), W14, ["paraId", "textId"])),
                Stampable::Row => (format!("{part}\0row"), (w14.as_str(), W14, ["paraId", "textId"])),
                Stampable::Drawing => (
                    format!("{part}\0drawing"),
                    (wp14.as_str(), WP14, ["anchorId", "editId"]),
                ),
            };
            let first = keep_or_issue(sink, work, node, &seed, names)?;
            match kind {
                Stampable::Paragraph if node == element => own = Some(format!("{prefix}p:{first}")),
                Stampable::Paragraph => paragraphs.push(format!("{prefix}p:{first}")),
                Stampable::Row => {
                    first_row.get_or_insert(first);
                }
                Stampable::Drawing => out.drawings = true,
            }
        }
        if work.is(element, W, "tbl") {
            if let Some(row) = first_row {
                own = Some(format!("{prefix}t:{row}"));
            }
        }
        if own.is_none() {
            own = paragraphs.first().cloned();
        }
        let mut ids = vec![own.unwrap_or_default()];
        ids.extend(paragraphs);
        for id in &ids {
            if !id.is_empty() && !out.created.contains(id) {
                out.created.push(id.clone());
            }
        }
        out.per_element.push(ids);
    }
    Ok(out)
}

fn stampable(work: &Work, node: Id) -> Option<Stampable> {
    if work.is(node, W, "p") {
        Some(Stampable::Paragraph)
    } else if work.is(node, W, "tr") {
        Some(Stampable::Row)
    } else if work.is_any(node, WP, &["inline", "anchor"]) {
        Some(Stampable::Drawing)
    } else {
        None
    }
}

/// `keep_or_issue`: the node's two ids kept or drawn and written; the first.
fn keep_or_issue(
    sink: &mut EditSink<'_, '_>,
    work: &mut Work,
    node: Id,
    seed: &str,
    (prefix, namespace, locals): (&str, &str, [&str; 2]),
) -> Result<String> {
    let raws: Vec<Option<String>> = locals
        .iter()
        .map(|local| work.get(node, Some(namespace), local).map(str::to_owned))
        .collect();
    let values: Vec<Option<(u32, &str)>> = raws
        .iter()
        .map(|raw| {
            let raw = raw.as_deref().filter(|r| valid_long_hex(r))?;
            Some((hex_value(raw)?, raw))
        })
        .collect();
    let ids = sink.keep_or_draw(seed, &values)?;
    for (local, value) in locals.iter().zip(&ids) {
        work.set(node, Arc::new(QName::new(Some(prefix), local, Some(namespace))), value);
    }
    Ok(ids.into_iter().next().unwrap_or_default())
}
