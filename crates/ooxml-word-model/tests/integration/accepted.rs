//! A table as accepting its tracked changes leaves it (docx-agent's `_Accepted`): rows and
//! grid columns kept, cells taken into the one before them, cells Accept All removes, and
//! positions mapped with their refusals and notes (made-up documents).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use ooxml_word_model::docx::tables::{Accepted, Axis, TableGrid};
use ooxml_word_model::docx::Docx;
use support::{package, Parts};

const AUTHOR: &str = r#"w:author="R" w:date="2026-10-01T09:00:00Z""#;

fn cell(content: &str, properties: &str) -> String {
    format!("<w:tc><w:tcPr>{properties}</w:tcPr>{content}</w:tc>")
}

fn text(t: &str) -> String {
    format!("<w:p><w:r><w:t>{t}</w:t></w:r></w:p>")
}

/// A paragraph whose mark is deleted.
fn marked(t: &str) -> String {
    format!(r#"<w:p><w:pPr><w:rPr><w:del w:id="9" {AUTHOR}/></w:rPr></w:pPr><w:r><w:t>{t}</w:t></w:r></w:p>"#)
}

/// A paragraph whose text and mark are deleted.
fn gone(t: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:rPr><w:del w:id="7" {AUTHOR}/></w:rPr></w:pPr><w:del w:id="8" {AUTHOR}><w:r><w:delText>{t}</w:delText></w:r></w:del></w:p>"#
    )
}

fn table(rows: &[String]) -> String {
    format!(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1"/><w:gridCol w:w="1"/><w:gridCol w:w="1"/></w:tblGrid>{}</w:tbl>"#,
        rows.concat()
    )
}

fn accepted(body: &str) -> Accepted {
    let docx = Docx::open(
        &package(&Parts {
            body: body.to_owned(),
            ..Parts::default()
        }),
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap();
    let part = docx.body().unwrap();
    let found = part.ids.tables().get(0).unwrap();
    TableGrid::new(part, &found).accepted("t:X")
}

fn row(cells: &[String], properties: &str) -> String {
    format!("<w:tr><w:trPr>{properties}</w:trPr>{}</w:tr>", cells.concat())
}

fn plain(t: &str) -> String {
    cell(&text(t), "")
}

#[test]
fn a_deleted_row_is_not_counted_and_said_once() {
    let deleted = format!(r#"<w:del w:id="1" {AUTHOR}/>"#);
    let a = accepted(&table(&[
        row(&[plain("A"), plain("B"), plain("C")], ""),
        row(&[plain("North"), plain("1"), plain("2")], &deleted),
        row(&[plain("South"), plain("3"), plain("4")], ""),
    ]));
    assert!(a.rows_active() && !a.active());
    assert_eq!(a.rows, vec![0, 2]);
    let mut notes = Vec::new();
    assert_eq!(a.to_row(1, "row", &mut notes).unwrap(), 2);
    assert_eq!(a.to_row(1, "row", &mut notes).unwrap(), 2);
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0].contains("document row 1 ('North') has a tracked deletion"),
        "{notes:?}"
    );
    let refused = a.to_row(2, "index", &mut notes).unwrap_err();
    assert_eq!(refused.field, "index");
    assert_eq!(
        refused.error.message,
        "index 2: t:X has rows 0 to 1 as accepted (document row(s) 1 deleted by a pending tracked change, not counted)"
    );
    assert_eq!(a.from_row(1), 0);
}

