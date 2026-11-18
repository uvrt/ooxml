//! Owns: the parts of `word_inspect`'s reply beside the blocks (docx-agent's `state`): the
//! notes the blocks reference (and the notes those reference), the comments they
//! reference with their anchors, every revision in them, the styles they use, and for a
//! cell its own properties.
//!
//! Must not: write blocks (the block writer does), or change the document.

use super::blocks::Writer;
use ooxml_word_model::address::resolve::{in_table, marker_range, range_text};
use ooxml_word_model::docx::comments::comments as comment_records;
use ooxml_word_model::docx::revisions::{records as revision_records, Record as Revision, KINDS};
use ooxml_word_model::docx::text::View;
use ooxml_word_model::docx::{at_path, child_val, find_path, style_id, style_kind, style_name, Docx};
use ooxml_word_model::ids::W;
use ooxml_word_model::markdown::read::{note_elements, Reader, Record};
use ooxml_xml::{Element, Node};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

impl Writer<'_> {
    /// Every note of a notes part, to write.
    pub(super) fn notes_of_part(&mut self, part: &str) {
        self.note_labels.extend(
            note_elements(self.docx)
                .into_iter()
                .filter(|n| n.0 == part)
                .map(|n| n.1),
        );
    }

    /// The notes `records` reference, depth first (tables' cells included, each table's
    /// before its cells' own).
    pub(super) fn referenced_notes(&mut self, records: &[Record]) {
        for record in records {
            self.note_labels.extend(referenced(record));
            if let Record::Table(t) = record {
                for cell in t.rows.iter().flatten() {
                    self.referenced_notes(&cell.records);
                }
            }
        }
    }

    /// The notes to write, each with its blocks.
    pub(super) fn notes(&mut self, reader: &mut Reader<'_>) -> Value {
        let notes: HashMap<String, (String, ooxml_word_model::markdown::read::Placed)> = note_elements(self.docx)
            .into_iter()
            .map(|(part, id, placed)| (id, (part, placed)))
            .collect();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut at = 0;
        while let Some(label) = self.note_labels.get(at).cloned() {
            at += 1;
            if !seen.insert(label.clone()) {
                continue;
            }
            let Some((part, placed)) = notes.get(&label) else {
                continue;
            };
            let records = reader.container(part, placed);
            let blocks = self.blocks(reader, &records, None);
            let kind = if label.starts_with("fn:") {
                "footnote"
            } else {
                "endnote"
            };
            out.push(json!({"id": label, "kind": kind, "blocks": blocks}));
            let mut more = Vec::new();
            for record in &records {
                walk(record, &mut |r| more.extend(referenced(r)));
            }
            self.note_labels.extend(more);
        }
        Value::Array(out)
    }

    /// The comments the blocks reference, with what each is attached to.
    pub(super) fn comments(&self, reader: &Reader<'_>) -> Value {
        let _ = reader;
        let records = comment_records(self.docx);
        let by_id: HashMap<&str, &ooxml_word_model::docx::comments::Comment> =
            records.iter().map(|c| (c.id.as_str(), c)).collect();
        let markers = comment_markers(self.docx, records.first().map(|c| c.part.as_str()));
        let mut out = Vec::new();
        for record in &records {
            let Some(places) = self.comment_places.get(&record.id) else {
                continue;
            };
            let anchor = anchor(self.docx, record, &by_id, &markers);
            out.push(json!({
                "id": record.id, "author": record.author, "initials": record.initials, "date": record.date,
                "parent": record.parent, "done": record.done, "t": record.says, "paragraphs": places,
                "anchor": anchor,
            }));
        }
        Value::Array(out)
    }

    /// Every revision in the blocks written: a paragraph's, or a row's or table's own.
    pub(super) fn revisions(&self) -> Value {
        let mut out = Vec::new();
        for record in revision_records(self.docx) {
            let Some(part) = self.docx.part(&record.part) else {
                continue;
            };
            let paragraph = (1..record.path.len())
                .rev()
                .find_map(|n| record.path.get(..n).and_then(|p| part.ids.paragraph_at(p)));
            let mut entry = Map::new();
            entry.insert("id".into(), Value::from(record.id.as_str()));
            entry.insert("kind".into(), Value::from(kind(part.root(), &record)));
            entry.insert("author".into(), attribute(&record.element, "author"));
            entry.insert("date".into(), attribute(&record.element, "date"));
            match paragraph {
                Some(p) if self.paragraph_ids.contains(&p.id) => {
                    entry.insert("paragraph".into(), Value::from(p.id.as_str()));
                    entry.insert("t".into(), Value::from(text(&record.element)));
                    out.push(Value::Object(entry));
                }
                Some(_) => {}
                None => {
                    let table = self
                        .tables
                        .iter()
                        .filter(|(p, path, _)| *p == record.part && record.path.starts_with(path))
                        .max_by_key(|(_, path, _)| path.len());
                    if let Some((_, _, id)) = table {
                        entry.insert("table".into(), Value::from(id.as_str()));
                        entry.insert("t".into(), Value::from(text(&record.element)));
                        out.push(Value::Object(entry));
                    }
                }
            }
        }
        Value::Array(out)
    }

    /// The styles the blocks use, by name, sorted by name.
    pub(super) fn styles(&self) -> Value {
        let mut used: Vec<(String, &Arc<Element>)> = self
            .used_styles
            .iter()
            .filter_map(|id| self.docx.style(id).map(|s| (id.clone(), s)))
            .collect();
        used.sort_by(|a, b| {
            let key = |(id, s): &(String, &Arc<Element>)| style_name(s).map_or_else(|| id.clone(), str::to_owned);
            key(a).cmp(&key(b))
        });
        let mut out = Map::new();
        for (id, style) in used {
            let based = child_val(style, "basedOn").map(str::to_owned);
            let based_name = based.as_deref().and_then(|b| self.docx.style(b)).map(|s| {
                let own = style_id(s);
                style_name(s).unwrap_or(own).to_owned()
            });
            let name = style_name(style).unwrap_or(&id).to_owned();
            out.insert(
                name,
                json!({"id": style_id(style), "kind": style_kind(style), "based_on": based_name.or(based)}),
            );
        }
        Value::Object(out)
    }
}

