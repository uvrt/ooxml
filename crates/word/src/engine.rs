//! Lane E: one document's layout state, changed by commits (the blocks an edit changed, as
//! the Python shim resolved them), laid out incrementally -- measure the new items, splice
//! them in, paginate from the change until the pages line up again, then the body's
//! page-number fields (PAGEREF, PAGE, NUMPAGES) to their fixed point as docx2svg's layout
//! does them -- and published as immutable snapshots that share the unchanged items.
//!
//! The owner (`Owner`) runs it on its own thread: commits that arrive while it lays out are
//! taken together by the next relayout; a check waits for the first snapshot that holds its
//! version, then only reads it.

use crate::incr::{repaginate, Change};
use crate::measure::{measure, Measured, Tables, BREAK, GLYPH, TAB};
use crate::model::{Export, FontIn, ItemIn, MeasureIn, SectionIn};
use crate::paginate::{
    item_from_export, page_lines, paginate_all, section_geo, Doc, FItem, It, LineBox, PLine, Pagination, Pos,
};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// One paragraph of an item (the item's own, or a cell's): what fields and facts read.
#[derive(Clone, Debug, Default)]
pub struct PAux {
    pub key: Option<String>,
    /// For a cell paragraph: its row, cell and place in the cell.
    pub row: u32,
    pub cell: u32,
    pub idx: u32,
    pub bookmarks: Vec<(String, usize)>,
    /// `(run, keyword, argument, text offset, current text)`.
    pub fields: Vec<(usize, String, Option<String>, usize, String)>,
    /// Text offset -> line (-1: none), kept where bookmarks or fields need it.
    pub line_of: Vec<i32>,
    /// Per line: the smallest half points of its text drawn (super/subscript left out; 0:
    /// none) and whether it draws anything but spaces.
    pub line_text: Vec<(i64, bool)>,
}

#[derive(Clone, Debug, Default)]
pub struct Aux {
    pub key: Option<String>,
    pub table: bool,
    pub paras: Vec<PAux>,
}

impl Aux {
    fn has_fields(&self) -> bool {
        self.paras.iter().any(|p| !p.fields.is_empty())
    }
}

fn para_aux(
    m: &MeasureIn,
    me: &Measured,
    key: Option<String>,
    bookmarks: &[(String, usize)],
    row: u32,
    cell: u32,
    idx: u32,
) -> PAux {
    let mut fields = Vec::new();
    let mut offset = 0usize;
    for (k, run) in m.runs.iter().enumerate() {
        if let Some((kw, arg)) = &run.field {
            fields.push((k, kw.clone(), arg.clone(), offset, run.text.clone()));
        }
        offset += run.text.chars().count();
    }
    let need = !fields.is_empty() || !bookmarks.is_empty();
    let mut line_of = Vec::new();
    if need {
        line_of = vec![-1i32; offset + 1];
        for (number, line) in me.lines.iter().enumerate() {
            for p in &me.pieces[line.start..line.end] {
                if p.source >= 0 && p.kind != TAB && p.kind != BREAK {
                    let slot = &mut line_of[p.source as usize];
                    if *slot < 0 {
                        *slot = number as i32;
                    }
                }
            }
        }
    }
    let mut line_text = Vec::with_capacity(me.lines.len());
    for line in &me.lines {
        let mut small = 0i64;
        let mut ink = false;
        for p in &me.pieces[line.start..line.end] {
            if p.hp == 0 || p.ch == '\0' || p.ch.is_whitespace() {
                continue;
            }
            ink = true;
            let va = p.run >= 0 && m.runs[p.run as usize].va;
            if !va && p.kind == GLYPH && (small == 0 || p.hp < small) {
                small = p.hp;
            }
        }
        line_text.push((small, ink));
    }
    PAux {
        key,
        row,
        cell,
        idx,
        bookmarks: bookmarks.to_vec(),
        fields,
        line_of,
        line_text,
    }
}

