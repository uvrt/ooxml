//! Owns: what a version laid out says it does not lay out or draw faithfully -- docx2svg's
//! layout warnings (`Layout.warnings`) and approximations (`Layout.approximations`), each
//! with docx2svg's code, message and page, in the order docx2svg's placer records them:
//! the body's cross references, its page-number fields, the passes that did not settle,
//! the sections' vertical alignment and the fields Word computes again, then page by page
//! what its body draws (tables laid out by approximate rules or off their grid, rows past
//! the page's foot, distributed paragraphs, inline drawings with no picture, floating
//! drawings that may not overlap) and its header's and footer's fields.
//!
//! A warning is recorded once (`Layout.warn`'s rule: the same code, message and page); an
//! approximation is a warning too (`layout-approximate:<reason>`).
//!
//! Must not: lay out or paint, or say anything docx2svg does not.

use ooxml_word_layout::fields::update::PageNumbers;
use ooxml_word_layout::fields::{cross_reference_problems, field_text, page_counts, Uncomputable};
use ooxml_word_layout::flow::{body_paragraphs, FlowItem};
use ooxml_word_layout::stories::story_references;
use ooxml_word_layout::tables::MeasuredTable;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::model::{Block, InlineObject, Paragraph};
use ooxml_word_model::value::Value;
use ooxml_word_model::{Model, WordDocument};
use std::collections::{BTreeSet, HashMap, HashSet};

/// One warning: docx2svg's `(code, message, 1-based page or None)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutWarning {
    /// Its stable code (`table-grid-mismatch`, `field-not-computed:PAGEREF`...).
    pub code: String,
    /// What it says.
    pub message: String,
    /// The 1-based page it is of, where it is of one.
    pub page: Option<usize>,
}

/// One approximation: docx2svg's `(code, message, 1-based page, element path)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approximation {
    /// `layout-approximate:<reason>`.
    pub code: String,
    /// What it says.
    pub message: String,
    /// The 1-based page it is on.
    pub page: Option<usize>,
    /// The element it is of.
    pub path: Option<String>,
}

/// The warnings and approximations of a version, in docx2svg's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Said {
    /// Every warning, approximations' included.
    pub warnings: Vec<LayoutWarning>,
    /// The approximations.
    pub approximations: Vec<Approximation>,
}

impl Said {
    fn warn(&mut self, code: impl Into<String>, message: impl Into<String>, page: Option<usize>) {
        let entry = LayoutWarning {
            code: code.into(),
            message: message.into(),
            page,
        };
        if !self.warnings.contains(&entry) {
            self.warnings.push(entry);
        }
    }

    fn approximate(&mut self, reason: &str, message: &str, page: Option<usize>, path: Option<&str>) {
        let entry = Approximation {
            code: format!("layout-approximate:{reason}"),
            message: message.to_owned(),
            page,
            path: path.map(str::to_owned),
        };
        if !self.approximations.contains(&entry) {
            self.approximations.push(entry.clone());
        }
        self.warn(entry.code, entry.message, page);
    }
}

/// The fields Word computes again when it exports (`RECOMPUTED_BY_WORD`).
const RECOMPUTED_BY_WORD: &[&str] = &["DATE", "TIME", "REF", "PAGEREF", "SEQ", "IF"];
/// The fields whose text depends on the pages (`PAGE_FIELDS`).
const PAGE_FIELDS: &[&str] = &["PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF"];

/// What a page's drawings say as they are painted: its floating drawings' positions not
/// modelled, and what their painters could not draw faithfully, in the order placed.
pub trait PageDrawings {
    /// The warnings of page `page`'s drawings.
    fn said(&mut self, model: &mut Model, layout: &DocumentLayout, page: usize) -> Vec<LayoutWarning>;
}

/// Whether a paragraph holds a drawing a painter draws: a floating one, or an inline one
/// that is not a picture.
fn holds_drawing(paragraph: &Paragraph) -> bool {
    paragraph
        .runs
        .iter()
        .any(|r| !r.anchors.is_empty() || r.drawings.iter().any(|d| matches!(d, InlineObject::Graphic(_))))
}

/// Whether blocks (cells' included) hold a drawing a painter draws.
fn blocks_hold_drawing(blocks: &[Block]) -> bool {
    blocks.iter().any(|b| match b {
        Block::Paragraph(p) => holds_drawing(p),
        Block::Table(t) => t
            .rows
            .iter()
            .any(|row| row.iter().any(|cell| blocks_hold_drawing(cell))),
    })
}

