//! Lines against Word's baselines:
//!
//! - `baselines`: whole documents, the repository's fixtures (`baseline-observations.json`;
//!   docx2svg's `test_baselines.py`): every line Word drew, `[page, baseline, spans]`, held
//!   to the layout's line on the same page at the same baseline with the same text;
//! - `line_box`: the line box (`make_line_box_probe`, `line-box-observations.json`;
//!   `test_line_box.py`): single-line paragraphs with space before and after, bottom
//!   borders and every line rule, each page's baselines held to the layout's in order.

use super::{check_family, corpus_fixture, lay_out, observations, probe, Floors, Scored};
use std::collections::BTreeMap;

/// The baseline documents and their fixture folders.
const BASELINES: &[(&str, &str)] = &[
    ("style-document.docx", "docx2svg"),
    ("layout-sweep.docx", "docx2svg"),
    ("sample-blank.docx", "samplelib"),
    ("sample-long.docx", "samplelib"),
    ("sample-resume.docx", "samplelib"),
    ("sample-simple.docx", "samplelib"),
    ("sample-10pages.docx", "wordto"),
    ("sample-1page.docx", "wordto"),
    ("sample-5pages.docx", "wordto"),
    ("sample-with-images.docx", "wordto"),
    ("sample-with-table.docx", "wordto"),
];

/// A line's text as compared: without white space (Word's PDF spaces a tab or a justified
/// gap its own way) and without bullets (Word's `ToUnicode` gives a symbol face's bullet
/// as U+2022, the layout draws its private-use code).
fn bare(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{2022}' && !('\u{f000}'..='\u{f0ff}').contains(c))
        .collect()
}

/// `round` as Python's: half to even.
fn round(v: f64) -> i64 {
    v.round_ties_even() as i64
}

/// The layout's text per page and baseline: every painted line's spans in pen order, the
/// lines on one baseline joined left to right (Word's lines are a baseline's text).
fn laid_lines(pages: &[std::sync::Arc<ooxml_word_layout::paint::PagePaint>]) -> BTreeMap<(usize, i64), String> {
    let mut spans: BTreeMap<(usize, i64), Vec<(f64, String)>> = BTreeMap::new();
    for (page, paint) in pages.iter().enumerate() {
        let boxed = paint.text_boxes.iter().flat_map(|b| b.lines.iter());
        for line in paint.lines.iter().chain(boxed) {
            for span in &line.spans {
                let x = span.pens().next().map_or(0.0, |x| {
                    let q = x.value();
                    *q.numer() as f64 / *q.denom() as f64
                });
                let text: String = span.chars.iter().collect();
                spans.entry((page, line.baseline)).or_default().push((x, text));
            }
        }
    }
    spans
        .into_iter()
        .map(|(key, mut parts)| {
            parts.sort_by(|a, b| a.0.total_cmp(&b.0));
            (key, bare(&parts.into_iter().map(|(_, t)| t).collect::<String>()))
        })
        .collect()
}

/// Word's lines `[page, baseline, text]` (or `[page, baseline, [[face, size, text], ...]]`)
/// against the layout's: a line agrees where the layout draws the same text on the same
/// page at the same baseline.
pub fn text_lines(
    scored: &mut Scored,
    case: &str,
    word: &serde_json::Value,
    pages: &[std::sync::Arc<ooxml_word_layout::paint::PagePaint>],
) {
    let laid = laid_lines(pages);
    let mut bad = 0;
    let mut total = 0;
    let mut problems = Vec::new();
    for line in word.as_array().into_iter().flatten() {
        let page = line[0].as_u64().unwrap_or_default() as usize;
        let y = round(line[1].as_f64().unwrap_or_default());
        let text = match &line[2] {
            serde_json::Value::String(text) => text.clone(),
            spans => spans
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s[2].as_str())
                .collect(),
        };
        let text = bare(&text);
        if text.is_empty() {
            continue;
        }
        total += 1;
        if laid.get(&(page, y)) != Some(&text) {
            bad += 1;
            let found = laid
                .iter()
                .find(|((p, _), t)| *p == page && **t == text)
                .map_or("not on the page".to_owned(), |((_, at), _)| format!("at y={at}"));
            problems.push(format!("p{} y={y} {text:?}: the layout's {found}", page + 1));
        }
    }
    scored.cases.insert(case.to_owned(), (bad, total));
    scored.problems.insert(case.to_owned(), problems);
}

