//! Owns: [`Watch`], an edit's watch on the file's text it is about to remove (docx-agent's
//! `Watch`): the units -- cells and paragraphs holding original text -- named before the
//! edit ([`Watch::row`], [`Watch::column`]; `word_delete` adds its blocks the same way
//! through [`Watch::unit`]), what of them left the final
//! view after it ([`Watch::report`]: `data.original_removed`), and the summary's words for
//! it ([`add`]).
//!
//! Must not: change the document. Units are found again after the edit by their first
//! paragraph's id, followed through the renames the edit made.

use crate::describe::short;
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Outcome, ToolResult};
use ooxml_word_edit::originals::{shown, Originals};
use ooxml_word_edit::WordDoc;
use ooxml_word_model::address::resolve::resolve;
use ooxml_word_model::docx::changes::changes_holding;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::{at_path, find_path, Docx, StoryPart};
use ooxml_word_model::ids::{Entry, TableId, W};
use ooxml_xml::{Element, Node};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Places a result's sample names; the rest counted.
const SAMPLE: usize = 5;
/// How many listed changes a reject call names at most.
const CHANGES: usize = 10;
/// Records that take content out of view (a move's source is shown where it went).
const DELETED: [&str; 2] = ["del", "cellDel"];

/// A cell or paragraph holding original text, as it was before the edit.
struct Unit {
    cell: bool,
    place: String,
    part: String,
    /// Its first paragraph's id and how far below the unit it stands.
    anchor: (String, usize),
    text: String,
}

/// The original text an edit is about to remove, and what of it left the final view.
pub(crate) struct Watch<'a> {
    originals: Originals<'a>,
    units: Vec<Unit>,
    seen: HashSet<(String, Vec<u32>)>,
}

