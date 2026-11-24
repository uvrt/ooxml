//! What a document keeps of its tables' cells Accept All removes (`accept_loses_text`, the
//! block tier's `block_fact` slot) read as a cold read does: a table whose second cell's last
//! paragraph mark is a tracked deletion, before and after its kept cell and a paragraph
//! outside it are set again -- also where a budget of a few bytes lets what it keeps go at
//! once.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{fixture, Monotonic};
use crate::kept_reads::Tight;
use ooxml_base::Limits;
use ooxml_derive::{CacheBudget, Limit};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_word_edit::facts::{accept_loses_text, LostCell};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, WordDoc};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

/// A paragraph, and a table whose second cell Accept All removes with its text (its last
/// paragraph mark deleted), before the body's end.
const LOSING: &str = concat!(
    r#"<w:p><w:r><w:t>Outside the table</w:t></w:r></w:p>"#,
    r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr>"#,
    r#"<w:tc><w:p><w:r><w:t>Kept cell</w:t></w:r></w:p></w:tc>"#,
    r#"<w:tc><w:p><w:pPr><w:rPr><w:del w:id="901" w:author="Reviewer" w:date="2026-01-01T00:00:00Z"/></w:rPr></w:pPr>"#,
    r#"<w:r><w:t>Lost cell text</w:t></w:r></w:p></w:tc>"#,
    r#"</w:tr></w:tbl>"#
);

/// The Markdown constructs fixture with [`LOSING`] at the end of its body.
fn losing() -> Package {
    let package = Package::open(fixture("generated/markdown/constructs.docx"), &Limits::default()).unwrap();
    let xml = String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    let at = xml.rfind("<w:sectPr").unwrap();
    let xml = format!("{}{LOSING}{}", &xml[..at], &xml[at..]);
    package.with_bytes("word/document.xml", Arc::from(xml.into_bytes()))
}

fn editor_within(budget: Arc<dyn CacheBudget>) -> Editor {
    Editor::open(
        losing(),
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        budget,
    )
    .unwrap()
}

/// `doc`'s `accept_loses_text` as kept, and as a document opened afresh reads it.
fn warm_and_cold(doc: &WordDoc) -> (Vec<LostCell>, Vec<LostCell>) {
    let cold = WordDoc::open(doc.package().clone(), Arc::new(Limit::new(64 << 20))).unwrap();
    (accept_loses_text(doc).unwrap(), accept_loses_text(&cold).unwrap())
}

/// The paragraph whose text starts with `start`: its id.
fn paragraph(doc: &WordDoc, start: &str) -> String {
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    let k = body.texts().iter().position(|t| t.starts_with(start)).unwrap();
    body.ids.paragraphs().get(k).unwrap().id
}

/// Kept across versions equals a cold read, before and after the paragraph outside the table
/// (the table kept) and the kept cell's paragraph (the table read again) are set again.
fn kept_and_cold(editor: &Editor) {
    let (warm, cold) = warm_and_cold(&editor.store().snapshot().doc().clone());
    assert_eq!(warm, cold);
    assert_eq!(warm.len(), 1, "{warm:?}");
    assert!(warm[0].text.contains("Lost cell"), "{warm:?}");
    for (start, text) in [
        ("Outside the table", "Outside, again"),
        ("Kept cell", "Kept cell, again"),
    ] {
        let command = SetText {
            target: paragraph(&editor.store().snapshot().doc().clone(), start),
            text: text.to_owned(),
            policy: RevisionPolicy::Untracked,
        };
        editor.run(&Caller::anonymous(), &command).unwrap();
        let (warm_now, cold_now) = warm_and_cold(&editor.store().snapshot().doc().clone());
        assert_eq!(warm_now, cold_now, "after setting {start}");
        assert_eq!(warm_now.len(), 1, "{warm_now:?}");
    }
}

#[test]
fn lost_cells_kept_across_versions_read_as_cold_after_edits() {
    let editor = editor_within(Arc::new(Limit::new(64 << 20)));
    kept_and_cold(&editor);
    let (kept, _) = editor.store().snapshot().doc().docx().unwrap().block_facts();
    assert!(kept > 0, "the tables' facts are kept");
}

#[test]
fn lost_cells_let_go_for_the_budget_read_as_cold() {
    let editor = editor_within(Arc::new(Tight(AtomicUsize::new(0))));
    kept_and_cold(&editor);
    let (_, bytes) = editor.store().snapshot().doc().docx().unwrap().block_facts();
    assert!(bytes <= 512, "{bytes}");
}
