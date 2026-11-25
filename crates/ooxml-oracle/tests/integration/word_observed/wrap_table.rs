//! Tables beside floating drawings, and drawings anchored in table cells
//! (`make_wrap_table_probe`, `make_cell_anchor_probe`, `make_cell_valign_probe`;
//! `wrap-table-observations.json`; docx2svg's `test_wrap_table.py`), and a text box beside
//! several tables (`make_wrap_tables_probe`, `wrap-tables-observations.json`;
//! `test_wrap_tables.py`): every line of Word's within half a pixel, per case family where
//! the reader counted them so, and every glyph where Word drew one.

use super::{check_family, observations, score_objects, Floors};

/// The `wrap-table` documents and their makers.
const WRAP_TABLE: &[(&str, &str)] = &[
    ("wrap-table-none", "make_wrap_table_probe"),
    ("wrap-table-14", "make_wrap_table_probe"),
    ("wrap-table-15", "make_wrap_table_probe"),
    ("cell-anchor-none", "make_cell_anchor_probe"),
    ("cell-anchor-14", "make_cell_anchor_probe"),
    ("cell-anchor-15", "make_cell_anchor_probe"),
    ("cell-valign-14", "make_cell_valign_probe"),
    ("cell-valign-15", "make_cell_valign_probe"),
];

/// Today's agreement (docx2svg's known differences were: `jc` 3 lines, a centring tie; the
/// cell-anchor `wrap` pages below mode 15, a drawing against the page in a cell, 147; mode
/// 15's centred cell, 8).
#[rustfmt::skip]
const WRAP_TABLE_FLOORS: Floors = &[
    ("wrap-table-none", &[("glyphs extra", 11029, 11029), ("glyphs face", 11029, 11029), ("glyphs not drawn", 11029, 11029), ("glyphs size", 11029, 11029), ("lines fit", 420, 420), ("lines floating", 18, 18), ("lines jc", 105, 108), ("lines kind", 33, 33), ("lines place", 163, 163), ("lines vertical", 84, 84), ("lines width", 48, 48), ("lines wrap", 54, 54), ("warnings", 1, 1)]),
    ("wrap-table-14", &[("glyphs extra", 11029, 11029), ("glyphs face", 11029, 11029), ("glyphs not drawn", 11029, 11029), ("glyphs size", 11029, 11029), ("lines fit", 420, 420), ("lines floating", 18, 18), ("lines jc", 105, 108), ("lines kind", 33, 33), ("lines place", 163, 163), ("lines vertical", 84, 84), ("lines width", 48, 48), ("lines wrap", 54, 54), ("warnings", 1, 1)]),
    ("wrap-table-15", &[("glyphs extra", 11029, 11029), ("glyphs face", 11029, 11029), ("glyphs not drawn", 11029, 11029), ("glyphs size", 11029, 11029), ("lines fit", 420, 420), ("lines floating", 18, 18), ("lines jc", 105, 108), ("lines kind", 33, 33), ("lines place", 163, 163), ("lines vertical", 84, 84), ("lines width", 48, 48), ("lines wrap", 54, 54), ("warnings", 1, 1)]),
    ("cell-anchor-none", &[("glyphs extra", 9718, 9718), ("glyphs face", 9718, 9718), ("glyphs not drawn", 9718, 9718), ("glyphs size", 9718, 9718), ("lines cell", 52, 52), ("lines h", 168, 168), ("lines size", 48, 48), ("lines v", 180, 180), ("lines wrap", 181, 291), ("warnings", 0, 1)]),
    ("cell-anchor-14", &[("glyphs extra", 9718, 9718), ("glyphs face", 9718, 9718), ("glyphs not drawn", 9718, 9718), ("glyphs size", 9718, 9718), ("lines cell", 52, 52), ("lines h", 168, 168), ("lines size", 48, 48), ("lines v", 180, 180), ("lines wrap", 181, 291), ("warnings", 0, 1)]),
    ("cell-anchor-15", &[("glyphs extra", 9718, 9718), ("glyphs face", 9718, 9718), ("glyphs not drawn", 9718, 9718), ("glyphs size", 9718, 9718), ("lines cell", 44, 52), ("lines h", 168, 168), ("lines size", 48, 48), ("lines v", 180, 180), ("lines wrap", 306, 306), ("warnings", 1, 1)]),
    ("cell-valign-14", &[("glyphs extra", 6890, 6890), ("glyphs face", 6890, 6890), ("glyphs not drawn", 6890, 6890), ("glyphs size", 6890, 6890), ("lines align", 185, 185), ("lines clamp", 216, 216), ("lines frame", 139, 139), ("lines long", 65, 65), ("lines merge", 83, 83), ("lines offset", 156, 156), ("lines room", 84, 84), ("lines tall", 92, 92), ("lines template", 12, 12), ("warnings", 1, 1)]),
    ("cell-valign-15", &[("glyphs extra", 6890, 6890), ("glyphs face", 6890, 6890), ("glyphs not drawn", 6890, 6890), ("glyphs size", 6890, 6890), ("lines align", 185, 185), ("lines clamp", 216, 216), ("lines frame", 139, 139), ("lines long", 65, 65), ("lines merge", 83, 83), ("lines offset", 156, 156), ("lines room", 84, 84), ("lines tall", 92, 92), ("lines template", 12, 12), ("warnings", 1, 1)]),
];

#[test]
fn wrap_table() {
    let recording = observations("wrap-table-observations.json");
    let names: Vec<&str> = WRAP_TABLE.iter().map(|(name, _)| *name).collect();
    check_family(&names, WRAP_TABLE_FLOORS, |name| {
        let maker = WRAP_TABLE.iter().find(|(n, _)| *n == name).map_or("", |(_, m)| m);
        score_objects(&recording, maker, name, Some("wrap-table"))
    });
}

/// Today's agreement.
#[rustfmt::skip]
const WRAP_TABLES_FLOORS: Floors = &[
    ("wraptables-none", &[("glyphs extra", 2333, 2333), ("glyphs face", 2333, 2333), ("glyphs not drawn", 2333, 2333), ("glyphs size", 2333, 2333), ("lines all", 418, 418), ("warnings", 1, 1)]),
    ("wraptables-12", &[("glyphs extra", 2333, 2333), ("glyphs face", 2333, 2333), ("glyphs not drawn", 2333, 2333), ("glyphs size", 2333, 2333), ("lines all", 418, 418), ("warnings", 1, 1)]),
    ("wraptables-15", &[("glyphs extra", 2333, 2333), ("glyphs face", 2333, 2333), ("glyphs not drawn", 2333, 2333), ("glyphs size", 2333, 2333), ("lines all", 418, 418), ("warnings", 1, 1)]),
];

#[test]
fn wrap_tables() {
    let recording = observations("wrap-tables-observations.json");
    check_family(
        &["wraptables-none", "wraptables-12", "wraptables-15"],
        WRAP_TABLES_FLOORS,
        |name| score_objects(&recording, "make_wrap_tables_probe", name, None),
    );
}
