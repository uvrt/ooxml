//! The pagination kernel over compact arrays: docx2svg's `paginate.paginate` / `_scan` /
//! `_keeps` / `_break_before` and `table.place_table`, for one-column sections without
//! notes, floating drawings or tables, drop caps, merged cells or section-break paragraphs
//! (the exporter refuses those).  px over one denominator, exact.

use crate::measure::LineH;
use crate::model::{Export, ItemIn, SectionIn};
use crate::units::Px;

#[derive(Clone, Copy, Debug)]
pub struct LineBox {
    pub pitch: i64,
    /// The pitch less an `auto` multiple's extra: what must fit (`paginate.fits`, `text-border`).
    pub fit: i64,
}

impl From<&LineH> for LineBox {
    fn from(h: &LineH) -> LineBox {
        LineBox {
            pitch: h.pitch,
            fit: h.fit(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FPara {
    pub block: u32,
    pub section: u32,
    pub lines: Vec<LineBox>,
    pub page_breaks: Vec<u32>,
    pub before: i64,
    pub after: i64,
    pub style: i32,
    pub contextual: bool,
    pub border: i64,
    pub top_border: i64,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub pbb: bool,
    pub widow: bool,
    pub section_start: bool,
}

#[derive(Clone, Debug)]
pub struct Stk {
    /// above + pitch + below
    pub h: i64,
    /// Lane E: the space above the line (its paragraph's gap and top border, first line) and
    /// its pitch: where the line is drawn in the cell.
    pub above: i64,
    pub pitch: i64,
    /// Lane E: the cell paragraph the line is of.
    pub para: u32,
    pub number: u32,
    pub para_lines: u32,
    pub widow: bool,
}

#[derive(Clone, Debug)]
pub struct FCell {
    pub margins: i64,
    /// Lane E: the top margin alone (where the cell's lines start).
    pub margin_top: i64,
    pub after: i64,
    pub stack: Vec<Stk>,
}

#[derive(Clone, Debug)]
pub struct FRow {
    pub top_border: i64,
    pub height: i64,
    pub header: bool,
    pub cant_split: bool,
    pub keeps_next: bool,
    pub cells: Vec<FCell>,
}

#[derive(Clone, Debug)]
pub struct FTable {
    pub block: u32,
    pub section: u32,
    pub section_start: bool,
    pub bottom_border: i64,
    pub first_style: i32,
    pub header_count: u32,
    pub rows: Vec<FRow>,
}

/// Items are shared between snapshots: a new flow is the old one's pointers with the changed
/// items replaced.
pub type It = std::sync::Arc<FItem>;

#[derive(Clone, Debug)]
pub enum FItem {
    P(FPara),
    T(FTable),
}

impl FItem {
    pub fn section(&self) -> u32 {
        match self {
            FItem::P(p) => p.section,
            FItem::T(t) => t.section,
        }
    }
    pub fn block(&self) -> u32 {
        match self {
            FItem::P(p) => p.block,
            FItem::T(t) => t.block,
        }
    }
    pub fn count(&self) -> u32 {
        match self {
            FItem::P(p) => p.lines.len() as u32,
            FItem::T(t) => t.rows.len() as u32,
        }
    }
}

/// A page's edges per story kind: (top, bottom), px.
#[derive(Clone, Debug)]
pub struct SectionGeo {
    pub kinds: [(i64, i64); 3], // default, first, even
    pub title_page: bool,
    pub new_page: bool,
    pub start_type: Option<String>,
    pub page_number_start: Option<i64>,
}

pub const DEFAULT: u8 = 0;
pub const FIRST: u8 = 1;
pub const EVEN: u8 = 2;

#[derive(Clone, Debug)]
pub struct Doc {
    pub px: Px,
    pub mode15: bool,
    pub even_odd: bool,
    pub sections: Vec<SectionGeo>,
}

pub fn section_geo(px: &Px, s: &SectionIn) -> SectionGeo {
    let mut kinds = [(0, 0); 3];
    for (k, name) in ["default", "first", "even"].iter().enumerate() {
        let (header, footer) = s.stories.as_ref().and_then(|m| m.get(*name).copied()).unwrap_or((0, 0));
        let mut top = s.top;
        if !s.top_neg && header != 0 {
            top = top.max(s.header + header);
        }
        let mut bottom = s.bottom;
        if !s.bottom_neg && footer != 0 {
            bottom = bottom.min(s.footer - footer);
        }
        kinds[k] = (top, bottom);
    }
    let _ = px;
    SectionGeo {
        kinds,
        title_page: s.title_page,
        new_page: s.new_page,
        start_type: s.start_type.clone(),
        page_number_start: s.page_number_start,
    }
}

impl Doc {
    pub fn new(e: &Export) -> Doc {
        let px = Px {
            den: e.px_den,
            lu: e.lu,
            dpi: e.dpi,
        };
        Doc {
            px,
            mode15: e.compat.unwrap_or(0) >= 15,
            even_odd: e.even_odd,
            sections: e.sections.iter().map(|s| section_geo(&px, s)).collect(),
        }
    }
    fn story_kind(&self, section: u32, section_first: bool, number: i64) -> u8 {
        if section_first && self.sections[section as usize].title_page {
            FIRST
        } else if self.even_odd && number % 2 == 0 {
            EVEN
        } else {
            DEFAULT
        }
    }
    /// `vertical.paragraph_gap_px`'s gap.
    #[inline]
    fn gap(&self, prev_after: i64, before: i64) -> (i64, i64) {
        let box_before = if before > prev_after {
            self.px.twips(before - prev_after)
        } else {
            0
        };
        (self.px.twips(prev_after) + box_before, box_before)
    }
}

/// The exported flow, with docx2svg's own heights (`use_ref`) or with heights given.
pub fn flow_from_export(e: &Export) -> Vec<It> {
    let px = Px {
        den: e.px_den,
        lu: e.lu,
        dpi: e.dpi,
    };
    e.items
        .iter()
        .map(|it| It::new(item_from_export(&px, it, None)))
        .collect()
}

fn lb(h: &crate::model::HeightIn) -> LineBox {
    let l = LineH {
        pitch: h.0,
        above: h.1,
        below: h.2,
        rule: h.3,
        line: h.4,
        natural: h.5,
    };
    LineBox::from(&l)
}

/// One exported item as the paginator's; `measured` replaces the exported heights (a
/// paragraph's lines, or each cell paragraph's in order) with the Rust measure's.
pub fn item_from_export(px: &Px, it: &ItemIn, measured: Option<&[Vec<LineH>]>) -> FItem {
    match it {
        ItemIn::Para(p) => FItem::P(FPara {
            block: p.block as u32,
            section: p.section as u32,
            lines: match measured {
                Some(m) => m[0].iter().map(LineBox::from).collect(),
                None => p.heights.iter().map(lb).collect(),
            },
            page_breaks: p.page_breaks.iter().map(|v| *v as u32).collect(),
            before: p.before,
            after: p.after,
            style: p.style as i32,
            contextual: p.contextual,
            border: p.border,
            top_border: p.top_border,
            keep_next: p.keep_next,
            keep_lines: p.keep_lines,
            pbb: p.pbb,
            widow: p.widow.unwrap_or(true),
            section_start: p.section_start,
        }),
        ItemIn::Table(t) => {
            let mut k = 0usize;
            let rows: Vec<FRow> = t
                .rows
                .iter()
                .map(|r| {
                    let cells = r
                        .cells
                        .iter()
                        .map(|c| {
                            assert!(c.merge.is_none(), "merged cells not ported");
                            let stack = match measured {
                                None => c
                                    .stack
                                    .iter()
                                    .map(|s| {
                                        let para = &c.paragraphs[s.4];
                                        Stk {
                                            h: s.0 + s.1 + s.2,
                                            above: s.0,
                                            pitch: s.1,
                                            para: s.4 as u32,
                                            number: s.3 as u32,
                                            para_lines: para.heights.len() as u32,
                                            widow: para.widow.unwrap_or(true),
                                        }
                                    })
                                    .collect(),
                                Some(m) => {
                                    let hs = &m[k..k + c.paragraphs.len()];
                                    k += c.paragraphs.len();
                                    crate::table::cell_stack(px, c, hs, t.previous_style, t.default_style)
                                }
                            };
                            let after = match measured {
                                None => c.after,
                                Some(_) => crate::table::cell_after(px, c, t.default_style),
                            };
                            FCell {
                                margins: c.mt + c.mb,
                                margin_top: c.mt,
                                after,
                                stack,
                            }
                        })
                        .collect::<Vec<_>>();
                    let height = match measured {
                        None => r.height,
                        Some(_) => crate::table::row_height(px, r, &cells),
                    };
                    FRow {
                        top_border: r.top_border,
                        height,
                        header: r.header,
                        cant_split: r.cant_split,
                        keeps_next: r.keeps_next,
                        cells,
                    }
                })
                .collect();
            let header_count = rows.iter().take_while(|r| r.header).count() as u32;
            FItem::T(FTable {
                block: t.block as u32,
                section: t.section as u32,
                section_start: t.section_start,
                bottom_border: t.bottom_border,
                first_style: t.first_style as i32,
                header_count,
                rows,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pos {
    pub item: u32,
    pub line: u32,
    pub part: Vec<u32>,
}

impl Pos {
    pub fn at(item: u32, line: u32) -> Pos {
        Pos {
            item,
            line,
            part: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub number: i64,
    pub section: u32,
    pub section_first: bool,
    pub story: u8,
    pub blank: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Pagination {
    pub starts: Vec<Pos>,
    /// Per page laid out (not blank): its page index (blank pages counted), and the number of
    /// the page before it -- the loop's state there (a checkpoint).
    pub page_at: Vec<u32>,
    pub number_before: Vec<i64>,
    /// The furthest position the page's decision read (where it overflowed or was forced).
    pub reach: Vec<Pos>,
    pub info: Vec<Info>,
}

// -- one page ---------------------------------------------------------------------------

struct TablePage {
    end: i64,
    next: Option<(u32, Vec<u32>)>,
    any_body: bool,
    /// `(row, top, height, header)` of each piece placed.
    pieces: Vec<(usize, bool, i64, i64)>,
}

fn slice_height(cell: &FCell, start: usize, end: usize) -> i64 {
    let mut total = 0;
    for s in &cell.stack[start..end] {
        total += s.h;
    }
    if end == cell.stack.len() {
        total += cell.after;
    }
    total
}

fn edge_below(t: &FTable, r: usize) -> i64 {
    if r + 1 < t.rows.len() {
        t.rows[r + 1].top_border
    } else {
        t.bottom_border
    }
}

fn split_counts(row: &FRow, starts: &[u32], room: i64, widow_control: bool) -> Vec<u32> {
    let mut out = Vec::with_capacity(row.cells.len());
    for (cell, &start) in row.cells.iter().zip(starts) {
        let start = start as usize;
        let mut count: i64 = 0;
        let n = cell.stack.len();
        while start + (count as usize) < n
            && cell.margins + slice_height(cell, start, start + count as usize + 1) <= room
        {
            count += 1;
        }
        if widow_control && count > 0 && start + (count as usize) < n {
            let line = &cell.stack[start + count as usize];
            if line.widow {
                let lines = line.para_lines as i64;
                let first = (start as i64 + count) - line.number as i64;
                let mut on_page = start as i64 + count - first.max(start as i64);
                let left = lines - line.number as i64;
                if left < 2 && lines >= 2 {
                    count -= 2 - left;
                    on_page -= 2 - left;
                }
                if 0 < on_page && on_page < 2 && first >= start as i64 {
                    count -= on_page;
                }
            }
        }
        out.push(count.max(0) as u32);
    }
    out
}

/// `table._place_table` without merges.
fn place_table(
    t: &FTable,
    y: i64,
    bottom: i64,
    row: usize,
    part: &[u32],
    first_on_page: bool,
    mode15: bool,
    split_kept: bool,
) -> TablePage {
    place_table_at(t, y, bottom, row, part, first_on_page, mode15, split_kept, None)
}

/// Lane E: each piece's lines per cell, `[start, end)` into the cell's stack.
pub type CellRanges = Vec<(u32, u32)>;

fn whole(row: &FRow, starts: Option<&[u32]>) -> CellRanges {
    row.cells
        .iter()
        .enumerate()
        .map(|(k, c)| (starts.map_or(0, |s| s[k]), c.stack.len() as u32))
        .collect()
}

/// `place_table`, with each piece's cell lines recorded in `ranges` where given (placing a
/// page to read it, not paginating).
fn place_table_at(
    t: &FTable,
    mut y: i64,
    bottom: i64,
    mut row: usize,
    part: &[u32],
    first_on_page: bool,
    mode15: bool,
    split_kept: bool,
    mut ranges: Option<&mut Vec<CellRanges>>,
) -> TablePage {
    let rows = &t.rows;
    let header_count = t.header_count as usize;
    let y0 = y;
    // pieces: (row, header)
    let mut pieces: Vec<(usize, bool, i64, i64)> = Vec::new();
    let mut part: Vec<u32> = part.to_vec();
    if first_on_page && (row > 0 || !part.is_empty()) && header_count > 0 && header_count <= row {
        for h in 0..header_count {
            if let Some(r) = ranges.as_deref_mut() {
                r.push(whole(&rows[h], None));
            }
            pieces.push((h, true, y, rows[h].top_border + rows[h].height));
            y += rows[h].top_border + rows[h].height;
        }
    }
    let any_body = |pieces: &Vec<(usize, bool, i64, i64)>| pieces.iter().any(|p| !p.1);
    macro_rules! leave {
        ($after:expr) => {{
            if mode15
                && header_count > 0
                && !pieces.is_empty()
                && !first_on_page
                && pieces.iter().all(|p| p.0 < header_count && !p.1)
            {
                if let Some(r) = ranges.as_deref_mut() {
                    r.clear();
                }
                return TablePage {
                    end: y0,
                    next: Some((0, Vec::new())),
                    any_body: false,
                    pieces: Vec::new(),
                };
            }
            let end = y + pieces.last().map_or(0, |p| edge_below(t, p.0));
            let any = any_body(&pieces);
            return TablePage {
                end,
                next: Some($after),
                any_body: any,
                pieces,
            };
        }};
    }
    while row < rows.len() {
        let current = &rows[row];
        let continued = !part.is_empty();
        let starts: Vec<u32> = if continued {
            part.clone()
        } else {
            vec![0; current.cells.len()]
        };
        let height = if continued {
            current
                .cells
                .iter()
                .zip(&starts)
                .map(|(c, &s)| c.margins + slice_height(c, s as usize, c.stack.len()))
                .max()
                .unwrap_or(0)
        } else {
            current.height
        };
        let edge = edge_below(t, row);
        let nothing_yet = first_on_page && !any_body(&pieces);
        if y + current.top_border + height + edge <= bottom {
            if let Some(r) = ranges.as_deref_mut() {
                r.push(whole(current, if continued { Some(&starts) } else { None }));
            }
            pieces.push((row, false, y, current.top_border + height));
            y += current.top_border + height;
            row += 1;
            part.clear();
            continue;
        }
        let header = row < header_count && !continued;
        let cant_split = current.cant_split;
        if !(cant_split || header || nothing_yet || split_kept) && current.keeps_next {
            leave!((row as u32, part.clone()));
        }
        if (cant_split || header) && !(nothing_yet && mode15) {
            if nothing_yet {
                if let Some(r) = ranges.as_deref_mut() {
                    r.push(whole(current, if continued { Some(&starts) } else { None }));
                }
                pieces.push((row, false, y, current.top_border + height));
                y += current.top_border + height;
                row += 1;
                part.clear();
                continue;
            }
            leave!((row as u32, part.clone()));
        }
        let room = bottom - edge - y - current.top_border;
        let mut counts = split_counts(current, &starts, room, mode15);
        if counts
            .iter()
            .zip(&starts)
            .zip(&current.cells)
            .any(|((&c, &s), cell)| c == 0 && (s as usize) < cell.stack.len())
        {
            counts.iter_mut().for_each(|c| *c = 0);
        }
        let split_ends: Vec<u32> = starts.iter().zip(&counts).map(|(s, c)| s + c).collect();
        let split_height = current
            .cells
            .iter()
            .zip(starts.iter().zip(&split_ends))
            .map(|(c, (&s, &e))| c.margins + slice_height(c, s as usize, e as usize))
            .max()
            .unwrap_or(0);
        if counts.iter().all(|&c| c == 0) {
            if nothing_yet {
                // CARRY_ON: placed whole past the foot
                if let Some(r) = ranges.as_deref_mut() {
                    r.push(whole(current, if continued { Some(&starts) } else { None }));
                }
                pieces.push((row, false, y, current.top_border + height));
                y += current.top_border + height;
                row += 1;
                let next = if row < rows.len() {
                    Some((row as u32, Vec::new()))
                } else {
                    None
                };
                return TablePage {
                    end: y + edge,
                    next,
                    any_body: true,
                    pieces,
                };
            }
            leave!((row as u32, part.clone()));
        }
        if let Some(r) = ranges.as_deref_mut() {
            r.push(starts.iter().zip(&split_ends).map(|(a, b)| (*a, *b)).collect());
        }
        pieces.push((row, false, y, current.top_border + split_height));
        y += current.top_border + split_height;
        return TablePage {
            end: y + edge,
            next: Some((row as u32, split_ends)),
            any_body: true,
            pieces,
        };
    }
    let any = any_body(&pieces);
    TablePage {
        end: y + t.bottom_border,
        next: None,
        any_body: any,
        pieces,
    }
}

struct Ctx<'a> {
    doc: &'a Doc,
    items: &'a [It],
}

impl<'a> Ctx<'a> {
    fn keeps_moves_line(&self) -> bool {
        self.doc.mode15
    }

    /// `paginate._keeps`.
    fn keeps(&self, start: &Pos, overflow: Pos) -> Pos {
        let index = overflow.item as usize;
        let mut number = overflow.line;
        let first = if index == start.item as usize { start.line } else { 0 };
        let mut target = overflow.clone();
        match &*self.items[index] {
            FItem::T(t) => {
                if !overflow.part.is_empty() {
                    return overflow;
                }
                while number > 0 && t.rows[number as usize - 1].keeps_next {
                    number -= 1;
                }
                if number < overflow.line && 0 < number && number <= t.header_count && self.doc.mode15 {
                    number = 0;
                }
                target = Pos::at(index as u32, number);
            }
            FItem::P(p) => {
                if number > 0 {
                    if p.keep_lines && first == 0 {
                        target = Pos::at(index as u32, 0);
                    } else if p.widow {
                        let count = p.lines.len() as u32;
                        let mut end = count;
                        if self.doc.mode15 {
                            end = p
                                .page_breaks
                                .iter()
                                .filter(|&&k| k >= number)
                                .map(|k| k + 1)
                                .min()
                                .unwrap_or(count);
                        }
                        let below = end as i64 - number as i64;
                        if below < 2 {
                            number = first.max(end.saturating_sub(2));
                        }
                        if first == 0 && 0 < number && number < 2 {
                            number = 0;
                        }
                        target = Pos::at(index as u32, number);
                    }
                }
            }
        }
        if target.line == 0 && target.part.is_empty() && target.item >= 1 && target.item - 1 >= start.item {
            let prev = target.item - 1;
            match &*self.items[prev as usize] {
                FItem::P(p) if p.keep_next => {
                    return if self.keeps_moves_line() {
                        self.keeps(start, Pos::at(prev, p.lines.len() as u32 - 1))
                    } else {
                        self.keeps(start, Pos::at(prev, 0))
                    };
                }
                FItem::T(t) if !t.rows.is_empty() && t.rows.last().unwrap().keeps_next => {
                    return self.keeps(start, Pos::at(prev, t.rows.len() as u32 - 1));
                }
                _ => {}
            }
        }
        target
    }

    /// `paginate._break_before` (one column: `empty_ok` false).
    fn break_before(&self, start: &Pos, overflow: Pos) -> Pos {
        let target = self.keeps(start, overflow.clone());
        if target <= *start {
            if self.doc.mode15
                && matches!(*self.items[overflow.item as usize], FItem::T(_))
                && Pos::at(overflow.item, 0) > *start
            {
                return Pos::at(overflow.item, 0);
            }
            return overflow;
        }
        target
    }

    fn table_gap(&self, previous: Option<usize>, t: &FTable) -> i64 {
        match previous.map(|k| &*self.items[k]) {
            Some(FItem::T(prev)) => {
                if t.rows.is_empty() {
                    0
                } else {
                    -prev.bottom_border.min(t.rows[0].top_border)
                }
            }
            Some(FItem::P(prev)) => {
                if prev.contextual && t.first_style != -2 && t.first_style == prev.style {
                    0
                } else {
                    self.doc.px.twips(prev.after)
                }
            }
            None => 0,
        }
    }

    /// `paginate._scan` for one page: `(end, forced, reach)`.
    fn scan(&self, start: &Pos, story: u8, rec: Option<&mut Vec<Placed>>) -> (Pos, bool, Pos) {
        self.scan_lines(start, story, rec, None)
    }

    /// `scan`, with every line drawn recorded in `lines` where given (lane E).
    fn scan_lines(
        &self,
        start: &Pos,
        story: u8,
        mut rec: Option<&mut Vec<Placed>>,
        mut lines: Option<&mut Vec<PLine>>,
    ) -> (Pos, bool, Pos) {
        let doc = self.doc;
        let items = self.items;
        let first_item = &items[start.item as usize];
        let sec = &doc.sections[first_item.section() as usize];
        let (top, bottom) = sec.kinds[story as usize];
        let mut y = top;
        let mut previous: Option<usize> = None;
        let mut first_on_page = true;
        let mut index = start.item as usize;
        let mut line = start.line;
        while index < items.len() {
            let item = &*items[index];
            let new_page_section = |prev: usize| -> bool {
                items[prev].section() != item.section() && doc.sections[item.section() as usize].new_page
            };
            match item {
                FItem::T(t) => {
                    if !first_on_page && line == 0 && start.part.is_empty() {
                        if let Some(p) = previous {
                            if new_page_section(p) {
                                let at = Pos::at(index as u32, 0);
                                return (at.clone(), true, at);
                            }
                        }
                    }
                    if line == 0 && !first_on_page {
                        y += self.table_gap(previous, t);
                    }
                    let part: &[u32] = if index == start.item as usize { &start.part } else { &[] };
                    let laid = place_table(t, y, bottom, line as usize, part, first_on_page, doc.mode15, false);
                    // `_kept_row_breaks` splits a kept row only below mode 15
                    let laid = if !doc.mode15 && self.kept_row_breaks(t, &laid, index, start) {
                        place_table(t, y, bottom, line as usize, part, first_on_page, doc.mode15, true)
                    } else {
                        laid
                    };
                    if let Some(out) = lines.as_deref_mut() {
                        let split_kept = !doc.mode15
                            && self.kept_row_breaks(
                                t,
                                &place_table(t, y, bottom, line as usize, part, first_on_page, doc.mode15, false),
                                index,
                                start,
                            );
                        let mut ranges = Vec::new();
                        let again = place_table_at(
                            t,
                            y,
                            bottom,
                            line as usize,
                            part,
                            first_on_page,
                            doc.mode15,
                            split_kept,
                            Some(&mut ranges),
                        );
                        for (&(row, header, top, _), cells) in again.pieces.iter().zip(&ranges) {
                            let content_top = top + t.rows[row].top_border;
                            for (c, &(a, b)) in cells.iter().enumerate() {
                                let cell = &t.rows[row].cells[c];
                                let mut line_y = content_top + cell.margin_top;
                                for k in a..b {
                                    let st = &cell.stack[k as usize];
                                    let top = line_y + st.above;
                                    out.push(PLine {
                                        item: index as u32,
                                        row: row as u32,
                                        cell: c as u32,
                                        para: st.para,
                                        number: st.number,
                                        top,
                                        pitch: st.pitch,
                                        header,
                                    });
                                    line_y += st.h;
                                }
                            }
                        }
                    }
                    if let Some(r) = rec.as_deref_mut() {
                        for &(row, header, top, h) in &laid.pieces {
                            if !header {
                                r.push(Placed {
                                    item: index as u32,
                                    line: row as u32,
                                    top,
                                    end: top + h,
                                    bottom,
                                });
                            }
                        }
                    }
                    if let Some((next_row, next_part)) = laid.next {
                        let reach = Pos {
                            item: index as u32,
                            line: next_row,
                            part: next_part.clone(),
                        };
                        if !laid.any_body {
                            return (self.break_before(start, Pos::at(index as u32, next_row)), false, reach);
                        }
                        if !next_part.is_empty() {
                            return (
                                Pos {
                                    item: index as u32,
                                    line: next_row,
                                    part: next_part,
                                },
                                false,
                                reach,
                            );
                        }
                        return (self.break_before(start, Pos::at(index as u32, next_row)), false, reach);
                    }
                    y = laid.end;
                    first_on_page = false;
                    previous = Some(index);
                    index += 1;
                    line = 0;
                    continue;
                }
                FItem::P(p) => {
                    if !first_on_page && line == 0 {
                        if let Some(prev) = previous {
                            if new_page_section(prev) {
                                let at = Pos::at(index as u32, 0);
                                return (at.clone(), true, at);
                            }
                        }
                        if p.pbb {
                            let at = Pos::at(index as u32, 0);
                            return (at.clone(), true, at);
                        }
                    }
                    let count = p.lines.len() as u32;
                    let mut number = line;
                    while number < count {
                        let h = p.lines[number as usize];
                        if number == 0 {
                            let gap = if first_on_page {
                                let keeps = p.section_start || (p.pbb && !doc.mode15);
                                let prev_after = if index > 0 {
                                    match &*items[index - 1] {
                                        FItem::P(a) => a.after,
                                        _ => 0,
                                    }
                                } else {
                                    0
                                };
                                if keeps && prev_after != 0 {
                                    doc.gap(prev_after, p.before).1
                                } else if keeps {
                                    doc.gap(0, p.before).0
                                } else {
                                    0
                                }
                            } else {
                                match &*items[previous.unwrap()] {
                                    FItem::T(_) => doc.px.twips(p.before),
                                    FItem::P(prev) => {
                                        let same = p.style == prev.style;
                                        let own_before = if p.contextual && same { 0 } else { p.before };
                                        let prev_after = if prev.contextual && same { 0 } else { prev.after };
                                        doc.gap(prev_after, own_before).0
                                    }
                                }
                            };
                            y += gap + p.top_border;
                        }
                        let border = if number == count - 1 || !doc.mode15 {
                            p.border
                        } else {
                            0
                        };
                        if !first_on_page && y + h.fit + border > bottom {
                            let overflow = Pos::at(index as u32, number);
                            return (self.break_before(start, overflow.clone()), false, overflow);
                        }
                        if let Some(r) = rec.as_deref_mut() {
                            r.push(Placed {
                                item: index as u32,
                                line: number,
                                top: y,
                                end: y + h.fit + border,
                                bottom,
                            });
                        }
                        if let Some(out) = lines.as_deref_mut() {
                            out.push(PLine {
                                item: index as u32,
                                row: u32::MAX,
                                cell: u32::MAX,
                                para: 0,
                                number,
                                top: y,
                                pitch: h.pitch,
                                header: false,
                            });
                        }
                        y += h.pitch;
                        first_on_page = false;
                        if !p.page_breaks.is_empty() && p.page_breaks.contains(&number) {
                            let end = if number + 1 < count {
                                Pos::at(index as u32, number + 1)
                            } else {
                                Pos::at(index as u32 + 1, 0)
                            };
                            return (end, true, Pos::at(index as u32, number));
                        }
                        number += 1;
                    }
                    y += p.border;
                    previous = Some(index);
                    index += 1;
                    line = 0;
                }
            }
        }
        let end = Pos::at(items.len() as u32, 0);
        (end.clone(), false, end)
    }

    fn kept_row_breaks(&self, t: &FTable, laid: &TablePage, index: usize, start: &Pos) -> bool {
        let Some((row, part)) = &laid.next else { return false };
        if !part.is_empty() {
            return false;
        }
        let row = *row as usize;
        if row >= t.rows.len() || !t.rows[row].keeps_next {
            return false;
        }
        self.keeps(start, Pos::at(index as u32, row as u32)) <= *start
    }
}

/// `paginate.paginate` from `resume` (`(position, page index, number before)`) until the end,
/// or until `until(position, page, number)` says the rest is known.
pub fn paginate(
    doc: &Doc,
    items: &[It],
    resume: Option<(Pos, u32, i64)>,
    until: &mut dyn FnMut(&Pos, u32, i64) -> bool,
) -> (Pagination, Option<(Pos, u32, i64)>) {
    let ctx = Ctx { doc, items };
    let mut out = Pagination::default();
    let (mut position, mut page, mut number) = resume.unwrap_or((Pos::default(), 0, 0));
    let mut first = true;
    while (position.item as usize) < items.len() {
        if !first && until(&position, page, number) {
            return (out, Some((position, page, number)));
        }
        first = false;
        out.page_at.push(page);
        out.number_before.push(number);
        let item = &*items[position.item as usize];
        let section_first = position.line == 0
            && position.part.is_empty()
            && (position.item == 0 || items[position.item as usize - 1].section() != item.section());
        let sec = &doc.sections[item.section() as usize];
        if section_first && position.item > 0 {
            // `section_start_number`
            let restarted = sec.page_number_start.is_some();
            let mut n = sec.page_number_start.unwrap_or(number + 1);
            let mut blanks = 0;
            let kind = sec.start_type.as_deref();
            if matches!(kind, Some("oddPage") | Some("evenPage"))
                && n % 2 != (if kind == Some("oddPage") { 1 } else { 0 })
            {
                if restarted {
                    n += 1;
                } else {
                    blanks = 1;
                    n += 1;
                }
            }
            if doc.even_odd && (page as i64 + blanks + 1) % 2 != n % 2 {
                blanks += 1;
            }
            number = n;
            for _ in 0..blanks {
                out.info.push(Info {
                    number: number - 1,
                    section: items[position.item as usize - 1].section(),
                    section_first: false,
                    story: DEFAULT,
                    blank: true,
                });
                page += 1;
            }
        } else if section_first && sec.page_number_start.is_some() {
            number = sec.page_number_start.unwrap();
        } else {
            number += 1;
        }
        let story = doc.story_kind(item.section(), section_first, number);
        out.info.push(Info {
            number,
            section: item.section(),
            section_first,
            story,
            blank: false,
        });
        out.starts.push(position.clone());
        let (end, _forced, reach) = ctx.scan(&position, story, None);
        out.reach.push(reach);
        position = end;
        page += 1;
    }
    (out, None)
}

pub fn paginate_all(doc: &Doc, items: &[It]) -> Pagination {
    paginate(doc, items, None, &mut |_, _, _| false).0
}

pub fn from_export_pages(e: &Export) -> Vec<Pos> {
    e.pages
        .iter()
        .map(|p| Pos {
            item: p.start.0 as u32,
            line: p.start.1 as u32,
            part: p.start.2.iter().map(|v| *v as u32).collect(),
        })
        .collect()
}

/// A line (a table row's part) as placed on its page: px from the page's top to its top, to
/// the end of what must fit, and the page's bottom margin.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub item: u32,
    pub line: u32,
    pub top: i64,
    pub end: i64,
    pub bottom: i64,
}

/// Lane E: a line drawn on a page -- a body paragraph's (`row` = `u32::MAX`) or a cell
/// paragraph's (its table item, row, cell, paragraph in the cell, line number) -- with its top
/// and pitch, px over `px_den`, as docx2svg's layout places it.
#[derive(Clone, Copy, Debug)]
pub struct PLine {
    pub item: u32,
    pub row: u32,
    pub cell: u32,
    pub para: u32,
    pub number: u32,
    pub top: i64,
    pub pitch: i64,
    pub header: bool,
}

/// Lane E: page `k`'s lines, placed again (lazily).
pub fn page_lines(doc: &Doc, items: &[It], pag: &Pagination, k: usize) -> Vec<PLine> {
    let ctx = Ctx { doc, items };
    let mut out = Vec::new();
    let story = pag.info[pag.page_at[k] as usize].story;
    ctx.scan_lines(&pag.starts[k], story, None, Some(&mut out));
    let end = pag.starts.get(k + 1).cloned().unwrap_or(Pos::at(items.len() as u32, 0));
    // what the page holds ends where the next page starts (a split row's part is the page's)
    out.retain(|p| {
        let at = Pos::at(p.item, if p.row == u32::MAX { p.number } else { p.row });
        at < end
            || (end.item == p.item
                && p.row != u32::MAX
                && (end.line > p.row || (end.line == p.row && !end.part.is_empty())))
    });
    out
}

/// Page `k` of `pag` placed again (lazily, when a fact asks): its lines with their tops.
pub fn place_page(doc: &Doc, items: &[It], pag: &Pagination, k: usize) -> Vec<Placed> {
    let ctx = Ctx { doc, items };
    let mut out = Vec::new();
    let story = pag.info[pag.page_at[k] as usize].story;
    ctx.scan(&pag.starts[k], story, Some(&mut out));
    // what the page holds ends where the next page starts
    let end = pag.starts.get(k + 1).cloned().unwrap_or(Pos::at(items.len() as u32, 0));
    out.retain(|p| Pos::at(p.item, p.line) < end || (end.item == p.item && end.line == p.line && !end.part.is_empty()));
    out
}
