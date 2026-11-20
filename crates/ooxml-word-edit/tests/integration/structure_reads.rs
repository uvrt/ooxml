//! What the structure edits read (lane P2c15): a bookmark, a note or a move whose place is
//! named by id, prepared while another writer's edit of a paragraph elsewhere commits,
//! commits without a conflict; an edit of what it depends on (a bookmark of the same name,
//! a block in the stretch a move rewrites) is still a conflict, and the edit is prepared
//! again on the head.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, written};
use ooxml_store::{Caller, Command, Txn};
use ooxml_word_edit::{
    AddBookmark, EditError, Editor, InsertNote, MoveBlocks, Moving, NoteKind, RevisionPolicy, SetText, WordDoc,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

/// `command`, held after its first prepare until told to go on.
struct Paused<C> {
    command: C,
    prepared: Sender<()>,
    go: Mutex<Receiver<()>>,
    first: AtomicBool,
}

impl<C: Command<WordDoc, Error = EditError>> Command<WordDoc> for Paused<C> {
    type Out = C::Out;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.command.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<C::Out, EditError> {
        let out = self.command.prepare(tx)?;
        if self.first.swap(false, Ordering::SeqCst) {
            self.prepared.send(()).unwrap();
            self.go.lock().unwrap().recv().unwrap();
        }
        Ok(out)
    }
}

/// An editor of the fixture after a first edit (which stamps every block, writing the
/// whole part), and the body's top-level paragraphs by their stamped ids.
fn opened() -> (Editor, Vec<String>) {
    let shared = editor(&fixture("generated/ids-and-markup.docx"));
    let ids = |shared: &Editor| -> Vec<String> {
        let docx = shared.store().snapshot().doc().docx().unwrap();
        let body = docx.body().unwrap();
        body.ids
            .paragraphs()
            .iter()
            .filter(|p| p.path.len() == 2)
            .map(|p| p.id.clone())
            .collect()
    };
    let first = ids(&shared).last().cloned().unwrap();
    shared.store().run(&Caller::anonymous(), &set_text(&first)).unwrap();
    let targets = ids(&shared);
    assert!(targets.len() >= 8, "{targets:?}");
    (shared, targets)
}

/// `last` prepared, `between` committed, then `last` committed: whether `last` succeeded,
/// and the store's conflicts.
fn interleaved<C, B>(shared: &Editor, last: C, between: &B) -> (bool, u64)
where
    C: Command<WordDoc, Error = EditError> + Send + Sync,
    C::Out: Send,
    B: Command<WordDoc, Error = EditError>,
{
    let (prepared, wait) = channel();
    let (release, go) = channel();
    let paused = Paused {
        command: last,
        prepared,
        go: Mutex::new(go),
        first: AtomicBool::new(true),
    };
    let done = std::thread::scope(|scope| {
        let handle = scope.spawn(|| shared.store().run(&Caller::named("last"), &paused).is_ok());
        wait.recv().unwrap();
        shared.store().run(&Caller::named("between"), between).unwrap();
        release.send(()).unwrap();
        handle.join().unwrap()
    });
    (done, shared.store().stats().conflicts)
}

fn set_text(target: &str) -> SetText {
    SetText {
        target: target.to_owned(),
        text: "Written meanwhile".to_owned(),
        policy: RevisionPolicy::Untracked,
    }
}

fn bookmark(place: &str, name: &str) -> AddBookmark {
    AddBookmark {
        place: place.to_owned(),
        name: name.to_owned(),
    }
}

#[test]
fn a_bookmark_by_id_commits_past_an_edit_elsewhere() {
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, bookmark(&targets[0], "Kept"), &set_text(&targets[6]));
    assert_eq!((done, conflicts), (true, 0));
    let (done, conflicts) = interleaved(
        &shared,
        bookmark(&format!("{}@0:1", targets[1]), "Range"),
        &set_text(&targets[6]),
    );
    assert_eq!((done, conflicts), (true, 0));
}

