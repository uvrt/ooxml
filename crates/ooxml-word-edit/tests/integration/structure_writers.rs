//! The random multi-writer oracle for the structure edits (lane P4b): writers on threads
//! mix tracked and untracked `set_text` and `insert_text` with bookmarks added and removed,
//! hyperlinks, notes inserted, edited and deleted, page setup, section breaks, headers and
//! footers and blocks moved, through one document's store; replayed serially in commit
//! order on the document as opened, every version is the same, byte for byte, as the
//! concurrent run made it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, policy, written, Rng, WORDS};
use ooxml_store::{Caller, Command, KeySet, Seq, Snapshot, Txn};
use ooxml_word_edit::{
    AddBookmark, AddHyperlink, Day, DeleteNote, EditError, EditNote, Editor, InsertCaption, InsertField, InsertNote,
    InsertSectionBreak, InsertText, Key, MoveBlocks, MoveNote, Moving, NoteKind, RemoveBookmark, RevisionPolicy,
    SectionValues, SetSection, SetStoryText, SetText, Tracking, UpdateFields, WordDoc,
};

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
            instruction: if rng.below(2) == 0 {
                "SEQ Figure \\* ARABIC"
            } else {
                "DATE"
            }
            .to_owned(),
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

/// An edit that, the first time it is prepared, waits after preparing until it is let go
/// (another commit made in between, deterministically).
struct Paused {
    edit: Edit,
    prepared: std::sync::mpsc::Sender<()>,
    go: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    first: std::sync::atomic::AtomicBool,
}

impl Command<WordDoc> for Paused {
    type Out = bool;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.edit.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<bool, EditError> {
        let out = self.edit.prepare(tx)?;
        if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.prepared.send(()).unwrap();
            self.go.lock().unwrap().recv().unwrap();
        }
        Ok(out)
    }
}

/// A step of the history: an edit, or the undo or redo of the step at that version.
enum Step<'a> {
    Edit(&'a Edit),
    Undo(Seq),
    Redo(Seq),
}

/// `step` run on `editor`: the version it made.
fn run_step(editor: &Editor, step: &Step<'_>) -> Snapshot<WordDoc> {
    match step {
        Step::Edit(edit) => editor.store().run(&Caller::anonymous(), *edit).unwrap().version,
        Step::Undo(seq) => editor.store().undo_step(*seq, &KeySet::new()).unwrap().version,
        Step::Redo(seq) => editor.store().redo_step(*seq).unwrap().version,
    }
}

/// `last` prepared on the fixture `name` as the steps `before` leave it, `between`
/// committed before `last` commits; each version the same, byte for byte, as the steps run
/// serially in that order make it.
fn interleaved(name: &str, before: &[Step<'_>], between: &Step<'_>, last: &Edit) {
    let bytes = fixture(name);
    let (shared, serial) = (editor(&bytes), editor(&bytes));
    for step in before {
        assert_eq!(
            written(run_step(&shared, step).doc()),
            written(run_step(&serial, step).doc())
        );
    }
    let (prepared, wait) = std::sync::mpsc::channel();
    let (release, go) = std::sync::mpsc::channel();
    let paused = Paused {
        edit: last.clone(),
        prepared,
        go: std::sync::Mutex::new(go),
        first: std::sync::atomic::AtomicBool::new(true),
    };
    let (first, second) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| shared.store().run(&Caller::named("last"), &paused));
        wait.recv().unwrap();
        let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_step(&shared, between)));
        release.send(()).unwrap();
        (first.unwrap(), handle.join().unwrap().unwrap())
    });
    assert_eq!(
        written(run_step(&serial, between).doc()),
        written(first.doc()),
        "the step between"
    );
    let replayed = serial.store().run(&Caller::anonymous(), last).unwrap();
    assert_eq!(
        written(replayed.version.doc()),
        written(second.version.doc()),
        "{last:?}"
    );
}

fn tracked(author: &str) -> RevisionPolicy {
    RevisionPolicy::Tracked(Tracking {
        author: author.to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    })
}

/// Seeds 344 and 349: the first tracked edit of the body declares `w16du` on its root; a
/// whole-part rewrite for it, made after the paragraph's revision ids were drawn, kept the
/// ids the snapshot gave (209/210) where a tracked note edit committed in between had
/// taken them, and the serial order gives 211/212. The root's head is written instead.
#[test]
fn a_tracked_edit_rebased_past_another_keeps_the_ids_drawn_where_it_lands() {
    // an untracked edit stamps the document first, as the seeds' earlier edits did
    let stamp = Edit::Set(SetText {
        target: "p:1A2B3C0B".to_owned(),
        text: "Quarter and".to_owned(),
        policy: RevisionPolicy::Untracked,
    });
    let note = Edit::Rewrite(EditNote {
        note: "fn:1".to_owned(),
        text: "beta alpha gamma Quarter".to_owned(),
        policy: tracked("Writer 3"),
    });
    let set = Edit::Set(SetText {
        target: "p:1A2B3C01".to_owned(),
        text: "and alpha".to_owned(),
        policy: tracked("Writer 0"),
    });
    interleaved(
        "generated/ids-and-markup.docx",
        &[Step::Edit(&stamp)],
        &Step::Edit(&note),
        &set,
    );
}