/// The paginator's item of an input, measured, and what the engine keeps beside it.
pub fn build(t: &Tables, it: &ItemIn) -> (FItem, Aux) {
    match it {
        ItemIn::Para(p) => {
            let me = measure(t, &p.measure);
            let lines: Vec<LineBox> = me.heights.iter().map(LineBox::from).collect();
            let one_column = !p.measure.columns;
            let page_breaks: Vec<u32> = me
                .lines
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    l.forced
                        && l.end > 0
                        && (me.pieces[l.end - 1].ch == '\u{c}' || (one_column && me.pieces[l.end - 1].ch == '\u{b}'))
                })
                .map(|(n, _)| n as u32)
                .collect();
            let aux = Aux {
                key: p.key.clone(),
                table: false,
                paras: vec![para_aux(
                    &p.measure,
                    &me,
                    p.key.clone(),
                    &p.bookmarks,
                    u32::MAX,
                    u32::MAX,
                    0,
                )],
            };
            let item = FItem::P(crate::paginate::FPara {
                block: p.block as u32,
                section: p.section as u32,
                lines,
                page_breaks,
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
            });
            (item, aux)
        }
        ItemIn::Table(tb) => {
            let mut hs = Vec::new();
            let mut paras = Vec::new();
            for (r, row) in tb.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    for (k, cp) in cell.paragraphs.iter().enumerate() {
                        let me = measure(t, &cp.measure);
                        paras.push(para_aux(
                            &cp.measure,
                            &me,
                            cp.key.clone(),
                            &cp.bookmarks,
                            r as u32,
                            c as u32,
                            k as u32,
                        ));
                        hs.push(me.heights);
                    }
                }
            }
            let item = item_from_export(&t.px, it, Some(&hs));
            (
                item,
                Aux {
                    key: tb.key.clone(),
                    table: true,
                    paras,
                },
            )
        }
    }
}

/// Lane E's commit: splices of the item list, in order (each in the list as the ones before
/// left it), and the measuring tables where they grew.
#[derive(Deserialize)]
pub struct Commit {
    pub version: u64,
    pub splices: Vec<Splice>,
    #[serde(default)]
    pub fonts: Option<Vec<FontIn>>,
    #[serde(default)]
    pub formats: Option<Vec<(i64, i64, i64, i64, Option<(i64, i64)>)>>,
    #[serde(default)]
    pub sections: Option<Vec<SectionIn>>,
}

#[derive(Deserialize)]
pub struct Splice {
    pub at: usize,
    pub remove: usize,
    pub items: Vec<ItemIn>,
}

#[derive(Clone, Debug, Default)]
pub struct Stat {
    pub version: u64,
    pub commits: usize,
    pub items_measured: usize,
    pub measure_ms: f64,
    pub paginate_ms: f64,
    pub fields_ms: f64,
    pub field_passes: usize,
    pub field_items: usize,
    pub publish_ms: f64,
    pub total_ms: f64,
    pub restart_page: usize,
    pub pages_laid: usize,
    pub pages: usize,
    pub parse_ms: f64,
    pub unsettled: bool,
}

pub struct Engine {
    pub t: Arc<Tables>,
    pub fonts: Vec<FontIn>,
    pub formats: Vec<(i64, i64, i64, i64, Option<(i64, i64)>)>,
    pub dens: (i64, i64, i64, i64),
    pub doc: Doc,
    pub inputs: Vec<Arc<ItemIn>>,
    pub flow: Vec<It>,
    pub aux: Vec<Arc<Aux>>,
    pub pag: Pagination,
    pub version: u64,
}

/// What a check reads: one layout, never changed.
pub struct Snap {
    pub version: u64,
    pub doc: Doc,
    pub flow: Vec<It>,
    pub aux: Vec<Arc<Aux>>,
    pub pag: Pagination,
    pub dpx: i64,
    keys: OnceLock<HashMap<String, usize>>,
    /// Every paragraph's key (cells' too) -> (item, paragraph in the item).
    paras: OnceLock<HashMap<String, (usize, usize)>>,
}

/// Items measured in parallel from this many on (fewer: on the owner's thread).
pub const PARALLEL_FROM: usize = 64;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

