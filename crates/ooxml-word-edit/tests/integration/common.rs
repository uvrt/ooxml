//! Helpers the test modules share: a clock, an unlimited cache budget, a seeded random
//! source, fixtures and editors (one copy instead of one per module).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, StoreConfig};
use ooxml_word_edit::{EditError, Editor, RevisionPolicy, Tracking, WordDoc};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Monotonic(pub Instant);

impl Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

pub struct Rng(pub u64);

impl Rng {
    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % (n.max(1) as u64)) as usize
    }
}

pub fn written(doc: &WordDoc) -> Vec<(String, Arc<[u8]>)> {
    doc.package()
        .part_names()
        .into_iter()
        .map(|name| {
            let bytes = doc.package().read(&name).unwrap().unwrap();
            (name, bytes)
        })
        .collect()
}

pub fn editor(bytes: &Arc<[u8]>) -> Editor {
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
pub fn best<C: Command<WordDoc, Error = EditError>>(bytes: &Arc<[u8]>, command: &C) -> Duration {
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

/// The best of three runs of `command`, each on a fresh editor that ran `first` before
/// (opening and `first` not counted): what a session's next call costs, the document's
/// reading and id sequences already made.
pub fn best_after<F, C>(bytes: &Arc<[u8]>, first: &F, command: &C) -> Duration
where
    F: Command<WordDoc, Error = EditError>,
    C: Command<WordDoc, Error = EditError>,
{
    (0..3)
        .map(|_| {
            let editor = editor(bytes);
            editor.run(&Caller::anonymous(), first).unwrap();
            let started = Instant::now();
            editor.run(&Caller::anonymous(), command).unwrap();
            started.elapsed()
        })
        .min()
        .unwrap()
}

pub fn fixture(name: &str) -> Arc<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ooxml-oracle/fixtures")
        .join(name);
    Arc::from(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

pub fn policy(rng: &mut Rng, writer: u64) -> RevisionPolicy {
    if rng.below(3) == 0 {
        RevisionPolicy::Untracked
    } else {
        RevisionPolicy::Tracked(Tracking {
            author: format!("Writer {writer}"),
            date: "2026-11-17T10:00:00Z".to_owned(),
        })
    }
}

pub const WORDS: &[&str] = &["alpha", "beta", "gamma", "report", "Quarter", "and"];
