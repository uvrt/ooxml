//! What a document's checks keep across its versions read as a cold read does: each
//! table's cells (`PartCells::of`, kept by the table element) after an edit inside a
//! table, and the texts' languages (`Detections`) after a paragraph's language changed.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::facts::{text_language, Detections, LanguageScope};
use ooxml_word_edit::{RevisionPolicy, SetText, WordDoc};
use ooxml_word_model::docx::cells::PartCells;
use ooxml_word_model::docx::formatting::Formatting;

/// Every body paragraph's place, and the cell each is in as `cells` reads it.
fn cells_of(doc: &WordDoc, kept: bool) -> Vec<(Vec<u32>, Option<ooxml_word_model::docx::formatting::Cell>)> {
    let docx = doc.docx().unwrap();
    let body = docx.body().unwrap();
    let formatting = Formatting::read(&docx);
    let mut cells = if kept {
        PartCells::of(&formatting, body)
    } else {
        PartCells::new(&formatting, body.root())
    };
    body.ids
        .paragraphs()
        .iter()
        .map(|p| (p.path.to_vec(), cells.cell(&p.path)))
        .collect()
}

#[test]
fn a_table_s_cells_kept_across_versions_are_read_as_cold_after_an_edit_in_a_table() {
    let editor = editor(&fixture("generated/markdown/constructs.docx"));
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
        target: in_table,
        text: "Other words in the cell".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).unwrap();
    let second = editor.store().snapshot().doc().clone();
    assert_eq!(cells_of(&second, true), cells_of(&second, false));
    assert_eq!(cells_of(&first, true), read, "the version before reads as it did");
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
    let editor = editor(&fixture("samplelib/sample-long.docx"));
    let first = editor.store().snapshot().doc().clone();
    let detections = Detections::default();
    assert_eq!(language(&first, Some(&detections)), language(&first, None));
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
        target,
        text: "Le rapport annuel décrit les travaux de la société pendant toute l'année et les résultats obtenus."
            .to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    editor.run(&Caller::anonymous(), &command).unwrap();
    let second = editor.store().snapshot().doc().clone();
    let kept = language(&second, Some(&detections));
    assert_eq!(kept, language(&second, None));
    assert_eq!(
        language(&second, Some(&detections)),
        kept,
        "read again from what it kept"
    );
}
