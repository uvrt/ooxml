//! What a document keeps of its paragraphs' lists (`numbering_skips_level`'s membership, the
//! block tier's `block_fact` slot) read as a cold read does: two paragraphs numbered at a
//! level whose label shows the level above, which nothing is numbered at, before and after
//! one of them and a paragraph outside the list are set again -- also where a budget of a
//! few bytes lets what it keeps go at once.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{fixture, Monotonic};
use crate::kept_reads::Tight;
use ooxml_base::Limits;
use ooxml_derive::{CacheBudget, Limit};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_word_edit::facts::{numbering_skips_level, SkippedLevel};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, WordDoc};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

/// A definition whose second level's label shows the first (`%1.%2`), and its instance.
const LISTS: &str = concat!(
    r#"<w:abstractNum w:abstractNumId="9"><w:multiLevelType w:val="multilevel"/>"#,
    r#"<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>"#,
    r#"<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1.%2"/></w:lvl>"#,
    r#"</w:abstractNum><w:num w:numId="9"><w:abstractNumId w:val="9"/></w:num>"#
);

/// A paragraph, and two paragraphs numbered at the second level only, before the body's end.
const SKIPPING: &str = concat!(
    r#"<w:p><w:r><w:t>Outside the list</w:t></w:r></w:p>"#,
    r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="9"/></w:numPr></w:pPr><w:r><w:t>First section</w:t></w:r></w:p>"#,
    r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="9"/></w:numPr></w:pPr><w:r><w:t>Second section</w:t></w:r></w:p>"#
);

/// The Markdown constructs fixture with [`LISTS`] in its numbering (before its instances,
/// as the schema orders them) and [`SKIPPING`] at the end of its body.
fn skipping() -> Package {
    let package = Package::open(fixture("generated/markdown/constructs.docx"), &Limits::default()).unwrap();
    let read = |name: &str| String::from_utf8(package.read(name).unwrap().unwrap().to_vec()).unwrap();
    let xml = read("word/document.xml");
    let at = xml.rfind("<w:sectPr").unwrap();
    let xml = format!("{}{SKIPPING}{}", &xml[..at], &xml[at..]);
    let numbering = read("word/numbering.xml");
    let at = numbering.find("<w:num ").unwrap();
    let numbering = format!("{}{LISTS}{}", &numbering[..at], &numbering[at..]);
    package
        .with_bytes("word/document.xml", Arc::from(xml.into_bytes()))
        .with_bytes("word/numbering.xml", Arc::from(numbering.into_bytes()))
}

fn editor_within(budget: Arc<dyn CacheBudget>) -> Editor {
    Editor::open(
        skipping(),
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        budget,
    )
    .unwrap()
}

/// `doc`'s `numbering_skips_level` as kept, and as a document opened afresh reads it.
fn warm_and_cold(doc: &WordDoc) -> (Vec<SkippedLevel>, Vec<SkippedLevel>) {
    let cold = WordDoc::open(doc.package().clone(), Arc::new(Limit::new(64 << 20))).unwrap();
    (
        numbering_skips_level(doc).unwrap(),
        numbering_skips_level(&cold).unwrap(),
    )
}

/// The paragraph whose text starts with `start`: its id.
fn paragraph(doc: &WordDoc, start: &str) -> String {
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    let k = body.texts().iter().position(|t| t.starts_with(start)).unwrap();
    body.ids.paragraphs().get(k).unwrap().id
}

/// Kept across versions equals a cold read, before and after the paragraph outside the list
/// and the first numbered paragraph are set again.
fn kept_and_cold(editor: &Editor) {
    let (warm, cold) = warm_and_cold(&editor.store().snapshot().doc().clone());
    assert_eq!(warm, cold);
    let skipped = |found: &[SkippedLevel]| found.iter().find(|s| s.list == 9).cloned();
    let first = skipped(&warm).unwrap_or_else(|| panic!("{warm:?}"));
    assert_eq!(
        (first.level, first.paragraphs, first.unused_levels.clone()),
        (1, 2, vec![0])
    );
    for (start, text) in [
        ("Outside the list", "Outside, again"),
        ("First section", "First section, again"),
    ] {
        let command = SetText {
            target: paragraph(&editor.store().snapshot().doc().clone(), start),
            text: text.to_owned(),
            policy: RevisionPolicy::Untracked,
        };
        editor.run(&Caller::anonymous(), &command).unwrap();
        let (warm_now, cold_now) = warm_and_cold(&editor.store().snapshot().doc().clone());
        assert_eq!(warm_now, cold_now, "after setting {start}");
        assert_eq!(skipped(&warm_now).map(|s| s.paragraphs), Some(2), "{warm_now:?}");
    }
}

#[test]
fn lists_kept_across_versions_read_as_cold_after_edits() {
    let editor = editor_within(Arc::new(Limit::new(64 << 20)));
    kept_and_cold(&editor);
    let (kept, _) = editor.store().snapshot().doc().docx().unwrap().block_facts();
    assert!(kept > 0, "the paragraphs' lists are kept");
}

#[test]
fn lists_let_go_for_the_budget_read_as_cold() {
    let editor = editor_within(Arc::new(Tight(AtomicUsize::new(0))));
    kept_and_cold(&editor);
    let (_, bytes) = editor.store().snapshot().doc().docx().unwrap().block_facts();
    assert!(bytes <= 512, "{bytes}");
}
