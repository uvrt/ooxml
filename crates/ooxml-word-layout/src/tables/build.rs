//! Owns: a table measured ([`lay_table`], docx2svg's `table.flow_table`): its widths
//! settled (measured content where Word sizes by it, else its grid), each cell's blocks
//! measured at its width -- paragraphs through the paragraph pipeline, nested tables as
//! tables of their own ([`nested_entry`]) -- each row's edges and height, and merges down
//! made tall enough for their cells. With `carry_on` the approximate rules lay out what
//! the measured ones refuse (`flow_table`'s `carry_on`), each recorded.
//!
//! Must not: place rows on pages (`super::place`), or lay out floating tables, floating
//! drawings text wraps around in a cell, or notes (lane L3c): those are refused.

use super::anchors::{self, wrap_cell, CellEdges, CellJob, CellPage};
use super::cells::{cell_stack, Align, CellBlock, CellDrawings, CellFlow, Content, Merge};
use super::content::{cell_extent, column_minimums, paragraph_minimum};
use super::floating::{floats, Floating, Standing, UNBORDERED_EDGE};
use super::grid::{cell_box, cell_width, grid_lines, left_offset, line_at, right_offset, CellBox, TableSpec};
use super::measured::{Approximated, MeasuredTable};
use super::rows::{edge_width, row_flow, settle_rows};
use super::widths::{autofit_room, autofit_unmodelled, Extents};
use crate::flow::{is_empty, paragraph_borders};
use crate::floating::anchors_of;
use crate::input::geometry;
use crate::rebreak::ParaSource;
use crate::paragraph::{measure_paragraph, MeasureContext, MeasuredParagraph, ParagraphJob, Unmeasured};
use crate::px::Px;
use crate::table::FloatingTable;
use crate::table::{TableContext, Unsupported};
use ooxml_base::Q;
use ooxml_word_model::model::{Block, Paragraph, Table};
use ooxml_word_model::resolve::{border_twips, drawn, ResolvedCell, ResolvedTable};
use ooxml_word_model::styles::StyleKind;
use ooxml_word_model::value::{key, Props, Text, Value};
use ooxml_word_model::WordDocument;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Where a table is laid out across: its column's left edge from the page's, and the
/// column's width, twips (exact: a nested table's column is its cell's text area).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// The column's left edge, from the page's.
    pub page_left: Q,
    /// The column's width.
    pub column_width: Q,
    /// The page's width (a floating table positioned against the page is laid out across
    /// it).
    pub page_width: Q,
    /// The page its cells' drawings are positioned on.
    pub page: CellPage,
}

/// One table to lay out.
pub struct TableInput<'a> {
    /// The table.
    pub table: &'a Table,
    /// Where it is laid out across.
    pub frame: Frame,
    /// The style of the paragraph before it (the one that applies).
    pub previous_style: Option<Text>,
    /// Its paragraphs' list labels from its first, in document order.
    pub labels: &'a [Option<String>],
    /// Nested in a cell.
    pub nested: bool,
}

fn unsupported(why: &str) -> Unsupported {
    Unsupported(why.to_owned())
}

fn int(x: i64) -> Q {
    Q::from_integer(i128::from(x))
}

pub(super) fn flag(props: &Props, name: &str) -> bool {
    props.get(name).is_some_and(Value::truthy)
}

fn number(props: &Props, name: &str) -> i64 {
    props.get(name).and_then(Value::int).unwrap_or(0)
}

/// The paragraph style that applies to `paragraph`: the named one, else the default
/// (`_paragraph_style_id`).
pub fn effective_style(document: &WordDocument, paragraph: &Paragraph) -> Option<Text> {
    let sheet = document.context.styles.as_deref()?;
    let declared = paragraph.properties.as_ref().and_then(|p| p.style_id.as_deref());
    if let Some(id) = declared.filter(|id| sheet.get(id).is_some_and(|s| s.kind == StyleKind::Paragraph)) {
        return Some(key(id));
    }
    sheet.default_of(StyleKind::Paragraph).map(|s| s.style_id.clone())
}

/// The default paragraph style's id.
fn default_style(document: &WordDocument) -> Option<Text> {
    let sheet = document.context.styles.as_deref()?;
    sheet.default_of(StyleKind::Paragraph).map(|s| s.style_id.clone())
}

