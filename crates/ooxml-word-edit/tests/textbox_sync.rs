//! A text box's VML fallback after an edit of an unstamped document: stamping gives the
//! box's paragraphs paraIds, and every edit -- a lone one or one ending a tool call's batch
//! (lines, a new paragraph, styled text) -- writes them into the fallback too, as
//! docx-agent's `_edit` and `Document.batch` do (`_sync_text_boxes`).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_word_edit::{Editor, InsertParagraph, RevisionPolicy, SetStyledText, SetText};
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

fn first_paragraph(editor: &Editor) -> String {
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let id = docx.body().unwrap().ids.paragraphs().iter().next().unwrap().id.clone();
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

#[test]
fn the_fixture_starts_unsynced() {
    let found = contents(&editor());
    assert_eq!(found.len(), 2);
    assert!(!found[0].contains("w14:paraId="), "the fixture is unstamped");
}

#[test]
fn lines_sync_the_fallback() {
    let editor = editor();
    let command = SetText {
        target: first_paragraph(&editor),
        text: "First line\nSecond line".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    assert!(editor.run(&Caller::anonymous(), &command).unwrap().out.changed);
    assert_synced(&editor);
}

#[test]
fn a_new_paragraph_syncs_the_fallback() {
    let editor = editor();
    let command = InsertParagraph {
        anchor: first_paragraph(&editor),
        after: true,
        text: "New".to_owned(),
        style: None,
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).unwrap();
    assert_synced(&editor);
}

#[test]
fn styled_text_syncs_the_fallback() {
    let editor = editor();
    let command = SetStyledText {
        target: first_paragraph(&editor),
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
