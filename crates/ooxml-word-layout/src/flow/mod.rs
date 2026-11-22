//! Owns: the body as the paginator sees it ([`Flow`], [`FlowItem`]): each top-level
//! paragraph measured, with its spacing (autospacing against its neighbours), its border
//! box against the paragraphs beside it, its keeps, its section and whether it starts it
//! or only holds a section break ([`ParaItem`]); each top-level table as its table layout
//! measured it ([`TableItem`]); and where the layout must stop ([`StopItem`]) --
//! docx2svg's `paginate.flow` for what L3a lays out.
//!
//! Measures are kept per block by `Arc` ([`MeasureCache`]: the model shares an
//! unchanged block's `Arc` between versions) and by content (the text engine's
//! [`Measurer`]), so a version that changed one block measures one block.
//!
//! Must not: place anything on a page, or lay tables out (a [`TableLayout`] does).

use crate::faces::Faces;
pub use crate::floating::{anchors_of, moves_text, AnchorRef, SIDE_WRAPS};
use crate::input::section_geometry;
use crate::notes::flowed::NoteMaker;
use crate::notes::{NoteLines, NoteRole};
use crate::paragraph::{measure_paragraph, MeasureContext, MeasuredParagraph, ParagraphJob, Unmeasured};
use crate::placeholder::Placeholder;
use crate::px::Px;
use crate::sections::Sections;
use crate::stop::Stop;
use crate::table::{TableContext, TableFlow, TableJob, TableLayout, Unsupported};
use crate::vertical::autospace_kept;
use ooxml_base::Locale;
use ooxml_text::Measurer;
use ooxml_word_model::model::{Block, Paragraph, Table};
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
    /// The footnotes referenced in it, each as its lines, by the line its reference is on
    /// (in line order, each line's in the order referenced).
    pub notes: Vec<(usize, NoteLines)>,
    /// What of the endnotes placed in the flow it is.
    pub role: Option<NoteRole>,
    /// An endnote's paragraph: the height of the continuation separator's line a page it
    /// starts opens with.
    pub continuation: Option<crate::heights::LineHeight>,
    /// Its floating drawings that take no room in their line: those text does not wrap
    /// around, and those it wraps top and bottom of or beside (`anchors_of`).
    pub anchors: Vec<AnchorRef>,
    /// What its lines are broken from, where the paginator may break them again.
    pub source: Option<Arc<crate::rebreak::ParaSource>>,
    /// The drop cap that drops into it.
    pub drop_cap: Option<Arc<DropCap>>,
    /// What a page lays its footnotes out from beyond their lines.
    pub note_forms: NoteForms,
    /// What it stands for where it is a placeholder of what the layout cannot lay out: its
    /// lines are estimated and hold no text.
    pub placeholder: Option<Arc<Placeholder>>,
}

impl ParaItem {
    /// How many lines it has.
    pub fn count(&self) -> usize {
        self.measured.heights.len()
    }

    /// The pitch of the continuation separator's line a page it starts opens with.
    pub fn continuation_pitch(&self) -> Option<Px> {
        self.continuation.as_ref().map(crate::heights::LineHeight::pitch)
    }

    /// Whether line `line` draws anything: no placeholder's line does, nor a line of no
    /// character but spaces (`undrawn`).
    pub fn draws(&self, line: usize) -> bool {
        self.placeholder.is_none() && !self.measured.undrawn.contains(&line)
    }

