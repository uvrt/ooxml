//! Text beside floating drawings (`make_wrap_side_probe`, `wrap-side-observations.json`;
//! docx2svg's `test_wrap_side.py`): every line of Word's within half a pixel, counted per
//! case family, and every glyph drawn where Word drew one.
//!
//! A case is a page: `wrap-side-*` sweeps square, tight and through wrapping (`square`),
//! the 360-twip least segment (`min`), the drawing's vertical reach (`vertical`), wrap
//! polygons (`polygon`) and several drawings, tabs and indents (`more`); `wrap-edge-*`
//! sweeps the drawing's edge by single twips (`right`, `left`) and onto half pixels
//! (`tie`), with the left margin 1,442 and 1,440 twips.

use super::glyphs::{compare, laid_glyphs, line_score, word_glyphs};
use super::{check_known, corpus, lay_out, observations, Known, Scored};

/// The case families of `wrap-side-*`'s pages, in order: `(family, pages)`.
const SIDE: &[(&str, usize)] = &[
    ("square", 55),
    ("min", 22),
    ("vertical", 30),
    ("polygon", 17),
    ("more", 22),
];
/// The case families of `wrap-edge-*`'s pages.
const EDGE: &[(&str, usize)] = &[("right", 48), ("left", 30), ("tie", 12)];

/// An edge on exactly half a pixel -- the text column's in mode 15, the page's below it --
/// goes to the pixel either way, by a rule not found.
const TIE: &str = "an edge on exactly half a pixel goes to the pixel either way, by a rule not found";

/// Lines (or glyph checks) of Word's the layout disagrees with, per document, and why
/// (docx2svg's `test_wrap_side.KNOWN`: the same counts).
const KNOWN: Known = &[
    (
        "wrap-side-15",
        &[(
            "lines polygon",
            26,
            "a polygon past its extent, with its distance (tight and through, 13 lines each): a half-pixel tie",
        )],
    ),
    ("wrap-edge-15-1442", &[("lines tie", 20, TIE)]),
    ("wrap-edge-none-1442", &[("lines tie", 15, TIE)]),
    ("wrap-edge-none-1440", &[("lines tie", 15, TIE)]),
    (
        "wrap-side-none",
        &[
            ("lines min", 5, "a half-pixel tie, as in mode 15"),
            (
                "lines more",
                133,
                "not modelled below mode 15: a line no segment takes text at goes further down than the drawing's \
                 foot; a tab's segment on a half-pixel tie; a hanging first line beside a drawing inside the indent",
            ),
            (
                "lines square",
                13,
                "not modelled below mode 15: wrapText left with no room left of the drawing puts the text right of \
                 it, and beside a 400-twip gap breaks words into it",
            ),
        ],
    ),
];

fn family_of(cases: &'static [(&'static str, usize)], page: usize) -> &'static str {
    let mut start = 0;
    for (family, pages) in cases {
        if page < start + pages {
            return family;
        }
        start += pages;
    }
    "past the last case"
}

fn score(name: &str) -> Scored {
    let recorded = &observations("wrap-side-observations.json")["documents"][name];
    let laid = lay_out(name, corpus("make_wrap_side_probe", name));
    let cases = if name.starts_with("wrap-side") { SIDE } else { EDGE };
    let model = laid_glyphs(&laid.pages);
    let word = word_glyphs(&recorded["objects"]);
    let lines = line_score(&model, &word);
    let mut scored = Scored::default();
    scored.lines(&lines, |page| family_of(cases, page));
    scored.glyphs(&compare(&model, &word));
    scored.warnings(&laid.warnings, &recorded["warnings"]);
    scored
}

fn check(name: &str) {
    let scored = score(name);
    if let Err(problems) = check_known(name, &scored, KNOWN) {
        panic!("{}\n{}", problems.join("\n"), scored.sample(40));
    }
}

#[test]
fn wrap_side_none() {
    check("wrap-side-none");
}

#[test]
fn wrap_side_15() {
    check("wrap-side-15");
}

#[test]
fn wrap_edge_none_1442() {
    check("wrap-edge-none-1442");
}

#[test]
fn wrap_edge_none_1440() {
    check("wrap-edge-none-1440");
}

#[test]
fn wrap_edge_15_1442() {
    check("wrap-edge-15-1442");
}

#[test]
fn wrap_edge_15_1440() {
    check("wrap-edge-15-1440");
}