/// History seed 209: a tracked edit prepared where the body's root declares `w16du` (a
/// tracked edit's step, redone) declares nothing; the undo of that redo, committed in
/// between, takes the declaration away, so the edit reads the root's head and is prepared
/// again.
#[test]
fn a_tracked_edit_reads_the_root_declarations_it_relies_on() {
    // an untracked edit stamps the document first, as the seed's earlier edits did
    let stamp = Edit::Set(SetText {
        target: "p@body/29".to_owned(),
        text: "gamma 0".to_owned(),
        policy: RevisionPolicy::Untracked,
    });
    let first = Edit::Set(SetText {
        target: "p@body/0".to_owned(),
        text: "alpha 3".to_owned(),
        policy: tracked("Writer 3"),
    });
    let set = Edit::Set(SetText {
        target: "p@body/5".to_owned(),
        text: "alpha 5".to_owned(),
        policy: tracked("Writer 5"),
    });
    interleaved(
        "samplelib/sample-long.docx",
        &[
            Step::Edit(&stamp),
            Step::Edit(&first),
            Step::Undo(Seq(2)),
            Step::Redo(Seq(2)),
        ],
        &Step::Undo(Seq(4)),
        &set,
    );
}

/// A tracked field's revision ids are drawn where it lands: it writes the annotation ids in
/// use, so an edit that numbered ids past them where it was prepared is prepared again.
#[test]
fn a_tracked_field_writes_the_annotation_ids() {
    let shared = editor(&fixture("generated/ids-and-markup.docx"));
    let field = Edit::Field(InsertField {
        at: "p:1A2B3C0E@0".to_owned(),
        instruction: "DATE".to_owned(),
        result: None,
        simple: false,
        policy: tracked("Writer 1"),
        today: Day {
            year: 2026,
            month: 11,
            day: 17,
        },
        layout: None,
    });
    let done = shared.store().run(&Caller::anonymous(), &field).unwrap();
    assert!(done.changed);
    assert!(done.writes.contains(&Key::Annotations));
}

/// The footnotes part as written, and the body.
fn notes_and_body(editor: &Editor) -> (String, String) {
    let head = editor.store().snapshot();
    let text = |part: &str| String::from_utf8(head.doc().tree(part).unwrap().unwrap().write()).unwrap();
    (text("word/footnotes.xml"), text("word/document.xml"))
}

/// docx-agent's tracked `move_note`: a reference has one note, so the reference is deleted
/// where it was and a copy of the note, numbered next, goes after it with a new reference
/// at the new place, both insertions; one's own inserted reference goes outright, and the
/// old note with it.
#[test]
fn a_note_moved_while_tracking_deletes_the_reference_and_inserts_a_copy() {
    let editor = editor(&fixture("generated/ids-and-markup.docx"));
    let moved = MoveNote {
        note: "fn:1".to_owned(),
        to: "p:1A2B3C02@1".to_owned(),
        policy: tracked("Writer A"),
    };
    let done = editor.run(&Caller::anonymous(), &moved).unwrap();
    assert_eq!(done.out.id.as_deref(), Some("fn:2"));
    assert_eq!(done.out.created, ["fn:2"]);
    let (notes, body) = notes_and_body(&editor);
    assert!(notes.contains(r#"<w:footnote w:id="1">"#), "the note stays: {notes}");
    let copy = &notes[notes.find(r#"<w:footnote w:id="2">"#).unwrap()..];
    assert!(copy.contains("<w:ins "), "the copy's content inserted: {copy}");
    assert!(body.contains(r#"<w:footnoteReference w:id="2"/>"#));
    let deleted = &body[body.find("<w:del ").unwrap()..];
    assert!(
        deleted.contains(r#"<w:footnoteReference w:id="1"/>"#),
        "the old reference deleted"
    );
    // moved again by its own author: the inserted reference goes outright, with its note
    let again = MoveNote {
        note: "fn:2".to_owned(),
        to: "p:1A2B3C02@3".to_owned(),
        policy: tracked("Writer A"),
    };
    let done = editor.run(&Caller::anonymous(), &again).unwrap();
    assert_eq!(done.out.id.as_deref(), Some("fn:3"));
    let (notes, body) = notes_and_body(&editor);
    assert!(!notes.contains(r#"<w:footnote w:id="2">"#), "{notes}");
    assert!(!body.contains(r#"<w:footnoteReference w:id="2"/>"#));
    assert!(body.contains(r#"<w:footnoteReference w:id="3"/>"#));
}