impl<'a> Watch<'a> {
    /// A watch over `now`, the version the edit starts from, of a document as `opened`.
    pub(crate) fn new(opened: Arc<Docx>, now: &'a WordDoc) -> Watch<'a> {
        Watch {
            originals: Originals::new(opened, now),
            units: Vec::new(),
            seen: HashSet::new(),
        }
    }

    /// The cell or paragraph at `path` of `story`, named `place`, watched when it holds
    /// original text.
    pub(crate) fn unit(&mut self, story: &StoryPart, place: String, path: &[u32]) {
        if self.seen.contains(&(story.part.clone(), path.to_vec())) {
            return;
        }
        let Some(element) = at_path(story.root(), path) else {
            return;
        };
        let cell = !element.name().is(Some(W), "p");
        let inside = story.ids.paragraphs_within(path);
        let mine: Vec<_> = if cell {
            inside
        } else {
            inside.into_iter().filter(|p| *p.path == *path).collect()
        };
        let Some(first) = mine.first() else { return };
        if !mine.iter().any(|p| self.originals.paragraph(&story.part, &p.id)) {
            return;
        }
        let text = self.originals.text(&story.part, element);
        if text.is_empty() {
            return;
        }
        self.seen.insert((story.part.clone(), path.to_vec()));
        self.units.push(Unit {
            cell,
            place,
            part: story.part.clone(),
            anchor: (first.id.clone(), first.path.len().saturating_sub(path.len())),
            text,
        });
    }

    /// A block deleted, named `id` on `docx` (the version before the edit; an alias
    /// followed as `followed`): a paragraph, a table's cells, or a block-level control's
    /// paragraphs (docx-agent's `Watch.block`).
    pub(crate) fn block(&mut self, docx: &Docx, id: &str, followed: &str) {
        // a paragraph's or table's id first: the controls are read (the whole document) only
        // for an id that names neither (a control's id is of another form)
        let Ok((story, entry)) = resolve(docx, followed) else {
            if let Some(control) = controls(docx).into_iter().find(|c| c.id == id) {
                let Some(story) = docx.part(&control.part) else { return };
                for paragraph in story.ids.paragraphs_within(&control.path) {
                    self.unit(story, paragraph.id.clone(), &paragraph.path);
                }
            }
            return;
        };
        match entry {
            Entry::Paragraph(n) => {
                if let Some(paragraph) = story.ids.paragraphs().get(n) {
                    self.unit(story, paragraph.id.clone(), &paragraph.path);
                }
            }
            Entry::Table(n) => {
                if let Some(table) = story.ids.tables().get(n) {
                    for index in 0..table.rows.len() {
                        self.row(docx, &story.part, &table, index);
                    }
                }
            }
        }
    }

    /// Row `index` of `table` (on `docx`, the version before the edit): its cells.
    pub(crate) fn row(&mut self, docx: &Docx, part: &str, table: &TableId, index: usize) {
        let Some(story) = docx.part(part) else { return };
        let Some((path, _)) = table.rows.get(index) else { return };
        for (cell, start, _) in placed_cells(story, path) {
            self.unit(story, format!("{}/c{index},{start}", table.id), &cell);
        }
    }

    /// Grid column `grid` of `table`: the cells only in it.
    pub(crate) fn column(&mut self, docx: &Docx, part: &str, table: &TableId, grid: usize) {
        let Some(story) = docx.part(part) else { return };
        for (r, (path, _)) in table.rows.iter().enumerate() {
            for (cell, start, span) in placed_cells(story, path) {
                if start == grid && span == 1 {
                    self.unit(story, format!("{}/c{r},{start}", table.id), &cell);
                }
            }
        }
    }

    /// `data.original_removed` once the edit made `after`: `None` when no original text
    /// left the final view. `doc` names the document in the calls it offers.
    pub(crate) fn report(&mut self, after: &WordDoc, doc: &str) -> Option<Value> {
        let docx = after.docx().ok()?;
        let mut lost: Vec<(&Unit, Option<Vec<u32>>)> = Vec::new();
        for unit in &self.units {
            let now = found_again(after, &docx, unit);
            let gone = now
                .as_ref()
                .and_then(|path| docx.part(&unit.part).and_then(|s| at_path(s.root(), path)))
                .is_none_or(|e| shown(e).is_empty());
            if gone {
                lost.push((unit, now));
            }
        }
        if lost.is_empty() {
            return None;
        }
        let cells = lost.iter().filter(|(u, _)| u.cell).count();
        let mut out = Map::new();
        out.insert("cells".into(), json!(cells));
        out.insert("paragraphs".into(), json!(lost.len() - cells));
        let sample: Vec<Value> = lost
            .iter()
            .take(SAMPLE)
            .map(|(u, _)| json!({"at": u.place, "text": short(&u.text, 60)}))
            .collect();
        out.insert("sample".into(), Value::Array(sample));
        if lost.len() > SAMPLE {
            out.insert("sample_left_out".into(), json!(lost.len() - SAMPLE));
        }
        out.insert("undo".into(), json!({"tool": "undo", "arguments": {"doc": doc}}));
        let held: Vec<(String, Vec<u32>)> = lost
            .iter()
            .filter_map(|(u, now)| now.clone().map(|p| (u.part.clone(), p)))
            .collect();
        let ids = changes_of(&docx, &mut self.originals, &held);
        if !ids.is_empty() {
            let ids: Vec<&String> = ids.iter().take(CHANGES).collect();
            out.insert(
                "reject".into(),
                json!({"tool": "word_changes", "arguments": {"doc": doc, "action": "reject", "ids": ids}}),
            );
        }
        Some(Value::Object(out))
    }
}

/// Where `unit` stands after the edit, found by its first paragraph's id.
fn found_again(after: &WordDoc, docx: &Docx, unit: &Unit) -> Option<Vec<u32>> {
    let story = docx.part(&unit.part)?;
    let id = after.ids().follow(&unit.anchor.0);
    let Some(Entry::Paragraph(k)) = story.ids.get(&id) else {
        return None;
    };
    let path = story.ids.paragraphs().get(k)?.path.to_vec();
    path.get(..path.len().checked_sub(unit.anchor.1)?).map(<[u32]>::to_vec)
}

/// A row's cells (also inside row-level `w:sdt` and `w:customXml`), each with its place,
/// first grid column (after `w:gridBefore`) and span (docx-agent's `_cells`).
fn placed_cells(story: &StoryPart, row: &[u32]) -> Vec<(Vec<u32>, usize, usize)> {
    fn walk(element: &Element, at: &mut Vec<u32>, column: &mut usize, out: &mut Vec<(Vec<u32>, usize, usize)>) {
        for (k, node) in element.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            let name = child.name();
            at.push(u32::try_from(k).unwrap_or(u32::MAX));
            if name.is(Some(W), "tc") {
                let span = number(child, &["tcPr", "gridSpan"]).unwrap_or(1).max(1);
                let span = usize::try_from(span).unwrap_or(1);
                out.push((at.clone(), *column, span));
                *column += span;
            } else if name.is(Some(W), "sdt") || name.is(Some(W), "sdtContent") || name.is(Some(W), "customXml") {
                walk(child, at, column, out);
            }
            at.pop();
        }
    }
    let Some(element) = at_path(story.root(), row) else {
        return Vec::new();
    };
    let mut column = usize::try_from(number(element, &["trPr", "gridBefore"]).unwrap_or(0)).unwrap_or(0);
    let mut out = Vec::new();
    walk(element, &mut row.to_vec(), &mut column, &mut out);
    out
}

fn number(element: &Element, path: &[&str]) -> Option<i64> {
    find_path(element, path)?.attribute(Some(W), "val")?.trim().parse().ok()
}

