//! Lane F: docx-agent's `check` of some ranges, its facts put together from one snapshot --
//! the pages the range's blocks are on, read lazily -- and from the records docx-agent made
//! of each block when it was made or changed (`ItemIn` `rec`, `docx_agent.rust.records`).
//! docx-agent's rules (`docx_agent.layout_facts`, `docx_agent.tools.scoped`, the reflow
//! summary's comparison) ported one for one; what a rule reads of the document as a whole is
//! made once per snapshot (`Global`); everything else reads only the range's pages and blocks
//! and their neighbours.
//!
//! What is not ported answers `{"fallback": reason}` and the check runs docx-agent's own way,
//! whole: a keep tried by laying out (`if_kept`, `if_released`), a column that may fit
//! (`column_fit`), a table that may reach past the margins (`past_margins`).

use crate::engine::{Aux, Snap};
use crate::model::ItemIn;
use crate::paginate::{page_lines, PLine, Pos};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// Entries per fact (`layout_facts.CAP`).
pub const CAP: usize = 20;
/// `scoped.EXTENT_PAGES`, `scoped.BREAKS_LISTED`.
pub const EXTENT_PAGES: usize = 12;
pub const BREAKS_LISTED: usize = 10;
/// `keep_trials.TRIALS`: trial layouts one check makes at most.
pub const TRIALS: usize = 4;
/// `layout_facts.WRAP_LINES`.
pub const WRAP_LINES: usize = 3;
/// `text_language.MIN_CONFIDENCE`.
pub const MIN_CONFIDENCE: f64 = 0.5;

#[derive(Deserialize, Clone)]
pub struct RangeReq {
    pub address: String,
    pub paragraphs: Vec<String>,
    /// The range's top-level blocks (whole blocks only: docx-agent checks it).
    pub blocks: Vec<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct Request {
    #[serde(default)]
    pub ranges: Vec<RangeReq>,
    /// What an entry may be about (`Scope.subjects`).
    #[serde(default)]
    pub subjects: Vec<String>,
    /// The scope's paragraphs (`Scope.paragraphs`).
    #[serde(default)]
    pub scoped: Vec<String>,
    #[serde(default)]
    pub keeps: bool,
    #[serde(default)]
    pub layout: bool,
    /// The document's aliases (old id -> new id), for `since_last_check`.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// Whether a `since_last_check` is asked (against the snapshot handed with the request).
    #[serde(default)]
    pub since: bool,
    /// Keys whose pages are asked (a check of numbers: `textfacts._layout`).
    #[serde(default)]
    pub pages_of: Vec<String>,
    /// Whether the fields facts are asked (`shared._field_facts`, its note aside).
    #[serde(default)]
    pub fields: bool,
    /// docx-agent's caps as its modules hold them at the call: `[CAP, EXTENT_PAGES,
    /// BREAKS_LISTED, TRIALS]`.
    #[serde(default = "caps")]
    pub caps: (usize, usize, usize, usize),
}

fn caps() -> (usize, usize, usize, usize) {
    (CAP, EXTENT_PAGES, BREAKS_LISTED, TRIALS)
}

/// Python's `round(x, n)`.
pub fn round(x: f64, n: usize) -> f64 {
    format!("{:.*}", n, x).parse().unwrap_or(x)
}

fn rec(it: &ItemIn) -> Option<&Value> {
    match it {
        ItemIn::Para(p) => p.rec.as_ref(),
        ItemIn::Table(t) => t.rec.as_ref(),
    }
}

fn b(v: Option<&Value>, k: &str) -> bool {
    v.and_then(|v| v.get(k)).and_then(Value::as_bool).unwrap_or(false)
}

fn s<'v>(v: Option<&'v Value>, k: &str) -> Option<&'v str> {
    v.and_then(|v| v.get(k)).and_then(Value::as_str)
}

fn f(v: Option<&Value>, k: &str) -> f64 {
    v.and_then(|v| v.get(k)).and_then(Value::as_f64).unwrap_or(0.0)
}

fn arr<'v>(v: Option<&'v Value>, k: &str) -> &'v [Value] {
    v.and_then(|v| v.get(k))
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
}

/// What the facts read of the document as a whole, made once per snapshot.
pub struct Global {
    /// The body's `empty_paragraphs` entry (every run in it), and its paragraphs' ids.
    pub empty_body: Option<Value>,
    /// `numbering_skips_level`'s entries over the whole document, unfiltered.
    pub numbering: Vec<Value>,
    /// `shared._field_facts` (its note aside), or why it is not made here.
    pub fields: Result<Value, String>,
}

fn make_global(s: &Snap) -> Global {
    // -- empty_paragraphs: the body's runs (layout_facts._empty_runs over _sequence(body))
    let mut states: Vec<(i64, Option<&str>, &str)> = Vec::new(); // (position, id, state)
    let mut position = 0i64;
    for it in &s.inputs {
        match &**it {
            ItemIn::Table(_) => states.push((-1, None, "kept")),
            ItemIn::Para(p) => {
                let r = p.rec.as_ref();
                if let Some(st) = r.and_then(|r| r.get("state")).and_then(Value::as_str) {
                    states.push((position, p.key.as_deref(), st));
                }
                position += 1;
            }
        }
    }
    let mut positions: Vec<i64> = Vec::new();
    let mut ids: Vec<&str> = Vec::new();
    let mut calls: Vec<Value> = Vec::new();
    let mut k = 0;
    while k < states.len() {
        if states[k].2 != "empty" {
            k += 1;
            continue;
        }
        let start = k;
        while k < states.len() && states[k].2 == "empty" {
            k += 1;
        }
        if start == 0 || k == states.len() || states[start - 1].2 != "text" || states[k].2 != "text" {
            continue;
        }
        let run = &states[start..k];
        let mine: Vec<&str> = run.iter().map(|r| r.1.unwrap_or("")).collect();
        // body paragraphs: siblings in order when their positions follow on
        let siblings = run.windows(2).all(|w| w[1].0 == w[0].0 + 1);
        let found: Vec<Value> = if mine.len() > 1 && siblings {
            vec![json!({"target": format!("{}..{}", mine[0], mine[mine.len() - 1])})]
        } else {
            mine.iter().rev().map(|i| json!({"target": i})).collect()
        };
        let mut joined = found;
        joined.extend(calls);
        calls = joined;
        positions.extend(run.iter().map(|r| r.0));
        ids.extend(mine);
    }
    let empty_body = if ids.is_empty() {
        None
    } else {
        Some(
            json!({"kind": "empty_paragraphs", "at": "body", "paragraphs": positions, "ids": ids,
                    "fix": {"tool": "word_delete", "calls": calls}}),
        )
    };
    // -- numbering_skips_level (layout_facts._skips_level): every paragraph in order
    let mut used: HashMap<String, BTreeSet<i64>> = HashMap::new();
    let mut groups: Vec<((String, i64, Value), Map<String, Value>, Vec<i64>, Option<Value>)> = Vec::new();
    let mut at: HashMap<(String, i64, String), usize> = HashMap::new();
    let mut visit = |r: &Value| {
        let Some(n) = r.get("num") else { return };
        let def = n.get("def").map(|d| d.to_string()).unwrap_or_default();
        let lvl = n.get("lvl").and_then(Value::as_i64).unwrap_or(0);
        let style = n.get("style").cloned().unwrap_or(Value::Null);
        used.entry(def.clone()).or_default().insert(lvl);
        let key = (def.clone(), lvl, style.to_string());
        if let Some(&g) = at.get(&key) {
            let c = groups[g].1.get("paragraphs").and_then(Value::as_i64).unwrap_or(0);
            groups[g].1.insert("paragraphs".into(), json!(c + 1));
        } else {
            let mut m = Map::new();
            m.insert("para".into(), r.get("id").cloned().unwrap_or(Value::Null));
            m.insert("style".into(), style.clone());
            m.insert("level".into(), json!(lvl));
            m.insert("list".into(), n.get("list").cloned().unwrap_or(Value::Null));
            m.insert("label".into(), n.get("label").cloned().unwrap_or(Value::Null));
            m.insert("paragraphs".into(), json!(1));
            let shown: Vec<i64> = n
                .get("shown")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            at.insert(key, groups.len());
            groups.push(((def, lvl, style), m, shown, n.get("first").cloned()));
        }
    };
    for it in &s.inputs {
        let r = rec(it);
        if let Some(r) = r {
            match &**it {
                ItemIn::Para(_) => visit(r),
                ItemIn::Table(_) => {
                    for p in arr(Some(r), "paras") {
                        visit(p);
                    }
                }
            }
        }
    }
    let mut numbering = Vec::new();
    for ((def, lvl, _), m, shown, first) in groups {
        let mine = used.get(&def).cloned().unwrap_or_default();
        let unused: Vec<i64> = shown
            .iter()
            .copied()
            .filter(|k| *k < lvl && !mine.contains(k))
            .collect();
        if unused.is_empty() {
            continue;
        }
        let mut out = Map::new();
        for key in ["para", "style", "level", "list"] {
            out.insert(key.into(), m[key].clone());
        }
        out.insert("unused_levels".into(), json!(unused));
        out.insert("label".into(), m["label"].clone());
        let all_unused = shown.iter().filter(|k| **k < lvl).all(|k| unused.contains(k));
        if all_unused {
            if let Some(v) = first.filter(|v| !v.is_null()) {
                out.insert("shows".into(), v);
            }
        }
        out.insert("paragraphs".into(), m["paragraphs"].clone());
        numbering.push(Value::Object(out));
    }
    Global {
        empty_body,
        numbering,
        fields: fields_facts(s),
    }
}

