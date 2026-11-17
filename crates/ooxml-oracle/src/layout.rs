//! Owns: the Word layout's differential -- each document laid out by the Rust layout over
//! the fonts docx2svg found, compared with docx2svg's layout records
//! (`python/docx2svg/tools/layout_differential.py`) box by box: the flow's paragraphs
//! (lines broken, each line's height, spacing, borders, keeps), the pages' starts and
//! numbers, and every body line's place (top and pitch). What L3a does not lay out yet
//! (tables, stand-in boxes, obstacles) cuts the comparison off at the first such item.
//!
//! Must not: change what is compared to make it pass.

use crate::export::{Unlimited, KEY};
use crate::text::fonts_of;
use ooxml_base::Locale;
use ooxml_derive::CacheBudget;
use ooxml_word_layout::flow::{FlowItem, ParaItem};
use ooxml_word_layout::paginate::Position;
use ooxml_word_layout::table::NoTables;
use ooxml_word_layout::{DocumentLayout, WordGlyphs, WordLayout};
use ooxml_word_model::model::Block;
use ooxml_word_model::WordDocument;
use serde_json::Value;
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
    /// Lines compared (breaks and heights).
    pub lines: usize,
    /// Pages compared.
    pub pages: usize,
    /// Placed lines compared.
    pub placed: usize,
    /// Page-number fields compared.
    pub fields: usize,
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
fn para_fields(p: &ParaItem) -> Vec<(&'static str, Value)> {
    let m = &p.measured;
    let heights: Vec<Value> = m
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
        ("page_breaks", serde_json::json!(m.page_breaks)),
        ("column_breaks", serde_json::json!(m.column_breaks)),
        ("undrawn", serde_json::json!(m.undrawn)),
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
/// paragraph it laid out (a table, stand-in box, obstacle, or a paragraph laid out by an
/// approximate rule, beside notes, drawings or a drop cap), and why.
fn cutoff(flow: &[Value]) -> (usize, Option<String>) {
    for (k, item) in flow.iter().enumerate() {
        let kind = item.get("kind").and_then(Value::as_str).unwrap_or_default();
        if kind != "para" {
            return (k, Some(kind.to_owned()));
        }
        let approximated = item
            .get("approximations")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        let flagged = ["notes", "drop_cap"]
            .iter()
            .any(|f| item.get(*f).and_then(Value::as_bool).unwrap_or(false))
            || int(item.get("anchors")) > 0;
        if approximated || flagged {
            return (k, Some("notes, drawings or an approximation".into()));
        }
    }
    (flow.len(), None)
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
        let p = match mine {
            FlowItem::Para(p) => p,
            FlowItem::Stop(s) => {
                diffs.push(place, "stopped in Rust only", s.reason.clone());
                return;
            }
            FlowItem::Table(_) => {
                diffs.push(place, "kind", "table vs para".into());
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
        for (name, value) in para_fields(p) {
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

/// Top-level paragraph paths by flow item.
fn item_paths(document: &WordDocument, layout: &DocumentLayout) -> Vec<Option<String>> {
    layout
        .flow
        .items
        .iter()
        .map(|item| match item {
            FlowItem::Para(p) => match document.body.get(p.body) {
                Some(Block::Paragraph(para)) => Some(para.path.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Whether a line draws nothing (the layout places it and draws no line for it).
fn undrawn(ours: &DocumentLayout, item: usize, line: usize) -> bool {
    ours.flow
        .items
        .get(item)
        .and_then(FlowItem::para)
        .is_some_and(|p| p.measured.undrawn.contains(&line))
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

/// A placed line as compared: its paragraph's path, its number, its top and pitch.
type LineBox = (String, usize, String, String);

/// The page's drawn lines of items before the cut.
fn our_lines(
    ours: &DocumentLayout,
    page: &ooxml_word_layout::paginate::LaidPage,
    paths: &[Option<String>],
    cut: usize,
) -> Vec<LineBox> {
    page.lines
        .iter()
        .filter(|l| l.item < cut && !undrawn(ours, l.item, l.line))
        .map(|l| {
            let path = paths.get(l.item).cloned().flatten().unwrap_or_default();
            (path, l.line, l.top.to_string(), l.pitch.to_string())
        })
        .collect()
}

/// docx2svg's drawn body lines of a page: a line of a paragraph before the cut; on a page
/// wholly before it, every line (one of a paragraph the Rust flow does not know is a
/// difference).
fn their_lines(
    drawn: &Value,
    item_of: &std::collections::HashMap<&str, usize>,
    cut: usize,
    whole_page: bool,
) -> Vec<LineBox> {
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
            )
        })
        .collect()
}

fn compare_pages(
    document: &WordDocument,
    ours: &DocumentLayout,
    theirs: &Value,
    cut: (usize, bool),
    diffs: &mut Diffs<'_>,
    outcome: &mut Outcome,
) {
    // A note, a drawing or an approximation at the cut can move any line of its page
    // (a note area, a band above its anchor): that page's lines are not compared.
    let (cut, whole_page_only) = cut;
    let starts: Vec<Position> = at(theirs, "starts")
        .as_array()
        .into_iter()
        .flatten()
        .map(position)
        .collect();
    let pages = &ours.pagination.pages;
    let paths = item_paths(document, ours);
    let item_of: std::collections::HashMap<&str, usize> = paths
        .iter()
        .enumerate()
        .filter_map(|(k, p)| Some((p.as_deref()?, k)))
        .collect();
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
        if whole_page_only && !whole_page {
            continue;
        }
        let mine = our_lines(ours, page, &paths, cut);
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
        Arc::new(NoTables),
        Locale::default(),
        &budget,
        KEY,
    );
    let started = Instant::now();
    let laid = engine.lay_out(&mut model, &document);
    outcome.layout_time += started.elapsed();
    let flow = at(theirs, "flow").as_array().cloned().unwrap_or_default();
    let (cut, why) = cutoff(&flow);
    match &why {
        Some(why) => outcome.cut.push((records.path.clone(), format!("item {cut}: {why}"))),
        None => outcome.whole += 1,
    }
    compare_fields(&laid, theirs, cut, &mut diffs, outcome);
    compare_flow(&laid, &flow, cut, &mut diffs, outcome);
    let page_level = why.as_deref().is_some_and(|w| w.starts_with("notes"));
    compare_pages(&document, &laid, theirs, (cut, page_level), &mut diffs, outcome);
    outcome.differences.extend(found);
    Ok(())
}
