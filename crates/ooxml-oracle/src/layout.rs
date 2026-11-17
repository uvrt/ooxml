//! Owns: the Word layout's differential -- each document laid out by the Rust layout over
//! the fonts docx2svg found, compared with docx2svg's layout records
//! (`python/docx2svg/tools/layout_differential.py`) box by box: the flow's paragraphs
//! (lines broken, each line's height, spacing, borders, keeps), the pages' starts and
//! numbers, every body line's place (top and pitch), table cells' lines included, and the
//! footnotes at each page's foot (the lines carried from the page before, each note's lines
//! on it); each table as measured (`layout_tables`). What the layout does not lay out
//! (stand-in boxes, obstacles, floating tables, notes and floating drawings in a table's
//! cells) cuts the comparison off at the first such item.
//!
//! Must not: change what is compared to make it pass.

use crate::export::{Unlimited, KEY};
use crate::layout_tables::{first_difference, measured, normalised, table_cut, table_record};
use crate::text::fonts_of;
use ooxml_base::Locale;
use ooxml_derive::CacheBudget;
use ooxml_word_layout::facts::{LayoutFacts, PageFacts};
use ooxml_word_layout::flow::{FlowItem, ParaItem};
use ooxml_word_layout::paginate::Position;
use ooxml_word_layout::paragraph::MeasuredParagraph;
use ooxml_word_layout::tables::WordTables;
use ooxml_word_layout::{DocumentLayout, WordGlyphs, WordLayout};
use ooxml_word_model::model::Block;
use ooxml_word_model::WordDocument;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One document's records.
#[derive(Debug, Default)]
pub struct LayoutRecords {
    /// The document's path.
    pub path: String,
    /// Its fonts record.
    pub fonts: Option<Value>,
    /// docx2svg's layout.
    pub layout: Option<Value>,
    /// Why docx2svg refused it.
    pub refused: Option<String>,
    /// Every page's SVG as docx2svg wrote it, where recorded.
    pub svg: Option<Vec<String>>,
}

/// A difference.
#[derive(Debug, Clone)]
pub struct Difference {
    /// The document.
    pub document: String,
    /// Where: a flow item or a page.
    pub at: String,
    /// What differs.
    pub cause: String,
    /// The detail.
    pub detail: String,
}

/// The differential's result.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Documents read.
    pub documents: usize,
    /// Documents compared to their end (nothing cut off).
    pub whole: usize,
    /// Documents cut off at a table, stand-in box or obstacle, and why.
    pub cut: Vec<(String, String)>,
    /// Flow paragraphs compared.
    pub paragraphs: usize,
    /// Tables compared.
    pub tables: usize,
    /// Lines compared (breaks and heights).
    pub lines: usize,
    /// Pages compared.
    pub pages: usize,
    /// Placed lines compared.
    pub placed: usize,
    /// Page-number fields compared.
    pub fields: usize,
    /// Field update results compared (fields and every paragraph's page reference).
    pub updates: usize,
    /// Footnote lines compared.
    pub note_lines: usize,
    /// Every difference.
    pub differences: Vec<Difference>,
    /// The Rust layout's own time.
    pub layout_time: Duration,
}

/// Read a records stream, handing each document to `each`.
pub fn read_layout_records(
    text: impl std::io::BufRead,
    mut each: impl FnMut(LayoutRecords) -> Result<(), String>,
) -> Result<(), String> {
    let mut current: Option<LayoutRecords> = None;
    for line in text.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(document) = value.get("document") {
            if let Some(done) = current.take() {
                each(done)?;
            }
            current = Some(LayoutRecords {
                path: document
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                ..LayoutRecords::default()
            });
            continue;
        }
        let Some(doc) = current.as_mut() else { continue };
        if let Some(f) = value.get("fonts") {
            doc.fonts = Some(f.clone());
        } else if let Some(l) = value.get("layout") {
            doc.layout = Some(l.clone());
        } else if let Some(r) = value.get("refused").and_then(Value::as_str) {
            doc.refused = Some(r.to_owned());
        } else if let Some(pages) = value.get("svg").and_then(Value::as_array) {
            doc.svg = Some(pages.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect());
        }
    }
    if let Some(done) = current.take() {
        each(done)?;
    }
    Ok(())
}

struct Diffs<'a> {
    document: &'a str,
    out: &'a mut Vec<Difference>,
}