/// The notes a record references (a table's: its cells').
fn referenced(record: &Record) -> Vec<String> {
    match record {
        Record::Paragraph(p) => p.notes.clone(),
        Record::Table(t) => t
            .rows
            .iter()
            .flatten()
            .flat_map(|c| c.records.iter().flat_map(referenced))
            .collect(),
        Record::Marker(_) => Vec::new(),
    }
}

/// Every record, depth first: tables' cells included.
fn walk(record: &Record, visit: &mut dyn FnMut(&Record)) {
    visit(record);
    if let Record::Table(t) = record {
        for cell in t.rows.iter().flatten() {
            for inner in &cell.records {
                walk(inner, visit);
            }
        }
    }
}

fn attribute(element: &Element, local: &str) -> Value {
    element.attribute(Some(W), local).map_or(Value::Null, Value::from)
}

/// A revision record's kind (docx-agent's `revision_kind`): a mark's or a row's own named
/// so.
fn kind(root: &Arc<Element>, record: &Revision) -> String {
    let local = record.element.name().local();
    let base = KINDS.iter().find(|(l, _)| *l == local).map_or("", |(_, k)| *k);
    if !["ins", "del", "moveFrom", "moveTo"].contains(&local) {
        return base.to_owned();
    }
    let parent = record
        .path
        .get(..record.path.len().saturating_sub(1))
        .and_then(|p| at_path(root, p));
    let grand = record
        .path
        .get(..record.path.len().saturating_sub(2))
        .and_then(|p| at_path(root, p));
    match parent {
        Some(p) if p.name().is(Some(W), "rPr") => {
            if grand.is_some_and(|g| g.name().is(Some(W), "pPr")) {
                format!("paragraph-mark-{base}")
            } else {
                format!("run-mark-{base}")
            }
        }
        Some(p) if p.name().is(Some(W), "trPr") => format!("row-{base}"),
        _ => base.to_owned(),
    }
}

/// A revision's text: its `w:t` and `w:delText` (insertions, deletions, moves only).
fn text(element: &Element) -> String {
    if !["ins", "del", "moveFrom", "moveTo"]
        .iter()
        .any(|t| element.name().is(Some(W), t))
    {
        return String::new();
    }
    let mut out = String::new();
    let mut stack = vec![element];
    while let Some(e) = stack.pop() {
        let children: Vec<&Arc<Element>> = e.child_elements().collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
        if e.name().is(Some(W), "t") || e.name().is(Some(W), "delText") {
            out.push_str(&ooxml_word_model::docx::text::leading_text(e));
        }
    }
    out
}

