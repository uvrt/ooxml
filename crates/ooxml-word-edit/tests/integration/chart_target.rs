//! A new chart's target: a row or cell is no block its paragraph goes after -- refused with
//! the table and the cell's paragraphs, as docx-agent's `insert_paragraph(after=)` refuses
//! it, before any part is made.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{ChartParts, EditError, InsertChart, RevisionPolicy};
use ooxml_xml::Tree;
use std::sync::Arc;

/// Parts never asked for: the target is refused first.
struct Unmade;

impl ChartParts for Unmade {
    fn parts(&self, _part: &str, _workbook_rel: &str) -> ooxml_word_edit::Result<(Tree, Arc<[u8]>)> {
        panic!("no chart is made for a refused target")
    }
}

#[test]
fn a_cell_is_refused_with_its_table_and_paragraphs() {
    let editor = editor(&fixture("generated/markdown/constructs.docx"));
    let command = InsertChart {
        at: ooxml_word_model::address::Address::from("t:6B000029/c0,0".to_owned()),
        chart: Unmade,
        size: (432.0, 252.0),
        policy: RevisionPolicy::Untracked,
    };
    let error = editor.run(&Caller::anonymous(), &command).unwrap_err();
    let EditError::Address(error) = error else {
        panic!("not an address refusal: {error:?}")
    };
    assert!(
        error
            .to_string()
            .starts_with("t:6B000029/c0,0 is a cell, not a paragraph or table: give its table t:6B000029 or a paragraph in it (p:6B00002A"),
        "{error}"
    );
}