/// How many paragraphs a block holds (a nested table's, all of them).
pub fn paragraphs_in(block: &Block) -> usize {
    match block {
        Block::Paragraph(_) => 1,
        Block::Table(t) => table_paragraphs(t),
    }
}

/// How many paragraphs a table holds, nested tables' included.
pub fn table_paragraphs(table: &Table) -> usize {
    table
        .rows
        .iter()
        .flat_map(|row| row.iter())
        .flat_map(|cell| cell.iter())
        .map(paragraphs_in)
        .sum()
}

/// What laying one table out keeps while it goes.
struct Laying<'i, 'x, 'a, 'm> {
    cx: &'x mut TableContext<'a, 'm>,
    input: &'i TableInput<'i>,
    /// Where it is laid out across: the input's, or the page for a floating table
    /// positioned against it.
    frame: Frame,
    /// Where it is positioned, if it floats.
    floating: Option<Floating>,
    carry_on: bool,
    spec: TableSpec,
    approximated: Vec<Approximated>,
    default_style: Option<Text>,
    autofit: bool,
}

/// Lay a table out (`flow_table`); `carry_on`: the approximate rules lay out what the
/// measured ones refuse.
pub fn lay_table(
    cx: &mut TableContext<'_, '_>,
    input: &TableInput<'_>,
    carry_on: bool,
) -> Result<MeasuredTable, Unsupported> {
    let resolved = cx.model.resolve_table(cx.document, input.table);
    let mode15 = crate::input::rules(cx.document).mode15();
    let default_style = default_style(cx.document);
    let mut laying = Laying {
        spec: TableSpec::new(resolved, input.nested, mode15),
        frame: input.frame,
        floating: None,
        cx,
        input,
        carry_on,
        approximated: Vec::new(),
        default_style,
        autofit: false,
    };
    laying.standing()?;
    let lines = laying.widths()?;
    laying.measure(lines)
}

impl Laying<'_, '_, '_, '_> {
    fn approximate(&mut self, reason: &'static str, message: String, path: &Text) {
        self.approximated.push(Approximated {
            reason,
            message,
            path: path.clone(),
        });
    }

    fn table_path(&self) -> Text {
        self.input.table.path.clone()
    }

    /// Whether it floats: a floating table is laid out in its frame as a table with its
    /// indent, or its alignment, is in the column (`floating_of`); one nested in a cell is
    /// not measured; one positioned as no measurement covers is laid out in the flow with
    /// `carry_on`.
    fn standing(&mut self) -> Result<(), Unsupported> {
        match floats(&self.input.table.properties, &self.spec.properties, self.spec.mode15) {
            Ok(Standing::InFlow) => Ok(()),
            Ok(Standing::Floating(_)) if self.input.nested => {
                Err(unsupported("a floating table nested in a cell (not measured)"))
            }
            Ok(Standing::Floating(floating)) => {
                if floating.page {
                    self.frame = Frame {
                        page_left: Q::from_integer(0),
                        column_width: self.frame.page_width,
                        ..self.frame
                    };
                }
                let indent = floating.x - i64::from(floating.x > 0);
                let props = &mut self.spec.properties;
                props.insert(std::borrow::Cow::Borrowed("tblInd"), Value::Width(indent, key("dxa")));
                let jc = floating.x_spec.as_deref().unwrap_or("left");
                props.insert(std::borrow::Cow::Borrowed("jc"), Value::Str(key(jc)));
                self.spec.indent_stated = true;
                self.floating = Some(floating);
                Ok(())
            }
            Err(why) if self.carry_on => {
                let path = self.table_path();
                self.approximate(
                    "table",
                    format!("{}: laid out in the flow, where it stands", why.0),
                    &path,
                );
                self.spec.properties.remove("tblpPr");
                Ok(())
            }
            Err(why) => Err(why),
        }
    }

    fn on_grid(&mut self) {
        let path = self.table_path();
        self.approximate(
            "table",
            "its columns are laid out on its w:tblGrid widths, as a fixed table's (Word writes the grid it last laid \
             the table out on)"
                .into(),
            &path,
        );
        self.spec.on_grid();
    }