/// The listed changes (`word_changes` list's ids), in document order, holding the deletion
/// records this session wrote that take the units at `held` out of view: in them, and
/// their rows' (a row's deletion is recorded in its `w:trPr`).
fn changes_of(docx: &Docx, originals: &mut Originals<'_>, held: &[(String, Vec<u32>)]) -> Vec<String> {
    let mut wanted: HashSet<(String, Vec<u32>)> = HashSet::new();
    for (part, path) in held {
        let Some(story) = docx.part(part) else { continue };
        let mut holders = vec![path.clone()];
        let row = (1..path.len())
            .rev()
            .map(|n| path.get(..n).unwrap_or_default())
            .find(|p| at_path(story.root(), p).is_some_and(|e| e.name().is(Some(W), "tr")));
        if let Some(row) = row {
            let deleted = at_path(story.root(), row).and_then(|r| find_path(r, &["trPr", "del"]));
            if deleted.is_some() {
                if let Some(k) = at_path(story.root(), row).and_then(|r| trpr_index(r)) {
                    holders.push(row.iter().copied().chain([k]).collect());
                }
            }
        }
        for holder in holders {
            let Some(element) = at_path(story.root(), &holder) else {
                continue;
            };
            let mut found = Vec::new();
            deleted_records(element, &mut holder.clone(), &mut found);
            for (path, record) in found {
                if originals.session_record(part, &record) {
                    wanted.insert((part.clone(), path));
                }
            }
        }
    }
    let wanted: Vec<(String, Vec<u32>)> = wanted.into_iter().collect();
    // a listed change's id is its first record's
    changes_holding(docx, &wanted)
}

fn trpr_index(row: &Element) -> Option<u32> {
    row.children()
        .iter()
        .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "trPr")))
        .and_then(|k| u32::try_from(k).ok())
}

/// The deletion records at or under `element` (at `at`), with their places.
fn deleted_records(element: &Arc<Element>, at: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, Arc<Element>)>) {
    let name = element.name();
    if name.namespace() == Some(W) && DELETED.contains(&name.local()) {
        out.push((at.clone(), Arc::clone(element)));
    }
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        at.push(u32::try_from(k).unwrap_or(u32::MAX));
        deleted_records(child, at, out);
        at.pop();
    }
}

/// The summary's words for a [`Watch::report`]: `""` when none (`said`).
fn said(found: &Value) -> String {
    let count = |k: &str| found.get(k).and_then(Value::as_u64).unwrap_or(0);
    let plural = |n: u64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    if count("cells") > 0 {
        parts.push(plural(count("cells"), "cell"));
    }
    if count("paragraphs") > 0 {
        parts.push(plural(count("paragraphs"), "paragraph"));
    }
    let sample = found
        .get("sample")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let shown: Vec<String> = sample
        .iter()
        .take(3)
        .map(|s| {
            let text = s.get("text").and_then(Value::as_str).unwrap_or_default();
            format!(
                "{} {}",
                s.get("at").and_then(Value::as_str).unwrap_or_default(),
                repr_str(&short(text, 30))
            )
        })
        .collect();
    let more = i64::try_from(sample.len()).unwrap_or(0) - 3 + i64::try_from(count("sample_left_out")).unwrap_or(0);
    let more = if more > 0 {
        format!(", {more} more")
    } else {
        String::new()
    };
    let mut text = format!(
        "; {} holding text the file had when opened left the final view ({}{more}): undo takes the call back",
        parts.join(" and "),
        shown.join(", ")
    );
    if let Some(ids) = found.pointer("/reject/arguments/ids").and_then(Value::as_array) {
        let listed: Vec<String> = ids.iter().map(|i| repr_str(i.as_str().unwrap_or_default())).collect();
        text.push_str(&format!(
            ", or word_changes reject ids [{}] keeps them",
            listed.join(", ")
        ));
    }
    text
}

/// `found` ([`Watch::report`]) into `result`'s summary and `data.original_removed`.
pub(crate) fn add(result: &mut ToolResult<Value>, found: Option<Value>) {
    let Some(found) = found else { return };
    let Outcome::Done(done) = &mut result.outcome else {
        return;
    };
    done.summary.push_str(&said(&found));
    let data = done.data.get_or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(data) = data {
        data.insert("original_removed".into(), found);
    }
}

#[cfg(test)]
mod tests {
    use super::said;
    use serde_json::json;

    #[test]
    fn said_names_counts_sample_and_reject() {
        let found = json!({"cells": 2, "paragraphs": 0,
            "sample": [{"at": "t:1/c0,0", "text": "a"}, {"at": "t:1/c0,1", "text": "b"}],
            "reject": {"arguments": {"ids": ["ch:1"]}}});
        assert_eq!(
            said(&found),
            "; 2 cells holding text the file had when opened left the final view (t:1/c0,0 'a', t:1/c0,1 'b'): \
             undo takes the call back, or word_changes reject ids ['ch:1'] keeps them"
        );
    }
}