    /// The footnotes referenced on line `line`.
    pub fn notes_on(&self, line: usize) -> impl Iterator<Item = &NoteLines> {
        self.notes.iter().filter(move |(l, _)| *l == line).map(|(_, n)| n)
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
    /// The footnotes referenced in its cells.
    pub notes: Arc<CellNotes>,
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

    /// Its index in the body's top-level blocks.
    pub fn body(&self) -> usize {
        match self {
            FlowItem::Para(p) => p.body,
            FlowItem::Table(t) => t.body,
            FlowItem::Stop(s) => s.body,
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
    /// Per section whose paragraphs reference footnotes, the heights of its footnote
    /// separator and continuation separator.
    pub separators: std::collections::BTreeMap<usize, (Px, Px)>,
    /// In mode 15, per footnote referenced in a section of several columns, its paragraphs
    /// as a flow in that section's columns: what the page's note area is laid out from.
    pub note_items: NoteItems,
    /// Below mode 15, per footnote referenced in a section of unequal columns, its lines
    /// broken in each of the section's column widths: what a column's foot holds of it.
    pub notes_at: NotesAt,
    /// The body has drawings text wraps beside or floating tables.
    pub wraps: bool,
}

/// `(note, section, width)` -> the note's lines broken in a column that wide.
pub type NotesAt = std::collections::BTreeMap<(i64, usize, i64), NoteLines>;

/// `(note, section)` -> the note's paragraphs as a flow in the section's columns.
pub type NoteItems = std::collections::BTreeMap<(i64, usize), Arc<[FlowItem]>>;

impl Flow {
    /// Section `section`'s footnote separator's and continuation separator's heights.
    pub fn separators(&self, section: usize) -> (Px, Px) {
        self.separators.get(&section).copied().unwrap_or((Px::ZERO, Px::ZERO))
    }
}

/// Where a flow's list labels come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Labels<'a> {
    /// Each paragraph's, in document order (cells included): the body's, as the model
    /// counts them ([`WordDocument::body_labels`]).
    Given(&'a [Option<String>]),
    /// None (a header's or footer's story, whose numbered paragraphs the model does not
    /// count).
    None,
}

impl Labels<'_> {
    /// Paragraph `block`'s label, where it has one.
    fn get(&self, block: usize) -> Option<&str> {
        match self {
            Labels::Given(all) => all.get(block).and_then(Option::as_deref),
            Labels::None => None,
        }
    }

    fn of(&self, block: usize) -> Option<String> {
        match self {
            Labels::Given(all) => all.get(block).cloned().flatten(),
            Labels::None => None,
        }
    }
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
    pub labels: Labels<'a>,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale measures are keyed under.
    pub locale: &'a Locale,
    /// The table layout.
    pub tables: &'a dyn TableLayout,
    /// Per top-level block, what of the notes placed in the flow it is (empty: none is).
    pub roles: &'a [Option<NoteRole>],
}

/// What the flow maker keeps between versions: the measures by block and by content.
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
pub(crate) fn is_empty(paragraph: &Paragraph) -> bool {
    !paragraph
        .runs
        .iter()
        .any(|r| !r.text.is_empty() || !r.breaks.is_empty() || !r.anchors.is_empty())
}

pub use crate::borders::paragraph_borders;

mod again;
mod cache;
mod carried;
mod drop;
mod notes;
mod source;
pub use again::OldFlow;
pub use cache::MeasureCache;
use cache::{Kept, KeptTable};
pub use drop::DropCap;
use notes::ParaNotes;
pub use notes::{CellNotes, NoteForms};

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

/// Where the layout stops at block `block` (top-level `body`, in `section`), and why.
fn stop(block: usize, body: usize, section: usize, reason: impl Into<String>) -> FlowItem {
    FlowItem::Stop(Arc::new(StopItem {
        block,
        body,
        section,
        reason: reason.into(),
    }))
}

/// Builds one version's flow.
pub struct FlowMaker<'a, 'm> {
    cx: &'a FlowContext<'a>,
    model: &'m mut Model,
    state: &'m mut FlowState,
    paragraphs: Vec<Arc<Paragraph>>,
    firsts: Vec<usize>,
    resolved: HashMap<usize, Arc<Props>>,
    notes: NoteMaker,
    separators: std::collections::BTreeMap<usize, (Px, Px)>,
    note_items: NoteItems,
    notes_at: NotesAt,
    /// The body has drawings text wraps beside or floating tables, or drop caps.
    wraps: bool,
    /// The drop cap the next paragraph carries.
    pending_drop: Option<Arc<DropCap>>,
    /// Asked before each top-level block: the flow ends there where it says stop.
    stop: Option<&'a Stop<'a>>,
    /// What it cannot lay out is a placeholder it carries on past ([`FlowMaker::carry_on`]).
    carry_on: bool,
}

impl<'a, 'm> FlowMaker<'a, 'm> {
    /// A builder of `cx.document`'s flow.
    pub fn new(cx: &'a FlowContext<'a>, model: &'m mut Model, state: &'m mut FlowState) -> FlowMaker<'a, 'm> {
        let (paragraphs, firsts) = body_paragraphs(cx.blocks);
        FlowMaker {
            cx,
            model,
            state,
            paragraphs,
            firsts,
            resolved: HashMap::new(),
            notes: NoteMaker::default(),
            separators: std::collections::BTreeMap::new(),
            note_items: NoteItems::new(),
            notes_at: NotesAt::new(),
            wraps: source::wraps(cx.blocks),
            pending_drop: None,
            stop: None,
            carry_on: false,
        }
    }

    /// What it cannot lay out taken in as a placeholder ([`Placeholder`]), the layout carrying
    /// on past it, rather than a stop (docx2svg's `CARRY_ON`, for the body's flow).
    #[must_use]
    pub fn carry_on(mut self) -> FlowMaker<'a, 'm> {
        self.carry_on = true;
        self
    }

    /// The flow ended before the first top-level block `stop` says stop at (a flow its
    /// layout's caller drops).
    #[must_use]
    pub fn until(mut self, stop: &'a Stop<'a>) -> FlowMaker<'a, 'm> {
        self.stop = Some(stop);
        self
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
        let all = 0..self.cx.blocks.len();
        let drops = self.drop_caps(all.clone());
        self.wraps = self.wraps || !drops.is_empty();
        let items = self.items_in(all, &drops);
        Flow {
            items,
            separators: self.separators,
            note_items: self.note_items,
            notes_at: self.notes_at,
            wraps: self.wraps,
        }
    }

    /// How many top-level paragraphs stand before top-level block `body`.
    fn top_level_before(&self, body: usize) -> usize {
        self.cx
            .blocks
            .get(..body)
            .unwrap_or_default()
            .iter()
            .filter(|b| matches!(b, Block::Paragraph(_)))
            .count()
    }

    /// The items of top-level blocks `range` (`drops`: the drop caps among them), the
    /// blocks before it laid out already.
    fn items_in(&mut self, range: std::ops::Range<usize>, drops: &[usize]) -> Vec<FlowItem> {
        let body = self.cx.blocks;
        let mut items = Vec::with_capacity(range.len());
        let mut top_level = self.top_level_before(range.start);
        let mut seen = vec![false; self.cx.sections.list.len().max(1)];
        if let Some(previous) = range.start.checked_sub(1).and_then(|k| body.get(k)) {
            // Every section up to the block before's has started.
            let before = top_level - usize::from(matches!(previous, Block::Paragraph(_)));
            let last = self.cx.sections.section_of(before);
            for s in seen.iter_mut().take(last + 1) {
                *s = true;
            }
        }
        for (index, block) in body.iter().enumerate().take(range.end).skip(range.start) {
            if self.stop.is_some_and(Stop::now) {
                break;
            }
            let first = self.firsts.get(index).copied().unwrap_or(0);
            let section = self.cx.sections.section_of(top_level);
            let section_start = !seen.get(section).copied().unwrap_or(true);
            if let Some(s) = seen.get_mut(section) {
                *s = true;
            }
            let item = match block {
                Block::Table(t) => self.table_item(t, index, first, section, section_start),
                Block::Paragraph(p) => {
                    if drops.contains(&index) {
                        // A drop cap: not in the stack; the paragraph after it carries it.
                        self.pending_drop = self.drop_cap(p, index, section).map(Arc::new);
                        if self.pending_drop.is_some() {
                            top_level += 1;
                            continue;
                        }
                    }
                    let item = self.paragraph_item(p, index, first, top_level, section, section_start);
                    top_level += 1;
                    item
                }
            };
            items.push(item);
        }
        items
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
        let first = self.firsts.get(body).copied().unwrap_or(0);
        let labels: Vec<Option<String>> = (first..first + self.paragraphs_in(body))
            .map(|block| self.cx.labels.of(block))
            .collect();
        let flowed = match geometry {
            None => Err(Unsupported("no section".into())),
            Some(g) => self.table_flow(table, &g, previous, labels),
        };
        match flowed {
            Ok(flow) => FlowItem::Table(Arc::new(TableItem {
                block,
                body,
                section,
                section_start,
                flow,
                notes: self.cell_notes(block, body, section),
            })),
            Err(why) => FlowItem::Stop(Arc::new(StopItem {
                block,
                body,
                section,
                reason: why.to_string(),
            })),
        }
    }

    /// How many paragraphs top-level block `body` holds (a table's cells').
    fn paragraphs_in(&self, body: usize) -> usize {
        let first = self.firsts.get(body).copied().unwrap_or(0);
        let next = self.firsts.get(body + 1).copied().unwrap_or(self.paragraphs.len());
        next.saturating_sub(first)
    }

    /// A table's flow, kept by its `Arc` while its section, the paragraph before it and
    /// its cells' labels are the same.
    fn table_flow(
        &mut self,
        table: &Arc<Table>,
        geometry: &crate::sections::SectionGeometry,
        previous: Option<Arc<Paragraph>>,
        labels: Vec<Option<String>>,
    ) -> Result<Arc<dyn TableFlow>, Unsupported> {
        let key = Arc::as_ptr(table) as usize;
        if let Some(kept) = self.state.cache.tables.get(&key) {
            let same = Arc::ptr_eq(&kept.table, table)
                && *kept.section == *geometry.props
                && match (&kept.previous, &previous) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
                && kept.labels == labels;
            if same {
                return Ok(Arc::clone(&kept.flow));
            }
        }
        let mut cx = TableContext {
            document: self.cx.document,
            model: &mut *self.model,
            measurer: &mut self.state.measurer,
            faces: self.cx.faces,
            locale: self.cx.locale,
        };
        let job = TableJob {
            table,
            section: geometry,
            previous: previous.as_deref(),
            labels: &labels,
        };
        let flow = self.cx.tables.flow_table(&mut cx, &job)?;
        self.state.cache.tables.insert(
            key,
            KeptTable {
                table: Arc::clone(table),
                section: Arc::clone(&geometry.props),
                previous,
                labels,
                flow: Arc::clone(&flow),
            },
        );
        Ok(flow)
    }

    fn measured(
        &mut self,
        paragraph: &Arc<Paragraph>,
        body: usize,
        section: usize,
    ) -> Result<Arc<MeasuredParagraph>, Unmeasured> {
        let document = self.cx.document;
        let label = self.cx.labels.of(self.firsts.get(body).copied().unwrap_or(0));
        let geometry = self.cx.sections.get(section).cloned();
        let Some(geometry) = geometry else {
            return Err(Unmeasured::Text("no section".into()));
        };
        if let Some(found) = self.state.cache.get(paragraph, &geometry.props, label.as_deref()) {
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
        };
        let measured = measure_paragraph(&mcx, &mut self.state.measurer, &job).map(Arc::new);
        self.state.cache.blocks.insert(
            Arc::as_ptr(paragraph) as usize,
            Kept {
                paragraph: Arc::clone(paragraph),
                section: Arc::clone(&geometry.props),
                label,
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
        if let Some(placeholder) = Placeholder::of(paragraph).filter(|_| self.carry_on) {
            return self.placeholder_item(paragraph, (body, block, section, section_start), placeholder);
        }
        let measured = match self.measured(paragraph, body, section) {
            Ok(m) => m,
            Err(why) => return stop(block, body, section, format!("{why:?}")),
        };
        let Some(pp) = self.props_of_block(block) else {
            return stop(block, body, section, "unresolved");
        };
        let (mut before, after) = self.spacing(&pp, block);
        let role = self.cx.roles.get(body).copied().flatten();
        if role == Some(NoteRole::Separator) && crate::input::rules(self.cx.document).mode15() {
            // The endnotes' separator keeps no space before in mode 15.
            before = 0;
        }
        let ParaNotes {
            notes,
            continuation,
            forms,
        } = match self.paragraph_notes(paragraph, &measured, section, role) {
            Ok(found) => found,
            Err(note) => return stop(block, body, section, format!("footnote {} cannot be laid out", note.0)),
        };
        let previous = self.neighbour_props(body, -1);
        let following = self.neighbour_props(body, 1);
        let (top_border, border) = paragraph_borders(&pp, previous.as_deref(), following.as_deref());
        let section_mark =
            self.cx.sections.ends_section(top_level) && is_empty(paragraph) && measured.heights.len() == 1;
        let anchors = anchors_of(paragraph, &measured);
        let source = self.para_source(paragraph, &measured, section);
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
            notes,
            role,
            continuation,
            anchors,
            source,
            drop_cap: self.pending_drop.take(),
            note_forms: forms,
            placeholder: None,
        }))
    }
}
