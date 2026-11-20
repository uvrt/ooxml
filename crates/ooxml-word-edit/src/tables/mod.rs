//! Owns: tables formatted as docx-agent's `edit/table_format.py` formats them --
//! [`FormatTable`], the edits `word_format_table` makes in one call (a table's properties,
//! a grid column's width, a row's height and breaking, a cell's formatting), each written
//! as its `Document` method writes it, in order, as one step -- with what they share: a
//! table's shape on an editable copy ([`shape`]), its properties ([`props`]) and Word's
//! records of them when tracked ([`tracker`]); and [`EditTable`], a table's rows and
//! columns inserted, copied and deleted, its cells merged and split; rows written against
//! the rows around them ([`rows_unlike`]) and empty cells written given their column's
//! formatting ([`LikeTheColumn`]).
//!
//! Must not: write but through the sink, or decide what a tool's arguments mean (the tool
//! does).

mod columns;
mod copy_row;
mod like_column;
mod merge;
mod new_column;
mod new_row;
mod props;
mod row_look;
mod rows;
pub(crate) mod shape;
mod split;
pub(crate) mod tracker;

pub(crate) use rows::placed;

pub use like_column::{ColumnCopy, LikeTheColumn};
pub use props::{CellValues, Look, TableValues, TableWidth};
pub use row_look::{
    row_cells, row_shown, rows_unlike, CellUnlike, FormatCall, LookField, RowUnlike, TextLook, ROW_WINDOW,
};
pub use rows::{pending_structure, EditTable, TableChange, TableChanged};

