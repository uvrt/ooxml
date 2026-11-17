//! Relayout equals full layout (docs/architecture/README.md, section 8): made-up documents
//! of paragraphs of every length -- keeps, page breaks, widow control off, spacing, exact
//! lines, section breaks of every kind and of several columns, `PAGE`, `NUMPAGES`,
//! `PAGEREF`, `SEQ` and `REF` fields with their bookmarks -- and tables (rows split across
//! pages, header rows, rows that may not split or keep with the next, cells merged across
//! and down, fixed and autofit widths, a field in a cell) edited one block at a time
//! (text, a keep, a cell's text, a header row, a paragraph or table inserted or deleted),
//! each version laid out again from the one before and
//! laid out whole in a fresh engine and model: the pages, their numbers and the fields'
//! values are equal. Fixed seeds here; `many_seeds` (ignored) runs many.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod support;

use support::{engine, field, model, open, para, run, sect};

/// xorshift64*: the tests' own generator, seeded.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// A made-up table: its rows of cells' word counts, and what its rows and cells state.
#[derive(Clone, Debug)]
struct Tbl {
    columns: usize,
    rows: Vec<Vec<u64>>,
    headers: usize,
    cant_split: Vec<bool>,
    keep: Vec<bool>,
    /// A cell of the first column merged down over this row and the next.
    merge_down: Option<usize>,
    /// A row whose first two cells are one.
    span: Option<usize>,
    autofit: bool,
    exact: Option<usize>,
}

fn tbl(rng: &mut Rng) -> Tbl {
    let columns = 1 + usize::try_from(rng.below(3)).unwrap();
    let count = 1 + usize::try_from(rng.below(9)).unwrap();
    let rows: Vec<Vec<u64>> = (0..count)
        .map(|_| {
            (0..columns)
                .map(|_| if rng.chance(10) { 0 } else { rng.below(40) })
                .collect()
        })
        .collect();
    let pick = |rng: &mut Rng| usize::try_from(rng.below(u64::try_from(count).unwrap())).unwrap();
    Tbl {
        columns,
        headers: if rng.chance(30) { 1 + pick(rng).min(1) } else { 0 },
        cant_split: (0..count).map(|_| rng.chance(15)).collect(),
        keep: (0..count).map(|_| rng.chance(10)).collect(),
        merge_down: (count > 1 && rng.chance(30)).then(|| pick(rng).min(count - 2)),
        span: (columns > 1 && rng.chance(30)).then(|| pick(rng)),
        autofit: rng.chance(40),
        exact: rng.chance(10).then(|| pick(rng)),
        rows,
    }
}

