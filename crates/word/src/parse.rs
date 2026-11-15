//! The body of a Word document read in Rust: its paragraphs and tables, runs, fields and
//! bookmarks as docx2svg's parser reads them (`docx2svg.parse.document`: the complex fields
//! threaded through the body, computed fields as one run, a table of contents' links, `REF`
//! fields replaced by their bookmarks' text), list labels counted as docx2svg's
//! `ListCounters` counts them, and the owner's first input (docx-agent's export,
//! `docx_agent.rust.mirror`) made from them -- byte for byte the export docx-agent's Python
//! makes.
//!
//! What a block's formatting resolves to (the style cascade, a run's character formats,
//! geometry, borders, a table's grid) is asked of a [`Resolver`] once per distinct input
//! (a document's thousands of paragraphs have a few dozen), as are the records docx-agent's
//! checks read.  Whatever this reader does not model refuses the document whole
//! ([`Refused`]); the caller then exports it as before.

use crate::xml::{self, Names, Node, Ns};
use num_integer::Integer;
use num_rational::Ratio;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;


pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";

/// Why a document is not read here (the caller exports it its own way).
#[derive(Debug, Clone)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn refuse<T>(why: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(why.into()))
}

/// What the parser asks of the document's own resolution (docx2svg's, in Python): each
/// question a kind and a JSON argument; the answer JSON text.  An error refuses the
/// document (the resolver's own `Unsupported`).
pub trait Resolver {
    fn call(&mut self, kind: &str, arg: &str) -> Result<String, String>;
}

type Q = Ratio<i128>;

// -- the model ---------------------------------------------------------------------------------

/// A computed field's run (docx2svg's `Field`).
#[derive(Clone, Debug)]
pub struct FieldRun {
    pub instruction: String,
    pub keyword: String,
}

/// One run as docx2svg reads it.
#[derive(Clone, Debug)]
pub struct Run {
    /// Its `w:rPr`, written out (the resolution's key); `None`: none.
    pub rpr: Option<String>,
    pub toc_link: bool,
    pub text: String,
    pub marks: Vec<(usize, String)>,
    pub field: Option<FieldRun>,
}

#[derive(Clone, Debug)]
pub struct Para {
    pub ppr: Option<String>,
    /// `w:pStyle`'s value as written.
    pub style_id: Option<String>,
    pub runs: Vec<Run>,
    /// `(start?, id, name, run index)`.
    pub bookmarks: Vec<(bool, Option<String>, Option<String>, usize)>,
    pub para_id: Option<String>,
    /// The table's style id (`""`: none named) for a cell paragraph, and the cell's
    /// conditional formats.
    pub table_style: Option<String>,
    pub conditions: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Table {
    /// `w:tblStyle` (`""`: none).
    pub style: String,
    /// rows -> cells -> paragraphs.
    pub rows: Vec<Vec<Vec<Para>>>,
    /// The table with its cells' runs left out: what its resolution reads.
    pub skeleton: String,
}

#[derive(Clone, Debug)]
pub enum Block {
    P(Para),
    T(Table),
}

impl Block {
    pub fn paragraphs(&self) -> Vec<&Para> {
        match self {
            Block::P(p) => vec![p],
            Block::T(t) => t.rows.iter().flat_map(|r| r.iter()).flat_map(|c| c.iter()).collect(),
        }
    }
}

/// One top-level body element and what it makes.
#[derive(Clone, Debug)]
pub struct Unit {
    pub key: Option<String>,
    pub block: Block,
    /// The fields begun in it and the bookmarks it starts (docx-agent's `records.fields_of`),
    /// and whether it is in a table of contents.
    pub fld: Vec<(String, Option<String>, Option<String>)>,
    pub bm: Vec<String>,
    pub toc: bool,
    /// The bookmarks its REF fields show.
    pub refs: Vec<String>,
    pub span: (usize, usize),
}

// -- the complex fields (docx2svg's `_Fields`) ------------------------------------------------

const COMPUTED: [&str; 7] = ["PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF", "REF", "SEQ", "STYLEREF"];

fn keyword(instruction: &str) -> String {
    instruction.split_whitespace().next().map(|w| w.to_uppercase()).unwrap_or_default()
}

fn merge_format(instruction: &str) -> bool {
    instruction.to_uppercase().replace(' ', "").contains("MERGEFORMAT")
}

#[derive(Clone, Debug)]
struct Open {
    visible: bool,
    instruction: Vec<String>,
    result_state: bool,
    computed: bool,
    instr_set: bool,
    instr_rpr: Option<String>,
    result_set: bool,
    result_rpr: Option<String>,
    result: Vec<String>,
}

#[derive(Default)]
struct Fields {
    stack: Vec<Open>,
    bookmarks: Option<Vec<(bool, Option<String>, Option<String>, usize)>>,
}

impl Fields {
    fn visible(&self) -> bool {
        self.stack.iter().all(|f| f.result_state && !f.computed)
    }
    fn in_toc_result(&self) -> bool {
        self.stack.iter().any(|f| f.result_state && keyword(&f.instruction.concat()) == "TOC")
    }
    fn computes(instruction: &str) -> bool {
        COMPUTED.contains(&keyword(instruction).as_str())
    }
}

/// The parser: the namespaces of the part and what it reads with.
pub struct Reader {
    pub names: Names,
    w: Ns,
    w14: Ns,
}

const RUN_MARKS: [&str; 5] = ["tab", "br", "cr", "noBreakHyphen", "softHyphen"];
const RUN_OBJECTS: [&str; 4] = ["drawing", "pict", "object", "AlternateContent"];

impl Reader {
    fn rpr_key(&self, rpr: Option<&Node>) -> Option<String> {
        rpr.map(|n| n.write(&self.names, Some(self.w)))
    }