impl Engine {
    pub fn new(e: Export) -> (Engine, Stat) {
        let t0 = Instant::now();
        let t = Tables::new(&e);
        let doc = Doc::new(&e);
        let built: Vec<(FItem, Aux)> = e.items.par_iter().map(|it| build(&t, it)).collect();
        let measure_ms = ms(t0);
        let mut flow = Vec::with_capacity(built.len());
        let mut aux = Vec::with_capacity(built.len());
        for (f, a) in built {
            flow.push(It::new(f));
            aux.push(Arc::new(a));
        }
        let t1 = Instant::now();
        let pag = paginate_all(&doc, &flow);
        let paginate_ms = ms(t1);
        let mut eng = Engine {
            t: Arc::new(t),
            fonts: e.fonts,
            formats: e.formats,
            dens: (e.w_den, e.px_den, e.lu, e.dpi),
            doc,
            inputs: e.items.into_iter().map(Arc::new).collect(),
            flow,
            aux,
            pag,
            version: 0,
        };
        let mut st = Stat {
            measure_ms,
            paginate_ms,
            items_measured: eng.flow.len(),
            ..Default::default()
        };
        let t2 = Instant::now();
        let (passes, items, unsettled) = eng.fields();
        st.fields_ms = ms(t2);
        st.field_passes = passes;
        st.field_items = items;
        st.unsettled = unsettled;
        st.pages = eng.pag.info.len();
        st.total_ms = ms(t0);
        (eng, st)
    }

    /// Apply commits in order, as one relayout.
    pub fn apply(&mut self, commits: Vec<Commit>) -> Stat {
        let t0 = Instant::now();
        let mut st = Stat {
            commits: commits.len(),
            ..Default::default()
        };
        let old_len = self.flow.len();
        // the dirty span [lo, hi) of the current list
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        for c in commits {
            if c.fonts.is_some() || c.formats.is_some() {
                if let Some(f) = c.fonts {
                    self.fonts = f;
                }
                if let Some(f) = c.formats {
                    self.formats = f;
                }
                let (w_den, px_den, lu, dpi) = self.dens;
                self.t = Arc::new(Tables::from_parts(&self.fonts, &self.formats, w_den, px_den, lu, dpi));
            }
            if let Some(sections) = c.sections {
                self.doc.sections = sections.iter().map(|s| section_geo(&self.doc.px, s)).collect();
                lo = 0;
                hi = hi.max(self.flow.len());
            }
            let tm = Instant::now();
            for sp in c.splices {
                let t = self.t.clone();
                // a few items: measured on the owner's own thread (waking the pool's threads
                // costs more than measuring them); many: in parallel
                let built: Vec<(FItem, Aux)> = if sp.items.len() < PARALLEL_FROM {
                    sp.items.iter().map(|it| build(&t, it)).collect()
                } else {
                    sp.items.par_iter().map(|it| build(&t, it)).collect()
                };
                st.items_measured += built.len();
                let n = built.len();
                let (a, b) = (sp.at, sp.at + sp.remove);
                assert!(b <= self.flow.len(), "splice past the end");
                let mut flow = Vec::with_capacity(n);
                let mut aux = Vec::with_capacity(n);
                for (f, x) in built {
                    flow.push(It::new(f));
                    aux.push(Arc::new(x));
                }
                self.flow.splice(a..b, flow);
                self.aux.splice(a..b, aux);
                self.inputs.splice(a..b, sp.items.into_iter().map(Arc::new));
                let delta = n as i64 - sp.remove as i64;
                let shift = |x: usize| -> usize {
                    if x <= a {
                        x
                    } else if x >= b {
                        (x as i64 + delta) as usize
                    } else {
                        a + n
                    }
                };
                if lo == usize::MAX {
                    lo = a;
                    hi = a + n;
                } else {
                    lo = lo.min(a);
                    hi = shift(hi).max(a + n);
                }
            }
            st.measure_ms += ms(tm);
            self.version = self.version.max(c.version);
        }
        if lo != usize::MAX {
            let cur = self.flow.len();
            let ch = Change {
                a: lo,
                b_old: old_len - (cur - hi),
                b_new: hi,
            };
            let tp = Instant::now();
            self.repaginate(ch, &mut st);
            st.paginate_ms += ms(tp);
        }
        let tf = Instant::now();
        let (passes, items, unsettled) = self.fields();
        st.fields_ms = ms(tf);
        st.field_passes = passes;
        st.field_items = items;
        st.unsettled = unsettled;
        st.pages = self.pag.info.len();
        st.version = self.version;
        st.total_ms = ms(t0);
        st
    }

    fn repaginate(&mut self, ch: Change, st: &mut Stat) {
        if self.pag.starts.is_empty() || ch.a >= self.flow.len().min(usize::MAX) && self.flow.is_empty() {
            self.pag = paginate_all(&self.doc, &self.flow);
            return;
        }
        let (p, s) = repaginate(&self.doc, &self.pag, &self.flow, ch);
        st.restart_page = s.restart_page;
        st.pages_laid += s.pages_laid;
        self.pag = p;
    }