fn tbl_xml(t: &Tbl, k: usize, field: Option<&str>) -> String {
    let width = 2160 / t.columns;
    let layout = if t.autofit {
        ""
    } else {
        r#"<w:tblLayout w:type="fixed"/>"#
    };
    let mut out = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders><w:top w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="8"/><w:bottom w:val="single" w:sz="4"/></w:tblBorders>{layout}<w:tblCellMar><w:top w:w="20" w:type="dxa"/><w:left w:w="60" w:type="dxa"/><w:bottom w:w="20" w:type="dxa"/><w:right w:w="60" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>{}</w:tblGrid>"#,
        format!(r#"<w:gridCol w:w="{width}"/>"#).repeat(t.columns)
    );
    for (r, row) in t.rows.iter().enumerate() {
        let mut trpr = String::new();
        if r < t.headers {
            trpr.push_str("<w:tblHeader/>");
        }
        if t.cant_split[r] {
            trpr.push_str("<w:cantSplit/>");
        }
        if t.exact == Some(r) {
            trpr.push_str(r#"<w:trHeight w:val="400" w:hRule="exact"/>"#);
        }
        out.push_str(&format!("<w:tr><w:trPr>{trpr}</w:trPr>"));
        let spanned = t.span == Some(r);
        let mut c = 0;
        while c < t.columns {
            let span = if spanned && c == 0 { 2 } else { 1 };
            let mut tcpr = format!(r#"<w:tcW w:w="{}" w:type="dxa"/>"#, width * span);
            if span > 1 {
                tcpr.push_str(r#"<w:gridSpan w:val="2"/>"#);
            }
            let merge = match t.merge_down {
                Some(m) if c == 0 && !spanned && m == r => Some("restart"),
                Some(m) if c == 0 && !spanned && m + 1 == r && t.span != Some(m) => Some("continue"),
                _ => None,
            };
            if let Some(kind) = merge {
                tcpr.push_str(&format!(r#"<w:vMerge w:val="{kind}"/>"#));
            }
            let words: Vec<String> = (0..row[c]).map(|w| format!("c{}x{}", k, w % 5)).collect();
            let ppr = if t.keep[r] && c == 0 { "<w:keepNext/>" } else { "" };
            let mut content = String::new();
            if r == 0 && c == 0 {
                content.push_str(&format!(r#"<w:bookmarkStart w:id="{k}" w:name="b{k}"/>"#));
            }
            content.push_str(&run(&words.join(" ")));
            if r == 0 && c == 0 {
                content.push_str(&format!(r#"<w:bookmarkEnd w:id="{k}"/>"#));
                if let Some(instruction) = field {
                    content.push_str(&field_xml(instruction));
                }
            }
            out.push_str(&format!("<w:tc><w:tcPr>{tcpr}</w:tcPr>{}</w:tc>", para(ppr, &content)));
            c += span;
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    out
}

fn field_xml(instruction: &str) -> String {
    format!("{}{}", run(" p"), field(instruction, "1"))
}

#[derive(Clone, Debug)]
struct Spec {
    table: Option<Tbl>,
    words: u64,
    keep_next: bool,
    keep_lines: bool,
    page_break_before: bool,
    widow_off: bool,
    before: u64,
    after: u64,
    exact: bool,
    page_break: bool,
    section: Option<(&'static str, u32)>,
    field: Option<String>,
    bookmark: Option<String>,
}

fn spec(rng: &mut Rng, k: usize) -> Spec {
    let kinds = ["nextPage", "continuous", "oddPage", "evenPage"];
    let section = rng.chance(6).then(|| {
        let kind = kinds[usize::try_from(rng.below(4)).unwrap()];
        (kind, u32::try_from(1 + rng.below(3)).unwrap())
    });
    let field = match rng.below(14) {
        0 => Some("PAGE".to_owned()),
        1 => Some("NUMPAGES".to_owned()),
        2 => Some(format!("PAGEREF b{} \\h", rng.below(40))),
        3 => Some("SEQ Figure".to_owned()),
        4 => Some(format!("REF b{} \\h", rng.below(40))),
        _ => None,
    };
    Spec {
        table: rng.chance(12).then(|| tbl(rng)),
        words: if rng.chance(15) { 0 } else { 1 + rng.below(60) },
        keep_next: rng.chance(10),
        keep_lines: rng.chance(5),
        page_break_before: rng.chance(3),
        widow_off: rng.chance(10),
        before: [0, 0, 60, 120, 240][usize::try_from(rng.below(5)).unwrap()],
        after: [0, 0, 60, 120, 200][usize::try_from(rng.below(5)).unwrap()],
        exact: rng.chance(8),
        page_break: rng.chance(3),
        section,
        field,
        bookmark: Some(format!("b{k}")),
    }
}

fn xml(specs: &[Spec]) -> String {
    let mut body = String::new();
    for (k, s) in specs.iter().enumerate() {
        if let Some(t) = &s.table {
            body.push_str(&tbl_xml(t, k, s.field.as_deref()));
            continue;
        }
        let mut ppr = String::new();
        for (on, tag) in [
            (s.keep_next, "<w:keepNext/>"),
            (s.keep_lines, "<w:keepLines/>"),
            (s.page_break_before, "<w:pageBreakBefore/>"),
            (s.widow_off, r#"<w:widowControl w:val="0"/>"#),
        ] {
            if on {
                ppr.push_str(tag);
            }
        }
        let rule = if s.exact {
            r#" w:line="300" w:lineRule="exact""#
        } else {
            ""
        };
        ppr.push_str(&format!(
            r#"<w:spacing w:before="{}" w:after="{}"{rule}/>"#,
            s.before, s.after
        ));
        if let Some((kind, columns)) = s.section {
            ppr.push_str(&sect(kind, columns));
        }
        let words: Vec<String> = (0..s.words).map(|w| format!("w{}x{}", k, w % 7)).collect();
        let mut content = String::new();
        if let Some(name) = &s.bookmark {
            content.push_str(&format!(r#"<w:bookmarkStart w:id="{k}" w:name="{name}"/>"#));
        }
        content.push_str(&run(&words.join(" ")));
        if s.bookmark.is_some() {
            content.push_str(&format!(r#"<w:bookmarkEnd w:id="{k}"/>"#));
        }
        if s.page_break {
            content.push_str(r#"<w:r><w:br w:type="page"/></w:r>"#);
        }
        if let Some(instruction) = &s.field {
            content.push_str(&field_xml(instruction));
        }
        body.push_str(&para(&ppr, &content));
    }
    body.push_str(&sect("nextPage", 1));
    body
}

fn edit(rng: &mut Rng, specs: &mut Vec<Spec>) {
    let k = usize::try_from(rng.below(u64::try_from(specs.len()).unwrap())).unwrap();
    match rng.below(7) {
        0 => specs[k].words = 1 + rng.below(80),
        5 => match &mut specs[k].table {
            Some(t) => {
                let r = usize::try_from(rng.below(u64::try_from(t.rows.len()).unwrap())).unwrap();
                let c = usize::try_from(rng.below(u64::try_from(t.columns).unwrap())).unwrap();
                t.rows[r][c] = rng.below(60);
            }
            None => specs[k].table = Some(tbl(rng)),
        },
        6 => match &mut specs[k].table {
            Some(t) => t.headers = if t.headers == 0 { 1 } else { 0 },
            None => specs[k].keep_lines = !specs[k].keep_lines,
        },
        1 => specs[k].keep_next = !specs[k].keep_next,
        2 => {
            let fresh = spec(rng, specs.len() + k);
            specs.insert(k, fresh);
        }
        3 if specs.len() > 2 => {
            specs.remove(k);
        }
        _ => specs[k].after = [0, 120, 400][usize::try_from(rng.below(3)).unwrap()],
    }
}

fn check(seed: u64, versions: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut specs: Vec<Spec> = (0..40).map(|k| spec(&mut rng, k)).collect();
    let settings = Some(
        r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>"#,
    );
    let settings = if seed.is_multiple_of(2) { settings } else { None };
    let mut kept_model = model();
    let mut kept = engine();
    let first = open(&mut kept_model, &xml(&specs), settings);
    let mut previous = kept.lay_out(&mut kept_model, &first);
    for version in 0..versions {
        edit(&mut rng, &mut specs);
        let body = xml(&specs);
        let document = open(&mut kept_model, &body, settings);
        let relaid = kept.relayout(&mut kept_model, &document, &previous);
        let mut fresh_model = model();
        let fresh_document = open(&mut fresh_model, &body, settings);
        let whole = engine().lay_out(&mut fresh_model, &fresh_document);
        assert!(
            whole.pagination.stopped.is_none(),
            "seed {seed} version {version}: stopped {:?}",
            whole.pagination.stopped
        );
        if relaid.pagination != whole.pagination {
            let (a, b) = (&relaid.pagination, &whole.pagination);
            let k = a.pages.iter().zip(&b.pages).position(|(x, y)| x != y);
            let brief = |p: Option<&ooxml_word_layout::paginate::LaidPage>| {
                p.map(|p| {
                    (
                        p.page,
                        p.number_before,
                        p.start.clone(),
                        p.end.clone(),
                        p.reach.clone(),
                        p.lines.len(),
                    )
                })
            };
            panic!(
                "seed {seed} version {version}: pages differ at {k:?} of {}/{}: {:?} vs {:?}; info {} vs {}; stats {:?}; stopped {:?} vs {:?}",
                a.pages.len(),
                b.pages.len(),
                brief(k.and_then(|k| a.pages.get(k))),
                brief(k.and_then(|k| b.pages.get(k))),
                a.info.len(),
                b.info.len(),
                relaid.relaid,
                a.stopped,
                b.stopped
            );
        }
        assert_eq!(
            relaid.fields, whole.fields,
            "seed {seed} version {version}: fields differ"
        );
        assert_eq!(
            (relaid.passes, relaid.settled),
            (whole.passes, whole.settled),
            "seed {seed} version {version}"
        );
        previous = relaid;
    }
}

#[test]
fn relayout_equals_full_layout() {
    for seed in 1..=6 {
        check(seed, 8);
    }
}

#[test]
#[ignore = "many seeds: run at the end of a lane and nightly"]
fn many_seeds() {
    for seed in 1..=300 {
        check(seed, 12);
    }
}

#[test]
fn an_edit_late_in_the_body_lays_out_few_pages() {
    let mut rng = Rng(7);
    let mut specs: Vec<Spec> = (0..300)
        .map(|k| {
            let mut s = spec(&mut rng, k);
            s.section = None;
            s.field = None;
            s.table = None;
            s
        })
        .collect();
    let mut m = model();
    let mut e = engine();
    let d = open(&mut m, &xml(&specs), None);
    let before = e.lay_out(&mut m, &d);
    specs[250].words = 3;
    let d = open(&mut m, &xml(&specs), None);
    let after = e.relayout(&mut m, &d, &before);
    let stats = after.relaid.first().copied().unwrap();
    assert!(stats.restart > 0, "{stats:?}");
    assert!(stats.laid < after.pagination.pages.len() / 2, "{stats:?}");
}
