//! The random multi-writer oracle (design section 5, "enforced by the random multi-writer
//! oracle"): writers on threads run `set_text` -- tracked and untracked, on paragraphs
//! picked at random -- through one document's store; replayed serially in commit order on
//! the document as opened, every version is the same, byte for byte, as the concurrent run
//! made it, ids included. Conflicts are counted: two writers drawing text ids from one
//! part's sequence are ordered by their commits.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Seq, Snapshot, StoreConfig};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, Tracking, WordDoc};
use std::path::PathBuf;
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

fn command(rng: &mut Rng, targets: &[String], writer: u64) -> SetText {
    let target = targets[rng.below(targets.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(6)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    let policy = if rng.below(2) == 0 {
        RevisionPolicy::Untracked
    } else {
        RevisionPolicy::Tracked(Tracking {
            author: format!("Writer {writer}"),
            date: "2026-11-17T10:00:00Z".to_owned(),
        })
    };
    SetText {
        target,
        text: words.join(" "),
        policy,
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
    let mut targets: Vec<String> = body.ids.paragraphs.iter().map(|p| p.id.clone()).collect();
    targets.truncate(40);
    let mut records: Vec<(Seq, SetText, Snapshot<WordDoc>)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (shared, targets) = (&shared, &targets);
                scope.spawn(move || {
                    let mut rng = Rng(seed ^ (w + 1).wrapping_mul(0x9E37_79B9));
                    let caller = Caller::named(&format!("writer {w}"));
                    (0..per_writer)
                        .filter_map(|_| {
                            let command = command(&mut rng, targets, w);
                            let done = shared.run(&caller, &command).unwrap();
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
fn concurrent_set_texts_equal_their_serial_order() {
    let mut conflicts = 0;
    for seed in [11u64, 12] {
        conflicts += run(seed, 4, 6).0;
    }
    assert!(conflicts > 0, "six writers on one part's text ids should conflict");
}
