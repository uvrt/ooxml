//! Owns: the values a scale's targets hold in a Word version (docx-agent's
//! `scales.scale_values`), each with the step its text is styled as now: a paragraph
//! (`p:A`, its text whole), a range in one paragraph (`p:A@4:5`), a cell (`t:X/c1,2`, its
//! text, its fill), a table's column (`t:X/col2`: its body cells, the header rows and the rows
//! a pending tracked deletion removes left out) and the values written by reference
//! (`{{d:scores/*/score}}`: each still where it was written, or its cell where it is the
//! cell's whole text); a label kept where a value was is read as the value's text, its record
//! beside it. With `range`, only the values in it; with `pages`, only those on them, each
//! with its first page.
//!
//! **The step a value has now**: the scale's step whose style its text's runs carry
//! (`w:rStyle`, by the style's name), the most carried where they differ; exact where every
//! run carries it, the style holds the step's colours, no run's direct colour, shading or
//! highlight hides them and a cell's fill is the step's; a cell shaded as a step whose text
//! carries none is that step, not exactly.
//!
//! A target that holds nothing to style is a value skipped, with why.
//!
//! Must not: write a step (the host's writer does), or choose the step a value is due.

use super::scale::{py_strip, ReadScale};
use super::targets::{column_target, reference_matches, reference_target};
use super::{cell_of, lay_out, not_laid, pages_failed, pages_refused, without_pages};
use crate::objects::{read_look, table_view};
use crate::pages::WordLayout;
use crate::written::py_space;
use ooxml_base::Cancel;
use ooxml_tools::ToolError;
use ooxml_word_edit::WordDoc;
use ooxml_word_model::address::resolve::InTable;
use ooxml_word_model::address::syntax::parse_range;
use ooxml_word_model::address::{in_table, paragraph, resolve, text_range, AddressError};
use ooxml_word_model::docx::provenance::text_records;
use ooxml_word_model::docx::tables::{cells_at, no_cell_text, own_rows, GridPlaces};
use ooxml_word_model::docx::text::{char_runs, View, OBJECT};
use ooxml_word_model::docx::{at_path, find_path, style_id, style_kind, style_name, Docx, StoryPart};
use ooxml_word_model::ids::{Entry, W};
use ooxml_xml::Element;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Where a value is, for the host's writer: a paragraph's characters, or a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueHandle {
    /// Characters `start..end` of the paragraph `paragraph`.
    Range {
        /// The paragraph's id.
        paragraph: String,
        /// The first character.
        start: usize,
        /// The character after the last.
        end: usize,
    },
    /// The cell `cell` (`t:X/c1,2`).
    Cell(String),
}

/// One value a scale's target holds (`ooxml_edit.tools.ScaleValue`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScaleValue {
    /// Where the value is (a paragraph, a range, a cell).
    pub target: String,
    /// The value as written.
    pub text: String,
    /// The target as the scale names it, where that named many values.
    pub given: Option<String>,
    /// The index of the step it is styled as now.
    pub now: Option<usize>,
    /// Whether it is styled exactly as that step says.
    pub exact: bool,
    /// What differs, where it is not.
    pub differs: Option<String>,
    /// Why it cannot be styled (the rest then unread).
    pub skip: Option<String>,
    /// The page it starts on, where `pages` were asked for.
    pub page: Option<usize>,
    /// Where it is, and the ids of the paragraphs it is in.
    pub handle: Option<(ValueHandle, Vec<String>)>,
    /// The label record kept where it is.
    pub kept: Option<Value>,
    /// The reference it was written from.
    pub source: Option<String>,
}

impl ScaleValue {
    fn skipped(target: &str, given: Option<&str>, why: String) -> ScaleValue {
        ScaleValue {
            target: target.to_owned(),
            given: given.map(str::to_owned),
            exact: true,
            skip: Some(why),
            ..ScaleValue::default()
        }
    }
}

/// What [`scale_values`] read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScaleValues {
    /// The values, target by target.
    pub values: Vec<ScaleValue>,
}

/// Why a target gives no value: a value skipped with why, or the whole read refused.
enum Missed {
    Skip(String),
    Refused(ToolError),
}

