//! AutoFit to Window reads the table as the call has written it so far: a call that sets
//! the table's borders and then fits it to the window writes what the two calls one after
//! the other write (the room the borders' outer halves take is the new borders'), not what
//! fitting the table as it stood before the call writes (lane ph3-pj, PJ1b-2: the fit
//! resolves a table in no version's reading from its structure, `resolved_shape`).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{
    Editor, FormatTable, InsertMarkdown, MarkdownOptions, RevisionPolicy, TableFormat, TableValues, WriteMap,
};
use ooxml_word_model::address::Address;

/// An editor of a document ending with a two-by-two table, and the table's id.
fn opened() -> (Editor, String) {
    let shared = editor(&fixture("generated/ids-and-markup.docx"));
    let markdown = InsertMarkdown {
        markdown: "| a | b |\n|---|---|\n| c | d |\n".to_owned(),
        at: "end".into(),
        style_map: WriteMap::default(),
        options: MarkdownOptions::default(),
        policy: RevisionPolicy::Untracked,
    };
    shared.store().run(&Caller::anonymous(), &markdown).unwrap();
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let table = docx.body().unwrap().ids.tables().iter().last().unwrap().id;
    (shared, table)
}

fn format(shared: &Editor, table: &str, formats: Vec<TableFormat>) {
    let command = FormatTable::new(Address::parse(table), formats, RevisionPolicy::Untracked);
    shared.store().run(&Caller::anonymous(), &command).unwrap();
}

fn borders() -> TableFormat {
    TableFormat::Table(TableValues {
        borders: Some("double".to_owned()),
        ..TableValues::default()
    })
}

fn window() -> TableFormat {
    TableFormat::Window { percent: 100.0 }
}

/// The table's XML as the editor holds it.
fn table_xml(shared: &Editor, table: &str) -> String {
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let found = docx
        .body()
        .unwrap()
        .ids
        .tables()
        .iter()
        .find(|t| t.id == table)
        .unwrap()
        .clone();
    String::from_utf8(found.element.write()).unwrap()
}

#[test]
fn a_window_fit_after_borders_in_one_call_reads_the_borders_the_call_wrote() {
    let (together, table) = opened();
    format(&together, &table, vec![borders(), window()]);
    let (apart, apart_table) = opened();
    format(&apart, &apart_table, vec![borders()]);
    format(&apart, &apart_table, vec![window()]);
    let (before, before_table) = opened();
    format(&before, &before_table, vec![window()]);
    format(&before, &before_table, vec![borders()]);
    let (one, two, stale) = (
        table_xml(&together, &table),
        table_xml(&apart, &apart_table),
        table_xml(&before, &before_table),
    );
    assert_eq!(one, two, "one call writes what the two do in order");
    assert_ne!(
        one, stale,
        "a double border's outer half takes room the fit gives the cells otherwise"
    );
}
