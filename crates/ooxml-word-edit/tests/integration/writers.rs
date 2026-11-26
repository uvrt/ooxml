//! The random multi-writer oracle (design section 5, "enforced by the random multi-writer
//! oracle"): writers on threads run the common edits -- `set_text`, `insert_text`,
//! `delete_text` and `replace`, tracked and untracked, on paragraphs picked at random --
//! through one document's store; replayed serially in commit order on
//! the document as opened, every version is the same, byte for byte, as the concurrent run
//! made it, ids included. Conflicts are counted: two writers drawing text ids from one
//! part's sequence are ordered by their commits.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, written, Rng};
use ooxml_store::{Batch, Command, Txn};
use ooxml_store::{Caller, Seq, Snapshot};
use ooxml_word_edit::find::Literal;
use ooxml_word_edit::{
    CellWrite, DeleteBlock, DeleteText, EditError, InsertMarkdown, InsertParagraph, InsertText, Keep, MarkdownOptions,
    ReplaceText, RevisionPolicy, SetCells, SetParagraphStyle, SetStyledText, SetText, Tracking, WordDoc, WriteMap,
};
use std::sync::Arc;

/// A table's id and its rows' places and ids.
type TableRows = (String, Vec<(Vec<u32>, String)>);

const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "report", "Quarter", "\t", "and", "fin :", "«oui»", "quoi?",
];

/// One of the common edits.
#[derive(Debug, Clone)]
enum Edit {
    Set(SetText),
    Insert(InsertText),
    Delete(DeleteText),
    Replace(ReplaceText),
    Paragraph(InsertParagraph),
    Style(SetParagraphStyle),
    Block(DeleteBlock),
    Cells(SetCells),
    Styled(SetStyledText),
    Markdown(InsertMarkdown),
    /// Several edits as one transaction and one step (a host's batch, design phase 3 §4):
    /// its changes a sequence, moved past what was published since as one.
    Batch(Batch<Edit>),
}

impl Command<WordDoc> for Edit {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        match self {
            Edit::Set(c) => c.validate(doc),
            Edit::Insert(c) => c.validate(doc),
            Edit::Delete(c) => c.validate(doc),
            Edit::Replace(c) => c.validate(doc),
            Edit::Paragraph(c) => c.validate(doc),
            Edit::Style(c) => c.validate(doc),
            Edit::Block(c) => c.validate(doc),
            Edit::Cells(c) => c.validate(doc),
            Edit::Styled(c) => c.validate(doc),
            Edit::Markdown(c) => c.validate(doc),
            Edit::Batch(c) => c.validate(doc),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        Ok(match self {
            Edit::Set(c) => c.prepare(tx)?.changed,
            Edit::Insert(c) => c.prepare(tx).map(|_| true)?,
            Edit::Delete(c) => c.prepare(tx).map(|_| true)?,
            Edit::Replace(c) => c.prepare(tx).map(|_| true)?,
            Edit::Paragraph(c) => c.prepare(tx).map(|_| true)?,
            Edit::Style(c) => c.prepare(tx).map(|_| true)?,
            Edit::Block(c) => c.prepare(tx).map(|_| true)?,
            Edit::Cells(c) => c.prepare(tx).map(|_| true)?,
            Edit::Styled(c) => c.prepare(tx).map(|_| true)?,
            Edit::Markdown(c) => c.prepare(tx)?.changed,
            Edit::Batch(c) => c.prepare(tx)?.into_iter().any(|out| out.unwrap_or(false)),
        })
    }
}

