//! Owns: tables formatted as docx-agent's `edit/table_format.py` formats them --
//! [`FormatTable`], the edits `word_format_table` makes in one call (a table's properties,
//! a grid column's width, a row's height and breaking, a cell's formatting), each written
//! as its `Document` method writes it, in order, as one step -- with what they share: a
//! table's shape on an editable copy ([`shape`]), its properties ([`props`]) and Word's
//! records of them when tracked ([`tracker`]); and [`EditTable`], a table's rows and
//! columns inserted, copied and deleted, its cells merged and split.
//!
//! Must not: write but through the sink, or decide what a tool's arguments mean (the tool
//! does).

mod body_lines;
mod columns;
mod copy_row;
mod keeps;
mod merge;
mod new_column;
mod new_row;
mod props;
mod row_look;
mod rows;
pub(crate) mod shape;
mod split;
pub(crate) mod tracker;
mod window;

pub(crate) use rows::placed;

pub use body_lines::{body_lines, body_width, BodyLines, BodyWidth};
pub use keeps::{loose_paragraph_after, table_keeps, table_keeps_at, TableKeeps};
pub use props::{CellValues, Look, TableValues, TableWidth};
pub use row_look::{
    row_cells, row_shown, rows_unlike, CellUnlike, FormatCall, LookField, RowUnlike, TextLook, ROW_WINDOW,
};
pub use rows::{pending_structure, EditTable, TableChange, TableChanged};

use crate::doc::Key;
use crate::fields::{EditLayout, Unknown};
use crate::names::{W, W16DU};
use crate::policy::{RevisionPolicy, Stamp};
use crate::props::PropertyTracker;
use crate::set_text::block_key;
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::stamp_document;
use crate::validate::Orders;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::direct;
use ooxml_word_model::docx::formatting::Formatting;
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
    /// Word's AutoFit to Window at `percent` of the room (`fit_table_to_window`; [`window`]).
    Window {
        /// Percent, above 0 and at most 100.
        percent: f64,
    },
    /// The rows' keeps: `keep_together` and `keep_with_next` as `word_format_table` gives
    /// them (`set_keep_together`, `set_table_keep_with_next`; [`keeps::plan`]).
    Keeps {
        /// Keep the table on one page (`Some(false)`: release it).
        keep_together: Option<bool>,
        /// Keep it with the next block too.
        keep_with_next: Option<bool>,
    },
}

/// Format a table: its edits in order, one step.
#[derive(Clone)]
pub struct FormatTable<'l> {
    /// The table (an id, an alias).
    pub table: String,
    /// The edits.
    pub formats: Vec<TableFormat>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
    /// The layout of the document's versions: a body table kept together
    /// ([`TableFormat::Keeps`], `keep_together` on) is laid out after the edits, for the
    /// pages it is drawn on ([`TableFormatted::pages`]); without one, that is
    /// [`EditError::Unsupported`].
    pub layout: Option<EditLayout<'l>>,
}

impl std::fmt::Debug for FormatTable<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormatTable")
            .field("table", &self.table)
            .field("formats", &self.formats)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// What one edit of a [`FormatTable`] did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableEdit {
    /// The table's id after the edit.
    pub id: String,
    /// Whether the edit changed the table (a keep already as asked does not).
    pub changed: bool,
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
    /// A body table kept together: the 0-based pages it is drawn on after the edits, or why
    /// the layout cannot say (docx-agent's `_taller_than_page` reads them); `None` where not
    /// asked, or the table is not the body's.
    pub pages: Option<std::result::Result<Vec<usize>, Unknown>>,
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