/// Today's agreement. The fixtures set in Cambria (samplelib, wordto) are laid out with
/// Caladea, whose line metrics are not Cambria's: their lines are a few pixels off Word's
/// from the first (docx2svg read Cambria's own numbers from the recording).
#[rustfmt::skip]
const BASELINE_FLOORS: Floors = &[
    ("style-document.docx", &[("lines all", 33, 33)]),
    ("layout-sweep.docx", &[("lines all", 91, 91)]),
    ("sample-blank.docx", &[("lines all", 0, 0)]),
    ("sample-long.docx", &[("lines all", 2, 453)]),
    ("sample-resume.docx", &[("lines all", 1, 20)]),
    ("sample-simple.docx", &[("lines all", 1, 28)]),
    ("sample-10pages.docx", &[("lines all", 4, 89)]),
    ("sample-1page.docx", &[("lines all", 1, 7)]),
    ("sample-5pages.docx", &[("lines all", 1, 56)]),
    ("sample-with-images.docx", &[("lines all", 1, 16)]),
    ("sample-with-table.docx", &[("lines all", 1, 42)]),
];

#[test]
fn baselines() {
    let recording = observations("baseline-observations.json");
    let names: Vec<&str> = BASELINES.iter().map(|(name, _)| *name).collect();
    check_family(&names, BASELINE_FLOORS, |name| {
        let folder = BASELINES.iter().find(|(n, _)| *n == name).map_or("", |(_, f)| f);
        let laid = lay_out(name, corpus_fixture(folder, name));
        let mut scored = Scored::default();
        text_lines(&mut scored, "lines all", &recording["documents"][name], &laid.pages);
        scored
    });
}

