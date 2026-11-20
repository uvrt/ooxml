//! The Word model on made-up documents: the cascade's toggle rule, fields threading, the
//! final view of a deleted mark, list labels (incremental against from scratch), table
//! regions, and reopening a version that changed one block.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::support;

use ooxml_word_model::labels::Counters;
use ooxml_word_model::model::Block;
use ooxml_word_model::value::Value;
use support::{model, open, p, package, Parts};

fn first_paragraph(d: &ooxml_word_model::WordDocument, k: usize) -> std::sync::Arc<ooxml_word_model::model::Paragraph> {
    match &d.body[k] {
        Block::Paragraph(p) => p.clone(),
        Block::Table(_) => panic!("block {k} is a table"),
    }
}

fn bold(parts: &Parts) -> bool {
    let (mut m, d) = open(parts);
    let para = first_paragraph(&d, 0);
    let r = m.resolve_paragraph(&d, &para);
    r.runs[0].get("b") == Some(&Value::Bool(true))
}

const TOGGLE_STYLES: &str = r#"<w:style w:type="paragraph" w:styleId="PB"><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="character" w:styleId="CB"><w:rPr><w:b/></w:rPr></w:style>"#;

#[test]
fn toggles_flip_across_style_levels_as_word_draws_them() {
    let styled = |defaults: &str, direct: &str| Parts {
        body: p(
            r#"<w:pStyle w:val="PB"/>"#,
            &format!(r#"<w:rStyle w:val="CB"/>{direct}"#),
            "x",
        ),
        styles: Some(format!(
            "<w:docDefaults><w:rPrDefault><w:rPr>{defaults}</w:rPr></w:rPrDefault></w:docDefaults>{TOGGLE_STYLES}"
        )),
        ..Parts::default()
    };
    // Paragraph bold and character bold: not bold (t03); with w:docDefaults bold, bold (g07).
    assert!(!bold(&styled("", "")));
    assert!(bold(&styled("<w:b/>", "")));
    // Direct formatting sets it absolutely (t04).
    assert!(bold(&styled("", "<w:b/>")));
    assert!(!bold(&styled("<w:b/>", r#"<w:b w:val="0"/>"#)));
}

#[test]
fn computed_fields_are_one_run_and_cached_ones_their_result() {
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE \* roman</w:instrText></w:r>
<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>iii</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>DATE</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>
<w:r><w:t>today</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let (_, d) = open(&Parts {
        body: body.into(),
        ..Parts::default()
    });
    let para = first_paragraph(&d, 0);
    let computed: Vec<_> = para.runs.iter().filter_map(|r| r.field.as_ref()).collect();
    assert_eq!(computed.len(), 1);
    assert_eq!(&*computed[0].keyword, "PAGE");
    assert_eq!(&*computed[0].instruction, r"PAGE \* roman");
    assert_eq!(para.text(), "iiitoday");
    assert_eq!(d.cached_fields.iter().map(|k| &**k).collect::<Vec<_>>(), ["DATE"]);
}

#[test]
fn a_deleted_mark_joins_the_paragraph_after_it() {
    let body = format!(
        "{}{}",
        r#"<w:p><w:pPr><w:rPr><w:del w:id="1" w:author="a"/></w:rPr></w:pPr><w:r><w:t>one </w:t></w:r></w:p>"#,
        p(r#"<w:jc w:val="center"/>"#, "", "two")
    );
    let (_, d) = open(&Parts {
        body,
        ..Parts::default()
    });
    assert_eq!(d.body.len(), 1);
    let joined = first_paragraph(&d, 0);
    assert_eq!(joined.text(), "one two");
    assert_eq!(&*joined.joins, &[1]);
    assert_eq!(&*joined.path, "w:body/w:p[1]");
    let jc = joined.properties.as_ref().unwrap().declared.get("jc");
    assert_eq!(jc, Some(&Value::Str("center".into())));
}

const NUMBERING: &str = r#"<w:abstractNum w:abstractNumId="1">
<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>
<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2"/></w:lvl>
<w:lvl w:ilvl="2"><w:start w:val="1"/><w:numFmt w:val="lowerRoman"/><w:lvlText w:val="(%3)"/><w:lvlRestart w:val="0"/></w:lvl>
</w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="5"/></w:lvlOverride></w:num>
<w:num w:numId="3"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="1"><w:startOverride w:val="3"/></w:lvlOverride></w:num>"#;

fn item(num: i64, ilvl: i64) -> String {
    if num == 0 {
        return p("", "", "plain");
    }
    p(
        &format!(r#"<w:numPr><w:ilvl w:val="{ilvl}"/><w:numId w:val="{num}"/></w:numPr>"#),
        "",
        "item",
    )
}

fn list(items: &[(i64, i64)]) -> Parts {
    Parts {
        body: items.iter().map(|&(n, l)| item(n, l)).collect(),
        numbering: Some(NUMBERING.into()),
        ..Parts::default()
    }
}

#[test]
fn labels_count_as_word_counts_them() {
    let (_, d) = open(&list(&[
        (1, 0),
        (1, 1),
        (1, 1),
        (1, 0),
        (0, 0),
        (2, 0),
        (2, 0),
        (1, 1),
        (3, 1),
        (3, 1),
    ]));
    let labels: Vec<_> = d.body_labels().into_iter().map(|l| l.unwrap_or_default()).collect();
    assert_eq!(labels, ["1.", "1.a", "1.b", "2.", "", "5.", "6.", "6.a", "6.c", "6.d"]);
    for (k, expected) in labels.iter().enumerate() {
        assert_eq!(d.label(k, 0).unwrap_or_default(), *expected, "block {k}");
    }
}

/// A small deterministic generator (no dependency): xorshift.
struct Rng(u64);

impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

fn scratch(items: &[(i64, i64)], numbering: &ooxml_word_model::numbering::Numbering) -> Vec<Option<String>> {
    let mut counters = Counters::default();
    items
        .iter()
        .map(|&(n, l)| if n == 0 { None } else { counters.item(numbering, n, l) })
        .collect()
}

#[test]
fn incremental_labels_equal_labels_from_scratch_after_every_edit() {
    for seed in [3, 17, 2026] {
        let mut rng = Rng(seed);
        let mut items: Vec<(i64, i64)> = (0..40).map(|_| (rng.next(4) as i64, rng.next(3) as i64)).collect();
        let mut m = model();
        for step in 0..30 {
            let d = m.open(&package(&list(&items))).unwrap();
            assert_eq!(
                d.body_labels(),
                scratch(&items, &d.context.numbering),
                "seed {seed} step {step}"
            );
            let at = rng.next(items.len() as u64) as usize;
            match rng.next(3) {
                0 => items.insert(at, (rng.next(4) as i64, rng.next(3) as i64)),
                1 if items.len() > 2 => {
                    items.remove(at);
                }
                _ => items[at] = (rng.next(4) as i64, rng.next(3) as i64),
            }
        }
    }
}

#[test]
fn a_reopened_version_reads_only_the_changed_block_and_equals_a_fresh_open() {
    let items: Vec<(i64, i64)> = (0..30).map(|k| (1 + (k % 2), k % 2)).collect();
    let parts = list(&items);
    let mut m = model();
    m.open(&package(&parts)).unwrap();
    let mut changed = parts.clone();
    changed.body = changed.body.replacen(">item<", ">edited<", 1);
    let reopened = m.open(&package(&changed)).unwrap();
    let stats = m.stats();
    assert_eq!((stats.elements, stats.elements_read, stats.label_blocks), (30, 1, 0));
    let (_, fresh) = open(&changed);
    assert_eq!(reopened.body, fresh.body);
    assert_eq!(reopened.body_labels(), fresh.body_labels());
    // Making a paragraph a list item finds that block's items again, and the labels after.
    let mut listed = changed.clone();
    listed.body = listed.body.replacen(&item(1, 0), &item(2, 0), 1);
    let again = m.open(&package(&listed)).unwrap();
    assert_eq!((m.stats().elements_read, m.stats().label_blocks), (1, 1));
    assert_eq!(again.body_labels(), open(&listed).1.body_labels());
}

#[test]
fn a_content_controls_paragraphs_keep_their_paths_when_it_is_taken_as_it_was() {
    let body = format!(
        "{}<w:sdt><w:sdtContent>{}</w:sdtContent></w:sdt>{}",
        p("", "", "before"),
        p("", "", "inside"),
        p("", "", "after")
    );
    let parts = Parts {
        body,
        ..Parts::default()
    };
    let mut m = model();
    m.open(&package(&parts)).unwrap();
    let mut changed = parts.clone();
    changed.body = changed.body.replacen(">before<", ">edited<", 1);
    let reopened = m.open(&package(&changed)).unwrap();
    assert_eq!(m.stats().elements_read, 1);
    let Block::Paragraph(inside) = &reopened.body[1] else {
        panic!("not a paragraph")
    };
    assert_eq!(&*inside.path, "w:body/w:sdt[1]/w:sdtContent[1]/w:p[1]");
    assert_eq!(reopened.body, open(&changed).1.body);
}

#[test]
fn a_table_style_region_formats_the_cells_in_it() {
    let styles = r#"<w:style w:type="table" w:styleId="T"><w:tblPr><w:tblStyleRowBandSize w:val="1"/></w:tblPr>
<w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr></w:tblStylePr>
<w:tblStylePr w:type="band1Horz"><w:tcPr><w:shd w:val="clear" w:fill="EEEEEE"/></w:tcPr></w:tblStylePr></w:style>"#;
    let cell = |t: &str| format!("<w:tc>{}</w:tc>", p("", "", t));
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="T"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>"#,
        cell("a"),
        cell("b"),
        cell("c"),
        cell("d")
    );
    let (mut m, d) = open(&Parts {
        body,
        styles: Some(styles.into()),
        ..Parts::default()
    });
    let Block::Table(table) = &d.body[0] else {
        panic!("not a table")
    };
    let resolved = m.resolve_table(&d, table);
    assert_eq!(resolved.rows.len(), 2);
    assert!(resolved.rows[1].cells[0].properties.contains_key("shd"));
    assert!(!resolved.rows[0].cells[0].properties.contains_key("shd"));
    let first = match &table.rows[0][0][0] {
        Block::Paragraph(p) => p.clone(),
        Block::Table(_) => panic!(),
    };
    // No w:tblLook: ECMA-376's default, the header row and first column on.
    assert_eq!(
        &*first.table_conditions,
        &["firstCol".into(), "firstRow".into(), "nwCell".into()]
    );
    let r = m.resolve_paragraph(&d, &first);
    assert_eq!(r.runs[0].get("b"), Some(&Value::Bool(true)));
}

#[test]
fn the_model_and_its_versions_cross_threads() {
    fn shared<T: Send + Sync>() {}
    fn owned<T: Send>() {}
    shared::<ooxml_word_model::WordDocument>();
    owned::<ooxml_word_model::Model>();
}