impl From<String> for Missed {
    fn from(why: String) -> Missed {
        Missed::Skip(why)
    }
}

/// A cell that is not there, refused as docx-agent's `IndexError` is: its message, the
/// table's cells as the options.
fn missing_cell(error: AddressError) -> ToolError {
    let way_out = ooxml_tools::WayOut::names(error.options)
        .unwrap_or_else(|| ooxml_tools::WayOut::call(crate::word_read::try_call(serde_json::json!({}))));
    ToolError::refusal(ooxml_tools::Refusal::NotFound, error.message, way_out)
}

/// What a text range is given as, in the words an address holding no text is told.
const NOT_TEXT: &str = "give a range (p:A@4:11), a position (p:A@5), a paragraph, a cell or a bookmark";

/// The table part of a cell address as written (`t:X` of `t:X/c1,2`).
fn table_of(address: &str) -> &str {
    address.rsplit_once("/c").map_or(address, |(table, _)| table)
}

/// What a target is, in the words a target that names nothing is told (values by reference
/// as a target is written, `{{d:...}}`).
const TARGETS: &str =
    "a target is a paragraph, a range p:A@4:5, a cell t:X/c1,2, a column t:X/col2 or {{d:...}} values";

/// What a target is, in the words a target of another kind is told (values by reference as
/// a target is written, as [`TARGETS`] says them).
const KINDS: &str = "a target is a paragraph, a range, a cell, a table's column or {{d:...}} values";

/// The values `scale`'s targets hold in the version `doc` (read as `docx`, named `name` in
/// the host's words), within `range` (`check`'s) and on `pages`, placed by `layout`.
///
/// # Errors
/// A range refused as `check` refuses it; `pages` asked for where the layout failed;
/// `internal` when a part is not XML.
#[allow(clippy::too_many_arguments)]
pub fn scale_values(
    doc: &WordDoc,
    docx: &Docx,
    name: &str,
    scale: &ReadScale,
    range: Option<&[String]>,
    pages: Option<&[usize]>,
    layout: Option<&dyn WordLayout>,
    cancel: &dyn Cancel,
) -> Result<ScaleValues, ToolError> {
    let records = text_records(docx, doc.ids().aliases()).map_err(|e| ToolError::internal(e.to_string(), "scale"))?;
    let reader = Reader {
        doc,
        docx,
        scale,
        styles: docx
            .styles()
            .iter()
            .filter(|s| style_kind(s) == "character")
            .map(|s| (style_id(s).to_owned(), style_name(s).unwrap_or(style_id(s)).to_owned()))
            .collect(),
        labels: records.labels.iter().map(|(p, l)| (p.clone(), l.clone())).collect(),
        facts: records.facts,
        label_order: records.labels,
    };
    let mut found = Vec::new();
    for target in &scale.targets {
        match reader.target(target) {
            Ok(values) => found.extend(values),
            Err(Missed::Skip(why)) => found.push(ScaleValue::skipped(target, None, why)),
            Err(Missed::Refused(error)) => return Err(error),
        }
    }
    let ids = doc.ids();
    let scope = crate::check::scope_of(range, &|a| ids.follow_address(a), docx, name)?;
    if pages.is_none() && scope.is_none() {
        return Ok(ScaleValues { values: found });
    }
    let laid = match pages {
        None => None,
        Some(_) => match lay_out(layout, doc.package(), cancel) {
            Some(Ok(laid)) => Some(laid),
            Some(Err(why)) if why.unlaid().is_some() => {
                return Err(pages_refused("scale", why, without_pages(name, range, &[], &["scale"])))
            }
            Some(Err(why)) => {
                let (code, message) = not_laid(why);
                return Err(pages_failed("numbers", code, &message));
            }
            None => {
                return Err(pages_failed(
                    "numbers",
                    "layout_failed",
                    "no layout of this document here",
                ))
            }
        },
    };
    let mut values = Vec::new();
    for mut value in found {
        if value.skip.is_none() {
            let ids: &[String] = value.handle.as_ref().map_or(&[], |(_, ids)| ids);
            if scope
                .as_ref()
                .is_some_and(|s| !ids.iter().any(|i| s.paragraphs.contains(i)))
            {
                continue;
            }
            if let (Some(laid), Some(wanted)) = (&laid, pages) {
                let on = laid.of_paragraphs(ids.iter().map(String::as_str));
                if !on.iter().any(|p| wanted.contains(p)) {
                    continue;
                }
                value.page = on.first().copied();
            }
        }
        values.push(value);
    }
    Ok(ScaleValues { values })
}