    /// The 0-based page index (blank pages counted) of a paragraph's text offset.
    fn page_of(&self, item: usize, pa: &PAux, offset: usize) -> Option<usize> {
        let lo = |o: usize| pa.line_of.get(o).copied().filter(|v| *v >= 0);
        let line = lo(offset)
            .or_else(|| if offset > 0 { lo(offset - 1) } else { None })
            .unwrap_or(0) as u32;
        if pa.row == u32::MAX {
            let pos = Pos::at(item as u32, line);
            let k = self.pag.starts.partition_point(|s| *s <= pos);
            if k == 0 {
                return None;
            }
            return Some(self.pag.page_at[k - 1] as usize);
        }
        // a cell paragraph: the first page it is drawn on (`paginate.drawn_pages`)
        let first = self
            .pag
            .starts
            .partition_point(|s| *s < Pos::at(item as u32, 0))
            .saturating_sub(1);
        let last = self.pag.starts.partition_point(|s| *s < Pos::at(item as u32 + 1, 0));
        for k in first..last {
            let lines = page_lines(&self.doc, &self.flow, &self.pag, k);
            if lines
                .iter()
                .any(|l| l.item == item as u32 && l.row == pa.row && l.cell == pa.cell && l.para == pa.idx)
            {
                return Some(self.pag.page_at[k] as usize);
            }
        }
        None
    }

    /// The body's computed fields to their fixed point (`layout._Placer.run_full`: values
    /// found on the layout, set, laid out again, until the values are those laid out with --
    /// four layouts at most).  Returns `(passes, items substituted, unsettled)`.
    pub fn fields(&mut self) -> (usize, usize, bool) {
        let mut passes = 0;
        let mut total = 0;
        loop {
            if !self.aux.iter().any(|a| a.has_fields()) {
                return (passes, total, false);
            }
            let mut marks: HashMap<&str, (usize, usize, usize)> = HashMap::new();
            for (i, a) in self.aux.iter().enumerate() {
                for (pi, p) in a.paras.iter().enumerate() {
                    for (name, off) in &p.bookmarks {
                        marks.entry(name.as_str()).or_insert((i, pi, *off));
                    }
                }
            }
            let pages = self.pag.info.len() as i64;
            let mut subs: Vec<(usize, Vec<(usize, usize, String)>)> = Vec::new();
            for (i, a) in self.aux.iter().enumerate() {
                if !a.has_fields() {
                    continue;
                }
                let mut mine = Vec::new();
                for (pi, p) in a.paras.iter().enumerate() {
                    for (run, kw, arg, offset, text) in &p.fields {
                        let value = match kw.as_str() {
                            "PAGEREF" => arg
                                .as_deref()
                                .and_then(|n| marks.get(n))
                                .and_then(|&(ti, tp, to)| self.page_of(ti, &self.aux[ti].paras[tp], to))
                                .map(|page| self.pag.info[page].number),
                            "PAGE" => self.page_of(i, p, *offset).map(|page| self.pag.info[page].number),
                            "NUMPAGES" => Some(pages),
                            _ => None,
                        };
                        if let Some(v) = value {
                            let s = v.to_string();
                            if &s != text {
                                mine.push((pi, *run, s));
                            }
                        }
                    }
                }
                if !mine.is_empty() {
                    subs.push((i, mine));
                }
            }
            if subs.is_empty() {
                return (passes, total, false);
            }
            if passes == 4 {
                return (passes, total, true);
            }
            passes += 1;
            total += subs.len();
            let t = self.t.clone();
            let (lo, hi) = (subs.first().unwrap().0, subs.last().unwrap().0 + 1);
            let rebuild = |(i, mine): (usize, Vec<(usize, usize, String)>)| {
                let mut input = (*self.inputs[i]).clone();
                for (pi, run, s) in mine {
                    let m = para_measure_mut(&mut input, &self.aux[i].paras[pi]);
                    let r = &mut m.runs[run];
                    r.text = s;
                    if r.fmts.len() > 1 {
                        r.fmts.truncate(1);
                    }
                }
                let built = build(&t, &input);
                (i, input, built)
            };
            let rebuilt: Vec<(usize, ItemIn, (FItem, Aux))> = if subs.len() < PARALLEL_FROM {
                subs.into_iter().map(rebuild).collect()
            } else {
                subs.into_par_iter().map(rebuild).collect()
            };
            for (i, input, (f, a)) in rebuilt {
                self.inputs[i] = Arc::new(input);
                self.flow[i] = It::new(f);
                self.aux[i] = Arc::new(a);
            }
            let mut st = Stat::default();
            self.repaginate(
                Change {
                    a: lo,
                    b_old: hi,
                    b_new: hi,
                },
                &mut st,
            );
        }
    }

