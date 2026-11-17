//! Owns: the body as the paginator sees it ([`Flow`], [`FlowItem`]): each top-level
//! paragraph measured, with its spacing (autospacing against its neighbours), its border
//! box against the paragraphs beside it, its keeps, its section and whether it starts it
//! or only holds a section break ([`ParaItem`]); each top-level table as its table layout
//! measured it ([`TableItem`]); and where the layout must stop ([`StopItem`]) --
//! docx2svg's `paginate.flow` for what L3a lays out.
//!
//! Measures are kept per block by identity ([`MeasureCache`]: the model shares an
//! unchanged block's `Arc` between versions) and by content (the text engine's
//! [`Measurer`]), so a version that changed one block measures one block.
//!
//! Must not: place anything on a page, or lay tables out (a [`TableLayout`] does).

use crate::faces::Faces;
use crate::input::section_geometry;
use crate::paragraph::{measure_paragraph, MeasureContext, MeasuredParagraph, ParagraphJob, Unmeasured};
use crate::px::Px;
use crate::sections::Sections;
use crate::table::{TableFlow, TableLayout};
use crate::vertical::{autospace_kept, border_px};
use ooxml_base::Locale;
use ooxml_text::Measurer;
use ooxml_word_model::model::{Block, Paragraph, SectionProps};
use ooxml_word_model::value::{Props, Text, Value};
use ooxml_word_model::{Model, WordDocument};
use std::collections::HashMap;
use std::sync::Arc;

/// A top-level paragraph as the paginator sees it (`paginate.Para`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParaItem {
    /// Its index among the body's paragraphs, cells included (`baselines.blocks` order).
    pub block: usize,
    /// Its index in the body's top-level blocks.
    pub body: usize,
    /// Its section.
    pub section: usize,
    /// Its lines.
    pub measured: Arc<MeasuredParagraph>,
    /// Space before and after, twips (autospacing applied).
    pub before: i64,
    /// Space after, twips.
    pub after: i64,
    /// `w:contextualSpacing`.
    pub contextual: bool,
    /// Its `w:pStyle`.
    pub style: Option<Text>,
    /// What its borders take below its last line and above its first.
    pub border: Px,
    /// Above its first line.
    pub top_border: Px,
    /// `w:keepNext`.
    pub keep_next: bool,
    /// `w:keepLines`.
    pub keep_lines: bool,
    /// `w:pageBreakBefore`.
    pub page_break_before: bool,
    /// `w:widowControl`, where stated.
    pub widow: Option<bool>,
    /// The first item of its section.
    pub section_start: bool,
    /// An empty paragraph holding a section break: it takes no room of its own.
    pub section_mark: bool,
}

impl ParaItem {
    /// How many lines it has.
    pub fn count(&self) -> usize {
        self.measured.heights.len()
    }
}

/// A top-level table as its table layout measured it.
#[derive(Debug, Clone)]
pub struct TableItem {
    /// Its first paragraph's block index.
    pub block: usize,
    /// Its index in the body's top-level blocks.
    pub body: usize,
    /// Its section.
    pub section: usize,
    /// The first item of its section.
    pub section_start: bool,
    /// Its rows, as the table layout placed them.
    pub flow: Arc<dyn TableFlow>,
}

/// Where the layout stops: what it cannot lay out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopItem {
    /// The block's index.
    pub block: usize,
    /// Its index in the body's top-level blocks.
    pub body: usize,
    /// Its section.
    pub section: usize,
    /// Why.
    pub reason: String,
}

/// One item of the flow.
#[derive(Debug, Clone)]
pub enum FlowItem {
    /// A paragraph.
    Para(Arc<ParaItem>),
    /// A table.
    Table(Arc<TableItem>),
    /// A stop.
    Stop(Arc<StopItem>),
}

impl FlowItem {
    /// Its section.
    pub fn section(&self) -> usize {
        match self {
            FlowItem::Para(p) => p.section,
            FlowItem::Table(t) => t.section,
            FlowItem::Stop(s) => s.section,
        }
    }

    /// Its first block.
    pub fn block(&self) -> usize {
        match self {
            FlowItem::Para(p) => p.block,
            FlowItem::Table(t) => t.block,
            FlowItem::Stop(s) => s.block,
        }
    }

