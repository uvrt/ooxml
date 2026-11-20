//! Undo and redo of Word edits (design section 4, Undo; lane P2c): undo and redo are
//! inverses on random sequences of the common edits, byte for byte; the id ledger only
//! grows across them (an id given is never given again); a scoped undo takes back one
//! writer's step while later steps moved its content, and refuses an entangled one; and
//! writers that undo and redo their own steps while others edit equal a serial replay in
//! commit order.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{written, Monotonic, Rng, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, Command, KeySet, Seq, Snapshot, StoreConfig, StoreError, Txn};
use ooxml_word_edit::{
    DeleteBlock, EditError, Editor, InsertParagraph, InsertText, RevisionPolicy, SetText, Tracking, WordDoc,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn editor(name: &str) -> Editor {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python/docx-agent/tests/fixtures")
        .join(name);
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(&path).unwrap());
    let package = Package::open(bytes, &Limits::default()).unwrap();
    Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(Unlimited),
    )
    .unwrap()
}

/// The body's first paragraphs' ids.
fn targets(editor: &Editor) -> Vec<String> {
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let mut ids: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .map(|p| p.id.clone())
        .collect();
    ids.truncate(30);
    ids
}

/// Every paragraph id value a version's story parts hold.
fn values(doc: &WordDoc) -> BTreeSet<u32> {
    let docx = doc.docx().unwrap();
    docx.parts
        .iter()
        .flat_map(|p| p.ids.paragraphs().iter().map(|q| q.id.clone()).collect::<Vec<_>>())
        .filter_map(|id| u32::from_str_radix(id.trim_start_matches("p:"), 16).ok())
        .collect()
}

#[derive(Debug, Clone)]
enum Edit {
    Set(SetText),
    Insert(InsertText),
    Paragraph(InsertParagraph),
    Block(DeleteBlock),
}

impl Command<WordDoc> for Edit {
    type Out = ();
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        match self {
            Edit::Set(c) => c.validate(doc),
            Edit::Insert(c) => c.validate(doc),
            Edit::Paragraph(c) => c.validate(doc),
            Edit::Block(c) => c.validate(doc),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<(), EditError> {
        match self {
            Edit::Set(c) => c.prepare(tx).map(drop),
            Edit::Insert(c) => c.prepare(tx).map(drop),
            Edit::Paragraph(c) => c.prepare(tx).map(drop),
            Edit::Block(c) => c.prepare(tx).map(drop),
        }
    }
}

fn policy(rng: &mut Rng, writer: usize) -> RevisionPolicy {
    if rng.below(2) == 0 {
        RevisionPolicy::Untracked
    } else {
        RevisionPolicy::Tracked(Tracking {
            author: format!("Writer {writer}"),
            date: "2026-11-18T10:00:00Z".to_owned(),
        })
    }
}

fn edit(rng: &mut Rng, targets: &[String], writer: usize) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    let word = ["alpha", "beta", "gamma", "delta"][rng.below(4)];
    match rng.below(6) {
        0 | 1 => Edit::Set(SetText {
            target,
            text: format!("{word} {writer}"),
            policy: policy(rng, writer),
        }),
        2 => Edit::Insert(InsertText {
            at: format!("{target}@0"),
            text: format!("{word} "),
            policy: policy(rng, writer),
        }),
        3 | 4 => Edit::Paragraph(InsertParagraph {
            anchor: target,
            after: rng.below(2) == 0,
            text: word.to_owned(),
            style: None,
            policy: policy(rng, writer),
        }),
        _ => Edit::Block(DeleteBlock {
            target,
            policy: policy(rng, writer),
        }),
    }
}

#[test]
fn undo_and_redo_are_inverses_and_keep_the_ledger() {
    for seed in [1u64, 2, 3] {
        let editor = editor("samplelib/sample-long.docx");
        let targets = targets(&editor);
        let mut rng = Rng(seed);
        let first = editor.store().snapshot();
        // the linear history's states, and where the head is in it
        let mut states = vec![written(first.doc())];
        let mut at = 0usize;
        let mut ever = values(first.doc());
        for step in 0..40 {
            match rng.below(5) {
                0..=2 => {
                    let Ok(done) = editor.run(&Caller::named("w"), &edit(&mut rng, &targets, 0)) else {
                        continue; // a target an earlier step deleted
                    };
                    if done.changed {
                        states.truncate(at + 1);
                        states.push(written(done.version.doc()));
                        at += 1;
                    }
                }
                3 => match editor.store().undo() {
                    Ok(_) => at -= 1,
                    Err(StoreError::NothingToUndo) => assert_eq!(at, 0),
                    Err(other) => panic!("{other}"),
                },
                _ => match editor.store().redo() {
                    Ok(_) => at += 1,
                    Err(StoreError::NothingToRedo) => assert_eq!(at + 1, states.len()),
                    Err(other) => panic!("{other}"),
                },
            }
            let head = editor.store().snapshot();
            assert_eq!(written(head.doc()), states[at], "seed {seed} step {step}");
            ever.extend(values(head.doc()));
            let missing: Vec<u32> = ever
                .iter()
                .copied()
                .filter(|v| !head.doc().ids().contains(*v))
                .collect();
            assert!(
                missing.is_empty(),
                "seed {seed} step {step}: the ledger lost {missing:X?}"
            );
        }
    }
}

