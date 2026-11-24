//! Owns: what accepting or rejecting tracked changes does to the tables' records
//! (docx-agent's `_follow_review`, `tools/review.py`): a record describes its table with
//! its tracked changes accepted, so accepting keeps it -- under the table's new id when
//! accepting renamed the table (a tracked first row inserted or deleted) -- and rejecting
//! anything in a recorded table drops its record (the values put back may not be the
//! source's). Each record dropped or moved as docx-agent's `store_provenance` does it,
//! the entries whose object is gone pruned after each.
//!
//! Must not: make the item again: where moving the last record would empty it first (as
//! `store_provenance` does, a new item named after the new id), the review is refused as
//! unsupported and the host's handler answers.

use super::item::{Draft, Fate};
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_word_model::address::resolve::direct;
use ooxml_word_model::docx::provenance::{kind_of, provenance, resolve_followed, target_in_file, Kind};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use ooxml_xml::Element;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A table's record the review may move or drop: its key, the table as it was, its record
/// (`None`: left out on reading).
struct Recorded {
    key: String,
    table: Arc<Element>,
    record: Option<String>,
}

/// The tables' records after a review (`accept`: else a rejection) through `sink`.
///
/// # Errors
/// [`EditError::Unsupported`] where the item would be made again; when a part is not XML.
pub(crate) fn follow_review(sink: &mut EditSink<'_, '_>, accept: bool) -> Result<()> {
    let now = sink.docx()?;
    let Some(mut draft) = Draft::open(sink, &now)? else {
        return Ok(());
    };
    let base = sink.base().docx()?;
    let aliases = sink.doc().ids().aliases().clone();
    let records = provenance(&base)?;
    let recorded: Vec<Recorded> = draft
        .keys()
        .into_iter()
        .filter(|key| key.rsplit_once('/').map_or(key.as_str(), |(_, l)| l).starts_with("t:"))
        .filter_map(|key| {
            let table = table(&base, &key)?;
            let record = records.text(&key);
            Some(Recorded { key, table, record })
        })
        .collect();
    for Recorded { key, table, record } in recorded {
        let found = resolve_followed(&now, &aliases, &key).and_then(|(part, entry)| match entry {
            Entry::Table(k) => part.ids.tables().get(k).map(|t| (t.id, t.element)),
            Entry::Paragraph(_) => None,
        });
        if !accept {
            if found.as_ref().is_none_or(|(_, element)| *element != table) {
                drop_record(&mut draft, &now, &aliases, &key);
            }
            continue;
        }
        let (Some((id, _)), Some(record)) = (found, record) else {
            continue;
        };
        if id == key {
            continue;
        }
        drop_record(&mut draft, &now, &aliases, &key);
        if draft.entries().next().is_none() {
            return Err(EditError::Unsupported(
                "word_changes accept moving the document's only table record (its item is made again)".to_owned(),
            ));
        }
        draft.store("table", &id, &record);
        prune(&mut draft, &now, &aliases);
    }
    draft.finish(sink, &now)
}

/// The table `key` names as written in `docx` (docx-agent's `_direct`).
fn table(docx: &Docx, key: &str) -> Option<Arc<Element>> {
    match direct(docx, key)? {
        (part, Entry::Table(k)) => part.ids.tables().head(k).map(|t| t.element),
        (_, Entry::Paragraph(_)) => None,
    }
}

/// docx-agent's `store_provenance(key, None)`: the entries keyed `key`, or the id `key`'s
/// table goes by now, dropped; then the entries whose object is gone.
fn drop_record(draft: &mut Draft, now: &Docx, aliases: &BTreeMap<String, String>, key: &str) {
    let current = resolve_followed(now, aliases, key).and_then(|(part, entry)| match entry {
        Entry::Table(k) => part.ids.tables().head(k).map(|t| t.id),
        Entry::Paragraph(_) => None,
    });
    let current = current.filter(|id| kind_of(id) == Some(Kind::Table));
    draft.settle(|entry| {
        let named = entry.attribute(None, "key").unwrap_or_default();
        if named == key || current.as_deref() == Some(named) {
            Fate::Dropped
        } else {
            Fate::Kept
        }
    });
    prune(draft, now, aliases);
}

/// docx-agent's `_prune_provenance` on the reviewed version.
fn prune(draft: &mut Draft, now: &Docx, aliases: &BTreeMap<String, String>) {
    draft.settle(|entry| {
        if target_in_file(now, aliases, entry.attribute(None, "key").unwrap_or_default()) {
            Fate::Kept
        } else {
            Fate::Dropped
        }
    });
}