    fn runs(&self, element: &Node, fields: &mut Fields, out: &mut Vec<Run>) -> Result<(), Refused> {
        let w = self.w;
        let properties = element.child(w, "rPr");
        let key = self.rpr_key(properties);
        let plain = fields.visible();
        let start = out.len();
        let mut text = String::new();
        let mut length = 0usize;
        let mut marks: Vec<(usize, String)> = Vec::new();
        let flush = |out: &mut Vec<Run>, text: &mut String, marks: &mut Vec<(usize, String)>, length: &mut usize| {
            if !text.is_empty() || !marks.is_empty() {
                out.push(Run {
                    rpr: key.clone(),
                    toc_link: false,
                    text: std::mem::take(text),
                    marks: std::mem::take(marks),
                    field: None,
                });
            }
            *length = 0;
        };
        for node in &element.children {
            if node.ns != w {
                // another namespace's content of a run (none docx2svg reads)
                if RUN_OBJECTS.contains(&node.name.as_str()) {
                    return refuse(format!("a run's {}", node.name));
                }
                continue;
            }
            let name = node.name.as_str();
            if name == "fldChar" {
                let kind = node.attr_any("fldCharType");
                match kind {
                    Some("begin") => {
                        let v = fields.visible();
                        fields.stack.push(Open {
                            visible: v,
                            instruction: vec![],
                            result_state: false,
                            computed: false,
                            instr_set: false,
                            instr_rpr: None,
                            result_set: false,
                            result_rpr: None,
                            result: vec![],
                        });
                    }
                    Some("separate") if !fields.stack.is_empty() => {
                        let top = fields.stack.last_mut().unwrap();
                        top.result_state = true;
                        top.computed = Fields::computes(&top.instruction.concat());
                    }
                    Some("end") if !fields.stack.is_empty() => {
                        let top = fields.stack.pop().unwrap();
                        let instruction = top.instruction.concat();
                        let computed = if top.result_state { top.computed } else { Fields::computes(&instruction) };
                        if top.visible && fields.visible() && computed {
                            flush(out, &mut text, &mut marks, &mut length);
                            let mut chosen = top.instr_rpr.clone();
                            if merge_format(&instruction) && top.result_set {
                                chosen = top.result_rpr.clone();
                            }
                            out.push(Run {
                                rpr: chosen,
                                toc_link: false,
                                text: top.result.concat(),
                                marks: vec![],
                                field: Some(FieldRun { instruction: instruction.trim().to_string(), keyword: keyword(&instruction) }),
                            });
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if name == "instrText" {
                if let Some(top) = fields.stack.last_mut() {
                    if !top.result_state {
                        top.instruction.push(node.text.clone());
                        if !top.instr_set {
                            top.instr_set = true;
                            top.instr_rpr = key.clone();
                        }
                    }
                }
                continue;
            }
            if !fields.visible() {
                let n = fields.stack.len();
                let outer_ok = fields.stack[..n - 1].iter().all(|f| f.result_state && !f.computed);
                let top = fields.stack.last_mut().unwrap();
                if top.computed && top.result_state && outer_ok && (name == "t" || name == "delText") {
                    top.result.push(node.text.clone());
                    if !top.result_set {
                        top.result_set = true;
                        // an empty w:rPr where the run has none: resolved alike
                        top.result_rpr = key.clone();
                    }
                }
                if RUN_OBJECTS.contains(&name) {
                    return refuse(format!("a {name} in a field"));
                }
                continue;
            }
            match name {
                "t" => {
                    text.push_str(&node.text);
                    length += node.text.chars().count();
                }
                "delText" => return refuse("deleted text"),
                _ if RUN_MARKS.contains(&name) => {
                    let kind = node.attr_any("type").unwrap_or(name).to_string();
                    marks.push((length, kind));
                }
                "footnoteReference" | "endnoteReference" | "endnoteRef" | "footnoteRef" | "separator"
                | "continuationSeparator" => return refuse("a note"),
                _ if RUN_OBJECTS.contains(&name) => return refuse(format!("a {name}")),
                "rPr" | "lastRenderedPageBreak" | "commentReference" | "annotationRef" => {}
                "sym" | "ptab" | "pgNum" | "dayShort" | "monthShort" | "yearShort" | "dayLong" | "monthLong"
                | "yearLong" | "contentPart" | "ruby" | "fldData" => return refuse(format!("a run's {name}")),
                _ => {}
            }
        }
        flush(out, &mut text, &mut marks, &mut length);
        if out.len() == start
            && plain
            && fields.visible()
            && !element.children.iter().any(|n| n.ns == w && (n.name == "fldChar" || n.name == "instrText"))
        {
            out.push(Run { rpr: key, toc_link: false, text: String::new(), marks: vec![], field: None });
        }
        Ok(())
    }

    fn simple_field(&self, node: &Node, fields: &mut Fields, out: &mut Vec<Run>) -> Result<(), Refused> {
        let w = self.w;
        let instruction = node.attr_any("instr").unwrap_or("").to_string();
        let inner: Vec<&Node> = node.children_named(w, "r").collect();
        if Fields::computes(&instruction) {
            if !fields.visible() {
                return Ok(());
            }
            let text: String = inner.iter().flat_map(|r| r.children_named(w, "t")).map(|t| t.text.as_str()).collect();
            let rpr = if !inner.is_empty() && merge_format(&instruction) { self.rpr_key(inner[0].child(w, "rPr")) } else { None };
            out.push(Run {
                rpr,
                toc_link: false,
                text,
                marks: vec![],
                field: Some(FieldRun { instruction: instruction.trim().to_string(), keyword: keyword(&instruction) }),
            });
            return Ok(());
        }
        for r in inner {
            self.runs(r, fields, out)?;
        }
        Ok(())
    }

    fn inline(&self, element: &Node, fields: &mut Fields, runs: &mut Vec<Run>) -> Result<(), Refused> {
        let w = self.w;
        for node in &element.children {
            if node.ns != w {
                return refuse(format!("a paragraph's {}", node.name));
            }
            match node.name.as_str() {
                "r" => self.runs(node, fields, runs)?,
                "fldSimple" => self.simple_field(node, fields, runs)?,
                "bookmarkStart" | "bookmarkEnd" => {
                    if fields.visible() {
                        let n = runs.len();
                        if let Some(b) = fields.bookmarks.as_mut() {
                            b.push((
                                node.name == "bookmarkStart",
                                node.attr_any("id").map(str::to_string),
                                node.attr_any("name").map(str::to_string),
                                n,
                            ));
                        }
                    }
                }
                "hyperlink" => {
                    let start = runs.len();
                    let toc = fields.in_toc_result();
                    self.inline(node, fields, runs)?;
                    if toc {
                        for r in &mut runs[start..] {
                            r.toc_link = true;
                        }
                    }
                }
                "pPr" | "proofErr" | "permStart" | "permEnd" | "commentRangeStart" | "commentRangeEnd" => {}
                other => return refuse(format!("a paragraph's {other}")),
            }
        }
        Ok(())
    }

    fn paragraph(&self, element: &Node, table_style: Option<&str>, conditions: &[&'static str], fields: &mut Fields) -> Result<Para, Refused> {
        let w = self.w;
        let ppr = element.child(w, "pPr");
        if let Some(p) = ppr {
            if p.child(w, "sectPr").is_some() {
                return refuse("a section break in a paragraph");
            }
            for n in p.descendants() {
                if n.ns == w && matches!(n.name.as_str(), "ins" | "del" | "moveFrom" | "moveTo" | "pPrChange" | "rPrChange") {
                    return refuse("a tracked change of a paragraph's properties");
                }
            }
        }
        let mut runs = Vec::new();
        let outer = fields.bookmarks.replace(Vec::new());
        let r = self.inline(element, fields, &mut runs);
        let bookmarks = std::mem::replace(&mut fields.bookmarks, outer).unwrap_or_default();
        r?;
        for r in &runs {
            if r.rpr.as_deref().is_some_and(|x| x.contains("Change") || x.contains(":ins") || x.contains(":del")) {
                // (a tracked change of a run's formatting: refused below by its key's parse)
            }
        }
        Ok(Para {
            ppr: ppr.map(|n| n.write(&self.names, Some(w))),
            style_id: ppr.and_then(|p| p.child(w, "pStyle")).and_then(|s| s.attr_any("val")).map(str::to_string),
            runs,
            bookmarks,
            para_id: element.attr(self.w14, "paraId").map(str::to_string),
            table_style: table_style.map(str::to_string),
            conditions: conditions.to_vec(),
        })
    }

    fn table(&self, element: &Node, fields: &mut Fields) -> Result<Table, Refused> {
        let w = self.w;
        let mut grid_seen = false;
        for c in &element.children {
            if c.ns != w {
                return refuse(format!("a table's {}", c.name));
            }
            match c.name.as_str() {
                "tblPr" | "tr" => {}
                "tblGrid" => grid_seen = true,
                "bookmarkStart" | "bookmarkEnd" | "proofErr" => return refuse("a bookmark between rows"),
                other => return refuse(format!("a table's {other}")),
            }
        }
        let _ = grid_seen;
        let tbl_pr = element.child(w, "tblPr");
        let style = tbl_pr.and_then(|p| p.child(w, "tblStyle")).and_then(|s| s.attr_any("val")).unwrap_or("").to_string();
        let look = table_look(tbl_pr.and_then(|p| p.child(w, "tblLook")));
        let rows_el: Vec<&Node> = element.children_named(w, "tr").collect();
        let mut rows = Vec::new();
        let n_rows = rows_el.len();
        for (k, row) in rows_el.iter().enumerate() {
            for c in &row.children {
                if c.ns != w || !matches!(c.name.as_str(), "trPr" | "tc" | "tblPrEx") {
                    return refuse(format!("a row's {}", c.name));
                }
                if c.name == "tblPrEx" {
                    return refuse("a row's table property exceptions");
                }
            }
            if let Some(tr_pr) = row.child(w, "trPr") {
                if tr_pr.children.iter().any(|n| matches!(n.name.as_str(), "del" | "ins" | "trPrChange")) {
                    return refuse("a tracked row");
                }
            }
            let cells: Vec<&Node> = row.children_named(w, "tc").collect();
            let mut out_cells = Vec::new();
            for (n, cell) in cells.iter().enumerate() {
                if let Some(tc_pr) = cell.child(w, "tcPr") {
                    if tc_pr.children.iter().any(|x| matches!(x.name.as_str(), "cellDel" | "cellIns" | "cellMerge" | "tcPrChange" | "vMerge" | "hMerge")) {
                        return refuse("a merged or tracked cell");
                    }
                }
                let conditions = cell_conditions(&look, k, n, n_rows, cells.len());
                let mut paras = Vec::new();
                for b in &cell.children {
                    if b.ns != w {
                        return refuse(format!("a cell's {}", b.name));
                    }
                    match b.name.as_str() {
                        "tcPr" => {}
                        "p" => paras.push(self.paragraph(b, Some(&style), &conditions, fields)?),
                        other => return refuse(format!("a cell's {other}")),
                    }
                }
                out_cells.push(paras);
            }
            rows.push(out_cells);
        }
        // the skeleton: the table's properties and structure, each cell's paragraphs with
        // their properties only
        let mut skel = element.clone();
        skel.text.clear();
        for row in skel.children.iter_mut() {
            if row.name != "tr" {
                continue;
            }
            row.attrs.clear();
            for cell in row.children.iter_mut() {
                if cell.name != "tc" {
                    continue;
                }
                for p in cell.children.iter_mut() {
                    if p.name == "p" {
                        p.attrs.clear();
                        p.children.retain(|c| c.ns == w && c.name == "pPr");
                        p.text.clear();
                    }
                }
            }
        }
        Ok(Table { style, rows, skeleton: skel.write(&self.names, Some(w)) })
    }
}

fn table_look(element: Option<&Node>) -> [bool; 6] {
    // firstRow, lastRow, firstColumn, lastColumn, noHBand, noVBand
    let names = ["firstRow", "lastRow", "firstColumn", "lastColumn", "noHBand", "noVBand"];
    let bits = [0x20, 0x40, 0x80, 0x100, 0x200, 0x400];
    match element {
        None => [true, false, true, false, false, true],
        Some(e) => {
            let mask = e.attr_any("val").unwrap_or("");
            let value = if !mask.is_empty() && mask.chars().all(|c| c.is_ascii_hexdigit()) {
                i64::from_str_radix(mask, 16).unwrap_or(0)
            } else {
                0
            };
            let mut out = [false; 6];
            for k in 0..6 {
                out[k] = match e.attr_any(names[k]) {
                    Some(v) => matches!(v.to_lowercase().as_str(), "1" | "true" | "on"),
                    None => value & bits[k] != 0,
                };
            }
            out
        }
    }
}

fn cell_conditions(look: &[bool; 6], row: usize, column: usize, rows: usize, columns: usize) -> Vec<&'static str> {
    let (row, column, rows, columns) = (row as i64, column as i64, rows as i64, columns as i64);
    let first_row = look[0] && row == 0;
    let last_row = look[1] && row == rows - 1;
    let first_col = look[2] && column == 0;
    let last_col = look[3] && column == columns - 1;
    let mut out = vec!["wholeTable"];
    if !look[5] && !(first_col || last_col) {
        let index = column - if look[2] { 1 } else { 0 };
        out.push(if index.rem_euclid(2) == 0 { "band1Vert" } else { "band2Vert" });
    }
    if !look[4] && !(first_row || last_row) {
        let index = row - if look[0] { 1 } else { 0 };
        out.push(if index.rem_euclid(2) == 0 { "band1Horz" } else { "band2Horz" });
    }
    for (name, on) in [("firstCol", first_col), ("lastCol", last_col), ("firstRow", first_row), ("lastRow", last_row)] {
        if on {
            out.push(name);
        }
    }
    if (first_row || last_row) && (first_col || last_col) {
        out.push(if first_col {
            if first_row { "nwCell" } else { "swCell" }
        } else if first_row {
            "neCell"
        } else {
            "seCell"
        });
    }
    out
}

// -- docx-agent's fields of a block (records.fields_of) ------------------------------------

const EMPTY_TOC: &str = "No table of contents entries found.";

fn py_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

fn target(instruction: &str) -> Option<String> {
    let words: Vec<&str> = instruction.split_whitespace().collect();
    if words.is_empty() || !matches!(words[0].to_uppercase().as_str(), "REF" | "PAGEREF" | "NOTEREF") {
        return None;
    }
    Some(words.get(1).map(|s| s.to_string()).unwrap_or_default())
}

/// One field docx-agent's scan finds: `[keyword, target, contents]`.
type Fld = (Option<String>, Option<String>, Option<String>);

/// `records.fields_of`: the fields begun in a block (the stack of those open before it
/// carried on) and the bookmarks it starts.
fn fields_of(names: &Names, w: Ns, element: &Node, stack: &mut Vec<(String, Option<bool>, Option<usize>)>) -> Result<(Vec<(String, Option<String>, Option<String>)>, Vec<String>), Refused> {
    let mut begun: Vec<Fld> = Vec::new();
    // [field index, text so far, ended, active]
    let mut tocs: Vec<(usize, String, bool)> = Vec::new();
    let _ = names;
    fn walk(n: &Node, parent: Option<&Node>, w: Ns, f: &mut dyn FnMut(&Node, Option<&Node>) -> Result<(), Refused>) -> Result<(), Refused> {
        f(n, parent)?;
        for c in &n.children {
            walk(c, Some(n), w, f)?;
        }
        Ok(())
    }
    let mut names_out = Vec::new();
    walk(element, None, w, &mut |node, parent| {
        if node.ns != w {
            return Ok(());
        }
        let tag = node.name.as_str();
        if tag == "t" && !tocs.is_empty() {
            for t in tocs.iter_mut() {
                if !t.2 {
                    t.1.push_str(&node.text);
                }
            }
            return Ok(());
        }
        if tag == "tab" && !tocs.is_empty() && parent.is_some_and(|p| p.ns == w && p.name == "r") {
            for t in tocs.iter_mut() {
                if !t.2 {
                    t.1.push('\t');
                }
            }
            return Ok(());
        }
        if tag == "p" && !tocs.is_empty() {
            for t in tocs.iter_mut() {
                if !t.2 {
                    t.1.push('\n');
                }
            }
            return Ok(());
        }
        if tag == "bookmarkStart" {
            names_out.push(node.attr_any("name").unwrap_or("").to_string());
        }
        match tag {
            "fldSimple" => {
                let instruction = node.attr_any("instr").unwrap_or("");
                let kw = keyword(instruction);
                let mut found: Fld = (Some(kw.clone()), target(instruction), None);
                if kw == "TOC" {
                    let text: String = node.descendants().iter().filter(|x| x.ns == w && x.name == "t").map(|x| x.text.as_str()).collect();
                    let s = py_strip(&text);
                    found.2 = Some(if s.is_empty() || s == EMPTY_TOC { "empty" } else { "some" }.into());
                }
                begun.push(found);
            }
            "instrText" => {
                if let Some(top) = stack.last_mut() {
                    if top.1.is_none() {
                        top.0.push_str(&node.text);
                    }
                }
            }
            "fldChar" => match node.attr_any("fldCharType") {
                Some("begin") => {
                    begun.push((None, None, None));
                    stack.push((String::new(), None, Some(begun.len() - 1)));
                }
                Some("separate") if !stack.is_empty() => {
                    let top = stack.last_mut().unwrap();
                    top.1 = Some(true);
                    if let Some(k) = top.2 {
                        begun[k].0 = Some(keyword(&top.0));
                        begun[k].1 = target(&top.0);
                        if begun[k].0.as_deref() == Some("TOC") {
                            tocs.push((k, String::new(), false));
                        }
                    }
                }
                Some("end") if !stack.is_empty() => {
                    let top = stack.pop().unwrap();
                    if let Some(k) = top.2 {
                        if begun[k].0.is_none() {
                            begun[k].0 = Some(keyword(&top.0));
                            begun[k].1 = target(&top.0);
                        }
                        for t in tocs.iter_mut() {
                            if t.0 == k {
                                t.2 = true;
                                let s = py_strip(t.1.trim_matches('\n'));
                                begun[k].2 = Some(if s.is_empty() || s == EMPTY_TOC { "empty" } else { "some" }.into());
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    for t in &tocs {
        if !t.2 {
            let s = py_strip(t.1.trim_matches('\n'));
            begun[t.0].2 = if !s.is_empty() && !EMPTY_TOC.starts_with(s) { Some("some".into()) } else { None };
        }
    }
    if stack.iter().any(|e| e.1.is_none()) {
        return refuse("a field's instruction going on past its block");
    }
    for e in stack.iter_mut() {
        e.2 = None;
    }
    let mut out = Vec::new();
    for f in begun {
        match f.0 {
            None => return refuse("a field begun with no instruction read"),
            Some(k) => out.push((k, f.1, f.2)),
        }
    }
    Ok((out, names_out))
}

// -- the body ----------------------------------------------------------------------------------

/// The body read: its units, in order, and the reader's namespaces.
pub struct Body {
    pub reader: Reader,
    pub units: Vec<Unit>,
}

/// Reads `document.xml`'s body.
pub fn read_body(data: &[u8]) -> Result<Body, Refused> {
    let mut names = Names::new();
    let root = xml::parse(data, &mut names).map_err(|e| Refused(e.to_string()))?;
    let w = names.intern(W);
    let w14 = names.intern(W14);
    let reader = Reader { names, w, w14 };
    let body = root.child(w, "body").ok_or_else(|| Refused("no body".into()))?;
    let mut fields = Fields::default();
    let mut units = Vec::new();
    let mut fstack: Vec<(String, Option<bool>, Option<usize>)> = Vec::new();
    let n = body.children.len();
    for (k, child) in body.children.iter().enumerate() {
        if child.ns != w {
            return refuse(format!("a body's {}", child.name));
        }
        let toc_before = fstack.iter().any(|e| keyword(&e.0) == "TOC");
        let block = match child.name.as_str() {
            "p" => Block::P(reader.paragraph(child, None, &[], &mut fields)?),
            "tbl" => Block::T(reader.table(child, &mut fields)?),
            "sectPr" if k == n - 1 => continue,
            other => return refuse(format!("a body's {other}")),
        };
        let (fld, bm) = fields_of(&reader.names, w, child, &mut fstack)?;
        let toc = toc_before || fld.iter().any(|f| f.0 == "TOC");
        let key = match &block {
            Block::P(p) => p.para_id.as_ref().map(|r| format!("p:{}", r.to_uppercase())),
            Block::T(_) => child
                .child(w, "tr")
                .and_then(|tr| tr.attr(w14, "paraId"))
                .map(|r| format!("t:{}", r.to_uppercase())),
        };
        let mut refs = Vec::new();
        for node in child.descendants() {
            if node.ns == w && (node.name == "instrText" || node.name == "fldSimple") {
                let text = if node.name == "instrText" { node.text.as_str() } else { node.attr_any("instr").unwrap_or("") };
                let words: Vec<&str> = text.split_whitespace().collect();
                if words.len() > 1 && words[0].to_uppercase() == "REF" && !refs.contains(&words[1].to_string()) {
                    refs.push(words[1].to_string());
                }
            }
        }
        units.push(Unit { key, block, fld, bm, toc, refs, span: child.span });
    }
    if !fields.stack.is_empty() {
        return refuse("a complex field open at the body's end");
    }
    let mut out = Body { reader, units };
    cross_references(&mut out)?;
    Ok(out)
}

/// docx2svg's `fields.cross_references` for what this reader supports: `REF` fields with
/// at most `\h`, their bookmark in one paragraph of the body, replaced by its runs; `SEQ` is
/// refused (the export refuses it too).
fn cross_references(body: &mut Body) -> Result<(), Refused> {
    // every paragraph of the body, in order
    let mut index: Vec<(usize, usize)> = Vec::new(); // (unit, paragraph within it)
    for (u, unit) in body.units.iter().enumerate() {
        for k in 0..unit.block.paragraphs().len() {
            index.push((u, k));
        }
    }
    let para = |units: &Vec<Unit>, i: usize| -> Para { units[index[i].0].block.paragraphs()[index[i].1].clone() };
    let mut has_ref = false;
    for unit in &body.units {
        for p in unit.block.paragraphs() {
            for r in &p.runs {
                if let Some(f) = &r.field {
                    match f.keyword.as_str() {
                        "SEQ" => return refuse("a SEQ field"),
                        "STYLEREF" => return refuse("a STYLEREF field"),
                        "REF" => has_ref = true,
                        _ => {}
                    }
                }
            }
        }
    }
    if !has_ref {
        return Ok(());
    }
    // bookmarks: name -> (start (para, run), end)
    let mut starts: HashMap<String, (Option<String>, (usize, usize))> = HashMap::new();
    let mut ends: HashMap<Option<String>, (usize, usize)> = HashMap::new();
    for i in 0..index.len() {
        let p = para(&body.units, i);
        for (start, id, name, run) in &p.bookmarks {
            if *start {
                if let Some(n) = name {
                    if !n.is_empty() && !starts.contains_key(n) {
                        starts.insert(n.clone(), (id.clone(), (i, *run)));
                    }
                }
            } else {
                ends.entry(id.clone()).or_insert((i, *run));
            }
        }
    }
    let mut replaced: Vec<(usize, Vec<Run>, Vec<(bool, Option<String>, Option<String>, usize)>)> = Vec::new();
    for i in 0..index.len() {
        let p = para(&body.units, i);
        if !p.runs.iter().any(|r| r.field.as_ref().is_some_and(|f| f.keyword == "REF")) {
            continue;
        }
        let mut runs = Vec::new();
        let mut index_of: HashMap<usize, usize> = HashMap::new();
        for (k, run) in p.runs.iter().enumerate() {
            index_of.insert(k, runs.len());
            let f = match &run.field {
                Some(f) if f.keyword == "REF" => f,
                _ => {
                    runs.push(run.clone());
                    continue;
                }
            };
            match reference_runs(&body.units, &index, run, f, &starts, &ends)? {
                Some(rs) => runs.extend(rs),
                None => runs.push(run.clone()),
            }
        }
        index_of.insert(p.runs.len(), runs.len());
        let moved = p.bookmarks.iter().map(|(s, id, name, k)| (*s, id.clone(), name.clone(), *index_of.get(k).unwrap_or(k))).collect();
        replaced.push((i, runs, moved));
    }
    for (i, runs, marks) in replaced {
        let (u, k) = index[i];
        let p: &mut Para = match &mut body.units[u].block {
            Block::P(p) => p,
            Block::T(t) => t.rows.iter_mut().flat_map(|r| r.iter_mut()).flat_map(|c| c.iter_mut()).nth(k).unwrap(),
        };
        p.runs = runs;
        p.bookmarks = marks;
    }
    Ok(())
}

/// `fields.reference_runs`; `None` where Word's result cannot be computed (the field is
/// drawn as cached, as docx2svg draws it).
fn reference_runs(
    units: &[Unit],
    index: &[(usize, usize)],
    run: &Run,
    field: &FieldRun,
    starts: &HashMap<String, (Option<String>, (usize, usize))>,
    ends: &HashMap<Option<String>, (usize, usize)>,
) -> Result<Option<Vec<Run>>, Refused> {
    let instruction = &field.instruction;
    let words: Vec<&str> = instruction.split_whitespace().collect();
    // switches: only \h is computed; anything else refused here (not ported)
    for w in &words[1..] {
        if w.starts_with('\\') && *w != "\\h" && *w != "\\H" {
            return refuse(format!("a REF switch {w}"));
        }
    }
    if instruction.contains('"') {
        return refuse("a quoted REF argument");
    }
    let name = match words.get(1) {
        Some(n) if !n.starts_with('\\') => n.to_string(),
        _ => return Ok(None),
    };
    let (mark, (sb, sr)) = match starts.get(&name) {
        Some(x) => x.clone(),
        None => return Ok(None),
    };
    let (eb, er) = match ends.get(&mark) {
        Some(e) => *e,
        None => return Ok(None),
    };
    if eb != sb {
        return Ok(None);
    }
    let (u, k) = index[sb];
    let paras = units[u].block.paragraphs();
    let source = &paras[k].runs;
    if sr > er || er > source.len() {
        return refuse("a bookmark's end before its start");
    }
    let source = &source[sr..er];
    for piece in source {
        if !piece.marks.is_empty() {
            return Ok(None);
        }
        if piece.field.as_ref().is_some_and(|f| matches!(f.keyword.as_str(), "PAGE" | "NUMPAGES" | "SECTIONPAGES" | "PAGEREF")) {
            return Ok(None);
        }
    }
    let text: String = source.iter().map(|p| p.text.as_str()).collect();
    if text.is_empty() {
        return Ok(Some(vec![Run { field: None, text: String::new(), ..run.clone() }]));
    }
    // split where the properties' object changes: each source piece is its own
    let mut out = Vec::new();
    for piece in source {
        if piece.text.is_empty() {
            continue;
        }
        out.push(Run { rpr: piece.rpr.clone(), toc_link: run.toc_link, text: piece.text.clone(), marks: vec![], field: None });
    }
    Ok(Some(out))
}

// -- what the resolver answers ------------------------------------------------------------------

/// A character format as docx2svg's exporter keys it, before its font is indexed:
/// `(face, bold, italic, half points, width half points, position, border)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FmtKey {
    face: Option<String>,
    bold: bool,
    italic: bool,
    hp: i64,
    whp: i64,
    position: i64,
    border: Option<(i64, i64)>,
}

fn fmt_key(v: &Value) -> Result<FmtKey, Refused> {
    let a = v.as_array().ok_or_else(|| Refused("a format".into()))?;
    Ok(FmtKey {
        face: a[0].as_str().map(str::to_string),
        bold: a[1].as_bool().unwrap_or(false),
        italic: a[2].as_bool().unwrap_or(false),
        hp: a[3].as_i64().unwrap_or(0),
        whp: a[4].as_i64().unwrap_or(0),
        position: a[5].as_i64().unwrap_or(0),
        border: a[6].as_array().map(|b| (b[0].as_i64().unwrap_or(0), b[1].as_i64().unwrap_or(0))),
    })
}

/// What a run's resolved properties give per character slot.
#[derive(Clone, Debug)]
struct RunInfo {
    vanish: bool,
    caps: bool,
    tracking: i64,
    scale: i64,
    kern: i64,
    va: bool,
    draw: String,
    complex: bool,
    hint: Option<String>,
    ea_lang: Option<String>,
    /// per slot (ascii, hAnsi, eastAsia, cs): (format with its width size, format without)
    slots: Vec<(FmtKey, FmtKey)>,
}

fn slot_index(c: char, complex: bool, hint: Option<&str>, ea_lang: Option<&str>) -> usize {
    if complex {
        return 3;
    }
    let code = c as u32;
    if code < 0x80 {
        return 0;
    }
    const EA: [(u32, u32); 11] = [
        (0x1100, 0x11FF),
        (0x2E80, 0x2FDF),
        (0x2FF0, 0x31FF),
        (0x3200, 0x4DBF),
        (0x4E00, 0x9FFF),
        (0xA000, 0xA4CF),
        (0xAC00, 0xD7AF),
        (0xF900, 0xFAFF),
        (0xFE30, 0xFE4F),
        (0xFF00, 0xFFEF),
        (0x20000, 0x2FFFF),
    ];
    if EA.iter().any(|(a, b)| *a <= code && code <= *b) {
        return 2;
    }
    if hint == Some("eastAsia") {
        if (0xA0..=0xFF).contains(&code) {
            let hinted = [0xA1, 0xA4, 0xA7, 0xA8, 0xAA, 0xAD, 0xAF, 0xD7, 0xF7].contains(&code)
                || (0xB0..0xB5).contains(&code)
                || (0xB6..0xBB).contains(&code)
                || (0xBC..0xC0).contains(&code);
            if hinted {
                return 2;
            }
            let chinese = ea_lang.unwrap_or("").to_lowercase().starts_with("zh");
            let hc = [0xE0, 0xE1, 0xE8, 0xE9, 0xEA, 0xEC, 0xED, 0xF2, 0xF3, 0xF9, 0xFA, 0xFC].contains(&code);
            return if chinese && hc { 2 } else { 1 };
        }
        return 2;
    }
    1
}

fn upper(c: char) -> Result<char, Refused> {
    if c.is_ascii() {
        return Ok(c.to_ascii_uppercase());
    }
    let mut u = c.to_uppercase();
    match (u.next(), u.next()) {
        (Some(x), None) if (x as u32) < 0x250 || x == c => Ok(x),
        _ => refuse("capitals of a character not read here"),
    }
}

// -- the export --------------------------------------------------------------------------------

/// The exporter's tables (docx-agent's `Exporter`): formats, fonts and styles indexed in
/// the order first asked, the characters measured.
#[derive(Default)]
struct Ex {
    fonts: HashMap<(String, bool, bool), usize>,
    font_list: Vec<(String, bool, bool)>,
    formats: HashMap<(i64, i64, i64, i64, Option<(i64, i64)>), usize>,
    format_list: Vec<(i64, i64, i64, i64, Option<(i64, i64)>)>,
    styles: HashMap<Option<String>, i64>,
    style_count: usize,
    chars: HashSet<char>,
    drawn: HashSet<(Option<String>, char)>,
    kern_fonts: HashSet<i64>,
    face_log: Vec<String>,
}

impl Ex {
    fn new() -> Ex {
        let mut e = Ex::default();
        e.styles.insert(None, -1);
        e.style_count = 1;
        for c in "0123456789".chars() {
            e.chars.insert(c);
        }
        e
    }
    fn style(&mut self, name: Option<&str>) -> i64 {
        let key = name.map(str::to_string);
        if let Some(k) = self.styles.get(&key) {
            return *k;
        }
        let k = self.style_count as i64 - 1;
        self.styles.insert(key, k);
        self.style_count += 1;
        k
    }
    fn fmt(&mut self, f: &FmtKey, with_width: bool) -> usize {
        if let Some(face) = &f.face {
            if !self.face_log.contains(face) {
                self.face_log.push(face.clone());
            }
        }
        let font = match &f.face {
            Some(face) => {
                let key = (face.clone(), f.bold, f.italic);
                match self.fonts.get(&key) {
                    Some(k) => *k as i64,
                    None => {
                        self.fonts.insert(key.clone(), self.font_list.len());
                        self.font_list.push(key);
                        self.font_list.len() as i64 - 1
                    }
                }
            }
            None => -1,
        };
        let key = (font, f.hp, if with_width { f.whp } else { f.hp }, f.position, f.border);
        if let Some(k) = self.formats.get(&key) {
            return *k;
        }
        self.formats.insert(key, self.format_list.len());
        self.format_list.push(key);
        self.format_list.len() - 1
    }
}

/// A length the export sends in px or in layout-unit widths: exact.
#[derive(Clone, Copy, Debug)]
enum Len {
    Px(Q),
    Wd(Q),
}

/// The export's values: JSON, with exact lengths encoded once their denominators are known.
#[derive(Clone, Debug)]
enum V {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Len(Len),
    List(Vec<V>),
    Obj(Vec<(&'static str, V)>),
    /// JSON text made elsewhere (Python's), as it is.
    Raw(String),
}

fn q_of(v: &Value) -> Result<Q, Refused> {
    let a = v.as_array().ok_or_else(|| Refused(format!("a fraction {v}")))?;
    Ok(Q::new(a[0].as_i64().unwrap_or(0) as i128, a[1].as_i64().unwrap_or(1) as i128))
}

fn opt_str(v: &Value) -> V {
    match v.as_str() {
        Some(s) => V::Str(s.into()),
        None => V::Null,
    }
}

/// Python's `json.dumps` of a string (ASCII only, as it writes by default).
pub fn py_json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{:04x}", u);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Dens {
    px: i128,
    w: i128,
}

fn dens(v: &V, d: &mut Dens) {
    match v {
        V::Len(Len::Px(q)) => d.px = d.px.lcm(q.denom()),
        V::Len(Len::Wd(q)) => d.w = d.w.lcm(q.denom()),
        V::List(l) => l.iter().for_each(|x| dens(x, d)),
        V::Obj(o) => o.iter().for_each(|(_, x)| dens(x, d)),
        _ => {}
    }
}

fn encode(v: &V, d: &Dens, out: &mut String) -> Result<(), Refused> {
    match v {
        V::Null => out.push_str("null"),
        V::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        V::Int(i) => {
            let _ = write!(out, "{i}");
        }
        V::Str(s) => py_json_str(s, out),
        V::Len(l) => {
            let x = match l {
                Len::Px(q) => *q * Q::from_integer(d.px),
                Len::Wd(q) => *q * Q::from_integer(d.w),
            };
            if !x.is_integer() {
                return refuse("a length off the export's denominators");
            }
            let _ = write!(out, "{}", x.to_integer());
        }
        V::List(l) => {
            out.push('[');
            for (k, x) in l.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                encode(x, d, out)?;
            }
            out.push(']');
        }
        V::Obj(o) => {
            out.push('{');
            for (k, (name, x)) in o.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                py_json_str(name, out);
                out.push(':');
                encode(x, d, out)?;
            }
            out.push('}');
        }
        V::Raw(s) => out.push_str(s),
    }
    Ok(())
}

/// What the resolver said of one paragraph key.
#[derive(Clone, Debug)]
struct ParaInfo {
    v: Value,
}

impl ParaInfo {
    fn i(&self, k: &str) -> i64 {
        self.v[k].as_i64().unwrap_or(0)
    }
    fn b(&self, k: &str) -> bool {
        self.v[k].as_bool().unwrap_or(false)
    }
}

/// List counting: docx2svg's `ListCounters`, the levels as the resolver gives them.
#[derive(Default)]
struct Counters {
    counts: HashMap<String, Vec<(i64, i64)>>,
    used: HashSet<(i64, i64)>,
}

#[derive(Clone, Debug)]
struct Level {
    text: Option<String>,
    start: Option<i64>,
    format: Option<String>,
    legal: bool,
    restart: Option<i64>,
}

#[derive(Clone, Debug)]
struct Levels {
    key: String,
    restarts: Vec<i64>,
    levels: HashMap<i64, Level>,
}

fn start_of(l: Option<&Level>) -> i64 {
    l.and_then(|l| l.start).unwrap_or(1)
}

fn roman(mut n: i64) -> String {
    if n <= 0 {
        return n.to_string();
    }
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, t) in table {
        while n >= v {
            s.push_str(t);
            n -= v;
        }
    }
    s
}

fn format_number(value: i64, fmt: Option<&str>) -> Result<String, Refused> {
    Ok(match fmt {
        Some("lowerLetter") | Some("upperLetter") => {
            if value < 1 {
                return refuse("a letter numbering below 1");
            }
            let c = (b'a' + ((value - 1) % 26) as u8) as char;
            let s: String = std::iter::repeat_n(c, ((value - 1) / 26 + 1) as usize).collect();
            if fmt == Some("upperLetter") {
                s.to_uppercase()
            } else {
                s
            }
        }
        Some("lowerRoman") | Some("upperRoman") => {
            if value <= 0 {
                return refuse("a roman numbering below 1");
            }
            let r = roman(value);
            if fmt == Some("upperRoman") {
                r.to_uppercase()
            } else {
                r
            }
        }
        Some("bullet") | Some("none") => String::new(),
        _ => value.to_string(),
    })
}

impl Counters {
    fn count(&mut self, lv: &Levels, num_id: i64, ilvl: i64) -> Result<Option<String>, Refused> {
        let level = match lv.levels.get(&ilvl) {
            Some(l) if l.text.is_some() => l.clone(),
            _ => return Ok(None),
        };
        let counts = self.counts.entry(lv.key.clone()).or_default();
        let first = !self.used.contains(&(num_id, ilvl));
        self.used.insert((num_id, ilvl));
        let get = |c: &Vec<(i64, i64)>, k: i64| c.iter().find(|x| x.0 == k).map(|x| x.1);
        let set = |c: &mut Vec<(i64, i64)>, k: i64, v: i64| match c.iter_mut().find(|x| x.0 == k) {
            Some(x) => x.1 = v,
            None => c.push((k, v)),
        };
        if first && lv.restarts.contains(&ilvl) {
            set(counts, ilvl, start_of(Some(&level)));
        } else {
            let v = get(counts, ilvl).unwrap_or(start_of(Some(&level)) - 1) + 1;
            set(counts, ilvl, v);
        }
        for shallower in 0..ilvl {
            if get(counts, shallower).is_none() {
                set(counts, shallower, start_of(lv.levels.get(&shallower)));
            }
        }
        let deeper: Vec<i64> = counts.iter().map(|x| x.0).filter(|k| *k > ilvl).collect();
        for d in deeper {
            let restart = lv.levels.get(&d).and_then(|l| l.restart);
            if ilvl < restart.unwrap_or(d) {
                counts.retain(|x| x.0 != d);
            }
        }
        let mut text = level.text.clone().unwrap();
        for number in 0..9 {
            let token = format!("%{}", number + 1);
            if text.contains(&token) {
                let other = lv.levels.get(&number);
                let value = get(counts, number).unwrap_or(start_of(other));
                let mut fmt = other.and_then(|o| o.format.clone());
                if level.legal && !matches!(fmt.as_deref(), Some("bullet") | Some("none")) {
                    fmt = Some("decimal".into());
                }
                text = text.replace(&token, &format_number(value, fmt.as_deref())?);
            }
        }
        Ok(Some(text))
    }
}

/// The export's work: the resolver's answers cached by their keys.
pub struct Exporter<'a> {
    r: &'a mut dyn Resolver,
    paras: HashMap<String, ParaInfo>,
    runs: HashMap<String, RunInfo>,
    marks: HashMap<String, Value>,
    geoms: HashMap<String, V>,
    borders: HashMap<String, (Q, Q)>,
    levels: HashMap<i64, Option<Levels>>,
    ex: Ex,
    pub calls: HashMap<&'static str, usize>,
}

fn pkey(p: &Para) -> String {
    let mut s = String::new();
    py_json_str(p.ppr.as_deref().unwrap_or(""), &mut s);
    s.push('|');
    match &p.table_style {
        None => s.push('-'),
        Some(t) => py_json_str(t, &mut s),
    }
    s.push('|');
    s.push_str(&p.conditions.join(","));
    s
}

fn pkey_json(p: &Para) -> Value {
    json!({"ppr": p.ppr, "tbl": p.table_style, "cond": p.conditions})
}

impl<'a> Exporter<'a> {
    pub fn new(r: &'a mut dyn Resolver) -> Exporter<'a> {
        Exporter {
            r,
            paras: HashMap::new(),
            runs: HashMap::new(),
            marks: HashMap::new(),
            geoms: HashMap::new(),
            borders: HashMap::new(),
            levels: HashMap::new(),
            ex: Ex::new(),
            calls: HashMap::new(),
        }
    }

    fn ask(&mut self, kind: &'static str, arg: &Value) -> Result<Value, Refused> {
        *self.calls.entry(kind).or_default() += 1;
        let text = self.r.call(kind, &arg.to_string()).map_err(Refused)?;
        serde_json::from_str(&text).map_err(|e| Refused(format!("{kind}: {e}")))
    }

    fn ask_raw(&mut self, kind: &'static str, arg: &Value) -> Result<String, Refused> {
        *self.calls.entry(kind).or_default() += 1;
        self.r.call(kind, &arg.to_string()).map_err(Refused)
    }

    fn para(&mut self, p: &Para) -> Result<ParaInfo, Refused> {
        let k = pkey(p);
        if let Some(found) = self.paras.get(&k) {
            return Ok(found.clone());
        }
        let v = self.ask("para", &pkey_json(p))?;
        let info = ParaInfo { v };
        self.paras.insert(k, info.clone());
        Ok(info)
    }

    fn run(&mut self, p: &Para, r: &Run) -> Result<RunInfo, Refused> {
        let mut k = pkey(p);
        k.push('|');
        py_json_str(r.rpr.as_deref().unwrap_or(""), &mut k);
        k.push_str(if r.rpr.is_none() { "-" } else { "+" });
        k.push_str(if r.toc_link { "|t" } else { "|f" });
        if let Some(found) = self.runs.get(&k) {
            return Ok(found.clone());
        }
        let v = self.ask("run", &json!({"p": pkey_json(p), "rpr": r.rpr, "toc": r.toc_link}))?;
        let mut slots = Vec::new();
        for s in v["slots"].as_array().ok_or_else(|| Refused("slots".into()))? {
            slots.push((fmt_key(&s[0])?, fmt_key(&s[1])?));
        }
        let info = RunInfo {
            vanish: v["vanish"].as_bool().unwrap_or(false),
            caps: v["caps"].as_bool().unwrap_or(false),
            tracking: v["tracking"].as_i64().unwrap_or(0),
            scale: v["scale"].as_i64().unwrap_or(100),
            kern: v["kern"].as_i64().unwrap_or(0),
            va: v["va"].as_bool().unwrap_or(false),
            draw: v["draw"].as_str().unwrap_or("").to_string(),
            complex: v["cs"].as_bool().unwrap_or(false),
            hint: v["hint"].as_str().map(str::to_string),
            ea_lang: v["ea_lang"].as_str().map(str::to_string),
            slots,
        };
        self.runs.insert(k, info.clone());
        Ok(info)
    }

    fn mark(&mut self, p: &Para) -> Result<Value, Refused> {
        let k = pkey(p);
        if let Some(found) = self.marks.get(&k) {
            return Ok(found.clone());
        }
        let v = self.ask("mark", &json!({"p": pkey_json(p)}))?;
        self.marks.insert(k, v.clone());
        Ok(v)
    }

    fn geometry(&mut self, p: &Para, width: Option<i64>) -> Result<V, Refused> {
        let k = format!("{}|{:?}", pkey(p), width);
        if let Some(found) = self.geoms.get(&k) {
            return Ok(found.clone());
        }
        let v = self.ask("geom", &json!({"p": pkey_json(p), "width": width}))?;
        let wd = |x: &Value| -> Result<V, Refused> { Ok(V::Len(Len::Wd(q_of(x)?))) };
        let mut tabs = Vec::new();
        for t in v["tabs"].as_array().unwrap_or(&vec![]) {
            tabs.push(V::List(vec![wd(&t[0])?, V::Str(t[1].as_str().unwrap_or("").into())]));
        }
        let g = V::Obj(vec![
            ("first_start", wd(&v["first_start"])?),
            ("start", wd(&v["start"])?),
            ("right", wd(&v["right"])?),
            ("tabs", V::List(tabs)),
            ("default_tab", wd(&v["default_tab"])?),
            ("hanging_stop", if v["hanging_stop"].is_null() { V::Null } else { wd(&v["hanging_stop"])? }),
            ("squeeze", V::Bool(v["squeeze"].as_bool().unwrap_or(false))),
            ("tab_origin", wd(&v["tab_origin"])?),
        ]);
        self.geoms.insert(k, g.clone());
        Ok(g)
    }

    fn borders(&mut self, prev: Option<&Para>, p: &Para, next: Option<&Para>) -> Result<(Q, Q), Refused> {
        let k = format!("{}#{}#{}", prev.map(pkey).unwrap_or_default(), pkey(p), next.map(pkey).unwrap_or_default());
        if let Some(found) = self.borders.get(&k) {
            return Ok(*found);
        }
        let v = self.ask(
            "borders",
            &json!({"prev": prev.map(pkey_json), "p": pkey_json(p), "next": next.map(pkey_json)}),
        )?;
        let out = (q_of(&v[0])?, q_of(&v[1])?);
        self.borders.insert(k, out);
        Ok(out)
    }

    fn levels(&mut self, num_id: i64) -> Result<Option<Levels>, Refused> {
        if let Some(found) = self.levels.get(&num_id) {
            return Ok(found.clone());
        }
        let v = self.ask("levels", &json!(num_id))?;
        let found = if v.is_null() {
            None
        } else {
            let mut levels = HashMap::new();
            for l in v["levels"].as_array().unwrap_or(&vec![]) {
                levels.insert(
                    l["ilvl"].as_i64().unwrap_or(0),
                    Level {
                        text: l["text"].as_str().map(str::to_string),
                        start: l["start"].as_i64(),
                        format: l["format"].as_str().map(str::to_string),
                        legal: l["legal"].as_bool().unwrap_or(false),
                        restart: l["restart"].as_i64(),
                    },
                );
            }
            Some(Levels {
                key: v["key"].as_str().unwrap_or("").to_string(),
                restarts: v["restarts"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default(),
                levels,
            })
        };
        self.levels.insert(num_id, found.clone());
        Ok(found)
    }

    /// `Exporter.measure_input` (with the mirror's additions), `geometry` given.
    fn measure(&mut self, p: &Para, label: Option<&Option<String>>, info: &ParaInfo, geometry: V) -> Result<V, Refused> {
        let mut runs = Vec::new();
        for r in &p.runs {
            let ri = self.run(p, r)?;
            let mut entry: Vec<(&'static str, V)> = vec![("text", V::Str(r.text.clone())), ("vanish", V::Bool(ri.vanish))];
            if !ri.vanish {
                let mut fmts: Vec<usize> = Vec::new();
                let mut seen: HashMap<char, usize> = HashMap::new();
                for c in r.text.chars() {
                    let index = match seen.get(&c) {
                        Some(k) => *k,
                        None => {
                            let slot = slot_index(c, ri.complex, ri.hint.as_deref(), ri.ea_lang.as_deref());
                            let f = ri.slots[slot].0.clone();
                            let k = self.ex.fmt(&f, true);
                            let drawn = if ri.caps { upper(c)? } else { c };
                            self.ex.chars.insert(drawn);
                            self.ex.chars.insert(c);
                            self.ex.drawn.insert((f.face.clone(), drawn));
                            seen.insert(c, k);
                            k
                        }
                    };
                    fmts.push(index);
                }
                let distinct: HashSet<usize> = fmts.iter().copied().collect();
                let fmts = if distinct.len() > 1 {
                    fmts
                } else if !fmts.is_empty() {
                    vec![fmts[0]]
                } else {
                    let slot = slot_index(' ', ri.complex, ri.hint.as_deref(), ri.ea_lang.as_deref());
                    let f = ri.slots[slot].1.clone();
                    vec![self.ex.fmt(&f, false)]
                };
                entry.push(("fmts", V::List(fmts.iter().map(|k| V::Int(*k as i64)).collect())));
                entry.push(("tracking", V::Int(ri.tracking)));
                entry.push(("scale", V::Int(ri.scale)));
                entry.push(("kern", V::Int(ri.kern)));
                entry.push(("caps", V::Bool(ri.caps)));
                let slot = slot_index('-', ri.complex, ri.hint.as_deref(), ri.ea_lang.as_deref());
                let hy = ri.slots[slot].0.clone();
                let h = self.ex.fmt(&hy, true);
                entry.push(("hyphen_fmt", V::Int(h as i64)));
                self.ex.chars.insert('-');
                if ri.kern != 0 {
                    for k in &fmts {
                        let font = self.ex.format_list[*k].0;
                        self.ex.kern_fonts.insert(font);
                    }
                }
            }
            for (_, kind) in &r.marks {
                if kind == "noBreakHyphen" {
                    self.ex.chars.insert('\u{2011}');
                }
                if !matches!(kind.as_str(), "tab" | "br" | "textWrapping" | "page" | "column" | "cr" | "noBreakHyphen" | "softHyphen") {
                    return refuse(format!("mark {kind:?}: not exported"));
                }
            }
            entry.push((
                "marks",
                V::List(r.marks.iter().map(|(pos, kind)| V::List(vec![V::Int(*pos as i64), V::Str(kind.clone())])).collect()),
            ));
            if let Some(f) = &r.field {
                if matches!(f.keyword.as_str(), "SEQ" | "SECTIONPAGES" | "STYLEREF") {
                    return refuse(format!("a {} field", f.keyword));
                }
                if matches!(f.keyword.as_str(), "PAGE" | "NUMPAGES" | "PAGEREF") {
                    let arg = page_field_switches(&f.instruction)?;
                    entry.push((
                        "field",
                        V::List(vec![
                            V::Str(f.keyword.clone()),
                            if f.keyword == "PAGEREF" { arg.map(V::Str).unwrap_or(V::Null) } else { V::Null },
                        ]),
                    ));
                }
            }
            if !ri.vanish {
                if ri.va {
                    entry.push(("va", V::Bool(true)));
                }
                entry.push(("draw", V::Str(ri.draw.clone())));
            }
            runs.push(V::Obj(entry));
        }
        let mut out: Vec<(&'static str, V)> = vec![("runs", V::List(runs))];
        let mark = self.mark(p)?;
        let mark_fmt = fmt_key(&mark["space"])?;
        out.push(("mark_fmt", V::Int(self.ex.fmt(&mark_fmt, false) as i64)));
        if let Some(label) = label {
            out.push(("label", self.label(label, &mark)?));
        }
        let mut lh = Vec::new();
        for f in mark["label_height"].as_array().unwrap_or(&vec![]) {
            let f = fmt_key(f)?;
            lh.push(V::Int(self.ex.fmt(&f, false) as i64));
        }
        out.push(("label_height_fmts", V::List(lh)));
        out.push(("geometry", geometry));
        out.push(("rule", V::Int(info.i("rule"))));
        out.push(("line", V::Int(info.i("line"))));
        out.push(("columns", V::Bool(false)));
        out.push(("draw", V::Str(info.v["draw"].as_str().unwrap_or("").into())));
        Ok(V::Obj(out))
    }

    /// `Exporter.label_input` (`label` the counted text, `None`: no level).
    fn label(&mut self, label: &Option<String>, mark: &Value) -> Result<V, Refused> {
        let text = match label {
            None => return Ok(V::Null),
            Some(t) => t,
        };
        let l = &mark["label"];
        if l.is_null() {
            return Ok(V::Null);
        }
        let caps = l["caps"].as_bool().unwrap_or(false);
        let complex = l["cs"].as_bool().unwrap_or(false);
        let hint = l["hint"].as_str().map(str::to_string);
        let ea = l["ea_lang"].as_str().map(str::to_string);
        let mut slots = Vec::new();
        for s in l["slots"].as_array().ok_or_else(|| Refused("label slots".into()))? {
            slots.push(fmt_key(s)?);
        }
        let mut chars = Vec::new();
        for c in text.chars() {
            let f = slots[slot_index(c, complex, hint.as_deref(), ea.as_deref())].clone();
            let drawn = if caps { upper(c)? } else { c };
            self.ex.chars.insert(drawn);
            self.ex.drawn.insert((f.face.clone(), drawn));
            let k = self.ex.fmt(&f, false);
            chars.push(V::List(vec![V::Str(drawn.to_string()), V::Int(k as i64)]));
        }
        let sp = slots[slot_index(' ', complex, hint.as_deref(), ea.as_deref())].clone();
        self.ex.chars.insert(' ');
        let space = self.ex.fmt(&sp, false);
        Ok(V::Obj(vec![
            ("chars", V::List(chars)),
            ("align", V::Str(l["align"].as_str().unwrap_or("left").into())),
            ("suffix", V::Str(l["suffix"].as_str().unwrap_or("tab").into())),
            ("space_fmt", V::Int(space as i64)),
        ]))
    }
}

/// A page field's switches: only `\* MERGEFORMAT` / `\* CHARFORMAT` (and `\h` on PAGEREF,
/// docx2svg's `switches` reads flags apart); its argument for PAGEREF.
fn page_field_switches(instruction: &str) -> Result<Option<String>, Refused> {
    // docx2svg.fields.switches / _argument for the plain forms these fields take
    if instruction.contains('"') {
        return refuse("a quoted page field argument");
    }
    let words: Vec<&str> = instruction.split_whitespace().collect();
    let mut k = 1;
    let mut arg = None;
    while k < words.len() {
        let w = words[k];
        if let Some(rest) = w.strip_prefix('\\') {
            if rest == "*" {
                let a = words.get(k + 1).map(|s| s.to_lowercase()).unwrap_or_default();
                if a != "mergeformat" && a != "charformat" {
                    return refuse(format!("field switch \\* {a}"));
                }
                k += 2;
                continue;
            }
            if rest.eq_ignore_ascii_case("h") || rest.eq_ignore_ascii_case("p") {
                if rest.eq_ignore_ascii_case("p") {
                    return refuse("field switch \\p");
                }
                k += 1;
                continue;
            }
            return refuse(format!("field switch {w}"));
        } else if arg.is_none() && k == 1 {
            arg = Some(w.to_string());
        } else {
            return refuse(format!("a page field's word {w}"));
        }
        k += 1;
    }
    Ok(arg)
}

/// The export's statistics (calls asked, items made).
#[derive(Default, Debug)]
pub struct Stats {
    pub items: usize,
    pub calls: HashMap<&'static str, usize>,
    pub parse_ms: f64,
    pub export_ms: f64,
}

fn para_neighbour<'b>(units: &'b [Unit], k: usize, side: i64) -> Option<&'b Para> {
    let j = k as i64 + side;
    if j < 0 || j as usize >= units.len() {
        return None;
    }
    let ps = units[j as usize].block.paragraphs();
    if ps.is_empty() {
        return para_neighbour(units, j as usize, side);
    }
    Some(if side > 0 { ps[0] } else { ps[ps.len() - 1] })
}

/// The owner's first input for `document.xml`'s body, made as docx-agent's mirror exports it:
/// JSON text, and what it asked.  `fields_stack_end`: whether a field stays open after the
/// body (refused).
pub fn export(data: &[u8], r: &mut dyn Resolver) -> Result<(String, Stats, Body), Refused> {
    let t0 = std::time::Instant::now();
    let body = read_body(data)?;
    let t1 = std::time::Instant::now();
    let mut ex = Exporter::new(r);
    let units = &body.units;
    // keys: unique, every block one
    let mut seen = HashSet::new();
    for u in units {
        match &u.key {
            Some(k) if seen.insert(k.clone()) => {}
            _ => return refuse("a block without a unique paraId"),
        }
    }
    // list labels (`Mirror._label_pass`)
    let mut counters = Counters::default();
    let mut labels: HashMap<*const Para, Option<String>> = HashMap::new();
    for u in units {
        for p in u.block.paragraphs() {
            let info = ex.para(p)?;
            let num = &info.v["num"];
            let num_id = num[0].as_i64().unwrap_or(0);
            if num_id != 0 {
                let ilvl = num[1].as_i64().unwrap_or(0);
                let label = match ex.levels(num_id)? {
                    Some(lv) => counters.count(&lv, num_id, ilvl)?,
                    None => None,
                };
                labels.insert(p as *const Para, label);
            }
        }
    }
    let mut items: Vec<V> = Vec::new();
    let mut faces: Vec<Vec<String>> = Vec::new();
    for (k, u) in units.iter().enumerate() {
        ex.ex.face_log.clear();
        let skeleton = match &u.block {
            Block::T(t) => Some(t.skeleton.clone()),
            Block::P(_) => None,
        };
        let rec = ex.ask_raw("record", &json!({"k": k, "xml": skeleton}))?;
        let rec = with_fields(rec, u, matches!(u.block, Block::T(_)));
        let item = match &u.block {
            Block::P(p) => {
                let info = ex.para(p)?;
                if info.b("refused") {
                    return refuse(info.v["why"].as_str().unwrap_or("refused").to_string());
                }
                let label = labels.get(&(p as *const Para)).cloned();
                let numbered = label.is_some();
                let mut before = info.i("before");
                let mut after = info.i("after");
                let prev = para_neighbour(units, k, -1);
                let next = para_neighbour(units, k, 1);
                let kept = |n: Option<&Para>, end: bool| -> Option<bool> {
                    match n {
                        None => {
                            if end {
                                Some(false)
                            } else {
                                None
                            }
                        }
                        Some(q) => Some(labels.contains_key(&(q as *const Para))),
                    }
                };
                if info.b("auto_before") {
                    let nn = kept(prev, false);
                    let keep = match nn {
                        None => false,
                        Some(x) => !(numbered && x),
                    };
                    if !keep {
                        before = 0;
                    }
                }
                if info.b("auto_after") {
                    let nn = kept(next, true);
                    let keep = match nn {
                        None => false,
                        Some(x) => !(numbered && x),
                    };
                    if !keep {
                        after = 0;
                    }
                }
                let side = |j: i64| -> Option<&Para> {
                    let j = k as i64 + j;
                    if j < 0 || j as usize >= units.len() {
                        return None;
                    }
                    match &units[j as usize].block {
                        Block::P(q) => Some(q),
                        Block::T(_) => None,
                    }
                };
                let (top_border, border) = ex.borders(side(-1), p, side(1))?;
                let geometry = ex.geometry(p, None)?;
                let measure = ex.measure(p, label.as_ref(), &info, geometry)?;
                let style = ex.ex.style(p.style_id.as_deref());
                let widow = &info.v["widow"];
                V::Obj(vec![
                    ("rec", V::Raw(rec)),
                    ("t", V::Str("p".into())),
                    ("block", V::Int(k as i64)),
                    ("key", opt_str(&json!(u.key))),
                    ("bookmarks", bookmarks_of(p)),
                    ("section", V::Int(0)),
                    ("before", V::Int(before)),
                    ("after", V::Int(after)),
                    ("contextual", V::Bool(info.b("contextual"))),
                    ("style", V::Int(style)),
                    ("border", V::Len(Len::Px(border))),
                    ("top_border", V::Len(Len::Px(top_border))),
                    ("keep_next", V::Bool(info.b("keep_next"))),
                    ("keep_lines", V::Bool(info.b("keep_lines"))),
                    ("pbb", V::Bool(info.b("pbb"))),
                    ("widow", if widow.is_null() { V::Null } else { V::Bool(widow.as_bool().unwrap_or(false)) }),
                    ("section_start", V::Bool(k == 0)),
                    ("section_mark", V::Bool(false)),
                    ("measure", measure),
                ])
            }
            Block::T(t) => table_item(&mut ex, units, k, u, t, &labels, rec)?,
        };
        faces.push(ex.ex.face_log.clone());
        items.push(item);
    }
    let t2 = std::time::Instant::now();
    // the document's own: the fonts measured, the sections, the constants
    let font_list: Vec<Value> = ex.ex.font_list.iter().map(|(f, b, i)| json!([f, b, i])).collect();
    let mut chars: Vec<char> = ex.ex.chars.iter().copied().collect();
    chars.sort();
    let mut kern: Vec<i64> = ex.ex.kern_fonts.iter().copied().collect();
    kern.sort();
    let mut drawn: Vec<Value> = ex.ex.drawn.iter().map(|(f, c)| json!([f, c.to_string()])).collect();
    drawn.sort_by_key(|v| v.to_string());
    let doc = ex.ask(
        "document",
        &json!({"fonts": font_list, "chars": chars.iter().collect::<String>(), "kern": kern, "drawn": drawn,
                "faces": faces, "styles": ex.ex.style_count}),
    )?;
    let mut sections = Vec::new();
    for s in doc["sections"].as_array().ok_or_else(|| Refused("sections".into()))? {
        let mut o: Vec<(&'static str, V)> = Vec::new();
        for (name, kind) in [
            ("top", 'p'),
            ("top_neg", 'b'),
            ("header", 'p'),
            ("bottom", 'p'),
            ("bottom_neg", 'b'),
            ("footer", 'p'),
            ("title_page", 'b'),
            ("start_type", 'r'),
            ("new_page", 'b'),
            ("page_number_start", 'r'),
            ("stories", 's'),
        ] {
            let v = &s[name];
            o.push((
                name,
                match kind {
                    'p' => V::Len(Len::Px(q_of(v)?)),
                    'b' => V::Bool(v.as_bool().unwrap_or(false)),
                    'r' => V::Raw(v.to_string()),
                    _ => {
                        if v.is_null() {
                            V::Null
                        } else {
                            let mut st = Vec::new();
                            for x in v.as_array().ok_or_else(|| Refused("stories".into()))? {
                                st.push((kind_name(x[0].as_str().unwrap_or("")), V::List(vec![V::Len(Len::Px(q_of(&x[1])?)), V::Len(Len::Px(q_of(&x[2])?))])));
                            }
                            V::Obj(st)
                        }
                    }
                },
            ));
        }
        sections.push(V::Obj(o));
    }
    let sections = V::List(sections);
    let formats = V::List(
        ex.ex
            .format_list
            .iter()
            .map(|(a, b, c, d, e)| {
                V::List(vec![
                    V::Int(*a),
                    V::Int(*b),
                    V::Int(*c),
                    V::Int(*d),
                    match e {
                        Some((x, y)) => V::List(vec![V::Int(*x), V::Int(*y)]),
                        None => V::Null,
                    },
                ])
            })
            .collect(),
    );
    let items = V::List(items);
    let mut d = Dens { px: doc["lu_den"].as_i64().unwrap_or(1) as i128, w: 1 };
    dens(&sections, &mut d);
    dens(&items, &mut d);
    let px_den = d.px.lcm(&(96 * 20));
    let w_den = d.w.lcm(&20);
    let d = Dens { px: px_den, w: w_den };
    let lu = q_of(&doc["lu"])? * Q::from_integer(px_den);
    if !lu.is_integer() {
        return refuse("the layout unit off the denominator");
    }
    let n_items = match &items {
        V::List(l) => l.len(),
        _ => 0,
    };
    let obj = V::Obj(vec![
        ("compat", V::Raw(doc["compat"].to_string())),
        ("even_odd", V::Bool(doc["even_odd"].as_bool().unwrap_or(false))),
        ("dpi", V::Raw(doc["dpi"].to_string())),
        ("styles", V::Int(ex.ex.style_count as i64)),
        ("fonts", V::Raw(doc["fonts"].as_str().unwrap_or("[]").to_string())),
        ("formats", formats),
        ("sections", sections),
        ("items", items),
        ("constants", V::Raw(doc["constants"].as_str().unwrap_or("null").to_string())),
        ("px_den", V::Int(px_den as i64)),
        ("w_den", V::Int(w_den as i64)),
        ("lu", V::Int(lu.to_integer() as i64)),
    ]);
    let mut out = String::with_capacity(1 << 20);
    encode(&obj, &d, &mut out)?;
    let t3 = std::time::Instant::now();
    let stats = Stats {
        items: n_items,
        calls: ex.calls.clone(),
        parse_ms: (t1 - t0).as_secs_f64() * 1e3,
        export_ms: (t3 - t1).as_secs_f64() * 1e3,
    };
    let _ = t2;
    Ok((out, stats, body))
}

fn kind_name(k: &str) -> &'static str {
    match k {
        "default" => "default",
        "first" => "first",
        "even" => "even",
        _ => "other",
    }
}

fn bookmarks_of(p: &Para) -> V {
    let mut offsets = Vec::new();
    let mut total = 0usize;
    for r in &p.runs {
        offsets.push(total);
        total += r.text.chars().count();
    }
    offsets.push(total);
    let n = p.runs.len();
    V::List(
        p.bookmarks
            .iter()
            .filter(|(start, _, name, _)| *start && name.as_deref().is_some_and(|s| !s.is_empty()))
            .map(|(_, _, name, run)| V::List(vec![V::Str(name.clone().unwrap()), V::Int(offsets[(*run).min(n)] as i64)]))
            .collect(),
    )
}

/// A unit's record with its fields and bookmarks (`Mirror.record`'s end).
fn with_fields(rec: String, u: &Unit, table: bool) -> String {
    let mut extra = String::new();
    if u.toc {
        if table {
            extra.push_str(",\"toc_table\":true");
        }
        extra.push_str(",\"toc\":true");
    }
    if !u.fld.is_empty() {
        extra.push_str(",\"fld\":[");
        for (k, (kw, t, c)) in u.fld.iter().enumerate() {
            if k > 0 {
                extra.push(',');
            }
            extra.push('[');
            py_json_str(kw, &mut extra);
            extra.push(',');
            match t {
                Some(t) => py_json_str(t, &mut extra),
                None => extra.push_str("null"),
            }
            extra.push(',');
            match c {
                Some(c) => py_json_str(c, &mut extra),
                None => extra.push_str("null"),
            }
            extra.push(']');
        }
        extra.push(']');
    }
    if !u.bm.is_empty() {
        extra.push_str(",\"bm\":[");
        for (k, n) in u.bm.iter().enumerate() {
            if k > 0 {
                extra.push(',');
            }
            py_json_str(n, &mut extra);
        }
        extra.push(']');
    }
    if extra.is_empty() {
        return rec;
    }
    let mut rec = rec;
    let end = rec.rfind('}').unwrap_or(rec.len());
    rec.truncate(end);
    rec.push_str(&extra);
    rec.push('}');
    rec
}

/// `Mirror.table_item`: the table resolved and sized by the resolver from its cells'
/// content (measured here: [`crate::linebreak::pieces`]).
fn table_item(
    ex: &mut Exporter,
    units: &[Unit],
    k: usize,
    u: &Unit,
    t: &Table,
    labels: &HashMap<*const Para, Option<String>>,
    rec: String,
) -> Result<V, Refused> {
    // every cell paragraph's measuring input but its geometry (the cell's width is not known
    // yet), in the export's order of asking
    let mut cells_in: Vec<Vec<Vec<(V, ParaInfo)>>> = Vec::new();
    let mut extents_in: Vec<Value> = Vec::new();
    let mut previous = None;
    if k > 0 {
        if let Block::P(p) = &units[k - 1].block {
            previous = Some(p);
        }
    }
    let prev_info = match previous {
        Some(p) => Some(ex.para(p)?),
        None => None,
    };
    let _ = prev_info;
    // the measuring of each cell paragraph's words, for autofit: asked of the resolver per
    // paragraph text (it measures with docx2svg's pieces)
    for (r, row) in t.rows.iter().enumerate() {
        let mut cells = Vec::new();
        for (c, cell) in row.iter().enumerate() {
            let mut paras = Vec::new();
            let mut ext = Vec::new();
            for p in cell {
                let info = ex.para(p)?;
                if info.b("refused") {
                    return refuse(info.v["why"].as_str().unwrap_or("refused").to_string());
                }
                let label = labels.get(&(p as *const Para)).cloned();
                let m = ex.measure(p, label.as_ref(), &info, V::Null)?;
                ext.push(p);
                paras.push((m, info));
            }
            let _ = (r, c);
            extents_in.push(json!(ext.iter().map(|p| para_words(p)).collect::<Vec<_>>()));
            cells.push(paras);
        }
        cells_in.push(cells);
    }
    let previous_key = previous.map(pkey_json);
    let v = ex.ask(
        "table",
        &json!({"xml": t.skeleton, "previous": previous_key, "cells": extents_in, "fonts": null}),
    )?;
    if let Some(why) = v["refused"].as_str() {
        return refuse(why.to_string());
    }
    let mut rows = Vec::new();
    let mut first_style: Option<Option<String>> = None;
    for (r, row) in t.rows.iter().enumerate() {
        let rv = &v["rows"][r];
        let mut cells = Vec::new();
        for (c, cell) in row.iter().enumerate() {
            let cv = &rv["cells"][c];
            let budget = cv["budget"].as_i64().unwrap_or(1).max(1);
            let mut paras = Vec::new();
            for (j, p) in cell.iter().enumerate() {
                let (m, info) = cells_in[r][c][j].clone();
                let geometry = ex.geometry(p, Some(budget))?;
                let m = match m {
                    V::Obj(mut o) => {
                        for e in o.iter_mut() {
                            if e.0 == "geometry" {
                                e.1 = geometry.clone();
                            }
                        }
                        V::Obj(o)
                    }
                    other => other,
                };
                let prev = if j > 0 { Some(&cell[j - 1]) } else { None };
                let next = cell.get(j + 1);
                let (top_border, border) = ex.borders(prev, p, next)?;
                if first_style.is_none() {
                    first_style = Some(p.style_id.clone());
                }
                let style = ex.ex.style(p.style_id.as_deref());
                let eff = info.v["eff_style"].as_str().map(str::to_string);
                let effective = ex.ex.style(eff.as_deref());
                let widow = &info.v["widow"];
                paras.push(V::Obj(vec![
                    ("block", V::Int(0)),
                    ("key", match &p.para_id {
                        Some(raw) => V::Str(format!("p:{}", raw.to_uppercase())),
                        None => V::Null,
                    }),
                    ("bookmarks", bookmarks_of(p)),
                    ("before", V::Int(info.i("before"))),
                    ("after", V::Int(info.i("after"))),
                    ("contextual", V::Bool(info.b("contextual"))),
                    ("style", V::Int(style)),
                    ("effective_style", V::Int(effective)),
                    ("top_border", V::Len(Len::Px(top_border))),
                    ("border", V::Len(Len::Px(border))),
                    ("hidden", V::Bool(false)),
                    ("widow", if widow.is_null() { V::Null } else { V::Bool(widow.as_bool().unwrap_or(false)) }),
                    ("keep_next", V::Bool(info.b("keep_next"))),
                    ("measure", m),
                ]));
            }
            cells.push(V::Obj(vec![
                ("mt", V::Len(Len::Px(q_of(&cv["mt"])?))),
                ("mb", V::Len(Len::Px(q_of(&cv["mb"])?))),
                ("merge", V::Null),
                ("paragraphs", V::List(paras)),
            ]));
        }
        let keeps = row
            .first()
            .and_then(|c| c.first())
            .map(|p| ex.para(p).map(|i| i.b("keep_next")))
            .transpose()?
            .unwrap_or(false);
        let tr = &rv["tr_height"];
        rows.push(V::Obj(vec![
            ("top_border", V::Len(Len::Px(q_of(&rv["top_border"])?))),
            ("header", V::Bool(rv["header"].as_bool().unwrap_or(false))),
            ("cant_split", V::Bool(rv["cant_split"].as_bool().unwrap_or(false))),
            ("keeps_next", V::Bool(keeps)),
            (
                "tr_height",
                if tr.is_null() { V::Null } else { V::List(vec![V::Len(Len::Px(q_of(&tr[0])?)), V::Raw(tr[1].to_string())]) },
            ),
            ("cells", V::List(cells)),
        ]));
    }
    let previous_style = previous.map(|p| ex.para(p)).transpose()?.and_then(|i| i.v["eff_style"].as_str().map(str::to_string));
    let ps = ex.ex.style(previous_style.as_deref());
    let ds = ex.ex.style(v["default_style"].as_str());
    let fs = match &first_style {
        Some(s) => ex.ex.style(s.as_deref()),
        None => -2,
    };
    Ok(V::Obj(vec![
        ("rec", V::Raw(rec)),
        ("draw", V::Str(v["draw"].as_str().unwrap_or("").into())),
        ("t", V::Str("t".into())),
        ("block", V::Int(k as i64)),
        ("key", opt_str(&json!(u.key))),
        ("section", V::Int(0)),
        ("section_start", V::Bool(k == 0)),
        ("bottom_border", V::Len(Len::Px(q_of(&v["bottom_border"])?))),
        ("previous_style", V::Int(ps)),
        ("default_style", V::Int(ds)),
        ("first_style", V::Int(fs)),
        ("rows", V::List(rows)),
    ]))
}

/// What the resolver measures of a cell paragraph for its table's autofit: its runs as
/// keys and texts.
fn para_words(p: &Para) -> Value {
    json!({
        "p": pkey_json(p),
        "runs": p.runs.iter().map(|r| json!({"rpr": r.rpr, "toc": r.toc_link, "text": r.text,
            "marks": r.marks.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            "field": r.field.as_ref().map(|f| json!([f.keyword, f.instruction]))})).collect::<Vec<_>>()
    })
}

#[allow(dead_code)]
fn _unused(_: &Map<String, Value>) {}