#[test]
fn a_column_only_deleted_cells_cover_goes() {
    let del = format!(r#"<w:cellDel w:id="2" {AUTHOR}/>"#);
    let a = accepted(&table(&[
        row(&[plain("Item"), cell(&gone("Old"), &del), plain("New")], ""),
        row(&[plain("Pens"), cell(&gone("1"), &del), plain("2")], ""),
    ]));
    assert_eq!(a.kept, vec![0, 2]);
    let mut notes = Vec::new();
    assert_eq!(a.to_grid(1, "col", &mut notes).unwrap(), 2);
    assert!(notes[0].ends_with("grid column 1 ('Old') has a tracked deletion not yet accepted and is not counted (word_changes reaches it); col 1 is grid column 2"), "{notes:?}");
    assert_eq!(a.place(0, 1, "first", &mut notes).unwrap(), (0, 2));
    assert_eq!(notes.len(), 1);
}

/// Word's Accept All removes a deleted cell that keeps nothing and moves the cells after it
/// left (Word 16.106, lane fix-moved-left, `Stagger-gone`): where that differs between rows,
/// a column as accepted is a different grid column in different rows. A position by row is
/// that row's own cell; a whole column (or every column, as a sync or a copy writes them) is
/// refused as not supported, naming the rows that differ.
#[test]
fn a_column_whose_cells_move_left_in_some_rows_is_mapped_by_row_and_refused_whole() {
    let del = format!(r#"<w:cellDel w:id="2" {AUTHOR}/>"#);
    let a = accepted(&table(&[
        row(&[cell(&gone("x"), &del), plain("B1"), plain("C1")], ""),
        row(&[plain("A2"), cell(&gone("y"), &del), plain("C2")], ""),
        row(&[plain("A3"), plain("B3"), plain("C3")], ""),
    ]));
    assert!(a.active() && a.any_moved());
    assert_eq!(a.width(), 3);
    assert_eq!(a.deleted(), Vec::<usize>::new());
    let mut notes = Vec::new();
    assert_eq!(a.to_row_grid(0, 0, "col", &mut notes).unwrap(), 1);
    assert_eq!(a.to_row_grid(1, 1, "col", &mut notes).unwrap(), 2);
    assert_eq!(a.to_row_grid(2, 1, "col", &mut notes).unwrap(), 1);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].ends_with("col 0 is grid column 1 in that row"), "{notes:?}");
    let refused = a.to_grid(0, "index", &mut notes).unwrap_err();
    assert_eq!((refused.error.code, refused.field.as_str()), ("not_supported", "index"));
    assert!(
        refused.error.message.contains("in row(s) 0 pending cell changes"),
        "{}",
        refused.error.message
    );
    let whole = a.whole_columns(0..a.width(), "table").unwrap_err();
    assert!(
        whole
            .error
            .message
            .starts_with("table: cols 0 to 2 of t:X as accepted is not one grid column in every row: in row(s) 0, 2"),
        "{}",
        whole.error.message
    );
    // the last column is only row 2's C3, where it was: one grid column
    assert!(a.whole_columns(2..3, "index").is_ok());
    assert_eq!(a.to_grid(2, "index", &mut notes).unwrap(), 2);
}

