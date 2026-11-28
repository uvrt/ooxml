//! What a document's checks keep across its versions read as a cold read does: each
//! table's cells (`PartCells::of`, the block tier's, kept by the block element) and its rows' keeps
//! (`row_keeps`, kept by the table element and the outermost table) after an edit inside a
//! table, and the texts' languages (`Detections`) after a paragraph's language changed --
//! also where a budget of a few bytes lets what they keep go at once.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::facts::{text_language, Detections, LanguageScope};
use ooxml_word_edit::{RevisionPolicy, SetText, WordDoc};
use ooxml_word_model::docx::cells::PartCells;
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::docx::keeps::{row_keeps, RowKeep};
use ooxml_word_model::docx::Docx;

/// Every body paragraph's place, and the cell each is in as `cells` reads it: through the
/// block tier the document's versions share (`kept`), or a document opened afresh.
fn cells_of(doc: &WordDoc, kept: bool) -> Vec<(Vec<u32>, Option<ooxml_word_model::docx::formatting::Cell>)> {
    let docx = if kept {
        doc.docx().unwrap()
    } else {
        std::sync::Arc::new(
            Docx::open(doc.package(), std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX))).unwrap(),
        )
    };
    let body = docx.body().unwrap();
    let mut cells = PartCells::of(&docx, body);
    body.ids
        .paragraphs()
        .iter()
        .map(|p| (p.path.to_vec(), cells.cell(&p.path)))
        .collect()
}

#[test]
fn a_table_s_cells_kept_across_versions_are_read_as_cold_after_an_edit_in_a_table() {
    cells_kept_and_cold(&editor(&fixture("generated/markdown/constructs.docx")));
}

#[test]
fn tables_readings_let_go_for_the_budget_are_read_as_cold() {
    let bytes = fixture("generated/markdown/constructs.docx");
    let package = ooxml_opc::Package::open(std::sync::Arc::clone(&bytes), &ooxml_base::Limits::default()).unwrap();
    let editor = ooxml_word_edit::Editor::open(
        package,
        ooxml_store::StoreConfig::default(),
        std::sync::Arc::new(crate::common::Monotonic(std::time::Instant::now())),
        std::sync::Arc::new(Tight(std::sync::atomic::AtomicUsize::new(0))),
    )
    .unwrap();
    cells_kept_and_cold(&editor);
    let (_, bytes) = editor.store().snapshot().doc().docx().unwrap().block_readings();
    assert!(bytes <= 512, "{bytes}");
}

/// The cells read with the tables' readings kept equal a cold read, before and after a
/// paragraph in a table is set again.
fn cells_kept_and_cold(editor: &ooxml_word_edit::Editor) {
    let first = editor.store().snapshot().doc().clone();
    let read = cells_of(&first, true);
    assert!(
        read.iter().filter(|(_, c)| c.is_some()).count() > 2,
        "the fixture has table cells"
    );
    // A paragraph inside the first table set again.
    let docx = first.docx().unwrap();
    let in_table = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .find(|p| p.path.len() > 2)
        .map(|p| p.id.clone())
        .unwrap();
    let command = SetText {
        target: ooxml_word_model::address::Address::from(in_table),
        text: "Other words in the cell".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).unwrap();
    let second = editor.store().snapshot().doc().clone();
    assert_eq!(cells_of(&second, true), cells_of(&second, false));
    assert_eq!(cells_of(&first, true), read, "the version before reads as it did");
}

/// Every story table's rows' keeps, in order.
type TablesRows = Vec<Option<Vec<RowKeep>>>;

/// Every story table's rows' keeps as `docx` reads them.
fn rows_of(docx: &Docx) -> TablesRows {
    let formatting = Formatting::read(docx);
    docx.parts
        .iter()
        .flat_map(|story| {
            let formatting = &formatting;
            story
                .ids
                .tables()
                .iter()
                .map(move |t| row_keeps(docx, formatting, story, &t.path).map(|r| r.to_vec()))
        })
        .collect()
}

/// The rows' keeps of `doc` as kept, and as a document opened afresh from its package reads
/// them.
fn rows_warm_and_cold(doc: &WordDoc) -> (TablesRows, TablesRows) {
    let warm = rows_of(&doc.docx().unwrap());
    let cold = rows_of(&Docx::open(doc.package(), std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX))).unwrap());
    (warm, cold)
}

/// The rows' keeps kept across versions equal a cold read, before and after a paragraph in
/// a table is set again.
fn rows_kept_and_cold(editor: &ooxml_word_edit::Editor) {
    let first = editor.store().snapshot().doc().clone();
    let (warm, cold) = rows_warm_and_cold(&first);
    assert!(warm.iter().flatten().count() > 0, "the fixture has tables");
    assert_eq!(warm, cold);
    let docx = first.docx().unwrap();
    let in_table = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .find(|p| p.path.len() > 2)
        .map(|p| p.id.clone())
        .unwrap();
    let command = SetText {
        target: ooxml_word_model::address::Address::from(in_table),
        text: "Other words in the cell".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).unwrap();
    let second = editor.store().snapshot().doc().clone();
    let (warm_after, cold_after) = rows_warm_and_cold(&second);
    assert_eq!(warm_after, cold_after);
    assert_eq!(rows_warm_and_cold(&first).0, warm, "the version before reads as it did");
}