/// Python's `repr` of a plain string (`None` where it holds anything but printable ASCII).
fn py_repr(x: &str) -> Option<String> {
    if !x.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return None;
    }
    let quote = if x.contains('\'') && !x.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in x.chars() {
        if c == '\\' || c == quote {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(quote);
    Some(out)
}

/// `shared._field_facts`: every story's fields counted by keyword, tables of contents
/// without entries, references to bookmarks the document lacks.
fn fields_facts(s: &Snap) -> Result<Value, String> {
    let parts = s
        .constants
        .get("field_parts")
        .and_then(Value::as_array)
        .ok_or("no fields read")?;
    let mut names: HashSet<String> = HashSet::new();
    let mut add_names = |list: &[Value]| -> Result<(), String> {
        for n in list.iter().filter_map(Value::as_str) {
            if !n.is_ascii() {
                return Err("a bookmark named outside ASCII".into());
            }
            names.insert(n.to_string());
        }
        Ok(())
    };
    add_names(arr(Some(&s.constants), "bookmarks"))?;
    for it in &s.inputs {
        add_names(arr(rec(it), "bm"))?;
    }
    let folded: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let mut counts = Map::new();
    let mut empty = Vec::new();
    let mut missing = Vec::new();
    for part in parts {
        let story = part.get("story").and_then(Value::as_str).unwrap_or("");
        let body = part.get("body").and_then(Value::as_bool).unwrap_or(false);
        let mut fields: Vec<&Value> = Vec::new();
        if body {
            for it in &s.inputs {
                fields.extend(arr(rec(it), "fld").iter());
            }
        } else {
            fields.extend(arr(Some(part), "fields").iter());
        }
        for (number, f) in fields.into_iter().enumerate() {
            let keyword = f.get(0).and_then(Value::as_str).unwrap_or("");
            let c = counts.get(keyword).and_then(Value::as_i64).unwrap_or(0);
            counts.insert(keyword.to_string(), json!(c + 1));
            let id = format!("fld:{story}/{number}");
            if keyword == "TOC" {
                match f.get(2).and_then(Value::as_str) {
                    Some("empty") => empty.push(json!(format!(
                        "{id}: the table of contents has no entries: no heading at its levels yet; add the headings, then update the fields"
                    ))),
                    Some(_) => {}
                    None => return Err("a table of contents read past its block".into()),
                }
            }
            if matches!(keyword, "REF" | "PAGEREF" | "NOTEREF") {
                let name = f.get(1).and_then(Value::as_str).unwrap_or("");
                let bare = name.strip_prefix("bm:").unwrap_or(name);
                if !bare.is_ascii() {
                    return Err("a reference named outside ASCII".into());
                }
                if !names.contains(bare) && !folded.contains(&bare.to_lowercase()) {
                    let shown = py_repr(name).ok_or("a reference's name to quote")?;
                    missing.push(json!(format!("{id}: {keyword} to a missing bookmark {shown}")));
                }
            }
        }
    }
    let mut out = Map::new();
    out.insert("counts".into(), Value::Object(counts));
    if !empty.is_empty() {
        out.insert("empty_toc".into(), Value::Array(empty));
    }
    if !missing.is_empty() {
        out.insert("missing_bookmarks".into(), Value::Array(missing));
    }
    Ok(Value::Object(out))
}

/// One check's reading of a snapshot.
struct F<'a> {
    s: &'a Snap,
    laid: RefCell<HashMap<usize, Rc<Vec<PLine>>>>,
    subjects: HashSet<&'a str>,
    scoped: HashSet<&'a str>,
    wanted: Option<BTreeSet<usize>>,
    /// `CAP`, `EXTENT_PAGES`, `BREAKS_LISTED`, `TRIALS` as the request says them.
    caps: (usize, usize, usize, usize),
    /// Top-level key -> item.
    items: &'a HashMap<String, usize>,
    /// Any paragraph key -> (item, paragraph in it).
    paras: &'a HashMap<String, (usize, usize)>,
}

struct Ext {
    page: usize,
    top: f64,
    bottom: f64,
    body: f64,
    area: (f64, f64),
}

impl<'a> F<'a> {
    fn lines(&self, k: usize) -> Rc<Vec<PLine>> {
        if let Some(found) = self.laid.borrow().get(&k) {
            return found.clone();
        }
        let made = Rc::new(page_lines(&self.s.doc, &self.s.flow, &self.s.pag, k));
        self.laid.borrow_mut().insert(k, made.clone());
        made
    }

    fn number(&self, k: usize) -> usize {
        self.s.pag.page_at[k] as usize + 1
    }

    fn shown(&self, pages: &[usize]) -> bool {
        match &self.wanted {
            None => true,
            Some(w) => pages.iter().any(|p| w.contains(p)),
        }
    }

    fn about(&self, entry: &Value) -> bool {
        let mut named = false;
        for key in ["para", "paras", "ids", "table", "drawing", "blocks"] {
            let Some(v) = entry.get(key) else { continue };
            if v.is_null() {
                continue;
            }
            named = true;
            let hit = match v {
                Value::Array(a) => a.iter().any(|x| x.as_str().is_some_and(|x| self.subjects.contains(x))),
                Value::String(x) => self.subjects.contains(x.as_str()),
                _ => false,
            };
            if hit {
                return true;
            }
        }
        !named
    }

    fn capped(&self, entries: Vec<Value>, name: &str, out: &mut Map<String, Value>) {
        let kept: Vec<Value> = entries.into_iter().filter(|e| self.about(e)).collect();
        let (cap, ..) = self.caps;
        let n = kept.len();
        out.insert(name.into(), Value::Array(kept.into_iter().take(cap).collect()));
        if n > cap {
            out.insert(format!("{name}_left_out"), json!(n - cap));
        }
    }

    /// The lines of a paragraph `(item, paragraph)` with their 1-based pages.
    fn placed(&self, item: usize, pi: usize) -> Vec<(usize, PLine)> {
        let a = &self.s.aux[item];
        let mut out = Vec::new();
        if !a.table {
            let n = a.paras[0].line_text.len().max(1) as u32;
            let first = self
                .s
                .pag
                .starts
                .partition_point(|s| *s <= Pos::at(item as u32, 0))
                .saturating_sub(1);
            let last = self
                .s
                .pag
                .starts
                .partition_point(|s| *s <= Pos::at(item as u32, n - 1))
                .saturating_sub(1);
            for k in first..=last.max(first) {
                if k >= self.s.pag.starts.len() {
                    break;
                }
                for l in self.lines(k).iter() {
                    if l.item as usize == item && l.row == u32::MAX {
                        out.push((self.number(k), *l));
                    }
                }
            }
        } else {
            let p = &a.paras[pi];
            let (first, last) = self.s.pages_of_item(item);
            for k in first..last.min(self.s.pag.starts.len()) {
                for l in self.lines(k).iter() {
                    if l.item as usize == item && l.row == p.row && l.cell == p.cell && l.para == p.idx {
                        out.push((self.number(k), *l));
                    }
                }
            }
        }
        out
    }

    fn para_of(&self, key: &str) -> Option<(usize, usize)> {
        self.paras.get(key).copied()
    }

    /// `_Facts.pages_of` (a paragraph's pages), `_table_pages` (a table's: its paragraphs').
    fn pages_of(&self, key: &str) -> Vec<usize> {
        let mut set = BTreeSet::new();
        if let Some((i, pi)) = self.para_of(key) {
            for (n, _) in self.placed(i, pi) {
                set.insert(n);
            }
        } else if let Some(&i) = self.items.get(key) {
            for pi in 0..self.s.aux[i].paras.len() {
                for (n, _) in self.placed(i, pi) {
                    set.insert(n);
                }
            }
        }
        set.into_iter().collect()
    }

    fn item_pages(&self, i: usize) -> Vec<usize> {
        let mut set = BTreeSet::new();
        for pi in 0..self.s.aux[i].paras.len() {
            for (n, _) in self.placed(i, pi) {
                set.insert(n);
            }
        }
        set.into_iter().collect()
    }

    /// `_Facts.first_line`: `(page, top)` where a top-level block is first drawn.
    fn first_line(&self, i: usize) -> Option<(usize, i64)> {
        let a = &self.s.aux[i];
        if !a.table {
            let found = self.placed(i, 0);
            let page = found.first()?.0;
            return found
                .iter()
                .filter(|(p, _)| *p == page)
                .map(|(_, l)| (page, l.top))
                .min_by_key(|x| x.1);
        }
        let mut best: Option<(usize, i64)> = None;
        for (pi, p) in a.paras.iter().enumerate() {
            if p.key.is_none() {
                continue;
            }
            for (page, l) in self.placed(i, pi) {
                if best.is_none_or(|b| (page, l.top) < b) {
                    best = Some((page, l.top));
                }
            }
        }
        best
    }

    fn pt(&self, px: i64) -> f64 {
        round(((px as i128) * 72) as f64 / ((self.s.dpx as i128) * 300) as f64, 2)
    }