/// What reading every target needs once.
struct Reader<'r> {
    doc: &'r WordDoc,
    docx: &'r Docx,
    scale: &'r ReadScale,
    /// The character styles' names, by id.
    styles: HashMap<String, String>,
    /// The labels kept, by paragraph id.
    labels: HashMap<String, Vec<Value>>,
    /// The text facts, by paragraph id, in the record's order.
    facts: Vec<(String, Vec<Value>)>,
    /// The labels, by paragraph id, in the record's order.
    label_order: Vec<(String, Vec<Value>)>,
}

/// A paragraph found: its part, its element, its place and its id.
struct Para<'d> {
    part: &'d StoryPart,
    element: Arc<Element>,
    path: Box<[u32]>,
    id: String,
}

/// A cell found: its id and its place in its part.
struct Cell<'d> {
    part: &'d StoryPart,
    id: String,
    path: Box<[u32]>,
}

fn said(message: &str) -> String {
    message.to_owned()
}

impl<'r> Reader<'r> {
    fn follow(&self, address: &str) -> String {
        self.doc.ids().follow_address(address)
    }

    /// The label kept in `paragraph` starting at `start` (and, with `end`, ending there).
    fn label_at(&self, paragraph: &str, start: usize, end: Option<usize>) -> Option<Value> {
        self.labels.get(paragraph)?.iter().find_map(|label| {
            let at = usize::try_from(label.get("at")?.as_u64()?).ok()?;
            let length = label
                .get("text")
                .and_then(Value::as_str)
                .map_or(0, |t| t.chars().count());
            (at == start && end.is_none_or(|e| at + length == e)).then(|| label.clone())
        })
    }

