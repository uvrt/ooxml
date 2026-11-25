//! The random multi-writer oracle for the style edits (lane P4d): writers on threads mix
//! tracked and untracked `set_text` and `insert_text` with list edits (paragraphs added to
//! lists, taken out, moved to a level, lists restarted and continued, a level formatted),
//! style sheet edits, Word's Convert (the document opened in mode 14) and paragraphs made
//! to follow a sample paragraph (`FollowSample`),
//! through one document's store; replayed serially in commit order on the document as
//! opened, every version is the same, byte for byte, as the concurrent run made it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{fixture, policy, written, Monotonic, Rng, Unlimited, WORDS};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, Command, Seq, Snapshot, StoreConfig, Txn};
use ooxml_word_edit::{
    EditError, Editor, FollowSample, FormatEdit, FormatPlan, InsertText, LevelFormat, ListAction, ListEdit, Property,
    RevisionPolicy, SetText, Setting, StyleAction, StyleEdit, StyleFormatting, Tracking, UpgradeToModern, Value,
    WordDoc,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Instant;

fn editor(bytes: &Arc<[u8]>) -> Editor {
    open(Package::open(Arc::clone(bytes), &Limits::default()).unwrap())
}

/// The document of `bytes` in compatibility mode 14 with two legacy options, for Convert.
fn legacy(bytes: &Arc<[u8]>) -> Editor {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    let settings = String::from_utf8(package.read("word/settings.xml").unwrap().unwrap().to_vec()).unwrap();
    let old = settings.replace(
        r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>"#,
        r#"<w:compat><w:noLeading/><w:spaceForUL/><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="14"/>"#,
    );
    assert_ne!(old, settings, "the fixture is in mode 15");
    open(package.with_bytes("word/settings.xml", Arc::from(old.into_bytes())))
}

fn open(package: Package) -> Editor {
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
    Style(StyleEdit),
    Template(UpgradeToModern),
    Follow(FollowSample),
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
            Edit::Style(c) => c.validate(doc),
            Edit::Template(c) => c.validate(doc),
            Edit::Follow(c) => c.validate(doc),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        Ok(match self {
            Edit::Set(c) => c.prepare(tx)?.changed,
            Edit::Insert(c) => c.prepare(tx).map(|_| true)?,
            Edit::List(c) => !c.prepare(tx)?.steps.is_empty(),
            Edit::Format(c) => c.prepare(tx)?.steps.iter().any(|s| s.changed),
            Edit::Style(c) => c.prepare(tx)?.changed,
            Edit::Template(c) => c.prepare(tx)?.changed,
            Edit::Follow(c) => c.prepare(tx)?.copies.iter().any(|c| c.changed()),
        })
    }
}

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
fn edit(rng: &mut Rng, targets: &[String], writer: u64, n: usize) -> Edit {
    let target = targets[rng.below(targets.len())].clone();
    let other = targets[rng.below(targets.len())].clone();
    let words: Vec<&str> = (0..1 + rng.below(4)).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    match rng.below(10) {
        9 => Edit::Follow(FollowSample {
            sample: other.clone(),
            copies: vec![target, targets[rng.below(targets.len())].clone()],
            source: None,
            policy: policy(rng, writer),
        }),
        7 | 8 if rng.below(8) == 0 => Edit::Template(UpgradeToModern),
        7 | 8 => Edit::Style(StyleEdit {
            action: style_action(rng, writer, n),
        }),
        0 => Edit::Set(SetText {
            target,
            text: words.join(" "),
            policy: policy(rng, writer),
        }),
        1 => Edit::Insert(InsertText {
            at: format!("{target}@{}", rng.below(3)).into(),
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

/// A style sheet edit: a style's formatting changed, one added, one removed (its uses
/// pointed at Normal), the unused purged, the default table style, the language.
fn style_action(rng: &mut Rng, writer: u64, n: usize) -> StyleAction {
    let flag = |property, on| Setting {
        property,
        value: Some(Value::Flag(on)),
    };
    let named = ["Normal", "Heading 2", "List Paragraph"][rng.below(3)].to_owned();
    match rng.below(6) {
        0 => StyleAction::Modify {
            name: named,
            kind: None,
            numbering: None,
            tabs: None,
            regions: Vec::new(),
            formatting: StyleFormatting {
                run: vec![flag(Property::Bold, rng.below(2) == 0)],
                ..StyleFormatting::default()
            },
        },
        1 => StyleAction::Add {
            name: format!("Writer {writer} style {}", n % 3),
            kind: "paragraph".to_owned(),
            based_on: Some("Heading 2".to_owned()),
            next_style: None,
            tabs: None,
            regions: Vec::new(),
            formatting: StyleFormatting {
                run: vec![flag(Property::Italic, true)],
                ..StyleFormatting::default()
            },
        },
        2 => StyleAction::Remove {
            name: "Heading 2".to_owned(),
            kind: None,
            replacement: Some("Normal".to_owned()),
        },
        3 => StyleAction::PurgeUnused,
        4 => StyleAction::SetDefault {
            name: ["Table Grid", "Normal Table"][rng.below(2)].to_owned(),
        },
        _ => StyleAction::SetLanguage {
            language: ["fr-BE", "nl-BE"][rng.below(2)].to_owned(),
            style: None,
        },
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

/// Panics with the first part that differs and the text around its first difference.
fn same(serial: &[(String, Arc<[u8]>)], concurrent: &[(String, Arc<[u8]>)], what: &str) {
    let names = |parts: &[(String, Arc<[u8]>)]| parts.iter().map(|p| p.0.clone()).collect::<Vec<_>>();
    assert_eq!(names(serial), names(concurrent), "{what}: the parts");
    for ((name, left), (_, right)) in serial.iter().zip(concurrent) {
        if left == right {
            continue;
        }
        let at = left
            .iter()
            .zip(right.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(left.len().min(right.len()));
        let window = |bytes: &[u8]| {
            String::from_utf8_lossy(&bytes[at.saturating_sub(300)..(at + 300).min(bytes.len())]).into_owned()
        };
        panic!(
            "{what}: {name} differs at byte {at}\nserial:     {}\nconcurrent: {}",
            window(left),
            window(right)
        );
    }
}

fn run(seed: u64, writers: u64, per_writer: usize) -> (u64, u64, usize, usize) {
    let bytes = fixture("generated/lists-and-styles.docx");
    let shared = legacy(&bytes);
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
    let serial = legacy(&bytes);
    for (at, command, version) in &records {
        let done = serial.run(&Caller::anonymous(), command).unwrap();
        // a format edit says it formatted its targets whether or not they changed (docx-agent)
        assert!(
            done.changed || matches!(command, Edit::Format(_)),
            "seed {seed}: {command:?} changed nothing serially"
        );
        assert_eq!(done.version.seq(), *at, "seed {seed}: commit order");
        same(
            &written(done.version.doc()),
            &written(version.doc()),
            &format!("seed {seed}: the document at version {at} after {command:?}"),
        );
    }
    let stats = shared.store().stats();
    let lists = records
        .iter()
        .filter(|r| matches!(r.1, Edit::List(_) | Edit::Format(_)))
        .count();
    let converted = records.iter().filter(|r| matches!(r.1, Edit::Template(_))).count();
    (stats.conflicts, stats.exclusive, lists, converted)
}

/// An edit that, the first time it is prepared, waits after preparing for another writer
/// to commit (`prepared`, then `committed`): it commits on a base older than the head.
struct Overtaken<'a> {
    edit: Edit,
    first: AtomicBool,
    prepared: &'a Barrier,
    committed: &'a Barrier,
}

impl Command<WordDoc> for Overtaken<'_> {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.edit.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        let out = self.edit.prepare(tx);
        if self.first.swap(false, Ordering::SeqCst) {
            self.prepared.wait();
            self.committed.wait();
        }
        out
    }
}

/// Seed 403's shape: a tracked `set_text` on a paragraph a tracked format edit gave a
/// formatting change copies that change's id and numbers the copy above the ids in use,
/// while a tracked insertion elsewhere commits first and draws the next id. The insertion
/// declares the annotation ids it writes, so the `set_text` is prepared again and both
/// orders give the serial document (the insertion once wrote its block's key alone: two
/// revision records with one id).
#[test]
fn a_renumbered_copy_and_a_concurrent_tracked_insertion() {
    let bytes = fixture("generated/lists-and-styles.docx");
    let tracked = |writer: u64| {
        RevisionPolicy::Tracked(Tracking {
            author: format!("Writer {writer}"),
            date: "2026-11-17T10:00:00Z".to_owned(),
        })
    };
    let format = Edit::Format(FormatEdit {
        targets: vec!["p:5A000002".to_owned()],
        plan: FormatPlan {
            run: vec![Setting {
                property: Property::Italic,
                value: Some(Value::Flag(true)),
            }],
            ..FormatPlan::default()
        },
        cleared: false,
        policy: tracked(7),
    });
    let set = Edit::Set(SetText {
        target: "p:5A000002".to_owned(),
        text: "Quarter".to_owned(),
        policy: tracked(7),
    });
    let insert = Edit::Insert(InsertText {
        at: "p:5A00000E@1".into(),
        text: "and gamma ".to_owned(),
        policy: tracked(1),
    });
    let shared = editor(&bytes);
    assert!(shared.run(&Caller::anonymous(), &format).unwrap().changed);
    let (prepared, committed) = (Barrier::new(2), Barrier::new(2));
    let overtaken = Overtaken {
        edit: set.clone(),
        first: AtomicBool::new(true),
        prepared: &prepared,
        committed: &committed,
    };
    let (set_version, insert_version) = std::thread::scope(|scope| {
        let setter = scope.spawn(|| shared.run(&Caller::named("writer 7"), &overtaken).unwrap().version);
        prepared.wait();
        let inserted = shared.run(&Caller::named("writer 1"), &insert).unwrap().version;
        committed.wait();
        (setter.join().unwrap(), inserted)
    });
    assert!(insert_version.seq() < set_version.seq(), "the insertion commits first");
    let serial = editor(&bytes);
    for command in [&format, &insert, &set] {
        serial.run(&Caller::anonymous(), command).unwrap();
    }
    same(
        &written(serial.store().snapshot().doc()),
        &written(set_version.doc()),
        "the set_text committed after the insertion",
    );
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
    let (mut conflicts, mut exclusive, mut lists, mut converted) = (0, 0, 0, 0);
    for seed in 400..460u64 {
        let (c, e, s, t) = run(seed, 8, 6);
        conflicts += c;
        exclusive += e;
        lists += s;
        converted += t;
    }
    println!(
        "60 seeds, 8 writers x 6 edits: {lists} list and format edits and {converted} conversions committed; \
         {conflicts} conflicts, {exclusive} exclusive runs"
    );
}