#[test]
fn a_scoped_undo_takes_back_a_step_later_steps_moved() {
    let editor = editor("samplelib/sample-long.docx");
    let targets = targets(&editor);
    let (one, two) = (Caller::named("one"), Caller::named("two"));
    // the first edit stamps the document's blocks with ids (the whole part): made by
    // another writer, so the steps below are splices of single blocks
    let stamp = Edit::Set(SetText {
        target: targets[20].clone(),
        text: "Stamped".to_owned(),
        policy: RevisionPolicy::Untracked,
    });
    editor.run(&Caller::named("zero"), &stamp).unwrap();
    let set = Edit::Set(SetText {
        target: targets[10].clone(),
        text: "Written by one".to_owned(),
        policy: RevisionPolicy::Untracked,
    });
    let mine = editor.run(&one, &set).unwrap().version;
    // another writer inserts a paragraph before it: its place moves
    let insert = Edit::Paragraph(InsertParagraph {
        anchor: targets[2].clone(),
        after: false,
        text: "New".to_owned(),
        style: None,
        policy: RevisionPolicy::Untracked,
    });
    let theirs = editor.run(&two, &insert).unwrap().version;
    let undone = editor.store().undo_step(mine.seq(), &KeySet::new()).unwrap();
    // their paragraph stays, the text written by one is gone (ids drawn stay drawn, so the
    // bytes are not those of their step alone on the document as opened)
    let body =
        |doc: &WordDoc| String::from_utf8(doc.package().read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    let after = body(undone.version.doc());
    assert!(after.contains(">New<"), "theirs gone");
    assert!(!after.contains("Written by one"), "mine kept");
    // and the step comes back on top of theirs
    let redone = editor.store().redo_step(mine.seq()).unwrap();
    assert_eq!(redone.version.seq(), Seq(theirs.seq().0 + 2));
    assert_eq!(written(redone.version.doc()), written(theirs.doc()));
    // a step a later one read is entangled: refused, nothing changed
    let over = Edit::Set(SetText {
        target: targets[10].clone(),
        text: "Written over".to_owned(),
        policy: RevisionPolicy::Untracked,
    });
    let head = editor.run(&two, &over).unwrap().version;
    let latest = editor.store().latest_step_by("one").unwrap();
    assert!(matches!(
        editor.store().undo_step(latest, &KeySet::new()),
        Err(StoreError::Entangled { .. })
    ));
    assert!(Arc::ptr_eq(&editor.store().snapshot(), &head));
}

/// What a writer did, as committed.
#[derive(Debug, Clone)]
enum Op {
    Edit(Edit),
    Undo(Seq),
    Redo(Seq),
}

fn writers_run(seed: u64, writers: usize, per_writer: usize) -> usize {
    let shared = editor("samplelib/sample-long.docx");
    let targets = targets(&shared);
    let mut records: Vec<(Seq, Op, Snapshot<WordDoc>)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (shared, targets) = (&shared, &targets);
                scope.spawn(move || {
                    let mut rng = Rng(seed ^ (w as u64 + 1).wrapping_mul(0x9E37_79B9));
                    let name = format!("writer {w}");
                    let caller = Caller::named(&name);
                    let mut undone: Vec<Seq> = Vec::new();
                    let mut out = Vec::new();
                    for _ in 0..per_writer {
                        let roll = rng.below(6);
                        let latest = shared.store().latest_step_by(&name);
                        let record = match (roll, latest, undone.last()) {
                            (4, Some(step), _) => shared.store().undo_step(step, &KeySet::new()).ok().map(|o| {
                                undone.push(step);
                                (o.version.seq(), Op::Undo(step), o.version)
                            }),
                            (5, _, Some(&step)) => shared.store().redo_step(step).ok().map(|o| {
                                undone.pop();
                                (o.version.seq(), Op::Redo(step), o.version)
                            }),
                            _ => {
                                let command = edit(&mut rng, targets, w);
                                shared
                                    .run(&caller, &command)
                                    .ok()
                                    .filter(|d| d.changed)
                                    .map(|d| (d.version.seq(), Op::Edit(command), d.version))
                            }
                        };
                        out.extend(record);
                    }
                    out
                })
            })
            .collect();
        for handle in handles {
            records.extend(handle.join().unwrap());
        }
    });
    records.sort_by_key(|r| r.0);
    let serial = editor("samplelib/sample-long.docx");
    for (at, op, version) in &records {
        let made = match op {
            Op::Edit(command) => serial.run(&Caller::anonymous(), command).unwrap().version,
            Op::Undo(step) => serial.store().undo_step(*step, &KeySet::new()).unwrap().version,
            Op::Redo(step) => serial.store().redo_step(*step).unwrap().version,
        };
        assert_eq!(made.seq(), *at, "seed {seed}: commit order");
        assert_eq!(
            written(made.doc()),
            written(version.doc()),
            "seed {seed}: version {at} ({op:?})"
        );
    }
    records.iter().filter(|r| !matches!(r.1, Op::Edit(_))).count()
}

#[test]
fn writers_undoing_their_own_steps_equal_their_serial_order() {
    let history_ops: usize = [21u64, 22].iter().map(|&seed| writers_run(seed, 4, 8)).sum();
    assert!(history_ops > 0, "some undo or redo should have committed");
}

/// The lane's end gate: sixty seeds of eight writers (run with `--ignored`, in release).
#[test]
#[ignore = "the end gate's long run: cargo test --release -- --ignored"]
fn sixty_seeds_of_eight_writers_with_undo() {
    let history_ops: usize = (200..260u64).map(|seed| writers_run(seed, 8, 6)).sum();
    println!("60 seeds, 8 writers x 6 ops: {history_ops} undos and redos committed");
}