    fn paragraph(&self, id: &str) -> Result<Para<'r>, String> {
        let (part, k) = paragraph(self.docx, &self.follow(id)).map_err(|e| e.message)?;
        let found = part
            .ids
            .paragraphs()
            .get(k)
            .ok_or_else(|| format!("no paragraph {id}"))?;
        Ok(Para {
            part,
            element: found.element,
            path: found.path,
            id: found.id,
        })
    }

    fn target(&self, target: &str) -> Result<Vec<ScaleValue>, Missed> {
        if let Some((table, column)) = column_target(target) {
            return Ok(self.column(target, table, column)?);
        }
        if let Some(wanted) = reference_target(target) {
            return Ok(self.referenced(target, wanted)?);
        }
        if target.contains('@') && !target.starts_with("p@") {
            return self.range_target(target);
        }
        if let Some(kind) = other_kind(target) {
            return Err(format!("{target} is a {kind}: {KINDS}").into());
        }
        let followed = self.follow(target);
        match in_table(self.docx, &followed) {
            Ok(Some(found)) => {
                let Some(column) = found.column else {
                    return Err(format!("{target} is a row: {KINDS}").into());
                };
                let cell = self
                    .cell(table_of(target), &found, column)
                    .map_err(|e| format!("no {target}: {}; {TARGETS}", e.message))?;
                return Ok(vec![self.cell_value(&cell, None, None)]);
            }
            Ok(None) => {}
            Err(error) => return Err(format!("no {target}: {}; {TARGETS}", said(&error.message)).into()),
        }
        match resolve(self.docx, &followed) {
            Ok((_, Entry::Paragraph(_))) => {
                let found = self.paragraph(target)?;
                Ok(vec![self.text_value(target, None, &found, 0, None, None, None)])
            }
            Ok((_, Entry::Table(_))) => {
                Err(format!("{target} is a table: name its column ({target}/col1) or its cells").into())
            }
            Err(error) => Err(format!("no {target}: {}; {TARGETS}", said(&error.message)).into()),
        }
    }

    /// A target with `@` (`p:A@4:5`, or what `Document.range` reads as the text an address
    /// holds: a paragraph's, a cell's): its characters, in one paragraph.
    fn range_target(&self, target: &str) -> Result<Vec<ScaleValue>, Missed> {
        let no_range = |why: &str| Missed::Skip(format!("no range {target}: {why}"));
        let followed = self.follow(target);
        let (start_para, end_id, start, end) = match parse_range(&followed) {
            Some(found) => {
                text_range(self.docx, &followed).map_err(|e| no_range(&e.message))?;
                let para = self.paragraph(&found.start_id).map_err(|e| no_range(&e))?;
                (para, Some(found.end_id), found.start, found.end)
            }
            None => self.text_of(target, &followed)?,
        };
        let end_para = end_id.map(|id| self.paragraph(&id));
        if end_para.is_some_and(|e| e.map_or(true, |e| e.id != start_para.id)) {
            return Err(format!("{target} runs across paragraphs: a value is in one paragraph (p:A@4:5)").into());
        }
        if end <= start {
            return Err(format!("{target} is a position, not characters: give p:A@<start>:<end>").into());
        }
        let label = self.label_at(&start_para.id, start, None);
        let stop = match &label {
            None => end,
            Some(label) => {
                start
                    + label
                        .get("text")
                        .and_then(Value::as_str)
                        .map_or(0, |t| t.chars().count())
            }
        };
        Ok(vec![self.text_value(
            target,
            None,
            &start_para,
            start,
            Some(stop),
            label,
            None,
        )])
    }

    /// The text an address that is no range holds, as a range (`_text_of`): a paragraph's
    /// whole text, a cell's from its first paragraph to its last; the last paragraph's id
    /// where it is another than the first.
    #[allow(clippy::type_complexity)]
    fn text_of(&self, target: &str, followed: &str) -> Result<(Para<'r>, Option<String>, usize, usize), Missed> {
        let no_range = |why: &str| Missed::Skip(format!("no range {target}: {why}"));
        let length = |p: &Para<'_>| char_runs(&p.element, View::Current).len();
        match in_table(self.docx, followed) {
            Ok(Some(found)) => {
                let Some(column) = found.column else {
                    return Err(no_range(&format!("{target} is a row, not text: {NOT_TEXT}")));
                };
                // a cell that is not there: docx-agent's `IndexError`, refused (not a value skipped)
                let cell = self
                    .cell(table_of(target), &found, column)
                    .map_err(|e| Missed::Refused(missing_cell(e)))?;
                let paragraphs = cell.part.ids.paragraphs_within(&cell.path);
                let (Some(first), Some(last)) = (paragraphs.first(), paragraphs.last()) else {
                    return Err(no_range(&format!("{target} holds no paragraph")));
                };
                let first = self.paragraph(&first.id).map_err(|e| no_range(&e))?;
                let last = self.paragraph(&last.id).map_err(|e| no_range(&e))?;
                let end = length(&last);
                Ok((first, Some(last.id), 0, end))
            }
            Ok(None) => match resolve(self.docx, followed) {
                Ok((_, Entry::Paragraph(_))) => {
                    let para = self.paragraph(followed).map_err(|e| no_range(&e))?;
                    let end = length(&para);
                    Ok((para, None, 0, end))
                }
                Ok((_, Entry::Table(_))) => Err(no_range(&format!("{target} is a table, not text: {NOT_TEXT}"))),
                Err(error) => Err(no_range(&error.message)),
            },
            Err(error) => Err(no_range(&error.message)),
        }
    }

    /// The cell over grid position `(row, column)` of the table `found` is in (named `table`
    /// as the target writes it): the one whose origin is there (`Table.cell`).
    fn cell(&self, table: &str, found: &InTable<'_>, column: usize) -> Result<Cell<'r>, AddressError> {
        let id = found
            .part
            .ids
            .tables()
            .get(found.table)
            .map(|t| t.id)
            .unwrap_or_default();
        let call = crate::word_read::try_call(serde_json::json!({}));
        let view = table_view(self.doc, self.docx, &id, &call)
            .map_err(|e| AddressError::not_found(e.message().to_owned(), Vec::new()))?;
        let missing = || {
            let ids: Vec<String> = (0..view.grid.len())
                .flat_map(|r| view.row_cells(r).into_iter().map(move |(c, _)| (r, c)))
                .filter(|&(r, c)| view.origin(r, c) == Some((r, c)))
                .map(|(r, c)| format!("{table}/c{r},{c}"))
                .take(50)
                .collect();
            AddressError::not_found(no_cell_text(&GridPlaces::from_positions(&view.grid), table, found.row, column), ids)
        };
        let (r, c) = view.origin(found.row, column).ok_or_else(missing)?;
        self.cell_at(&view.part, &view.table.id, &view.table.rows, r, c)
            .ok_or_else(missing)
    }

    fn cell_at(
        &self,
        part: &str,
        table: &str,
        rows: &[(Box<[u32]>, String)],
        row: usize,
        column: usize,
    ) -> Option<Cell<'r>> {
        let part = self.docx.part(part)?;
        let row_path = &rows.get(row)?.0;
        let element = at_path(part.root(), row_path)?;
        let cell = cells_at(element, row_path).into_iter().find(|c| c.column == column)?;
        Some(Cell {
            part,
            id: format!("{table}/c{row},{column}"),
            path: cell.path,
        })
    }

    /// A paragraph's characters `start..end` (to its end for `None`) as a value of `target`.
    #[allow(clippy::too_many_arguments)]
    fn text_value(
        &self,
        target: &str,
        given: Option<&str>,
        para: &Para<'_>,
        start: usize,
        end: Option<usize>,
        label: Option<Value>,
        source: Option<&str>,
    ) -> ScaleValue {
        let atoms = char_runs(&para.element, View::Current);
        let (text, runs) = runs_over(&atoms, start, end);
        if py_strip(&text).is_empty() {
            return ScaleValue::skipped(target, given, format!("{target} holds no text"));
        }
        let (now, exact, differs) = self.now(&para.element, &runs, None, false);
        let stop = end.unwrap_or(atoms.len());
        let whole = start == 0 && end.is_none();
        let kept = match label {
            None if whole => self.label_at(&para.id, 0, Some(stop)),
            other => other,
        };
        let address = if whole {
            para.id.clone()
        } else {
            format!("{}@{start}:{stop}", para.id)
        };
        ScaleValue {
            target: address,
            text: py_strip(&text).to_owned(),
            given: given.map(str::to_owned),
            now,
            exact,
            differs,
            handle: Some((
                ValueHandle::Range {
                    paragraph: para.id.clone(),
                    start,
                    end: stop,
                },
                vec![para.id.clone()],
            )),
            kept,
            source: source.map(str::to_owned),
            ..ScaleValue::default()
        }
    }

    /// A cell's text as a value (`_cell_value`).
    fn cell_value(&self, cell: &Cell<'_>, given: Option<&str>, source: Option<&str>) -> ScaleValue {
        let paragraphs = cell.part.ids.paragraphs_within(&cell.path);
        // each paragraph's id, text and length
        let mut read: Vec<(String, String, usize)> = Vec::new();
        let mut runs: Vec<(Arc<Element>, Box<[u32]>)> = Vec::new();
        for p in &paragraphs {
            let atoms = char_runs(&p.element, View::Current);
            let (text, own) = runs_over(&atoms, 0, None);
            read.push((p.id.clone(), text, atoms.len()));
            runs.extend(own.into_iter().map(|r| (Arc::clone(&p.element), r)));
        }
        let text = py_strip(&read.iter().map(|(_, t, _)| t.as_str()).collect::<Vec<_>>().join("\n")).to_owned();
        if text.is_empty() {
            return ScaleValue::skipped(&cell.id, given, format!("{} is empty", cell.id));
        }
        let holding: Vec<&(String, String, usize)> = read.iter().filter(|(_, t, _)| !py_strip(t).is_empty()).collect();
        let handle = Some((
            ValueHandle::Cell(cell.id.clone()),
            read.iter().map(|(id, _, _)| id.clone()).collect(),
        ));
        if self.scale.labels && holding.len() > 1 {
            if holding.iter().all(|(_, t, _)| self.scale.step_of_label(t).is_some()) {
                // labels, one per paragraph: no one value is kept for them
                return ScaleValue {
                    target: cell.id.clone(),
                    text,
                    given: given.map(str::to_owned),
                    exact: true,
                    handle,
                    ..ScaleValue::default()
                };
            }
            return ScaleValue::skipped(
                &cell.id,
                given,
                format!(
                    "{} holds {} paragraphs of text: a label takes the place of one value, in one paragraph",
                    cell.id,
                    holding.len()
                ),
            );
        }
        let kept = match holding.as_slice() {
            [(id, _, length)] => self.label_at(id, 0, Some(*length)),
            _ => None,
        };
        let fill = cell_fill(cell.part.root(), &cell.path);
        let (mut now, mut exact, mut differs) = self.now_of(&runs, fill.as_deref(), true);
        if now.is_none() {
            if let Some(fill) = &fill {
                if let Some(k) = self.scale.steps.iter().position(|s| s.fill == *fill) {
                    (now, exact, differs) = (Some(k), false, Some("text not in the step's style".to_owned()));
                }
            }
        }
        ScaleValue {
            target: cell.id.clone(),
            text,
            given: given.map(str::to_owned),
            now,
            exact,
            differs,
            handle,
            kept,
            source: source.map(str::to_owned),
            ..ScaleValue::default()
        }
    }

    fn now(
        &self,
        paragraph: &Arc<Element>,
        runs: &[Box<[u32]>],
        fill: Option<&str>,
        cell: bool,
    ) -> (Option<usize>, bool, Option<String>) {
        let runs: Vec<(Arc<Element>, Box<[u32]>)> = runs.iter().map(|r| (Arc::clone(paragraph), r.clone())).collect();
        self.now_of(&runs, fill, cell)
    }

    /// The step the runs (each a place below its paragraph) carry, whether exactly, and what
    /// differs (`_now`).
    fn now_of(
        &self,
        runs: &[(Arc<Element>, Box<[u32]>)],
        fill: Option<&str>,
        cell: bool,
    ) -> (Option<usize>, bool, Option<String>) {
        let mut found: Vec<Option<usize>> = Vec::new();
        let mut direct = false;
        for (paragraph, path) in runs {
            let properties = at_path(paragraph, path).and_then(|run| run.find(Some(W), "rPr"));
            let style = properties
                .and_then(|p| p.find(Some(W), "rStyle"))
                .map(|s| s.attribute(Some(W), "val").unwrap_or(""))
                .and_then(|id| self.styles.get(id));
            found.push(self.scale.step_of_style(style.map(String::as_str)));
            if properties.is_some_and(|p| {
                ["color", "shd", "highlight"]
                    .iter()
                    .any(|t| p.find(Some(W), t).is_some())
            }) {
                direct = true;
            }
        }
        let steps: Vec<usize> = found.iter().flatten().copied().collect();
        let Some(now) = mode(&steps) else {
            return (None, true, None);
        };
        let Some(step) = self.scale.steps.get(now) else {
            return (None, true, None);
        };
        if found.iter().collect::<HashSet<_>>().len() > 1 {
            return (Some(now), false, Some("styled in part".to_owned()));
        }
        if let Some(style) = self.docx.find_style(&self.scale.style_name(now), "character") {
            let (fill, text) = colours_of(style);
            if fill.as_deref() != Some(step.fill.as_str()) || text.as_deref() != Some(step.text.as_str()) {
                return (
                    Some(now),
                    false,
                    Some(format!(
                        "style {} fill {}, text {}",
                        style_name(style).unwrap_or(style_id(style)),
                        fill.as_deref().unwrap_or("none"),
                        text.as_deref().unwrap_or("auto")
                    )),
                );
            }
        }
        if direct {
            return (Some(now), false, Some("direct colour on its text".to_owned()));
        }
        if cell && fill.unwrap_or("").to_uppercase() != step.fill {
            return (Some(now), false, Some(format!("cell fill {}", fill.unwrap_or("none"))));
        }
        (Some(now), true, None)
    }

    /// A table's column's body cells as values (`_column`).
    fn column(&self, target: &str, table: &str, column: usize) -> Result<Vec<ScaleValue>, String> {
        let call = crate::word_read::try_call(serde_json::json!({}));
        let view =
            table_view(self.doc, self.docx, table, &call).map_err(|e| format!("no table {table}: {}", e.message()))?;
        let width = view.column_count();
        if column >= width {
            return Err(format!(
                "{table} has grid columns 0 to {}: no column {column}",
                i64::try_from(width).unwrap_or(0) - 1
            ));
        }
        let element = &view.table.element;
        let marked = own_rows(element)
            .iter()
            .take_while(|(row, _)| find_path(row, &["trPr", "tblHeader"]).is_some())
            .count();
        let look = read_look(find_path(element, &["tblPr", "tblLook"]));
        let header = if marked == 0 && look[0] { 1 } else { marked };
        let rows = view.accepted(self.docx).table.rows;
        let body: Vec<usize> = if marked > 0 {
            rows.iter().copied().filter(|&r| r >= header).collect()
        } else {
            rows.iter().copied().skip(header).collect()
        };
        let mut out = Vec::new();
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        for r in body {
            let Some(origin) = view.origin(r, column) else { continue };
            if !seen.insert(origin) || origin.0 != r {
                continue;
            }
            if let Some(cell) = self.cell_at(&view.part, &view.table.id, &view.table.rows, origin.0, origin.1) {
                out.push(self.cell_value(&cell, Some(target), None));
            }
        }
        if out.is_empty() {
            let pending = view.table.rows.len().saturating_sub(rows.len());
            let more = if pending > 0 {
                format!("; {pending} more deleted by a pending tracked change")
            } else {
                String::new()
            };
            return Err(format!(
                "{target}: no body cells ({header} header row(s) of {}{more})",
                rows.len()
            ));
        }
        Ok(out)
    }

    /// The values written by a reference `wanted` names (`_referenced`).
    fn referenced(&self, target: &str, wanted: &str) -> Result<Vec<ScaleValue>, String> {
        let body = self.docx.body();
        let order: HashMap<String, usize> = body
            .map(|b| b.ids.paragraphs().iter().enumerate().map(|(k, p)| (p.id, k)).collect())
            .unwrap_or_default();
        let facts = self
            .facts
            .iter()
            .flat_map(|(p, list)| list.iter().map(move |f| (p, f, false)));
        let labels = self
            .label_order
            .iter()
            .flat_map(|(p, list)| list.iter().map(move |l| (p, l, true)));
        let mut out: Vec<(usize, usize, ScaleValue)> = Vec::new();
        for (paragraph_id, fact, is_label) in facts.chain(labels) {
            let Some(source) = fact.get("source").and_then(Value::as_str) else {
                continue;
            };
            if !reference_matches(wanted, source) {
                continue;
            }
            let Ok(para) = self.paragraph(paragraph_id) else {
                continue;
            };
            let at = fact
                .get("at")
                .and_then(Value::as_u64)
                .and_then(|a| usize::try_from(a).ok())
                .unwrap_or(0);
            let written = fact.get("text").and_then(Value::as_str).unwrap_or("");
            let label = is_label.then(|| fact.clone());
            let source = (!is_label).then_some(source);
            let mut value = None;
            if body.is_some_and(|b| std::ptr::eq(b, para.part)) {
                if let Some(cell) = cell_of(para.part, &para.path).and_then(|c| c.id) {
                    if let Some(found) = self.cell_by_id(&cell) {
                        if py_strip(&cell_text(&found)) == py_strip(written) {
                            value = Some(self.cell_value(&found, Some(target), source));
                        }
                    }
                }
            }
            let end = at + written.chars().count();
            let value = value.unwrap_or_else(|| {
                let address = format!("{}@{at}:{end}", para.id);
                self.text_value(&address, Some(target), &para, at, Some(end), label, source)
            });
            out.push((order.get(&para.id).copied().unwrap_or(order.len()), at, value));
        }
        if out.is_empty() {
            return Err(format!(
                "{target}: no value written by a reference like it is still in the document's text"
            ));
        }
        out.sort_by_key(|(order, at, _)| (*order, *at));
        let mut seen = HashSet::new();
        Ok(out
            .into_iter()
            .map(|(_, _, v)| v)
            .filter(|v| seen.insert(v.target.clone()))
            .collect())
    }

    /// The cell an id (`t:X/c1,2`) names, as `Document.get` finds it.
    fn cell_by_id(&self, id: &str) -> Option<Cell<'r>> {
        let found = in_table(self.docx, id).ok()??;
        self.cell(table_of(id), &found, found.column?).ok()
    }
}