/// Today's agreement: every probe but Cambria's exact; Cambria's is laid out with Caladea,
/// whose line metrics are not Cambria's.
#[rustfmt::skip]
const LINE_BOX_FLOORS: Floors = &[
    ("line-box-after-arial-13", &[("lines all", 123, 123)]),
    ("line-box-after-arial-22", &[("lines all", 124, 124)]),
    ("line-box-after-arial-29", &[("lines all", 125, 125)]),
    ("line-box-after-arial-40", &[("lines all", 127, 127)]),
    ("line-box-after-calibri-13", &[("lines all", 123, 123)]),
    ("line-box-after-calibri-22", &[("lines all", 124, 124)]),
    ("line-box-after-calibri-29", &[("lines all", 125, 125)]),
    ("line-box-after-calibri-40", &[("lines all", 127, 127)]),
    ("line-box-after-cambria-13", &[("lines all", 0, 123)]),
    ("line-box-after-cambria-22", &[("lines all", 0, 124)]),
    ("line-box-after-cambria-29", &[("lines all", 0, 125)]),
    ("line-box-after-cambria-40", &[("lines all", 0, 127)]),
    ("line-box-after-timesnewroman-13", &[("lines all", 123, 123)]),
    ("line-box-after-timesnewroman-22", &[("lines all", 124, 124)]),
    ("line-box-after-timesnewroman-29", &[("lines all", 125, 125)]),
    ("line-box-after-timesnewroman-40", &[("lines all", 127, 127)]),
    ("line-box-before-calibri-13", &[("lines all", 123, 123)]),
    ("line-box-before-calibri-22", &[("lines all", 124, 124)]),
    ("line-box-before-calibri-29", &[("lines all", 125, 125)]),
    ("line-box-before-calibri-40", &[("lines all", 127, 127)]),
    ("line-box-before-cambria-13", &[("lines all", 0, 123)]),
    ("line-box-before-cambria-22", &[("lines all", 0, 124)]),
    ("line-box-before-cambria-29", &[("lines all", 0, 125)]),
    ("line-box-before-cambria-40", &[("lines all", 0, 127)]),
    ("line-box-before-timesnewroman-13", &[("lines all", 123, 123)]),
    ("line-box-before-timesnewroman-22", &[("lines all", 124, 124)]),
    ("line-box-before-timesnewroman-29", &[("lines all", 125, 125)]),
    ("line-box-before-timesnewroman-40", &[("lines all", 127, 127)]),
    ("line-box-border-calibri-22", &[("lines all", 267, 267)]),
    ("line-box-border-calibri-29", &[("lines all", 269, 269)]),
    ("line-box-border-calibri-40", &[("lines all", 273, 273)]),
    ("line-box-border-timesnewroman-22", &[("lines all", 267, 267)]),
    ("line-box-border-timesnewroman-29", &[("lines all", 269, 269)]),
    ("line-box-border-timesnewroman-40", &[("lines all", 273, 273)]),
    ("line-box-combo-after-calibri-22", &[("lines all", 446, 446)]),
    ("line-box-combo-after-timesnewroman-29", &[("lines all", 448, 448)]),
    ("line-box-combo-before-calibri-22", &[("lines all", 446, 446)]),
    ("line-box-combo-before-timesnewroman-29", &[("lines all", 448, 448)]),
    ("line-box-combo-both-calibri-22", &[("lines all", 447, 447)]),
    ("line-box-combo-both-timesnewroman-29", &[("lines all", 449, 449)]),
    ("line-box-exact-calibri-13", &[("lines all", 249, 249)]),
    ("line-box-exact-calibri-22", &[("lines all", 249, 249)]),
    ("line-box-exact-couriernew-20", &[("lines all", 249, 249)]),
    ("line-box-exact-timesnewroman-22", &[("lines all", 249, 249)]),
    ("line-box-multiple-after-calibri-22", &[("lines all", 125, 125)]),
    ("line-box-multiple-after-cambria-22", &[("lines all", 0, 125)]),
    ("line-box-multiple-arial-29", &[("lines all", 505, 505)]),
    ("line-box-multiple-calibri-22", &[("lines all", 499, 499)]),
    ("line-box-multiple-calibri-40", &[("lines all", 515, 515)]),
    ("line-box-multiple-cambria-22", &[("lines all", 0, 499)]),
    ("line-box-multiple-cambria-29", &[("lines all", 0, 505)]),
    ("line-box-multiple-timesnewroman-29", &[("lines all", 505, 505)]),
];

#[test]
fn line_box() {
    let recording = observations("line-box-observations.json");
    let probes = recording["probes"]
        .as_object()
        .unwrap_or_else(|| panic!("line-box: no probes"));
    let names: Vec<&str> = probes.keys().map(String::as_str).collect();
    check_family(&names, LINE_BOX_FLOORS, |name| {
        let laid = lay_out(name, probe("make_line_box_probe", name));
        let mut bad = 0;
        let mut total = 0;
        let mut problems = Vec::new();
        for (page, word) in probes[name].as_array().into_iter().flatten().enumerate() {
            let mut ours: Vec<i64> = laid
                .pages
                .get(page)
                .map(|paint| {
                    paint
                        .lines
                        .iter()
                        .filter(|l| l.story.is_none())
                        .map(|l| l.baseline)
                        .collect()
                })
                .unwrap_or_default();
            ours.sort_unstable();
            ours.dedup();
            for (k, baseline) in word.as_array().into_iter().flatten().enumerate() {
                let baseline = baseline.as_i64().unwrap_or_default();
                total += 1;
                if ours.get(k) != Some(&baseline) {
                    bad += 1;
                    problems.push(format!(
                        "p{} line {k}: Word {baseline}, layout {:?}",
                        page + 1,
                        ours.get(k)
                    ));
                }
            }
        }
        let mut scored = Scored::default();
        scored.cases.insert("lines all".into(), (bad, total));
        scored.problems.insert("lines all".into(), problems);
        scored
    });
}
