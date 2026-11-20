//! Tables on made-up documents over one face (support::Mono; a page of 2160 twips holds
//! seven lines; a 1080-twip column holds one 8-letter word a line): a row as tall as its
//! tallest cell, a row split across pages under its repeated header, a row that may not
//! split moved whole, a cell merged down making its last row taller, a nested table one
//! line as tall as itself, and the lines placed rows draw.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::support;

use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::paginate::Position;
use ooxml_word_layout::px::Px;
use ooxml_word_layout::table::TableFlow;
use ooxml_word_layout::tables::MeasuredTable;
use ooxml_word_layout::DocumentLayout;
use support::{engine, model, open, para, run, sect};

const WORD: &str = "abcdefgh";

fn words(n: usize) -> String {
    vec![WORD; n].join(" ")
}

fn cell(tcpr: &str, content: &str) -> String {
    format!("<w:tc><w:tcPr>{tcpr}</w:tcPr>{content}</w:tc>")
}

fn text_cell(tcpr: &str, n: usize) -> String {
    cell(tcpr, &para("", &run(&words(n))))
}

fn row(trpr: &str, cells: &[String]) -> String {
    format!("<w:tr><w:trPr>{trpr}</w:trPr>{}</w:tr>", cells.concat())
}

/// A fixed table of `columns` 1080-twip columns, no margins and no borders.
fn table(columns: usize, rows: &[String]) -> String {
    format!(
        r#"<w:tbl><w:tblPr><w:tblLayout w:type="fixed"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>{}</w:tblGrid>{}</w:tbl>"#,
        r#"<w:gridCol w:w="1080"/>"#.repeat(columns),
        rows.concat()
    )
}

fn lay_out(body: &str) -> DocumentLayout {
    let mut m = model();
    let d = open(&mut m, &format!("{body}{}", sect("nextPage", 1)), None);
    engine().lay_out(&mut m, &d)
}

/// One line's pitch: a one-line paragraph's.
fn pitch() -> Px {
    let layout = lay_out(&para("", &run(WORD)));
    layout.flow.items[0].para().unwrap().measured.heights[0].pitch()
}

fn measured(layout: &DocumentLayout, item: usize) -> &MeasuredTable {
    let FlowItem::Table(t) = &layout.flow.items[item] else {
        panic!("item {item} is not a table: {:?}", layout.flow.items[item]);
    };
    t.flow.as_any().downcast_ref::<MeasuredTable>().unwrap()
}

fn lines(n: i64) -> Px {
    pitch().scale(i128::from(n), 1).unwrap()
}

#[test]
fn a_row_is_its_tallest_cell() {
    let layout = lay_out(&table(2, &[row("", &[text_cell("", 1), text_cell("", 3)])]));
    assert!(layout.pagination.stopped.is_none(), "{:?}", layout.pagination.stopped);
    let t = measured(&layout, 0);
    assert_eq!(t.rows[0].height, lines(3));
    assert_eq!(t.rows[0].cells[1].stack.len(), 3);
}

#[test]
fn a_measured_cell_shares_the_model_s_resolved_cell() {
    let mut m = model();
    let body = table(2, &[row("", &[text_cell("", 1), text_cell("", 3)])]);
    let d = open(&mut m, &format!("{body}{}", sect("nextPage", 1)), None);
    let layout = engine().lay_out(&mut m, &d);
    let ooxml_word_model::model::Block::Table(source) = &d.body[0] else {
        panic!("not a table: {:?}", d.body[0]);
    };
    let resolved = m.resolve_table(&d, source);
    let t = measured(&layout, 0);
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            assert!(std::sync::Arc::ptr_eq(&cell.resolved, &resolved.rows[r].cells[c]));
        }
    }
}

#[test]
fn a_row_splits_under_its_repeated_header() {
    let body = table(
        2,
        &[
            row("<w:tblHeader/>", &[text_cell("", 1), text_cell("", 1)]),
            row("", &[text_cell("", 1), text_cell("", 12)]),
        ],
    );
    let layout = lay_out(&body);
    let pages = &layout.pagination.pages;
    assert_eq!(
        pages.len(),
        2,
        "{:?}",
        pages.iter().map(|p| p.start.clone()).collect::<Vec<_>>()
    );
    // The header and six lines of the row on the first page; the header again and the
    // row's other six lines on the second.
    assert_eq!(
        pages[1].start,
        Position {
            item: 0,
            line: 1,
            part: vec![1, 6]
        }
    );
    let second = &pages[1].rows;
    assert!(second[0].row.header && second[0].row.row == 0, "{second:?}");
    assert!(second[1].row.continued && second[1].row.row == 1, "{second:?}");
    let drawn = measured(&layout, 0).lines(&second[1].row);
    assert_eq!(drawn.len(), 6);
    assert_eq!(drawn[0].top, second[1].row.top);
}

#[test]
fn a_row_that_may_not_split_moves_whole() {
    let body = format!(
        "{}{}",
        para("", &run(&words(12))),
        table(2, &[row("<w:cantSplit/>", &[text_cell("", 3), text_cell("", 1)])])
    );
    let layout = lay_out(&body);
    let starts: Vec<Position> = layout.pagination.pages.iter().map(|p| p.start.clone()).collect();
    // The paragraph's last five lines leave two on the second page: the row of three lines,
    // which would split there, moves whole to the third.
    assert_eq!(starts.last(), Some(&Position::at(1, 0)), "{starts:?}");
}

#[test]
fn a_cell_merged_down_makes_its_last_row_taller() {
    let body = table(
        2,
        &[
            row("", &[text_cell(r#"<w:vMerge w:val="restart"/>"#, 4), text_cell("", 1)]),
            row("", &[cell(r#"<w:vMerge/>"#, &para("", "")), text_cell("", 1)]),
        ],
    );
    let layout = lay_out(&body);
    let t = measured(&layout, 0);
    assert_eq!(t.rows[0].height, lines(1));
    assert_eq!(t.rows[1].height, lines(3));
    assert_eq!(t.rows[0].cells[0].span_height, Some(lines(4)));
}

#[test]
fn a_nested_table_is_one_line_as_tall_as_itself() {
    let inner = table(1, &[row("", &[text_cell("", 1)]), row("", &[text_cell("", 2)])]);
    let body = table(
        2,
        &[row(
            "",
            &[cell("", &format!("{inner}{}", para("", ""))), text_cell("", 1)],
        )],
    );
    let layout = lay_out(&body);
    let t = measured(&layout, 0);
    // The empty paragraph after the nested table takes no room.
    assert_eq!(t.rows[0].height, lines(3));
    let page = &layout.pagination.pages[0];
    let drawn = t.lines(&page.rows[0].row);
    // The nested table's three lines, then the other cell's one.
    assert_eq!(drawn.iter().map(|l| l.block).collect::<Vec<_>>(), vec![0, 1, 1, 3]);
}