#[test]
fn cells_taken_in_answer_as_the_cell_taking_them_and_a_lost_cell_is_refused() {
    let del = format!(r#"<w:cellDel w:id="3" {AUTHOR}/>"#);
    let a = accepted(&table(&[
        row(&[plain("Left"), cell(&text("Kept"), &del), plain("Right")], ""),
        row(&[cell(&marked("Joined"), ""), plain("into it"), plain("Alone")], ""),
        row(&[plain("One"), plain("Two"), cell(&marked("Lost"), "")], ""),
    ]));
    assert!(a.active());
    assert_eq!(a.kept, vec![0, 1, 2]);
    assert_eq!(a.given.get(0, 1), Some(0));
    assert_eq!(a.given.get(1, 1), Some(0));
    assert!(a.lost.contains(&(2, 2)));
    let mut notes = Vec::new();
    assert_eq!(a.at(0, 1, &mut notes).unwrap(), 0);
    assert_eq!(a.at(1, 1, &mut notes).unwrap(), 0);
    assert_eq!(
        notes,
        vec![
            "(0, 1) is in the cell a pending tracked merge makes of (0, 0): written there".to_owned(),
            "(1, 1) is in the cell accepting a deleted paragraph mark makes of (1, 0): written there".to_owned(),
        ]
    );
    let lost = a.at(2, 2, &mut notes).unwrap_err();
    assert_eq!((lost.error.code, lost.field.as_str()), ("refused", "cells"));
}

#[test]
fn labels_count_the_rows_and_columns_kept() {
    let deleted = format!(r#"<w:del w:id="1" {AUTHOR}/>"#);
    let body = table(&[
        row(&[plain("A"), plain("B"), plain("C")], ""),
        row(&[plain("North"), plain("1"), plain("2")], &deleted),
        row(&[plain("North"), plain("3"), plain("4")], ""),
    ]);
    let docx = Docx::open(
        &package(&Parts {
            body,
            ..Parts::default()
        }),
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap();
    let part = docx.body().unwrap();
    let found = part.ids.tables().get(0).unwrap();
    let grid = TableGrid::new(part, &found);
    let a = grid.accepted("t:X");
    let mut notes = Vec::new();
    // the deleted row's "North" is not counted: one match
    assert_eq!(
        a.locate(&grid, (Axis::Label("North"), Axis::Label("C")), &mut notes)
            .unwrap(),
        (2, 2)
    );
    assert!(notes.is_empty());
}

#[test]
fn a_cell_of_billions_of_columns_taken_in_is_one_run() {
    // the cell after `Joined` spans 2147483645 columns (the largest span Word opens) and is
    // taken into it: its positions answer as that cell, kept as one run, and only the
    // grid's three columns are counted
    let wide = cell(&text("into it"), r#"<w:gridSpan w:val="2147483645"/>"#);
    let a = accepted(&table(&[
        row(&[cell(&marked("Joined"), ""), wide], ""),
        row(&[plain("One"), plain("Two"), plain("Three")], ""),
    ]));
    assert!(a.active());
    assert_eq!(a.kept, vec![0, 1, 2]);
    assert_eq!(a.given.get(0, 1), Some(0));
    assert_eq!(a.given.get(0, 2_147_483_645), Some(0));
    assert_eq!(a.given.get(0, 2_147_483_646), None);
    assert_eq!(a.given.get(1, 1), None);
    assert_eq!(a.given.iter().count(), 1);
}

/// Word 16.106's Accept All of the shapes where the readers' final views differed before
/// one View module (`crates/ooxml-oracle/word/observations/view-word.json`,
/// `probes/view_probe.py`): each case's table, its rows as accepted and the cells each keeps,
/// held to the rows and cells Word's accepted table has; whether the part's facts see a
/// cell end there.
#[test]
fn rows_and_cell_ends_as_accepted_follow_words_accept_all() {
    use ooxml_word_model::view::{row_ends, View};
    let observed: serde_json::Value =
        serde_json::from_str(include_str!("../../../ooxml-oracle/word/observations/view-word.json")).unwrap();
    for (name, rows, ends) in word_cases() {
        let word = &observed[name]["blocks"][1]["table"];
        let word_rows = word.as_array().unwrap();
        let body = table(&rows.iter().map(|cells| row(cells, "")).collect::<Vec<_>>());
        let a = accepted(&body);
        assert_eq!(a.height(), word_rows.len(), "{name}: rows as accepted");
        let docx = Docx::open(
            &package(&Parts {
                body: body.clone(),
                ..Parts::default()
            }),
            std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
        )
        .unwrap();
        let part = docx.body().unwrap();
        assert_eq!(part.facts().cell_ends(), ends, "{name}: a cell end");
        let table = part.root().find(Some(ooxml_word_model::ids::W), "body").unwrap();
        let table = table.find(Some(ooxml_word_model::ids::W), "tbl").unwrap();
        let kept: Vec<_> = table
            .child_elements()
            .filter(|r| r.name().local() == "tr" && !View::Final.removes_row(r))
            .collect();
        let cells: Vec<usize> = kept.iter().map(|r| row_ends(r).groups.len()).collect();
        let word_cells: Vec<usize> = word_rows.iter().map(|r| r.as_array().unwrap().len()).collect();
        assert_eq!(cells, word_cells, "{name}: cells kept per row");
    }
}

/// The cases of `view-word.json`: each its name, its table's rows of cells, and whether a
/// cell end is there.
fn word_cases() -> Vec<(&'static str, Vec<Vec<String>>, bool)> {
    let deleted_cell = format!(r#"<w:cellDel w:id="5" {AUTHOR}/>"#);
    let control = |content: &str| {
        format!(r#"<w:sdt><w:sdtPr><w:id w:val="1"/></w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>"#)
    };
    let empty = "<w:p/>".to_owned();
    vec![
        (
            "AllCellsDeleted",
            vec![
                vec![plain("A1"), plain("B1")],
                vec![cell(&text("A2"), &deleted_cell), cell(&text("B2"), &deleted_cell)],
                vec![plain("A3"), plain("B3")],
            ],
            false,
        ),
        (
            "AllCellsDeletedEmpty",
            vec![
                vec![plain("A1"), plain("B1")],
                vec![cell(&empty, &deleted_cell), cell(&empty, &deleted_cell)],
                vec![plain("A3"), plain("B3")],
            ],
            false,
        ),
        (
            "ControlEndsCell",
            vec![
                vec![cell(&control(&marked("P1")), ""), plain("Q1")],
                vec![plain("P2"), plain("Q2")],
            ],
            true,
        ),
        (
            "ParagraphEndsCell",
            vec![
                vec![cell(&marked("P1"), ""), plain("Q1")],
                vec![plain("P2"), plain("Q2")],
            ],
            true,
        ),
        (
            "KeptAfterGone",
            vec![
                vec![cell(&marked("Y1"), ""), cell(&text("Z1"), &deleted_cell)],
                vec![plain("Y2"), plain("Z2")],
            ],
            false,
        ),
        (
            "KeptAfterPlain",
            vec![
                vec![plain("Y1"), cell(&text("Z1"), &deleted_cell)],
                vec![plain("Y2"), plain("Z2")],
            ],
            false,
        ),
    ]
}

/// Word's Accept All of a deleted cell whose content stays, in the only row dividing grid
/// columns 0 and 1 (lane fix-final-grid, 19830c99, `HostSplit`): the two are one column.
#[test]
fn columns_no_cell_divides_once_accepted_are_one_position() {
    let deleted = format!(r#"<w:cellDel w:id="3" {AUTHOR}/>"#);
    let a = accepted(&table(&[
        row(&[cell(&text("A"), r#"<w:gridSpan w:val="2"/>"#), plain("C")], ""),
        row(&[plain("D"), cell(&text(""), &deleted), plain("F")], ""),
    ]));
    assert_eq!(a.kept, [0, 2]);
    assert_eq!(a.width(), 2);
    assert_eq!(
        a.shape(),
        "t:X has columns 0 to 1 as accepted; grid columns 0 and 1 are one column in this view (no cell divides them)"
    );
    let mut notes = Vec::new();
    assert_eq!(a.to_grid(1, "col", &mut notes), Ok(2));
    assert_eq!(
        notes,
        [
            "col positions count t:X as accepted: grid columns 0 and 1 are one column in this view (no cell divides \
          them); col 1 is grid column 2"
        ]
    );
}

/// The same table's final-view Markdown: no cell spans the one column grid columns 0 and 1
/// make, as in Word's Accept All of it.
#[test]
fn the_final_markdown_counts_spans_in_the_view_grid() {
    let deleted = format!(r#"<w:cellDel w:id="3" {AUTHOR}/>"#);
    let body = table(&[
        row(&[cell(&text("A"), r#"<w:gridSpan w:val="2"/>"#), plain("C")], ""),
        row(&[plain("D"), cell(&text(""), &deleted), plain("F")], ""),
    ]);
    let docx = Docx::open(
        &package(&Parts {
            body,
            ..Parts::default()
        }),
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap();
    let markdown = ooxml_word_model::markdown::to_plain_markdown(&docx, None).unwrap();
    assert!(!markdown.contains("colspan"), "{markdown}");
    assert!(markdown.contains("| A | C |"), "{markdown}");
}

/// The original view of a cell with a pending properties change takes its old properties,
/// clean, and the grid is rebuilt from them (user decision original-clean-grid-2026-11-28):
/// a span the change made is gone; a vertical merge either side has stays.
#[test]
fn the_original_view_takes_a_changed_cells_old_properties() {
    use ooxml_word_model::markdown::{to_markdown, Stories};
    use ooxml_word_model::view::View;
    let change = |old: &str| format!(r#"<w:tcPrChange w:id="4" {AUTHOR}><w:tcPr>{old}</w:tcPr></w:tcPrChange>"#);
    let body = table(&[
        row(
            &[
                cell(
                    &text("A"),
                    &format!(r#"<w:gridSpan w:val="2"/><w:vMerge w:val="restart"/>{}"#, change("")),
                ),
                plain("C"),
            ],
            "",
        ),
        row(
            &[
                cell(&text(""), &format!(r#"<w:vMerge/>{}"#, change(""))),
                plain("E"),
                plain("F"),
            ],
            "",
        ),
    ]);
    let docx = Docx::open(
        &package(&Parts {
            body,
            ..Parts::default()
        }),
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap();
    let original = to_markdown(&docx, None, View::Original, Stories::Body).unwrap();
    assert!(original.contains("rowspan=\"2\""), "{original}");
    assert!(!original.contains("colspan"), "{original}");
    let markup = to_markdown(&docx, None, View::Markup, Stories::Body).unwrap();
    assert!(markup.contains("colspan=\"2\""), "{markup}");
}
