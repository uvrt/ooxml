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

/// Each block id a reading of `bytes` by place gave ([`by_place`]) (positional `p@body/3`,
/// `t@body/0`, `t:X/r1`, a repeat `p:X#1`) that the document opened -- every block stamped
/// in memory -- names otherwise: the old id and the id it has now.
pub fn stamped_ids(bytes: &Arc<[u8]>) -> Vec<(String, String)> {
    use ooxml_word_model::ids::{story_name, PartIds};
    let raw = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    let (word, _) = ooxml_word_model::WordPackage::open(raw.clone());
    let main = ooxml_word_model::parts::main_part(&raw).unwrap().unwrap();
    let names = |ids: PartIds| {
        let mut out: Vec<String> = ids.paragraphs().iter().map(|p| p.id).collect();
        for table in ids.tables().iter() {
            out.push(table.id.clone());
            out.extend(table.rows.iter().map(|(_, r)| r.clone()));
        }
        out
    };
    let mut renamed = Vec::new();
    for part in ooxml_word_model::parts::parts(&raw).unwrap() {
        let story = story_name(&main, &part);
        let (Some(old), Some(new)) = (raw.tree(&part).unwrap(), word.tree(&part).unwrap()) else {
            continue;
        };
        let pairs = by_place(&story, old.root())
            .into_iter()
            .zip(names(PartIds::new(&story, new.root())));
        renamed.extend(pairs.filter(|(a, b)| a != b));
    }
    // the longest first: `p@body/12` before `p@body/1`
    renamed.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    renamed
}

/// `text` (an address, a list of options) with every old id of `bytes` ([`stamped_ids`])
/// as the document names it now.
pub fn now(bytes: &Arc<[u8]>, text: &str) -> String {
    let renamed = stamped_ids(bytes);
    let mut out = String::new();
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for (old, new) in &renamed {
            if let Some(after) = rest.strip_prefix(old.as_str()) {
                if !after.starts_with(|c: char| c.is_ascii_digit()) {
                    out.push_str(new);
                    rest = after;
                    continue 'scan;
                }
            }
        }
        let mut chars = rest.chars();
        out.extend(chars.next());
        rest = chars.as_str();
    }
    out
}

/// The names a reading of the part `root` (story `story`) gave its blocks by place before
/// every block had an id of its own (stamp on open; the index no longer gives them): what
/// this suite's data was written with. In [`ooxml_word_model::ids::PartIds`]' order: the
/// paragraphs, then each table and its rows.
fn by_place(story: &str, root: &ooxml_xml::Element) -> Vec<String> {
    use ooxml_word_model::ids::{valid_long_hex, PartIds};
    use std::collections::HashMap;
    let ids = PartIds::new(story, root);
    let prefix = if story == "body" {
        String::new()
    } else {
        format!("{story}/")
    };
    let raw_of = |name: &str| {
        name.rsplit_once(':')
            .map(|(_, raw)| raw.to_owned())
            .filter(|r| !r.is_empty())
    };
    // repeats were counted by spelling, paragraphs and rows together in document order
    let mut blocks: Vec<(Box<[u32]>, Option<String>)> = ids.paragraphs().iter().map(|p| (p.path, p.raw)).collect();
    for table in ids.tables().iter() {
        blocks.extend(table.rows.iter().map(|(path, name)| (path.clone(), raw_of(name))));
    }
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen: HashMap<String, usize> = HashMap::new();
    let repeat: HashMap<Box<[u32]>, usize> = blocks
        .into_iter()
        .map(|(path, raw)| {
            let k = raw.map_or(0, |r| {
                let n = seen.entry(r.to_uppercase()).or_insert(0);
                *n += 1;
                *n - 1
            });
            (path, k)
        })
        .collect();
    let at = |path: &[u32]| repeat.get(path).copied().unwrap_or(0);
    let mut out: Vec<String> = ids
        .paragraphs()
        .iter()
        .map(|p| match (&p.raw, at(&p.path)) {
            (None, _) => format!("p@{story}/{}", p.index),
            (Some(raw), 0) => format!("{prefix}p:{raw}"),
            (Some(raw), k) => format!("{prefix}p:{raw}#{k}"),
        })
        .collect();
    for (n, table) in ids.tables().iter().enumerate() {
        let good = |path: &[u32], name: &str| raw_of(name).filter(|r| valid_long_hex(r) && at(path) == 0);
        let id = match table.rows.first().and_then(|(path, name)| good(path, name)) {
            Some(raw) => format!("{prefix}t:{raw}"),
            None => format!("t@{story}/{n}"),
        };
        out.push(id.clone());
        for (k, (path, name)) in table.rows.iter().enumerate() {
            out.push(match good(path, name) {
                Some(raw) => format!("{prefix}tr:{raw}"),
                None => format!("{id}/r{k}"),
            });
        }
    }
    out
}
