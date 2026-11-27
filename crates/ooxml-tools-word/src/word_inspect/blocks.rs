//! Owns: the blocks of `word_inspect`'s reply (docx-agent's `state._Writer`): a paragraph
//! with its effective formatting, runs and what it holds, a table with its cells' fill and
//! drawn text colour, a block-level content control with its blocks; and what the blocks
//! reference, gathered for the rest of the reply (notes, comments, revisions, styles).
//!
//! Must not: read records itself (the Markdown reader makes them), or lay out.

use ooxml_word_model::docx::cells::model_path;
use ooxml_word_model::docx::formatting::{Cell, EffectiveParagraph, EffectiveRun};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::W;
use ooxml_word_model::markdown::read::{
    CellRecord, Kind, ParagraphRecord, Placed, Reader, Record, RunRecord, TableRecord,
};
use ooxml_word_model::value::{Props, Value as PropValue};
use ooxml_xml::{Children, Element, Node};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// What the blocks written reference, for the rest of the reply.
pub(super) struct Writer<'d> {
    pub(super) docx: &'d Docx,
    /// Notes to write, in the order met.
    pub(super) note_labels: Vec<String>,
    /// Every paragraph written, and those it joins.
    pub(super) paragraph_ids: BTreeSet<String>,
    /// The paragraphs referencing each comment, by comment id.
    pub(super) comment_places: BTreeMap<String, Vec<String>>,
    /// Style ids the blocks use.
    pub(super) used_styles: BTreeSet<String>,
    /// Every table written: part, place, id.
    pub(super) tables: Vec<(String, Box<[u32]>, String)>,
    /// With `layout`: each block to place (its id, then the ids it joins), in the order
    /// their places were kept (`None`: blocks are not placed).
    pub(super) placing: Option<Vec<Vec<String>>>,
}

/// Python's `bool`-as-JSON of an optional string.
fn opt(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::from)
}

/// The construct's name.
fn construct(kind: Kind) -> &'static str {
    match kind {
        Kind::Paragraph => "paragraph",
        Kind::Heading => "heading",
        Kind::Bullet => "bullet",
        Kind::Ordered => "ordered",
        Kind::Quote => "quote",
        Kind::Code => "code",
        Kind::Rule => "rule",
    }
}

/// A float as Python writes it in JSON (`11.0`).
fn float(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
}