impl FormatTable<'_> {
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
                TableFormat::Window { .. } if columns == 0 => {
                    return Err(EditError::Refused("the table has no grid of columns to fit".to_owned()));
                }
                TableFormat::Window { percent } if !(*percent > 0.0 && *percent <= 100.0) => {
                    return Err(EditError::Refused(
                        "a width in percent is above 0 and at most 100".to_owned(),
                    ));
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

impl Command<WordDoc> for FormatTable<'_> {
    type Out = TableFormatted;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        let found = find_table(doc, &docx, &self.table)?;
        self.check(&docx, &found)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<TableFormatted> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let mut out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        let kept = self.formats.iter().any(|f| {
            matches!(
                f,
                TableFormat::Keeps {
                    keep_together: Some(true),
                    ..
                }
            )
        });
        if kept {
            out.pages = keeps::table_pages(&sink, self.layout, &self.table)?;
        }
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

impl FormatTable<'_> {
    /// The edits, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<TableFormatted> {
        let docx = sink.docx()?;
        let found = find_table(sink.doc(), &docx, &self.table)?;
        self.check(&docx, &found)?;
        let room = room_for(sink.doc(), &docx, &found);
        sink.read(block_key(&found.part, &found.id));
        // keeps first in the call are planned before the document is stamped (docx-agent's
        // `_write_keeps` stamps only what it writes); a keep already as asked, alone, writes
        // nothing at all
        let keeps_first = matches!(self.formats.first(), Some(TableFormat::Keeps { .. }));
        let mut plans = if keeps_first {
            self.keep_plans(&docx, &found)?
        } else {
            Vec::new()
        };
        if keeps_first && self.formats.len() == 1 && plans.iter().flatten().all(|p| p.write.changes.is_empty()) {
            let edits = plans.into_iter().flatten().map(|p| TableEdit {
                id: found.id.clone(),
                changed: false,
                renamed: Vec::new(),
                warnings: p.write.warnings,
            });
            return Ok(TableFormatted {
                edits: edits.collect(),
                pages: None,
            });
        }
        let renamed = stamp_document(&mut *sink, &docx)?;
        let (docx, found) = if renamed.is_empty() {
            (docx, found)
        } else {
            let docx = sink.docx()?;
            let found = find_table(sink.doc(), &docx, &self.table)?;
            (docx, found)
        };
        if !keeps_first {
            plans = self.keep_plans(&docx, &found)?;
        }
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
        let inputs = Inputs {
            style_ids: &style_ids,
            room,
            orders,
            plans: &plans,
            window: self.window_inputs(sink, &found, room)?,
        };
        let (written, unchanged) = self.applied(sink, &mut work, &inputs)?;
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
        let edits = self.outcomes((written.warnings, &unchanged), &plans, &id, &renamed);
        Ok(TableFormatted { edits, pages: None })
    }

    /// Each edit's outcome: the table's id after them, the renames (the first edit's), its
    /// warnings -- a released last row's said with that id -- and whether it changed the
    /// table (a keep already as asked did not).
    fn outcomes(
        &self,
        (warnings, unchanged): (Vec<Vec<String>>, &[bool]),
        plans: &[Option<KeepPlan>],
        id: &str,
        renamed: &[(String, String)],
    ) -> Vec<TableEdit> {
        warnings
            .into_iter()
            .enumerate()
            .zip(&self.formats)
            .map(|((k, mut warnings), format)| {
                let plan = plans.get(k).and_then(Option::as_ref);
                if plan.is_some_and(|p| p.released) {
                    warnings.push(keeps::released_note(id));
                }
                let kept = plan.is_some_and(|p| !p.write.changes.is_empty());
                let fits = unchanged.get(k).copied().unwrap_or(false);
                if fits {
                    warnings.push(format!("{id} already fits the window: nothing changed"));
                }
                TableEdit {
                    id: id.to_owned(),
                    changed: !fits && (!matches!(format, TableFormat::Keeps { .. }) || kept),
                    renamed: if k == 0 { renamed.to_vec() } else { Vec::new() },
                    warnings,
                }
            })
            .collect()
    }

    /// Each edit on the table's copy, recorded by `stamp` when tracked; each one's
    /// warnings.
    fn apply(
        &self,
        work: &mut Work,
        mut stamp: Option<&mut Stamp>,
        inputs: &Inputs<'_>,
    ) -> (Vec<Vec<String>>, Vec<bool>) {
        let Inputs {
            style_ids,
            room,
            orders,
            plans,
            ..
        } = *inputs;
        let mut unchanged = vec![false; self.formats.len()];
        let table = work.root();
        // the rows' paragraphs found before any edit moves them (a row's new `w:trPr`)
        let leads: Vec<Vec<(Id, Option<bool>)>> = plans
            .iter()
            .map(|p| {
                p.as_ref()
                    .map_or_else(Vec::new, |p| keeps::leads(work, table, &p.rows, &p.write.changes))
            })
            .collect();
        let mut styles = style_ids.iter();
        let mut out = Vec::new();
        for (k, format) in self.formats.iter().enumerate() {
            let mut warnings = Vec::new();
            match format {
                TableFormat::Keeps { .. } => {
                    if let (Some(plan), Some(leads)) = (plans.get(k).and_then(Option::as_ref), leads.get(k)) {
                        write_keeps(work, stamp.as_deref_mut(), leads, orders);
                        warnings.extend(plan.write.warnings.iter().cloned());
                    }
                }
                TableFormat::Table(values) => {
                    let style = values.style.as_ref().and_then(|_| styles.next()).map(String::as_str);
                    warnings.extend(set_table(work, stamp.as_deref_mut(), values, (style, room), orders));
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
                TableFormat::Window { percent } => {
                    if let Some(window) = &inputs.window {
                        let fits = fit_window(work, stamp.as_deref_mut(), window, *percent, orders);
                        if let Some(flag) = unchanged.get_mut(k) {
                            *flag = fits;
                        }
                    }
                }
            }
            out.push(warnings);
        }
        (out, unchanged)
    }
}

/// A keep format's plan: the rows' keeps it was planned on, what it writes, and whether
/// it releases a last row keeping alone.
pub(crate) struct KeepPlan {
    rows: Vec<keeps::RowKeep>,
    write: keeps::KeepWrite,
    released: bool,
}

