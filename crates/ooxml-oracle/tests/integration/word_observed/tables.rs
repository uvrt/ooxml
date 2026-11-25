//! Tables against what Word drew: every line of Word's within half a pixel and every glyph
//! where Word drew one (the line score `wrap_side` keeps), per case family where the
//! probe's reader counted them so.
//!
//! - `tables`: the table probes (`make_table_{geometry,content,style_size,row,merge,border,
//!   pages}_probe`; `table-observations.json`; docx2svg's `test_tables.py`);
//! - `float_table`, `nested_table`, `pct_table`, `autofit_over`, `autofit_width`: floating,
//!   nested, percent-wide and autofit tables (`make_<family>_probe`, a page's family the
//!   case whose `Case N` line Word drew on it);
//! - `grid_mismatch`: rows whose cells do not fill the grid, tracked or not;
//! - `table_foot`: rows at a page's foot, a cell merged down split across pages
//!   (`test_table_foot.py`).
//!
//! The rule pixels (`read_render.rect_score`) are not scored here: the lines are.

use super::{check_family, observations, score_objects, Floors};

/// The table probes' kinds: `table-<kind>-<setting>` is `make_table_<kind>_probe`'s.
const TABLE_KINDS: &[&str] = &["geometry", "content", "style-size", "row", "merge", "border", "pages"];

/// The documents of `table-observations.json`.
const TABLES: &[&str] = &[
    "table-border-12",
    "table-border-14",
    "table-border-15",
    "table-border-none",
    "table-content-12",
    "table-content-14",
    "table-content-15",
    "table-content-none",
    "table-geometry-12",
    "table-geometry-14",
    "table-geometry-15",
    "table-geometry-none",
    "table-merge-12",
    "table-merge-14",
    "table-merge-15",
    "table-merge-none",
    "table-pages-12",
    "table-pages-14",
    "table-pages-15",
    "table-pages-none",
    "table-row-12",
    "table-row-14",
    "table-row-15",
    "table-row-none",
    "table-style-size-12",
    "table-style-size-14",
    "table-style-size-14-centred",
    "table-style-size-14-override",
    "table-style-size-15",
    "table-style-size-15-centred",
    "table-style-size-15-override",
    "table-style-size-none",
    "table-style-size-sizes-16-24",
    "table-style-size-sizes-20-22",
    "table-style-size-sizes-20-24",
    "table-style-size-sizes-21-24",
    "table-style-size-sizes-22-22",
    "table-style-size-sizes-22-23",
    "table-style-size-sizes-22-25",
    "table-style-size-sizes-24-20",
    "table-style-size-sizes-24-22",
    "table-style-size-sizes-24-28",
    "table-style-size-sizes-26-24",
];

fn table_maker(name: &str) -> String {
    let kind = TABLE_KINDS
        .iter()
        .find(|kind| name.starts_with(&format!("table-{kind}-")))
        .unwrap_or_else(|| panic!("{name}: no table probe"));
    format!("make_table_{}_probe", kind.replace('-', "_"))
}

