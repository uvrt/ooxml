//! The random multi-writer oracle (design section 5, "enforced by the random multi-writer
//! oracle"): writers on threads run the common edits -- `set_text`, `insert_text`,
//! `delete_text` and `replace`, tracked and untracked, on paragraphs picked at random --
//! through one document's store; replayed serially in commit order on
//! the document as opened, every version is the same, byte for byte, as the concurrent run
//! made it, ids included. Conflicts are counted: two writers drawing text ids from one
//! part's sequence are ordered by their commits.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Seq, Snapshot, StoreConfig};
use ooxml_store::{Command, Txn};
use ooxml_word_edit::{
    DeleteText, EditError, Editor, InsertText, Keep, ReplaceText, RevisionPolicy, SetText, Tracking, WordDoc,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A table's id and its rows' places and ids.
type TableRows = (String, Vec<(Vec<u32>, String)>);

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

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % (n.max(1) as u64)) as usize
    }
}

fn fixture(name: &str) -> Arc<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python/docx-agent/tests/fixtures")
        .join(name);
    Arc::from(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
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

const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "report", "Quarter", "\t", "and",
];

/// One of the common edits.
#[derive(Debug, Clone)]
enum Edit {
    Set(SetText),
    Insert(InsertText),
    Delete(DeleteText),
    Replace(ReplaceText),
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
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        Ok(match self {
            Edit::Set(c) => c.prepare(tx)?.changed,
            Edit::Insert(c) => c.prepare(tx).map(|_| true)?,
            Edit::Delete(c) => c.prepare(tx).map(|_| true)?,
            Edit::Replace(c) => c.prepare(tx).map(|_| true)?,
        })
    }
}

fn command(rng: &mut Rng, targets: &[String], writer: u64) -> SetText {
    let target = targets[rng.below(targets.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(6)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    SetText {
        target,
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

/// A random common edit (positions within the first characters, so they stay in range).
fn edit(rng: &mut Rng, targets: &[String], writer: u64) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    match rng.below(8) {
        0..=3 => Edit::Set(command(rng, targets, writer)),
        4 | 5 => Edit::Insert(InsertText {
            at: format!("{target}@{}", rng.below(3)),
            text: format!("{} ", WORDS[rng.below(WORDS.len())]),
            policy: policy(rng, writer),
        }),
        6 => Edit::Delete(DeleteText {
            range: format!("{target}@0:{}", 1 + rng.below(2)),
            policy: policy(rng, writer),
        }),
        _ => Edit::Replace(ReplaceText {
            find: WORDS[rng.below(6)].to_owned(),
            with: "omega".to_owned(),
            case: rng.below(2) == 0,
            count: Some(1),
            keep: Keep::First,
            policy: policy(rng, writer),
        }),
    }
}

/// The parts a version writes, as bytes, with the archive's order.
fn written(doc: &WordDoc) -> Vec<(String, Arc<[u8]>)> {
    doc.package()
        .part_names()
        .into_iter()
        .map(|name| {
            let bytes = doc.package().read(&name).unwrap().unwrap();
            (name, bytes)
        })
        .collect()
}

fn run(seed: u64, writers: u64, per_writer: usize) -> (u64, u64) {
    let bytes = fixture("samplelib/sample-long.docx");
    let shared = editor(&bytes);
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let body = docx.body().unwrap();
    let mut targets: Vec<String> = body.ids.paragraphs().iter().map(|p| p.id.clone()).collect();
    targets.truncate(40);
    let mut records: Vec<(Seq, Edit, Snapshot<WordDoc>)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (shared, targets) = (&shared, &targets);
                scope.spawn(move || {
                    let mut rng = Rng(seed ^ (w + 1).wrapping_mul(0x9E37_79B9));
                    let caller = Caller::named(&format!("writer {w}"));
                    (0..per_writer)
                        .filter_map(|_| {
                            let command = edit(&mut rng, targets, w);
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
    (stats.conflicts, stats.exclusive)
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
            target: id.clone(),
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
        target: "p:7FFFFFFE".to_owned(),
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
    assert!(conflicts > 0);
}
