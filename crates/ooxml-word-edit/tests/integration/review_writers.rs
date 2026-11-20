//! The random multi-writer oracle for the review edits (lane P4a): writers on threads mix
//! tracked and untracked `set_text` and `insert_text` with accepting and rejecting changes
//! (by author, by kind, everything) and resolving, reopening and deleting comments, through
//! one document's store; replayed serially in commit order on the document as opened,
//! every version is the same, byte for byte, as the concurrent run made it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, policy, written, Rng, WORDS};
use ooxml_store::{Caller, Command, Seq, Snapshot, Txn};
use ooxml_word_edit::{CommentEdit, CommentEdits, EditError, InsertText, Review, Selection, SetText, WordDoc};
use ooxml_word_model::docx::comments::comments;

/// One edit of the mix.
#[derive(Debug, Clone)]
enum Edit {
    Set(SetText),
    Insert(InsertText),
    Review(Review),
    Comments(CommentEdits),
}

impl Command<WordDoc> for Edit {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        match self {
            Edit::Set(c) => c.validate(doc),
            Edit::Insert(c) => c.validate(doc),
            Edit::Review(c) => c.validate(doc),
            Edit::Comments(c) => c.validate(doc),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        Ok(match self {
            Edit::Set(c) => c.prepare(tx)?.changed,
            Edit::Insert(c) => c.prepare(tx).map(|_| true)?,
            Edit::Review(c) => c.prepare(tx)?.records > 0,
            Edit::Comments(c) => c.prepare(tx)?.iter().any(|o| o.changed),
        })
    }
}

/// A random edit of the mix; `threads`, the comments' ids as opened.
fn edit(rng: &mut Rng, targets: &[String], threads: &[String], writer: u64) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(4)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    match rng.below(10) {
        0..=2 => Edit::Set(SetText {
            target,
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        3 | 4 => Edit::Insert(InsertText {
            at: format!("{target}@{}", rng.below(3)),
            text: format!("{} ", words.join(" ")),
            policy: policy(rng, writer),
        }),
        5 | 6 => Edit::Review(Review {
            accept: rng.below(2) == 0,
            selection: Selection::Matching {
                author: Some(format!("Writer {}", rng.below(4))),
                kind: None,
                within: None,
            },
        }),
        7 => Edit::Review(Review {
            accept: rng.below(2) == 0,
            selection: if rng.below(4) == 0 {
                Selection::All
            } else {
                Selection::Matching {
                    author: None,
                    kind: Some(["insertion", "deletion", "replacement"][rng.below(3)].to_owned()),
                    within: Some(target),
                }
            },
        }),
        _ => Edit::Comments(CommentEdits {
            edit: [CommentEdit::Resolve, CommentEdit::Reopen, CommentEdit::Delete][rng.below(3)],
            ids: vec![threads[rng.below(threads.len())].clone()],
        }),
    }
}

fn run(seed: u64, writers: u64, per_writer: usize) -> (u64, u64, usize, usize) {
    let bytes = fixture("generated/markdown/review.docx");
    let shared = editor(&bytes);
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let targets: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let threads: Vec<String> = comments(&docx).into_iter().map(|c| c.id).collect();
    let mut records: Vec<(Seq, Edit, Snapshot<WordDoc>)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (shared, targets, threads) = (&shared, &targets, &threads);
                scope.spawn(move || {
                    let mut rng = Rng(seed ^ (w + 1).wrapping_mul(0x9E37_79B9));
                    let caller = Caller::named(&format!("writer {w}"));
                    (0..per_writer)
                        .filter_map(|_| {
                            let command = edit(&mut rng, targets, threads, w);
                            // a refusal (a paragraph or comment a concurrent edit removed)
                            // changes nothing
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
            "seed {seed}: the document at version {at}"
        );
    }
    let stats = shared.store().stats();
    let reviews = records.iter().filter(|r| matches!(r.1, Edit::Review(_))).count();
    let comment_edits = records.iter().filter(|r| matches!(r.1, Edit::Comments(_))).count();
    (stats.conflicts, stats.exclusive, reviews, comment_edits)
}

#[test]
fn concurrent_reviews_equal_their_serial_order() {
    let (mut reviews, mut comment_edits) = (0, 0);
    for seed in [21u64, 22, 23] {
        let (_, _, r, c) = run(seed, 4, 6);
        reviews += r;
        comment_edits += c;
    }
    assert!(
        reviews > 0 && comment_edits > 0,
        "the mix settles changes and edits comments"
    );
}

/// The lane's end gate: sixty seeds of eight writers (run with `--ignored`, in release).
#[test]
#[ignore = "the end gate's long run: cargo test --release -- --ignored"]
fn sixty_seeds_of_eight_writers_reviewing() {
    let (mut conflicts, mut exclusive, mut reviews, mut comment_edits) = (0, 0, 0, 0);
    for seed in 200..260u64 {
        let (c, e, r, m) = run(seed, 8, 6);
        conflicts += c;
        exclusive += e;
        reviews += r;
        comment_edits += m;
    }
    println!(
        "60 seeds, 8 writers x 6 edits: {reviews} reviews and {comment_edits} comment edits committed; \
         {conflicts} conflicts, {exclusive} exclusive runs"
    );
}