    fn rec(&self, i: usize) -> Option<&'a Value> {
        rec(&self.s.inputs[i])
    }

    fn key(&self, i: usize) -> Option<&'a str> {
        self.s.aux[i].key.as_deref()
    }

    /// `DocumentLayout.extent` of some paragraphs, on `pages` (None: every page).
    fn extent(&self, paragraphs: &[String], pages: Option<&[usize]>) -> Vec<Ext> {
        let mut wanted: HashSet<(usize, usize)> = HashSet::new();
        for p in paragraphs {
            if let Some(found) = self.para_of(p) {
                wanted.insert(found);
            } else if let Some(&i) = self.items.get(p.as_str()) {
                for pi in 0..self.s.aux[i].paras.len() {
                    wanted.insert((i, pi));
                }
            }
        }
        let mut items: Vec<usize> = wanted.iter().map(|(i, _)| *i).collect();
        items.sort();
        items.dedup();
        let mut by_page: BTreeMap<usize, (i64, i64, usize)> = BTreeMap::new();
        for &i in &items {
            let a = &self.s.aux[i];
            for (pi, _) in a.paras.iter().enumerate() {
                if !wanted.contains(&(i, pi)) {
                    continue;
                }
                for (n, l) in self.placed(i, pi) {
                    if pages.is_some_and(|w| !w.contains(&n)) {
                        continue;
                    }
                    let e = by_page.entry(n).or_insert((i64::MAX, i64::MIN, 0));
                    e.0 = e.0.min(l.top);
                    e.1 = e.1.max(l.top + l.pitch);
                    e.2 += 1;
                }
            }
        }
        let bodies = self.s.constants.get("bodies").and_then(Value::as_array);
        by_page
            .into_iter()
            .map(|(page, (top, bottom, _))| {
                let info = &self.s.pag.info[page - 1];
                let (at, ab) = self.s.doc.sections[info.section as usize].kinds[info.story as usize];
                let area = (self.pt(at), self.pt(ab));
                let body = bodies
                    .and_then(|v| v.get(info.section as usize))
                    .and_then(Value::as_f64)
                    .unwrap_or(round(area.1 - area.0, 2));
                Ext {
                    page,
                    top: self.pt(top),
                    bottom: self.pt(bottom),
                    body,
                    area,
                }
            })
            .collect()
    }

    /// `_new_page_after(k)`: the top-level block after `i` starts a page whatever keeps it.
    fn new_page_after(&self, i: usize) -> bool {
        if i + 1 >= self.s.inputs.len() {
            return false;
        }
        let next = self.rec(i + 1);
        if matches!(&*self.s.inputs[i + 1], ItemIn::Para(_)) && (b(next, "pbb") || b(next, "bs")) {
            return true;
        }
        b(self.rec(i), "be")
    }

    fn is_para(&self, i: usize) -> bool {
        matches!(&*self.s.inputs[i], ItemIn::Para(_))
    }

    /// The items whose lines may be on the wanted pages, and one on either side.
    fn near(&self) -> (usize, usize) {
        let n = self.s.inputs.len();
        let Some(w) = &self.wanted else { return (0, n) };
        let (Some(&lo), Some(&hi)) = (w.first(), w.last()) else {
            return (0, 0);
        };
        let ks: Vec<usize> = (0..self.s.pag.starts.len())
            .filter(|k| {
                let p = self.number(*k);
                p >= lo && p <= hi
            })
            .collect();
        let (Some(&k0), Some(&k1)) = (ks.first(), ks.last()) else {
            return (0, 0);
        };
        let first = self.s.pag.starts[k0].item as usize;
        let last = self.s.pag.starts.get(k1 + 1).map_or(n, |s| s.item as usize + 1);
        (first.saturating_sub(1), (last + 1).min(n))
    }
}

/// The range part of a check: `scoped.range_pages` with extent and keeps.
fn ranges(fx: &F, req: &Request, fallback: &mut Option<String>) -> (Vec<Value>, Vec<usize>) {
    let trials = &mut 0usize;
    let mut out = Vec::new();
    let mut every = BTreeSet::new();
    for r in &req.ranges {
        let mut pages = BTreeSet::new();
        for p in &r.paragraphs {
            pages.extend(fx.pages_of(p));
        }
        if pages.is_empty() {
            for b in &r.blocks {
                pages.extend(fx.pages_of(b));
            }
        }
        let pages: Vec<usize> = pages.into_iter().collect();
        let mut m = Map::new();
        m.insert("range".into(), json!(r.address));
        m.insert("blocks".into(), json!(r.blocks.len()));
        m.insert("pages".into(), json!(pages));
        if !pages.is_empty() {
            // range_extent
            let extents = fx.extent(&r.paragraphs, Some(&pages));
            let mut listed = Vec::new();
            let (_, extent_pages, breaks_listed, trial_cap) = fx.caps;
            for e in extents.iter().take(extent_pages) {
                let mut x = Map::new();
                x.insert("page".into(), json!(e.page));
                x.insert("top".into(), json!(e.top));
                x.insert("bottom".into(), json!(e.bottom));
                x.insert("body".into(), json!(e.body));
                if e.area.1 - e.area.0 < e.body - 0.01 {
                    x.insert("area".into(), json!([e.area.0, e.area.1]));
                }
                listed.push(Value::Object(x));
            }
            m.insert("extent".into(), Value::Array(listed));
            let height: f64 = extents.iter().map(|e| e.bottom - e.top).sum();
            m.insert("height".into(), json!(round(height, 2)));
            if extents.len() > extent_pages {
                m.insert("extent_more".into(), json!(extents.len() - extent_pages));
            }
            // range_keeps
            let items: Vec<Option<usize>> = r.blocks.iter().map(|k| fx.items.get(k.as_str()).copied()).collect();
            if !r.blocks.is_empty() && items.iter().all(Option::is_some) {
                let items: Vec<usize> = items.into_iter().map(Option::unwrap).collect();
                let drawn: Vec<Vec<usize>> = items.iter().map(|&i| fx.item_pages(i)).collect();
                if drawn.iter().all(|d| !d.is_empty()) {
                    let last = items.len() - 1;
                    // Document._keep_changes: a paragraph keeps together (and with the next,
                    // but the last); a table's rows keep with the next (its last but the last's)
                    let kept = items.iter().enumerate().all(|(k, &i)| {
                        let r = fx.rec(i);
                        if fx.is_para(i) {
                            b(r, "kt") && (k == last || b(r, "kwn"))
                        } else {
                            b(r, if k < last { "kmid" } else { "klast" })
                        }
                    });
                    let all: BTreeSet<usize> = drawn.iter().flatten().copied().collect();
                    let one_page = all.len() == 1;
                    m.insert("one_page".into(), json!(one_page));
                    m.insert("kept".into(), json!(kept));
                    if !one_page {
                        let mut breaks = Vec::new();
                        let mut all_page_break = true;
                        for (k, &i) in items.iter().enumerate() {
                            let pages = &drawn[k];
                            if k > 0 && drawn[k - 1].iter().max() < pages.iter().min() {
                                let mut gap = Map::new();
                                gap.insert("after".into(), json!(r.blocks[k - 1]));
                                gap.insert("before".into(), json!(r.blocks[k]));
                                gap.insert("page".into(), json!(pages.iter().min()));
                                let starts = (fx.is_para(i) && (b(fx.rec(i), "pbb") || b(fx.rec(i), "bs")))
                                    || (fx.is_para(items[k - 1]) && b(fx.rec(items[k - 1]), "be"));
                                if starts {
                                    gap.insert("page_break".into(), json!(true));
                                } else {
                                    all_page_break = false;
                                }
                                breaks.push(Value::Object(gap));
                            }
                            if pages.len() > 1 {
                                all_page_break = false;
                                breaks.push(json!({"inside": r.blocks[k], "pages": pages}));
                            }
                        }
                        let n = breaks.len();
                        m.insert(
                            "breaks".into(),
                            Value::Array(breaks.into_iter().take(breaks_listed).collect()),
                        );
                        if n > breaks_listed {
                            m.insert("breaks_more".into(), json!(n - breaks_listed));
                        }
                        let whole = fx.extent(&r.paragraphs, None);
                        let height: f64 = whole.iter().map(|e| e.bottom - e.top).sum();
                        let room = whole.iter().map(|e| e.area.1 - e.area.0).fold(f64::INFINITY, f64::min);
                        let room = if whole.is_empty() { 0.0 } else { room };
                        if !whole.is_empty() && height > room + 0.5 {
                            m.insert("taller_than_page".into(), json!(true));
                        } else if !kept && !all_page_break {
                            m.insert(
                                "fix".into(),
                                json!({"tool": "word_format", "calls": [{"targets": [r.address], "keep_together": true}]}),
                            );
                        }
                    }
                    if req.keeps {
                        // keep_trials._wanted: try the keep, or releasing it
                        let on = if m.contains_key("fix") {
                            Some(true)
                        } else if kept && one_page && pages[0] >= 2 && released_starts(fx, &items, pages[0]) {
                            Some(false)
                        } else {
                            None
                        };
                        if let Some(on) = on {
                            *trials += 1;
                            if *trials > trial_cap {
                                *fallback = Some("more keeps to try than a check tries".into());
                            } else if let Some(effect) = keep_trial(fx, &items, on, &r.address) {
                                m.insert(if on { "if_kept" } else { "if_released" }.into(), effect);
                            }
                        }
                    }
                }
            }
        }
        every.extend(pages.iter().copied());
        out.push(Value::Object(m));
    }
    (out, every.into_iter().collect())
}