#[test]
fn a_table_s_rows_keeps_kept_across_versions_are_read_as_cold_after_an_edit_in_a_table() {
    rows_kept_and_cold(&editor(&fixture("generated/markdown/constructs.docx")));
}

#[test]
fn rows_keeps_let_go_for_the_budget_are_read_as_cold() {
    let bytes = fixture("generated/markdown/constructs.docx");
    let package = ooxml_opc::Package::open(std::sync::Arc::clone(&bytes), &ooxml_base::Limits::default()).unwrap();
    let editor = ooxml_word_edit::Editor::open(
        package,
        ooxml_store::StoreConfig::default(),
        std::sync::Arc::new(crate::common::Monotonic(std::time::Instant::now())),
        std::sync::Arc::new(Tight(std::sync::atomic::AtomicUsize::new(0))),
    )
    .unwrap();
    rows_kept_and_cold(&editor);
    let (_, bytes) = editor.store().snapshot().doc().docx().unwrap().row_keeps();
    assert!(bytes <= 512, "{bytes}");
}

fn language(doc: &WordDoc, detections: Option<&Detections>) -> Option<ooxml_word_edit::facts::TextLanguage> {
    let scope = LanguageScope::new(true, true);
    let scope = match detections {
        Some(kept) => scope.with_detections(kept),
        None => scope,
    };
    text_language(doc, scope).unwrap()
}

#[test]
fn texts_languages_kept_across_checks_are_read_as_cold_after_a_language_change() {
    languages_kept_and_cold(&Detections::default());
}

/// A budget of a few bytes: the judgements kept are let go almost at once.
pub(crate) struct Tight(pub(crate) std::sync::atomic::AtomicUsize);

impl ooxml_derive::CacheBudget for Tight {
    fn reserve(&self, bytes: usize) -> bool {
        use std::sync::atomic::Ordering;
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used + bytes <= 512).then_some(used + bytes)
            })
            .is_ok()
    }
    fn release(&self, bytes: usize) {
        use std::sync::atomic::Ordering;
        let _ = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |u| Some(u.saturating_sub(bytes)));
    }
    fn used(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[test]
fn paragraphs_judgements_let_go_for_the_budget_are_judged_as_cold() {
    let detections = Detections::new(std::sync::Arc::new(Tight(std::sync::atomic::AtomicUsize::new(0))));
    languages_kept_and_cold(&detections);
    assert!(detections.judgements().bytes <= 512, "{:?}", detections.judgements());
    // The texts' detections do not fit: none is kept.
    assert_eq!(detections.texts().entries, 0, "{:?}", detections.texts());
    assert_eq!(detections.texts().bytes, 0, "{:?}", detections.texts());
}

/// The texts' detections are counted on the account they are given, beside the
/// paragraphs' judgements, and given back with them.
#[test]
fn texts_detections_are_on_the_documents_account() {
    let budget = std::sync::Arc::new(ooxml_derive::Limit::new(64 << 20));
    let detections = Detections::new(std::sync::Arc::clone(&budget) as std::sync::Arc<dyn ooxml_derive::CacheBudget>);
    let editor = editor(&fixture("samplelib/sample-long.docx"));
    let doc = editor.store().snapshot().doc().clone();
    assert_eq!(language(&doc, Some(&detections)), language(&doc, None));
    let texts = detections.texts();
    assert!(texts.entries > 0 && texts.bytes > 0, "{texts:?}");
    use ooxml_derive::CacheBudget;
    assert_eq!(budget.used(), texts.bytes + detections.judgements().bytes);
    drop(detections);
    assert_eq!(budget.used(), 0);
}

/// A check with `detections` equals a cold one, before and after a paragraph's text is
/// set in another language, and reads again from what it kept.
fn languages_kept_and_cold(detections: &Detections) {
    let editor = editor(&fixture("samplelib/sample-long.docx"));
    let first = editor.store().snapshot().doc().clone();
    assert_eq!(language(&first, Some(detections)), language(&first, None));
    let docx = first.docx().unwrap();
    let target = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .find(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .unwrap();
    let command = SetText {
        target: ooxml_word_model::address::Address::from(target),
        text: "Le rapport annuel décrit les travaux de la société pendant toute l'année et les résultats obtenus."
            .to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).unwrap();
    let second = editor.store().snapshot().doc().clone();
    let kept = language(&second, Some(detections));
    assert_eq!(kept, language(&second, None));
    assert_eq!(
        language(&second, Some(detections)),
        kept,
        "read again from what it kept"
    );
}
