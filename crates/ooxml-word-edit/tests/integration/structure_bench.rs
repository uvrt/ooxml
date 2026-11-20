//! The structure edits timed on a long document (lane P4b's hand-back): run with
//! `STRUCTURE_BENCH=<path to a .docx> cargo test --release -p ooxml-word-edit --test
//! integration -- --ignored --nocapture structure_bench::`; each edit's best of three runs,
//! each on the document as opened.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{best, editor};
use ooxml_word_edit::{
    AddBookmark, InsertNote, InsertSectionBreak, MoveBlocks, Moving, NoteKind, RevisionPolicy, SectionValues,
    SetSection, SetStoryText,
};
use std::sync::Arc;
use std::time::Duration;

#[test]
#[ignore = "a timing on a long document: STRUCTURE_BENCH=<docx> cargo test --release -- --ignored"]
fn structure_edits_on_a_long_document() {
    let Ok(path) = std::env::var("STRUCTURE_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let probe = editor(&bytes);
    let docx = probe.store().snapshot().doc().docx().unwrap();
    let ids: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .collect();
    let (early, middle, late) = (ids[10].clone(), ids[ids.len() / 2].clone(), ids[ids.len() - 10].clone());
    println!("{} body paragraphs", ids.len());
    let report = |name: &str, took: Duration| println!("{name}: {:.1} ms", took.as_secs_f64() * 1000.0);
    links_and_notes(&bytes, &middle, &report);
    sections_and_moves(&bytes, (&early, &late), &report);
}

fn links_and_notes(bytes: &Arc<[u8]>, middle: &str, report: &dyn Fn(&str, Duration)) {
    let untracked = RevisionPolicy::Untracked;
    report(
        "add_bookmark",
        best(
            bytes,
            &AddBookmark {
                place: format!("{middle}@0:3"),
                name: "Bench".into(),
            },
        ),
    );
    report(
        "insert_note",
        best(
            bytes,
            &InsertNote {
                kind: NoteKind::Footnote,
                at: format!("{middle}@2"),
                text: "A note.".into(),
                policy: untracked.clone(),
            },
        ),
    );
}

fn sections_and_moves(bytes: &Arc<[u8]>, (early, late): (&str, &str), report: &dyn Fn(&str, Duration)) {
    let untracked = RevisionPolicy::Untracked;
    report(
        "set_section s:body",
        best(
            bytes,
            &SetSection {
                section: "s:body".into(),
                values: SectionValues {
                    columns: Some(2),
                    ..SectionValues::default()
                },
                policy: untracked.clone(),
            },
        ),
    );
    report(
        "insert_section_break (early)",
        best(
            bytes,
            &InsertSectionBreak {
                after: Some(early.to_owned()),
                before: None,
                kind: "nextPage".into(),
                policy: untracked.clone(),
            },
        ),
    );
    report(
        "set header text",
        best(
            bytes,
            &SetStoryText {
                section: "s:body".into(),
                which: "header".into(),
                kind: "default".into(),
                text: "Bench".into(),
                policy: untracked.clone(),
            },
        ),
    );
    report(
        "move_blocks (early after late)",
        best(
            bytes,
            &MoveBlocks {
                blocks: Moving::Blocks(early.to_owned()),
                after: Some(late.to_owned()),
                before: None,
                policy: untracked,
            },
        ),
    );
}
