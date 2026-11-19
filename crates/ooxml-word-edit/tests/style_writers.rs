//! The random multi-writer oracle for the style edits (lane P4d): writers on threads mix
//! tracked and untracked `set_text` and `insert_text` with list edits (paragraphs added to
//! lists, taken out, moved to a level, lists restarted and continued, a level formatted),
//! through one document's store; replayed serially in commit order on the document as
//! opened, every version is the same, byte for byte, as the concurrent run made it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, Seq, Snapshot, StoreConfig, Txn};
use ooxml_word_edit::{
    EditError, Editor, FormatEdit, FormatPlan, InsertText, LevelFormat, ListAction, ListEdit, Property, RevisionPolicy,
    SetText, Setting, Tracking, Value, WordDoc,
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
    List(ListEdit),
    Format(FormatEdit),
}

impl Command<WordDoc> for Edit {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        match self {
            Edit::Set(c) => c.validate(doc),
            Edit::Insert(c) => c.validate(doc),
            Edit::List(c) => c.validate(doc),
            Edit::Format(c) => c.validate(doc),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        Ok(match self {
            Edit::Set(c) => c.prepare(tx)?.changed,
            Edit::Insert(c) => c.prepare(tx).map(|_| true)?,
            Edit::List(c) => !c.prepare(tx)?.steps.is_empty(),
            Edit::Format(c) => c.prepare(tx)?.steps.iter().any(|s| s.changed),
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

/// A random list action.
fn action(rng: &mut Rng, other: &str) -> ListAction {
    let level = i64::try_from(rng.below(3)).unwrap();
    match rng.below(7) {
        0 | 1 => ListAction::Add {
            kind: if rng.below(2) == 0 { "bullet" } else { "number" }.to_owned(),
            level,
        },
        2 => ListAction::Remove,
        3 => ListAction::Level(level),
        4 => ListAction::Restart(1 + level),
        5 => ListAction::Continue(if rng.below(2) == 0 {
            Some(other.to_owned())
        } else {
            None
        }),
        _ => ListAction::Format(LevelFormat {
            format: Some(if rng.below(2) == 0 { "lowerRoman" } else { "decimal" }.to_owned()),
            text_at: Some(18.0 * f64::from(u8::try_from(rng.below(3)).unwrap())),
            ..LevelFormat::default()
        }),
    }
}

/// A random edit of the mix.
fn edit(rng: &mut Rng, targets: &[String], writer: u64, _n: usize) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    let other = targets[rng.below(targets.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(4)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    match rng.below(7) {
        0 => Edit::Set(SetText {
            target,
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        1 => Edit::Insert(InsertText {
            at: format!("{target}@{}", rng.below(3)),
            text: format!("{} ", words.join(" ")),
            policy: policy(rng, writer),
        }),
        5 | 6 => Edit::Format(FormatEdit {
            targets: vec![if rng.below(2) == 0 {
                target
            } else {
                format!("{target}@0:2")
            }],
            plan: plan(rng),
            cleared: false,
            policy: policy(rng, writer),
        }),
        _ => Edit::List(ListEdit {
            targets: if rng.below(3) == 0 {
                vec![target, other.clone()]
            } else {
                vec![target]
            },
            action: action(rng, &other),
            policy: policy(rng, writer),
        }),
    }
}

/// What a format edit sets: direct formatting, a style, keeps.
fn plan(rng: &mut Rng) -> FormatPlan {
    let flag = |property, on| Setting {
        property,
        value: Some(Value::Flag(on)),
    };
    match rng.below(4) {
        0 => FormatPlan {
            run: vec![flag(Property::Bold, rng.below(2) == 0), flag(Property::Italic, true)],
            ..FormatPlan::default()
        },
        1 => FormatPlan {
            paragraph_style: Some("Heading 2".to_owned()),
            ..FormatPlan::default()
        },
        2 => FormatPlan {
            paragraph: vec![flag(Property::KeepWithNext, true)],
            run: vec![Setting {
                property: Property::Size,
                value: Some(Value::Number(13.0)),
            }],
            ..FormatPlan::default()
        },
        _ => FormatPlan {
            clear_direct: true,
            ..FormatPlan::default()
        },
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
    let bytes = fixture("generated/lists-and-styles.docx");
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
        // a format edit says it formatted its targets whether or not they changed (docx-agent)
        assert!(
            done.changed || matches!(command, Edit::Format(_)),
            "seed {seed}: {command:?} changed nothing serially"
        );
        assert_eq!(done.version.seq(), *at, "seed {seed}: commit order");
        assert_eq!(
            written(done.version.doc()),
            written(version.doc()),
            "seed {seed}: the document at version {at} after {command:?}"
        );
    }
    let stats = shared.store().stats();
    let lists = records
        .iter()
        .filter(|r| matches!(r.1, Edit::List(_) | Edit::Format(_)))
        .count();
    (stats.conflicts, stats.exclusive, lists)
}

#[test]
fn concurrent_list_edits_equal_their_serial_order() {
    let mut lists = 0;
    for seed in [41u64, 42, 43] {
        lists += run(seed, 4, 6).2;
    }
    assert!(lists > 0, "the mix commits list edits");
}

/// The lane's end gate: sixty seeds of eight writers (run with `--ignored`, in release).
#[test]
#[ignore = "the end gate's long run: cargo test --release -- --ignored"]
fn sixty_seeds_of_eight_writers_styling() {
    let (mut conflicts, mut exclusive, mut lists) = (0, 0, 0);
    for seed in 400..460u64 {
        let (c, e, s) = run(seed, 8, 6);
        conflicts += c;
        exclusive += e;
        lists += s;
    }
    println!(
        "60 seeds, 8 writers x 6 edits: {lists} list and format edits committed; {conflicts} conflicts, \
         {exclusive} exclusive runs"
    );
}
