//! The object edits timed on a long document (lane P4c's hand-back): run with
//! `OBJECTS_BENCH=<path to a .docx> cargo test --release -p ooxml-word-edit --test integration
//! -- --ignored --nocapture object_bench::`; each edit's best of three runs, each on the
//! document as opened (the first edit stamps the whole document).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{best, editor};
use ooxml_word_edit::{
    CellValues, EditTable, FormatTable, RevisionPolicy, TableChange, TableFormat, TableValues, TableWidth, Tracking,
};
use std::sync::Arc;
use std::time::Duration;

#[test]
#[ignore = "a timing on a long document: OBJECTS_BENCH=<docx> cargo test --release -- --ignored"]
fn object_edits_on_a_long_document() {
    let Ok(path) = std::env::var("OBJECTS_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let probe = editor(&bytes);
    let docx = probe.store().snapshot().doc().docx().unwrap();
    let tables: Vec<String> = docx.body().unwrap().ids.tables().iter().map(|t| t.id.clone()).collect();
    println!("{} tables", tables.len());
    let table = tables[tables.len() / 2].clone();
    let report = |name: &str, took: Duration| println!("{name}: {:.1} ms", took.as_secs_f64() * 1000.0);
    let tracked = RevisionPolicy::Tracked(Tracking {
        author: "Bench".into(),
        date: "2026-11-18T10:00:00Z".into(),
    });
    let whole = TableFormat::Table(TableValues {
        style: Some("Table Grid".into()),
        width: Some(TableWidth::Percent(80.0)),
        alignment: Some("center".into()),
        borders: Some("single".into()),
        header_rows: Some(1),
        look: Some([true, false, true, false, true, false]),
        ..TableValues::default()
    });
    let column = (0..3)
        .map(|r| TableFormat::Cell {
            row: r,
            column: 0,
            values: CellValues {
                shading: Some(Some("D9E2F3".into())),
                borders: Some("single".into()),
                ..CellValues::default()
            },
        })
        .collect::<Vec<_>>();
    for (name, formats, policy) in [
        (
            "format table (style, width, borders, look)",
            vec![whole.clone()],
            RevisionPolicy::Untracked,
        ),
        ("format table, tracked", vec![whole], tracked.clone()),
        ("format column cells, tracked", column, tracked.clone()),
    ] {
        let command = FormatTable {
            table: table.clone(),
            formats,
            policy,
        };
        report(name, best(&bytes, &command));
    }
    for (name, change, policy) in [
        (
            "delete row",
            TableChange::DeleteRow { index: 1 },
            RevisionPolicy::Untracked,
        ),
        (
            "delete row, tracked",
            TableChange::DeleteRow { index: 1 },
            tracked.clone(),
        ),
        (
            "delete column",
            TableChange::DeleteColumn { index: 0 },
            RevisionPolicy::Untracked,
        ),
        (
            "delete column, tracked",
            TableChange::DeleteColumn { index: 0 },
            tracked.clone(),
        ),
    ] {
        let command = EditTable {
            table: table.clone(),
            change,
            policy,
        };
        report(name, best(&bytes, &command));
    }
}