use crate::doc::Key;
use crate::names::{W, W16DU};
use crate::policy::{RevisionPolicy, Stamp};
use crate::set_text::block_key;
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::stamp_document;
use crate::validate::Orders;
use crate::work::{scope_above, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::direct;
use ooxml_word_model::docx::{at_path, find_path, Docx};
use ooxml_word_model::ids::Entry;
use ooxml_xml::Element;
use std::sync::Arc;
use tracker::TableTracker;

/// One edit of [`FormatTable`], as one of docx-agent's `Document` methods makes it.
#[derive(Debug, Clone, PartialEq)]
pub enum TableFormat {
    /// The table's properties (`set_table`).
    Table(TableValues),
    /// A grid column's width, in points (`set_column_width`).
    ColumnWidth {
        /// The grid column.
        column: usize,
        /// Points.
        width: f64,
    },
    /// A row's height (points, exactly or at least) and breaking (`set_row`).
    Row {
        /// The row.
        row: usize,
        /// The height and whether it is exact.
        height: Option<(f64, bool)>,
        /// Whether the row may not break across pages.
        cant_split: Option<bool>,
    },
    /// The cell over a grid position (`format_cell`).
    Cell {
        /// The row.
        row: usize,
        /// The grid column.
        column: usize,
        /// What to set.
        values: CellValues,
    },
}

/// Format a table: its edits in order, one step.
#[derive(Debug, Clone)]
pub struct FormatTable {
    /// The table (an id, an alias).
    pub table: String,
    /// The edits.
    pub formats: Vec<TableFormat>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What one edit of a [`FormatTable`] did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableEdit {
    /// The table's id after the edit.
    pub id: String,
    /// Ids renamed (stamping, by the first edit), old to new.
    pub renamed: Vec<(String, String)>,
    /// What it did that the caller did not ask for.
    pub warnings: Vec<String>,
}

/// What a [`FormatTable`] did: each edit's outcome, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableFormatted {
    /// The edits.
    pub edits: Vec<TableEdit>,
}

/// A table found: its part, its place there and its id.
pub(crate) struct Found {
    pub(crate) part: String,
    pub(crate) path: Vec<u32>,
    pub(crate) id: String,
    pub(crate) element: Arc<Element>,
}

/// The table `target` names (an id, or an alias followed).
pub(crate) fn find_table(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Found> {
    let followed = doc.ids().follow(target);
    let block = (followed != target)
        .then(|| direct(docx, &followed))
        .flatten()
        .or_else(|| direct(docx, target));
    let Some((story, Entry::Table(k))) = block else {
        return Err(EditError::Refused(format!("{target} is not a table")));
    };
    let found = story
        .ids
        .tables()
        .get(k)
        .ok_or_else(|| EditError::Refused(format!("{target} is not a table")))?;
    Ok(Found {
        part: story.part.clone(),
        path: found.path.to_vec(),
        id: found.id.clone(),
        element: found.element.clone(),
    })
}

/// The width (twips) a table sits in (docx-agent's `_room_for`): its cell's text width,
/// else its section's, else A4's with one-inch margins.
fn room_for(doc: &WordDoc, docx: &Docx, found: &Found) -> i64 {
    let Some(root) = docx.part(&found.part).map(|p| p.root().clone()) else {
        return 9026;
    };
    let cell = (1..found.path.len())
        .rev()
        .filter_map(|n| at_path(&root, found.path.get(..n)?))
        .find(|e| e.name().is(Some(W), "tc"));
    if let Some(cell) = cell {
        let width = find_path(cell, &["tcPr", "tcW"]);
        let twips = width
            .filter(|w| w.attribute(Some(W), "type").is_none_or(|t| t == "dxa"))
            .and_then(|w| w.attribute(Some(W), "w")?.trim().parse::<i64>().ok())
            .filter(|w| *w != 0);
        return twips.map_or(4000, |w| (w - 216).max(360));
    }
    if found.part != docx.main {
        return 9026;
    }
    let Ok(section) = crate::sections::find_section(doc, docx, &found.id) else {
        return 9026;
    };
    let properties = &section.section.properties;
    let int = |node: Option<&Element>, local: &str| {
        node.and_then(|n| n.attribute(Some(W), local)?.trim().parse::<i64>().ok())
            .unwrap_or(0)
    };
    match properties.find(Some(W), "pgSz") {
        Some(size) => {
            let margins = properties.find(Some(W), "pgMar").map(|m| &**m);
            let width = int(Some(size), "w") - int(margins, "left") - int(margins, "right") - int(margins, "gutter");
            if width == 0 {
                9026
            } else {
                width
            }
        }
        None => 9026,
    }
}

impl FormatTable {
    /// Each edit's arguments against the table as it is (docx-agent's checks before an
    /// edit); `id` the table's id then.
    fn check(&self, docx: &Docx, found: &Found) -> Result<()> {
        let work = Work::of(&found.element, Vec::new());
        let table = work.root();
        let rows = shape::own_rows(&work, table).len();
        let columns = shape::grid_widths(&work, table).len();
        for format in &self.formats {
            match format {
                TableFormat::Table(values) => {
                    if let Some(style) = &values.style {
                        crate::builtin::resolve(docx, style, "table")?;
                    }
                    if values.header_rows.is_some_and(|h| h > rows) {
                        return Err(EditError::Refused(
                            "header_rows is between 0 and the number of rows".to_owned(),
                        ));
                    }
                }
                TableFormat::ColumnWidth { column, .. } if *column >= columns => {
                    return Err(EditError::Refused(format!("{} has no column {column}", found.id)));
                }
                TableFormat::Row { row, .. } if *row >= rows => {
                    return Err(EditError::Refused(format!("{} has no row {row}", found.id)));
                }
                TableFormat::Cell { row, column, .. } if shape::cell_over(&work, table, *row, *column).is_none() => {
                    return Err(EditError::Refused(format!(
                        "{} has no cell at row {row}, column {column}",
                        found.id
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Command<WordDoc> for FormatTable {
    type Out = TableFormatted;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        let found = find_table(doc, &docx, &self.table)?;
        self.check(&docx, &found)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<TableFormatted> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// What the edits wrote, for the sink.
pub(crate) struct Written {
    /// The revision ids the edits gave (tracked).
    pub(crate) numbered: Option<crate::sink::Numbered>,
    /// Whether a record carries a UTC date.
    pub(crate) dated: bool,
    /// Each edit's warnings.
    pub(crate) warnings: Vec<Vec<String>>,
}

impl Written {
    /// What a tracked write numbered and whether it dated a record; nothing untracked.
    fn of(tracked: Option<(crate::sink::Numbered, bool)>) -> Written {
        Written {
            numbered: tracked.map(|t| t.0),
            dated: tracked.is_some_and(|t| t.1),
            warnings: Vec::new(),
        }
    }
}

impl FormatTable {
    /// The edits, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<TableFormatted> {
        let docx = sink.docx()?;
        let found = find_table(sink.doc(), &docx, &self.table)?;
        self.check(&docx, &found)?;
        let room = room_for(sink.doc(), &docx, &found);
        sink.read(block_key(&found.part, &found.id));
        let renamed = stamp_document(&mut *sink, &docx)?;
        let mut style_ids = Vec::new();
        for format in &self.formats {
            if let TableFormat::Table(TableValues { style: Some(style), .. }) = format {
                style_ids.push(crate::builtin::ensure(sink, style, "table")?);
            }
        }
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::Path(found.part.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &found.path));
        let written = match sink.policy().clone() {
            RevisionPolicy::Tracked(tracking) => {
                let (warnings, numbered, dated) = sink.stamped(tracking, |stamp| {
                    Ok(self.apply(&mut work, Some(stamp), &style_ids, room, orders))
                })?;
                Written {
                    numbered: Some(numbered),
                    dated,
                    warnings,
                }
            }
            RevisionPolicy::Untracked => Written {
                numbered: None,
                dated: false,
                warnings: self.apply(&mut work, None, &style_ids, room, orders),
            },
        };
        sink.keep_written(&found.part, None, &work);
        let new = work.build().ok_or_else(|| EditError::Path(found.part.clone()))?;
        let docx = sink.docx()?;
        let id = docx
            .part(&found.part)
            .and_then(|p| p.ids.table_at(&found.path))
            .map_or_else(|| found.id.clone(), |t| t.id);
        // an edit that leaves the table as it was writes nothing (docx-agent's part, written
        // again unchanged, is the same bytes)
        if new.as_ref() != element.as_ref() {
            write(sink, &found, &id, new, &written)?;
        }
        let edits = written
            .warnings
            .into_iter()
            .enumerate()
            .map(|(k, warnings)| TableEdit {
                id: id.clone(),
                renamed: if k == 0 { renamed.clone() } else { Vec::new() },
                warnings,
            })
            .collect();
        Ok(TableFormatted { edits })
    }

    /// Each edit on the table's copy, recorded by `stamp` when tracked; each one's
    /// warnings.
    fn apply(
        &self,
        work: &mut Work,
        mut stamp: Option<&mut Stamp>,
        style_ids: &[String],
        room: i64,
        orders: &Orders,
    ) -> Vec<Vec<String>> {
        let table = work.root();
        let mut styles = style_ids.iter();
        let mut out = Vec::new();
        for format in &self.formats {
            let mut warnings = Vec::new();
            match format {
                TableFormat::Table(values) => {
                    let style = values.style.as_ref().and_then(|_| styles.next()).map(String::as_str);
                    let tracker = stamp.is_some().then(|| TableTracker::new(work, table, true));
                    props::apply_table(work, table, values, style, room, orders);
                    if let (Some(tracker), Some(stamp)) = (tracker, stamp.as_deref_mut()) {
                        tracker.record(work, stamp, true, true, orders);
                    }
                    if let Some(count) = values.header_rows {
                        props::set_header_rows(work, table, count, orders);
                        if stamp.is_some() {
                            warnings.push("Word does not track header rows: they are applied untracked".to_owned());
                        }
                    }
                }
                TableFormat::ColumnWidth { column, width } => {
                    let tracker = stamp.is_some().then(|| TableTracker::new(work, table, true));
                    props::set_column_width(work, table, *column, props::twips(*width), orders);
                    if let (Some(tracker), Some(stamp)) = (tracker, stamp.as_deref_mut()) {
                        tracker.record(work, stamp, false, true, orders);
                    }
                }
                TableFormat::Row {
                    row,
                    height,
                    cant_split,
                } => {
                    if let Some(found) = shape::own_rows(work, table).get(*row).copied() {
                        props::set_row(work, found, *height, *cant_split, orders);
                    }
                    if stamp.is_some() {
                        warnings.push(
                            "Word does not track a row's height, breaking or header: applied untracked".to_owned(),
                        );
                    }
                }
                TableFormat::Cell { row, column, values } => {
                    let tracker = stamp.is_some().then(|| TableTracker::new(work, table, false));
                    if let Some(cell) = shape::cell_over(work, table, *row, *column) {
                        props::set_cell(work, table, cell, values, orders);
                    }
                    if let (Some(tracker), Some(stamp)) = (tracker, stamp.as_deref_mut()) {
                        tracker.record(work, stamp, false, true, orders);
                    }
                }
            }
            out.push(warnings);
        }
        out
    }
}

/// The table's copy put in place, with its revision ids drawn at commit, `w16du` declared
/// where a record carries a UTC date, and the author among the people.
pub(crate) fn write(
    sink: &mut EditSink<'_, '_>,
    found: &Found,
    id: &str,
    new: Arc<Element>,
    written: &Written,
) -> Result<()> {
    let draws = written
        .numbered
        .map(|n| annotation_draws(&new, n, 0))
        .unwrap_or_default();
    let mut keys = vec![block_key(&found.part, id)];
    if written.numbered.is_some() {
        keys.push(Key::Annotations);
    }
    sink.replace_drawing(&found.part, &found.path, new, keys, draws)?;
    if written.dated {
        sink.ensure_w14(&found.part, false, &[("w16du", W16DU)])?;
    }
    if let (RevisionPolicy::Tracked(tracking), Some(n)) = (sink.policy().clone(), written.numbered) {
        sink.people(
            (n.to > n.from).then_some(tracking.author.as_str()),
            Some(&tracking.author),
        )?;
    }
    Ok(())
}
