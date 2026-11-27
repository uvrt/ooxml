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