/// Whether a kept range on one page starts it only because of its keep (`keep_trials._wanted`):
/// its first block the first drawn on its first page, a block before it, no page break or
/// section start between them.
fn released_starts(fx: &F, items: &[usize], start: usize) -> bool {
    let first = items[0];
    if first == 0 {
        return false;
    }
    let s = fx.s;
    let Some(k) = (0..s.pag.starts.len()).find(|k| fx.number(*k) == start) else {
        return false;
    };
    let drawn_first = fx.lines(k).first().map(|l| l.item as usize);
    if drawn_first != Some(first) {
        return false;
    }
    let before = first - 1;
    let starts = (fx.is_para(first) && (b(fx.rec(first), "pbb") || b(fx.rec(first), "bs")))
        || (fx.is_para(before) && b(fx.rec(before), "be"));
    !starts
}

/// `keep_trials._trial`: the range's keep made (`on`) or released, the document laid out
/// again from the snapshot, and where the range and what follows go.
fn keep_trial(fx: &F, items: &[usize], on: bool, address: &str) -> Option<Value> {
    use crate::paginate::FItem;
    let s = fx.s;
    let last = items.len() - 1;
    let mut eng = crate::engine::Engine::of_snapshot(s);
    let mut changed = false;
    for (k, &i) in items.iter().enumerate() {
        let r = fx.rec(i);
        match &*s.flow[i] {
            FItem::P(p) => {
                let mut p2 = p.clone();
                // Document._keep_values: on, each keep it lacks; off, each direct one its
                // styles do not give
                let mut set = |name: &str, current: bool, direct: &str, styled: &str| -> bool {
                    if on {
                        if !current {
                            changed = true;
                            return true;
                        }
                        current
                    } else if b(r, direct) && !b(r, styled) {
                        changed = true;
                        b(r, styled)
                    } else {
                        let _ = name;
                        current
                    }
                };
                p2.keep_lines = set("keep_together", b(r, "kt"), "ktd", "kts");
                if k < last {
                    p2.keep_next = set("keep_with_next", b(r, "kwn"), "kwnd", "kwns");
                }
                eng.flow[i] = std::sync::Arc::new(FItem::P(p2));
            }
            FItem::T(t) => {
                let mut t2 = t.clone();
                let rows = arr(r, "rows");
                for (n, row) in rows.iter().enumerate() {
                    if k == last && n + 1 == rows.len() {
                        continue;
                    }
                    if row.get(0).is_none_or(Value::is_null) || n >= t2.rows.len() {
                        continue;
                    }
                    let keeps = row.get(1).and_then(Value::as_bool).unwrap_or(false);
                    let declared = row.get(3).and_then(Value::as_bool).unwrap_or(false);
                    let styled = row.get(4).and_then(Value::as_bool).unwrap_or(false);
                    if on && !keeps {
                        t2.rows[n].keeps_next = true;
                        changed = true;
                    } else if !on && declared && !styled {
                        t2.rows[n].keeps_next = false;
                        changed = true;
                    }
                }
                eng.flow[i] = std::sync::Arc::new(FItem::T(t2));
            }
        }
    }
    if !changed {
        return None;
    }
    let lo = *items.iter().min()?;
    let hi = *items.iter().max()? + 1;
    let mut st = crate::engine::Stat::default();
    eng.repaginate(
        crate::incr::Change {
            a: lo,
            b_old: hi,
            b_new: hi,
        },
        &mut st,
    );
    eng.fields();
    let then = eng.snapshot();
    let after = if hi < s.inputs.len() { Some(hi) } else { None };
    let (now_start, _, now_after) = span_pages(fx, s, items, after);
    let fy = F {
        s: &then,
        laid: RefCell::new(HashMap::new()),
        subjects: HashSet::new(),
        scoped: HashSet::new(),
        wanted: None,
        caps: fx.caps,
        items: then.keys(),
        paras: then.paras(),
    };
    let (then_start, then_end, then_after) = span_pages(&fy, &then, items, after);
    let (now_start, then_start) = (now_start?, then_start?);
    let mut e = Map::new();
    e.insert("starts".into(), json!(then_start));
    e.insert("ends".into(), json!(then_end));
    e.insert("one_page".into(), json!(Some(then_start) == then_end));
    let (kept_start, free_start) = if on {
        (then_start, now_start)
    } else {
        (now_start, then_start)
    };
    if kept_start > free_start && kept_start > 1 {
        let page = kept_start - 1;
        let (kept_room, free_room) = if on {
            (room(&fy, page), room(fx, page))
        } else {
            (room(fx, page), room(&fy, page))
        };
        let (then_room, now_room) = if on {
            (kept_room, free_room)
        } else {
            (free_room, kept_room)
        };
        e.insert(
            "blank".into(),
            json!({"page": page, "pt": then_room.0, "share": then_room.1, "now_pt": now_room.0,
                   "now_share": now_room.1}),
        );
    }
    e.insert("document_pages".into(), json!([s.page_count(), then.page_count()]));
    if after.is_some() {
        if let (Some(a), Some(b2)) = (now_after, then_after) {
            e.insert("after".into(), json!([a, b2]));
        }
    }
    if !on {
        e.insert(
            "fix".into(),
            json!({"tool": "word_format", "calls": [{"targets": [address], "keep_together": false}]}),
        );
    }
    let said = said(&e, on);
    e.insert("said".into(), json!(said));
    Some(Value::Object(e))
}

/// `keep_trials._Pages`: the first and last page the items are drawn on, and the first page
/// of the item `after`.
fn span_pages(
    fx: &F,
    s: &Snap,
    items: &[usize],
    after: Option<usize>,
) -> (Option<usize>, Option<usize>, Option<usize>) {
    let mut drawn = BTreeSet::new();
    for &i in items {
        drawn.extend(fx.item_pages(i));
    }
    let _ = s;
    let start = drawn.iter().next().copied();
    let end = drawn.iter().next_back().copied();
    let after = after.and_then(|a| fx.item_pages(a).first().copied());
    (start, end, after)
}

/// `keep_trials._room`: the blank at the foot of a page's body, pt (one decimal) and share
/// of the body's height (two decimals).
fn room(fx: &F, page: usize) -> (f64, f64) {
    let s = fx.s;
    let info = &s.pag.info[page - 1];
    let (top, bottom) = s.doc.sections[info.section as usize].kinds[info.story as usize];
    let mut lowest = top;
    if let Some(k) = (0..s.pag.starts.len()).find(|k| fx.number(*k) == page) {
        for l in fx.lines(k).iter() {
            lowest = lowest.max(l.top + l.pitch);
        }
    }
    let height = bottom - top;
    let blank = (bottom - lowest).max(0);
    let pt = round(((blank as i128) * 72) as f64 / ((s.dpx as i128) * 300) as f64, 1);
    let share = if height > 0 {
        round(blank as f64 / height as f64, 2)
    } else {
        0.0
    };
    (pt, share)
}

