//! Owns: `word_design_facts`' page and block facts (docx-agent's `edit/design.py`,
//! "sections", "table frames", "theme" and "patterns"): each section's page grid and the
//! header and footer each variant shows; each top-level body table's frame against its
//! section's body, with the width most of them share; the theme's name, palette and fonts;
//! and the block shapes repeated under headings of one level.
//!
//! Must not: change a document, or lay pages out (a frame is what a table states).

use super::design_look::{pt, pt_or_null, Design};
use super::design_styles::empty;
use super::design_tables::element_at;
use crate::table_read::{grid_widths, own_rows, row_cells, width};
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::outline::{body_blocks, OutlineLevels};
use ooxml_word_model::docx::sections::{section_story, sections, Section, KINDS, MARGINS};
use ooxml_word_model::docx::{find_path, style_id, style_name, Docx};
use ooxml_word_model::ids::W;
use ooxml_word_model::model::Table;
use ooxml_word_model::value::key;
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::Element;
use serde_json::{Map, Value};

/// Rows with an empty last cell listed per table; the rest counted.
const EMPTY_ROWS_LISTED: usize = 10;
/// The theme's colour slots, in Word's order.
const THEME_SLOTS: [&str; 12] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink",
];

/// Each section's page grid and the stories its header and footer variants show.
pub(crate) fn section_facts(design: &Design<'_>) -> Vec<Value> {
    let docx = design.docx;
    let even = design.formatting().context().settings.even_and_odd_headers;
    let all = sections(docx);
    let mut out = Vec::new();
    for (index, section) in all.iter().enumerate() {
        let mut item = Map::new();
        item.insert("id".into(), Value::from(section.id.as_str()));
        item.insert("start".into(), Value::from(section.start()));
        item.insert("orientation".into(), Value::from(section.orientation()));
        let (w, h) = section.page().map_or((None, None), |(w, h)| (Some(w), Some(h)));
        item.insert("page".into(), Value::Array(vec![pt_or_null(w), pt_or_null(h)]));
        let margins: Map<String, Value> = MARGINS
            .iter()
            .filter_map(|side| section.margin(side).map(|v| ((*side).to_owned(), pt(v))))
            .collect();
        item.insert("margins".into(), Value::Object(margins));
        item.insert("columns".into(), columns(section));
        let title = section.title_page();
        if title {
            item.insert("title_page".into(), Value::Bool(true));
        }
        let mut linked = Vec::new();
        for which in ["header", "footer"] {
            let mut shown = Map::new();
            for kind in KINDS {
                if (kind == "first" && !title) || (kind == "even" && !even) {
                    continue;
                }
                let story = section_story(docx, &all, index, which, kind);
                shown.insert(kind.into(), story.map_or(Value::Null, Value::from));
                if !section.owns(which, kind) && index > 0 {
                    linked.push(Value::from(format!("{which} {kind}")));
                }
            }
            item.insert(format!("{which}s"), Value::Object(shown));
        }
        if !linked.is_empty() {
            item.insert("linked_to_previous".into(), Value::Array(linked));
        }
        out.push(Value::Object(item));
    }
    out
}

fn columns(section: &Section) -> Value {
    let columns = section.column_layout();
    let mut out = Map::new();
    out.insert("count".into(), Value::from(columns.count));
    out.insert("space".into(), pt_or_null(columns.space));
    if columns.separator {
        out.insert("separator".into(), Value::Bool(true));
    }
    if !columns.widths.is_empty() {
        let widths: Vec<Value> = columns
            .widths
            .iter()
            .map(|(w, s)| Value::Array(vec![pt_or_null(*w), pt(*s)]))
            .collect();
        out.insert("widths".into(), Value::Array(widths));
    }
    Value::Object(out)
}

/// Whether a cell holds no text and nothing drawn.
fn empty_cell(cell: &Element) -> bool {
    let mut stack = vec![cell];
    while let Some(e) = stack.pop() {
        for child in e.child_elements() {
            if child.name().namespace() == Some(W) {
                match child.name().local() {
                    "t" if !child.text_content().trim().is_empty() => return false,
                    "drawing" | "pict" | "object" | "sym" | "fldSimple" | "sdt" => return false,
                    _ => {}
                }
            }
            stack.push(child);
        }
    }
    true
}