/// What `layout` says, its paragraphs and tables resolved in `model`; `drawings` paints
/// the pages that hold drawings for what their painters say.
pub fn said(model: &mut Model, layout: &DocumentLayout, drawings: &mut dyn PageDrawings) -> Said {
    let mut out = Said::default();
    let document = &layout.document;
    for (keyword, why) in cross_reference_problems(document) {
        out.warn(
            format!("field-not-computed:{keyword}"),
            format!("a {keyword} field in the body is drawn as Word cached it ({why})"),
            None,
        );
    }
    body_fields(layout, &mut out);
    if !layout.settled {
        out.warn(
            "field-layout-unsettled",
            "the body's page-number fields did not settle on one layout in four passes; the last is drawn",
            None,
        );
    }
    document_notes(document, &mut out);
    let mut pages = PageWalk {
        tables: HashSet::new(),
        paragraphs: body_paragraphs(&document.body).0,
        drawn: false,
    };
    // A header's, footer's or note's drawing may be on any page.
    let everywhere = document.stories.values().any(|b| blocks_hold_drawing(b))
        || document
            .footnotes
            .notes
            .values()
            .chain(document.endnotes.notes.values())
            .any(|b| blocks_hold_drawing(b));
    for page in 0..layout.page_count() {
        pages.drawn = everywhere;
        pages.body(model, layout, page, &mut out);
        if pages.drawn {
            for w in drawings.said(model, layout, page) {
                out.warn(w.code, w.message, w.page);
            }
        }
        pages.stories(layout, page, &mut out);
    }
    out
}

/// The body's page-number fields that cannot be computed (`_computed_fields`).
fn body_fields(layout: &DocumentLayout, out: &mut Said) {
    let (paragraphs, _) = body_paragraphs(&layout.document.body);
    let has = paragraphs.iter().any(|p| {
        p.runs
            .iter()
            .any(|r| r.field.as_ref().is_some_and(|f| PAGE_FIELDS.contains(&&*f.keyword)))
    });
    if !has {
        return;
    }
    let facts = ooxml_word_layout::facts::LayoutFacts::of(layout);
    let numbers = PageNumbers::new(layout, &facts);
    let marks = bookmarks(&paragraphs);
    for paragraph in &paragraphs {
        for run in paragraph.runs.iter() {
            let Some(field) = run.field.as_ref().filter(|f| PAGE_FIELDS.contains(&&*f.keyword)) else {
                continue;
            };
            if &*field.keyword == "PAGEREF" {
                let target = ooxml_word_layout::fields::argument(&field.instruction)
                    .and_then(|name| marks.get(name.as_str()))
                    .and_then(|block| paragraphs.get(*block))
                    .filter(|p| facts.first_page(&p.path).is_some());
                let Some(target) = target else {
                    out.warn(
                        "field-not-computed:PAGEREF",
                        "a PAGEREF field in the body is drawn as Word cached it (its bookmark is not in the body, or not laid out)",
                        None,
                    );
                    continue;
                };
                if let Err(why) = numbers.page_reference(&field.instruction, &target.path) {
                    out.warn(
                        "field-not-computed:PAGEREF",
                        format!("a PAGEREF field in the body is drawn as Word cached it ({})", why.0),
                        None,
                    );
                }
                continue;
            }
            if facts.first_page(&paragraph.path).is_none() {
                continue;
            }
            if let Err(why) = numbers.page_field(&field.instruction, &field.keyword, &paragraph.path) {
                out.warn(
                    format!("field-not-computed:{}", field.keyword),
                    format!(
                        "a {} field in the body is drawn as Word cached it ({})",
                        field.keyword, why.0
                    ),
                    None,
                );
            }
        }
    }
}

/// Bookmark name -> the body paragraph its start is in, the first of a name winning.
fn bookmarks(paragraphs: &[std::sync::Arc<Paragraph>]) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for (block, paragraph) in paragraphs.iter().enumerate() {
        for mark in paragraph.bookmarks.iter() {
            if let (true, Some(name)) = (mark.start, mark.name.as_deref().filter(|n| !n.is_empty())) {
                out.entry(name.to_owned()).or_insert(block);
            }
        }
    }
    out
}

