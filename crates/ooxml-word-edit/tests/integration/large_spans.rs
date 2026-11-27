//! Tables whose `w:gridSpan` or `w:gridBefore` holds billions of columns (2147483645, the
//! largest Word opens, and 2147483647, one it refuses) edited by `set_cells` and a cell
//! split: the grid places are kept as one run per cell, so each edit or refusal is quick and
//! small, not one place per grid column.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::editor;
use ooxml_store::Caller;
use ooxml_word_edit::{
    CellWrite, EditError, EditTable, Editor, Review, RevisionPolicy, Selection, SetCells, TableChange,
};
use ooxml_word_model::docx::tables::{grid_span, GRID_VALUE_MAX};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

/// The values tried, as written: the largest span Word opens, `2147483647` (refused by
/// Word), and a value past 64 bits, which wraps to 1661992959 columns.
fn huge() -> [String; 3] {
    [
        GRID_VALUE_MAX.to_string(),
        (GRID_VALUE_MAX + 2).to_string(),
        "99999999999999999999".to_owned(),
    ]
}

/// Well under what one place per grid column would take (seconds and gigabytes).
const QUICK: Duration = Duration::from_secs(5);

fn cell(span: Option<&str>, text: &str) -> String {
    let span = span.map_or_else(String::new, |s| {
        format!(r#"<w:tcPr><w:gridSpan w:val="{s}"/></w:tcPr>"#)
    });
    format!("<w:tc>{span}<w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>")
}

/// A document of one table on a grid of two columns: row 0 a cell of `w:gridSpan` `span`
/// and one cell after it, row 1 `w:gridBefore` `span` and one cell, row 2 two plain cells.
fn document(span: &str) -> Editor {
    let rows = [
        format!("<w:tr>{}{}</w:tr>", cell(Some(span), "wide"), cell(None, "after")),
        format!(
            r#"<w:tr><w:trPr><w:gridBefore w:val="{span}"/></w:trPr>{}</w:tr>"#,
            cell(None, "late")
        ),
        format!("<w:tr>{}{}</w:tr>", cell(None, "a"), cell(None, "b")),
    ];
    table_document(&rows.concat())
}

/// A document of one table on a grid of two columns whose rows are `rows`.
fn table_document(rows: &str) -> Editor {
    let body = format!(
        r#"<w:document xmlns:w="{W}"><w:body><w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{rows}</w:tbl><w:p/></w:body></w:document>"#
    );
    let parts = [
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned()),
        ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml", body),
    ];
    let mut zip = Writer::new();
    for (name, data) in parts {
        let entry = NewEntry {
            name: name.to_owned(),
            method: Method::Deflated,
            dos_time: 0,
            dos_date: 33,
            external_attr: 0,
            create_system: 3,
        };
        zip.add(&entry, data.as_bytes()).unwrap();
    }
    editor(&Arc::from(zip.finish(b"").unwrap()))
}

/// The document's table, by its id.
fn table(editor: &Editor) -> ooxml_word_model::address::Address {
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let id = docx.parts[0].ids.tables().iter().next().unwrap().id.clone();
    ooxml_word_model::address::Address::from(id)
}

fn set(editor: &Editor, row: usize, column: usize) -> Result<Vec<String>, EditError> {
    let command = SetCells {
        table: table(editor),
        writes: vec![CellWrite::at(row, column, "new".to_owned())],
        policy: RevisionPolicy::Untracked,
        accepted: None,
    };
    // each id's place in the table (`<table>/cR,C`), the table's own id aside
    editor.run(&Caller::anonymous(), &command).map(|done| {
        done.out
            .ids
            .iter()
            .filter_map(|id| id.rsplit('/').next().map(str::to_owned))
            .collect()
    })
}

fn split(editor: &Editor, row: usize, column: usize) -> Result<(), EditError> {
    let command = EditTable {
        table: table(editor),
        change: TableChange::Split {
            row,
            column,
            rows: 2,
            columns: Some(1),
        },
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).map(|_| ())
}

