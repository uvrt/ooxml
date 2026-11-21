//! The contact image's options, fitting and SVG as ooxml-edit's `tiles.py` makes them (the
//! expected values are its own, run on the same inputs).

use super::*;

const SHEET: TileAsk<'static> = TileAsk {
    name: "sheet",
    what: "pages",
    width: None,
    max_edge: 2576,
};

fn plain(columns: i64, width: i64) -> Tiling {
    Tiling {
        columns,
        width,
        asked: None,
        why: None,
    }
}

#[test]
fn the_defaults_are_four_columns_of_360_px() {
    let tiling = tiling_of(TileOptions::default(), 30, SHEET).unwrap();
    assert_eq!(tiling, plain(4, 360));
    assert_eq!(tiling.total_width(), 1480);
    assert_eq!(tiling.shrunk(), None);
}

#[test]
fn tiles_too_wide_side_by_side_are_narrowed_saying_why() {
    let options = TileOptions {
        columns: Some(8),
        width: Some(1280),
    };
    let tiling = tiling_of(options, 30, SHEET).unwrap();
    assert_eq!((tiling.columns, tiling.width, tiling.asked), (8, 313, Some(1280)));
    assert_eq!(
        tiling.why.as_deref(),
        Some("8 columns of 1280 px make the sheet 10312 px wide, over the 2576 px limit")
    );
    let wide = TileAsk {
        width: Some(2000),
        ..SHEET
    };
    let tiling = tiling_of(TileOptions::default(), 3, wide).unwrap();
    assert_eq!((tiling.columns, tiling.width, tiling.asked), (3, 848, Some(2000)));
    assert_eq!(
        tiling.why.as_deref(),
        Some("width 2000 is over a tile's 1280 px; then 3 columns of 1280 px make the sheet 3872 px wide, over the 2576 px limit")
    );
}

#[test]
fn tall_tiles_are_narrowed_to_the_widest_that_fit() {
    let tiling = fit(&plain(4, 360), &[1.294; 30], SHEET).unwrap();
    assert_eq!((tiling.width, tiling.asked), (226, Some(360)));
    assert_eq!(
        tiling.narrowed_note(),
        " (narrowed from 360 px: 30 pages in 4 column(s) of 360 px make the sheet 3960 px tall, over the 2576 px limit)"
    );
    let refused = fit(&plain(1, 360), &[1.3; 30], SHEET).unwrap_err();
    assert_eq!(
        refused.message(),
        "30 pages in 1 column(s) make the sheet over the 2576 px limit even at a tile width of 160: at most 10 pages at these columns; or more columns"
    );
    assert_eq!(refused.field(), Some("pages"));
}

#[test]
fn bounds_are_refused_with_the_way_out() {
    let options = TileOptions {
        columns: Some(9),
        width: None,
    };
    let refused = tiling_of(options, 4, SHEET).unwrap_err();
    assert_eq!(refused.field(), Some("sheet.columns"));
    assert!(refused.way_out().is_some());
    let many = tiling_of(TileOptions::default(), 31, SHEET).unwrap_err();
    assert_eq!(many.code().as_str(), "limit");
}

fn base64(bytes: &[u8], out: &mut String) {
    let map = [(b"abc".as_slice(), "YWJj"), (b"de".as_slice(), "ZGU=")];
    let found = map.iter().find(|(b, _)| *b == bytes).map_or("", |(_, t)| *t);
    out.push_str(found);
}

#[test]
fn the_svg_is_tiles_py_s_byte_for_byte() {
    let tiles = [
        Tile {
            png: b"abc",
            width: 10,
            height: 13,
            label: "page 1",
        },
        Tile {
            png: b"de",
            width: 10,
            height: 12,
            label: "a very long label that will not fit at all in it",
        },
    ];
    let (svg, width, height) = tiled_svg(&tiles, &plain(2, 160), base64);
    assert_eq!((width, height), (344, 49));
    assert_eq!(
        svg,
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="344" height="49" viewBox="0 0 344 49"><rect width="344" height="49" fill="#d4d4d4"/>"##,
            r##"<rect x="7.5" y="7.5" width="11" height="14" fill="none" stroke="#8c8c8c"/><image x="8" y="8" width="10" height="13" href="data:image/png;base64,YWJj"/><text x="10" y="37" font-family="sans-serif" font-size="13" fill="#1a1a1a">page 1</text>"##,
            r##"<rect x="175.5" y="7.5" width="11" height="13" fill="none" stroke="#8c8c8c"/><image x="176" y="8" width="10" height="12" href="data:image/png;base64,ZGU="/><text x="178" y="37" font-family="sans-serif" font-size="13" fill="#1a1a1a">a very long label th…</text></svg>"##
        )
    );
}