/// Python's `round(x)` (half to even).
fn py_round0(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Python's `f"{x:g}"` of a point length (one decimal at most).
fn py_g(x: f64) -> String {
    format!("{x}")
}

fn plural_pages(count: u64) -> String {
    format!("{count} page{}", if count == 1 { "" } else { "s" })
}

/// `keep_trials._said`.
fn said(e: &Map<String, Value>, on: bool) -> String {
    let start = e["starts"].as_u64().unwrap_or(0);
    let end = e["ends"].as_u64().unwrap_or(0);
    let mut out = String::from(if on {
        "keeping it whole (this range alone): "
    } else {
        "releasing its keep (this range alone): "
    });
    out += &format!("starts on page {start}");
    if start == end {
        out += ", on one page";
    } else {
        out += &format!(", over pages {start}-{end}");
    }
    if let Some(blank) = e.get("blank") {
        out += &format!(
            "; page {} {} {}% blank ({} pt), now {}%",
            blank["page"],
            if on { "left" } else { "then" },
            py_round0(blank["share"].as_f64().unwrap_or(0.0) * 100.0),
            py_g(blank["pt"].as_f64().unwrap_or(0.0)),
            py_round0(blank["now_share"].as_f64().unwrap_or(0.0) * 100.0)
        );
    }
    let pages = e["document_pages"].as_array().cloned().unwrap_or_default();
    let (before, after) = (pages[0].as_u64().unwrap_or(0), pages[1].as_u64().unwrap_or(0));
    if before == after {
        out += &format!("; the document stays at {}", plural_pages(before));
    } else {
        out += &format!("; the document goes from {before} to {}", plural_pages(after));
    }
    if let Some(moved) = e.get("after").and_then(Value::as_array) {
        let (a, b2) = (moved[0].as_u64().unwrap_or(0), moved[1].as_u64().unwrap_or(0));
        if a == b2 {
            out += &format!("; what follows stays on page {a}");
        } else {
            out += &format!("; what follows moves from page {a} to {b2}");
        }
    }
    out
}

/// `layout_facts._Facts.run`, on the range's pages and blocks.
fn layout(fx: &F, g: &Global, fallback: &mut Option<String>) -> Value {
    let s = fx.s;
    let mut out = Map::new();
    let (lo, hi) = fx.near();
    // -- smallest_text
    let mut smallest: BTreeMap<&str, i64> = BTreeMap::new();
    if let Some(w) = &fx.wanted {
        for k in 0..s.pag.starts.len() {
            if !w.contains(&fx.number(k)) {
                continue;
            }
            for l in fx.lines(k).iter() {
                let a = &s.aux[l.item as usize];
                let (p, kind) = if l.row == u32::MAX {
                    (&a.paras[0], "body")
                } else {
                    match a
                        .paras
                        .iter()
                        .find(|p| p.row == l.row && p.cell == l.cell && p.idx == l.para)
                    {
                        Some(p) => (p, "table"),
                        None => continue,
                    }
                };
                let Some(key) = &p.key else { continue };
                if !fx.scoped.contains(key.as_str()) {
                    continue;
                }
                let (hp, _) = p.line_text.get(l.number as usize).copied().unwrap_or((0, false));
                if hp > 0 {
                    let e = smallest.entry(kind).or_insert(i64::MAX);
                    *e = (*e).min(hp);
                }
            }
        }
    }
    let mut st = Map::new();
    for (k, v) in &smallest {
        st.insert((*k).into(), json!(*v as f64 / 2.0));
    }
    out.insert("smallest_text".into(), Value::Object(st));
    // -- direct_formatting: the scope's paragraphs in order
    let mut entries: Vec<(i64, usize, Value)> = Vec::new();
    let mut order = 0usize;
    let each_para = |i: usize, f2: &mut dyn FnMut(&Value, usize, usize)| {
        if let Some(r) = rec(&s.inputs[i]) {
            match &*s.inputs[i] {
                ItemIn::Para(_) => f2(r, i, 0),
                ItemIn::Table(_) => {
                    for (k, p) in arr(Some(r), "paras").iter().enumerate() {
                        f2(p, i, k);
                    }
                }
            }
        }
    };
    for i in lo..hi {
        each_para(i, &mut |r: &Value, _i, _k| {
            order += 1;
            let Some(id) = r.get("id").and_then(Value::as_str) else {
                return;
            };
            if !fx.scoped.contains(id) {
                return;
            }
            let Some(df) = r.get("df") else { return };
            let pages = fx.pages_of(id);
            if !fx.shown(&pages) {
                return;
            }
            let mut e = df.clone();
            let props = e.get("props").and_then(Value::as_object).map_or(0, |m| m.len()) as i64;
            if let Value::Object(m) = &mut e {
                // "pages" after the entry's own keys, as `found["pages"] = pages` puts it
                m.insert("pages".into(), json!(pages));
            }
            entries.push((-props, order, e));
        });
    }
    entries.sort_by_key(|a| ((a.0 == 0), a.0, a.1));
    fx.capped(
        entries.into_iter().map(|e| e.2).collect(),
        "direct_formatting",
        &mut out,
    );
    // -- spacers
    let empty = |i: usize| -> bool {
        if !fx.is_para(i) || b(fx.rec(i), "content") {
            return false;
        }
        let found = fx.placed(i, 0);
        if found.is_empty() {
            return false;
        }
        !s.aux[i].paras[0].line_text.iter().any(|(_, ink)| *ink)
    };
    let space = |i: Option<usize>, side: &str| -> f64 {
        match i {
            Some(i) if fx.is_para(i) => f(fx.rec(i), side),
            _ => 0.0,
        }
    };
    let mut spacers = Vec::new();
    let n = s.inputs.len();
    let mut k = lo;
    // a run that reaches into [lo, hi) from before
    while k > 0 && empty(k) {
        k -= 1;
    }
    while k < hi {
        if !empty(k) {
            k += 1;
            continue;
        }
        let start = k;
        while k < n && empty(k) {
            k += 1;
        }
        let mut run: Vec<usize> = (start..k).collect();
        let before = if start > 0 { Some(start - 1) } else { None };
        let after = if k < n { Some(k) } else { None };
        match after {
            None => {
                run.pop();
            }
            Some(a) if run.len() == 1 && before.is_some_and(|x| !fx.is_para(x)) && !fx.is_para(a) => continue,
            _ => {}
        }
        if run.is_empty() {
            continue;
        }
        let Some((page, top)) = fx.first_line(run[0]) else {
            continue;
        };
        let ids: Vec<&str> = run.iter().map(|&i| fx.key(i).unwrap_or("")).collect();
        let mut e = Map::new();
        e.insert("paras".into(), json!(ids));
        e.insert("page".into(), json!(page));
        let previous_after = space(before, "sa");
        let above = previous_after.max(space(Some(run[0]), "sb"));
        let following = after.and_then(|a| fx.first_line(a));
        let target = if ids.len() == 1 {
            ids[0].to_string()
        } else {
            format!("{}..{}", ids[0], ids[ids.len() - 1])
        };
        e.insert(
            "fix".into(),
            json!({"tool": "word_delete", "calls": [{"target": target}]}),
        );
        if let Some((fpage, ftop)) = following.filter(|x| x.0 == page) {
            let _ = fpage;
            let gap = round(above + fx.pt(ftop - top), 1);
            e.insert(
                "height_pt".into(),
                json!(round(gap - previous_after.max(space(after, "sb")), 1)),
            );
            e.insert("next".into(), json!(fx.key(after.unwrap())));
            if fx.is_para(after.unwrap()) && gap <= 1584.0 {
                e.insert(
                    "keep_space".into(),
                    json!({"tool": "word_format", "calls": [{"targets": [fx.key(after.unwrap())], "space_before": gap}]}),
                );
            }
        } else {
            let bottom = run
                .iter()
                .flat_map(|&i| fx.placed(i, 0))
                .filter(|(p, _)| *p == page)
                .map(|(_, l)| l.top + l.pitch)
                .max()
                .unwrap_or(top);
            e.insert("height_pt".into(), json!(round(above + fx.pt(bottom - top), 1)));
            e.insert("page_end".into(), json!(true));
            if let Some(a) = after {
                e.insert("next".into(), json!(fx.key(a)));
            }
        }
        if fx.shown(&[page]) {
            spacers.push(Value::Object(e));
        }
    }
    spacers.sort_by(|a, b| {
        let x = a["height_pt"].as_f64().unwrap_or(0.0);
        let y = b["height_pt"].as_f64().unwrap_or(0.0);
        y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)
    });
    fx.capped(spacers, "spacers", &mut out);
    // -- empty_paragraphs
    let mut empties = Vec::new();
    let mut add_empty = |e: &Value| {
        let mut pages = BTreeSet::new();
        for id in arr(Some(e), "ids") {
            if let Some(id) = id.as_str() {
                pages.extend(fx.pages_of(id));
            }
        }
        let pages: Vec<usize> = pages.into_iter().collect();
        if fx.shown(&pages) {
            empties.push(e.clone());
        }
    };
    if let Some(e) = &g.empty_body {
        if arr(Some(e), "ids")
            .iter()
            .any(|x| x.as_str().is_some_and(|x| fx.subjects.contains(x)))
        {
            add_empty(e);
        }
    }
    for i in lo..hi {
        for e in arr(fx.rec(i), "er") {
            add_empty(e);
        }
    }
    fx.capped(empties, "empty_paragraphs", &mut out);
    // -- keep_with_next_at_foot
    let taller_kept: HashSet<usize> = (lo..hi)
        .filter(|&i| !fx.is_para(i) && b(fx.rec(i), "kt") && fx.item_pages(i).len() > 1)
        .collect();
    let mut kwn = Vec::new();
    if let Some(w) = &fx.wanted {
        for k in 0..s.pag.starts.len() {
            let number = fx.number(k);
            if !w.contains(&number) {
                continue;
            }
            let lines = fx.lines(k);
            let Some(line) = lines.iter().fold(None::<&PLine>, |best, l| match best {
                Some(x) if x.top >= l.top => Some(x),
                _ => Some(l),
            }) else {
                continue;
            };
            let i = line.item as usize;
            if line.row != u32::MAX {
                // a table's last row last on the page, kept with the next block
                let rows = arr(fx.rec(i), "rows");
                let row = line.row as usize;
                if i + 1 >= n || rows.is_empty() || row != rows.len() - 1 {
                    continue;
                }
                let last = &rows[rows.len() - 1];
                if last.get(0).is_none_or(Value::is_null) || !last.get(1).and_then(Value::as_bool).unwrap_or(false) {
                    continue;
                }
                let pages = fx.item_pages(i);
                if pages.last() != Some(&number) {
                    continue;
                }
                let Some(following) = fx.first_line(i + 1) else {
                    continue;
                };
                if following.0 <= number || fx.new_page_after(i) {
                    continue;
                }
                kwn.push(
                    json!({"table": fx.key(i), "row": row, "page": number, "next": fx.key(i + 1),
                                "next_page": following.0}),
                );
                continue;
            }
            if i + 1 >= n || !fx.is_para(i) {
                continue;
            }
            let Some(key) = fx.key(i) else { continue };
            if fx.pages_of(key).last() != Some(&number) {
                continue;
            }
            let Some(following) = fx.first_line(i + 1) else {
                continue;
            };
            if following.0 <= number {
                continue;
            }
            if !b(fx.rec(i), "kwn") || fx.new_page_after(i) {
                continue;
            }
            kwn.push(json!({"para": key, "style": s_or_null(fx.rec(i), "st"), "page": number,
                            "next": fx.key(i + 1), "next_page": following.0}));
        }
    }
    // _kept_rows: the tables drawn, in the order first drawn
    let drawn = drawn_rows(fx, lo, hi);
    for (i, rows) in &drawn {
        let i = *i;
        if fx
            .wanted
            .as_ref()
            .is_some_and(|w| !rows.values().any(|d| d.keys().any(|p| w.contains(p))))
        {
            continue;
        }
        let r = fx.rec(i);
        let own = arr(r, "rows").len();
        let header_rows = r.and_then(|r| r.get("hr")).and_then(Value::as_u64).unwrap_or(0) as usize;
        let rkn = arr(r, "rkn");
        let taller = taller_kept.contains(&i);
        for (&row, d) in rows {
            let Some(after) = rows.get(&(row + 1)) else { continue };
            if row < header_rows || row >= own || row >= rkn.len() {
                continue;
            }
            let page = *d.keys().max().unwrap();
            let next_page = *after.keys().min().unwrap();
            if next_page <= page || !fx.shown(&[page]) {
                continue;
            }
            if !rkn[row].as_bool().unwrap_or(false) {
                continue;
            }
            let mut e = json!({"table": fx.key(i), "row": row, "page": page, "next_row": row + 1,
                               "next_page": next_page});
            if taller {
                e["taller_than_page"] = json!(true);
            }
            kwn.push(e);
            if taller {
                break;
            }
        }
    }
    fx.capped(kwn, "keep_with_next_at_foot", &mut out);
    // -- table_parted
    let mut parted = Vec::new();
    let parted_kind = |i: usize, before: bool| -> Option<&'static str> {
        let r = fx.rec(i);
        if !b(r, "txt") {
            return None;
        }
        if b(r, "cap") {
            return Some("caption");
        }
        if before && b(r, "colon") {
            return Some("colon");
        }
        if fx.placed(i, 0).len() == 1 {
            return Some("one_line");
        }
        None
    };
    for i in lo..hi {
        if fx.is_para(i) {
            continue;
        }
        let pages = fx.item_pages(i);
        if pages.is_empty() {
            continue;
        }
        let tkey = fx.key(i);
        if i > 0 && fx.is_para(i - 1) {
            let lead = i - 1;
            let drawn = fx.item_pages(lead);
            if let Some(&last) = drawn.last() {
                if last < pages[0] && fx.shown(&[last, pages[0]]) {
                    if let Some(why) = parted_kind(lead, true) {
                        if !b(fx.rec(lead), "kwn") && !fx.new_page_after(lead) {
                            parted.push(json!({"table": tkey, "para": fx.key(lead), "side": "lead_in", "kind": why,
                                "para_page": last, "table_page": pages[0],
                                "fix": {"tool": "word_format", "calls": [{"targets": [fx.key(lead)], "keep_with_next": true}]}}));
                        }
                    }
                }
            }
        }
        if i + 1 < n && fx.is_para(i + 1) {
            let after = i + 1;
            let drawn = fx.item_pages(after);
            let tlast = *pages.last().unwrap();
            let Some(&first) = drawn.first() else { continue };
            if !(first > tlast && fx.shown(&[tlast, first])) {
                continue;
            }
            let Some(why) = parted_kind(after, false) else { continue };
            if b(fx.rec(after), "pbb") || b(fx.rec(after), "bs") {
                continue;
            }
            let rows = arr(fx.rec(i), "rows");
            let Some(last_row) = rows.last() else { continue };
            if last_row.get(1).and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            let mut e = json!({"table": tkey, "para": fx.key(after), "side": "line_after", "kind": why,
                               "row": rows.len() - 1, "row_page": tlast, "para_page": first});
            if pages.len() == 1 {
                let mut call = json!({"table": tkey, "scope": "table", "keep_with_next": true});
                if !b(fx.rec(i), "kt") {
                    call["keep_together"] = json!(true);
                }
                e["fix"] = json!({"tool": "word_format_table", "calls": [call]});
            } else if let Some(lead) = last_row.get(0).filter(|v| !v.is_null()) {
                e["fix"] = json!({"tool": "word_format", "calls": [{"targets": [lead], "keep_with_next": true}]});
            }
            parted.push(e);
        }
    }
    fx.capped(parted, "table_parted", &mut out);
    // -- numbering_skips_level
    let skips: Vec<Value> = g
        .numbering
        .iter()
        .filter(|e| {
            e["para"]
                .as_str()
                .is_some_and(|p| fx.subjects.contains(p) && fx.shown(&fx.pages_of(p)))
        })
        .cloned()
        .collect();
    fx.capped(skips, "numbering_skips_level", &mut out);
    // -- table_grid_mismatch, accept_loses_text: the scope's tables on the pages
    let mut mismatched = Vec::new();
    let mut ragged = Vec::new();
    let mut lost = Vec::new();
    for i in lo..hi {
        if fx.is_para(i) {
            continue;
        }
        let Some(key) = fx.key(i) else { continue };
        let pages = fx.item_pages(i);
        if fx.subjects.contains(key) && fx.shown(&pages) {
            for e in arr(fx.rec(i), "gm") {
                mismatched.push(with_pages(e, &pages));
            }
            for e in arr(fx.rec(i), "rg") {
                ragged.push(with_pages(e, &pages));
            }
        }
        if fx.shown(&pages) && fx.subjects.contains(key) {
            for e in arr(fx.rec(i), "al") {
                lost.push(with_pages(e, &pages));
            }
        }
    }
    mismatched.extend(ragged);
    fx.capped(mismatched, "table_grid_mismatch", &mut out);
    fx.capped(lost, "accept_loses_text", &mut out);
    // -- punctuation_spacing
    let mut punct = Vec::new();
    for i in lo..hi {
        each_para(i, &mut |r: &Value, _i, _k| {
            let Some(id) = r.get("id").and_then(Value::as_str) else {
                return;
            };
            if !fx.subjects.contains(id) {
                return;
            }
            let entries = arr(Some(r), "pu");
            if entries.is_empty() || !fx.shown(&fx.pages_of(id)) {
                return;
            }
            punct.extend(entries.iter().cloned());
        });
    }
    fx.capped(punct, "punctuation_spacing", &mut out);
    // -- text_language
    if let Some(found) = text_language(fx, lo, hi) {
        out.insert("text_language".into(), found);
    }
    // -- settings: keep_together
    let mut kt = Vec::new();
    for i in lo..hi {
        if fx.is_para(i) || !b(fx.rec(i), "kt") {
            continue;
        }
        let pages = fx.item_pages(i);
        if !pages.is_empty() && !fx.shown(&pages) {
            continue;
        }
        let direct: Vec<Value> = arr(fx.rec(i), "rows")
            .iter()
            .filter(|r| r.get(2).and_then(Value::as_bool).unwrap_or(false) && !r[0].is_null())
            .map(|r| r[0].clone())
            .collect();
        let mut e = json!({"table": fx.key(i), "pages": pages, "direct": direct});
        if b(fx.rec(i), "wn") {
            e["with_next"] = json!(true);
        }
        if taller_kept.contains(&i) {
            e["taller_than_page"] = json!(true);
        }
        kt.push(e);
    }
    let mut settings = Map::new();
    fx.capped(kt, "keep_together", &mut settings);
    out.insert("settings".into(), Value::Object(settings));
    // -- split_rows, header_not_repeated
    let mut split = Vec::new();
    let mut unrepeated = Vec::new();
    for (i, rows) in &drawn {
        let i = *i;
        let pages: BTreeSet<usize> = rows.values().flat_map(|d| d.keys().copied()).collect();
        if pages.len() < 2 || fx.wanted.as_ref().is_some_and(|w| !pages.iter().any(|p| w.contains(p))) {
            continue;
        }
        let r = fx.rec(i);
        let header_rows = r.and_then(|r| r.get("hr")).and_then(Value::as_u64).unwrap_or(0) as usize;
        let ordered: Vec<usize> = pages.iter().copied().collect();
        for (&row, d) in rows {
            if d.len() < 2 {
                continue;
            }
            let sets: HashSet<&Vec<(u32, u32, u32)>> = d.values().collect();
            let repeated = row < header_rows && sets.len() == 1;
            let dp: Vec<usize> = d.keys().copied().collect();
            if !repeated && fx.shown(&dp) {
                split.push(json!({"table": fx.key(i), "row": row, "pages": dp}));
            }
        }
        if !fx.shown(&ordered) {
            continue;
        }
        let head: BTreeSet<usize> = rows.get(&0).map(|d| d.keys().copied().collect()).unwrap_or_default();
        let missing: Vec<usize> = ordered[1..].iter().copied().filter(|p| !head.contains(p)).collect();
        if header_rows == 0 {
            let fix = json!({"tool": "word_format_table", "calls": [{"table": fx.key(i), "scope": "table", "header_rows": 1}]});
            let looks = arr(r, "hl");
            if !looks.is_empty() {
                unrepeated.push(
                    json!({"table": fx.key(i), "pages": ordered, "header_rows": 0, "header_look": looks,
                                       "fix": fix}),
                );
            } else if r.and_then(|r| r.get("cols")).and_then(Value::as_i64) != Some(2) {
                unrepeated.push(
                    json!({"table": fx.key(i), "pages": ordered, "header_rows": 0, "header_look": [],
                                       "fix_if_header": fix}),
                );
            }
        } else if !missing.is_empty() {
            unrepeated.push(json!({"table": fx.key(i), "pages": ordered, "header_rows": header_rows,
                                   "not_repeated_on": missing}));
        }
    }
    fx.capped(split, "split_rows", &mut out);
    fx.capped(unrepeated, "header_not_repeated", &mut out);
    // -- column_fit: none where no column of a table on the pages could be roomy beside one
    // that wraps (layout_facts._fit_columns' conditions, each with a point to spare: the
    // widths here are the measured ones, not the drawn ones); else not ported
    for (i, _) in &drawn {
        if fx.key(*i).is_some_and(|k| fx.subjects.contains(k)) && column_fit_possible(fx, *i) {
            *fallback = Some("column_fit: a column that may fit".into());
        }
    }
    out.insert("column_fit".into(), json!([]));
    // -- past_margins: a table's reach (docx-agent's record), else not ported
    let mut past = Vec::new();
    for (i, rows) in &drawn {
        let r = fx.rec(*i);
        match r.and_then(|r| r.get("inside")).and_then(Value::as_bool) {
            Some(true) => {}
            _ => {
                let _ = rows;
                *fallback = Some("past_margins: a table whose reach is not known to stay inside".into());
            }
        }
        let _ = &mut past;
    }
    fx.capped(past, "past_margins", &mut out);
    out.insert("uncertain".into(), json!([]));
    Value::Object(out)
}

