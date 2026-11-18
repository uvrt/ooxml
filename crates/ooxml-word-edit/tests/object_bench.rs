//! The object edits timed on a long document (lane P4c's hand-back): run with
//! `OBJECTS_BENCH=<path to a .docx> cargo test --release --test object_bench -- --ignored
//! --nocapture`; each edit's best of three runs, each on the document as opened (the first
//! edit stamps the whole document).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, StoreConfig};
use ooxml_word_edit::{
    CellValues, EditError, Editor, FormatTable, RevisionPolicy, TableFormat, TableValues, TableWidth, Tracking, WordDoc,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Monotonic(Instant);

impl Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

fn editor(bytes: &Arc<[u8]>) -> Editor {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(Unlimited),
    )
    .unwrap()
}

/// The best of three runs of `command`, each on a fresh editor (opening not counted).
fn best<C: Command<WordDoc, Error = EditError>>(bytes: &Arc<[u8]>, command: &C) -> Duration {
    (0..3)
        .map(|_| {
            let editor = editor(bytes);
            let started = Instant::now();
            editor.run(&Caller::anonymous(), command).unwrap();
            started.elapsed()
        })
        .min()
        .unwrap()
}

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
        ("format column cells, tracked", column, tracked),
    ] {
        let command = FormatTable {
            table: table.clone(),
            formats,
            policy,
        };
        report(name, best(&bytes, &command));
    }
}
