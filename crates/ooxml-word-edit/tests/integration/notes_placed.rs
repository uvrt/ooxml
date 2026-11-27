//! A note's reference never goes in a text box, as Word 16.106 puts none there: inserting
//! or moving one to a text box's paragraph is refused, the way out the places beside the
//! text box in the paragraph anchoring it. Moved there, the reference landed in the text
//! box's drawing and its VML fallback both, and Word did not open the file (the Word
//! suite's `chart-places.docx` case).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{EditError, InsertNote, MoveNote, NoteKind, RevisionPolicy, Tracking};

/// `p@body/2` is the text box's paragraph: `p@body/1` anchors the text box after its 19
/// characters ("A text box follows.").
const IN_BOX: &str = "p@body/2@1";

fn tracked() -> RevisionPolicy {
    RevisionPolicy::Tracked(Tracking {
        author: "Writer A".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    })
}

fn options(error: EditError) -> Vec<String> {
    match error {
        EditError::Argument {
            field,
            message,
            options,
        } => {
            assert_eq!(field, "at");
            assert!(message.contains("text box"), "{message}");
            options
        }
        other => panic!("not an argument refusal: {other:?}"),
    }
}

#[test]
fn a_note_moved_into_a_text_box_is_refused_with_the_places_beside_it() {
    for policy in [RevisionPolicy::Untracked, tracked()] {
        let editor = editor(&fixture("generated/charts/chart-places.docx"));
        let moved = MoveNote {
            note: "fn:1".to_owned(),
            to: ooxml_word_model::address::Address::from(crate::common::now(
                &fixture("generated/charts/chart-places.docx"),
                IN_BOX,
            )),
            policy: policy.clone(),
        };
        let error = editor.run(&Caller::anonymous(), &moved).unwrap_err();
        let options = options(error);
        assert_eq!(options.len(), 2, "{options:?}");
        assert!(
            options[0].ends_with("@19") && options[1].ends_with("@20"),
            "{options:?}"
        );
        // the way out works: the reference goes before the text box
        let again = MoveNote {
            note: "fn:1".to_owned(),
            to: ooxml_word_model::address::Address::from(options[0].clone()),
            policy,
        };
        editor.run(&Caller::anonymous(), &again).unwrap();
        let head = editor.store().snapshot();
        let body = String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap();
        for content in body.split("<w:txbxContent>").skip(1) {
            let content = &content[..content.find("</w:txbxContent>").unwrap()];
            assert!(!content.contains("footnoteReference"), "{content}");
        }
    }
}

#[test]
fn a_note_inserted_in_a_text_box_is_refused_with_the_places_beside_it() {
    let editor = editor(&fixture("generated/charts/chart-places.docx"));
    let inserted = InsertNote {
        kind: NoteKind::Endnote,
        at: ooxml_word_model::address::Address::from(crate::common::now(
            &fixture("generated/charts/chart-places.docx"),
            IN_BOX,
        )),
        text: "A note.".to_owned(),
        policy: tracked(),
    };
    let options = options(editor.run(&Caller::anonymous(), &inserted).unwrap_err());
    assert_eq!(options.len(), 2, "{options:?}");
}