    /// Its lines (a table's rows).
    pub fn count(&self) -> usize {
        match self {
            FlowItem::Para(p) => p.count(),
            FlowItem::Table(t) => t.flow.rows(),
            FlowItem::Stop(_) => 1,
        }
    }

    /// The paragraph, if it is one.
    pub fn para(&self) -> Option<&ParaItem> {
        match self {
            FlowItem::Para(p) => Some(p),
            _ => None,
        }
    }
}

/// The body's flow.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    /// Its items, in order.
    pub items: Vec<FlowItem>,
}

/// What a block's measure depends on beyond its content: its section's geometry, its list
/// label and the parts it resolves against.
#[derive(Debug, Clone)]
struct Kept {
    paragraph: Arc<Paragraph>,
    section: Arc<SectionProps>,
    label: Option<String>,
    notes: Vec<bool>,
    measured: Result<Arc<MeasuredParagraph>, Unmeasured>,
}

/// Measures kept per block by identity: the model shares an unchanged block's `Arc`
/// between versions, and each entry holds its block, so an address names one block for
/// as long as it is kept. Valid for one set of parts (styles, numbering, settings, theme)
/// and faces: [`MeasureCache::clear`] when they change.
#[derive(Debug, Default)]
pub struct MeasureCache {
    blocks: HashMap<usize, Kept>,
}

impl MeasureCache {
    /// Forget everything.
    pub fn clear(&mut self) {
        self.blocks.clear();
    }

    /// How many blocks are kept.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    fn get(
        &self,
        paragraph: &Arc<Paragraph>,
        section: &Arc<SectionProps>,
        label: Option<&str>,
        notes: &[bool],
    ) -> Option<Result<Arc<MeasuredParagraph>, Unmeasured>> {
        let kept = self.blocks.get(&(Arc::as_ptr(paragraph) as usize))?;
        let same = Arc::ptr_eq(&kept.paragraph, paragraph)
            && (Arc::ptr_eq(&kept.section, section) || kept.section == *section)
            && kept.label.as_deref() == label
            && kept.notes == notes;
        same.then(|| kept.measured.clone())
    }

    /// Keep only the top-level paragraphs of `blocks` (a new version's body): the rest
    /// are dropped.
    pub fn retain_blocks(&mut self, blocks: &[Block]) {
        let live: std::collections::HashSet<usize> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(Arc::as_ptr(p) as usize),
                Block::Table(_) => None,
            })
            .collect();
        self.blocks.retain(|k, _| live.contains(k));
    }
}

/// Where a flow's list labels come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Labels {
    /// The body's, counted by the model.
    Body,
    /// None (a header's or footer's story, whose numbered paragraphs the model does not
    /// count).
    None,
}

/// What building a flow needs.
pub struct FlowContext<'a> {
    /// The document.
    pub document: &'a WordDocument,
    /// The story laid out: the body's blocks, or a header's or footer's.
    pub blocks: &'a [Block],
    /// Its sections.
    pub sections: &'a Sections,
    /// Its list labels.
    pub labels: Labels,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale measures are keyed under.
    pub locale: &'a Locale,
    /// The table layout.
    pub tables: &'a dyn TableLayout,
    /// Per top-level block, per run, whether a run draws a note's number (empty: none).
    pub note_runs: &'a HashMap<usize, Vec<bool>>,
}

/// What the flow builder keeps between versions: the measures by block and by content.
pub struct FlowState {
    /// Measures by block.
    pub cache: MeasureCache,
    /// Measures by content.
    pub measurer: Measurer,
}

fn flag(props: &Props, key: &str) -> bool {
    props.get(key).is_some_and(Value::truthy)
}

fn int(props: &Props, key: &str) -> i64 {
    props.get(key).and_then(Value::int).unwrap_or(0)
}