#[test]
fn set_cells_writes_across_a_span_of_billions_of_columns() {
    for raw in huge() {
        let span = grid_span(Some(&raw));
        assert!(span > 1_000_000_000, "{raw}");
        let started = Instant::now();
        let editor = document(&raw);
        // anywhere in the wide cell is the wide cell; the cells after it, past the span
        assert_eq!(set(&editor, 0, 0).unwrap(), ["c0,0"], "{span}");
        assert_eq!(set(&editor, 0, span - 1).unwrap(), ["c0,0"], "{span}");
        assert_eq!(set(&editor, 0, span).unwrap(), [format!("c0,{span}")], "{span}");
        assert_eq!(set(&editor, 1, span).unwrap(), [format!("c1,{span}")], "{span}");
        assert_eq!(set(&editor, 2, 1).unwrap(), ["c2,1"], "{span}");
        // no cell before the late row's cell: refused, the table's shape counted, not listed
        let Err(EditError::Address(refused)) = set(&editor, 1, 7) else {
            panic!("{span}: (1, 7) is no cell");
        };
        let columns = span + 1;
        assert!(
            refused
                .message
                .contains(&format!("3 row(s) (0 to 2) and {columns} column(s) (0 to {span})")),
            "{}",
            refused.message
        );
        assert!(
            refused.message.contains(&format!(
                "merged: (0, 0)-(0, {}) (a merged cell's id is its first position's)",
                span - 1
            )),
            "{}",
            refused.message
        );
        let empty = 2 * columns - 2 - 1;
        assert!(
            refused.message.ends_with(&format!(
                "; no cell at (1, 0), (1, 1), (1, 2), (1, 3), (1, 4), (1, 5), (1, 6), (1, 7), (1, 8), (1, 9) \
                 and {} more",
                empty - 10
            )),
            "{}",
            refused.message
        );
        assert!(started.elapsed() < QUICK, "{span}: {:?}", started.elapsed());
    }
}

#[test]
fn a_split_in_a_table_of_billions_of_columns_is_quick() {
    for raw in huge() {
        let span = grid_span(Some(&raw));
        assert!(span > 1_000_000_000, "{raw}");
        let started = Instant::now();
        let editor = document(&raw);
        // no cell at the place: refused with the table's shape, counted by runs
        let Err(EditError::Refused(message)) = split(&editor, 1, 3) else {
            panic!("{span}: (1, 3) is no cell");
        };
        assert!(message.contains(&format!("and {} column(s)", span + 1)), "{message}");
        assert!(message.contains(" more"), "{message}");
        // the cell after the wide one, and the wide one, each split into two rows
        split(&editor, 0, span).unwrap();
        split(&editor, 0, 0).unwrap();
        assert!(started.elapsed() < QUICK, "{span}: {:?}", started.elapsed());
    }
}

#[test]
fn a_review_leaves_the_grid_of_a_row_reaching_billions_of_columns_with_a_warning() {
    // accepting an insertion in a table whose first row reaches past the 63 columns a Word
    // table takes: its grid is not fitted (one w:gridCol per column), it is left as it is
    // and the review says so
    for raw in huge() {
        let started = Instant::now();
        let inserted = r#"<w:tc><w:p><w:ins w:id="1" w:author="R" w:date="2026-11-25T00:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins></w:p></w:tc>"#;
        let editor = table_document(&format!(
            "<w:tr>{}{}</w:tr><w:tr>{inserted}{}</w:tr>",
            cell(Some(&raw), "wide"),
            cell(None, "after"),
            cell(None, "b")
        ));
        let review = Review {
            accept: true,
            selection: Selection::All,
        };
        let done = editor.run(&Caller::anonymous(), &review).unwrap();
        let reach = grid_span(Some(&raw)) + 1;
        assert!(
            done.out.warnings.iter().any(|w| w.contains(&format!(
                "a table's grid was left as it is: its row 0 reaches {reach} grid columns, past the 63"
            ))),
            "{raw}: {:?}",
            done.out.warnings
        );
        let tree = editor
            .store()
            .snapshot()
            .doc()
            .tree("word/document.xml")
            .unwrap()
            .unwrap();
        let xml = String::from_utf8(tree.write()).unwrap();
        assert_eq!(xml.matches("<w:gridCol ").count(), 2, "{raw}");
        assert!(!xml.contains("w:gridAfter"), "{raw}");
        assert!(!xml.contains("<w:ins "), "{raw}: the insertion is accepted");
        assert!(started.elapsed() < QUICK, "{raw}: {:?}", started.elapsed());
    }
}
