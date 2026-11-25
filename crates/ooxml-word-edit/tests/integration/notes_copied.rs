//! Blocks holding a note's reference are never copied or moved into a text box, as Word
//! 16.106 has no note reference there (its Insert Footnote is off in a text box, and its paste
//! there loses the reference: fu-word3's measurements): the copy or move is refused, the way
//! out the places before and after the paragraph anchoring the text box, as `word_notes`
//! refuses a reference there (`notes_placed`). Blocks without a reference still go in.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{
    BookmarkPolicy, CopyBlocks, CopyRange, CopySource, EditError, ListPolicy, MoveBlocks, Moving, RevisionPolicy,
    StylePolicy, Tracking, UnmappedPolicy,
};

/// chart-places.docx: `p@body/0` holds footnote 1's reference, `p@body/1` anchors the text
/// box whose first paragraph is `p@body/2`.
fn copy(block: &str, at: &str, policy: RevisionPolicy) -> CopyBlocks {
    CopyBlocks {
        source: CopySource::Own,
        range: CopyRange::Blocks(block.to_owned()),
        at: at.into(),
        policy,
        styles: StylePolicy::default(),
        style_map: Vec::new(),
        unmapped: UnmappedPolicy::default(),
        lists: ListPolicy::default(),
        bookmarks: BookmarkPolicy::default(),
    }
}

fn policies() -> [RevisionPolicy; 2] {
    [
        RevisionPolicy::Untracked,
        RevisionPolicy::Tracked(Tracking {
            author: "Writer A".to_owned(),
            date: "2026-11-17T10:00:00Z".to_owned(),
        }),
    ]
}

fn text_boxes(editor: &ooxml_word_edit::Editor) -> Vec<String> {
    let head = editor.store().snapshot();
    let body = String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap();
    body.split("<w:txbxContent>")
        .skip(1)
        .map(|c| c[..c.find("</w:txbxContent>").unwrap()].to_owned())
        .collect()
}

#[test]
fn blocks_holding_a_note_reference_copied_into_a_text_box_are_refused_with_the_places_beside_it() {
    for policy in policies() {
        for at in ["after:p@body/2", "before:p@body/2", "replace:p@body/2"] {
            let editor = editor(&fixture("generated/charts/chart-places.docx"));
            let error = editor
                .run(&Caller::anonymous(), &copy("p@body/0", at, policy.clone()))
                .unwrap_err();
            let EditError::Argument {
                field,
                message,
                options,
            } = error
            else {
                panic!("{at}: not an argument refusal: {error:?}")
            };
            assert_eq!(field, "at");
            assert!(message.contains("text box"), "{message}");
            assert_eq!(options, ["before:p@body/1", "after:p@body/1"], "{at}");
            // the way out works, and no reference lands in a text box
            for way_out in options {
                let editor = crate::common::editor(&fixture("generated/charts/chart-places.docx"));
                editor
                    .run(&Caller::anonymous(), &copy("p@body/0", &way_out, policy.clone()))
                    .unwrap();
                for content in text_boxes(&editor) {
                    assert!(!content.contains("footnoteReference"), "{content}");
                }
            }
        }
    }
}

#[test]
fn blocks_without_a_note_reference_still_go_into_a_text_box() {
    let editor = editor(&fixture("generated/charts/chart-places.docx"));
    editor
        .run(
            &Caller::anonymous(),
            &copy("p@body/6", "after:p@body/2", RevisionPolicy::Untracked),
        )
        .unwrap();
    assert!(text_boxes(&editor).iter().any(|c| c.contains("The end.")));
}

#[test]
fn blocks_holding_a_note_reference_moved_into_a_text_box_are_refused_with_the_anchoring_paragraph() {
    for policy in policies() {
        for after in [true, false] {
            let moved = |target: &str| MoveBlocks {
                blocks: Moving::Blocks("p@body/0".to_owned()),
                after: after.then(|| target.to_owned()),
                before: (!after).then(|| target.to_owned()),
                policy: policy.clone(),
            };
            let editor = editor(&fixture("generated/charts/chart-places.docx"));
            let error = editor.run(&Caller::anonymous(), &moved("p@body/2")).unwrap_err();
            let EditError::Argument {
                field,
                message,
                options,
            } = error
            else {
                panic!("not an argument refusal: {error:?}")
            };
            assert_eq!(field, if after { "after" } else { "before" });
            assert!(message.contains("text box"), "{message}");
            assert_eq!(options, ["p@body/1"]);
            // the way out works (after the anchoring paragraph; before it the block already is)
            if after {
                editor.run(&Caller::anonymous(), &moved("p@body/1")).unwrap();
                for content in text_boxes(&editor) {
                    assert!(!content.contains("footnoteReference"), "{content}");
                }
            }
        }
    }
}