/// The fields of `effective` that a run shows where they differ from the baseline.
fn run_fields(run: &EffectiveRun) -> [(&'static str, Value); 12] {
    [
        ("b", Value::Bool(run.bold)),
        ("i", Value::Bool(run.italic)),
        ("u", opt(run.underline.as_deref())),
        ("strike", Value::Bool(run.strike)),
        ("dstrike", Value::Bool(run.double_strike)),
        ("caps", Value::Bool(run.caps)),
        ("small_caps", Value::Bool(run.small_caps)),
        ("sz", float(run.size)),
        ("font", opt(run.font.as_deref())),
        ("color", Value::from(run.color.as_str())),
        ("highlight", opt(run.highlight.as_deref())),
        ("valign", opt(run.vertical_align.as_deref())),
    ]
}

impl<'d> Writer<'d> {
    pub(super) fn new(docx: &'d Docx) -> Writer<'d> {
        Writer {
            docx,
            note_labels: Vec::new(),
            paragraph_ids: BTreeSet::new(),
            comment_places: BTreeMap::new(),
            used_styles: BTreeSet::new(),
            tables: Vec::new(),
            placing: None,
        }
    }

    /// Keep the place of `out`'s placements (the next key), to be filled once every block
    /// is written and the layout asked at once (`state._Writer._place`).
    fn place(&mut self, out: &mut Map<String, Value>, id: &str, joins: &[String]) {
        if let Some(asked) = &mut self.placing {
            out.insert("placements".into(), crate::placements::pending(asked.len()));
            asked.push(std::iter::once(id.to_owned()).chain(joins.iter().cloned()).collect());
        }
    }

    /// The blocks of `records`; `cell` the table style of the cell they are in.
    pub(super) fn blocks(&mut self, reader: &mut Reader<'_>, records: &[Record], cell: Option<&str>) -> Vec<Value> {
        let mut stack: Vec<Vec<Value>> = vec![Vec::new()];
        let mut controls: Vec<Map<String, Value>> = Vec::new();
        for record in records {
            match record {
                Record::Marker(m) if m.begin => {
                    let mut control = Map::new();
                    control.insert("type".into(), Value::from("content_control"));
                    control.insert("id".into(), Value::from(m.id.as_str()));
                    control.insert("kind".into(), Value::from(m.kind));
                    control.insert("tag".into(), opt(m.tag.as_deref()));
                    control.insert("alias".into(), opt(m.alias.as_deref()));
                    if !m.joins.is_empty() {
                        control.insert("joins".into(), json!(m.joins));
                    }
                    controls.push(control);
                    stack.push(Vec::new());
                }
                Record::Marker(_) => {
                    if stack.len() > 1 {
                        let inner = stack.pop().unwrap_or_default();
                        if let Some(mut control) = controls.pop() {
                            control.insert("blocks".into(), Value::Array(inner));
                            if let Some(top) = stack.last_mut() {
                                top.push(Value::Object(control));
                            }
                        }
                    }
                }
                Record::Paragraph(p) => {
                    let out = self.paragraph(reader, p, cell);
                    if let Some(top) = stack.last_mut() {
                        top.push(out);
                    }
                }
                Record::Table(t) => {
                    let out = self.table(reader, t);
                    if let Some(top) = stack.last_mut() {
                        top.push(out);
                    }
                }
            }
        }
        // A control left open (its end out of the records) is written with what it holds.
        while stack.len() > 1 {
            let inner = stack.pop().unwrap_or_default();
            if let Some(mut control) = controls.pop() {
                control.insert("blocks".into(), Value::Array(inner));
                if let Some(top) = stack.last_mut() {
                    top.push(Value::Object(control));
                }
            }
        }
        stack.pop().unwrap_or_default()
    }

    fn paragraph(&mut self, reader: &mut Reader<'_>, record: &ParagraphRecord, cell: Option<&str>) -> Value {
        self.paragraph_ids.insert(record.id.clone());
        self.paragraph_ids.extend(record.joins.iter().cloned());
        let default = self
            .docx
            .default_style("paragraph")
            .map(|s| ooxml_word_model::docx::style_id(s).to_owned());
        if let Some(style) = record.style_id.clone().or(default) {
            if !style.is_empty() {
                self.used_styles.insert(style);
            }
        }
        let mut out = Map::new();
        out.insert("type".into(), Value::from("paragraph"));
        out.insert("id".into(), Value::from(record.id.as_str()));
        if !record.joins.is_empty() {
            out.insert("joins".into(), json!(record.joins));
        }
        out.insert("story".into(), Value::from(record.story.as_str()));
        out.insert("style".into(), opt(record.style_name.as_deref()));
        out.insert("markdown".into(), Value::from(construct(record.kind)));
        match record.kind {
            Kind::Heading => {
                out.insert("level".into(), Value::from(record.level));
            }
            Kind::Bullet | Kind::Ordered => {
                out.insert("level".into(), Value::from(record.level + 1));
            }
            _ => {}
        }
        if let Some(member) = &record.membership {
            out.insert(
                "list".into(),
                json!({"num_id": member.num_id, "level": member.level, "format": member.format,
                       "label": record.label, "number": record.number, "from_style": member.from_style}),
            );
        }
        out.insert("t".into(), Value::from(record.text.as_str()));
        let Some(last) = record.elements.last() else {
            return Value::Object(out);
        };
        let cell = cell.and_then(|style| reader.cell_of(&record.part, last, style));
        let formatting = reader.shared().formatting();
        let owner = properties_owner(reader, record, last);
        let base = formatting.run(&owner, None, 'a', cell.as_ref());
        let para = formatting.paragraph(&owner, cell.as_ref());
        out.insert("effective".into(), effective(&base, &para));
        let runs: Vec<Value> = record.detail.runs.iter().map(|r| self.run(r, &base)).collect();
        out.insert("runs".into(), Value::Array(runs));
        let written = new_text(reader, &owner, &base, cell.as_ref());
        if !written.is_empty() {
            out.insert("new_text".into(), Value::Object(written));
        }
        self.details(&mut out, record);
        for comment in &record.detail.comments {
            self.comment_places
                .entry(comment.clone())
                .or_default()
                .push(record.id.clone());
        }
        if !record.text_boxes.is_empty() {
            let mut boxes = Vec::new();
            for (id, content) in &record.text_boxes {
                let records = reader.container(&record.part, content);
                boxes.push(json!({"id": id, "blocks": self.blocks(reader, &records, None)}));
            }
            out.insert("text_boxes".into(), Value::Array(boxes));
        }
        if let Some((id, kind)) = &record.section {
            out.insert("section".into(), json!({"id": id, "type": kind}));
        }
        self.place(&mut out, &record.id, &record.joins);
        Value::Object(out)
    }

    /// Links, bookmarks, notes, comments, revisions, pictures, drawings, controls, fields.
    fn details(&self, out: &mut Map<String, Value>, record: &ParagraphRecord) {
        let d = &record.detail;
        let lists: [(&str, Vec<Value>); 9] = [
            (
                "links",
                d.links
                    .iter()
                    .map(|(id, href, t)| json!({"id": id, "href": href, "t": t}))
                    .collect(),
            ),
            (
                "bookmarks",
                d.bookmarks.iter().map(|b| Value::from(b.as_str())).collect(),
            ),
            ("notes", record.notes.iter().map(|n| Value::from(n.as_str())).collect()),
            ("comments", d.comments.iter().map(|c| Value::from(c.as_str())).collect()),
            (
                "revisions",
                d.revisions.iter().flatten().map(|r| Value::from(r.as_str())).collect(),
            ),
            (
                "pictures",
                d.drawings.iter().filter(|x| x.picture).map(picture).collect(),
            ),
            (
                "drawings",
                d.drawings.iter().filter(|x| !x.picture).map(drawing).collect(),
            ),
            (
                "controls",
                d.controls
                    .iter()
                    .map(|(id, kind)| json!({"id": id, "kind": kind}))
                    .collect(),
            ),
            (
                "fields",
                d.fields
                    .iter()
                    .map(|(i, r)| json!({"instruction": i, "result": r}))
                    .collect(),
            ),
        ];
        for (key, values) in lists {
            if !values.is_empty() {
                out.insert(key.into(), Value::Array(values));
            }
        }
    }

    fn run(&mut self, run: &RunRecord, base: &EffectiveRun) -> Value {
        let mut out = Map::new();
        out.insert("t".into(), Value::from(run.text.as_str()));
        if let Some(style) = &run.style {
            if !style.is_empty() {
                out.insert("style".into(), Value::from(style.as_str()));
                for s in self.docx.styles() {
                    if ooxml_word_model::docx::style_name(s) == Some(style.as_str()) {
                        self.used_styles.insert(ooxml_word_model::docx::style_id(s).to_owned());
                    }
                }
            }
        }
        if !run.direct.is_empty() {
            out.insert("direct".into(), json!(run.direct));
        }
        for ((key, value), (_, baseline)) in run_fields(&run.effective).into_iter().zip(run_fields(base)) {
            if value != baseline {
                out.insert(key.into(), value);
            }
        }
        if let Some(revision) = &run.revision {
            out.insert("rev".into(), Value::from(revision.as_str()));
        }
        Value::Object(out)
    }

    fn table(&mut self, reader: &mut Reader<'_>, record: &TableRecord) -> Value {
        self.tables
            .push((record.part.clone(), record.path.clone(), record.id.clone()));
        if let Some(style) = ooxml_word_model::docx::find_path(&record.element, &["tblPr", "tblStyle"]) {
            self.used_styles
                .insert(style.attribute(Some(W), "val").unwrap_or("").to_owned());
        }
        let mut out = Map::new();
        out.insert("type".into(), Value::from("table"));
        out.insert("id".into(), Value::from(record.id.as_str()));
        if !record.joins.is_empty() {
            out.insert("joins".into(), json!(record.joins));
        }
        out.insert("story".into(), Value::from(story_of(self.docx, &record.part)));
        out.insert("style".into(), opt(record.style_name.as_deref()));
        out.insert("columns".into(), Value::from(record.columns));
        out.insert("header_rows".into(), Value::from(record.header_rows));
        let mut rows = Vec::new();
        for line in &record.rows {
            let mut cells = Vec::new();
            for cell in line {
                let mut c = Map::new();
                c.insert("id".into(), Value::from(cell.id.as_str()));
                c.insert("row".into(), Value::from(cell.row));
                c.insert("column".into(), Value::from(cell.column));
                c.insert("row_span".into(), Value::from(cell.row_span));
                c.insert("column_span".into(), Value::from(cell.column_span));
                for (k, v) in self.cell_colours(reader, record, cell) {
                    c.insert(k, v);
                }
                let blocks = self.blocks(reader, &cell.records, Some(&record.table_style));
                c.insert("blocks".into(), Value::Array(blocks));
                cells.push(Value::Object(c));
            }
            rows.push(Value::Array(cells));
        }
        out.insert("rows".into(), Value::Array(rows));
        self.place(&mut out, &record.id, &[]);
        Value::Object(out)
    }

    /// A cell's `fill` as drawn and `text_color` (docx-agent's `cell_colours`).
    fn cell_colours(&self, reader: &Reader<'_>, table: &TableRecord, cell: &CellRecord) -> Map<String, Value> {
        let mut out = Map::new();
        let Some(props) = resolved_cell(reader, table, cell) else {
            return out;
        };
        let shd = props.get("shd");
        let fill = shading(shd);
        let automatic = automatic_text(shd);
        let formatting = reader.shared().formatting();
        let drawn = |resolved: &Props, shaded: bool| -> String {
            if let Some(colour) = colour(resolved.get("color")) {
                return colour.to_uppercase();
            }
            let highlighted = resolved
                .get("highlight")
                .and_then(PropValue::str)
                .is_some_and(|h| highlight(h).is_some());
            match &automatic {
                Some(white) if !shaded && !highlighted && shading(resolved.get("shd")).is_none() => white.clone(),
                _ => "000000".to_owned(),
            }
        };
        let mut colours: Vec<String> = Vec::new();
        let mut first: Option<(&ParagraphRecord, Option<Cell>)> = None;
        for record in &cell.records {
            let Record::Paragraph(p) = record else { continue };
            let Some(last) = p.elements.last() else { continue };
            let here = reader.cell_of(&p.part, last, &table.table_style);
            if first.is_none() {
                first = Some((p, here.clone()));
            }
            let shaded = shading(formatting.paragraph(&last.element, here.as_ref()).resolved.get("shd")).is_some();
            for run in &p.detail.runs {
                if !run.text.is_empty() && !run.effective.hidden {
                    let value = drawn(&run.effective.resolved, shaded);
                    if !colours.contains(&value) {
                        colours.push(value);
                    }
                }
            }
        }
        if colours.is_empty() {
            if let Some((p, here)) = first {
                if let Some(last) = p.elements.last() {
                    let shaded =
                        shading(formatting.paragraph(&last.element, here.as_ref()).resolved.get("shd")).is_some();
                    let base = formatting.run(&last.element, None, 'a', here.as_ref());
                    colours.push(drawn(&base.resolved, shaded));
                }
            }
        }
        out.insert("fill".into(), opt(fill.map(|f| f.to_uppercase()).as_deref()));
        match colours.as_slice() {
            [one] => {
                out.insert("text_color".into(), Value::from(one.as_str()));
            }
            [] => {}
            _ => {
                out.insert("text_color".into(), Value::from("mixed"));
                out.insert("text_colors".into(), json!(colours));
            }
        }
        out
    }
}

/// The paragraph whose properties a record's formatting reads: its last element, but for a
/// paragraph of a story (not a cell's: Accept All joins those across cell ends) whose mark the
/// final view deletes with nothing after it to join, which has no properties of its own there (the layout model's
/// `following_properties`: the story's next paragraph's, none at its end).
fn properties_owner(reader: &Reader<'_>, record: &ParagraphRecord, last: &Placed) -> Arc<Element> {
    let removed = ooxml_word_model::docx::find_path(&last.element, &["pPr", "rPr"]).is_some_and(|mark| {
        mark.child_elements()
            .any(|c| c.name().is(Some(W), "del") || c.name().is(Some(W), "moveFrom"))
    });
    let at_end = || {
        let Some(part) = reader.docx.part(&record.part) else {
            return false;
        };
        let Some((k, parent)) = last.path.split_last() else {
            return false;
        };
        let Some(container) = ooxml_word_model::docx::at_path(part.root(), parent) else {
            return false;
        };
        !container
            .children()
            .iter()
            .skip(*k as usize + 1)
            .any(|n| matches!(n, Node::Element(e) if ["p", "tbl", "sdt"].iter().any(|t| e.name().is(Some(W), t))))
    };
    // A block of the body (`w:document/w:body/w:p`) or of a header's or footer's root.
    let story_level = last.path.len() <= 2;
    if !(removed && story_level && at_end()) {
        return Arc::clone(&last.element);
    }
    let kept: Vec<Node> = last
        .element
        .children()
        .iter()
        .filter(|n| !matches!(n, Node::Element(e) if e.name().is(Some(W), "pPr")))
        .cloned()
        .collect();
    Arc::new(last.element.with_children(Children::from_vec(kept)))
}

/// A paragraph's formatting and its runs' baseline, as `effective` shows them.
fn effective(base: &EffectiveRun, para: &EffectiveParagraph) -> Value {
    json!({
        "font": base.font, "sz": float(base.size), "color": base.color, "b": base.bold, "i": base.italic,
        "alignment": para.alignment,
        "indent": {"left": float(para.indent_left), "right": float(para.indent_right),
                   "first_line": float(para.first_line), "hanging": float(para.hanging)},
        "spacing": {"before": float(para.space_before), "after": float(para.space_after),
                    "line": para.line_spacing.map(float), "rule": para.line_rule},
        "keep_with_next": para.keep_with_next, "keep_lines": para.keep_together,
        "page_break_before": para.page_break_before, "outline_level": para.outline_level,
    })
}

/// The layout model's resolved properties of a cell (none for one in a text box).
fn resolved_cell(reader: &Reader<'_>, table: &TableRecord, cell: &CellRecord) -> Option<Props> {
    let part = reader.docx.part(&table.part)?;
    let (top, at) = (1..=cell.path.len()).find_map(|n| {
        let prefix = cell.path.get(..n)?;
        part.ids.table_at(prefix).map(|t| (t, n))
    })?;
    if in_text_box(part.root(), &cell.path) {
        return None;
    }
    let facts = reader.shared().table_facts(&table.part, &top);
    let path = model_path(&top.element, cell.path.get(at..)?)?;
    facts.cell_properties(&path).cloned()
}

fn in_text_box(root: &Arc<Element>, path: &[u32]) -> bool {
    let mut node = Arc::clone(root);
    for k in path {
        let Some(next) = node.children().get(*k as usize).and_then(Node::as_element).cloned() else {
            return false;
        };
        if next.name().is(Some(W), "txbxContent") {
            return true;
        }
        node = next;
    }
    false
}

/// docx2svg's `_color`: a six-digit hex colour, else none.
fn colour(value: Option<&PropValue>) -> Option<String> {
    let text = value.and_then(PropValue::str)?;
    (text != "auto" && text.len() == 6 && text.chars().all(|c| c.is_ascii_hexdigit())).then(|| text.to_owned())
}

/// docx2svg's `_shading`: a shading's fill, none for `nil` or no colour.
fn shading(value: Option<&PropValue>) -> Option<String> {
    let PropValue::Shading(s) = value? else { return None };
    if s.kind.as_deref() == Some("nil") {
        return None;
    }
    let fill = s.fill.as_deref()?;
    (fill != "auto" && fill.len() == 6 && fill.chars().all(|c| c.is_ascii_hexdigit())).then(|| fill.to_owned())
}

/// White where the fill is darker than Word's measured threshold (Rec. 601 luma below 60).
fn automatic_text(shd: Option<&PropValue>) -> Option<String> {
    let fill = shading(shd)?;
    let byte = |k: usize| {
        fill.get(k..k + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(f64::from)
    };
    let (r, g, b) = (byte(0)?, byte(2)?, byte(4)?);
    (0.299 * r + 0.587 * g + 0.114 * b < 60.0).then(|| "FFFFFF".to_owned())
}

/// `w:highlight`'s colour names.
fn highlight(name: &str) -> Option<&'static str> {
    Some(match name {
        "black" => "000000",
        "blue" => "0000FF",
        "cyan" => "00FFFF",
        "green" => "00FF00",
        "magenta" => "FF00FF",
        "red" => "FF0000",
        "yellow" => "FFFF00",
        "white" => "FFFFFF",
        "darkBlue" => "000080",
        "darkCyan" => "008080",
        "darkGreen" => "008000",
        "darkMagenta" => "800080",
        "darkRed" => "800000",
        "darkYellow" => "808000",
        "darkGray" => "808080",
        "lightGray" => "C0C0C0",
        _ => return None,
    })
}

fn story_of(docx: &Docx, part: &str) -> String {
    docx.part(part).map(|p| p.story.clone()).unwrap_or_default()
}

fn picture(d: &ooxml_word_model::markdown::read::DrawingRecord) -> Value {
    json!({"id": d.id, "name": d.name, "alt": d.alt, "floating": d.floating,
           "size": d.size.map(|(w, h)| vec![float(w), float(h)])})
}

fn drawing(d: &ooxml_word_model::markdown::read::DrawingRecord) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), opt(d.id.as_deref()));
    out.insert("kind".into(), Value::from(d.kind));
    out.insert("name".into(), opt(d.name.as_deref()));
    out.insert("alt".into(), Value::from(d.alt.as_str()));
    out.insert("floating".into(), Value::Bool(d.floating));
    if let Some(graphic) = &d.graphic {
        out.insert(graphic.names().0.into(), graphic.json());
    }
    if let Some(members) = &d.members {
        let members: Vec<Value> = members
            .iter()
            .map(|m| {
                let mut one = Map::new();
                one.insert("id".into(), Value::from(m.id.as_str()));
                one.insert("kind".into(), Value::from(m.kind));
                one.insert("name".into(), opt(m.name.as_deref()));
                if let Some(graphic) = &m.graphic {
                    one.insert(graphic.names().0.into(), graphic.json());
                }
                Value::Object(one)
            })
            .collect();
        out.insert("members".into(), Value::Array(members));
    }
    Value::Object(out)
}