#[test]
fn a_bookmark_of_the_same_name_added_meanwhile_is_a_conflict() {
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, bookmark(&targets[0], "Same"), &bookmark(&targets[6], "same"));
    // prepared again on the head, it finds the name taken: refused, as serially
    assert_eq!((done, conflicts), (false, 1));
}

#[test]
fn a_bookmark_over_a_place_named_by_condition_reads_the_whole_document() {
    let (shared, targets) = opened();
    shared
        .store()
        .run(&Caller::anonymous(), &bookmark(&targets[0], "First"))
        .unwrap();
    // `bm:First` is resolved by finding the bookmark: any commit meanwhile is a conflict
    let (done, conflicts) = interleaved(&shared, bookmark("bm:First", "Again"), &set_text(&targets[6]));
    assert_eq!((done, conflicts), (true, 1));
}

#[test]
fn a_note_by_id_commits_past_an_edit_elsewhere() {
    let (shared, targets) = opened();
    let note = InsertNote {
        kind: NoteKind::Footnote,
        at: format!("{}@0", targets[0]),
        text: "A note".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    let (done, conflicts) = interleaved(&shared, note, &set_text(&targets[6]));
    assert_eq!((done, conflicts), (true, 0));
}

fn moved(first: &str, after: &str) -> MoveBlocks {
    MoveBlocks {
        blocks: Moving::Blocks(first.to_owned()),
        after: Some(after.to_owned()),
        before: None,
        policy: RevisionPolicy::Untracked,
    }
}

#[test]
fn a_move_commits_past_an_edit_outside_its_ends_and_conflicts_on_them() {
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, moved(&targets[0], &targets[2]), &set_text(&targets[6]));
    assert_eq!((done, conflicts), (true, 0));
    // the moved block is one of the stretch's ends, which the move reads and rewrites
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, moved(&targets[0], &targets[2]), &set_text(&targets[0]));
    assert_eq!((done, conflicts), (true, 1));
    // the paragraph between the block and its anchor is inside the stretch but not at its
    // ends: the move opens only the ends and their neighbours (`Opening::Ends`) and writes
    // back only the runs of children that changed (`EditSink::splice_stretch`), so the
    // paragraph is neither read nor written -- no conflict, and the edit to it is kept, as
    // in serial order
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, moved(&targets[0], &targets[2]), &set_text(&targets[1]));
    assert_eq!((done, conflicts), (true, 0));
    let serial = opened().0;
    serial
        .store()
        .run(&Caller::anonymous(), &set_text(&targets[1]))
        .unwrap();
    serial
        .store()
        .run(&Caller::anonymous(), &moved(&targets[0], &targets[2]))
        .unwrap();
    assert_eq!(
        written(serial.store().snapshot().doc()),
        written(shared.store().snapshot().doc())
    );
}

#[test]
fn an_edit_commits_past_the_first_note_and_the_parts_it_makes() {
    // the first note adds the notes parts and their relationships; an edit elsewhere
    // depends on the main part's relationships only through the records' item they name
    let (shared, targets) = opened();
    let note = InsertNote {
        kind: NoteKind::Endnote,
        at: format!("{}@0", targets[6]),
        text: "The first note".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    let (done, conflicts) = interleaved(&shared, set_text(&targets[0]), &note);
    assert_eq!((done, conflicts), (true, 0));
}

#[test]
fn a_bookmark_commits_past_a_note_that_writes_its_notes_part_whole() {
    // the notes part, written whole, holds no bookmark of the name on the head
    let (shared, targets) = opened();
    let note = |at: &str| InsertNote {
        kind: NoteKind::Footnote,
        at: format!("{at}@0"),
        text: "A note".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    shared.store().run(&Caller::anonymous(), &note(&targets[7])).unwrap();
    let (done, conflicts) = interleaved(&shared, bookmark(&targets[0], "Beside"), &note(&targets[6]));
    assert_eq!((done, conflicts), (true, 0));
}
