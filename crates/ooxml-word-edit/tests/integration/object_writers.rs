//! The random multi-writer oracle for the object edits (lane P4c): writers on threads mix
//! tracked and untracked `set_text` and `insert_text` (in body and cell paragraphs) and
//! `set_cells` with tables formatted (`FormatTable`: a table's properties, a column's
//! width, a row's height, cells' shading and borders) and rows inserted and deleted
//! (`EditTable`: rows inserted, copied and deleted, columns inserted and deleted, cells
//! merged and split) through
//! one document's store;
//! replayed serially in commit order on the document as opened, every version is the
//! same, byte for byte, as the concurrent run made it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
#![allow(clippy::cast_precision_loss)]

use crate::common::{editor, fixture, policy, written, Rng, WORDS};
use ooxml_store::{Caller, Command, Seq, Snapshot, Txn};
use ooxml_word_edit::{
    CellValues, CellWrite, EditError, EditTable, FormatTable, InsertText, SetCells, SetText, TableChange, TableFormat,
    TableValues, TableWidth, WordDoc,
};

/// One edit of the mix.
#[derive(Debug, Clone)]
enum Edit {
    Set(SetText),
    Insert(InsertText),
    Cells(SetCells),
    Format(FormatTable<'static>),
    Rows(EditTable),
}

impl Command<WordDoc> for Edit {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        match self {
            Edit::Set(c) => c.validate(doc),
            Edit::Insert(c) => c.validate(doc),
            Edit::Cells(c) => c.validate(doc),
            Edit::Format(c) => c.validate(doc),
            Edit::Rows(c) => c.validate(doc),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        Ok(match self {
            Edit::Set(c) => c.prepare(tx)?.changed,
            Edit::Insert(c) => c.prepare(tx).map(|_| true)?,
            Edit::Cells(c) => c.prepare(tx).map(|out| !out.edits.is_empty())?,
            Edit::Format(c) => c.prepare(tx).map(|_| true)?,
            Edit::Rows(c) => c.prepare(tx).map(|_| true)?,
        })
    }
}

const FILLS: &[&str] = &["FFFF00", "D9E2F3", "E2EFDA"];

/// A random formatting of a table.
fn format(rng: &mut Rng) -> TableFormat {
    let (row, column) = (rng.below(3), rng.below(3));
    match rng.below(5) {
        0 => TableFormat::Table(TableValues {
            style: (rng.below(2) == 0).then(|| "Table Grid".to_owned()),
            width: Some(TableWidth::Length(ooxml_word_edit::units::twips(
                200.0 + 50.0 * rng.below(4) as f64,
            ))),
            alignment: Some(["left", "center", "right"][rng.below(3)].to_owned()),
            borders: Some(["single", "none", "double"][rng.below(3)].to_owned()),
            ..TableValues::default()
        }),
        1 => TableFormat::ColumnWidth {
            column,
            width: ooxml_word_edit::units::twips(40.0 + 10.0 * rng.below(5) as f64),
        },
        2 => TableFormat::Row {
            row,
            height: Some((
                ooxml_word_edit::units::twips(12.0 + rng.below(10) as f64),
                rng.below(2) == 0,
            )),
            cant_split: Some(rng.below(2) == 0),
        },
        _ => TableFormat::Cell {
            row,
            column,
            values: CellValues {
                shading: Some(Some(FILLS[rng.below(FILLS.len())].to_owned())),
                borders: (rng.below(2) == 0).then(|| "single".to_owned()),
                vertical_alignment: Some("center".to_owned()),
                width: None,
            },
        },
    }
}

/// A random edit of the mix: text in body and cell paragraphs, cells written, tables
/// formatted, rows inserted and deleted.
fn edit(rng: &mut Rng, targets: &[String], tables: &[String], writer: u64) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    let table = tables[rng.below(tables.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(4)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    match rng.below(10) {
        0 | 1 => Edit::Set(SetText {
            target: ooxml_word_model::address::Address::from(target),
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        2 => Edit::Insert(InsertText {
            at: format!("{target}@{}", rng.below(3)).into(),
            text: format!("{} ", words.join(" ")),
            policy: policy(rng, writer),
        }),
        3 => Edit::Cells(SetCells {
            table: ooxml_word_model::address::Address::from(table),
            writes: vec![CellWrite::at(rng.below(3), rng.below(3), words.join(" "))],
            policy: policy(rng, writer),
            accepted: None,
        }),
        8 | 9 => Edit::Rows(EditTable {
            table: ooxml_word_model::address::Address::from(table),
            change: match rng.below(8) {
                0 => TableChange::DeleteRow { index: rng.below(3) },
                1 => TableChange::DeleteColumn { index: rng.below(2) },
                2 => TableChange::InsertColumn {
                    index: rng.below(2),
                    right: rng.below(2) == 0,
                },
                3 => TableChange::CopyRow {
                    index: (rng.below(2) == 0).then(|| rng.below(3)),
                    below: rng.below(2) == 0,
                },
                4 => TableChange::Merge {
                    first: (rng.below(2), rng.below(2)),
                    last: (1 + rng.below(2), 1 + rng.below(2)),
                },
                5 => TableChange::Split {
                    row: rng.below(3),
                    column: rng.below(2),
                    rows: 1 + rng.below(2),
                    columns: (rng.below(2) == 0).then(|| 1 + rng.below(2)),
                },
                _ => TableChange::InsertRow {
                    index: (rng.below(2) == 0).then(|| rng.below(3)),
                    below: rng.below(2) == 0,
                    plain: true,
                },
            },
            policy: policy(rng, writer),
        }),
        _ => Edit::Format(FormatTable::new(
            ooxml_word_model::address::Address::from(table),
            (0..1 + rng.below(3)).map(|_| format(rng)).collect(),
            policy(rng, writer),
        )),
    }
}

fn run(seed: u64, writers: u64, per_writer: usize) -> (u64, u64, usize) {
    let bytes = fixture("generated/markdown/constructs.docx");
    let shared = editor(&bytes);
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let body = docx.body().unwrap();
    let targets: Vec<String> = body.ids.paragraphs().iter().map(|p| p.id.clone()).collect();
    let tables: Vec<String> = body.ids.tables().iter().map(|t| t.id.clone()).collect();
    assert!(!tables.is_empty(), "the fixture has tables");
    let mut records: Vec<(Seq, Edit, Snapshot<WordDoc>)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (shared, targets, tables) = (&shared, &targets, &tables);
                scope.spawn(move || {
                    let mut rng = Rng(seed ^ (w + 1).wrapping_mul(0x9E37_79B9));
                    let caller = Caller::named(&format!("writer {w}"));
                    (0..per_writer)
                        .filter_map(|_| {
                            let command = edit(&mut rng, targets, tables, w);
                            // a refusal (a cell a concurrent edit took away) changes nothing
                            let done = shared.run(&caller, &command).ok()?;
                            done.changed.then(|| (done.version.seq(), command, done.version))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            records.extend(handle.join().unwrap());
        }
    });
    records.sort_by_key(|r| r.0);
    let serial = editor(&bytes);
    for (at, command, version) in &records {
        let done = serial.run(&Caller::anonymous(), command).unwrap();
        assert!(done.changed, "seed {seed}: {command:?} changed nothing serially");
        assert_eq!(done.version.seq(), *at, "seed {seed}: commit order");
        assert_eq!(
            written(done.version.doc()),
            written(version.doc()),
            "seed {seed}: the document at version {at} after {command:?}"
        );
    }
    let stats = shared.store().stats();
    let formats = records
        .iter()
        .filter(|r| matches!(r.1, Edit::Format(_) | Edit::Rows(_)))
        .count();
    (stats.conflicts, stats.exclusive, formats)
}

#[test]
fn concurrent_table_formatting_equals_its_serial_order() {
    let mut formats = 0;
    for seed in [41u64, 42, 43] {
        formats += run(seed, 4, 6).2;
    }
    assert!(formats > 0, "the mix commits table formatting");
}

/// The lane's end gate: sixty seeds of eight writers (run with `--ignored`, in release).
#[test]
#[ignore = "the end gate's long run: cargo test --release -- --ignored"]
fn sixty_seeds_of_eight_writers_formatting_tables() {
    let (mut conflicts, mut exclusive, mut structure) = (0, 0, 0);
    for seed in 400..460u64 {
        let (c, e, s) = run(seed, 8, 6);
        conflicts += c;
        exclusive += e;
        structure += s;
    }
    println!(
        "60 seeds, 8 writers x 6 edits: {structure} table formattings and row edits committed; {conflicts} conflicts, \
         {exclusive} exclusive runs"
    );
}
