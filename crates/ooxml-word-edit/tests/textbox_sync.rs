//! A text box's VML fallback after an edit of an unstamped document, as docx-agent's
//! `_edit` and `Document.batch` keep it (`_sync_text_boxes`): an edit that changes the
//! box's content -- a lone one or one ending a tool call's batch (lines, a new paragraph,
//! styled text) -- writes the box's paragraphs, stamped, into the fallback too; an edit
//! elsewhere leaves the fallback as it was, though stamping gave the box's paragraphs ids
//! (Word reads `mc:Choice` and writes the fallback again on saving).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_word_edit::{Editor, InsertParagraph, MoveBlocks, Moving, RevisionPolicy, SetStyledText, SetText};
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
        .join("../../python/docx-agent/tests/fixtures/generated/charts/chart-places.docx");
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

/// Every text box content of the body as written (`w:txbxContent`, choice and fallback in
/// turn), without namespace declarations (the fallback's ancestors declare some the
/// choice's content declares on its own elements).
fn contents(editor: &Editor) -> Vec<String> {
    let tree = editor
        .store()
        .snapshot()
        .doc()
        .tree("word/document.xml")
        .unwrap()
        .unwrap();
    let text = String::from_utf8(tree.write()).unwrap();
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("<w:txbxContent") {
        let end = rest[start..].find("</w:txbxContent>").unwrap() + start + "</w:txbxContent>".len();
        out.push(undeclared(&rest[start..end]));
        rest = &rest[end..];
    }
    out
}

/// `xml` without its ` xmlns:...="..."` declarations.
fn undeclared(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(at) = rest.find(" xmlns:") {
        out.push_str(&rest[..at]);
        let quote = rest[at..].find('"').unwrap() + at + 1;
        rest = &rest[rest[quote..].find('"').unwrap() + quote + 1..];
    }
    out.push_str(rest);
    out
}

/// The id of the body's first paragraph (outside the text box).
fn first_paragraph(editor: &Editor) -> String {
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let id = docx.body().unwrap().ids.paragraphs().iter().next().unwrap().id.clone();
    id
}

/// The id of the text box's first paragraph.
fn box_paragraph(editor: &Editor) -> String {
    let doc = editor.store().snapshot().doc().clone();
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    let tree = doc.tree("word/document.xml").unwrap().unwrap();
    let id = body
        .ids
        .paragraphs()
        .iter()
        .find(|p| {
            p.path.len() > 2
                && ooxml_word_model::docx::at_path(tree.root(), &p.path)
                    .is_some_and(|e| e.text_content() == "A chart in a text box:")
        })
        .unwrap()
        .id
        .clone();
    id
}

/// The text box (one, a choice and its fallback) holds the same stamped paragraphs in both.
fn assert_synced(editor: &Editor) {
    let found = contents(editor);
    assert_eq!(found.len(), 2, "one text box: its choice and its fallback");
    assert!(
        found[0].contains("w14:paraId="),
        "stamping gave the box's paragraphs paraIds"
    );
    assert_eq!(found[0], found[1], "the fallback holds what the choice holds");
}

/// Stamping gave the box's paragraphs ids, and the fallback is as it was.
fn assert_left(editor: &Editor, fallback: &str) {
    let found = contents(editor);
    assert_eq!(found.len(), 2, "one text box: its choice and its fallback");
    assert!(
        found[0].contains("w14:paraId="),
        "stamping gave the box's paragraphs paraIds"
    );
    assert_eq!(found[1], fallback, "the fallback is as it was");
}

#[test]
fn the_fixture_starts_unsynced() {
    let found = contents(&editor());
    assert_eq!(found.len(), 2);
    assert!(!found[0].contains("w14:paraId="), "the fixture is unstamped");
}

/// One edit of a target, run on an editor.
type Edit = dyn Fn(&Editor, String);

#[test]
fn an_edit_elsewhere_leaves_the_fallback() {
    let fallback = contents(&editor())[1].clone();
    let edits: [Box<Edit>; 4] = [
        Box::new(|editor, target| {
            let command = SetText {
                target,
                text: "One".to_owned(),
                policy: RevisionPolicy::Untracked,
            };
            editor.run(&Caller::anonymous(), &command).unwrap();
        }),
        Box::new(|editor, target| {
            let command = SetText {
                target,
                text: "First line\nSecond line".to_owned(),
                policy: RevisionPolicy::Untracked,
            };
            editor.run(&Caller::anonymous(), &command).unwrap();
        }),
        Box::new(|editor, anchor| {
            let command = InsertParagraph {
                anchor,
                after: true,
                text: "New".to_owned(),
                style: None,
                policy: RevisionPolicy::Untracked,
            };
            editor.run(&Caller::anonymous(), &command).unwrap();
        }),
        Box::new(|editor, target| {
            let command = SetStyledText {
                target,
                text: "Plain **strong**".to_owned(),
                paragraph_style: None,
                policy: RevisionPolicy::Untracked,
            };
            editor.run(&Caller::anonymous(), &command).unwrap();
        }),
    ];
    for edit in edits {
        let editor = editor();
        let target = first_paragraph(&editor);
        edit(&editor, target);
        assert_left(&editor, &fallback);
    }
}

#[test]
fn a_text_change_syncs_the_fallback() {
    let editor = editor();
    let command = SetText {
        target: box_paragraph(&editor),
        text: "Changed".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    assert!(editor.run(&Caller::anonymous(), &command).unwrap().out.changed);
    assert_synced(&editor);
    assert!(contents(&editor)[1].contains("Changed"));
}

#[test]
fn lines_in_the_box_sync_the_fallback() {
    let editor = editor();
    let command = SetText {
        target: box_paragraph(&editor),
        text: "First line\nSecond line".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    assert!(editor.run(&Caller::anonymous(), &command).unwrap().out.changed);
    assert_synced(&editor);
}

#[test]
fn a_new_paragraph_in_the_box_syncs_the_fallback() {
    let editor = editor();
    let command = InsertParagraph {
        anchor: box_paragraph(&editor),
        after: true,
        text: "New".to_owned(),
        style: None,
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).unwrap();
    assert_synced(&editor);
}

#[test]
fn styled_text_in_the_box_syncs_the_fallback() {
    let editor = editor();
    let command = SetStyledText {
        target: box_paragraph(&editor),
        text: "Plain **strong**".to_owned(),
        paragraph_style: None,
        policy: RevisionPolicy::Untracked,
    };
    assert!(editor
        .run(&Caller::anonymous(), &command)
        .unwrap()
        .out
        .edits
        .iter()
        .any(|e| e.changed));
    assert_synced(&editor);
}

/// chart-places.docx case 148: the block holding the text box moved before the first
/// block (an untracked move, so as many boxes as before in the stretch that changed, in
/// another order) takes its box along with its content unchanged; stamping gives the box's
/// paragraphs ids, but its fallback stays as it was, as docx-agent keeps it (#94).
#[test]
fn a_text_box_moved_with_its_block_is_not_synced() {
    let editor = editor();
    let fallback = contents(&editor)[1].clone();
    let moved = MoveBlocks {
        blocks: Moving::Blocks("p@body/4".to_owned()),
        after: None,
        before: Some("p@body/0".to_owned()),
        policy: RevisionPolicy::Untracked,
    };
    assert!(editor.run(&Caller::anonymous(), &moved).unwrap().changed);
    let now = contents(&editor);
    assert!(now[0].contains("w14:paraId"), "the box's paragraphs are stamped");
    assert_eq!(now[1], fallback);
}