/// Whether `_fit_columns` could find a column of table `i` to fit, with a point to spare.
fn column_fit_possible(fx: &F, i: usize) -> bool {
    const SLACK: f64 = 1.0;
    let r = fx.rec(i);
    let grid: Vec<f64> = arr(r, "grid").iter().filter_map(Value::as_f64).collect();
    let cells = arr(r, "cm");
    if grid.len() < 2 {
        return false;
    }

    struct Col {
        text_width: f64,
        margins: f64,
        widest: f64,
        word: f64,
        lines: usize,
    }
    let mut cols: BTreeMap<usize, Col> = BTreeMap::new();
    for p in &fx.s.aux[i].paras {
        let Some(row) = cells.get(p.row as usize).and_then(Value::as_array) else {
            return true;
        };
        let Some(cell) = row.get(p.cell as usize).and_then(Value::as_array) else {
            return true;
        };
        let num = |k: usize| cell.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let (start, span, left, right) = (num(0) as usize, num(1) as usize, num(2), num(3));
        if span != 1 || start >= grid.len() {
            continue;
        }
        let c = cols.entry(start).or_insert(Col {
            text_width: grid[start] - left - right,
            margins: left + right,
            widest: 0.0,
            word: 0.0,
            lines: 0,
        });
        c.text_width = c.text_width.min(grid[start] - left - right);
        c.margins = c.margins.max(left + right);
        let inked: Vec<&(i64, i64)> = p.line_ink.iter().filter(|x| x.0 >= 0).collect();
        c.lines = c.lines.max(inked.len());
        let unit = p.ink_unit as f64;
        for (ink, word) in inked {
            c.widest = c.widest.max(*ink as f64 / unit);
            c.word = c.word.max(*word as f64 / unit);
        }
    }
    for (&c, roomy) in &cols {
        if roomy.lines != 1 || roomy.text_width <= 0.0 || roomy.widest > 0.6 * roomy.text_width + SLACK {
            continue;
        }
        let need = roomy.widest.max(roomy.word) + 3.0 + roomy.margins;
        if grid[c] - need < 18.0 - SLACK {
            continue;
        }
        let beside = [c.wrapping_sub(1), c + 1]
            .iter()
            .any(|n| cols.get(n).is_some_and(|x| x.lines >= WRAP_LINES));
        if beside {
            return true;
        }
    }
    false
}

