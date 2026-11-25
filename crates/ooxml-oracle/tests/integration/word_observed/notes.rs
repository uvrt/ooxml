//! Footnotes and endnotes against what Word drew:
//!
//! - `footnote_draw`, `endnotes`, `cell_footnotes`: every text object Word drew for
//!   `make_footnote_draw_probe` (separators, numbering formats, positions, notes carried
//!   over), `make_endnote_probe` and `make_cell_footnote_probe` (a note referenced in a table
//!   cell): every line within half a pixel, every glyph where Word drew one, and the pages
//!   (docx2svg's `test_footnote_draw.py`, `test_endnotes.py`, `test_cell_footnotes.py`);
//! - `footnotes`: the notes' room on the page (`make_footnote_probe`,
//!   `footnote-observations.json`; `test_paginate.py`): every anchor, candidate and note
//!   line Word drew, on the same page at the same baseline with the same text.

use super::lines::text_lines;
use super::{check_family, documents_of, lay_out, observations, probe, score_objects, Floors, Scored};

/// A recording of text objects, each document `make_<maker>_probe`'s, scored as one.
pub fn objects_family(recording: &str, maker: &str, floors: Floors) {
    let recording = observations(recording);
    let names = documents_of(&recording);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    check_family(&names, floors, |name| score_objects(&recording, maker, name, None));
}

/// A recording of lines `[page, baseline, text]` per document, the document
/// `<prefix><key>` of `make_<maker>_probe`.
pub fn text_family(recording: &str, maker: &str, prefix: &str, floors: Floors) {
    let recording = observations(recording);
    let names = documents_of(&recording);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    check_family(&names, floors, |name| {
        let laid = lay_out(name, probe(maker, &format!("{prefix}{name}")));
        let mut scored = Scored::default();
        text_lines(&mut scored, "lines all", &recording["documents"][name], &laid.pages);
        scored
    });
}

