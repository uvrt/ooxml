//! What a document keeps of its blocks' facts across its versions (the block tier's
//! `block_fact` slot) read as a cold read does: `direct_formatting` of a document with
//! paragraphs and runs formatted directly and a table kept together, and
//! `table_grid_mismatch` and `empty_paragraphs` (the same slot; `punctuation_spacing` beside
//! them), after its formatted paragraph, a cell's paragraph, a paragraph with spacing
//! problems, an off-grid table's paragraph and paragraphs beside blank lines in a cell and
//! in a nested table are set again -- also where a budget of a few bytes lets what it keeps
//! go at once.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{fixture, Monotonic};
use crate::kept_reads::Tight;
use ooxml_base::Limits;
use ooxml_derive::{CacheBudget, Limit};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_word_edit::facts::{
    direct_formatting, empty_paragraphs, punctuation_spacing, table_grid_mismatch, DirectFormatting, GridMismatch,
    SpacingProblem,
};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, WordDoc};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

/// Paragraphs formatted directly (the paragraph, all its text, some of it), a table kept
/// together whose rows keep with the next by direct formatting, paragraphs whose
/// punctuation's spacing their language does not want, a table with a row past its grid, and
/// a table whose cell and nested table's cell hold blank lines used as space, before the
/// body's end.
const FORMATTED: &str = concat!(
    r#"<w:p><w:pPr><w:jc w:val="center"/><w:spacing w:after="400"/></w:pPr>"#,
    r#"<w:r><w:rPr><w:b/><w:sz w:val="30"/></w:rPr><w:t>Formatted all through</w:t></w:r></w:p>"#,
    r#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>Partly</w:t></w:r><w:r><w:t xml:space="preserve"> plain</w:t></w:r></w:p>"#,
    r#"<w:tbl><w:tblGrid><w:gridCol w:w="4500"/></w:tblGrid>"#,
    r#"<w:tr><w:tc><w:p><w:pPr><w:keepNext/><w:jc w:val="right"/></w:pPr><w:r><w:t>Kept one</w:t></w:r></w:p></w:tc></w:tr>"#,
    r#"<w:tr><w:tc><w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>Kept two</w:t></w:r></w:p></w:tc></w:tr>"#,
    r#"<w:tr><w:tc><w:p><w:r><w:rPr><w:color w:val="FF0000"/></w:rPr><w:t>Last row</w:t></w:r></w:p></w:tc></w:tr>"#,
    r#"</w:tbl>"#,
    r#"<w:p><w:r><w:rPr><w:lang w:val="fr-FR"/></w:rPr><w:t xml:space="preserve">Vraiment ? Oui ! Bien sûr : toujours.</w:t></w:r></w:p>"#,
    r#"<w:p><w:r><w:rPr><w:lang w:val="en-US"/></w:rPr><w:t xml:space="preserve">Really ? Yes !</w:t></w:r></w:p>"#,
    r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>"#,
    r#"<w:tr><w:tc><w:tcPr><w:gridSpan w:val="3"/></w:tcPr><w:p><w:r><w:t>Off the grid</w:t></w:r></w:p></w:tc></w:tr>"#,
    r#"<w:tr><w:tc><w:p><w:r><w:t>On it</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc></w:tr>"#,
    r#"</w:tbl>"#,
    r#"<w:tbl><w:tblGrid><w:gridCol w:w="4500"/></w:tblGrid><w:tr><w:tc>"#,
    r#"<w:p><w:r><w:t>Above the gap</w:t></w:r></w:p><w:p/><w:p/><w:p><w:r><w:t>Below the gap</w:t></w:r></w:p>"#,
    r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc>"#,
    r#"<w:p><w:r><w:t>Nested above</w:t></w:r></w:p><w:p/><w:p><w:r><w:t>Nested below</w:t></w:r></w:p>"#,
    r#"</w:tc></w:tr></w:tbl><w:p/></w:tc></w:tr></w:tbl>"#
);

/// The Markdown constructs fixture with [`FORMATTED`] at the end of its body.
fn formatted() -> Package {
    let package = Package::open(fixture("generated/markdown/constructs.docx"), &Limits::default()).unwrap();
    let xml = String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    let at = xml.rfind("<w:sectPr").unwrap();
    let xml = format!("{}{FORMATTED}{}", &xml[..at], &xml[at..]);
    package.with_bytes("word/document.xml", Arc::from(xml.into_bytes()))
}

fn editor_within(budget: Arc<dyn CacheBudget>) -> Editor {
    Editor::open(
        formatted(),
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        budget,
    )
    .unwrap()
}