/// Every comment marker of every story but the comments part: by `w:id`, the part and the
/// first range start, range end and reference of that id.
type Markers = HashMap<String, (String, HashMap<&'static str, Box<[u32]>>)>;

fn comment_markers(docx: &Docx, comments_part: Option<&str>) -> Markers {
    let mut out: Markers = HashMap::new();
    for part in &docx.parts {
        if Some(part.part.as_str()) == comments_part {
            continue;
        }
        let mut path = Vec::new();
        every(part.root(), &mut path, &mut |e, path| {
            for local in ["commentRangeStart", "commentRangeEnd", "commentReference"] {
                if e.name().is(Some(W), local) {
                    let id = e.attribute(Some(W), "id").unwrap_or("").to_owned();
                    let found = out.entry(id).or_insert_with(|| (part.part.clone(), HashMap::new()));
                    found.1.entry(local).or_insert_with(|| path.to_vec().into_boxed_slice());
                }
            }
        });
    }
    out
}

fn every(root: &Element, path: &mut Vec<u32>, visit: &mut dyn FnMut(&Element, &[u32])) {
    for (k, node) in root.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        visit(child, path);
        every(child, path, visit);
        path.pop();
    }
}

/// What a comment is attached to: its range (a reply's own, else its thread's), else the
/// place of its reference; `{"id", "t"}` in the final view, or none.
fn anchor(
    docx: &Docx,
    record: &ooxml_word_model::docx::comments::Comment,
    by_id: &HashMap<&str, &ooxml_word_model::docx::comments::Comment>,
    markers: &Markers,
) -> Value {
    let mut seen = HashSet::new();
    let mut fallback: Option<(String, Box<[u32]>)> = None;
    let mut current = Some(record);
    while let Some(r) = current {
        if !seen.insert(r.id.clone()) {
            break;
        }
        if let Some((part, found)) = r.w_id.as_ref().and_then(|w| markers.get(w)) {
            if let Some(start) = found.get("commentRangeStart") {
                return range_value(docx, part, start, found.get("commentRangeEnd").map(|p| &**p));
            }
            if fallback.is_none() {
                if let Some(reference) = found.get("commentReference") {
                    fallback = Some((part.clone(), reference.clone()));
                }
            }
        }
        current = r.parent.as_deref().and_then(|p| by_id.get(p).copied());
    }
    match fallback {
        Some((part, at)) => range_value(docx, &part, &at, Some(&at)),
        None => Value::Null,
    }
}

fn range_value(docx: &Docx, part: &str, start: &[u32], end: Option<&[u32]>) -> Value {
    let Some(story) = docx.part(part) else {
        return Value::Null;
    };
    let Ok(range) = marker_range(story, start, end, View::Current) else {
        return Value::Null;
    };
    let id = if range.start_id == range.end_id {
        format!("{}@{}:{}", range.start_id, range.start, range.end)
    } else {
        format!("{}@{}..{}@{}", range.start_id, range.start, range.end_id, range.end)
    };
    json!({"id": id, "t": range_text(story, &range, View::Current)})
}