/// The sections' vertical alignment, and the fields Word computes again (`_notes`).
fn document_notes(document: &WordDocument, out: &mut Said) {
    for section in document.sections.iter() {
        if let Some(v) = section
            .properties
            .vertical_alignment
            .as_deref()
            .filter(|v| matches!(*v, "center" | "both" | "bottom"))
        {
            out.warn(
                "vertical-alignment-not-drawn",
                format!(
                    "a section's body is aligned {v} between its margins (w:vAlign), which is not laid out: its body \
                     is drawn from the top margin; its header and footer, which it does not move, are where Word \
                     draws them"
                ),
                None,
            );
        }
    }
    let cached: BTreeSet<&str> = document.cached_fields.iter().map(|k| &**k).collect();
    for keyword in cached.into_iter().filter(|k| RECOMPUTED_BY_WORD.contains(k)) {
        out.warn(
            format!("field-cached:{keyword}"),
            format!(
                "a {keyword} field is drawn as Word cached it in the file; Word computes it again when it exports, \
                 so the two can differ"
            ),
            None,
        );
    }
}

/// A walk of the pages: the body's paragraphs (cells included), and the tables said of
/// already.
struct PageWalk {
    tables: HashSet<usize>,
    paragraphs: Vec<std::sync::Arc<Paragraph>>,
    /// The page holds a drawing a painter draws.
    drawn: bool,
}

impl PageWalk {
    /// What page `page`'s body draws that docx2svg warns of, in its drawing order.
    fn body(&mut self, model: &mut Model, layout: &DocumentLayout, page: usize, out: &mut Said) {
        let Some(laid) = layout.laid_page(page) else { return };
        let number = Some(page + 1);
        let mut lines = laid.lines.iter().map(|l| (l.item, Some(l.line))).peekable();
        let mut rows = laid.rows.iter().map(|r| (r.item, None)).peekable();
        // The page's items in flow order: its lines' and its rows'.
        let mut order: Vec<(usize, Option<usize>)> = Vec::new();
        loop {
            let next = match (lines.peek(), rows.peek()) {
                (Some(a), Some(b)) if a.0 <= b.0 => lines.next(),
                (Some(_), Some(_)) | (None, Some(_)) => rows.next(),
                (Some(_), None) => lines.next(),
                (None, None) => None,
            };
            let Some(next) = next else { break };
            order.push(next);
        }
        let mut done: HashSet<usize> = HashSet::new();
        for (item, line) in order {
            match layout.flow.items.get(item) {
                Some(FlowItem::Para(p)) => {
                    let (Some(paragraph), Some(line)) = (self.paragraphs.get(p.block).cloned(), line) else {
                        continue;
                    };
                    self.drawn |= holds_drawing(&paragraph);
                    Self::line(model, layout, (&paragraph, p, line), number, out);
                }
                Some(FlowItem::Table(t)) if done.insert(item) => {
                    self.table(model, layout, laid, (item, t), number, out);
                }
                _ => {}
            }
        }
    }

    /// A body line of `paragraph` (flow item `item`), line `line` of it, on page `number`.
    fn line(
        model: &mut Model,
        layout: &DocumentLayout,
        (paragraph, item, line): (&std::sync::Arc<Paragraph>, &ooxml_word_layout::flow::ParaItem, usize),
        number: Option<usize>,
        out: &mut Said,
    ) {
        let document = &layout.document;
        let multi = layout.sections.get(item.section).is_some_and(|s| s.multi_column());
        if line == 0 && multi && layout.mode15 && layout.roles.get(item.body).is_some_and(Option::is_some) {
            // In mode 15 Word balances the text above the endnotes and sets them as a
            // region of their own (`paginate.flow`'s carry-on): not modelled.
            out.approximate(
                "columns",
                "an endnote in a section of several text columns is laid out as in a column of its own (not measured)",
                number,
                Some(&paragraph.path),
            );
        }
        let resolved = model.resolve_paragraph(document, paragraph);
        if resolved.paragraph.get("jc").and_then(Value::str) == Some("distribute") {
            out.warn(
                "alignment-distribute-approximate",
                "a distributed paragraph's last line is spread evenly between its glyphs; Word spreads it by a rule \
                 not measured",
                None,
            );
        }
        if line == 0 {
            for (k, run) in paragraph.runs.iter().enumerate() {
                let underline = resolved.runs.get(k).and_then(|p| p.get("u").and_then(Value::str));
                if let Some(u) = underline.filter(|u| !matches!(*u, "single" | "words" | "dotted" | "dash" | "none")) {
                    if run.text.chars().any(|c| !c.is_whitespace()) {
                        out.warn(
                            "underline-style-approximate",
                            format!(
                                "a {u} underline is drawn approximately (only single, words, dotted and dash \
                                 underlines are measured to the pixel)"
                            ),
                            None,
                        );
                    }
                }
            }
        }
        if line == 0
            && paragraph
                .runs
                .iter()
                .any(|r| r.drawings.iter().any(|d| matches!(d, InlineObject::None)))
        {
            out.warn(
                "drawing-not-drawn",
                "an inline drawing that is not a picture (a chart, a shape) is drawn as a placeholder of its extent",
                None,
            );
        }
        for anchor in item.anchors.iter().filter(|a| a.line == line) {
            if !anchor.anchor.allow_overlap {
                out.warn(
                    "drawing-overlap-not-modelled",
                    "a floating drawing that may not overlap another (allowOverlap=\"0\") is drawn where it is \
                     positioned; Word moves it clear of the drawings before it",
                    number,
                );
            }
        }
    }