/// The paragraphs of the body in document order, cells included, and each top-level
/// block's first index among them.
pub fn body_paragraphs(body: &[Block]) -> (Vec<Arc<Paragraph>>, Vec<usize>) {
    fn walk(block: &Block, out: &mut Vec<Arc<Paragraph>>) {
        match block {
            Block::Paragraph(p) => out.push(Arc::clone(p)),
            Block::Table(t) => {
                for row in t.rows.iter() {
                    for cell in row.iter() {
                        for inner in cell.iter() {
                            walk(inner, out);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut firsts = Vec::with_capacity(body.len());
    for block in body {
        firsts.push(out.len());
        walk(block, &mut out);
    }
    (out, firsts)
}

/// Whether a paragraph holds nothing but its mark (no text, tab, break or drawing).
fn is_empty(paragraph: &Paragraph) -> bool {
    !paragraph
        .runs
        .iter()
        .any(|r| !r.text.is_empty() || !r.breaks.is_empty() || !r.anchors.is_empty())
}

const BORDER_SIDES: [&str; 6] = ["top", "left", "bottom", "right", "between", "bar"];

fn border_of<'a>(pp: &'a Props, side: &str) -> Option<&'a ooxml_word_model::value::Border> {
    match pp.get(format!("pBdr.{side}").as_str()) {
        Some(Value::Border(b)) => Some(b),
        _ => None,
    }
}

fn drawn(border: Option<&ooxml_word_model::value::Border>) -> bool {
    border.is_some_and(|b| !matches!(b.style.as_deref(), None | Some("nil" | "none")))
}

/// The height one `w:pBdr` side takes (`side_px`): space and width, a `double` line
/// three widths; a `w:between` border its space on both sides.
fn side_px(border: Option<&ooxml_word_model::value::Border>, between: bool) -> Px {
    let Some(b) = border.filter(|b| drawn(Some(b))) else {
        return Px::ZERO;
    };
    let tall = if b.style.as_deref() == Some("double") { 3 } else { 1 };
    let height = border_px(b.size * tall, b.space);
    if between {
        height + Px::twips(b.space * 20)
    } else {
        height
    }
}

/// What decides whether two consecutive paragraphs share one border box (`border_box`).
fn border_box(pp: &Props) -> (Vec<Option<&ooxml_word_model::value::Border>>, i64, i64) {
    (
        BORDER_SIDES.iter().map(|s| border_of(pp, s)).collect(),
        int(pp, "ind.left"),
        int(pp, "ind.right"),
    )
}

/// `(above, below)`: the room a paragraph's borders take above its first line and below
/// its last, given the paragraphs beside it (`paragraph_borders_px`).
pub fn paragraph_borders(pp: &Props, previous: Option<&Props>, following: Option<&Props>) -> (Px, Px) {
    let own = border_box(pp);
    if !own.0.iter().any(|b| drawn(*b)) {
        return (Px::ZERO, Px::ZERO);
    }
    let after_same = following.is_some_and(|f| border_box(f) == own);
    let before_same = previous.is_some_and(|p| border_box(p) == own);
    let between = border_of(pp, "between");
    let upper = match between.filter(|b| drawn(Some(b))) {
        Some(b) => Px::twips(b.space * 20),
        None => Px::ZERO,
    };
    let above = if before_same {
        side_px(between, true) - upper
    } else {
        side_px(border_of(pp, "top"), false)
    };
    let below = if after_same {
        upper
    } else {
        side_px(border_of(pp, "bottom"), false)
    };
    (above, below)
}

/// Whether a side of a resolved paragraph is autospaced (`autospaced`).
fn autospaced(document: &WordDocument, pp: &Props, side: &str) -> bool {
    flag(pp, &format!("spacing.{side}Autospacing"))
        && !document.context.settings.option("doNotUseHTMLParagraphAutoSpacing")
}

impl FlowState {
    /// A state measuring under `measurer`.
    pub fn new(measurer: Measurer) -> FlowState {
        FlowState {
            cache: MeasureCache::default(),
            measurer,
        }
    }
}

/// Builds one version's flow.
pub struct FlowBuilder<'a, 'm> {
    cx: &'a FlowContext<'a>,
    model: &'m mut Model,
    state: &'m mut FlowState,
    paragraphs: Vec<Arc<Paragraph>>,
    firsts: Vec<usize>,
    resolved: HashMap<usize, Arc<Props>>,
}

impl<'a, 'm> FlowBuilder<'a, 'm> {
    /// A builder of `cx.document`'s flow.
    pub fn new(cx: &'a FlowContext<'a>, model: &'m mut Model, state: &'m mut FlowState) -> FlowBuilder<'a, 'm> {
        let (paragraphs, firsts) = body_paragraphs(cx.blocks);
        FlowBuilder {
            cx,
            model,
            state,
            paragraphs,
            firsts,
            resolved: HashMap::new(),
        }
    }

    fn props_of_block(&mut self, block: usize) -> Option<Arc<Props>> {
        if let Some(found) = self.resolved.get(&block) {
            return Some(Arc::clone(found));
        }
        let paragraph = self.paragraphs.get(block)?;
        let found = self.model.resolve_paragraph_props(self.cx.document, paragraph);
        self.resolved.insert(block, Arc::clone(&found));
        Some(found)
    }

    fn numbered(&mut self, block: usize) -> Option<bool> {
        let pp = self.props_of_block(block)?;
        Some(int(&pp, "numPr.numId") != 0)
    }

    /// The flow.
    pub fn build(mut self) -> Flow {
        let body = self.cx.blocks;
        let mut items = Vec::with_capacity(body.len());
        let mut top_level = 0;
        let mut seen = vec![false; self.cx.sections.list.len().max(1)];
        for (index, block) in body.iter().enumerate() {
            let first = self.firsts.get(index).copied().unwrap_or(0);
            let section = self.cx.sections.section_of(top_level);
            let section_start = !seen.get(section).copied().unwrap_or(true);
            if let Some(s) = seen.get_mut(section) {
                *s = true;
            }
            let item = match block {
                Block::Table(t) => self.table_item(t, index, first, section, section_start),
                Block::Paragraph(p) => {
                    let item = self.paragraph_item(p, index, first, top_level, section, section_start);
                    top_level += 1;
                    item
                }
            };
            items.push(item);
        }
        Flow { items }
    }

    fn table_item(
        &mut self,
        table: &Arc<ooxml_word_model::model::Table>,
        body: usize,
        block: usize,
        section: usize,
        section_start: bool,
    ) -> FlowItem {
        let previous = match body.checked_sub(1).and_then(|k| self.cx.blocks.get(k)) {
            Some(Block::Paragraph(p)) => Some(Arc::clone(p)),
            _ => None,
        };
        let geometry = self.cx.sections.get(section).cloned();
        let flowed = geometry
            .ok_or_else(|| crate::table::Unsupported("no section".into()))
            .and_then(|g| {
                self.cx
                    .tables
                    .flow_table(self.cx.document, table, &g, previous.as_deref())
            });
        match flowed {
            Ok(flow) => FlowItem::Table(Arc::new(TableItem {
                block,
                body,
                section,
                section_start,
                flow,
            })),
            Err(why) => FlowItem::Stop(Arc::new(StopItem {
                block,
                body,
                section,
                reason: why.to_string(),
            })),
        }
    }

    fn measured(
        &mut self,
        paragraph: &Arc<Paragraph>,
        body: usize,
        section: usize,
    ) -> Result<Arc<MeasuredParagraph>, Unmeasured> {
        let document = self.cx.document;
        let label = match self.cx.labels {
            Labels::Body => document.label(body, 0),
            Labels::None => None,
        };
        let notes = self.cx.note_runs.get(&body).cloned().unwrap_or_default();
        let geometry = self.cx.sections.get(section).cloned();
        let Some(geometry) = geometry else {
            return Err(Unmeasured::Text("no section".into()));
        };
        if let Some(found) = self
            .state
            .cache
            .get(paragraph, &geometry.props, label.as_deref(), &notes)
        {
            return found;
        }
        let resolved = self.model.resolve_paragraph(document, paragraph);
        let text_geometry = section_geometry(document, &resolved.paragraph, &geometry);
        let mcx = MeasureContext {
            document,
            faces: self.cx.faces,
            locale: self.cx.locale,
        };
        let job = ParagraphJob {
            paragraph,
            resolved: &resolved,
            geometry: &text_geometry,
            columns: geometry.multi_column(),
            label: label.as_deref(),
            notes: &notes,
        };
        let measured = measure_paragraph(&mcx, &mut self.state.measurer, &job).map(Arc::new);
        self.state.cache.blocks.insert(
            Arc::as_ptr(paragraph) as usize,
            Kept {
                paragraph: Arc::clone(paragraph),
                section: Arc::clone(&geometry.props),
                label,
                notes,
                measured: measured.clone(),
            },
        );
        measured
    }

    fn neighbour_props(&mut self, body: usize, offset: isize) -> Option<Arc<Props>> {
        let k = body.checked_add_signed(offset)?;
        match self.cx.blocks.get(k)? {
            Block::Paragraph(_) => {
                let block = *self.firsts.get(k)?;
                self.props_of_block(block)
            }
            Block::Table(_) => None,
        }
    }

    /// What of a paragraph this layout does not lay out: a drop cap's frame and a section
    /// of unequal columns (a paragraph broken again for each column's width).
    fn not_laid_out(&mut self, block: usize, section: usize) -> Option<&'static str> {
        if self.cx.sections.get(section).is_some_and(|s| s.unequal_columns()) {
            return Some("a section of unequal columns");
        }
        let pp = self.props_of_block(block)?;
        let drop_cap = match pp.get("framePr") {
            Some(Value::Attrs(attrs)) => attrs
                .iter()
                .any(|(k, v)| &**k == "dropCap" && matches!(&**v, "drop" | "margin")),
            _ => false,
        };
        drop_cap.then_some("a drop cap")
    }

    fn spacing(&mut self, pp: &Props, block: usize) -> (i64, i64) {
        let document = self.cx.document;
        let mut before = int(pp, "spacing.before");
        let mut after = int(pp, "spacing.after");
        let numbered = self.numbered(block).unwrap_or(false);
        if autospaced(document, pp, "before") {
            let neighbour = match block.checked_sub(1) {
                Some(b) => self.numbered(b),
                None => None,
            };
            if !autospace_kept(numbered, neighbour) {
                before = 0;
            }
        }
        if autospaced(document, pp, "after") {
            let neighbour = Some(self.numbered(block + 1).unwrap_or(false));
            if !autospace_kept(numbered, neighbour) {
                after = 0;
            }
        }
        (before, after)
    }

    fn paragraph_item(
        &mut self,
        paragraph: &Arc<Paragraph>,
        body: usize,
        block: usize,
        top_level: usize,
        section: usize,
        section_start: bool,
    ) -> FlowItem {
        if let Some(why) = self.not_laid_out(block, section) {
            return FlowItem::Stop(Arc::new(StopItem {
                block,
                body,
                section,
                reason: why.into(),
            }));
        }
        let measured = match self.measured(paragraph, body, section) {
            Ok(m) => m,
            Err(why) => {
                return FlowItem::Stop(Arc::new(StopItem {
                    block,
                    body,
                    section,
                    reason: format!("{why:?}"),
                }))
            }
        };
        let Some(pp) = self.props_of_block(block) else {
            return FlowItem::Stop(Arc::new(StopItem {
                block,
                body,
                section,
                reason: "unresolved".into(),
            }));
        };
        let (before, after) = self.spacing(&pp, block);
        let previous = self.neighbour_props(body, -1);
        let following = self.neighbour_props(body, 1);
        let (top_border, border) = paragraph_borders(&pp, previous.as_deref(), following.as_deref());
        let section_mark =
            self.cx.sections.ends_section(top_level) && is_empty(paragraph) && measured.heights.len() == 1;
        FlowItem::Para(Arc::new(ParaItem {
            block,
            body,
            section,
            before,
            after,
            contextual: flag(&pp, "contextualSpacing"),
            style: paragraph.properties.as_ref().and_then(|p| p.style_id.clone()),
            border,
            top_border,
            keep_next: flag(&pp, "keepNext"),
            keep_lines: flag(&pp, "keepLines"),
            page_break_before: flag(&pp, "pageBreakBefore"),
            widow: pp.get("widowControl").map(Value::truthy),
            section_start,
            section_mark,
            measured,
        }))
    }
}
