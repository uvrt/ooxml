//! Deleting a block-level content control with its content, as docx-agent's `delete_block`
//! does (`_delete_control_block`, `remove_control(keep_content=False)`): untracked, the
//! control goes with its paragraphs, which it reports removed; tracked, its paragraphs are
//! recorded as deleted and stay, the control is removed untracked and a warning says so; a
//! control in a line is refused with the call that takes it out.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_word_edit::{DeleteBlock, EditError, Editor, RevisionPolicy, Tracking};
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

fn editor() -> Editor {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python/docx-agent/tests/fixtures/generated/markdown/constructs.docx");
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let package = Package::open(bytes, &Limits::default()).unwrap();
    Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(Unlimited),
    )
    .unwrap()
}

fn body(editor: &Editor) -> String {
    let tree = editor
        .store()
        .snapshot()
        .doc()
        .tree("word/document.xml")
        .unwrap()
        .unwrap();
    String::from_utf8(tree.write()).unwrap()
}

#[test]
fn untracked_the_control_goes_with_its_content() {
    let editor = editor();
    let command = DeleteBlock {
        target: "cc:303".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    let done = editor.run(&Caller::anonymous(), &command).unwrap();
    assert_eq!(done.out.id, None);
    assert_eq!(done.out.removed, ["cc:303", "p:6B00003F", "p:6B000040"]);
    assert!(done.out.warnings.is_empty());
    let xml = body(&editor);
    assert!(!xml.contains("w:id=\"303\""));
    assert!(!xml.contains("6B00003F") && !xml.contains("6B000040"));
}

#[test]
fn tracked_its_paragraphs_are_deleted_and_the_control_removed_untracked() {
    let editor = editor();
    let command = DeleteBlock {
        target: "cc:303".to_owned(),
        policy: RevisionPolicy::Tracked(Tracking {
            author: "Writer 1".to_owned(),
            date: "2026-11-19T10:00:00Z".to_owned(),
        }),
    };
    let done = editor.run(&Caller::anonymous(), &command).unwrap();
    assert_eq!(done.out.removed, ["cc:303"]);
    assert_eq!(done.out.warnings.len(), 1);
    let xml = body(&editor);
    assert!(!xml.contains("w:id=\"303\""), "the control itself is gone");
    assert!(
        xml.contains("6B00003F") && xml.contains("6B000040"),
        "its paragraphs stay"
    );
    assert!(xml.contains("<w:del "), "recorded as deleted");
}

#[test]
fn a_control_in_a_line_is_refused() {
    let editor = editor();
    let command = DeleteBlock {
        target: "cc:301".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    let Err(EditError::Refused(message)) = editor.run(&Caller::anonymous(), &command).map(|_| ()) else {
        panic!("refused");
    };
    assert!(message.contains("remove_control"), "{message}");
}