fn command(rng: &mut Rng, targets: &[String], writer: u64) -> SetText {
    let target = targets[rng.below(targets.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(6)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    SetText {
        target: ooxml_word_model::address::Address::from(target),
        text: words.join(" "),
        policy: policy(rng, writer),
    }
}

fn policy(rng: &mut Rng, writer: u64) -> RevisionPolicy {
    if rng.below(2) == 0 {
        RevisionPolicy::Untracked
    } else {
        RevisionPolicy::Tracked(Tracking {
            author: format!("Writer {writer}"),
            date: "2026-11-17T10:00:00Z".to_owned(),
        })
    }
}

/// Two lines into a cell of the first table.
fn cells(rng: &mut Rng, writer: u64) -> SetCells {
    SetCells {
        table: ooxml_word_model::address::Address::from("t@body/0".to_owned()),
        writes: vec![CellWrite::at(
            rng.below(2),
            rng.below(2),
            format!("{}\n{}", WORDS[rng.below(WORDS.len())], WORDS[rng.below(4)]),
        )],
        stamping: false,
        policy: policy(rng, writer),
        accepted: None,
    }
}

/// A block deletion's target: a content control now and then (of `controls`), the first
/// table now and then, else `target`.
fn block_target(rng: &mut Rng, target: String, controls: &[String]) -> String {
    if !controls.is_empty() && rng.below(3) == 0 {
        controls[rng.below(controls.len())].clone()
    } else if rng.below(4) == 0 {
        "t@body/0".to_owned()
    } else {
        target
    }
}

/// A random common edit (positions within the first characters, so they stay in range);
/// a block deletion takes one of `controls` (content controls) now and then.
fn edit(rng: &mut Rng, (targets, controls): (&[String], &[String]), writer: u64) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    match rng.below(15) {
        0..=3 => Edit::Set(command(rng, targets, writer)),
        13 => Edit::Styled(SetStyledText {
            target: ooxml_word_model::address::Address::from(target),
            text: format!(
                "{} **{}** *{}*",
                WORDS[rng.below(WORDS.len())],
                WORDS[rng.below(WORDS.len())],
                WORDS[rng.below(4)]
            ),
            paragraph_style: None,
            policy: policy(rng, writer),
        }),
        14 => Edit::Markdown(InsertMarkdown {
            markdown: format!(
                "## {}\n\n- {} *{}*\n- {}\n\n| {} |\n|---|\n| {} |\n",
                WORDS[rng.below(WORDS.len())],
                WORDS[rng.below(WORDS.len())],
                WORDS[rng.below(4)],
                WORDS[rng.below(WORDS.len())],
                WORDS[rng.below(4)],
                WORDS[rng.below(WORDS.len())]
            ),
            at: ooxml_word_model::address::Place::parse(
                &[format!("after:{target}"), format!("before:{target}"), "end".to_owned()][rng.below(3)],
            ),
            style_map: WriteMap::default(),
            options: MarkdownOptions::default(),
            policy: policy(rng, writer),
        }),
        8 => Edit::Paragraph(InsertParagraph {
            anchor: target,
            after: rng.below(2) == 0,
            text: WORDS[rng.below(WORDS.len())].to_owned(),
            style: None,
            policy: policy(rng, writer),
        }),
        12 => Edit::Cells(cells(rng, writer)),
        11 => Edit::Block(DeleteBlock {
            target: ooxml_word_model::address::Address::from(block_target(rng, target, controls)),
            policy: policy(rng, writer),
        }),
        10 => Edit::Style(SetParagraphStyle {
            target: ooxml_word_model::address::Address::from(target),
            style: [Some("Heading 1"), Some("Quote"), Some("List Bullet 2"), None][rng.below(4)].map(str::to_owned),
            policy: policy(rng, writer),
        }),
        9 => {
            let mut lines = command(rng, targets, writer);
            lines.text = format!("{}\n{}", lines.text, WORDS[rng.below(WORDS.len())]);
            Edit::Set(lines)
        }
        4 | 5 => Edit::Insert(InsertText {
            at: format!("{target}@{}", rng.below(3)).into(),
            text: format!("{} ", WORDS[rng.below(WORDS.len())]),
            policy: policy(rng, writer),
        }),
        6 => Edit::Delete(DeleteText {
            range: format!("{target}@0:{}", 1 + rng.below(2)).into(),
            policy: policy(rng, writer),
        }),
        _ => Edit::Replace(ReplaceText {
            pattern: Arc::new(Literal::new(WORDS[rng.below(6)], rng.below(2) == 0, false, "omega").unwrap()),
            across: false,
            within: None,
            // the first match, or every one (a find read by condition: design section 5)
            count: (rng.below(2) == 0).then_some(1),
            keep: Keep::First,
            policy: policy(rng, writer),
        }),
    }
}

fn run(seed: u64, writers: u64, per_writer: usize) -> (u64, u64) {
    run_on("samplelib/sample-long.docx", seed, writers, per_writer)
}

/// A batch of two to five random common edits, as a host's batch runs them: one transaction.
fn batch(rng: &mut Rng, targets: (&[String], &[String]), writer: u64) -> Edit {
    let ops = (0..2 + rng.below(4)).map(|_| edit(rng, targets, writer)).collect();
    Edit::Batch(Batch::new(ops))
}

fn run_on(name: &str, seed: u64, writers: u64, per_writer: usize) -> (u64, u64) {
    run_with(name, seed, writers, per_writer, false).0
}

/// [`run_on`], where `batches`: every other writer runs batches of edits; what the store
/// counted (conflicts and exclusive runs, then rebases).
fn run_with(name: &str, seed: u64, writers: u64, per_writer: usize, batches: bool) -> ((u64, u64), u64) {
    let bytes = fixture(name);
    let shared = editor(&bytes);
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let body = docx.body().unwrap();
    let mut targets: Vec<String> = body.ids.paragraphs().iter().map(|p| p.id.clone()).collect();
    targets.truncate(40);
    let controls: Vec<String> = ooxml_word_model::docx::controls::controls(&docx)
        .into_iter()
        .map(|c| c.id)
        .collect();
    let mut records: Vec<(Seq, Edit, Snapshot<WordDoc>)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (shared, targets, controls) = (&shared, &targets, &controls);
                scope.spawn(move || {
                    let mut rng = Rng(seed ^ (w + 1).wrapping_mul(0x9E37_79B9));
                    let caller = Caller::named(&format!("writer {w}"));
                    (0..per_writer)
                        .filter_map(|_| {
                            let command = if batches && w % 2 == 1 {
                                batch(&mut rng, (targets, controls), w)
                            } else {
                                edit(&mut rng, (targets, controls), w)
                            };
                            // a refusal (a range a concurrent edit shortened) changes nothing
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
    let mut seq = Seq(0);
    for (at, command, version) in &records {
        // a no-op in the serial run (the text already that) makes no version: skip numbers
        let done = serial.run(&Caller::anonymous(), command).unwrap();
        assert!(done.changed, "seed {seed}: {command:?} changed nothing serially");
        seq = done.version.seq();
        assert_eq!(seq, *at, "seed {seed}: commit order");
        assert_eq!(
            written(done.version.doc()),
            written(version.doc()),
            "seed {seed}: the document at version {at}"
        );
    }
    assert_eq!(seq, records.last().map_or(Seq(0), |r| r.0));
    let stats = shared.store().stats();
    ((stats.conflicts, stats.exclusive), stats.rebased)
}

#[test]
fn concurrent_edits_equal_their_serial_order() {
    let mut conflicts = 0;
    for seed in [11u64, 12] {
        conflicts += run(seed, 4, 6).0;
    }
    assert!(conflicts > 0, "writers stamping one document should conflict");
}

#[test]
fn concurrent_batches_beside_writers_equal_their_serial_order() {
    // batches' change sequences moved past the writers' commits as one (design phase 3 §4)
    let mut rebased = 0;
    for seed in [41u64, 42] {
        rebased += run_with("samplelib/sample-long.docx", seed, 4, 4, true).1;
    }
    run_with("wordto/sample-with-table.docx", 43, 4, 4, true);
    assert!(rebased > 0, "batches beside writers should rebase");
}

#[test]
fn concurrent_control_deletes_equal_their_serial_order() {
    for seed in [31u64, 32, 33] {
        run_on("generated/markdown/constructs.docx", seed, 4, 6);
        run_on("generated/ids-and-markup.docx", seed, 4, 6);
    }
}

#[test]
fn concurrent_edits_on_a_table_equal_their_serial_order() {
    for seed in [21u64, 22] {
        run_on("wordto/sample-with-table.docx", seed, 4, 6);
    }
}

#[test]
fn memoised_problems_equal_the_full_check_version_by_version() {
    let bytes = fixture("generated/charts/chart-places.docx");
    let editor = editor(&bytes);
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let targets: Vec<String> = docx
        .parts
        .iter()
        .flat_map(|p| p.ids.paragraphs().iter().map(|q| q.id.clone()))
        .collect();
    let mut rng = Rng(7);
    for k in 0..6 {
        let done = editor
            .run(&Caller::anonymous(), &command(&mut rng, &targets, k))
            .unwrap();
        let doc = done.version.doc();
        assert_eq!(
            ooxml_word_edit::validate::problems(doc).unwrap(),
            ooxml_word_edit::validate::full_problems(doc).unwrap()
        );
    }
}

#[test]
fn a_batch_is_one_step_and_an_atomic_batch_refuses_whole() {
    let bytes = fixture("samplelib/sample-simple.docx");
    let (batched, single) = (editor(&bytes), editor(&bytes));
    let docx = batched.store().snapshot().doc().docx().unwrap();
    let ids: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .map(|p| p.id.clone())
        .take(3)
        .collect();
    let ops: Vec<SetText> = ids
        .iter()
        .map(|id| SetText {
            target: ooxml_word_model::address::Address::from(id.clone()),
            text: format!("Text of {id}"),
            policy: RevisionPolicy::Untracked,
        })
        .collect();
    let done = batched
        .run(&Caller::anonymous(), &ooxml_store::Batch::new(ops.clone()))
        .unwrap();
    assert_eq!(done.version.seq(), Seq(1), "one version for the batch");
    assert!(done.out.iter().all(Result::is_ok));
    let mut last = None;
    for op in &ops {
        last = Some(single.run(&Caller::anonymous(), op).unwrap().version);
    }
    assert_eq!(written(done.version.doc()), written(last.unwrap().doc()));
    // an atomic batch with an op that refuses changes nothing
    let mut bad = ops.clone();
    bad.push(SetText {
        target: ooxml_word_model::address::Address::from("p:7FFFFFFE".to_owned()),
        text: "x".to_owned(),
        policy: RevisionPolicy::Untracked,
    });
    assert!(batched
        .run(&Caller::anonymous(), &ooxml_store::Batch::atomic(bad.clone()))
        .is_err());
    assert_eq!(batched.store().snapshot().seq(), Seq(1));
    // not atomic: the refused op is reported, the others commit
    let done = batched
        .run(&Caller::anonymous(), &ooxml_store::Batch::new(bad))
        .unwrap();
    assert!(done.out.last().unwrap().is_err());
}

#[test]
fn each_version_reads_as_a_fresh_reading_does() {
    let bytes = fixture("generated/charts/chart-places.docx");
    let editor = editor(&bytes);
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let targets: Vec<String> = docx
        .parts
        .iter()
        .flat_map(|p| p.ids.paragraphs().iter().map(|q| q.id.clone()))
        .collect();
    let mut rng = Rng(5);
    for k in 0..5 {
        let done = editor
            .run(&Caller::anonymous(), &command(&mut rng, &targets, k))
            .unwrap();
        let kept = done.version.doc().docx().unwrap();
        let fresh = ooxml_word_model::docx::Docx::open(done.version.doc().package()).unwrap();
        for (a, b) in kept.parts.iter().zip(&fresh.parts) {
            let ids = |p: &ooxml_word_model::docx::StoryPart| -> Vec<(String, Vec<u32>)> {
                p.ids
                    .paragraphs()
                    .iter()
                    .map(|q| (q.id.clone(), q.path.to_vec()))
                    .collect()
            };
            assert_eq!(ids(a), ids(b));
            let rows = |p: &ooxml_word_model::docx::StoryPart| -> Vec<TableRows> {
                p.ids
                    .tables()
                    .iter()
                    .map(|t| {
                        (
                            t.id.clone(),
                            t.rows.iter().map(|(x, y)| (x.to_vec(), y.clone())).collect(),
                        )
                    })
                    .collect()
            };
            assert_eq!(rows(a), rows(b));
        }
    }
}

/// The lane's end gate: sixty seeds of eight writers (run with `--ignored`, in release).
#[test]
#[ignore = "the end gate's long run: cargo test --release -- --ignored"]
fn sixty_seeds_of_eight_writers() {
    let (mut conflicts, mut exclusive) = (0, 0);
    for seed in 100..160u64 {
        let (c, e) = run(seed, 8, 6);
        conflicts += c;
        exclusive += e;
    }
    println!("60 seeds, 8 writers x 6 edits: {conflicts} conflicts, {exclusive} exclusive runs");
    let (mut c_conflicts, mut c_exclusive) = (0, 0);
    for seed in 100..160u64 {
        let (c, e) = run_on("generated/markdown/constructs.docx", seed, 8, 6);
        c_conflicts += c;
        c_exclusive += e;
    }
    println!(
        "60 seeds with content controls, 8 writers x 6 edits: {c_conflicts} conflicts, {c_exclusive} exclusive runs"
    );
    assert!(conflicts > 0);
}