/// Today's agreement.
#[rustfmt::skip]
const FOOTNOTE_DRAW_FLOORS: Floors = &[
    ("fndraw-after-14", &[("glyphs extra", 546, 546), ("glyphs face", 546, 546), ("glyphs not drawn", 546, 546), ("glyphs size", 546, 546), ("lines all", 22, 22), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-after-15", &[("glyphs extra", 546, 546), ("glyphs face", 546, 546), ("glyphs not drawn", 546, 546), ("glyphs size", 546, 546), ("lines all", 22, 22), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-after-named", &[("glyphs extra", 546, 546), ("glyphs face", 546, 546), ("glyphs not drawn", 546, 546), ("glyphs size", 546, 546), ("lines all", 22, 22), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-after-none", &[("glyphs extra", 546, 546), ("glyphs face", 546, 546), ("glyphs not drawn", 546, 546), ("glyphs size", 546, 546), ("lines all", 22, 22), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-base-14", &[("glyphs extra", 1018, 1018), ("glyphs face", 1018, 1018), ("glyphs not drawn", 1018, 1018), ("glyphs size", 1018, 1018), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-base-15", &[("glyphs extra", 1018, 1018), ("glyphs face", 1018, 1018), ("glyphs not drawn", 1018, 1018), ("glyphs size", 1018, 1018), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-base-named", &[("glyphs extra", 1018, 1018), ("glyphs face", 1018, 1018), ("glyphs not drawn", 1018, 1018), ("glyphs size", 1018, 1018), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-base-none", &[("glyphs extra", 1018, 1018), ("glyphs face", 1018, 1018), ("glyphs not drawn", 1018, 1018), ("glyphs size", 1018, 1018), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-beneath-14", &[("glyphs extra", 1021, 1021), ("glyphs face", 1021, 1021), ("glyphs not drawn", 1021, 1021), ("glyphs size", 1021, 1021), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-beneath-15", &[("glyphs extra", 1021, 1021), ("glyphs face", 1021, 1021), ("glyphs not drawn", 1021, 1021), ("glyphs size", 1021, 1021), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-beneath-named", &[("glyphs extra", 1021, 1021), ("glyphs face", 1021, 1021), ("glyphs not drawn", 1021, 1021), ("glyphs size", 1021, 1021), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-chain-14", &[("glyphs extra", 432, 432), ("glyphs face", 432, 432), ("glyphs not drawn", 432, 432), ("glyphs size", 432, 432), ("lines all", 28, 28), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-chain-15", &[("glyphs extra", 432, 432), ("glyphs face", 432, 432), ("glyphs not drawn", 432, 432), ("glyphs size", 432, 432), ("lines all", 28, 28), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-chain-named", &[("glyphs extra", 432, 432), ("glyphs face", 432, 432), ("glyphs not drawn", 432, 432), ("glyphs size", 432, 432), ("lines all", 28, 28), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-chain-none", &[("glyphs extra", 432, 432), ("glyphs face", 432, 432), ("glyphs not drawn", 432, 432), ("glyphs size", 432, 432), ("lines all", 28, 28), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-docpage-14", &[("glyphs extra", 1079, 1079), ("glyphs face", 1079, 1079), ("glyphs not drawn", 1079, 1079), ("glyphs size", 1079, 1079), ("lines all", 64, 64), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-docpage-15", &[("glyphs extra", 1079, 1079), ("glyphs face", 1079, 1079), ("glyphs not drawn", 1079, 1079), ("glyphs size", 1079, 1079), ("lines all", 64, 64), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-docpage-named", &[("glyphs extra", 1079, 1079), ("glyphs face", 1079, 1079), ("glyphs not drawn", 1079, 1079), ("glyphs size", 1079, 1079), ("lines all", 64, 64), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-docpr-14", &[("glyphs extra", 120, 120), ("glyphs face", 120, 120), ("glyphs not drawn", 120, 120), ("glyphs size", 120, 120), ("lines all", 12, 12), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-docpr-15", &[("glyphs extra", 120, 120), ("glyphs face", 120, 120), ("glyphs not drawn", 120, 120), ("glyphs size", 120, 120), ("lines all", 12, 12), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-docpr-named", &[("glyphs extra", 120, 120), ("glyphs face", 120, 120), ("glyphs not drawn", 120, 120), ("glyphs size", 120, 120), ("lines all", 12, 12), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-eachpage-14", &[("glyphs extra", 1065, 1065), ("glyphs face", 1065, 1065), ("glyphs not drawn", 1065, 1065), ("glyphs size", 1065, 1065), ("lines all", 64, 64), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-eachpage-15", &[("glyphs extra", 1065, 1065), ("glyphs face", 1065, 1065), ("glyphs not drawn", 1065, 1065), ("glyphs size", 1065, 1065), ("lines all", 64, 64), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-eachpage-named", &[("glyphs extra", 1065, 1065), ("glyphs face", 1065, 1065), ("glyphs not drawn", 1065, 1065), ("glyphs size", 1065, 1065), ("lines all", 64, 64), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-eachpage-none", &[("glyphs extra", 1065, 1065), ("glyphs face", 1065, 1065), ("glyphs not drawn", 1065, 1065), ("glyphs size", 1065, 1065), ("lines all", 64, 64), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-format-14", &[("glyphs extra", 594, 594), ("glyphs face", 594, 594), ("glyphs not drawn", 594, 594), ("glyphs size", 594, 594), ("lines all", 36, 36), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-format-15", &[("glyphs extra", 594, 594), ("glyphs face", 594, 594), ("glyphs not drawn", 594, 594), ("glyphs size", 594, 594), ("lines all", 36, 36), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-format-named", &[("glyphs extra", 594, 594), ("glyphs face", 594, 594), ("glyphs not drawn", 594, 594), ("glyphs size", 594, 594), ("lines all", 36, 36), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-format-none", &[("glyphs extra", 594, 594), ("glyphs face", 594, 594), ("glyphs not drawn", 594, 594), ("glyphs size", 594, 594), ("lines all", 36, 36), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-normal-14", &[("glyphs extra", 439, 439), ("glyphs face", 439, 439), ("glyphs not drawn", 439, 439), ("glyphs size", 439, 439), ("lines all", 11, 11), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-normal-15", &[("glyphs extra", 439, 439), ("glyphs face", 439, 439), ("glyphs not drawn", 439, 439), ("glyphs size", 439, 439), ("lines all", 11, 11), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-normal-named", &[("glyphs extra", 439, 439), ("glyphs face", 439, 439), ("glyphs not drawn", 439, 439), ("glyphs size", 439, 439), ("lines all", 11, 11), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-normal-none", &[("glyphs extra", 439, 439), ("glyphs face", 439, 439), ("glyphs not drawn", 439, 439), ("glyphs size", 439, 439), ("lines all", 11, 11), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-overflow-14", &[("glyphs extra", 5105, 5105), ("glyphs face", 5105, 5105), ("glyphs not drawn", 5105, 5105), ("glyphs size", 5105, 5105), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-overflow-15", &[("glyphs extra", 5105, 5105), ("glyphs face", 5105, 5105), ("glyphs not drawn", 5105, 5105), ("glyphs size", 5105, 5105), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-overflow-named", &[("glyphs extra", 5105, 5105), ("glyphs face", 5105, 5105), ("glyphs not drawn", 5105, 5105), ("glyphs size", 5105, 5105), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-overflow-none", &[("glyphs extra", 5105, 5105), ("glyphs face", 5105, 5105), ("glyphs not drawn", 5105, 5105), ("glyphs size", 5105, 5105), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-sectbeneath-14", &[("glyphs extra", 1025, 1025), ("glyphs face", 1025, 1025), ("glyphs not drawn", 1025, 1025), ("glyphs size", 1025, 1025), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-sectbeneath-15", &[("glyphs extra", 1025, 1025), ("glyphs face", 1025, 1025), ("glyphs not drawn", 1025, 1025), ("glyphs size", 1025, 1025), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-sectbeneath-named", &[("glyphs extra", 1025, 1025), ("glyphs face", 1025, 1025), ("glyphs not drawn", 1025, 1025), ("glyphs size", 1025, 1025), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-sectbeneath-none", &[("glyphs extra", 1025, 1025), ("glyphs face", 1025, 1025), ("glyphs not drawn", 1025, 1025), ("glyphs size", 1025, 1025), ("lines all", 32, 32), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-separators-14", &[("glyphs extra", 5107, 5107), ("glyphs face", 5107, 5107), ("glyphs not drawn", 5107, 5107), ("glyphs size", 5107, 5107), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-separators-15", &[("glyphs extra", 5107, 5107), ("glyphs face", 5107, 5107), ("glyphs not drawn", 5107, 5107), ("glyphs size", 5107, 5107), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-separators-named", &[("glyphs extra", 5107, 5107), ("glyphs face", 5107, 5107), ("glyphs not drawn", 5107, 5107), ("glyphs size", 5107, 5107), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-separators-none", &[("glyphs extra", 5107, 5107), ("glyphs face", 5107, 5107), ("glyphs not drawn", 5107, 5107), ("glyphs size", 5107, 5107), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-unnamed-14", &[("glyphs extra", 5104, 5104), ("glyphs face", 5104, 5104), ("glyphs not drawn", 5104, 5104), ("glyphs size", 5104, 5104), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-unnamed-15", &[("glyphs extra", 5104, 5104), ("glyphs face", 5104, 5104), ("glyphs not drawn", 5104, 5104), ("glyphs size", 5104, 5104), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("fndraw-unnamed-named", &[("glyphs extra", 5104, 5104), ("glyphs face", 5104, 5104), ("glyphs not drawn", 5104, 5104), ("glyphs size", 5104, 5104), ("lines all", 100, 100), ("pages", 1, 1), ("warnings", 1, 1)]),
];

#[test]
fn footnote_draw() {
    objects_family(
        "footnote-draw-observations.json",
        "make_footnote_draw_probe",
        FOOTNOTE_DRAW_FLOORS,
    );
}

/// Today's agreement.
#[rustfmt::skip]
const ENDNOTE_FLOORS: Floors = &[
    ("endnote-base-14", &[("glyphs extra", 1464, 1464), ("glyphs face", 1464, 1464), ("glyphs not drawn", 1464, 1464), ("glyphs size", 1464, 1464), ("lines all", 47, 47), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-base-15", &[("glyphs extra", 1464, 1464), ("glyphs face", 1464, 1464), ("glyphs not drawn", 1464, 1464), ("glyphs size", 1464, 1464), ("lines all", 47, 47), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-base-none", &[("glyphs extra", 1464, 1464), ("glyphs face", 1464, 1464), ("glyphs not drawn", 1464, 1464), ("glyphs size", 1464, 1464), ("lines all", 47, 47), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-docpr-14", &[("glyphs extra", 124, 124), ("glyphs face", 124, 124), ("glyphs not drawn", 124, 124), ("glyphs size", 124, 124), ("lines all", 12, 12), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-docpr-15", &[("glyphs extra", 124, 124), ("glyphs face", 124, 124), ("glyphs not drawn", 124, 124), ("glyphs size", 124, 124), ("lines all", 12, 12), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-format-14", &[("glyphs extra", 322, 322), ("glyphs face", 322, 322), ("glyphs not drawn", 322, 322), ("glyphs size", 322, 322), ("lines all", 21, 21), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-format-15", &[("glyphs extra", 322, 322), ("glyphs face", 322, 322), ("glyphs not drawn", 322, 322), ("glyphs size", 322, 322), ("lines all", 21, 21), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-format-none", &[("glyphs extra", 322, 322), ("glyphs face", 322, 322), ("glyphs not drawn", 322, 322), ("glyphs size", 322, 322), ("lines all", 21, 21), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-normal-14", &[("glyphs extra", 440, 440), ("glyphs face", 440, 440), ("glyphs not drawn", 440, 440), ("glyphs size", 440, 440), ("lines all", 11, 11), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-normal-15", &[("glyphs extra", 440, 440), ("glyphs face", 440, 440), ("glyphs not drawn", 440, 440), ("glyphs size", 440, 440), ("lines all", 11, 11), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-normal-none", &[("glyphs extra", 440, 440), ("glyphs face", 440, 440), ("glyphs not drawn", 440, 440), ("glyphs size", 440, 440), ("lines all", 11, 11), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-overflow-14", &[("glyphs extra", 3310, 3310), ("glyphs face", 3310, 3310), ("glyphs not drawn", 3310, 3310), ("glyphs size", 3310, 3310), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-overflow-15", &[("glyphs extra", 3310, 3310), ("glyphs face", 3310, 3310), ("glyphs not drawn", 3310, 3310), ("glyphs size", 3310, 3310), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-overflow-none", &[("glyphs extra", 3310, 3310), ("glyphs face", 3310, 3310), ("glyphs not drawn", 3310, 3310), ("glyphs size", 3310, 3310), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-sectend-14", &[("glyphs extra", 815, 815), ("glyphs face", 815, 815), ("glyphs not drawn", 815, 815), ("glyphs size", 815, 815), ("lines all", 24, 24), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-sectend-15", &[("glyphs extra", 815, 815), ("glyphs face", 815, 815), ("glyphs not drawn", 815, 815), ("glyphs size", 815, 815), ("lines all", 24, 24), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-separators-14", &[("glyphs extra", 3312, 3312), ("glyphs face", 3312, 3312), ("glyphs not drawn", 3312, 3312), ("glyphs size", 3312, 3312), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-separators-15", &[("glyphs extra", 3312, 3312), ("glyphs face", 3312, 3312), ("glyphs not drawn", 3312, 3312), ("glyphs size", 3312, 3312), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-separators-none", &[("glyphs extra", 3312, 3312), ("glyphs face", 3312, 3312), ("glyphs not drawn", 3312, 3312), ("glyphs size", 3312, 3312), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-unnamed-14", &[("glyphs extra", 3309, 3309), ("glyphs face", 3309, 3309), ("glyphs not drawn", 3309, 3309), ("glyphs size", 3309, 3309), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
    ("endnote-unnamed-15", &[("glyphs extra", 3309, 3309), ("glyphs face", 3309, 3309), ("glyphs not drawn", 3309, 3309), ("glyphs size", 3309, 3309), ("lines all", 72, 72), ("pages", 1, 1), ("warnings", 1, 1)]),
];

#[test]
fn endnotes() {
    objects_family("endnote-observations.json", "make_endnote_probe", ENDNOTE_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const CELL_FOOTNOTE_FLOORS: Floors = &[
    ("cellfn-15", &[("glyphs extra", 22363, 22363), ("glyphs face", 22363, 22363), ("glyphs not drawn", 22363, 22363), ("glyphs size", 22363, 22363), ("lines all", 706, 706), ("warnings", 1, 1)]),
    ("cellfn-none", &[("glyphs extra", 22361, 22363), ("glyphs face", 22361, 22361), ("glyphs not drawn", 22361, 22363), ("glyphs size", 22361, 22361), ("lines all", 700, 706), ("warnings", 1, 1)]),
];

#[test]
fn cell_footnotes() {
    objects_family(
        "cell-footnote-observations.json",
        "make_cell_footnote_probe",
        CELL_FOOTNOTE_FLOORS,
    );
}

/// Today's agreement. Mode 15 every line; below it a case where Word leaves a line's second
/// note to the next page (docx2svg's eight) moves every later case a page on, so each line
/// after it is on another page than Word's.
#[rustfmt::skip]
const FOOTNOTE_FLOORS: Floors = &[
    ("12", &[("lines all", 239, 2013)]),
    ("14", &[("lines all", 239, 2013)]),
    ("15", &[("lines all", 2013, 2013)]),
    ("15-sep20", &[("lines all", 2013, 2013)]),
    ("none", &[("lines all", 239, 2013)]),
];

#[test]
fn footnotes() {
    text_family(
        "footnote-observations.json",
        "make_footnote_probe",
        "footnote-",
        FOOTNOTE_FLOORS,
    );
}
