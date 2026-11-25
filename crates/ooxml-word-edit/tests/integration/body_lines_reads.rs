//! A table put on the body's lines (`FormatTable`'s `OnBody`, `width_of: "body"`) beside a
//! paragraph put between the table and the list item before it, interleaved by hand: the
//! format prepared and held, the insert committed meanwhile, then the format committed. The
//! lines are read in the format's transaction, so the insert is a conflict and the format,
//! prepared again, gives the width and indent of the two one at a time in commit order. A
//! width worked out before the transaction (as `word_format_table` did) is stale there.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use crate::interleave::interleaved;
use ooxml_store::Caller;
use ooxml_word_edit::{
    body_width, units::twips, Editor, FormatTable, InsertMarkdown, MarkdownOptions, RevisionPolicy, TableFormat,
    TableValues, TableWidth, WriteMap,
};
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::find_path;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// An editor of a document whose body ends with a list, then a table: the list's last
/// item's id and the table's.
fn opened() -> (Editor, String, String) {
    let shared = editor(&fixture("generated/ids-and-markup.docx"));
    let markdown = insert("end", "- one\n- two\n\n| a | b |\n|---|---|\n| c | d |\n");
    shared.store().run(&Caller::anonymous(), &markdown).unwrap();
    let docx = shared.store().snapshot().doc().docx().unwrap();
    let body = docx.body().unwrap();
    let table = body.ids.tables().iter().last().unwrap();
    let item = body
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == table.path.len() && p.path < table.path)
        .last()
        .unwrap()
        .id
        .clone();
    (shared, item, table.id)
}

fn insert(at: &str, markdown: &str) -> InsertMarkdown {
    InsertMarkdown {
        markdown: markdown.to_owned(),
        at: at.into(),
        style_map: WriteMap::default(),
        options: MarkdownOptions::default(),
        policy: RevisionPolicy::Untracked,
    }
}

/// A plain paragraph put after the list's last item: between it and the table.
fn between(item: &str) -> InsertMarkdown {
    insert(&format!("after:{item}"), "A plain paragraph.")
}

fn on_body(table: &str) -> FormatTable<'static> {
    let formats = vec![TableFormat::OnBody {
        percent: 100.0,
        alignment: None,
        values: TableValues::default(),
    }];
    FormatTable::new(Address::parse(table), formats, RevisionPolicy::Untracked)
}

/// The table's width and indent as its properties state them (`w:tblW`, `w:tblInd`).
fn width_and_indent(shared: &Editor, table: &str) -> (Option<String>, Option<String>) {
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
    let value = |local: &str| {
        find_path(&found.element, &["tblPr", local]).and_then(|e| e.attribute(Some(W), "w").map(str::to_owned))
    };
    (value("tblW"), value("tblInd"))
}

/// The insert, then the format, one at a time: what the table comes to.
fn serial(table: &str, item: &str) -> (Option<String>, Option<String>) {
    let (alone, _, _) = opened();
    alone.store().run(&Caller::named("between"), &between(item)).unwrap();
    alone.store().run(&Caller::named("last"), &on_body(table)).unwrap();
    width_and_indent(&alone, table)
}

#[test]
fn the_body_s_lines_read_in_the_transaction_conflict_with_a_paragraph_put_before_the_table() {
    let (shared, item, table) = opened();
    let (done, conflicts) = interleaved(&shared, on_body(&table), &between(&item));
    assert!(done);
    assert!(
        conflicts >= 1,
        "the insert between the item and the table is a conflict"
    );
    let expected = serial(&table, &item);
    assert_eq!(width_and_indent(&shared, &table), expected);
    // the list item's indent is not the plain paragraph's: the order matters
    let (first, _, _) = opened();
    first.store().run(&Caller::anonymous(), &on_body(&table)).unwrap();
    assert_ne!(width_and_indent(&first, &table), expected);
}

#[test]
fn a_width_worked_out_before_the_transaction_is_stale_after_a_paragraph_put_before_the_table() {
    // word_format_table before this fix: the lines read on the head, then the edit run
    let (shared, item, table) = opened();
    let head = shared.store().snapshot();
    let body = body_width(head.doc(), &table, 100.0, None).unwrap().unwrap();
    let mut formats: Vec<TableFormat> = body
        .columns
        .iter()
        .enumerate()
        .map(|(column, &width)| TableFormat::ColumnWidth {
            column,
            width: twips(width),
        })
        .collect();
    formats.push(TableFormat::Table(TableValues {
        width: Some(TableWidth::Length(twips(body.width))),
        alignment: Some("left".to_owned()),
        indent: Some(twips(body.indent)),
        ..TableValues::default()
    }));
    shared.store().run(&Caller::named("between"), &between(&item)).unwrap();
    let baked = FormatTable::new(Address::parse(&table), formats, RevisionPolicy::Untracked);
    shared.store().run(&Caller::named("last"), &baked).unwrap();
    assert_ne!(width_and_indent(&shared, &table), serial(&table, &item));
}