/// What new text written over the paragraph gets where it differs from `base` (its first
/// run of text's formatting, else its mark's), docx-agent's `new_text`.
fn new_text(reader: &Reader<'_>, paragraph: &Element, base: &EffectiveRun, cell: Option<&Cell>) -> Map<String, Value> {
    let formatting = reader.shared().formatting();
    let got = match first_text_run(paragraph) {
        Some(run) => formatting.run(paragraph, Some(&run), 'a', cell),
        None => {
            let Some(mark) = ooxml_word_model::docx::find_path(paragraph, &["pPr", "rPr"]) else {
                return Map::new();
            };
            let kept: Vec<Node> = mark
                .children()
                .iter()
                .filter(|n| match n {
                    Node::Element(e) => !["ins", "del", "moveFrom", "moveTo", "rPrChange"]
                        .iter()
                        .any(|t| e.name().is(Some(W), t)),
                    _ => false,
                })
                .cloned()
                .collect();
            if kept.is_empty() {
                return Map::new();
            }
            let properties = mark.with_children(Children::from_vec(kept));
            formatting.run_properties(paragraph, Some(&properties), 'a', cell)
        }
    };
    let mut out = Map::new();
    let pairs = [
        ("font", opt(got.font.as_deref()), opt(base.font.as_deref())),
        ("sz", float(got.size), float(base.size)),
        (
            "color",
            Value::from(got.color.as_str()),
            Value::from(base.color.as_str()),
        ),
        ("b", Value::Bool(got.bold), Value::Bool(base.bold)),
        ("i", Value::Bool(got.italic), Value::Bool(base.italic)),
    ];
    for (key, value, baseline) in pairs {
        if value != baseline {
            out.insert(key.into(), value);
        }
    }
    out
}

/// The paragraph's first run that shows text: a `w:t`, not in a deletion, a move's source,
/// a nested paragraph or a text box.
fn first_text_run(paragraph: &Element) -> Option<Arc<Element>> {
    fn walk(node: &Element) -> Option<Arc<Element>> {
        for child in node.child_elements() {
            if child.name().is(Some(W), "r") {
                if child.find(Some(W), "t").is_some() {
                    return Some(Arc::clone(child));
                }
                // A run's own descendants hold no shown run of this paragraph.
                continue;
            }
            if ["moveFrom", "del", "p", "txbxContent"]
                .iter()
                .any(|t| child.name().is(Some(W), t))
            {
                continue;
            }
            if let Some(found) = walk(child) {
                return Some(found);
            }
        }
        None
    }
    walk(paragraph)
}