fn s_or_null(r: Option<&Value>, k: &str) -> Value {
    s(r, k).map_or(Value::Null, |x| json!(x))
}

fn with_pages(e: &Value, pages: &[usize]) -> Value {
    let mut e = e.clone();
    if let Value::Object(m) = &mut e {
        m.insert("pages".into(), json!(pages));
    }
    e
}

/// `_drawn_rows`: per table drawn on the pages near the range (in the order first drawn),
/// per row, per page, the lines drawn there `(cell, paragraph, line)`.
fn drawn_rows(fx: &F, lo: usize, hi: usize) -> Vec<(usize, BTreeMap<usize, BTreeMap<usize, Vec<(u32, u32, u32)>>>)> {
    let mut out: Vec<(usize, BTreeMap<usize, BTreeMap<usize, Vec<(u32, u32, u32)>>>)> = Vec::new();
    for i in lo..hi {
        if fx.is_para(i) {
            continue;
        }
        let (first, last) = fx.s.pages_of_item(i);
        let mut rows: BTreeMap<usize, BTreeMap<usize, Vec<(u32, u32, u32)>>> = BTreeMap::new();
        for k in first..last.min(fx.s.pag.starts.len()) {
            let number = fx.number(k);
            for l in fx.lines(k).iter() {
                if l.item as usize == i && l.row != u32::MAX {
                    rows.entry(l.row as usize)
                        .or_default()
                        .entry(number)
                        .or_default()
                        .push((l.cell, l.para, l.number));
                }
            }
        }
        for d in rows.values_mut() {
            for v in d.values_mut() {
                v.sort();
                v.dedup();
            }
        }
        if !rows.is_empty() {
            out.push((i, rows));
        }
    }
    out
}

/// `text_language.facts` of the scope's paragraphs on the pages (`whole` false: `checked`).
fn text_language(fx: &F, lo: usize, hi: usize) -> Option<Value> {
    struct J<'v> {
        id: &'v str,
        r: &'v Value,
    }
    let s = fx.s;
    let mut judged: Vec<J> = Vec::new();
    let mut short: HashSet<&str> = HashSet::new();
    let mut unclear = 0usize;
    for input in &s.inputs[lo..hi] {
        let Some(r) = rec(input) else { continue };
        let mine: Vec<&Value> = match &**input {
            ItemIn::Para(_) => vec![r],
            ItemIn::Table(_) => arr(Some(r), "paras").iter().collect(),
        };
        for r in mine {
            let Some(id) = r.get("id").and_then(Value::as_str) else {
                continue;
            };
            if !fx.scoped.contains(id) || !fx.shown(&fx.pages_of(id)) {
                continue;
            }
            if let Some(j) = r.get("lj") {
                judged.push(J { id, r: j });
            } else {
                match r.get("lw").and_then(Value::as_str) {
                    Some("too_short") => {
                        short.insert(id);
                    }
                    Some("unclear") => unclear += 1,
                    _ => {}
                }
            }
        }
    }
    if judged.is_empty() {
        return None;
    }
    let mut counts = Map::new();
    if !short.is_empty() {
        counts.insert("too_short".into(), json!(short.len()));
    }
    if unclear > 0 {
        counts.insert("unclear".into(), json!(unclear));
    }
    let c = &s.constants;
    let default_tag = c.get("lang_tag").cloned().unwrap_or(Value::Null);
    let default_source = c
        .get("lang_source")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .unwrap_or("none");
    let languages: Vec<&str> = c
        .get("languages")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut words: HashMap<&str, i64> = HashMap::new();
    for j in &judged {
        if j.r["conf"].as_f64().unwrap_or(0.0) >= MIN_CONFIDENCE {
            if let Some(l) = j.r["lang"].as_str() {
                *words.entry(l).or_insert(0) += j.r["words"].as_i64().unwrap_or(0);
            }
        }
    }
    let total: i64 = words.values().sum();
    let mut out = Map::new();
    if total == 0 {
        out.insert(
            "checked".into(),
            json!({"tag": default_tag, "source": default_source, "detected": null, "paragraphs": judged.len()}),
        );
        out.extend(counts);
        return Some(Value::Object(out));
    }
    let mut dominant = languages.first().copied().unwrap_or("");
    let mut best = -1i64;
    for l in &languages {
        let w = words.get(l).copied().unwrap_or(0);
        if w > best {
            best = w;
            dominant = l;
        }
    }
    out.insert(
        "checked".into(),
        json!({"tag": default_tag, "source": default_source, "detected": dominant,
               "share": round(words[dominant] as f64 / total as f64, 2), "words": total,
               "paragraphs": judged.len()}),
    );
    let mismatched = |j: &J| -> bool {
        j.r["profiled"].as_bool().unwrap_or(false)
            && j.r["prim"].as_str() != j.r["lang"].as_str()
            && j.r["conf"].as_f64().unwrap_or(0.0) >= MIN_CONFIDENCE
    };
    let listed: Vec<&J> = judged.iter().filter(|j| mismatched(j)).collect();
    // _grouped: consecutive top-level paragraphs alike as one range
    // a top-level paragraph's place among the body's blocks
    let order = |id: &str| -> Option<usize> { fx.items.get(id).copied().filter(|&i| fx.is_para(i)) };
    let alike = |x: &J, y: &J| -> bool {
        let (Some(a), Some(z)) = (order(x.id), order(y.id)) else {
            return false;
        };
        if (x.r["prim"].as_str(), x.r["source"].as_str(), x.r["lang"].as_str())
            != (y.r["prim"].as_str(), y.r["source"].as_str(), y.r["lang"].as_str())
        {
            return false;
        }
        (a + 1..z).all(|k| fx.is_para(k) && (fx.key(k).is_some_and(|key| short.contains(key)) || !b(fx.rec(k), "txt")))
    };
    let mut entries: Vec<Value> = Vec::new();
    let mut run: Vec<&J> = Vec::new();
    let flush = |run: &mut Vec<&J>, entries: &mut Vec<Value>| {
        if run.is_empty() {
            return;
        }
        let words: i64 = run.iter().map(|j| j.r["words"].as_i64().unwrap_or(0)).sum();
        let weighted: f64 = run
            .iter()
            .map(|j| j.r["conf"].as_f64().unwrap_or(0.0) * j.r["words"].as_i64().unwrap_or(0) as f64)
            .fold(0.0, |a, x| a + x);
        let confidence = round(weighted / words as f64, 2);
        let (first, last) = (run[0], run[run.len() - 1]);
        let mut e = Map::new();
        if run.len() == 1 {
            e.insert("para".into(), json!(first.id));
        } else {
            e.insert("range".into(), json!(format!("{}..{}", first.id, last.id)));
            e.insert(
                "paragraphs".into(),
                json!(order(last.id).unwrap_or(0) + 1 - order(first.id).unwrap_or(0)),
            );
        }
        e.insert("tag".into(), first.r["tag"].clone());
        e.insert(
            "source".into(),
            json!(first.r["source"].as_str().filter(|x| !x.is_empty()).unwrap_or("none")),
        );
        e.insert("detected".into(), first.r["lang"].clone());
        e.insert("confidence".into(), json!(confidence));
        e.insert("words".into(), json!(words));
        entries.push(Value::Object(e));
        run.clear();
    };
    for j in listed {
        if let Some(last) = run.last() {
            if !alike(last, j) {
                flush(&mut run, &mut entries);
            }
        }
        run.push(j);
    }
    flush(&mut run, &mut entries);
    if !entries.is_empty() {
        let n = entries.len();
        out.insert(
            "mismatched".into(),
            Value::Array(entries.into_iter().take(CAP).collect()),
        );
        if n > CAP {
            out.insert("mismatched_left_out".into(), json!(n - CAP));
        }
    }
    out.extend(counts);
    Some(Value::Object(out))
}

