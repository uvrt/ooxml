//! Pagination and keeps against the pages Word drew:
//!
//! - `keeps`, `pagination`, `page_fit`, `sections`: widow and orphan control, keepLines,
//!   keepNext and its chains, page breaks (`make_keep_probe`), page sizes and margins
//!   (`make_pagination_probe`), the fit of a page's last line swept by single layout units
//!   (`make_page_fit_probe`) and section breaks (`make_section_probe`): the page Word drew
//!   each line of each paragraph on (docx2svg's `test_paginate.py`, the recordings'
//!   `blocks`/`lines`/`tops`), held to the page the layout puts it on;
//! - `page_top`, `page_top_collapse`: space before at a page's top, after natural and manual
//!   breaks and sections (`make_page_top_probe`, `make_page_top_collapse_probe`;
//!   `test_page_top.py`): every line Word drew, on the same page at the same baseline with
//!   the same text;
//! - `columns`, `break_end`: text in columns, balanced, broken, with notes, pictures and
//!   tables (`make_columns_probe`; `test_columns.py`), and a page break at a paragraph's end
//!   (`make_break_end_probe`; `test_break_end.py`): every line within half a pixel, every
//!   glyph where Word drew one, and the pages.

use super::notes::{objects_family, text_family};
use super::{check_family, documents_of, lay_out, observations, probe, Floors, Scored};
use std::collections::BTreeMap;

/// Word's pages of each paragraph's lines (`pages.expand_pages`): per body paragraph, in
/// order, the page of each line it drew; a paragraph none of whose lines was matched is
/// left out.
fn word_pages(recorded: &serde_json::Value) -> BTreeMap<usize, Vec<usize>> {
    let starts: BTreeMap<(u64, u64), u64> = recorded["tops"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let n = |k: usize| t[k].as_u64().unwrap_or_default();
            ((n(1), n(2)), n(0))
        })
        .collect();
    let mut out = BTreeMap::new();
    let mut page = None;
    for block in 0..recorded["blocks"].as_u64().unwrap_or_default() {
        let count = recorded["lines"]
            .get(block.to_string())
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1);
        if count == 0 {
            continue;
        }
        let mut lines = Vec::new();
        for line in 0..count {
            page = starts.get(&(block, line)).copied().or(page);
            lines.push(page.unwrap_or_default() as usize);
        }
        out.insert(block as usize, lines);
    }
    out
}

/// The layout's pages of each body paragraph's lines: `w:body/w:p[k]` is paragraph `k - 1`.
fn laid_pages(pages: &[std::sync::Arc<ooxml_word_layout::paint::PagePaint>]) -> BTreeMap<usize, Vec<usize>> {
    let mut out: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (page, paint) in pages.iter().enumerate() {
        for line in paint.lines.iter().filter(|l| l.story.is_none()) {
            let block = line
                .path
                .strip_prefix("w:body/w:p[")
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|k| k.parse::<usize>().ok());
            if let Some(k) = block {
                out.entry(k - 1).or_default().push(page);
            }
        }
    }
    out
}

