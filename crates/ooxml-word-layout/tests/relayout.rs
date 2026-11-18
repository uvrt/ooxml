//! Relayout equals full layout (docs/architecture/README.md, section 8): made-up documents
//! of paragraphs of every length -- keeps, page breaks, widow control off, spacing, exact
//! lines, section breaks of every kind and of several columns, `PAGE`, `NUMPAGES`,
//! `PAGEREF`, `SEQ` and `REF` fields with their bookmarks, tables (rows split across
//! pages, header rows, rows that may not split or keep with the next, cells merged across
//! and down, fixed and autofit widths, a field in a cell, a floating drawing in a cell --
//! its text beside or below it, the cell aligned or not), footnote and endnote
//! references, floating drawings text wraps beside or top and bottom of, a header of
//! changing height, drop caps, and sections with a line between their columns, unequal
//! columns, line numbers or footnotes beneath the text -- edited one block at a time (text, a keep, a cell's text, a header
//! row, a note, a drawing, a paragraph or table inserted or deleted, the header), each
//! version laid out again from the one before and
//! laid out whole in a fresh engine and model: the pages, their numbers and the fields'
//! values are equal. Fixed seeds here; `many_seeds` (ignored) runs many.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod support;

use ooxml_word_layout::facts::LayoutFacts;
use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::incremental::same_item;
use ooxml_word_layout::paint::svg::{page_svg, NoDrawings, NoPictures, SvgContext};
use ooxml_word_layout::paint::{paint_page, DocumentNames, GlyphSize, PaintContext};
use ooxml_word_layout::{DocumentLayout, WordLayout};
use ooxml_word_model::Model;
use support::{anchor, engine, field, model, notes_part, open, open_with, para, run, sect, Part};

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
    /// Floating, positioned this far below the paragraph after it (twips).
    floating: Option<u64>,
    /// A floating drawing in a row's first cell: the row, its wrap and its offset across
    /// (twips), and whether the cell is aligned to its middle.
    cell_drawing: Option<(usize, &'static str, i64, bool)>,
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
        floating: rng.chance(20).then(|| rng.below(300)),
        cell_drawing: rng.chance(25).then(|| {
            let wrap =
                ["wrapSquare", "wrapTopAndBottom", "wrapNone", "wrapTight"][usize::try_from(rng.below(4)).unwrap()];
            (pick(rng), wrap, i64::try_from(rng.below(600)).unwrap(), rng.chance(30))
        }),
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
    let width = if t.floating.is_some() { width / 2 } else { width };
    let position = t.floating.map_or(String::new(), |y| {
        format!(r#"<w:tblpPr w:leftFromText="120" w:rightFromText="120" w:vertAnchor="text" w:tblpY="{y}"/>"#)
    });
    let mut out = format!(
        r#"<w:tbl><w:tblPr>{position}<w:tblW w:w="0" w:type="auto"/><w:tblBorders><w:top w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="8"/><w:bottom w:val="single" w:sz="4"/></w:tblBorders>{layout}<w:tblCellMar><w:top w:w="20" w:type="dxa"/><w:left w:w="60" w:type="dxa"/><w:bottom w:w="20" w:type="dxa"/><w:right w:w="60" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>{}</w:tblGrid>"#,
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
            let drawn = t.cell_drawing.filter(|d| d.0 == r && c == 0);
            if drawn.is_some_and(|d| d.3) {
                tcpr.push_str(r#"<w:vAlign w:val="center"/>"#);
            }
            let words: Vec<String> = (0..row[c]).map(|w| format!("c{}x{}", k, w % 5)).collect();
            let ppr = if t.keep[r] && c == 0 { "<w:keepNext/>" } else { "" };
            let mut content = String::new();
            if r == 0 && c == 0 {
                content.push_str(&format!(r#"<w:bookmarkStart w:id="{k}" w:name="b{k}"/>"#));
            }
            if let Some((_, wrap, x, _)) = drawn {
                content.push_str(&anchor(wrap, x, 0, 500, 300));
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
    footnote: Option<u64>,
    endnote: Option<u64>,
    drawing: Option<(&'static str, i64)>,
    /// A list item at this level.
    list: Option<u64>,
    /// In the style the header's STYLEREF shows.
    heading: bool,
    /// A drop cap of its first letter (a frame paragraph before it).
    drop_cap: bool,
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
        footnote: rng.chance(12).then(|| 1 + rng.below(8)),
        endnote: rng.chance(5).then(|| 1 + rng.below(4)),
        drawing: drawing(rng),
        list: rng.chance(20).then(|| rng.below(3)),
        heading: rng.chance(8),
        drop_cap: rng.chance(4),
    }
}

/// What a section break's `w:sectPr` holds beyond its kind and columns: a line between its
/// columns, unequal columns, line numbers, footnotes beneath the text.
fn section_xml(kind: &str, columns: u32, flavour: u64) -> String {
    let base = sect(kind, columns);
    match flavour {
        0 => base.replace(r#"w:space="120"/>"#, r#"w:space="120" w:sep="1"/>"#),
        1 if columns > 1 => base.replace(
            &format!(r#"<w:cols w:num="{columns}" w:space="120"/>"#),
            &format!(
                r#"<w:cols w:num="{columns}" w:equalWidth="0">{}</w:cols>"#,
                (0..columns)
                    .map(|c| format!(r#"<w:col w:w="{}" w:space="120"/>"#, 500 + 300 * c))
                    .collect::<String>()
            ),
        ),
        2 => base.replace(
            "<w:cols ",
            r#"<w:lnNumType w:countBy="2" w:restart="continuous" w:distance="200"/><w:cols "#,
        ),
        3 => base.replace(
            "<w:cols ",
            r#"<w:lnNumType w:countBy="1" w:restart="newSection"/><w:cols "#,
        ),
        4 => base.replace(
            "<w:pgSz ",
            r#"<w:footnotePr><w:pos w:val="beneathText"/></w:footnotePr><w:pgSz "#,
        ),
        _ => base,
    }
}

fn drawing(rng: &mut Rng) -> Option<(&'static str, i64)> {
    let wrap = ["wrapSquare", "wrapTopAndBottom", "wrapNone"][usize::try_from(rng.below(3)).unwrap()];
    rng.chance(8).then(|| (wrap, i64::try_from(rng.below(1200)).unwrap()))
}

/// A document's parts beside its body: its footnotes and endnotes, and a header of
/// `header` words.
struct Extra {
    footnotes: Vec<(u32, String)>,
    endnotes: Vec<(u32, String)>,
    header: u64,
}

impl Extra {
    fn new(rng: &mut Rng) -> Extra {
        let mut text = |k: u64| {
            (0..1 + rng.below(40))
                .map(|w| format!("n{k}x{}", w % 5))
                .collect::<Vec<_>>()
                .join(" ")
        };
        Extra {
            footnotes: (1..=8).map(|k| (u32::try_from(k).unwrap(), text(k))).collect(),
            endnotes: (1..=4).map(|k| (u32::try_from(k).unwrap(), text(k))).collect(),
            header: 3,
        }
    }

    fn parts(&self) -> Vec<Part<'static>> {
        let header = (0..self.header).map(|w| format!("h{w}")).collect::<Vec<_>>().join(" ");
        vec![
            Part {
                id: "rIdF",
                kind: "footnotes",
                name: "footnotes.xml",
                xml: notes_part("footnote", &self.footnotes),
            },
            Part {
                id: "rIdE",
                kind: "endnotes",
                name: "endnotes.xml",
                xml: notes_part("endnote", &self.endnotes),
            },
            Part {
                id: "rIdH",
                kind: "header",
                name: "header1.xml",
                xml: format!(
                    r#"<w:hdr {}>{}</w:hdr>"#,
                    support::W,
                    para("", &format!("{}{}", run(&header), field("STYLEREF \"heading 1\"", "?")))
                ),
            },
            Part {
                id: "rIdN",
                kind: "numbering",
                name: "numbering.xml",
                xml: numbering_part(),
            },
            Part {
                id: "rIdS",
                kind: "styles",
                name: "styles.xml",
                xml: format!(
                    r#"<w:styles {}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/></w:rPr></w:style></w:styles>"#,
                    support::W
                ),
            },
        ]
    }
}

/// A numbering part of one list: three levels, each `%n.` after the levels above.
fn numbering_part() -> String {
    let levels: String = (0..3)
        .map(|l| {
            let text: String = (1..=l + 1).map(|n| format!("%{n}.")).collect();
            format!(
                r#"<w:lvl w:ilvl="{l}"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="{text}"/><w:pPr><w:ind w:left="{}" w:hanging="300"/></w:pPr></w:lvl>"#,
                300 * (l + 1)
            )
        })
        .collect();
    format!(
        r#"<w:numbering {}><w:abstractNum w:abstractNumId="0">{levels}</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#,
        support::W
    )
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
        if s.heading {
            ppr.insert_str(0, r#"<w:pStyle w:val="Heading1"/>"#);
        }
        if let Some(level) = s.list {
            ppr.push_str(&format!(
                r#"<w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="1"/></w:numPr>"#
            ));
        }
        if let Some((kind, columns)) = s.section {
            ppr.push_str(&section_xml(kind, columns, (k as u64) % 8));
        }
        if s.drop_cap && s.words > 0 && s.section.is_none() {
            body.push_str(&para(
                r#"<w:framePr w:dropCap="drop" w:lines="2" w:wrap="around" w:vAnchor="text" w:hAnchor="text"/>"#,
                &run("D"),
            ));
        }
        let words: Vec<String> = (0..s.words).map(|w| format!("w{}x{}", k, w % 7)).collect();
        let mut content = String::new();
        if let Some((wrap, x)) = s.drawing {
            content.push_str(&anchor(wrap, x, 0, 720, 400));
        }
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
        if let Some(id) = s.footnote {
            content.push_str(&format!(r#"<w:r><w:footnoteReference w:id="{id}"/></w:r>"#));
        }
        if let Some(id) = s.endnote {
            content.push_str(&format!(r#"<w:r><w:endnoteReference w:id="{id}"/></w:r>"#));
        }
        if let Some(instruction) = &s.field {
            content.push_str(&field_xml(instruction));
        }
        body.push_str(&para(&ppr, &content));
    }
    body.push_str(&sect("nextPage", 1).replace(
        "<w:sectPr>",
        r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/>"#,
    ));
    body
}

fn edit(rng: &mut Rng, specs: &mut Vec<Spec>, extra: &mut Extra) {
    let k = usize::try_from(rng.below(u64::try_from(specs.len()).unwrap())).unwrap();
    match rng.below(11) {
        7 => specs[k].footnote = (specs[k].footnote.is_none()).then(|| 1 + rng.below(8)),
        8 => specs[k].drawing = drawing(rng),
        9 => specs[k].endnote = (specs[k].endnote.is_none()).then(|| 1 + rng.below(4)),
        10 if rng.chance(30) => extra.header = 1 + rng.below(30),
        10 if rng.chance(30) => specs[k].list = specs[k].list.is_none().then(|| rng.below(3)),
        10 if rng.chance(30) => specs[k].heading = !specs[k].heading,
        10 if rng.chance(30) => specs[k].drop_cap = !specs[k].drop_cap,
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

/// What a run laid out: pages with footnotes, with bands, beside drawings, and endnote
/// paragraphs.
#[derive(Default, Debug)]
struct Seen {
    notes: usize,
    bands: usize,
    wraps: usize,
    endnotes: usize,
}

/// The flow made again is the flow made from scratch: item by item alike, at the same
/// blocks and sections (tables are made by two engines, so only their kind is compared).
fn same_flow(relaid: &[FlowItem], whole: &[FlowItem], at: &str) {
    assert_eq!(relaid.len(), whole.len(), "{at}: flow");
    for (k, (x, y)) in relaid.iter().zip(whole).enumerate() {
        let alike = match (x, y) {
            (FlowItem::Para(_), FlowItem::Para(_)) => same_item(x, y),
            _ => std::mem::discriminant(x) == std::mem::discriminant(y),
        };
        assert!(
            alike && x.block() == y.block() && x.section() == y.section(),
            "{at}: flow item {k} differs: alike {alike}, blocks {} {}, sections {} {}; {:?} vs {:?}",
            x.block(),
            y.block(),
            x.section(),
            y.section(),
            x.para().map(|p| (
                p.body,
                p.drop_cap.is_some(),
                p.measured.heights.len(),
                p.before,
                p.after
            )),
            y.para().map(|p| (
                p.body,
                p.drop_cap.is_some(),
                p.measured.heights.len(),
                p.before,
                p.after
            )),
        );
    }
}

/// Each pass laid out again is that pass laid out from scratch: its flow and its pages.
fn same_passes(relaid: &ooxml_word_layout::DocumentLayout, whole: &ooxml_word_layout::DocumentLayout, at: &str) {
    for k in 0..relaid.passes.min(whole.passes) {
        if let (Some((fa, pa)), Some((fb, pb))) = (relaid.pass(k), whole.pass(k)) {
            same_flow(&fa.items, &fb.items, &format!("{at} pass {k}"));
            if pa != pb {
                for (x, y) in pa.pages.iter().zip(&pb.pages) {
                    if x != y {
                        let fields = [
                            ("start", x.start == y.start),
                            ("end", x.end == y.end),
                            ("lines", x.lines == y.lines),
                            ("rows", x.rows == y.rows),
                            ("notes", x.notes == y.notes),
                            ("carried", x.carried == y.carried),
                            ("body_end", x.body_end == y.body_end),
                            ("columns", x.columns == y.columns),
                            ("note_area", x.note_area == y.note_area),
                            ("broken", x.broken == y.broken),
                            ("lines_before", x.lines_before == y.lines_before),
                            ("reach", x.reach == y.reach),
                            ("wraps", x.wraps == y.wraps),
                            ("page", x.page == y.page),
                            ("number_before", x.number_before == y.number_before),
                            ("forced", x.forced == y.forced),
                            ("continued", x.continued == y.continued),
                            ("opens", x.opens == y.opens),
                            ("bands", x.bands == y.bands),
                            ("carry_out", x.carry_out == y.carry_out),
                        ];
                        panic!(
                            "{at} pass {k}: page {} differs: {:?}; {:?} vs {:?}; stats {:?}",
                            x.page,
                            fields.iter().filter(|f| !f.1).collect::<Vec<_>>(),
                            x.bands,
                            y.bands,
                            relaid.relaid
                        );
                    }
                }
            }
            assert!(pa == pb, "{at} pass {k}: pages differ");
        }
    }
}

/// Page `page` of `laid` as SVG, its header, footer and footnotes laid out by `engine`.
fn page_svg_of(engine: &mut WordLayout, model: &mut Model, laid: &DocumentLayout, page: usize) -> Option<String> {
    let names = DocumentNames;
    let faces = support::Mono;
    let cx = PaintContext {
        faces: &faces,
        names: &names,
        glyph_size: GlyphSize::Device,
        decimal: '.',
    };
    let svg = SvgContext {
        drawings: &NoDrawings,
        names: &names,
        pictures: &NoPictures,
        glyph_size: GlyphSize::Device,
    };
    let parts = engine.page_parts(model, laid, page);
    paint_page(laid, model, page, &parts, &cx).map(|p| page_svg(&p, &svg))
}

/// The relayout's facts and every page's SVG equal the full layout's.
fn outputs_equal(
    at: &str,
    (kept, kept_model, relaid): (&mut WordLayout, &mut Model, &DocumentLayout),
    (fresh, fresh_model, whole): (&mut WordLayout, &mut Model, &DocumentLayout),
) {
    assert!(LayoutFacts::of(relaid) == LayoutFacts::of(whole), "{at}: facts differ");
    for page in 0..whole.page_count() {
        let ours = page_svg_of(kept, kept_model, relaid, page);
        let theirs = page_svg_of(fresh, fresh_model, whole, page);
        assert!(ours == theirs, "{at}: page {page}'s SVG differs");
    }
}

fn check(seed: u64, versions: usize) -> Seen {
    let mut seen = Seen::default();
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut specs: Vec<Spec> = (0..40).map(|k| spec(&mut rng, k)).collect();
    let mut extra = Extra::new(&mut rng);
    let settings = Some(
        r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>"#,
    );
    let settings = if seed.is_multiple_of(2) { settings } else { None };
    let mut kept_model = model();
    let mut kept = engine();
    let first = open_with(&mut kept_model, &xml(&specs), settings, &extra.parts());
    let mut previous = kept.lay_out(&mut kept_model, &first);
    for version in 0..versions {
        edit(&mut rng, &mut specs, &mut extra);
        let body = xml(&specs);
        let parts = extra.parts();
        let document = open_with(&mut kept_model, &body, settings, &parts);
        let relaid = kept.relayout(&mut kept_model, &document, &previous);
        let mut fresh_model = model();
        let fresh_document = open_with(&mut fresh_model, &body, settings, &parts);
        let mut fresh = engine();
        let whole = fresh.lay_out(&mut fresh_model, &fresh_document);
        assert!(
            whole.pagination.stopped.is_none(),
            "seed {seed} version {version}: stopped {:?}",
            whole.pagination.stopped
        );
        same_passes(&relaid, &whole, &format!("seed {seed} version {version}"));
        assert!(
            relaid.pagination == whole.pagination,
            "seed {seed} version {version}: pages differ; stats {:?}",
            relaid.relaid
        );
        assert_eq!(
            relaid.fields, whole.fields,
            "seed {seed} version {version}: fields differ"
        );
        assert_eq!(
            (relaid.passes, relaid.settled),
            (whole.passes, whole.settled),
            "seed {seed} version {version}"
        );
        let at = format!("seed {seed} version {version}");
        outputs_equal(
            &at,
            (&mut kept, &mut kept_model, &relaid),
            (&mut fresh, &mut fresh_model, &whole),
        );
        let pages = &relaid.pagination.pages;
        seen.notes += pages.iter().filter(|p| !p.notes.is_empty()).count();
        seen.bands += pages.iter().filter(|p| !p.bands.is_empty()).count();
        seen.wraps += pages.iter().filter(|p| !p.wraps.is_empty()).count();
        seen.endnotes += relaid.roles.iter().filter(|r| r.is_some()).count();
        previous = relaid;
    }
    seen
}

#[test]
fn relayout_equals_full_layout() {
    let mut seen = Seen::default();
    for seed in 1..=6 {
        let found = check(seed, 8);
        seen.notes += found.notes;
        seen.bands += found.bands;
        seen.wraps += found.wraps;
        seen.endnotes += found.endnotes;
    }
    assert!(
        seen.notes > 0 && seen.bands > 0 && seen.wraps > 0 && seen.endnotes > 0,
        "the documents exercise notes and drawings: {seen:?}"
    );
}

#[test]
#[ignore = "many seeds: run at the end of a lane and nightly"]
fn many_seeds() {
    for seed in 1..=300 {
        let _ = check(seed, 12);
    }
}

#[test]
#[ignore = "sixty seeds: a lane's end gate (its rule: no long testing)"]
fn sixty_seeds() {
    for seed in 1..=60 {
        let _ = check(seed, 12);
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
            s.footnote = None;
            s.endnote = None;
            s.drawing = None;
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