/// The rows of a table whose last cell is empty, with the height each states.
fn empty_rows(table: &Element) -> Vec<Value> {
    let mut out = Vec::new();
    for (number, row) in own_rows(table).iter().enumerate() {
        let cells = row_cells(row);
        let last = match cells.as_slice() {
            [_, .., last] => last,
            _ => continue,
        };
        if !empty_cell(&last.0.element) {
            continue;
        }
        let mut found = Map::new();
        found.insert("row".into(), Value::from(number));
        found.insert("height".into(), Value::Null);
        let height = find_path(&row.element, &["trPr", "trHeight"]);
        if let Some(value) = height.and_then(|h| h.attribute(Some(W), "val")).and_then(py_int) {
            found.insert("height".into(), pt(value as f64 / 20.0));
            let exact = height.and_then(|h| h.attribute(Some(W), "hRule")) == Some("exact");
            found.insert("rule".into(), Value::from(if exact { "exact" } else { "at_least" }));
        }
        out.push(Value::Object(found));
    }
    out
}

/// Each top-level body table's frame as it states it, with `narrower_than_usual` on one
/// narrower than the width most share.
pub(crate) fn table_frames(design: &Design<'_>) -> Vec<Value> {
    let docx = design.docx;
    let Some(body) = docx.body() else {
        return Vec::new();
    };
    let all = sections(docx);
    let default = docx.default_style("table").map(|s| style_id(s).to_owned());
    let mut out = Vec::new();
    for table in body.ids.tables().iter() {
        let in_cell = (1..table.path.len())
            .filter_map(|n| element_at(body.root(), table.path.get(..n)?))
            .any(|e| e.name().is(Some(W), "tc"));
        if in_cell {
            continue;
        }
        let element = &table.element;
        let section = all.iter().find(|s| *s.path > *table.path).or_else(|| all.last());
        let text_width = section.and_then(Section::text_width).filter(|w| *w != 0);
        let body_width = text_width.map(|w| w as f64 / 20.0);
        let columns: Vec<f64> = grid_widths(element).iter().map(|w| *w as f64 / 20.0).collect();
        let stated = width(find_path(element, &["tblPr", "tblW"]));
        let frame = match (&stated, body_width) {
            (Value::Number(n), _) if n.as_f64().is_some_and(|v| v > 0.0) => n.as_f64().unwrap_or(0.0),
            (Value::String(s), Some(body)) if s.ends_with('%') => {
                body * s.trim_end_matches('%').parse::<f64>().unwrap_or(0.0) / 100.0
            }
            _ => columns.iter().sum(),
        };
        let indent = find_path(element, &["tblPr", "tblInd"])
            .filter(|n| n.attribute(Some(W), "type").filter(|t| !t.is_empty()).unwrap_or("dxa") == "dxa")
            .map_or(0.0, |n| {
                n.attribute(Some(W), "w").and_then(py_int).unwrap_or(0) as f64 / 20.0
            });
        let style = match find_path(element, &["tblPr", "tblStyle"]) {
            Some(node) => node.attribute(Some(W), "val").map(str::to_owned),
            None => default.clone(),
        };
        let name = style
            .as_deref()
            .map(|id| docx.style(id).and_then(|s| style_name(s)).unwrap_or(id).to_owned());
        let mut item = Map::new();
        item.insert("table".into(), Value::from(table.id.as_str()));
        item.insert("style".into(), name.map_or(Value::Null, Value::from));
        item.insert("width".into(), pt(frame));
        if indent != 0.0 {
            item.insert("indent".into(), pt(indent));
        }
        if let Some(body) = body_width {
            item.insert("body".into(), pt(body));
            let reach = indent + frame - body;
            if reach > 0.5 {
                item.insert("over_body".into(), pt(reach));
            }
        }
        item.insert("columns".into(), Value::Array(columns.iter().map(|c| pt(*c)).collect()));
        let empty = empty_rows(element);
        if !empty.is_empty() {
            let more = empty.len().saturating_sub(EMPTY_ROWS_LISTED);
            item.insert(
                "empty_last_cell".into(),
                Value::Array(empty.into_iter().take(EMPTY_ROWS_LISTED).collect()),
            );
            if more > 0 {
                item.insert("empty_last_cell_more".into(), Value::from(more));
            }
        }
        out.push(Value::Object(item));
    }
    if let Some(usual) = usual_width(&out).as_ref().and_then(Value::as_f64) {
        for item in &mut out {
            let narrower = item
                .get("width")
                .and_then(Value::as_f64)
                .is_some_and(|w| w < usual - 0.5);
            if let (true, Value::Object(map)) = (narrower, item) {
                map.insert("narrower_than_usual".into(), Value::Bool(true));
            }
        }
    }
    out
}

