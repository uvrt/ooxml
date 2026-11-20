//! What adding a comment or a content control, and resolving a thread, read (lane P2c18):
//! placed by id, prepared
//! while another writer's edit of a paragraph elsewhere commits, it commits without a
//! conflict; an edit of what it depends on (the paragraph it marks, the comment part and
//! the largest annotation id, a control's number) is still a conflict, and the edit is
//! prepared again on the head, giving the serial result.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, written};
use crate::interleave::{interleaved, opened, set_text};
use ooxml_store::{Caller, Command};
use ooxml_word_edit::{
    AddComments, CommentEdit, CommentEdits, CommentWrite, ControlKind, EditError, Editor, InsertControl,
    RevisionPolicy, Who, WordDoc,
};
use ooxml_word_model::docx::comments::comments;

fn comment(place: &str) -> AddComments {
    AddComments {
        items: vec![CommentWrite::Add {
            place: place.to_owned(),
            text: "A comment".to_owned(),
        }],
        who: Who {
            author: "Reader".to_owned(),
            date: "2026-11-20T00:00:00Z".to_owned(),
            initials: "R".to_owned(),
        },
    }
}

fn control(at: &str) -> InsertControl {
    InsertControl {
        at: at.to_owned(),
        kind: ControlKind::Text,
        items: Vec::new(),
        tag: Some("tag".to_owned()),
        alias: None,
        policy: RevisionPolicy::Untracked,
    }
}

/// The document after `first` then `second`, run one after the other.
fn serial<A, B>(first: &A, second: &B) -> Vec<(String, std::sync::Arc<[u8]>)>
where
    A: Command<WordDoc, Error = EditError>,
    B: Command<WordDoc, Error = EditError>,
{
    let (shared, _): (Editor, _) = opened();
    shared.store().run(&Caller::anonymous(), first).unwrap();
    shared.store().run(&Caller::anonymous(), second).unwrap();
    written(shared.store().snapshot().doc())
}

#[test]
fn a_comment_by_id_commits_past_an_edit_elsewhere() {
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, comment(&targets[0]), &set_text(&targets[6]));
    assert_eq!((done, conflicts), (true, 0));
    let (done, conflicts) = interleaved(&shared, comment(&format!("{}@0:1", targets[1])), &set_text(&targets[6]));
    assert_eq!((done, conflicts), (true, 0));
    let (done, conflicts) = interleaved(
        &shared,
        comment(&format!("{}@0..{}@1", targets[2], targets[3])),
        &set_text(&targets[6]),
    );
    assert_eq!((done, conflicts), (true, 0));
}

#[test]
fn a_comment_conflicts_with_an_edit_of_what_it_marks_and_gives_the_serial_result() {
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, comment(&targets[1]), &set_text(&targets[1]));
    assert_eq!((done, conflicts), (true, 1));
    assert_eq!(
        written(shared.store().snapshot().doc()),
        serial(&set_text(&targets[1]), &comment(&targets[1]))
    );
}

#[test]
fn two_comments_at_once_are_numbered_as_in_the_serial_order() {
    // both write the comment parts, and the first takes the largest annotation id
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, comment(&targets[0]), &comment(&targets[6]));
    assert_eq!((done, conflicts), (true, 1));
    assert_eq!(
        written(shared.store().snapshot().doc()),
        serial(&comment(&targets[6]), &comment(&targets[0]))
    );
}

#[test]
fn a_control_by_id_commits_past_an_edit_elsewhere() {
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, control(&format!("{}@0", targets[0])), &set_text(&targets[6]));
    assert_eq!((done, conflicts), (true, 0));
}

#[test]
fn two_controls_at_once_take_the_numbers_of_the_serial_order() {
    // the second chose the number the first took: prepared again, it takes the next
    let (shared, targets) = opened();
    let (first, second) = (
        control(&format!("{}@0", targets[6])),
        control(&format!("{}@0", targets[0])),
    );
    let (done, conflicts) = interleaved(&shared, second.clone(), &first);
    assert_eq!((done, conflicts), (true, 1));
    assert_eq!(written(shared.store().snapshot().doc()), serial(&first, &second));
}

#[test]
fn a_comment_and_a_control_elsewhere_do_not_conflict() {
    let (shared, targets) = opened();
    let (done, conflicts) = interleaved(&shared, comment(&targets[0]), &control(&format!("{}@0", targets[6])));
    assert_eq!((done, conflicts), (true, 0));
}

#[test]
fn resolving_a_thread_commits_past_an_edit_elsewhere_and_conflicts_with_another_resolve() {
    let opened = || {
        let shared = editor(&fixture("generated/markdown/review.docx"));
        let docx = shared.store().snapshot().doc().docx().unwrap();
        let threads: Vec<String> = comments(&docx).into_iter().map(|c| c.id).collect();
        let targets: Vec<String> = docx
            .body()
            .unwrap()
            .ids
            .paragraphs()
            .iter()
            .map(|p| p.id.clone())
            .collect();
        assert!(threads.len() >= 2, "{threads:?}");
        (shared, threads, targets)
    };
    let edit = |edit: CommentEdit, id: &str| CommentEdits {
        edit,
        ids: vec![id.to_owned()],
    };
    let (shared, threads, targets) = opened();
    let last = targets.last().unwrap();
    let (done, conflicts) = interleaved(&shared, edit(CommentEdit::Resolve, &threads[0]), &set_text(last));
    assert_eq!((done, conflicts), (true, 0));
    // both write commentsExtended whole
    let (shared, threads, _) = opened();
    let (done, conflicts) = interleaved(
        &shared,
        edit(CommentEdit::Resolve, &threads[0]),
        &edit(CommentEdit::Resolve, &threads[1]),
    );
    assert_eq!((done, conflicts), (true, 1));
}