/// `doc`'s `direct_formatting` as kept, and as a document opened afresh from its package
/// reads it.
fn warm_and_cold(doc: &WordDoc) -> (Vec<DirectFormatting>, Vec<DirectFormatting>) {
    let cold = WordDoc::open(doc.package().clone(), Arc::new(Limit::new(64 << 20))).unwrap();
    let spacing = |d: &WordDoc| punctuation_spacing(d, Some("fr-BE")).unwrap();
    let (spacing_warm, spacing_cold) = (spacing(doc), spacing(&cold));
    assert_eq!(spacing_warm, spacing_cold, "punctuation_spacing");
    assert!(
        spacing_warm.iter().any(|p| matches!(p, SpacingProblem::Spacing(_)))
            && spacing_warm
                .iter()
                .any(|p| matches!(p, SpacingProblem::LanguageMismatch(_))),
        "{spacing_warm:?}"
    );
    let (grid_warm, grid_cold) = (
        table_grid_mismatch(doc, None).unwrap(),
        table_grid_mismatch(&cold, None).unwrap(),
    );
    assert_eq!(grid_warm, grid_cold, "table_grid_mismatch");
    let (empty_warm, empty_cold) = (empty_paragraphs(doc).unwrap(), empty_paragraphs(&cold).unwrap());
    assert_eq!(empty_warm, empty_cold, "empty_paragraphs");
    let cells: Vec<&str> = empty_warm
        .iter()
        .map(|e| e.at.as_str())
        .filter(|at| *at != "body")
        .collect();
    assert_eq!(
        cells.len(),
        2,
        "the cell's and the nested table's blank lines: {empty_warm:?}"
    );
    assert!(
        grid_warm.iter().any(|g| matches!(g, GridMismatch::OffGrid { .. })),
        "{grid_warm:?}"
    );
    (direct_formatting(doc).unwrap(), direct_formatting(&cold).unwrap())
}

/// The paragraph whose text starts with `start`: its id.
fn paragraph(doc: &WordDoc, start: &str) -> String {
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    let texts = body.texts();
    let k = texts.iter().position(|t| t.starts_with(start)).unwrap();
    body.ids.paragraphs().get(k).unwrap().id
}

/// `direct_formatting` kept across versions equals a cold read, before and after the
/// formatted paragraph and a kept row's paragraph are set again; the version before reads
/// as it did.
fn kept_and_cold(editor: &Editor) {
    let first = editor.store().snapshot().doc().clone();
    let (warm, cold) = warm_and_cold(&first);
    assert_eq!(warm, cold);
    assert!(
        warm.iter().any(|e| e.props.iter().any(|p| p.0 == "alignment")),
        "{warm:?}"
    );
    assert!(warm.iter().any(|e| e.in_part.contains(&"italic")), "{warm:?}");
    assert!(
        !warm.iter().flat_map(|e| &e.props).any(|p| p.0 == "keep_with_next"),
        "a kept table's rows keep by their table: {warm:?}"
    );
    for (start, text) in [
        ("Formatted all", "Formatted, set again"),
        ("Kept two", "Kept two, again"),
        ("Vraiment", "Vraiment ? Toujours!"),
        ("Off the grid", "Still off the grid"),
        ("Above the gap", "Above the gap, again"),
        ("Nested below", "Nested below, again"),
    ] {
        let command = SetText {
            target: paragraph(&editor.store().snapshot().doc().clone(), start),
            text: text.to_owned(),
            policy: RevisionPolicy::Untracked,
        };
        editor.run(&Caller::anonymous(), &command).unwrap();
        let now = editor.store().snapshot().doc().clone();
        let (warm_now, cold_now) = warm_and_cold(&now);
        assert_eq!(warm_now, cold_now, "after setting {start}");
    }
    assert_eq!(
        direct_formatting(&first).unwrap(),
        warm,
        "the version before reads as it did"
    );
}

#[test]
fn direct_formatting_kept_across_versions_reads_as_cold_after_edits() {
    let editor = editor_within(Arc::new(Limit::new(64 << 20)));
    kept_and_cold(&editor);
    let (kept, _) = editor.store().snapshot().doc().docx().unwrap().block_facts();
    assert!(kept > 0, "the paragraphs' facts are kept");
}

#[test]
fn direct_formatting_let_go_for_the_budget_reads_as_cold() {
    let editor = editor_within(Arc::new(Tight(AtomicUsize::new(0))));
    kept_and_cold(&editor);
    let (_, bytes) = editor.store().snapshot().doc().docx().unwrap().block_facts();
    assert!(bytes <= 512, "{bytes}");
}