/// The characters `start..end` of a paragraph's (Python's slice: clamped), as text with each
/// object read as a space, and the runs holding those that are not white space, in order.
fn runs_over(atoms: &[(char, Option<Box<[u32]>>)], start: usize, end: Option<usize>) -> (String, Vec<Box<[u32]>>) {
    let end = end.unwrap_or(atoms.len()).min(atoms.len());
    let chosen = atoms.get(start.min(end)..end).unwrap_or_default();
    let text = chosen
        .iter()
        .map(|(c, _)| if *c == OBJECT { ' ' } else { *c })
        .collect();
    let mut runs: Vec<Box<[u32]>> = Vec::new();
    for (c, run) in chosen {
        if let Some(run) = run {
            if !py_space(*c) && !runs.contains(run) {
                runs.push(run.clone());
            }
        }
    }
    (text, runs)
}

/// A cell's text: its paragraphs' text, one line each (`Cell.text`).
fn cell_text(cell: &Cell<'_>) -> String {
    cell.part
        .ids
        .paragraphs_within(&cell.path)
        .iter()
        .map(|p| ooxml_word_model::docx::text::paragraph_text(&p.element, View::Current))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The cell at `path`'s fill, upper case (`None`: none, or `auto`).
fn cell_fill(root: &Arc<Element>, path: &[u32]) -> Option<String> {
    let cell = at_path(root, path)?;
    let fill = find_path(cell, &["tcPr", "shd"])?.attribute(Some(W), "fill")?;
    (fill != "auto").then(|| fill.to_uppercase())
}

/// A character style's fill and text colour, upper case (`_colours_of`).
fn colours_of(style: &Element) -> (Option<String>, Option<String>) {
    let Some(properties) = style.find(Some(W), "rPr") else {
        return (None, None);
    };
    let read = |tag: &str, attribute: &str| {
        properties
            .find(Some(W), tag)
            .and_then(|n| n.attribute(Some(W), attribute))
            .filter(|v| !v.is_empty())
            .map(str::to_uppercase)
    };
    (read("shd", "fill"), read("color", "val"))
}

/// The most common of `steps`, the lowest among equals (`max(set(steps), key=steps.count)`).
fn mode(steps: &[usize]) -> Option<usize> {
    let mut counts: Vec<(usize, usize)> = Vec::new();
    for &s in steps {
        match counts.iter_mut().find(|(k, _)| *k == s) {
            Some((_, n)) => *n += 1,
            None => counts.push((s, 1)),
        }
    }
    counts.sort_unstable();
    let best = counts.iter().map(|(_, n)| *n).max()?;
    counts.into_iter().find(|(_, n)| *n == best).map(|(k, _)| k)
}

/// The kind of thing a target of another kind names (`Document.get`'s class), by its form.
fn other_kind(target: &str) -> Option<&'static str> {
    const FORMS: [(&str, &str); 13] = [
        ("hl:", "hyperlink"),
        ("bm:", "bookmark"),
        ("d:", "drawing"),
        ("fld:", "field"),
        ("fn:", "note"),
        ("en:", "note"),
        ("c:", "comment"),
        ("c#", "comment"),
        ("rev:", "revision"),
        ("cc:", "contentcontrol"),
        ("cc@", "contentcontrol"),
        ("s:", "section"),
        ("s@", "section"),
    ];
    if let Some((_, kind)) = FORMS.iter().find(|(prefix, _)| target.starts_with(prefix)) {
        return Some(kind);
    }
    if !target.contains([':', '@', '/']) && !target.trim().is_empty() {
        return Some("story");
    }
    None
}
