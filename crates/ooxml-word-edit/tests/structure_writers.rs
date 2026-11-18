//! The random multi-writer oracle for the structure edits (lane P4b): writers on threads
//! mix tracked and untracked `set_text` and `insert_text` with bookmarks added and removed,
//! hyperlinks, notes inserted, edited and deleted, page setup, section breaks, headers and
//! footers and blocks moved, through one document's store; replayed serially in commit
//! order on the document as opened, every version is the same, byte for byte, as the
//! concurrent run made it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, Seq, Snapshot, StoreConfig, Txn};
use ooxml_word_edit::{
    AddBookmark, AddHyperlink, Day, DeleteNote, EditError, EditNote, Editor, InsertCaption, InsertField, InsertNote,
    InsertSectionBreak, InsertText, UpdateFields,
    MoveBlocks, Moving, NoteKind, RemoveBookmark, RevisionPolicy, SectionValues, SetSection, SetStoryText, SetText,
    Tracking, WordDoc,
};
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

/// One edit of the mix.
#[derive(Debug, Clone)]
enum Edit {
    Set(SetText),
    Insert(InsertText),
    Bookmark(AddBookmark),
    Unmark(RemoveBookmark),
    Link(AddHyperlink),
    Note(InsertNote),
    Rewrite(EditNote),
    Unnote(DeleteNote),
    Setup(SetSection),
    Break(InsertSectionBreak),
    Story(SetStoryText),
    Move(MoveBlocks),
    Field(InsertField<'static>),
    Caption(InsertCaption<'static>),
    Update(UpdateFields<'static>),
}

impl Command<WordDoc> for Edit {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        match self {
            Edit::Set(c) => c.validate(doc),
            Edit::Insert(c) => c.validate(doc),
            Edit::Bookmark(c) => c.validate(doc),
            Edit::Unmark(c) => c.validate(doc),
            Edit::Link(c) => c.validate(doc),
            Edit::Note(c) => c.validate(doc),
            Edit::Rewrite(c) => c.validate(doc),
            Edit::Unnote(c) => c.validate(doc),
            Edit::Setup(c) => c.validate(doc),
            Edit::Break(c) => c.validate(doc),
            Edit::Story(c) => c.validate(doc),
            Edit::Move(c) => c.validate(doc),
            Edit::Field(c) => c.validate(doc),
            Edit::Caption(c) => c.validate(doc),
            Edit::Update(c) => c.validate(doc),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        Ok(match self {
            Edit::Set(c) => c.prepare(tx)?.changed,
            Edit::Insert(c) => c.prepare(tx).map(|_| true)?,
            Edit::Bookmark(c) => c.prepare(tx)?.changed,
            Edit::Unmark(c) => c.prepare(tx)?.changed,
            Edit::Link(c) => c.prepare(tx)?.changed,
            Edit::Note(c) => c.prepare(tx).map(|_| true)?,
            Edit::Rewrite(c) => c.prepare(tx).map(|_| true)?,
            Edit::Unnote(c) => c.prepare(tx).map(|_| true)?,
            Edit::Setup(c) => c.prepare(tx)?.changed,
            Edit::Break(c) => c.prepare(tx)?.changed,
            Edit::Story(c) => c.prepare(tx).map(|_| true)?,
            Edit::Move(c) => c.prepare(tx)?.changed,
            Edit::Field(c) => c.prepare(tx)?.changed,
            Edit::Caption(c) => c.prepare(tx)?.changed,
            Edit::Update(c) => c.prepare(tx)?.changed,
        })
    }
}

fn policy(rng: &mut Rng, writer: u64) -> RevisionPolicy {
    if rng.below(3) == 0 {
        RevisionPolicy::Untracked
    } else {
        RevisionPolicy::Tracked(Tracking {
            author: format!("Writer {writer}"),
            date: "2026-11-17T10:00:00Z".to_owned(),
        })
    }
}

const WORDS: &[&str] = &["alpha", "beta", "gamma", "report", "Quarter", "and"];

/// A random edit of the mix (`n`: the writer's edit count, for unique names).
fn edit(rng: &mut Rng, targets: &[String], writer: u64, n: usize) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    let other = targets[rng.below(targets.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(4)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    let name = format!("W{writer}N{n}");
    let note = format!("fn:{}", 1 + rng.below(3));
    match rng.below(17) {
        0 | 1 => Edit::Set(SetText {
            target,
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        2 => Edit::Insert(InsertText {
            at: format!("{target}@{}", rng.below(3)),
            text: format!("{} ", words.join(" ")),
            policy: policy(rng, writer),
        }),
        3 => Edit::Bookmark(AddBookmark {
            place: format!("{target}@0:{}", 1 + rng.below(2)),
            name,
        }),
        4 => Edit::Unmark(RemoveBookmark {
            name: format!("W{}N{}", rng.below(4), rng.below(6)),
        }),
        5 => Edit::Link(AddHyperlink {
            range: format!("{target}@0:1"),
            url: Some(format!("https://example.com/{name}")),
            anchor: None,
            policy: RevisionPolicy::Untracked,
        }),
        6 => Edit::Note(InsertNote {
            kind: if rng.below(2) == 0 {
                NoteKind::Footnote
            } else {
                NoteKind::Endnote
            },
            at: format!("{target}@{}", rng.below(2)),
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        7 => Edit::Rewrite(EditNote {
            note,
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        8 => Edit::Unnote(DeleteNote {
            note,
            policy: policy(rng, writer),
        }),
        9 => Edit::Setup(SetSection {
            section: "s:body".to_owned(),
            values: SectionValues {
                columns: Some(1 + i64::try_from(rng.below(3)).unwrap()),
                ..SectionValues::default()
            },
            policy: policy(rng, writer),
        }),
        10 => Edit::Break(InsertSectionBreak {
            after: Some(target),
            before: None,
            kind: "continuous".to_owned(),
            policy: policy(rng, writer),
        }),
        11 => Edit::Story(SetStoryText {
            section: "s:body".to_owned(),
            which: if rng.below(2) == 0 { "header" } else { "footer" }.to_owned(),
            kind: "default".to_owned(),
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        14..=16 => field_edit(rng, target, writer, &words),
        _ => Edit::Move(MoveBlocks {
            blocks: Moving::Blocks(target),
            after: Some(other),
            before: None,
            policy: RevisionPolicy::Untracked,
        }),
    }
}

/// A field edit of the mix: a field or caption inserted (its result counted through the
/// document), or every result updated.
fn field_edit(rng: &mut Rng, target: String, writer: u64, words: &[&str]) -> Edit {
    let today = Day {
        year: 2026,
        month: 11,
        day: 17,
    };
    match 14 + rng.below(3) {
        14 => Edit::Field(InsertField {
            at: format!("{target}@{}", rng.below(2)),
            instruction: if rng.below(2) == 0 { "SEQ Figure \\* ARABIC" } else { "DATE" }.to_owned(),
            result: None,
            simple: rng.below(2) == 0,
            policy: policy(rng, writer),
            today,
            layout: None,
        }),
        15 => Edit::Caption(InsertCaption {
            anchor: target,
            after: rng.below(2) == 0,
            label: "Figure".to_owned(),
            text: words.join(" "),
            policy: policy(rng, writer),
            today,
            layout: None,
        }),
        _ => Edit::Update(UpdateFields {
            ids: None,
            today,
            layout: None,
        }),
    }
}

/// The parts a version writes, as bytes, in the archive's order.
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

fn run(seed: u64, writers: u64, per_writer: usize) -> (u64, u64, usize) {
    let bytes = fixture("generated/ids-and-markup.docx");
    let shared = editor(&bytes);
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let targets: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .collect();
    let mut records: Vec<(Seq, Edit, Snapshot<WordDoc>)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (shared, targets) = (&shared, &targets);
                scope.spawn(move || {
                    let mut rng = Rng(seed ^ (w + 1).wrapping_mul(0x9E37_79B9));
                    let caller = Caller::named(&format!("writer {w}"));
                    (0..per_writer)
                        .filter_map(|n| {
                            let command = edit(&mut rng, targets, w, n);
                            // a refusal (a block a concurrent edit removed, a name taken, an
                            // edit not ported tracked) changes nothing
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
    let structure = records
        .iter()
        .filter(|r| !matches!(r.1, Edit::Set(_) | Edit::Insert(_)))
        .count();
    (stats.conflicts, stats.exclusive, structure)
}

#[test]
fn concurrent_structure_edits_equal_their_serial_order() {
    let mut structure = 0;
    for seed in [31u64, 32, 33] {
        structure += run(seed, 4, 6).2;
    }
    assert!(structure > 0, "the mix commits structure edits");
}

/// The lane's end gate: sixty seeds of eight writers (run with `--ignored`, in release).
#[test]
#[ignore = "the end gate's long run: cargo test --release -- --ignored"]
fn sixty_seeds_of_eight_writers_structuring() {
    let (mut conflicts, mut exclusive, mut structure) = (0, 0, 0);
    for seed in 300..360u64 {
        let (c, e, s) = run(seed, 8, 6);
        conflicts += c;
        exclusive += e;
        structure += s;
    }
    println!(
        "60 seeds, 8 writers x 6 edits: {structure} structure edits committed; {conflicts} conflicts, \
         {exclusive} exclusive runs"
    );
}