    /// Falls back to the grid where `carry_on` allows, else refuses.
    fn fall_back(&mut self, why: Unsupported) -> Result<(), Unsupported> {
        if !self.carry_on || self.input.table.grid.is_empty() {
            return Err(why);
        }
        self.on_grid();
        Ok(())
    }

    /// The first row's cells' width types.
    fn first_kinds(&self) -> BTreeSet<Option<String>> {
        self.spec
            .rows()
            .first()
            .map(|r| {
                r.cells
                    .iter()
                    .map(|c| cell_width(c).map(|(_, k)| k.to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn extents(&mut self) -> Result<Extents, Unsupported> {
        let mode15 = self.spec.mode15;
        let resolved = Arc::clone(&self.spec.resolved);
        let mut out = HashMap::new();
        for cell in resolved.rows.iter().flat_map(|r| &r.cells) {
            if cell.merge.as_deref() == Some("continue") {
                continue;
            }
            let found = cell_extent(self.cx, self.input.table, cell, mode15)?;
            out.insert((cell.row, cell.index), found);
        }
        Ok(out)
    }

    /// The widths settled and the grid lines, twips from the column's left edge.
    fn widths(&mut self) -> Result<Vec<Q>, Unsupported> {
        let frame = self.frame;
        let first_dxa =
            self.spec.rows().first().is_some_and(|r| {
                !r.cells.is_empty() && r.cells.iter().all(|c| matches!(cell_width(c), Some((_, "dxa"))))
            });
        if self.spec.in_percent() && first_dxa {
            match column_minimums(self.cx, &self.spec, self.input.table) {
                Ok(m) => self.spec.minimums = Some(m),
                Err(why) => self.fall_back(why)?,
            }
        }
        self.autofit = !self.spec.fixed();
        let kinds = self.first_kinds();
        let one_kind = |k: &str| kinds.len() == 1 && kinds.contains(&Some(k.to_owned()));
        if self.autofit && !(self.spec.in_percent() && (one_kind("pct") || one_kind("dxa"))) {
            match self.extents() {
                Ok(found) => self.spec.extents = Some(found),
                Err(why) => {
                    if (self.spec.in_percent() || autofit_unmodelled(&self.spec).is_some()) && !self.carry_on {
                        return Err(why);
                    }
                    self.spec.extents = None;
                }
            }
        }
        let checked = grid_lines(&self.spec, frame.column_width).and_then(|lines| {
            match autofit_unmodelled(&self.spec).filter(|_| self.autofit && self.spec.extents.is_none()) {
                Some(why) => Err(unsupported(why)),
                None => Ok(lines),
            }
        });
        let lines = match checked {
            Ok(lines) => lines,
            Err(why) => {
                self.fall_back(why)?;
                self.autofit = false;
                grid_lines(&self.spec, frame.column_width)?
            }
        };
        let widened = self.spec.resolved.grid.len() > self.input.table.grid.len()
            && int(self.spec.resolved.grid.iter().sum()) > autofit_room(&self.spec, frame.column_width);
        if self.autofit && self.spec.extents.is_some() && widened {
            let path = self.table_path();
            self.approximate(
                "table",
                "an autofit table with a row past its w:tblGrid, wider than the room, is sized from its content on \
                 the widened grid (not measured)"
                    .into(),
                &path,
            );
        }
        Ok(lines)
    }

    /// Each cell's blocks with their index among the table's paragraphs.
    fn entries_of(&self) -> HashMap<(usize, usize), Vec<(usize, Block)>> {
        let mut out: HashMap<(usize, usize), Vec<(usize, Block)>> = HashMap::new();
        let mut index = 0;
        for (r, row) in self.input.table.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                for block in cell.iter() {
                    out.entry((r, c)).or_default().push((index, block.clone()));
                    index += paragraphs_in(block);
                }
            }
        }
        out
    }

    fn measure(mut self, lines: Vec<Q>) -> Result<MeasuredTable, Unsupported> {
        let mut entries = self.entries_of();
        let resolved = Arc::clone(&self.spec.resolved);
        let mut rows = Vec::with_capacity(resolved.rows.len());
        for row in &resolved.rows {
            let mut cells = Vec::with_capacity(row.cells.len());
            for cell in &row.cells {
                let items = if cell.merge.as_deref() == Some("continue") {
                    Vec::new()
                } else {
                    entries.remove(&(row.index, cell.index)).unwrap_or_default()
                };
                cells.push(self.cell(cell, &lines, &items)?);
            }
            rows.push(row_flow(row, cells));
        }
        let merges = settle_rows(&mut rows);
        let bottom_border = rows.last().map_or(Px::ZERO, |r| edge_width(&r.cells_borders(), &[]));
        let keeps = rows.iter().map(MeasuredTable::leads_kept).collect();
        let header_rows = rows.iter().take_while(|r| r.header).count();
        let mut table = MeasuredTable {
            path: self.table_path(),
            rows,
            bottom_border,
            grid: lines,
            page_left: self.frame.page_left,
            mode15: self.spec.mode15,
            header_rows,
            keeps,
            merges,
            approximation: None,
            approximated: std::mem::take(&mut self.approximated),
            measures: Vec::new(),
            extras: Vec::new(),
            floating: None,
            fit_right: Q::default(),
        };
        table.fit_right = fit_right(&table, &resolved);
        if let Some(position) = self.floating.take() {
            table.floating = Some(floating_box(position, &table, &resolved));
        }
        table.measures = measures_of(&table);
        table.extras = super::drawn::extras_of(&table);
        Ok(table)
    }

    fn cell(&mut self, cell: &ResolvedCell, lines: &[Q], items: &[(usize, Block)]) -> Result<CellFlow, Unsupported> {
        let frame = self.frame;
        let cell_box = cell_box(&self.spec, frame.page_left, lines, cell);
        let document = self.cx.document;
        let resolved: Vec<Option<Arc<Props>>> = items
            .iter()
            .map(|(_, b)| match b {
                Block::Paragraph(p) => Some(self.cx.model.resolve_paragraph_props(document, p)),
                Block::Table(_) => None,
            })
            .collect();
        let align = match cell.properties.get("vAlign").and_then(Value::str) {
            Some("center") => Align::Center,
            Some("bottom") => Align::Bottom,
            _ => Align::Top,
        };
        let mut placed: Vec<CellBlock> = Vec::with_capacity(items.len());
        let mut sources: Vec<Option<ParaSource>> = Vec::with_capacity(items.len());
        for (k, (offset, block)) in items.iter().enumerate() {
            let (entry, source) = match block {
                Block::Table(nested) => {
                    let previous = k
                        .checked_sub(1)
                        .and_then(|j| items.get(j))
                        .and_then(|(_, b)| match b {
                            Block::Paragraph(p) => Some(effective_style(document, p)),
                            Block::Table(_) => None,
                        })
                        .flatten();
                    (self.nested_entry(nested, cell, lines, *offset, previous)?, None)
                }
                Block::Paragraph(p) => {
                    let near = |j: Option<usize>| j.and_then(|j| resolved.get(j)).cloned().flatten();
                    let neighbours = (near(k.checked_sub(1)), near(Some(k + 1)));
                    let job = CellParagraph {
                        paragraph: p,
                        offset: *offset,
                        cell_box: &cell_box,
                    };
                    let (entry, source) =
                        self.paragraph_entry(&job, (neighbours.0.as_deref(), neighbours.1.as_deref()))?;
                    (entry, Some(source))
                }
            };
            placed.push(entry);
            sources.push(source);
        }
        hide_marks(cell, items, &mut placed);
        let mut loose = Vec::new();
        let reach = self.around_drawings(cell, (lines, &cell_box), (&mut placed, &sources), &mut loose, align)?;
        let previous_style = self.input.previous_style.as_ref();
        let (stack, after) = cell_stack(&placed, previous_style, self.default_style.as_ref());
        let mut content = stack.iter().map(|l| l.height()).sum::<Px>() + after;
        if let Some(reach) = reach {
            // The row holds a drawing text wraps around in it (`make_cell_anchor_probe.py`).
            content = content.max(reach - Px::twips(cell.margins[0]));
        }
        let merge = match cell.merge.as_deref() {
            Some("restart") => Merge::Restart,
            Some("continue") => Merge::Continue,
            _ => Merge::None,
        };
        Ok(CellFlow {
            resolved: Arc::new(cell.clone()),
            column: cell.column,
            border_top: side_twips(cell, 0),
            border_bottom: side_twips(cell, 2),
            margin_top: Px::twips(cell.margins[0]),
            margin_bottom: Px::twips(cell.margins[2]),
            merge,
            merge_stated: cell.merge.as_deref().is_some_and(|m| !m.is_empty()),
            align,
            cell_box,
            entries: placed,
            content,
            rows: 1,
            span_height: None,
            stack,
            after,
            loose,
        })
    }

    /// A cell laid out around the drawings anchored in it (`wrap_cell`): how far down the
    /// lowest one text wraps around reaches, and in a cell aligned vertically, where those
    /// it does not wrap around are (`loose`).
    fn around_drawings(
        &mut self,
        cell: &ResolvedCell,
        (lines, cell_box): (&[Q], &CellBox),
        (placed, sources): (&mut [CellBlock], &[Option<ParaSource>]),
        loose: &mut Vec<(Px, Px)>,
        align: Align,
    ) -> Result<Option<Px>, Unsupported> {
        if placed.iter().all(|e| e.drawings.anchors.is_empty()) {
            return Ok(None);
        }
        let job = CellJob {
            document: self.cx.document,
            faces: self.cx.faces,
            locale: self.cx.locale,
            cell,
            cell_box,
            edges: CellEdges::of(cell, self.frame.page_left, lines),
            page: self.frame.page,
            styles: (self.input.previous_style.as_ref(), self.default_style.as_ref()),
            sources,
            mode15: self.spec.mode15,
        };
        wrap_cell(&job, placed, (align != Align::Top).then_some(loose))
    }

    /// A table nested in `cell`: one line as tall as the table, laid out in the cell's
    /// text area as a mode-15 table is in its column (`nested_entry`).
    fn nested_entry(
        &mut self,
        table: &Table,
        cell: &ResolvedCell,
        lines: &[Q],
        offset: usize,
        previous_style: Option<Text>,
    ) -> Result<CellBlock, Unsupported> {
        let page_left = self.frame.page_left;
        let text_left = page_left + line_at(lines, cell.column) + int(left_offset(cell));
        let text_right = page_left + line_at(lines, cell.column + cell.span) - int(right_offset(cell));
        let count = table_paragraphs(table);
        let labels = self.input.labels.get(offset..offset + count).unwrap_or_default();
        let input = TableInput {
            table,
            frame: Frame {
                page_left: text_left,
                column_width: text_right - text_left,
                ..self.frame
            },
            previous_style,
            labels,
            nested: true,
        };
        let flow = lay_table(self.cx, &input, self.carry_on)?;
        self.approximated.extend(flow.approximated.iter().cloned());
        let height = flow.whole_height();
        Ok(CellBlock {
            block: offset,
            content: Content::Table(Arc::new(flow)),
            pitches: vec![height],
            before: 0,
            after: 0,
            top_border: Px::ZERO,
            border: Px::ZERO,
            contextual: false,
            style: None,
            effective_style: None,
            hidden: false,
            keep_next: false,
            widow: None,
            drawings: CellDrawings::default(),
        })
    }

    /// One paragraph of a cell, measured at the cell's width.
    fn paragraph_entry(
        &mut self,
        job: &CellParagraph<'_>,
        neighbours: (Option<&Props>, Option<&Props>),
    ) -> Result<(CellBlock, ParaSource), Unsupported> {
        let document = self.cx.document;
        let paragraph = job.paragraph;
        let frame = paragraph
            .properties
            .as_ref()
            .is_some_and(|p| p.declared.contains_key("framePr"));
        if frame {
            if !self.carry_on {
                return Err(unsupported("a frame in a cell"));
            }
            self.approximate(
                "frame",
                "a frame (w:framePr) in a cell is laid out as a paragraph of the cell".into(),
                &paragraph.path,
            );
        }
        refuse_unlaid(paragraph)?;
        let paragraph = &self.cell_drawings_laid(paragraph)?;
        let resolved = self.cx.model.resolve_paragraph(document, paragraph);
        let pp = Arc::clone(&resolved.paragraph);
        let width = job.cell_box.budget.max(1);
        let text_geometry = geometry(document, &pp, width);
        let label = self.input.labels.get(job.offset).cloned().flatten();
        let mcx = MeasureContext {
            document,
            faces: self.cx.faces,
            locale: self.cx.locale,
        };
        let measured = measure_paragraph(
            &mcx,
            self.cx.measurer,
            &ParagraphJob {
                paragraph,
                resolved: &resolved,
                geometry: &text_geometry,
                columns: false,
                label: label.as_deref(),
            },
        )
        .map_err(|why| match why {
            Unmeasured::Text(_) => unsupported("a cell paragraph that cannot be measured"),
            Unmeasured::NoFaceMetrics => unsupported("no face metrics in a cell"),
        })?;
        self.word_wider(paragraph, &measured, job.cell_box.budget)?;
        let (top_border, border) = paragraph_borders(&pp, neighbours.0, neighbours.1);
        let drawings = CellDrawings {
            anchors: anchors_of(paragraph, &measured),
            approximate: anchors::approximate(paragraph, self.spec.mode15),
            ..CellDrawings::default()
        };
        let source = ParaSource {
            paragraph: Arc::clone(paragraph),
            resolved: resolved.clone(),
            geometry: Arc::unwrap_or_clone(Arc::clone(&measured.geometry)),
            columns: false,
            first_column: None,
            notes: Vec::new(),
        };
        let entry = CellBlock {
            block: job.offset,
            pitches: measured.heights.iter().map(|h| h.pitch).collect(),
            content: Content::Paragraph(Arc::new(measured)),
            before: number(&pp, "spacing.before"),
            after: number(&pp, "spacing.after"),
            top_border,
            border,
            contextual: flag(&pp, "contextualSpacing"),
            style: paragraph.properties.as_ref().and_then(|p| p.style_id.clone()),
            effective_style: effective_style(document, paragraph),
            hidden: false,
            keep_next: flag(&pp, "keepNext"),
            widow: pp.get("widowControl").map(Value::truthy),
            drawings,
        };
        Ok((entry, source))
    }

    /// A cell paragraph as its floating drawings let it be laid out: where they wrap
    /// text in a way not measured in a cell, refused, or with `carry_on` made drawings
    /// text does not wrap around, said (`cell_anchors_unmodelled`, `unwrapped`).
    fn cell_drawings_laid(&mut self, paragraph: &Paragraph) -> Result<Arc<Paragraph>, Unsupported> {
        let Some(why) = anchors::unmodelled(paragraph, self.spec.mode15) else {
            return Ok(Arc::new(paragraph.clone()));
        };
        if !self.carry_on {
            return Err(Unsupported(why));
        }
        self.approximate(
            "cell-drawing",
            format!("{why}: laid out as if text did not wrap around it"),
            &paragraph.path,
        );
        Ok(Arc::new(anchors::unwrapped(paragraph)))
    }

    /// A word wider than its column in an autofit table Word would widen the column for
    /// (not modelled): refused, or broken inside it with `carry_on`.
    fn word_wider(
        &mut self,
        paragraph: &Paragraph,
        measured: &MeasuredParagraph,
        budget: i64,
    ) -> Result<(), Unsupported> {
        let emergency = measured.measured.lines.iter().any(|l| l.emergency);
        if !(self.autofit && emergency && self.spec.extents.is_none()) {
            return Ok(());
        }
        if paragraph_minimum(self.cx, paragraph)? <= int(budget) {
            return Ok(());
        }
        if !self.carry_on {
            return Err(unsupported("autofit: a word wider than its column"));
        }
        self.approximate(
            "table",
            "a word wider than its column, which Word would widen the column for, is broken inside it".into(),
            &paragraph.path,
        );
        Ok(())
    }
}

/// One paragraph of a cell to measure.
struct CellParagraph<'a> {
    paragraph: &'a Paragraph,
    offset: usize,
    cell_box: &'a CellBox,
}

/// A drawing in a cell this layout cannot read (a VML shape it cannot read, an embedded
/// object): refused (docx2svg draws a placeholder of its estimated height).
fn refuse_unlaid(paragraph: &Paragraph) -> Result<(), Unsupported> {
    if paragraph
        .runs
        .iter()
        .any(|r| r.breaks.iter().any(|(_, k)| &**k == "drawing"))
    {
        return Err(unsupported("a drawing in a cell"));
    }
    Ok(())
}

/// A cell's last paragraph that takes no room: under `w:hideMark` an empty one
/// (`hides_mark`), and an empty one after a nested table.
fn hide_marks(cell: &ResolvedCell, items: &[(usize, Block)], placed: &mut [CellBlock]) {
    let count = placed.len();
    let last_paragraph = match items.last() {
        Some((_, Block::Paragraph(p))) => Some(p),
        _ => None,
    };
    if let (Some(p), Some(last)) = (last_paragraph, placed.last_mut()) {
        if flag(&cell.properties, "hideMark") && is_empty(p) {
            last.hidden = true;
        }
    }
    let after_table = count > 1 && matches!(placed.get(count - 2).map(|b| &b.content), Some(Content::Table(_)));
    if let (true, Some(last)) = (after_table, placed.last_mut()) {
        let empty = match &last.content {
            Content::Paragraph(m) => m.measured.pieces.is_empty(),
            Content::Table(_) => false,
        };
        if empty {
            last.hidden = true;
        }
    }
}

/// Its paragraphs' measures by block, nested tables' included.
fn measures_of(table: &MeasuredTable) -> Vec<Option<Arc<MeasuredParagraph>>> {
    let mut out: Vec<Option<Arc<MeasuredParagraph>>> = Vec::new();
    fn put(out: &mut Vec<Option<Arc<MeasuredParagraph>>>, block: usize, m: &Arc<MeasuredParagraph>) {
        if out.len() <= block {
            out.resize(block + 1, None);
        }
        if let Some(slot) = out.get_mut(block) {
            *slot = Some(Arc::clone(m));
        }
    }
    for entry in table.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.entries) {
        match &entry.content {
            Content::Paragraph(m) => put(&mut out, entry.block, m),
            Content::Table(nested) => {
                for (k, m) in nested.measures.iter().enumerate() {
                    if let Some(m) = m {
                        put(&mut out, entry.block + k, m);
                    }
                }
            }
        }
    }
    out
}

/// How far right of its column's left edge a table reaches as a room measures it
/// (`table_fits`).
fn fit_right(table: &MeasuredTable, resolved: &ResolvedTable) -> Q {
    let right = table.grid.last().copied().unwrap_or_default();
    let Some(last) = resolved.rows.first().and_then(|r| r.cells.last()) else {
        return right;
    };
    if table.mode15 {
        right + int(super::grid::outer_part(last.right_edge.0.as_ref()))
    } else {
        right - int(super::grid::right_offset(last))
    }
}

/// A floating table as text goes beside it (`table_wrap`'s box, `floating_edges`): its
/// outer grid lines in exact px from the page's left edge, and its outer borders' outer
/// halves past them, or `UNBORDERED_EDGE` twips where there is none.
fn floating_box(position: Floating, table: &MeasuredTable, resolved: &ResolvedTable) -> FloatingTable {
    let px = Q::new(300, 1440);
    let line = |q: Option<&Q>| (table.page_left + q.copied().unwrap_or_default()) * px;
    let edge = |border: Option<&ooxml_word_model::value::Border>| {
        if drawn(border) {
            Q::new(i128::from(border_twips(border)), 2)
        } else {
            int(UNBORDERED_EDGE)
        }
    };
    let cells = resolved.rows.first().map(|r| r.cells.as_slice()).unwrap_or_default();
    let left = cells.first().map_or_else(Q::default, |c| edge(c.left_edge.0.as_ref()));
    let right = cells.last().map_or_else(Q::default, |c| edge(c.right_edge.0.as_ref()));
    FloatingTable {
        position,
        first: line(table.grid.first()),
        last: line(table.grid.last()),
        edges: (left, right),
    }
}

/// Twips of a border side of a resolved cell.
pub fn side_twips(cell: &ResolvedCell, side: usize) -> i64 {
    border_twips(cell.borders.get(side).and_then(Option::as_ref))
}