    /// A table on the page (`_table_drawn`'s warnings).
    fn table(
        &mut self,
        model: &mut Model,
        layout: &DocumentLayout,
        laid: &ooxml_word_layout::paginate::LaidPage,
        (item, t): (usize, &ooxml_word_layout::flow::TableItem),
        number: Option<usize>,
        out: &mut Said,
    ) {
        let Some(table) = t.flow.as_any().downcast_ref::<MeasuredTable>() else {
            return;
        };
        let path = &*table.path;
        for row in laid.rows.iter().filter(|r| r.item == item) {
            if row.row.overflowed && !row.row.header {
                let message = format!(
                    "a row of {path} (w:tr[{}]) not one line of which fits on an empty page is drawn whole, past \
                     the page's foot (Word cuts it off there: not modelled)",
                    row.row.row + 1
                );
                out.approximate(
                    "table",
                    &message,
                    number,
                    Some(&format!("{path}/w:tr[{}]", row.row.row + 1)),
                );
            }
        }
        if !self.tables.insert(item) {
            return;
        }
        if let Some(why) = &table.approximation {
            let message = format!(
                "{path} is laid out by approximate rules, the measured ones refusing it ({why}); what each \
                 approximates is reported with it"
            );
            out.approximate("table", &message, number, Some(path));
        }
        for found in table.approximated.iter().filter(|a| a.reason != "placeholder") {
            out.approximate(found.reason, &found.message, number, Some(&found.path));
        }
        let Some(Block::Table(model_table)) = layout.document.body.get(t.body) else {
            return;
        };
        let resolved = model.resolve_table(&layout.document, model_table);
        if resolved.mismatched.is_empty() {
            return;
        }
        let grid = model_table.grid.len();
        let rows: Vec<String> = resolved
            .mismatched
            .iter()
            .take(10)
            .map(|(row, reach)| format!("w:tr[{}] reaches {reach}", row + 1))
            .collect();
        let more = if resolved.mismatched.len() > 10 {
            format!(" and {} more", resolved.mismatched.len() - 10)
        } else {
            String::new()
        };
        let widened = resolved
            .mismatched
            .iter()
            .any(|(_, reach)| usize::try_from(*reach).is_ok_and(|r| r > grid));
        let how = if widened {
            "laid out on the grid widened to its widest row"
        } else {
            "the shorter rows drawn short"
        };
        out.warn(
            "table-grid-mismatch",
            format!(
                "{path}: rows off its w:tblGrid of {grid} column(s) ({}{more} grid columns): {how}, as Word lays it out",
                rows.join(", ")
            ),
            number,
        );
    }

    /// Page `page`'s header's and footer's fields that cannot be computed (`_draw_story`).
    fn stories(&mut self, layout: &DocumentLayout, page: usize, out: &mut Said) {
        let Some(info) = layout.pagination.info.get(page) else {
            return;
        };
        if info.blank {
            return;
        }
        let document = &layout.document;
        let references = story_references(&document.sections);
        let (Some((headers, footers)), Some(section)) =
            (references.get(info.section), document.sections.get(info.section))
        else {
            return;
        };
        let counts = page_counts(layout, info.section);
        let format = section.properties.page_number_format.as_deref();
        for refs in [headers, footers] {
            let Some(id) = refs.get(info.story.name()) else {
                continue;
            };
            let Some(blocks) = document.stories.get(id) else {
                continue;
            };
            let part = document.story_parts.get(id).cloned().unwrap_or_else(|| id.clone());
            let (paragraphs, _) = body_paragraphs(blocks);
            for paragraph in &paragraphs {
                for run in paragraph.runs.iter() {
                    let Some(field) = run.field.as_ref().filter(|f| &*f.keyword != "STYLEREF") else {
                        continue;
                    };
                    if let Err(Uncomputable(why)) =
                        field_text(&field.instruction, &field.keyword, info.number, counts, format)
                    {
                        out.warn(
                            format!("field-not-computed:{}", field.keyword),
                            format!("a {} field in {part} is drawn as Word cached it ({why})", field.keyword),
                            None,
                        );
                    }
                }
            }
        }
    }
}