/// Today's agreement.
#[rustfmt::skip]
const TABLES_FLOORS: Floors = &[
    ("table-border-12", &[("glyphs extra", 2453, 2453), ("glyphs face", 2453, 2453), ("glyphs not drawn", 2453, 2453), ("glyphs size", 2453, 2453), ("lines all", 222, 222), ("warnings", 1, 1)]),
    ("table-border-14", &[("glyphs extra", 2453, 2453), ("glyphs face", 2453, 2453), ("glyphs not drawn", 2453, 2453), ("glyphs size", 2453, 2453), ("lines all", 222, 222), ("warnings", 1, 1)]),
    ("table-border-15", &[("glyphs extra", 2453, 2453), ("glyphs face", 2453, 2453), ("glyphs not drawn", 2453, 2453), ("glyphs size", 2453, 2453), ("lines all", 222, 222), ("warnings", 1, 1)]),
    ("table-border-none", &[("glyphs extra", 2453, 2453), ("glyphs face", 2453, 2453), ("glyphs not drawn", 2453, 2453), ("glyphs size", 2453, 2453), ("lines all", 222, 222), ("warnings", 1, 1)]),
    ("table-content-12", &[("glyphs extra", 3558, 3558), ("glyphs face", 3558, 3558), ("glyphs not drawn", 3558, 3558), ("glyphs size", 3558, 3558), ("lines all", 218, 218), ("warnings", 1, 1)]),
    ("table-content-14", &[("glyphs extra", 3558, 3558), ("glyphs face", 3558, 3558), ("glyphs not drawn", 3558, 3558), ("glyphs size", 3558, 3558), ("lines all", 218, 218), ("warnings", 1, 1)]),
    ("table-content-15", &[("glyphs extra", 3558, 3558), ("glyphs face", 3558, 3558), ("glyphs not drawn", 3558, 3558), ("glyphs size", 3558, 3558), ("lines all", 218, 218), ("warnings", 1, 1)]),
    ("table-content-none", &[("glyphs extra", 3558, 3558), ("glyphs face", 3558, 3558), ("glyphs not drawn", 3558, 3558), ("glyphs size", 3558, 3558), ("lines all", 218, 218), ("warnings", 1, 1)]),
    ("table-geometry-12", &[("glyphs extra", 1583, 1583), ("glyphs face", 1583, 1583), ("glyphs not drawn", 1583, 1583), ("glyphs size", 1583, 1583), ("lines all", 136, 136), ("warnings", 1, 1)]),
    ("table-geometry-14", &[("glyphs extra", 1583, 1583), ("glyphs face", 1583, 1583), ("glyphs not drawn", 1583, 1583), ("glyphs size", 1583, 1583), ("lines all", 136, 136), ("warnings", 1, 1)]),
    ("table-geometry-15", &[("glyphs extra", 1583, 1583), ("glyphs face", 1583, 1583), ("glyphs not drawn", 1583, 1583), ("glyphs size", 1583, 1583), ("lines all", 136, 136), ("warnings", 1, 1)]),
    ("table-geometry-none", &[("glyphs extra", 1583, 1583), ("glyphs face", 1583, 1583), ("glyphs not drawn", 1583, 1583), ("glyphs size", 1583, 1583), ("lines all", 136, 136), ("warnings", 1, 1)]),
    ("table-merge-12", &[("glyphs extra", 674, 674), ("glyphs face", 674, 674), ("glyphs not drawn", 674, 674), ("glyphs size", 674, 674), ("lines all", 103, 103), ("warnings", 1, 1)]),
    ("table-merge-14", &[("glyphs extra", 674, 674), ("glyphs face", 674, 674), ("glyphs not drawn", 674, 674), ("glyphs size", 674, 674), ("lines all", 103, 103), ("warnings", 1, 1)]),
    ("table-merge-15", &[("glyphs extra", 674, 674), ("glyphs face", 674, 674), ("glyphs not drawn", 674, 674), ("glyphs size", 674, 674), ("lines all", 103, 103), ("warnings", 1, 1)]),
    ("table-merge-none", &[("glyphs extra", 674, 674), ("glyphs face", 674, 674), ("glyphs not drawn", 674, 674), ("glyphs size", 674, 674), ("lines all", 103, 103), ("warnings", 1, 1)]),
    ("table-pages-12", &[("glyphs extra", 6782, 6782), ("glyphs face", 6782, 6782), ("glyphs not drawn", 6782, 6782), ("glyphs size", 6782, 6782), ("lines all", 1456, 1456), ("warnings", 1, 1)]),
    ("table-pages-14", &[("glyphs extra", 6782, 6782), ("glyphs face", 6782, 6782), ("glyphs not drawn", 6782, 6782), ("glyphs size", 6782, 6782), ("lines all", 1456, 1456), ("warnings", 1, 1)]),
    ("table-pages-15", &[("glyphs extra", 6790, 6790), ("glyphs face", 6790, 6790), ("glyphs not drawn", 6790, 6790), ("glyphs size", 6790, 6790), ("lines all", 1458, 1458), ("warnings", 1, 1)]),
    ("table-pages-none", &[("glyphs extra", 6782, 6782), ("glyphs face", 6782, 6782), ("glyphs not drawn", 6782, 6782), ("glyphs size", 6782, 6782), ("lines all", 1456, 1456), ("warnings", 1, 1)]),
    ("table-row-12", &[("glyphs extra", 2116, 2116), ("glyphs face", 2116, 2116), ("glyphs not drawn", 2116, 2116), ("glyphs size", 2116, 2116), ("lines all", 270, 270), ("warnings", 1, 1)]),
    ("table-row-14", &[("glyphs extra", 2116, 2116), ("glyphs face", 2116, 2116), ("glyphs not drawn", 2116, 2116), ("glyphs size", 2116, 2116), ("lines all", 270, 270), ("warnings", 1, 1)]),
    ("table-row-15", &[("glyphs extra", 2116, 2116), ("glyphs face", 2116, 2116), ("glyphs not drawn", 2116, 2116), ("glyphs size", 2116, 2116), ("lines all", 270, 270), ("warnings", 1, 1)]),
    ("table-row-none", &[("glyphs extra", 2116, 2116), ("glyphs face", 2116, 2116), ("glyphs not drawn", 2116, 2116), ("glyphs size", 2116, 2116), ("lines all", 270, 270), ("warnings", 1, 1)]),
    ("table-style-size-12", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-14", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-14-centred", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-14-override", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-15", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-15-centred", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-15-override", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-none", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-16-24", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-20-22", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-20-24", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-21-24", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-22-22", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-22-23", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-22-25", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-24-20", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-24-22", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-24-28", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
    ("table-style-size-sizes-26-24", &[("glyphs extra", 498, 498), ("glyphs face", 498, 498), ("glyphs not drawn", 498, 498), ("glyphs size", 498, 498), ("lines all", 37, 37), ("warnings", 1, 1)]),
];

#[test]
fn tables() {
    let recording = observations("table-observations.json");
    check_family(TABLES, TABLES_FLOORS, |name| {
        score_objects(&recording, &table_maker(name), name, None)
    });
}

/// A family of `make_<family>_probe` in three settings, its pages counted per case family.
fn settings_family(family: &str, floors: Floors) {
    let recording = observations(&format!("{family}-observations.json"));
    let maker = format!("make_{}_probe", family.replace('-', "_"));
    let names: Vec<String> = ["none", "14", "15"].iter().map(|s| format!("{family}-{s}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    check_family(&names, floors, |name| {
        score_objects(&recording, &maker, name, Some(family))
    });
}

/// Today's agreement.
#[rustfmt::skip]
const FLOAT_TABLE_FLOORS: Floors = &[
    ("float-table-none", &[("glyphs extra", 43780, 43780), ("glyphs face", 43780, 43780), ("glyphs not drawn", 43780, 43780), ("glyphs size", 43780, 43780), ("lines dist", 102, 102), ("lines edge", 309, 309), ("lines flow", 198, 198), ("lines h", 242, 242), ("lines no case", 1, 1), ("lines room", 101, 104), ("lines v", 273, 274), ("warnings", 1, 1)]),
    ("float-table-14", &[("glyphs extra", 43780, 43780), ("glyphs face", 43780, 43780), ("glyphs not drawn", 43780, 43780), ("glyphs size", 43780, 43780), ("lines dist", 102, 102), ("lines edge", 309, 309), ("lines flow", 198, 198), ("lines h", 242, 242), ("lines no case", 1, 1), ("lines room", 101, 104), ("lines v", 273, 274), ("warnings", 1, 1)]),
    ("float-table-15", &[("glyphs extra", 43363, 43780), ("glyphs face", 43363, 43363), ("glyphs not drawn", 43363, 43780), ("glyphs size", 43363, 43363), ("lines dist", 104, 104), ("lines edge", 304, 310), ("lines flow", 198, 198), ("lines h", 233, 242), ("lines no case", 1, 11), ("lines room", 102, 102), ("lines v", 257, 264), ("warnings", 0, 1)]),
];

#[test]
fn float_table() {
    settings_family("float-table", FLOAT_TABLE_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const NESTED_TABLE_FLOORS: Floors = &[
    ("nested-table-none", &[("glyphs extra", 1825, 1825), ("glyphs face", 1825, 1825), ("glyphs not drawn", 1825, 1825), ("glyphs size", 1825, 1825), ("lines place", 65, 65), ("lines row", 44, 44), ("lines sample", 30, 34), ("lines space", 29, 29), ("lines width", 10, 12), ("warnings", 1, 1)]),
    ("nested-table-14", &[("glyphs extra", 1825, 1825), ("glyphs face", 1825, 1825), ("glyphs not drawn", 1825, 1825), ("glyphs size", 1825, 1825), ("lines place", 65, 65), ("lines row", 44, 44), ("lines sample", 30, 34), ("lines space", 29, 29), ("lines width", 10, 12), ("warnings", 1, 1)]),
    ("nested-table-15", &[("glyphs extra", 1825, 1825), ("glyphs face", 1825, 1825), ("glyphs not drawn", 1825, 1825), ("glyphs size", 1825, 1825), ("lines place", 63, 65), ("lines row", 44, 44), ("lines sample", 34, 34), ("lines space", 29, 29), ("lines width", 12, 12), ("warnings", 1, 1)]),
];

#[test]
fn nested_table() {
    settings_family("nested-table", NESTED_TABLE_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const PCT_TABLE_FLOORS: Floors = &[
    ("pct-table-none", &[("glyphs extra", 3200, 3230), ("glyphs face", 3200, 3200), ("glyphs not drawn", 3200, 3230), ("glyphs size", 3200, 3200), ("lines content", 56, 56), ("lines dxa", 24, 24), ("lines grid", 22, 24), ("lines margin", 12, 12), ("lines not modelled", 9, 24), ("lines place", 12, 12), ("lines share", 22, 24), ("lines width", 54, 56), ("warnings", 0, 1)]),
    ("pct-table-14", &[("glyphs extra", 3200, 3230), ("glyphs face", 3200, 3200), ("glyphs not drawn", 3200, 3230), ("glyphs size", 3200, 3200), ("lines content", 56, 56), ("lines dxa", 24, 24), ("lines grid", 22, 24), ("lines margin", 12, 12), ("lines not modelled", 9, 24), ("lines place", 12, 12), ("lines share", 22, 24), ("lines width", 54, 56), ("warnings", 0, 1)]),
    ("pct-table-15", &[("glyphs extra", 3177, 3230), ("glyphs face", 3177, 3177), ("glyphs not drawn", 3177, 3230), ("glyphs size", 3177, 3177), ("lines content", 56, 56), ("lines dxa", 24, 24), ("lines grid", 24, 24), ("lines margin", 8, 12), ("lines not modelled", 11, 24), ("lines place", 12, 12), ("lines share", 24, 24), ("lines width", 52, 56), ("warnings", 0, 1)]),
];

#[test]
fn pct_table() {
    settings_family("pct-table", PCT_TABLE_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const AUTOFIT_OVER_FLOORS: Floors = &[
    ("autofit-over-none", &[("glyphs extra", 1678, 1678), ("glyphs face", 1678, 1678), ("glyphs not drawn", 1678, 1678), ("glyphs size", 1678, 1678), ("lines over", 59, 62), ("warnings", 0, 1)]),
    ("autofit-over-14", &[("glyphs extra", 1678, 1678), ("glyphs face", 1678, 1678), ("glyphs not drawn", 1678, 1678), ("glyphs size", 1678, 1678), ("lines over", 59, 62), ("warnings", 0, 1)]),
    ("autofit-over-15", &[("glyphs extra", 1678, 1678), ("glyphs face", 1678, 1678), ("glyphs not drawn", 1678, 1678), ("glyphs size", 1678, 1678), ("lines over", 63, 63), ("warnings", 1, 1)]),
];

#[test]
fn autofit_over() {
    settings_family("autofit-over", AUTOFIT_OVER_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const AUTOFIT_WIDTH_FLOORS: Floors = &[
    ("autofit-width-none", &[("glyphs extra", 4036, 4113), ("glyphs face", 4036, 4036), ("glyphs not drawn", 4036, 4113), ("glyphs size", 4036, 4036), ("lines auto", 29, 29), ("lines indent", 34, 34), ("lines share", 52, 52), ("lines span", 9, 38), ("lines tblW", 24, 24), ("lines widen", 21, 21), ("warnings", 0, 1)]),
    ("autofit-width-14", &[("glyphs extra", 4036, 4113), ("glyphs face", 4036, 4036), ("glyphs not drawn", 4036, 4113), ("glyphs size", 4036, 4036), ("lines auto", 29, 29), ("lines indent", 34, 34), ("lines share", 52, 52), ("lines span", 9, 38), ("lines tblW", 24, 24), ("lines widen", 21, 21), ("warnings", 0, 1)]),
    ("autofit-width-15", &[("glyphs extra", 4036, 4113), ("glyphs face", 4036, 4036), ("glyphs not drawn", 4036, 4113), ("glyphs size", 4036, 4036), ("lines auto", 29, 29), ("lines indent", 30, 30), ("lines share", 52, 52), ("lines span", 9, 38), ("lines tblW", 24, 24), ("lines widen", 21, 21), ("warnings", 0, 1)]),
];

#[test]
fn autofit_width() {
    settings_family("autofit-width", AUTOFIT_WIDTH_FLOORS);
}

/// Today's agreement.
#[rustfmt::skip]
const GRID_MISMATCH_FLOORS: Floors = &[
    ("grid-mismatch-none", &[("glyphs extra", 1731, 1731), ("glyphs face", 1731, 1731), ("glyphs not drawn", 1731, 1731), ("glyphs size", 1731, 1731), ("lines all", 161, 161), ("warnings", 1, 1)]),
    ("grid-mismatch-15", &[("glyphs extra", 1731, 1731), ("glyphs face", 1731, 1731), ("glyphs not drawn", 1731, 1731), ("glyphs size", 1731, 1731), ("lines all", 161, 161), ("warnings", 1, 1)]),
    ("grid-mismatch-tracked-none", &[("glyphs extra", 150, 150), ("glyphs face", 150, 150), ("glyphs not drawn", 150, 150), ("glyphs size", 150, 150), ("lines all", 16, 16), ("warnings", 1, 1)]),
    ("grid-mismatch-tracked-15", &[("glyphs extra", 150, 150), ("glyphs face", 150, 150), ("glyphs not drawn", 150, 150), ("glyphs size", 150, 150), ("lines all", 16, 16), ("warnings", 1, 1)]),
];

#[test]
fn grid_mismatch() {
    let recording = observations("grid-mismatch-observations.json");
    let names = [
        "grid-mismatch-none",
        "grid-mismatch-15",
        "grid-mismatch-tracked-none",
        "grid-mismatch-tracked-15",
    ];
    check_family(&names, GRID_MISMATCH_FLOORS, |name| {
        score_objects(&recording, "make_grid_mismatch_probe", name, None)
    });
}

/// Today's agreement.
#[rustfmt::skip]
const TABLE_FOOT_FLOORS: Floors = &[
    ("foot-none", &[("glyphs extra", 7531, 7531), ("glyphs face", 7531, 7531), ("glyphs not drawn", 7531, 7531), ("glyphs size", 7531, 7531), ("lines all", 2099, 2099), ("warnings", 1, 1)]),
    ("foot-15", &[("glyphs extra", 7411, 7411), ("glyphs face", 7411, 7411), ("glyphs not drawn", 7411, 7411), ("glyphs size", 7411, 7411), ("lines all", 2093, 2093), ("warnings", 1, 1)]),
];

#[test]
fn table_foot() {
    let recording = observations("table-foot-observations.json");
    check_family(&["foot-none", "foot-15"], TABLE_FOOT_FLOORS, |name| {
        score_objects(&recording, "make_table_foot_probe", name, None)
    });
}