    pub fn snapshot(&self) -> Snap {
        Snap {
            version: self.version,
            doc: self.doc.clone(),
            flow: self.flow.clone(),
            aux: self.aux.clone(),
            pag: self.pag.clone(),
            dpx: self.dens.1,
            keys: OnceLock::new(),
            paras: OnceLock::new(),
        }
    }
}

fn para_measure_mut<'a>(input: &'a mut ItemIn, pa: &PAux) -> &'a mut MeasureIn {
    match input {
        ItemIn::Para(p) => &mut p.measure,
        ItemIn::Table(t) => &mut t.rows[pa.row as usize].cells[pa.cell as usize].paragraphs[pa.idx as usize].measure,
    }
}

// -- reading a snapshot -----------------------------------------------------------------

#[derive(Deserialize)]
pub struct Query {
    /// The top-level blocks asked about (a range's, and a block on either side).
    pub blocks: Vec<String>,
    /// `blocks` is `[first, last]` of a span: every block between them asked about, and one
    /// on either side (a caller that holds no copy of the block list: the thin check).
    #[serde(default)]
    pub span: bool,
    /// Only each page's extent of the span's own lines (`[page, top, bottom]`, px over
    /// `px_den`): the range facts computed here, nothing else sent back.
    #[serde(default)]
    pub extent_only: bool,
}

impl Snap {
    fn keys(&self) -> &HashMap<String, usize> {
        self.keys.get_or_init(|| {
            let mut m = HashMap::with_capacity(self.aux.len());
            for (i, a) in self.aux.iter().enumerate() {
                if let Some(k) = &a.key {
                    m.insert(k.clone(), i);
                }
            }
            m
        })
    }

    fn paras(&self) -> &HashMap<String, (usize, usize)> {
        self.paras.get_or_init(|| {
            let mut m = HashMap::with_capacity(self.aux.len() * 2);
            for (i, a) in self.aux.iter().enumerate() {
                for (pi, p) in a.paras.iter().enumerate() {
                    if let Some(k) = &p.key {
                        m.insert(k.clone(), (i, pi));
                    }
                }
            }
            m
        })
    }