/// What a cell states itself, for a reply on one cell (docx-agent's `_cell` and
/// `cell_properties`).
pub(super) fn cell(docx: &Docx, address: &str) -> Option<Value> {
    let found = in_table(docx, address).ok()??;
    let column = found.column?;
    let table = found.part.ids.tables().get(found.table)?;
    let (row_path, _) = table.rows.get(found.row)?;
    let row = at_path(found.part.root(), row_path)?;
    let (tc, start) = cells_of(row)
        .into_iter()
        .find(|(_, start, span)| (*start..start + span).contains(&column))
        .map(|(c, s, _)| (c, s))?;
    let properties = tc.find(Some(W), "tcPr");
    let find = |local: &str| properties.and_then(|p| p.find(Some(W), local));
    let val = |e: Option<&Arc<Element>>| e.and_then(|e| e.attribute(Some(W), "val")).map(str::to_owned);
    let borders = find("tcBorders").map(|b| {
        Value::Object(
            b.child_elements()
                .map(|n| (n.name().local().to_owned(), border(n)))
                .collect(),
        )
    });
    let margins = find("tcMar").map(|m| {
        Value::Object(
            m.child_elements()
                .map(|n| (n.name().local().to_owned(), points(n.attribute(Some(W), "w"))))
                .collect(),
        )
    });
    let span = find("gridSpan")
        .and_then(|g| g.attribute(Some(W), "val"))
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1);
    let merge = find("vMerge").map(|m| {
        m.attribute(Some(W), "val")
            .filter(|v| !v.is_empty())
            .unwrap_or("continue")
            .to_owned()
    });
    let shading = find("shd").map(|s| {
        json!({"fill": s.attribute(Some(W), "fill"), "pattern": s.attribute(Some(W), "val"), "color": s.attribute(Some(W), "color")})
    });
    let _ = start;
    let properties = json!({
        "width": width(find("tcW").map(|e| &**e)),
        "span": span,
        "vertical_merge": merge,
        "shading": shading,
        "borders": borders,
        "margins": margins,
        "vertical_alignment": val(find("vAlign")),
        "text_direction": val(find("textDirection")),
    });
    let cell_column = start_of(row, column)?;
    Some(
        json!({"id": format!("{}/c{},{}", table.id, found.row, cell_column), "table": table.id,
                "row": found.row, "column": cell_column, "properties": properties}),
    )
}

/// A row's own cells with the grid column each starts at and its span.
fn cells_of(row: &Arc<Element>) -> Vec<(Arc<Element>, usize, usize)> {
    let before = find_path(row, &["trPr", "gridBefore"])
        .and_then(|g| g.attribute(Some(W), "val"))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut at = before;
    let mut out = Vec::new();
    for cell in row.child_elements().filter(|c| c.name().is(Some(W), "tc")) {
        let span = find_path(cell, &["tcPr", "gridSpan"])
            .and_then(|g| g.attribute(Some(W), "val"))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1);
        out.push((Arc::clone(cell), at, span));
        at += span;
    }
    out
}

/// The grid column the cell covering `column` starts at.
fn start_of(row: &Arc<Element>, column: usize) -> Option<usize> {
    cells_of(row)
        .into_iter()
        .find(|(_, s, span)| (*s..s + span).contains(&column))
        .map(|(_, s, _)| s)
}

fn points(value: Option<&str>) -> Value {
    value
        .and_then(|v| v.trim().parse::<i64>().ok())
        .and_then(|v| serde_json::Number::from_f64(v as f64 / 20.0))
        .map_or(Value::Null, Value::Number)
}

fn border(node: &Element) -> Value {
    let mut out = Map::new();
    out.insert("style".into(), attribute(node, "val"));
    let int = |local: &str| {
        node.attribute(Some(W), local)
            .map(|v| v.trim().parse::<i64>().unwrap_or(0))
    };
    if let Some(size) = int("sz") {
        out.insert("size".into(), json!(size as f64 / 8.0));
    }
    if node.attribute(Some(W), "color").is_some() {
        out.insert("color".into(), attribute(node, "color"));
    }
    if let Some(space) = int("space") {
        out.insert("space".into(), json!(space));
    }
    if node.attribute(Some(W), "themeColor").is_some() {
        out.insert("theme_color".into(), attribute(node, "themeColor"));
    }
    Value::Object(out)
}

/// A width as docx-agent reads it: `auto`, `n%` or points.
fn width(node: Option<&Element>) -> Value {
    let Some(node) = node else { return Value::Null };
    let kind = node
        .attribute(Some(W), "type")
        .filter(|k| !k.is_empty())
        .unwrap_or("dxa");
    let raw = node.attribute(Some(W), "w").filter(|w| !w.is_empty()).unwrap_or("0");
    match kind {
        "auto" => Value::from("auto"),
        "nil" => Value::Null,
        "pct" => {
            let text = raw.trim_end_matches('%');
            let value = if raw.ends_with('%') {
                text.parse::<f64>().ok()
            } else {
                text.parse::<i64>().ok().map(|v| v as f64 / 50.0)
            };
            value.map_or(Value::Null, |v| Value::from(format!("{}%", python_g(v))))
        }
        _ => points(Some(raw)),
    }
}

/// Python's `{:g}` for the values a width in percent takes.
fn python_g(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e16 {
        format!("{}", value as i64)
    } else {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}