/// The width most of the frames share (within half a point), where at least two do.
pub(crate) fn usual_width(frames: &[Value]) -> Option<Value> {
    let mut groups: Vec<(f64, Vec<&Value>)> = Vec::new();
    for frame in frames {
        let Some(width) = frame.get("width") else { continue };
        let at = (width.as_f64().unwrap_or(0.0) * 2.0).round_ties_even() / 2.0;
        match groups.iter_mut().find(|(k, _)| *k == at) {
            Some((_, list)) => list.push(width),
            None => groups.push((at, vec![width])),
        }
    }
    let mut best: Option<&Vec<&Value>> = None;
    for (_, widths) in &groups {
        if best.is_none_or(|b| widths.len() > b.len()) {
            best = Some(widths);
        }
    }
    best.filter(|w| w.len() >= 2)
        .and_then(|w| w.first().map(|v| (*v).clone()))
}

/// The theme's name, palette and fonts (`None` where the package has no theme).
pub(crate) fn theme(design: &Design<'_>) -> Value {
    let docx = design.docx;
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
    let root = ooxml_word_model::parts::related(&docx.package, &docx.main, rel)
        .ok()
        .flatten()
        .and_then(|part| docx.package.tree(&part).ok().flatten());
    let context = design.formatting().context();
    let scheme = &context.colours.scheme;
    if root.is_none() && scheme.is_empty() {
        return Value::Null;
    }
    let mut out = Map::new();
    if let Some(name) = root
        .as_ref()
        .and_then(|t| t.root().attribute(None, "name"))
        .filter(|n| !n.is_empty())
    {
        out.insert("name".into(), Value::from(name));
    }
    let colors: Map<String, Value> = THEME_SLOTS
        .iter()
        .filter_map(|slot| scheme.get(*slot).map(|v| ((*slot).to_owned(), Value::from(&**v))))
        .collect();
    out.insert("colors".into(), Value::Object(colors));
    if let Some(fonts) = &context.font_scheme {
        let mut map = Map::new();
        for (name, faces) in [("major", &fonts.major), ("minor", &fonts.minor)] {
            let mut entry = Map::new();
            entry.insert("latin".into(), Value::from(&*faces.latin));
            if !faces.east_asian.is_empty() {
                entry.insert("east_asian".into(), Value::from(&*faces.east_asian));
            }
            if !faces.complex_script.is_empty() {
                entry.insert("complex_script".into(), Value::from(&*faces.complex_script));
            }
            map.insert(name.into(), Value::Object(entry));
        }
        out.insert("fonts".into(), Value::Object(map));
    }
    Value::Object(out)
}

/// One body block as a shape reads it: its heading level, its token (`None`: not counted)
/// and its id.
type Token = (Option<i64>, Option<String>, Option<String>);

/// A shape (heading level and tokens) with the headings having it and their last blocks.
type Shapes = Vec<((i64, Vec<String>), Vec<(String, String)>)>;

