//! Owns: a table's record of where its values came from, kept or dropped as part of the
//! edit that placed them (docx-agent's `store_provenance` with a table's key): the entry's
//! text replaced where it has one, else a new entry at the end -- the item made where the
//! document has none -- or the entry dropped, the item going with its last entry.
//!
//! Reads: the main part's relationships and the item.
//!
//! Must not: decide what the record says (the tool that placed the values does), or follow
//! the other entries (every command's follow-up does, [`super::Followed`]).

use super::item::Draft;
use super::made::make_item;
use crate::sink::EditSink;
use crate::Result;
use ooxml_word_model::docx::provenance::dumps;
use serde_json::{Map, Value};

/// The kind of a table's entry.
const TABLE: &str = "table";

/// The table keyed `key` (its durable id) given `record` as its record through `sink`, or
/// its record dropped (`None`).
///
/// # Errors
/// When the main part's relationships or the item are not XML.
pub(crate) fn store_table_record(
    sink: &mut EditSink<'_, '_>,
    key: &str,
    record: Option<&Map<String, Value>>,
) -> Result<()> {
    let mut docx = sink.docx()?;
    let mut draft = Draft::open(sink, &docx)?;
    match record {
        None => {
            if let Some(draft) = draft.as_mut() {
                draft.drop_entry(TABLE, key);
            }
        }
        Some(record) => {
            if draft.is_none() {
                make_item(sink, &docx, key)?;
                docx = sink.docx()?;
                draft = Draft::open(sink, &docx)?;
            }
            if let Some(draft) = draft.as_mut() {
                draft.store(TABLE, key, &dumps(&Value::Object(record.clone())));
            }
        }
    }
    match draft {
        Some(draft) => draft.finish(sink, &docx),
        None => Ok(()),
    }
}