/// A recording of paragraphs' pages: every line of Word's on the page the layout puts the
/// same line of the same paragraph on.
fn pages_family(recording: &str, maker: &str, prefix: &str, floors: Floors) {
    let recording = observations(recording);
    let names = documents_of(&recording);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    check_family(&names, floors, |name| {
        let laid = lay_out(name, probe(maker, &format!("{prefix}{name}")));
        let ours = laid_pages(&laid.pages);
        let mut bad = 0;
        let mut total = 0;
        let mut problems = Vec::new();
        for (block, lines) in word_pages(&recording["documents"][name]) {
            for (line, page) in lines.iter().enumerate() {
                total += 1;
                let laid = ours.get(&block).and_then(|l| l.get(line));
                if laid != Some(page) {
                    bad += 1;
                    problems.push(format!(
                        "paragraph {block} line {line}: Word p{}, layout {laid:?}",
                        page + 1
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

/// Today's agreement.
#[rustfmt::skip]
const KEEP_FLOORS: Floors = &[
    ("12", &[("lines all", 12172, 12172)]),
    ("14", &[("lines all", 12172, 12172)]),
    ("15", &[("lines all", 12172, 12172)]),
    ("none", &[("lines all", 12172, 12172)]),
    ("nostyles", &[("lines all", 1433, 1433)]),
];

#[test]
fn keeps() {
    pages_family("keep-observations.json", "make_keep_probe", "keep-", KEEP_FLOORS);
}

/// Today's agreement: a paragraph broken into another count of lines than Word's moves every
/// later line, so a document disagrees from its first difference on.
#[rustfmt::skip]
const PAGINATION_FLOORS: Floors = &[
    ("a4-narrow", &[("lines all", 891, 930)]),
    ("a4-normal", &[("lines all", 995, 1071)]),
    ("a4-wide", &[("lines all", 299, 1151)]),
    ("a5-narrow", &[("lines all", 150, 1345)]),
    ("a5-normal", &[("lines all", 116, 1681)]),
    ("a5-wide", &[("lines all", 394, 1927)]),
    ("letter-narrow", &[("lines all", 106, 902)]),
    ("letter-normal", &[("lines all", 981, 1036)]),
    ("letter-normal-12", &[("lines all", 982, 1036)]),
    ("letter-normal-14", &[("lines all", 982, 1036)]),
    ("letter-normal-none", &[("lines all", 982, 1036)]),
    ("letter-wide", &[("lines all", 999, 1110)]),
    ("sections", &[("lines all", 584, 1132)]),
];

#[test]
fn pagination() {
    pages_family(
        "pagination-observations.json",
        "make_pagination_probe",
        "",
        PAGINATION_FLOORS,
    );
}

/// Today's agreement.
#[rustfmt::skip]
const PAGE_FIT_FLOORS: Floors = &[
    ("12", &[("lines all", 22458, 22458)]),
    ("14", &[("lines all", 22458, 22458)]),
    ("15", &[("lines all", 22458, 22458)]),
    ("none", &[("lines all", 22458, 22458)]),
];

#[test]
fn page_fit() {
    pages_family(
        "page-fit-observations.json",
        "make_page_fit_probe",
        "page-fit-",
        PAGE_FIT_FLOORS,
    );
}

/// Today's agreement.
#[rustfmt::skip]
const SECTION_FLOORS: Floors = &[
    ("14", &[("lines all", 260, 260)]),
    ("15", &[("lines all", 260, 260)]),
    ("none", &[("lines all", 260, 260)]),
];

#[test]
fn sections() {
    pages_family(
        "section-observations.json",
        "make_section_probe",
        "section-",
        SECTION_FLOORS,
    );
}

/// Today's agreement.
#[rustfmt::skip]
const PAGE_TOP_FLOORS: Floors = &[
    ("page-top-12-0", &[("lines all", 238, 238)]),
    ("page-top-12-1", &[("lines all", 238, 238)]),
    ("page-top-12-2", &[("lines all", 238, 238)]),
    ("page-top-12-3", &[("lines all", 238, 238)]),
    ("page-top-12-4", &[("lines all", 238, 238)]),
    ("page-top-12-5", &[("lines all", 238, 238)]),
    ("page-top-12-6", &[("lines all", 238, 238)]),
    ("page-top-12-7", &[("lines all", 238, 238)]),
    ("page-top-14-0", &[("lines all", 238, 238)]),
    ("page-top-14-1", &[("lines all", 238, 238)]),
    ("page-top-14-2", &[("lines all", 238, 238)]),
    ("page-top-14-3", &[("lines all", 238, 238)]),
    ("page-top-14-4", &[("lines all", 238, 238)]),
    ("page-top-14-5", &[("lines all", 238, 238)]),
    ("page-top-14-6", &[("lines all", 238, 238)]),
    ("page-top-14-7", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-0", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-1", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-2", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-3", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-4", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-5", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-6", &[("lines all", 238, 238)]),
    ("page-top-14-suppress-7", &[("lines all", 238, 238)]),
    ("page-top-15-0", &[("lines all", 238, 238)]),
    ("page-top-15-1", &[("lines all", 238, 238)]),
    ("page-top-15-2", &[("lines all", 238, 238)]),
    ("page-top-15-3", &[("lines all", 238, 238)]),
    ("page-top-15-4", &[("lines all", 238, 238)]),
    ("page-top-15-5", &[("lines all", 238, 238)]),
    ("page-top-15-6", &[("lines all", 238, 238)]),
    ("page-top-15-7", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-0", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-1", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-2", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-3", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-4", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-5", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-6", &[("lines all", 238, 238)]),
    ("page-top-compat-empty-7", &[("lines all", 238, 238)]),
    ("page-top-nocompat-0", &[("lines all", 238, 238)]),
    ("page-top-nocompat-1", &[("lines all", 238, 238)]),
    ("page-top-nocompat-2", &[("lines all", 238, 238)]),
    ("page-top-nocompat-3", &[("lines all", 238, 238)]),
    ("page-top-nocompat-4", &[("lines all", 238, 238)]),
    ("page-top-nocompat-5", &[("lines all", 238, 238)]),
    ("page-top-nocompat-6", &[("lines all", 238, 238)]),
    ("page-top-nocompat-7", &[("lines all", 238, 238)]),
    ("page-top-none-0", &[("lines all", 238, 238)]),
    ("page-top-none-1", &[("lines all", 238, 238)]),
    ("page-top-none-2", &[("lines all", 238, 238)]),
    ("page-top-none-3", &[("lines all", 238, 238)]),
    ("page-top-none-4", &[("lines all", 238, 238)]),
    ("page-top-none-5", &[("lines all", 238, 238)]),
    ("page-top-none-6", &[("lines all", 238, 238)]),
    ("page-top-none-7", &[("lines all", 238, 238)]),
];

#[test]
fn page_top() {
    text_family("page-top-observations.json", "make_page_top_probe", "", PAGE_TOP_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const PAGE_TOP_COLLAPSE_FLOORS: Floors = &[
    ("12", &[("lines all", 181, 181)]),
    ("14", &[("lines all", 181, 181)]),
    ("15", &[("lines all", 181, 181)]),
    ("none", &[("lines all", 181, 181)]),
];

#[test]
fn page_top_collapse() {
    text_family(
        "page-top-collapse-observations.json",
        "make_page_top_collapse_probe",
        "page-top-collapse-",
        PAGE_TOP_COLLAPSE_FLOORS,
    );
}

/// Today's agreement.
#[rustfmt::skip]
const COLUMNS_FLOORS: Floors = &[
    ("cols-balance-14", &[("glyphs extra", 1503, 1503), ("glyphs face", 1503, 1503), ("glyphs not drawn", 1503, 1503), ("glyphs size", 1503, 1503), ("lines all", 78, 78), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-balance-15", &[("glyphs extra", 1503, 1503), ("glyphs face", 1503, 1503), ("glyphs not drawn", 1503, 1503), ("glyphs size", 1503, 1503), ("lines all", 78, 78), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-balance-none", &[("glyphs extra", 1503, 1503), ("glyphs face", 1503, 1503), ("glyphs not drawn", 1503, 1503), ("glyphs size", 1503, 1503), ("lines all", 78, 78), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-balancelong-14", &[("glyphs extra", 900, 900), ("glyphs face", 900, 900), ("glyphs not drawn", 900, 900), ("glyphs size", 900, 900), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-balancelong-15", &[("glyphs extra", 900, 900), ("glyphs face", 900, 900), ("glyphs not drawn", 900, 900), ("glyphs size", 900, 900), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-balancelong-none", &[("glyphs extra", 900, 900), ("glyphs face", 900, 900), ("glyphs not drawn", 900, 900), ("glyphs size", 900, 900), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-breaks-14", &[("glyphs extra", 307, 307), ("glyphs face", 307, 307), ("glyphs not drawn", 307, 307), ("glyphs size", 307, 307), ("lines all", 25, 25), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-breaks-15", &[("glyphs extra", 307, 307), ("glyphs face", 307, 307), ("glyphs not drawn", 307, 307), ("glyphs size", 307, 307), ("lines all", 25, 25), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-breaks-none", &[("glyphs extra", 307, 307), ("glyphs face", 307, 307), ("glyphs not drawn", 307, 307), ("glyphs size", 307, 307), ("lines all", 25, 25), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-dropcap-14", &[("glyphs extra", 1188, 1188), ("glyphs face", 1188, 1188), ("glyphs not drawn", 1188, 1188), ("glyphs size", 1188, 1188), ("lines all", 33, 33), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-dropcap-15", &[("glyphs extra", 1188, 1188), ("glyphs face", 1188, 1188), ("glyphs not drawn", 1188, 1188), ("glyphs size", 1188, 1188), ("lines all", 33, 33), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-dropcap-none", &[("glyphs extra", 1188, 1188), ("glyphs face", 1188, 1188), ("glyphs not drawn", 1188, 1188), ("glyphs size", 1188, 1188), ("lines all", 33, 33), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-endnotes-14", &[("glyphs extra", 1301, 1301), ("glyphs face", 1301, 1301), ("glyphs not drawn", 1301, 1301), ("glyphs size", 1301, 1301), ("lines all", 67, 67), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-endnotes-15", &[("glyphs extra", 722, 1301), ("glyphs face", 722, 722), ("glyphs not drawn", 722, 1301), ("glyphs size", 707, 722), ("lines all", 1, 45), ("pages", 1, 1), ("warnings", 0, 1)]),
    ("cols-endnotes-none", &[("glyphs extra", 1301, 1301), ("glyphs face", 1301, 1301), ("glyphs not drawn", 1301, 1301), ("glyphs size", 1301, 1301), ("lines all", 67, 67), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-floating-14", &[("glyphs extra", 3739, 3739), ("glyphs face", 3739, 3739), ("glyphs not drawn", 3739, 3739), ("glyphs size", 3739, 3739), ("lines all", 111, 111), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-floating-15", &[("glyphs extra", 3739, 3739), ("glyphs face", 3739, 3739), ("glyphs not drawn", 3739, 3739), ("glyphs size", 3739, 3739), ("lines all", 111, 111), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-floating-none", &[("glyphs extra", 3739, 3739), ("glyphs face", 3739, 3739), ("glyphs not drawn", 3739, 3739), ("glyphs size", 3739, 3739), ("lines all", 111, 111), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-flow2-14", &[("glyphs extra", 10959, 10959), ("glyphs face", 10959, 10959), ("glyphs not drawn", 10959, 10959), ("glyphs size", 10959, 10959), ("lines all", 257, 257), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-flow2-15", &[("glyphs extra", 10959, 10959), ("glyphs face", 10959, 10959), ("glyphs not drawn", 10959, 10959), ("glyphs size", 10959, 10959), ("lines all", 257, 257), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-flow2-none", &[("glyphs extra", 10959, 10959), ("glyphs face", 10959, 10959), ("glyphs not drawn", 10959, 10959), ("glyphs size", 10959, 10959), ("lines all", 257, 257), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-flow3-14", &[("glyphs extra", 8076, 8076), ("glyphs face", 8076, 8076), ("glyphs not drawn", 8076, 8076), ("glyphs size", 8076, 8076), ("lines all", 283, 283), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-flow3-15", &[("glyphs extra", 8076, 8076), ("glyphs face", 8076, 8076), ("glyphs not drawn", 8076, 8076), ("glyphs size", 8076, 8076), ("lines all", 283, 283), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-flow3-none", &[("glyphs extra", 8076, 8076), ("glyphs face", 8076, 8076), ("glyphs not drawn", 8076, 8076), ("glyphs size", 8076, 8076), ("lines all", 283, 283), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-footnote-14", &[("glyphs extra", 1643, 1643), ("glyphs face", 1643, 1643), ("glyphs not drawn", 1643, 1643), ("glyphs size", 1643, 1643), ("lines all", 56, 56), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-footnote-15", &[("glyphs extra", 1643, 1643), ("glyphs face", 1643, 1643), ("glyphs not drawn", 1643, 1643), ("glyphs size", 1643, 1643), ("lines all", 56, 56), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-footnote-none", &[("glyphs extra", 1643, 1643), ("glyphs face", 1643, 1643), ("glyphs not drawn", 1643, 1643), ("glyphs size", 1643, 1643), ("lines all", 56, 56), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-geometry-14", &[("glyphs extra", 442, 442), ("glyphs face", 442, 442), ("glyphs not drawn", 442, 442), ("glyphs size", 442, 442), ("lines all", 27, 27), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-geometry-15", &[("glyphs extra", 442, 442), ("glyphs face", 442, 442), ("glyphs not drawn", 442, 442), ("glyphs size", 442, 442), ("lines all", 27, 27), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-geometry-none", &[("glyphs extra", 442, 442), ("glyphs face", 442, 442), ("glyphs not drawn", 442, 442), ("glyphs size", 442, 442), ("lines all", 27, 27), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-header-14", &[("glyphs extra", 7111, 7111), ("glyphs face", 7111, 7111), ("glyphs not drawn", 7111, 7111), ("glyphs size", 7111, 7111), ("lines all", 110, 110), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-header-15", &[("glyphs extra", 7111, 7111), ("glyphs face", 7111, 7111), ("glyphs not drawn", 7111, 7111), ("glyphs size", 7111, 7111), ("lines all", 110, 110), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-header-none", &[("glyphs extra", 7111, 7111), ("glyphs face", 7111, 7111), ("glyphs not drawn", 7111, 7111), ("glyphs size", 7111, 7111), ("lines all", 110, 110), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-keeps-14", &[("glyphs extra", 3164, 3164), ("glyphs face", 3164, 3164), ("glyphs not drawn", 3164, 3164), ("glyphs size", 3164, 3164), ("lines all", 309, 309), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-keeps-15", &[("glyphs extra", 3164, 3164), ("glyphs face", 3164, 3164), ("glyphs not drawn", 3164, 3164), ("glyphs size", 3164, 3164), ("lines all", 309, 309), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-keeps-none", &[("glyphs extra", 3164, 3164), ("glyphs face", 3164, 3164), ("glyphs not drawn", 3164, 3164), ("glyphs size", 3164, 3164), ("lines all", 309, 309), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notes3-14", &[("glyphs extra", 1993, 1993), ("glyphs face", 1993, 1993), ("glyphs not drawn", 1993, 1993), ("glyphs size", 1993, 1993), ("lines all", 56, 56), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notes3-15", &[("glyphs extra", 1993, 1993), ("glyphs face", 1993, 1993), ("glyphs not drawn", 1993, 1993), ("glyphs size", 1993, 1993), ("lines all", 56, 56), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notes3-none", &[("glyphs extra", 1993, 1993), ("glyphs face", 1993, 1993), ("glyphs not drawn", 1993, 1993), ("glyphs size", 1993, 1993), ("lines all", 56, 56), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesover-14", &[("glyphs extra", 3110, 3110), ("glyphs face", 3110, 3110), ("glyphs not drawn", 3110, 3110), ("glyphs size", 3110, 3110), ("lines all", 111, 111), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesover-15", &[("glyphs extra", 3110, 3110), ("glyphs face", 3110, 3110), ("glyphs not drawn", 3110, 3110), ("glyphs size", 3110, 3110), ("lines all", 106, 106), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesover-none", &[("glyphs extra", 3110, 3110), ("glyphs face", 3110, 3110), ("glyphs not drawn", 3110, 3110), ("glyphs size", 3110, 3110), ("lines all", 111, 111), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesregion-14", &[("glyphs extra", 91, 425), ("glyphs face", 91, 91), ("glyphs not drawn", 91, 425), ("glyphs size", 83, 91), ("lines all", 5, 31), ("pages", 0, 1), ("warnings", 0, 1)]),
    ("cols-notesregion-15", &[("glyphs extra", 330, 425), ("glyphs face", 330, 330), ("glyphs not drawn", 330, 425), ("glyphs size", 330, 330), ("lines all", 18, 24), ("pages", 1, 1), ("warnings", 0, 1)]),
    ("cols-notesregion-none", &[("glyphs extra", 91, 425), ("glyphs face", 91, 91), ("glyphs not drawn", 91, 425), ("glyphs size", 83, 91), ("lines all", 5, 31), ("pages", 0, 1), ("warnings", 0, 1)]),
    ("cols-notesright-14", &[("glyphs extra", 1273, 1273), ("glyphs face", 1273, 1273), ("glyphs not drawn", 1273, 1273), ("glyphs size", 1273, 1273), ("lines all", 66, 66), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesright-15", &[("glyphs extra", 1273, 1273), ("glyphs face", 1273, 1273), ("glyphs not drawn", 1273, 1273), ("glyphs size", 1273, 1273), ("lines all", 56, 56), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesright-none", &[("glyphs extra", 1273, 1273), ("glyphs face", 1273, 1273), ("glyphs not drawn", 1273, 1273), ("glyphs size", 1273, 1273), ("lines all", 66, 66), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesunequal-14", &[("glyphs extra", 2172, 2172), ("glyphs face", 2172, 2172), ("glyphs not drawn", 2172, 2172), ("glyphs size", 2172, 2172), ("lines all", 58, 58), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesunequal-15", &[("glyphs extra", 2172, 2172), ("glyphs face", 2172, 2172), ("glyphs not drawn", 2172, 2172), ("glyphs size", 2172, 2172), ("lines all", 55, 55), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-notesunequal-none", &[("glyphs extra", 2172, 2172), ("glyphs face", 2172, 2172), ("glyphs not drawn", 2172, 2172), ("glyphs size", 2172, 2172), ("lines all", 58, 58), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-picture-14", &[("glyphs extra", 698, 698), ("glyphs face", 698, 698), ("glyphs not drawn", 698, 698), ("glyphs size", 698, 698), ("lines all", 45, 45), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-picture-15", &[("glyphs extra", 698, 698), ("glyphs face", 698, 698), ("glyphs not drawn", 698, 698), ("glyphs size", 698, 698), ("lines all", 45, 45), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-picture-none", &[("glyphs extra", 698, 698), ("glyphs face", 698, 698), ("glyphs not drawn", 698, 698), ("glyphs size", 698, 698), ("lines all", 45, 45), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-regions-14", &[("glyphs extra", 550, 550), ("glyphs face", 550, 550), ("glyphs not drawn", 550, 550), ("glyphs size", 550, 550), ("lines all", 71, 71), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-regions-15", &[("glyphs extra", 550, 550), ("glyphs face", 550, 550), ("glyphs not drawn", 550, 550), ("glyphs size", 550, 550), ("lines all", 71, 71), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-regions-none", &[("glyphs extra", 550, 550), ("glyphs face", 550, 550), ("glyphs not drawn", 550, 550), ("glyphs size", 550, 550), ("lines all", 71, 71), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-sep-14", &[("glyphs extra", 10969, 10969), ("glyphs face", 10969, 10969), ("glyphs not drawn", 10969, 10969), ("glyphs size", 10969, 10969), ("lines all", 229, 229), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-sep-15", &[("glyphs extra", 10969, 10969), ("glyphs face", 10969, 10969), ("glyphs not drawn", 10969, 10969), ("glyphs size", 10969, 10969), ("lines all", 229, 229), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-sep-none", &[("glyphs extra", 10969, 10969), ("glyphs face", 10969, 10969), ("glyphs not drawn", 10969, 10969), ("glyphs size", 10969, 10969), ("lines all", 229, 229), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-table-14", &[("glyphs extra", 735, 735), ("glyphs face", 735, 735), ("glyphs not drawn", 735, 735), ("glyphs size", 735, 735), ("lines all", 61, 61), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-table-15", &[("glyphs extra", 735, 735), ("glyphs face", 735, 735), ("glyphs not drawn", 735, 735), ("glyphs size", 735, 735), ("lines all", 61, 61), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-table-none", &[("glyphs extra", 735, 735), ("glyphs face", 735, 735), ("glyphs not drawn", 735, 735), ("glyphs size", 735, 735), ("lines all", 61, 61), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-unequal-14", &[("glyphs extra", 8144, 8144), ("glyphs face", 8144, 8144), ("glyphs not drawn", 8144, 8144), ("glyphs size", 8144, 8144), ("lines all", 300, 300), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-unequal-15", &[("glyphs extra", 8144, 8144), ("glyphs face", 8144, 8144), ("glyphs not drawn", 8144, 8144), ("glyphs size", 8144, 8144), ("lines all", 300, 300), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("cols-unequal-none", &[("glyphs extra", 8144, 8144), ("glyphs face", 8144, 8144), ("glyphs not drawn", 8144, 8144), ("glyphs size", 8144, 8144), ("lines all", 300, 300), ("pages", 1, 1), ("warnings", 1, 1)]),
];

#[test]
fn columns() {
    objects_family("columns-observations.json", "make_columns_probe", COLUMNS_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const BREAK_END_FLOORS: Floors = &[
    ("break-end-15", &[("glyphs extra", 1034, 1034), ("glyphs face", 1034, 1034), ("glyphs not drawn", 1034, 1034), ("glyphs size", 1034, 1034), ("lines all", 49, 49), ("warnings", 1, 1)]),
    ("break-end-none", &[("glyphs extra", 1034, 1034), ("glyphs face", 1034, 1034), ("glyphs not drawn", 1034, 1034), ("glyphs size", 1034, 1034), ("lines all", 49, 49), ("warnings", 1, 1)]),
];

#[test]
fn break_end() {
    objects_family("break-end-observations.json", "make_break_end_probe", BREAK_END_FLOORS);
}

/// `make_table_keep_probe`'s cases, in order: a `Case N` line starts case `N`'s markers.
const TABLE_KEEP_CASES: &[&str] = &[
    "plain",
    "all",
    "but last",
    "row 5",
    "rows 4-5",
    "row 4",
    "row 5 cell a",
    "row 5 cell b",
    "row 5 plain, two lines",
    "row 5 first",
    "row 5 last",
    "foot plain",
    "foot last row",
    "foot all",
    "foot but last",
    "long all",
    "long but last",
    "long header all",
    "before plain",
    "before all",
    "cantSplit plain",
    "cantSplit row 5",
    "header all",
    "nested inner",
    "nested outer",
    "in a cell plain",
    "in a cell all",
    "row 5 a first only",
    "row 5 b first only",
    "row 5 style",
    "row 6 splits",
    "row 5, row 6 splits",
    "row 5 wrapped, cantSplit",
    "row 5 wrapped plain",
    "last row, page break after",
    "all, page break after",
    "kept row 6 of 2",
    "plain row 6 of 2",
    "kept row 6 of 4",
    "plain row 6 of 4",
    "kept row 6 of 8",
    "plain row 6 of 8",
    "kept row 6 of 4, all",
    "kept last row",
    "kept rows of 4",
    "kept tall row",
    "kept tall row at the top",
    "kept nested row",
    "plain last row",
    "plain rows of 4",
    "plain tall row",
    "plain tall row at the top",
    "plain nested row",
    "header, kept row 6",
    "cantSplit tall row",
    "long kept rows of 4",
    "long kept rows of 4, header",
    "chain head par table",
    "chain head par table at the top",
    "chain head par table plain rows",
    "chain head table",
    "chain table par",
    "chain two tables",
    "chain paragraphs",
];

/// The markers in `text`, as `read_table_keep_probe.MARKER` finds them: `Case \d+|After
/// \d+|Lead|Broken|Head|Mid|Tail|T\d+[ab]\d+|[RC]\d+[ab]\d?|N\d+|H[ab]|C[ab]|K\d+|P\d+`,
/// leftmost first, the first alternative that matches.
fn markers(text: &str) -> Vec<String> {
    fn digits(s: &[char], i: usize) -> usize {
        s[i..].iter().take_while(|c| c.is_ascii_digit()).count()
    }
    fn ab(s: &[char], i: usize) -> bool {
        matches!(s.get(i), Some('a' | 'b'))
    }
    let s: Vec<char> = text.chars().collect();
    let starts = |i: usize, word: &str| s[i..].iter().take(word.len()).copied().eq(word.chars());
    let matched = |i: usize| -> usize {
        for word in ["Case ", "After "] {
            if starts(i, word) {
                let n = digits(&s, i + word.len());
                if n > 0 {
                    return word.len() + n;
                }
            }
        }
        for word in ["Lead", "Broken", "Head", "Mid", "Tail"] {
            if starts(i, word) {
                return word.len();
            }
        }
        let first = s[i];
        if first == 'T' {
            let n = digits(&s, i + 1);
            if n > 0 && ab(&s, i + 1 + n) {
                let m = digits(&s, i + 2 + n);
                if m > 0 {
                    return 2 + n + m;
                }
            }
        }
        if first == 'R' || first == 'C' {
            let n = digits(&s, i + 1);
            if n > 0 && ab(&s, i + 1 + n) {
                return 2 + n + usize::from(s.get(i + 2 + n).is_some_and(char::is_ascii_digit));
            }
        }
        if matches!(first, 'N' | 'K' | 'P') {
            let n = digits(&s, i + 1);
            if n > 0 {
                return 1 + n;
            }
        }
        if (first == 'H' || first == 'C') && ab(&s, i + 1) {
            return 2;
        }
        0
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let n = matched(i);
        if n > 0 {
            out.push(s[i..i + n].iter().collect());
            i += n;
        } else {
            i += 1;
        }
    }
    out
}

/// Per case, each marker's page less the case's anchor's (`read_table_keep_probe._cases`),
/// from the layout's lines in page order.
fn laid_cases(
    pages: &[std::sync::Arc<ooxml_word_layout::paint::PagePaint>],
) -> BTreeMap<String, BTreeMap<String, i64>> {
    let mut out: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    let mut current: Option<(String, i64)> = None;
    for (page, paint) in pages.iter().enumerate() {
        let page = page as i64;
        for line in paint.lines.iter().filter(|l| l.story.is_none()) {
            let text: String = line.spans.iter().flat_map(|s| s.chars.iter()).collect();
            for marker in markers(&text) {
                if let Some(number) = marker.strip_prefix("Case ") {
                    let name = number
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| TABLE_KEEP_CASES.get(n))
                        .map_or_else(|| marker.clone(), |name| (*name).to_owned());
                    out.insert(name.clone(), BTreeMap::new());
                    current = Some((name, page));
                } else if let Some((name, anchor)) = &current {
                    out.entry(name.clone())
                        .or_default()
                        .entry(marker)
                        .or_insert(page - anchor);
                }
            }
        }
    }
    out
}

/// Today's agreement.
#[rustfmt::skip]
const TABLE_KEEP_FLOORS: Floors = &[
    ("15", &[("cases", 63, 64)]),
    ("none", &[("cases", 63, 64)]),
];

#[test]
fn table_keep() {
    let recording = observations("table-keep-observations.json");
    let names = documents_of(&recording);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    check_family(&names, TABLE_KEEP_FLOORS, |name| {
        let laid = lay_out(name, probe("make_table_keep_probe", &format!("table-keep-{name}")));
        let ours = laid_cases(&laid.pages);
        let mut bad = 0;
        let mut total = 0;
        let mut problems = Vec::new();
        for (case, word) in recording["documents"][name].as_object().into_iter().flatten() {
            total += 1;
            let word: BTreeMap<String, i64> = word
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or_default()))
                .collect();
            if ours.get(case) != Some(&word) {
                bad += 1;
                problems.push(format!("{case}: Word {word:?}, layout {:?}", ours.get(case)));
            }
        }
        let mut scored = Scored::default();
        scored.cases.insert("cases".into(), (bad, total));
        scored.problems.insert("cases".into(), problems);
        scored
    });
}