// -- since_last_check (`LayoutSummary.compare`) ---------------------------------------------

fn canon<'x>(aliases: &'x HashMap<String, String>, key: &'x str) -> &'x str {
    let mut k = key;
    let mut seen = 0;
    while let Some(next) = aliases.get(k) {
        k = next.as_str();
        seen += 1;
        if seen > aliases.len() {
            break;
        }
    }
    k
}

/// Per laid page: a digest of what it draws (its lines, by paragraph id through `aliases`,
/// where and how; its tables' rows; its header and footer's page number and count), and its
/// body's paragraph ids in drawing order.
fn page_digests(s: &Snap, aliases: &HashMap<String, String>) -> Vec<(u64, Vec<String>)> {
    use std::hash::{Hash, Hasher};
    let stories = s.constants.get("stories");
    let epoch = s.constants.get("epoch").map(|v| v.to_string()).unwrap_or_default();
    let pages = s.pag.info.len() as i64;
    let mut out = vec![(0u64, Vec::new()); s.pag.info.len()];
    for k in 0..s.pag.starts.len() {
        let index = s.pag.page_at[k] as usize;
        let info = &s.pag.info[index];
        let mut h = std::collections::hash_map::DefaultHasher::new();
        epoch.hash(&mut h);
        (info.section, info.story).hash(&mut h);
        // what the page's header and footer show of its number and the page count
        let fields = stories
            .and_then(|v| v.get(info.section as usize))
            .and_then(|v| v.get(info.story as usize))
            .and_then(Value::as_array);
        for kw in fields.into_iter().flatten().filter_map(Value::as_str) {
            match kw {
                "PAGE" => info.number.hash(&mut h),
                "NUMPAGES" => pages.hash(&mut h),
                _ => kw.hash(&mut h),
            }
        }
        let lines = page_lines(&s.doc, &s.flow, &s.pag, k);
        let mut ids = Vec::new();
        let mut last_row: Option<(u32, u32)> = None;
        for l in &lines {
            let a: &Aux = &s.aux[l.item as usize];
            let p = if l.row == u32::MAX {
                a.paras.first()
            } else {
                a.paras
                    .iter()
                    .find(|p| p.row == l.row && p.cell == l.cell && p.idx == l.para)
            };
            let key = p.and_then(|p| p.key.as_deref()).map(|k| canon(aliases, k));
            key.hash(&mut h);
            (l.top, l.pitch).hash(&mut h);
            if let Some(p) = p {
                p.line_draw.get(l.number as usize).hash(&mut h);
            }
            if l.row != u32::MAX {
                // a cell's line: where its table and cell are (the table's widths and borders)
                (a.draw, l.cell).hash(&mut h);
                if last_row != Some((l.item, l.row)) {
                    last_row = Some((l.item, l.row));
                    // the row by its table's id, not its place among the blocks
                    (a.key.as_deref().map(|k| canon(aliases, k)), l.row).hash(&mut h);
                }
            }
            if let Some(key) = key {
                ids.push(key.to_string());
            }
        }
        // a table row's borders: the rows drawn here, their tops and heights
        out[index] = (h.finish(), ids);
    }
    out
}

/// A page's (1-based) body lines as `page_digests` reads them (tests and measurements).
pub fn page_line_digests(s: &Snap, page: usize) -> Vec<(Option<String>, i64, i64, u64, u64, u32, u32)> {
    let Some(k) = (0..s.pag.starts.len()).find(|k| s.pag.page_at[*k] as usize + 1 == page) else {
        return Vec::new();
    };
    page_lines(&s.doc, &s.flow, &s.pag, k)
        .iter()
        .map(|l| {
            let a = &s.aux[l.item as usize];
            let p = if l.row == u32::MAX {
                a.paras.first()
            } else {
                a.paras
                    .iter()
                    .find(|p| p.row == l.row && p.cell == l.cell && p.idx == l.para)
            };
            let d = p.and_then(|p| p.line_draw.get(l.number as usize).copied()).unwrap_or(0);
            (p.and_then(|p| p.key.clone()), l.top, l.pitch, d, a.draw, l.row, l.cell)
        })
        .collect()
}

/// `since_last_check` against an earlier snapshot of the same document.
pub fn since(now: &Snap, before: &Snap, aliases: &HashMap<String, String>) -> Value {
    let a = page_digests(before, aliases);
    let b = page_digests(now, aliases);
    let count = a.len().max(b.len());
    let limit = a.len().min(b.len());
    let mut changed = Vec::new();
    for n in 1..=count.min(limit) {
        if a[n - 1].0 != b[n - 1].0 {
            changed.push(n);
        }
    }
    let unknown: Vec<usize> = (limit + 1..=count).collect();
    let first = |d: &Vec<(u64, Vec<String>)>| -> Vec<(String, usize)> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (n, (_, ids)) in d.iter().enumerate() {
            for id in ids {
                if seen.insert(id.clone()) {
                    out.push((id.clone(), n + 1));
                }
            }
        }
        out
    };
    let fb: HashMap<String, usize> = first(&a).into_iter().collect();
    let mut moved = Map::new();
    for (key, page) in first(&b) {
        if let Some(&p) = fb.get(&key) {
            if p != page && moved.len() < 20 {
                moved.insert(key, json!([p, page]));
            }
        }
    }
    let mut out = Map::new();
    out.insert("changed".into(), json!(changed));
    out.insert("pages".into(), json!([a.len(), b.len()]));
    if !moved.is_empty() {
        out.insert("moved".into(), Value::Object(moved));
    }
    if !unknown.is_empty() {
        out.insert("unknown".into(), json!(unknown));
    }
    Value::Object(out)
}

/// One check's answer: `{"version", "pages", "blocks", "range", "on", "layout", "since",
/// "pages_of", "fallback"}`.
pub fn answer(s: &Snap, req: &Request, seen: Option<&Snap>) -> Value {
    let g = s.global.get_or_init(|| make_global(s));
    let fx0 = F {
        s,
        laid: RefCell::new(HashMap::new()),
        subjects: req.subjects.iter().map(String::as_str).collect(),
        scoped: req.scoped.iter().map(String::as_str).collect(),
        wanted: None,
        caps: req.caps,
        items: s.keys(),
        paras: s.paras(),
    };
    let mut fallback = None;
    let mut out = Map::new();
    out.insert("version".into(), json!(s.version));
    out.insert("pages".into(), json!(s.page_count()));
    out.insert("blocks".into(), json!(s.inputs.len()));
    if !req.ranges.is_empty() {
        let (found, on) = ranges(&fx0, req, &mut fallback);
        out.insert("range".into(), Value::Array(found));
        out.insert("on".into(), json!(on));
        if req.layout {
            let fx = F {
                wanted: Some(on.iter().copied().collect()),
                ..fx0
            };
            out.insert("layout".into(), layout(&fx, g, &mut fallback));
        }
    }
    if req.since {
        if let Some(before) = seen {
            out.insert("since".into(), since(s, before, &req.aliases));
        }
    }
    if !req.pages_of.is_empty() {
        out.insert("pages_of".into(), json!(s.pages_of(&req.pages_of)));
    }
    if req.fields {
        match &g.fields {
            Ok(v) => {
                out.insert("fields".into(), v.clone());
            }
            Err(why) => fallback = Some(format!("fields: {why}")),
        }
    }
    if let Some(why) = fallback {
        out.insert("fallback".into(), json!(why));
    }
    Value::Object(out)
}