impl Diffs<'_> {
    fn push(&mut self, at: String, cause: &str, detail: String) {
        self.out.push(Difference {
            document: self.document.to_owned(),
            at,
            cause: cause.to_owned(),
            detail,
        });
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `v[key]`, `Null` where there is none.
fn at<I: serde_json::value::Index>(v: &Value, key: I) -> &Value {
    v.get(key).unwrap_or(&Value::Null)
}

fn int(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

/// The paragraph's lines and heights as docx2svg's record writes them.
fn para_fields(p: &ParaItem, last: &MeasuredParagraph) -> Vec<(&'static str, Value)> {
    let m = &p.measured;
    let heights: Vec<Value> = last
        .heights
        .iter()
        .map(|h| {
            serde_json::json!([
                h.pitch.to_string(),
                h.text_above.to_string(),
                h.text_below.to_string(),
                h.natural.to_string(),
                h.rule.name(),
                h.line
            ])
        })
        .collect();
    let lines: Vec<Value> = m
        .measured
        .lines
        .iter()
        .map(|l| serde_json::json!([l.start, l.end, l.forced]))
        .collect();
    vec![
        ("section", serde_json::json!(p.section)),
        ("lines", Value::Array(lines)),
        ("heights", Value::Array(heights)),
        ("page_breaks", serde_json::json!(last.page_breaks)),
        ("column_breaks", serde_json::json!(last.column_breaks)),
        ("undrawn", serde_json::json!(last.undrawn)),
        ("before", serde_json::json!(p.before)),
        ("after", serde_json::json!(p.after)),
        ("contextual", serde_json::json!(p.contextual)),
        ("style", serde_json::json!(p.style.as_deref())),
        ("border", serde_json::json!(p.border.to_string())),
        ("top_border", serde_json::json!(p.top_border.to_string())),
        ("keep_next", serde_json::json!(p.keep_next)),
        ("keep_lines", serde_json::json!(p.keep_lines)),
        ("page_break_before", serde_json::json!(p.page_break_before)),
        ("widow", serde_json::json!(p.widow)),
        ("section_start", serde_json::json!(p.section_start)),
        ("section_mark", serde_json::json!(p.section_mark)),
    ]
}

/// The first flow item the comparison stops before: docx2svg's first that is not a
/// paragraph or table it laid out (a stand-in box, an obstacle, a table the layout does
/// not lay out yet: [`table_cut`]; a table with floating drawings in its cells only where
/// the Rust layout stops at it, `ours` being its items), and why. A paragraph docx2svg lays
/// out by a rule it did not measure (its approximations) is compared too: the Rust layout
/// lays it out by the same rule.
fn cutoff(flow: &[Value], ours: &[FlowItem]) -> (usize, Option<String>) {
    for (k, item) in flow.iter().enumerate() {
        let kind = item.get("kind").and_then(Value::as_str).unwrap_or_default();
        if kind == "table" {
            match item.get("table").and_then(table_cut) {
                Some(why)
                    if why.ends_with("a floating drawing in a cell")
                        && !matches!(ours.get(k), Some(FlowItem::Stop(_))) =>
                {
                    continue
                }
                Some(why) => return (k, Some(why.to_owned())),
                None => continue,
            }
        }
        if kind != "para" {
            return (k, Some(kind.to_owned()));
        }
    }
    (flow.len(), None)
}

/// Where the body is cut anyway, the first item the comparison must stop before: a
/// paragraph holding a page-number field (its value may be a page past the cut).
fn model_cut(document: &WordDocument, ours: &DocumentLayout, cut: bool) -> Option<(usize, &'static str)> {
    if !cut {
        return None;
    }
    ours.flow.items.iter().enumerate().find_map(|(k, item)| {
        let FlowItem::Para(p) = item else { return None };
        let Some(Block::Paragraph(para)) = document.body.get(p.body) else {
            return None;
        };
        let fields = para.runs.iter().any(|r| {
            r.field
                .as_ref()
                .is_some_and(|f| ooxml_word_layout::fields::PAGE_FIELDS.contains(&&*f.keyword))
        });
        fields.then_some((k, "a page-number field in a body cut further on"))
    })
}

/// The footnotes at each page's foot, as docx2svg's records write them: per page, how
/// many lines of a note the page before could not hold and that note, and of each note
/// its id, how many of its lines are on the page and how many it has.
fn compare_notes(ours: &DocumentLayout, theirs: &Value, cut: usize, diffs: &mut Diffs<'_>) {
    let Some(expected) = theirs.get("notes") else { return };
    let flow_len = at(theirs, "flow").as_array().map_or(0, Vec::len);
    if cut < flow_len {
        return;
    }
    let mine: Vec<Value> = ours
        .pagination
        .notes()
        .into_iter()
        .map(|(page, (carried, placed))| {
            let notes: Vec<Value> = placed
                .iter()
                .map(|n| json!([n.lines.first().and_then(|l| l.note), n.taken, n.lines.len()]))
                .collect();
            json!([page, carried.len(), carried.first().and_then(|l| l.note), notes])
        })
        .collect();
    let expected = expected.as_array().cloned().unwrap_or_default();
    if mine != expected {
        let first = mine.iter().zip(&expected).position(|(a, b)| a != b);
        let detail = match first {
            Some(j) => format!(
                "{} vs {}",
                mine.get(j).map(text).unwrap_or_default(),
                expected.get(j).map(text).unwrap_or_default()
            ),
            None => format!("{} pages vs {}", mine.len(), expected.len()),
        };
        diffs.push("notes".into(), "page notes", detail);
    }
}

/// One table compared as measured; `false` where it differs (the flow after it then
/// differs too, and is not compared).
fn compare_table(mine: &FlowItem, item: &Value, place: &str, diffs: &mut Diffs<'_>) -> bool {
    let Some(table) = measured(mine) else {
        diffs.push(place.to_owned(), "table", "not laid out by the Rust tables".into());
        return false;
    };
    let FlowItem::Table(t) = mine else { return false };
    let facts = [
        ("block", json!(t.block), item.get("block").cloned()),
        ("section", json!(t.section), item.get("section").cloned()),
        (
            "section_start",
            json!(t.section_start),
            item.get("section_start").cloned(),
        ),
        ("rows", json!(t.flow.rows()), item.get("rows").cloned()),
    ];
    for (name, ours, theirs) in facts {
        if Some(&ours) != theirs.as_ref() {
            diffs.push(place.to_owned(), name, format!("{ours} vs {theirs:?}"));
            return false;
        }
    }
    let first = int(item.get("block"));
    let theirs = normalised(at(item, "table"), first);
    match first_difference("table", &table_record(table, 0), &theirs) {
        Some(found) => {
            diffs.push(place.to_owned(), "table", found);
            false
        }
        None => true,
    }
}

fn compare_flow(ours: &DocumentLayout, theirs: &[Value], cut: usize, diffs: &mut Diffs<'_>, outcome: &mut Outcome) {
    if cut == theirs.len() && ours.flow.items.len() != theirs.len() {
        diffs.push(
            "flow".into(),
            "item count",
            format!("{} vs {}", ours.flow.items.len(), theirs.len()),
        );
    }
    for (k, (mine, item)) in ours.flow.items.iter().zip(theirs).enumerate().take(cut) {
        let place = format!("item {k} (block {})", int(item.get("block")));
        let theirs_kind = item.get("kind").and_then(Value::as_str).unwrap_or_default();
        let p = match mine {
            FlowItem::Para(p) if theirs_kind == "para" => p,
            FlowItem::Table(_) if theirs_kind == "table" => {
                if !compare_table(mine, item, &place, diffs) {
                    return;
                }
                outcome.tables += 1;
                continue;
            }
            FlowItem::Stop(s) => {
                diffs.push(place, "stopped in Rust only", s.reason.clone());
                return;
            }
            FlowItem::Para(_) | FlowItem::Table(_) => {
                let ours = if matches!(mine, FlowItem::Table(_)) {
                    "table"
                } else {
                    "para"
                };
                diffs.push(place, "kind", format!("{ours} vs {theirs_kind}"));
                return;
            }
        };
        outcome.paragraphs += 1;
        outcome.lines += p.count();
        if i64::try_from(p.block).ok() != item.get("block").and_then(Value::as_i64) {
            diffs.push(
                place.clone(),
                "block",
                format!("{} vs {}", p.block, text(at(item, "block"))),
            );
        }
        let last = last_lines(ours, k).unwrap_or(&p.measured);
        for (name, value) in para_fields(p, last) {
            let expected = item.get(name).cloned().unwrap_or(Value::Null);
            if value != expected {
                diffs.push(place.clone(), name, format!("{value} vs {expected}"));
                break;
            }
        }
    }
}

/// The body's page-number fields' texts, where the whole body is laid out by both.
fn compare_fields(ours: &DocumentLayout, theirs: &Value, cut: usize, diffs: &mut Diffs<'_>, outcome: &mut Outcome) {
    let flow_len = at(theirs, "flow").as_array().map_or(0, Vec::len);
    let expected: Vec<&Value> = at(theirs, "fields").as_array().into_iter().flatten().collect();
    if cut < flow_len || expected.is_empty() {
        return;
    }
    let (paragraphs, _) = ooxml_word_layout::flow::body_paragraphs(&ours.document.body);
    for field in expected {
        let (block, k) = (
            usize::try_from(int(field.get(0))).unwrap_or(0),
            usize::try_from(int(field.get(1))).unwrap_or(0),
        );
        let mine = paragraphs
            .get(block)
            .and_then(|p| p.runs.get(k))
            .map(|r| r.text.to_string());
        outcome.fields += 1;
        if mine.as_deref() != field.get(2).and_then(Value::as_str) {
            diffs.push(
                format!("field {block}/{k}"),
                "field text",
                format!("{mine:?} vs {}", text(at(field, 2))),
            );
        }
    }
    if !ours.settled {
        diffs.push(
            "fields".into(),
            "fields unsettled in Rust",
            format!("{} passes", ours.passes),
        );
    }
}

/// Every body paragraph's path (cells included) and the flow item it is drawn by.
fn item_paths(document: &WordDocument, layout: &DocumentLayout) -> HashMap<String, usize> {
    let (paragraphs, _) = ooxml_word_layout::flow::body_paragraphs(&document.body);
    let mut out = HashMap::new();
    for (k, item) in layout.flow.items.iter().enumerate() {
        let blocks = match item {
            FlowItem::Para(p) => p.block..p.block + 1,
            FlowItem::Table(t) => match document.body.get(t.body) {
                Some(Block::Table(table)) => t.block..t.block + ooxml_word_layout::tables::table_paragraphs(table),
                _ => continue,
            },
            FlowItem::Stop(_) => continue,
        };
        for block in blocks {
            if let Some(p) = paragraphs.get(block) {
                out.insert(p.path.to_string(), k);
            }
        }
    }
    out
}

/// A paragraph's lines as the last page that broke them again left them (docx2svg
/// records a paragraph as its pagination left it); `None` where none did.
fn last_lines(ours: &DocumentLayout, item: usize) -> Option<&MeasuredParagraph> {
    ours.pagination
        .pages
        .iter()
        .rev()
        .find_map(|page| page.broken.get(&item))
        .map(|again| &*again.measured)
}

fn position(v: &Value) -> Position {
    Position {
        item: usize::try_from(int(v.get(0))).unwrap_or(0),
        line: usize::try_from(int(v.get(1))).unwrap_or(0),
        part: v
            .get(2)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_u64().and_then(|n| u32::try_from(n).ok()))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A placed line as compared: its paragraph's path, its number, its top, pitch, baseline
/// and text column.
type LineBox = (String, usize, String, String, i64, Option<usize>);

/// The facts' lines of a page drawn for items before the cut.
fn our_lines(facts: &PageFacts, cut: usize) -> Vec<LineBox> {
    facts
        .lines
        .iter()
        .filter(|l| l.source.item() < cut)
        .map(|l| {
            (
                l.path.to_string(),
                l.number,
                l.top.to_string(),
                l.pitch.to_string(),
                l.baseline,
                l.column,
            )
        })
        .collect()
}

/// docx2svg's drawn body lines of a page: a line of a paragraph before the cut; on a page
/// wholly before it, every line (one of a paragraph the Rust flow does not know is a
/// difference).
fn their_lines(drawn: &Value, item_of: &HashMap<&str, usize>, cut: usize, whole_page: bool) -> Vec<LineBox> {
    at(drawn, "lines")
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| match item_of.get(at(l, 0).as_str().unwrap_or_default()) {
            Some(item) => *item < cut,
            None => whole_page,
        })
        .map(|l| {
            (
                text(at(l, 0)),
                usize::try_from(int(l.get(1))).unwrap_or(0),
                text(at(l, 2)),
                text(at(l, 3)),
                int(l.get(4)),
                l.get(5).and_then(Value::as_u64).and_then(|c| usize::try_from(c).ok()),
            )
        })
        .collect()
}

fn compare_pages(
    document: &WordDocument,
    ours: &DocumentLayout,
    facts: &LayoutFacts,
    theirs: &Value,
    cut: usize,
    diffs: &mut Diffs<'_>,
    outcome: &mut Outcome,
) {
    let starts: Vec<Position> = at(theirs, "starts")
        .as_array()
        .into_iter()
        .flatten()
        .map(position)
        .collect();
    let pages = &ours.pagination.pages;
    let owned = item_paths(document, ours);
    let item_of: HashMap<&str, usize> = owned.iter().map(|(p, k)| (p.as_str(), *k)).collect();
    let flow_len = at(theirs, "flow").as_array().map_or(0, Vec::len);
    let laid_pages: Vec<&Value> = at(theirs, "pages")
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| !at(at(p, "info"), 4).as_bool().unwrap_or(false))
        .collect();
    for (k, start) in starts.iter().enumerate() {
        if start.item >= cut {
            break;
        }
        let place = format!("page {k}");
        let Some(page) = pages.get(k) else {
            diffs.push(place, "page count", format!("{} vs {}", pages.len(), starts.len()));
            return;
        };
        outcome.pages += 1;
        if page.start != *start {
            diffs.push(place, "page start", format!("{:?} vs {start:?}", page.start));
            return;
        }
        let info = ours.pagination.info.get(page.page);
        let theirs_info = at(theirs, "info").as_array().and_then(|all| {
            all.iter()
                .find(|i| i.get(0).and_then(Value::as_u64) == u64::try_from(page.page).ok())
        });
        let mine =
            info.map(|i| serde_json::json!([page.page, i.number, i.section, i.section_first, i.story.name(), i.blank]));
        if mine.as_ref() != theirs_info {
            diffs.push(place.clone(), "page info", format!("{mine:?} vs {theirs_info:?}"));
        }
        let Some(drawn) = laid_pages.get(k) else { continue };
        let whole_page = starts.get(k + 1).map_or(cut == flow_len, |next| next.item < cut);
        let Some(page_facts) = facts.pages.get(page.page) else {
            continue;
        };
        compare_frame(page_facts, drawn, &place, diffs);
        if whole_page {
            compare_blocks(page_facts, drawn, &place, diffs);
        }
        let mine = our_lines(page_facts, cut);
        let theirs_lines = their_lines(drawn, &item_of, cut, whole_page);
        outcome.placed += mine.len();
        if mine != theirs_lines {
            let first = mine.iter().zip(&theirs_lines).position(|(a, b)| a != b);
            let detail = match first {
                Some(j) => format!("line {j}: {:?} vs {:?}", mine.get(j), theirs_lines.get(j)),
                None => format!("{} lines vs {}", mine.len(), theirs_lines.len()),
            };
            diffs.push(place, "placed lines", detail);
        }
    }
}

/// A page's size and text area against docx2svg's (`frame`: width and height in px, the
/// column's left and right edges, the text area's top and bottom), where recorded.
fn compare_frame(facts: &PageFacts, drawn: &Value, place: &str, diffs: &mut Diffs<'_>) {
    let Some(theirs) = drawn.get("frame") else { return };
    let f = &facts.frame;
    let mine = json!([
        f.width_px,
        f.height_px,
        f.left.to_string(),
        f.right.to_string(),
        f.top.to_string(),
        f.bottom.to_string()
    ]);
    if &mine != theirs {
        diffs.push(place.to_owned(), "page frame", format!("{mine} vs {theirs}"));
    }
}

/// The body blocks a page draws something of against docx2svg's `Page.blocks`, where
/// recorded.
fn compare_blocks(facts: &PageFacts, drawn: &Value, place: &str, diffs: &mut Diffs<'_>) {
    let Some(theirs) = drawn.get("blocks") else { return };
    let mine: Vec<&str> = facts.blocks.iter().map(|b| &**b).collect();
    if json!(mine) != *theirs {
        diffs.push(place.to_owned(), "page blocks", format!("{mine:?} vs {theirs}"));
    }
}

/// The footnotes' lines at each page's foot (their paragraph's path, number, top, pitch,
/// baseline, column) against docx2svg's `note_lines`; a page whose notes the Rust paint
/// does not draw is skipped and counted as a difference of its own.
fn compare_note_lines(
    engine: &mut WordLayout,
    model: &mut ooxml_word_model::Model,
    laid: &DocumentLayout,
    theirs: &Value,
    diffs: &mut Diffs<'_>,
    outcome: &mut Outcome,
) {
    let pages = at(theirs, "pages").as_array().cloned().unwrap_or_default();
    for (k, page) in pages.iter().enumerate() {
        let expected: Vec<LineBox> = at(page, "note_lines")
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| {
                (
                    text(at(l, 0)),
                    usize::try_from(int(l.get(1))).unwrap_or(0),
                    text(at(l, 2)),
                    text(at(l, 3)),
                    int(l.get(4)),
                    l.get(5).and_then(Value::as_u64).and_then(|c| usize::try_from(c).ok()),
                )
            })
            .collect();
        let parts = engine.page_parts(model, laid, k);
        if let Some(why) = parts.unpainted.first() {
            if !expected.is_empty() {
                diffs.push(format!("page {k}"), "note lines not drawn", why.clone());
            }
            continue;
        }
        let mine: Vec<LineBox> = parts
            .footnotes
            .iter()
            .flat_map(|notes| {
                let (paragraphs, _) = ooxml_word_layout::flow::body_paragraphs(&notes.blocks);
                notes
                    .walked
                    .lines
                    .iter()
                    .map(move |l| {
                        let path = l.block.and_then(|b| paragraphs.get(b)).map(|p| p.path.to_string());
                        (
                            path.unwrap_or_default(),
                            l.number,
                            l.top.to_string(),
                            l.pitch.to_string(),
                            l.baseline,
                            l.column,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        outcome.note_lines += mine.len();
        if mine != expected {
            let first = mine.iter().zip(&expected).position(|(a, b)| a != b);
            let detail = match first {
                Some(j) => format!("line {j}: {:?} vs {:?}", mine.get(j), expected.get(j)),
                None => format!("{} lines vs {}", mine.len(), expected.len()),
            };
            diffs.push(format!("page {k}"), "note lines", detail);
        }
    }
}

/// Lay one document out and compare it with its records.
pub fn run_document(records: &LayoutRecords, outcome: &mut Outcome) -> Result<(), String> {
    outcome.documents += 1;
    let mut found = Vec::new();
    let mut diffs = Diffs {
        document: &records.path,
        out: &mut found,
    };
    let Some(theirs) = &records.layout else {
        if let Some(why) = &records.refused {
            outcome
                .cut
                .push((records.path.clone(), format!("refused by docx2svg: {why}")));
        }
        return Ok(());
    };
    let bytes = std::fs::read(&records.path).map_err(|e| format!("{}: {e}", records.path))?;
    let fonts = match &records.fonts {
        Some(f) => fonts_of(f, Some(&bytes))?,
        None => {
            outcome.cut.push((
                records.path.clone(),
                "no fonts record: docx2svg measured nothing".into(),
            ));
            return Ok(());
        }
    };
    let (mut model, document) = crate::export::open(bytes)?;
    let budget: Arc<dyn CacheBudget> = Arc::new(Unlimited);
    let mut engine = WordLayout::new(
        Arc::new(WordGlyphs::new(fonts)),
        Arc::new(WordTables),
        Locale::default(),
        &budget,
        KEY,
    );
    let started = Instant::now();
    let laid = engine.lay_out(&mut model, &document);
    outcome.layout_time += started.elapsed();
    let flow = at(theirs, "flow").as_array().cloned().unwrap_or_default();
    let (cut, why) = cutoff(&flow, &laid.flow.items);
    let (cut, why) = match model_cut(&laid.document, &laid, cut < flow.len()) {
        Some((k, reason)) if k < cut => (k, Some(reason.to_owned())),
        _ => (cut, why),
    };
    match &why {
        Some(why) => outcome.cut.push((records.path.clone(), format!("item {cut}: {why}"))),
        None => outcome.whole += 1,
    }
    compare_fields(&laid, theirs, cut, &mut diffs, outcome);
    compare_notes(&laid, theirs, cut, &mut diffs);
    compare_flow(&laid, &flow, cut, &mut diffs, outcome);
    let facts = LayoutFacts::of(&laid);
    compare_pages(&laid.document, &laid, &facts, theirs, cut, &mut diffs, outcome);
    if why.is_none() {
        compare_note_lines(&mut engine, &mut model, &laid, theirs, &mut diffs, outcome);
        let updates = crate::layout_updates::compare_updates(&laid, &facts, theirs);
        outcome.updates += updates.compared;
        for (place, detail) in updates.differences {
            diffs.push(place, "field update", detail);
        }
    }
    outcome.differences.extend(found);
    Ok(())
}