    /// The 1-based pages (blank pages counted) each paragraph or table is drawn on
    /// (`DocumentLayout.pages_of`), `[]` for a key not drawn.
    pub fn pages_of(&self, keys: &[String]) -> Vec<Vec<usize>> {
        let paras = self.paras();
        let items = self.keys();
        let mut placed: HashMap<usize, Vec<PLine>> = HashMap::new();
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let found = paras
                .get(key)
                .copied()
                .or_else(|| items.get(key).map(|&i| (i, usize::MAX)));
            let mut pages: Vec<usize> = Vec::new();
            if let Some((i, pi)) = found {
                let a = &self.aux[i];
                if !a.table {
                    let n = a.paras[0].line_text.len().max(1) as u32;
                    let first = self
                        .pag
                        .starts
                        .partition_point(|s| *s <= Pos::at(i as u32, 0))
                        .saturating_sub(1);
                    let last = self
                        .pag
                        .starts
                        .partition_point(|s| *s <= Pos::at(i as u32, n - 1))
                        .saturating_sub(1);
                    for k in first..=last.max(first) {
                        pages.push(self.pag.page_at[k] as usize + 1);
                    }
                } else {
                    let (first, last) = self.pages_of_item(i);
                    for k in first..last.min(self.pag.starts.len()) {
                        let lines = placed
                            .entry(k)
                            .or_insert_with(|| page_lines(&self.doc, &self.flow, &self.pag, k));
                        let hit = lines.iter().any(|l| {
                            l.item as usize == i
                                && (pi == usize::MAX || {
                                    let p = &a.paras[pi];
                                    l.row == p.row && l.cell == p.cell && l.para == p.idx
                                })
                        });
                        if hit {
                            pages.push(self.pag.page_at[k] as usize + 1);
                        }
                    }
                }
            }
            pages.dedup();
            out.push(pages);
        }
        out
    }

    pub fn page_count(&self) -> usize {
        self.pag.info.len()
    }

    /// Laid-out page `k`'s span of items.
    fn pages_of_item(&self, item: usize) -> (usize, usize) {
        let first = self
            .pag
            .starts
            .partition_point(|s| *s <= Pos::at(item as u32, 0))
            .saturating_sub(1);
        let last = self
            .pag
            .starts
            .partition_point(|s| *s < Pos::at(item as u32 + 1, 0))
            .max(first + 1);
        (first, last)
    }

    /// Every line on the pages the asked blocks are drawn on: one batched answer.
    pub fn query(&self, q: &Query) -> Value {
        let keys = self.keys();
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        let mut missing = Vec::new();
        let spanned: Vec<String>;
        let mut inside: Vec<String> = Vec::new();
        let blocks: &[String] = if q.span && q.blocks.len() == 2 {
            match (keys.get(&q.blocks[0]), keys.get(&q.blocks[1])) {
                (Some(&a), Some(&b)) => {
                    inside = (a..=b).filter_map(|i| self.aux[i].key.clone()).collect();
                    let (a, b) = (a.saturating_sub(1), (b + 1).min(self.aux.len() - 1));
                    spanned = (a..=b).filter_map(|i| self.aux[i].key.clone()).collect();
                    &spanned
                }
                _ => &q.blocks,
            }
        } else {
            &q.blocks
        };
        for b in blocks {
            match keys.get(b) {
                Some(&i) => {
                    let (a, z) = self.pages_of_item(i);
                    lo = lo.min(a);
                    hi = hi.max(z);
                }
                None => missing.push(b.clone()),
            }
        }
        let mut pages = Vec::new();
        if q.extent_only && lo != usize::MAX {
            let inside_items: std::collections::HashSet<usize> =
                inside.iter().filter_map(|k| keys.get(k).copied()).collect();
            let mut extent = Vec::new();
            for k in lo..hi.min(self.pag.starts.len()) {
                let lines = page_lines(&self.doc, &self.flow, &self.pag, k);
                let mut top = i64::MAX;
                let mut bottom = i64::MIN;
                for l in lines.iter().filter(|l| inside_items.contains(&(l.item as usize))) {
                    top = top.min(l.top);
                    bottom = bottom.max(l.top + l.pitch);
                }
                if top != i64::MAX {
                    extent.push(json!([self.pag.page_at[k] as usize + 1, top, bottom]));
                }
            }
            return json!({"version": self.version, "pages": self.page_count(), "px_den": self.dpx, "extent": extent});
        }
        if lo != usize::MAX {
            for k in lo..hi.min(self.pag.starts.len()) {
                let lines = page_lines(&self.doc, &self.flow, &self.pag, k);
                let index = self.pag.page_at[k] as usize;
                let info = &self.pag.info[index];
                let (top, bottom) = self.doc.sections[info.section as usize].kinds[info.story as usize];
                let mut out = Vec::with_capacity(lines.len());
                for l in &lines {
                    out.push(self.line_json(l));
                }
                pages.push(
                    json!({"page": index + 1, "section": info.section, "top": top, "bottom": bottom, "lines": out}),
                );
            }
        }
        json!({"version": self.version, "pages": self.page_count(), "px_den": self.dpx, "laid": pages, "missing": missing,
               "inside": inside})
    }

    fn line_json(&self, l: &PLine) -> Value {
        let a = &self.aux[l.item as usize];
        if l.row == u32::MAX {
            let p = &a.paras[0];
            let (hp, ink) = p.line_text.get(l.number as usize).copied().unwrap_or((0, false));
            json!([p.key, l.number, l.top, l.pitch, hp, ink])
        } else {
            let p = a
                .paras
                .iter()
                .find(|p| p.row == l.row && p.cell == l.cell && p.idx == l.para);
            let (key, hp, ink) = match p {
                Some(p) => {
                    let (hp, ink) = p.line_text.get(l.number as usize).copied().unwrap_or((0, false));
                    (p.key.clone(), hp, ink)
                }
                None => (None, 0, false),
            };
            json!([key, l.number, l.top, l.pitch, hp, ink, a.key, l.row, l.cell, l.para, l.header])
        }
    }

    /// The first page (1-based, blank pages counted) of every paragraph drawn in the body,
    /// with where it is first drawn there (item, row, cell, paragraph): what
    /// `since_last_check` compares, in drawing order.
    pub fn first_pages(&self) -> HashMap<&str, (usize, (u32, u32, u32, u32))> {
        let mut out: HashMap<&str, (usize, (u32, u32, u32, u32))> = HashMap::with_capacity(self.aux.len() * 2);
        for (i, a) in self.aux.iter().enumerate() {
            if !a.table {
                let p = &a.paras[0];
                if let (Some(key), Some(_)) = (&p.key, p.line_text.first()) {
                    let pos = Pos::at(i as u32, 0);
                    let k = self.pag.starts.partition_point(|s| *s <= pos).saturating_sub(1);
                    if k < self.pag.page_at.len() {
                        out.entry(key.as_str())
                            .or_insert((self.pag.page_at[k] as usize + 1, (i as u32, 0, 0, 0)));
                    }
                }
            } else {
                let (first, last) = self.pages_of_item(i);
                for k in first..last.min(self.pag.starts.len()) {
                    for l in page_lines(&self.doc, &self.flow, &self.pag, k) {
                        if l.item as usize != i {
                            continue;
                        }
                        if let Some(p) = a
                            .paras
                            .iter()
                            .find(|p| p.row == l.row && p.cell == l.cell && p.idx == l.para)
                        {
                            if let Some(key) = &p.key {
                                out.entry(key.as_str()).or_insert((
                                    self.pag.page_at[k] as usize + 1,
                                    (i as u32, l.row + 1, l.cell, l.para),
                                ));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// `moved` against an earlier snapshot: each paragraph whose first page changed, in the
    /// order this layout first draws them.
    pub fn moved(&self, before: &Snap) -> Vec<(String, usize, usize)> {
        let a = before.first_pages();
        let b = self.first_pages();
        let mut out: Vec<(usize, (u32, u32, u32, u32), String, usize)> = b
            .iter()
            .filter_map(|(k, (p, at))| a.get(k).filter(|q| q.0 != *p).map(|q| (*p, *at, k.to_string(), q.0)))
            .collect();
        out.sort();
        out.into_iter().map(|(p, _, k, q)| (k, q, p)).collect()
    }
}

// -- Word as one format of the owner (crate::owner) -----------------------------------------

/// Word's layout behind a format-neutral owner.
pub type WordLayout = Engine;

impl crate::owner::Format for Engine {
    type Commit = Commit;
    type Snap = Snap;
    type Stat = Stat;

    fn parse(&self, text: &str) -> Result<Commit, String> {
        serde_json::from_str::<Commit>(text).map_err(|e| e.to_string())
    }
    fn version_of(c: &Commit) -> u64 {
        c.version
    }
    fn apply(&mut self, commits: Vec<Commit>) -> Stat {
        Engine::apply(self, commits)
    }
    fn snapshot(&self) -> Snap {
        Engine::snapshot(self)
    }
    fn snap_version(s: &Snap) -> u64 {
        s.version
    }
}

/// A Word document's owner, and what laying it out first took.
pub struct Owner {
    pub core: crate::owner::Owner<Engine>,
    pub first: Stat,
}

impl Owner {
    pub fn open(e: Export) -> Owner {
        let (eng, st) = Engine::new(e);
        Owner {
            core: crate::owner::Owner::start(eng, "word-layout-owner"),
            first: st,
        }
    }
    pub fn commit(&self, json: String) {
        self.core.commit(json);
    }
    pub fn wait(&self, version: u64) -> Arc<Snap> {
        self.core.wait(version)
    }
    pub fn stop(&self) {
        self.core.stop();
    }
}

pub fn stat_json(st: &Stat, waited: f64) -> Value {
    json!({"version": st.version, "commits": st.commits, "items_measured": st.items_measured, "parse_ms": st.parse_ms,
           "measure_ms": st.measure_ms, "paginate_ms": st.paginate_ms, "fields_ms": st.fields_ms,
           "field_passes": st.field_passes, "field_items": st.field_items, "publish_ms": st.publish_ms,
           "total_ms": st.total_ms, "restart_page": st.restart_page, "pages_laid": st.pages_laid, "pages": st.pages,
           "commit_to_publish_ms": waited, "unsettled": st.unsettled})
}

pub fn pass_json(p: &crate::owner::Pass<Stat>) -> Value {
    let mut st = p.stat.clone();
    st.commits = p.commits;
    st.parse_ms = p.parse_ms;
    st.publish_ms = p.publish_ms;
    stat_json(&st, p.commit_to_publish_ms)
}