fn tokens(design: &Design<'_>) -> Vec<Token> {
    let docx: &Docx = design.docx;
    let levels = OutlineLevels::new(docx);
    let sources = design.formatting().sources();
    let mut out = Vec::new();
    for block in body_blocks(docx, &controls(docx)) {
        let element = &block.element;
        let id = block.id.clone();
        let indexed = id.as_deref().and_then(|i| docx.body()?.ids.get(i)).is_some();
        if element.name().is(Some(W), "p") {
            if !indexed || empty(element) {
                out.push((None, None, id));
                continue;
            }
            let style = levels.style_id(element);
            let name = style
                .as_deref()
                .map(|s| docx.style(s).and_then(|e| style_name(e)).unwrap_or(s).to_owned());
            out.push((block.level, Some(name.unwrap_or_default()), id));
        } else if element.name().is(Some(W), "tbl") && indexed {
            let declared = find_path(element, &["tblPr", "tblStyle"])
                .and_then(|s| s.attribute(Some(W), "val"))
                .unwrap_or("");
            let probe = Table {
                style_id: Some(key(declared)),
                ..Table::default()
            };
            let name = sources
                .table_style_of(&probe)
                .and_then(|s| design.sheet()?.get(&s).cloned())
                .map(|s| s.name.clone().unwrap_or_else(|| s.style_id.clone()));
            let count = element.find(Some(W), "tblGrid").map_or(0, |g| {
                g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
            });
            let token = format!("table {}, {count} columns", name.as_deref().unwrap_or("no style"));
            out.push((None, Some(token), id));
        } else {
            out.push((None, Some("control".into()), id));
        }
    }
    out
}

/// Block shapes repeated under two or more headings of one level, most common first.
pub(crate) fn patterns(design: &Design<'_>) -> Vec<Value> {
    let blocks = tokens(design);
    let mut shapes: Shapes = Vec::new();
    let mut headings: Vec<(i64, usize)> = Vec::new();
    for (k, (level, _, id)) in blocks.iter().enumerate() {
        let Some(level) = *level else { continue };
        match headings.iter_mut().find(|(l, _)| *l == level) {
            Some((_, n)) => *n += 1,
            None => headings.push((level, 1)),
        }
        let mut shape: Vec<String> = Vec::new();
        let mut last: Option<String> = None;
        for (next_level, token, next_id) in blocks.iter().skip(k + 1) {
            if next_level.is_some() {
                break;
            }
            let Some(token) = token else { continue };
            if shape.last() != Some(token) {
                shape.push(token.clone());
            }
            last.clone_from(next_id);
        }
        let wanted = shape.len() >= 2 || shape.iter().any(|t| t.starts_with("table "));
        if shape.is_empty() || !wanted {
            continue;
        }
        let heading = id.clone().unwrap_or_else(|| "None".into());
        let last = last.unwrap_or_else(|| "None".into());
        let key = (level, shape);
        match shapes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, found)) => found.push((heading, last)),
            None => shapes.push((key, vec![(heading, last)])),
        }
    }
    let mut out: Vec<(usize, i64, Vec<String>, Value)> = Vec::new();
    for ((level, shape), found) in shapes {
        if found.len() < 2 {
            continue;
        }
        let of = headings.iter().find(|(l, _)| *l == level).map_or(0, |(_, n)| *n);
        let examples: Vec<Value> = found
            .iter()
            .take(3)
            .map(|(h, last)| {
                let mut e = Map::new();
                e.insert("heading".into(), Value::from(h.as_str()));
                e.insert("blocks".into(), Value::from(format!("{h}..{last}")));
                Value::Object(e)
            })
            .collect();
        let mut entry = Map::new();
        entry.insert("heading_level".into(), Value::from(level));
        entry.insert("shape".into(), Value::from(shape.clone()));
        entry.insert("count".into(), Value::from(found.len()));
        entry.insert("of".into(), Value::from(of));
        entry.insert("examples".into(), Value::Array(examples));
        out.push((found.len(), level, shape, Value::Object(entry)));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));
    out.into_iter().map(|(_, _, _, v)| v).collect()
}