impl FormatTable<'_> {
    /// Each keep format's plan, in the formats' places (`None` for the others), against
    /// the table at `found`.
    fn keep_plans(&self, docx: &Docx, found: &Found) -> Result<Vec<Option<KeepPlan>>> {
        let mut out = Vec::new();
        for format in &self.formats {
            let TableFormat::Keeps {
                keep_together,
                keep_with_next,
            } = format
            else {
                out.push(None);
                continue;
            };
            let rows = keeps::row_keeps(docx, found)?;
            let work = Work::of(&found.element, Vec::new());
            let own = shape::own_rows(&work, work.root()).len();
            let (write, released) = keeps::plan(&rows, &found.id, own, *keep_together, *keep_with_next)?;
            out.push(Some(KeepPlan { rows, write, released }));
        }
        Ok(out)
    }
}

/// What every edit of one [`FormatTable`] reads.
struct Inputs<'a> {
    style_ids: &'a [String],
    room: i64,
    orders: &'a Orders,
    plans: &'a [Option<KeepPlan>],
    window: Option<WindowInputs>,
}

/// What AutoFit to Window reads beside the table: the room it fits, whether the document
/// lays tables out as mode 15 does, and the styles (a style given in the same call too).
struct WindowInputs {
    room: window::Room,
    modern: bool,
    formatting: Formatting,
}

/// The table's properties (`set_table`), its style as `style`, a percent width of `room`
/// (tracked: recorded; header rows applied untracked, said).
fn set_table(
    work: &mut Work,
    stamp: Option<&mut Stamp>,
    values: &TableValues,
    (style, room): (Option<&str>, i64),
    orders: &Orders,
) -> Vec<String> {
    let table = work.root();
    let tracked = stamp.is_some();
    let tracker = tracked.then(|| TableTracker::new(work, table, true));
    props::apply_table(work, table, values, style, room, orders);
    if let (Some(tracker), Some(stamp)) = (tracker, stamp) {
        tracker.record(work, stamp, true, true, orders);
    }
    let mut warnings = Vec::new();
    if let Some(count) = values.header_rows {
        props::set_header_rows(work, table, count, orders);
        if tracked {
            warnings.push("Word does not track header rows: they are applied untracked".to_owned());
        }
    }
    warnings
}

/// The rows' keeps written on their paragraphs `leads` (tracked: each paragraph's old
/// properties recorded).
fn write_keeps(work: &mut Work, stamp: Option<&mut Stamp>, leads: &[(Id, Option<bool>)], orders: &Orders) {
    let roots: Vec<Id> = leads.iter().map(|l| l.0).collect();
    let tracker = stamp.is_some().then(|| PropertyTracker::start(work, &roots));
    keeps::write(work, leads, orders);
    if let (Some(tracker), Some(stamp)) = (tracker, stamp) {
        tracker.record(work, stamp, orders);
    }
}

/// AutoFit to Window on the table of `work` (tracked: its properties, cells and grid
/// recorded); whether it was already so (nothing changed).
fn fit_window(
    work: &mut Work,
    stamp: Option<&mut Stamp>,
    inputs: &WindowInputs,
    percent: f64,
    orders: &Orders,
) -> bool {
    let table = work.root();
    let before = work.build_element(table);
    let tracker = stamp.is_some().then(|| TableTracker::new(work, table, false));
    #[allow(clippy::cast_possible_truncation)]
    let pct = (percent * 50.0).round_ties_even() as i64;
    // a table with no grid is refused before (check)
    let _ = window::write(
        work,
        table,
        (pct, inputs.room, inputs.modern),
        &inputs.formatting,
        orders,
    );
    if let (Some(tracker), Some(stamp)) = (tracker, stamp) {
        tracker.record(work, stamp, true, true, orders);
    }
    before == work.build_element(table)
}

impl FormatTable<'_> {
    /// What AutoFit to Window reads, where the call fits the table to the window.
    fn window_inputs(&self, sink: &EditSink<'_, '_>, found: &Found, room: i64) -> Result<Option<WindowInputs>> {
        if !self.formats.iter().any(|f| matches!(f, TableFormat::Window { .. })) {
            return Ok(None);
        }
        let docx = sink.docx()?;
        Ok(Some(WindowInputs {
            room: window::window_room(sink.doc(), &docx, found, room),
            modern: crate::template::compatibility_mode(sink.doc())? >= 15,
            formatting: Formatting::read(&docx),
        }))
    }

    /// The edits on `work`, recorded when tracked: what they wrote, and per edit whether
    /// it fit a table already fitting the window.
    fn applied(
        &self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        inputs: &Inputs<'_>,
    ) -> Result<(Written, Vec<bool>)> {
        Ok(match sink.policy().clone() {
            RevisionPolicy::Tracked(tracking) => {
                let ((warnings, unchanged), numbered, dated) =
                    sink.stamped(tracking, |stamp| Ok(self.apply(work, Some(stamp), inputs)))?;
                let written = Written {
                    numbered: Some(numbered),
                    dated,
                    warnings,
                };
                (written, unchanged)
            }
            RevisionPolicy::Untracked => {
                let (warnings, unchanged) = self.apply(work, None, inputs);
                let written = Written {
                    numbered: None,
                    dated: false,
                    warnings,
                };
                (written, unchanged)
            }
        })
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
