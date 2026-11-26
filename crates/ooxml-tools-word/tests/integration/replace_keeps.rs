//! `word_insert_markdown` at `replace:` (round 60, R141; docx-agent 0.1's round 42 rules):
//! the replaced blocks go, untracked or as tracked deletions, and the new paragraphs keep the
//! replaced ones' paragraph settings by kind and position, the reply saying what each kept
//! (`data.kept`). docx-agent's own cases are `test_replace_keeps.py`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_format, word_insert_markdown, CallEditor, Editing, WordFormat, WordInsertMarkdown};
use ooxml_word_edit::{Editor, Tracking};
use serde_json::{json, Value};
use std::sync::Arc;

const FIXTURE: &[u8] = include_bytes!("../../../../crates/ooxml-oracle/fixtures/generated/lists-and-styles.docx");

/// The fixture with `markdown` at its end, then `call(editing, blocks)` made: the reply, and
/// the body's XML after it.
fn replaced(tracked: bool, markdown: &str, call: impl Fn(&Editing<'_>, &[String]) -> Value) -> (Value, String) {
    let package = Package::open(Arc::from(FIXTURE), &Limits::default()).unwrap();
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let tracking = Tracking {
        author: "Writer".to_owned(),
        date: "2026-11-26T10:00:00Z".to_owned(),
    };
    let mut editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-26T10:00:00Z");
    let made = insert(&editing, "end", markdown);
    let blocks: Vec<String> = made["data"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap().to_owned())
        .collect();
    if tracked {
        editing.tracking = Some(&tracking);
    }
    let reply = call(&editing, &blocks);
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let body = String::from_utf8(docx.body().unwrap().root().write()).unwrap();
    (reply, body)
}

fn format(editing: &Editing<'_>, targets: &[&str], settings: Value) {
    let mut args = settings;
    args["doc"] = json!("d1");
    args["targets"] = json!(targets);
    let result = word_format(&WordFormat::accept(args, Some("docx")).unwrap().0, editing);
    assert!(result.to_json().get("error").is_none(), "{:?}", result.to_json());
}

fn insert(editing: &Editing<'_>, at: &str, markdown: &str) -> Value {
    let args = json!({"doc": "d1", "at": at, "markdown": markdown});
    Value::Object(word_insert_markdown(&WordInsertMarkdown::accept(args, Some("docx")).unwrap().0, editing).to_json())
}

/// The `w:p` element whose text holds `text`, as written.
fn paragraph<'a>(body: &'a str, text: &str) -> &'a str {
    let at = body.find(text).unwrap_or_else(|| panic!("{text} not in the body"));
    let start = body[..at].rfind("<w:p ").or_else(|| body[..at].rfind("<w:p>")).unwrap();
    let end = body[at..].find("</w:p>").unwrap() + at;
    &body[start..end]
}

#[test]
fn new_paragraphs_keep_the_replaced_ones_settings_and_say_so() {
    let (reply, body) = replaced(false, "Alpha body.\n\nBravo body.\n\nAfter.", |editing, blocks| {
        format(
            editing,
            &[&blocks[0]],
            json!({"keep_with_next": true, "page_break_before": true}),
        );
        format(editing, &[&blocks[1]], json!({"keep_lines": true}));
        insert(
            editing,
            &format!("replace:{}..{}", blocks[0], blocks[1]),
            "New one.\n\nNew two.\n\nNew three.",
        )
    });
    assert_eq!(reply["data"]["blocks"].as_array().unwrap().len(), 3, "{reply}");
    let kept = reply["data"]["kept"].as_array().unwrap();
    assert_eq!(kept.len(), 3, "{reply}");
    assert_eq!(kept[0]["block"], reply["data"]["blocks"][0], "{reply}");
    assert_eq!(
        kept[0]["settings"],
        json!(["keep_with_next", "page_break_before"]),
        "{reply}"
    );
    assert_eq!(kept[1]["settings"], json!(["keep_lines"]), "{reply}");
    assert!(
        reply["summary"]
            .as_str()
            .unwrap()
            .ends_with("3 kept the replaced paragraphs' settings"),
        "{reply}"
    );
    assert!(paragraph(&body, "New one.").contains("<w:pageBreakBefore/>"), "{body}");
    // the last donor taken again: its settings, no second page break
    assert!(paragraph(&body, "New three.").contains("<w:keepLines/>"), "{body}");
    assert!(!body.contains("Alpha body.") && !body.contains("Bravo body."), "{body}");
    assert!(body.contains("After."), "{body}");
}

#[test]
fn a_tracked_replacement_deletes_the_old_blocks_and_inserts_the_new_after_them() {
    let (reply, body) = replaced(true, "Alpha body.\n\nAfter.", |editing, blocks| {
        format(editing, &[&blocks[0]], json!({"keep_with_next": true}));
        insert(editing, &format!("replace:{}", blocks[0]), "Tracked new.")
    });
    assert_eq!(
        reply["data"]["kept"][0]["settings"],
        json!(["keep_with_next"]),
        "{reply}"
    );
    let old = body.find("Alpha body.").expect("the old text stays, deleted");
    let new = body.find("Tracked new.").unwrap();
    assert!(old < new, "the new paragraph goes after the deleted one: {body}");
    assert!(paragraph(&body, "Alpha body.").contains("<w:delText"), "{body}");
    assert!(paragraph(&body, "Tracked new.").contains("<w:ins "), "{body}");
    assert!(paragraph(&body, "Tracked new.").contains("<w:keepNext/>"), "{body}");
}

#[test]
fn a_replacement_writing_nothing_still_takes_the_blocks_away() {
    let (reply, body) = replaced(false, "Alpha body.\n\nAfter.", |editing, blocks| {
        insert(editing, &format!("replace:{}", blocks[0]), "<!-- nothing -->")
    });
    assert!(reply.get("error").is_none(), "{reply}");
    assert!(!body.contains("Alpha body.") && body.contains("After."), "{body}");
}

#[test]
fn the_paragraph_after_a_last_table_is_replaced_with_the_new_one_placed_first() {
    // docx-agent's order, untracked: the new blocks go in before the old ones go, so the body
    // never ends in the table
    let (reply, body) = replaced(false, "| a | b |\n|---|---|\n| 1 | 2 |\n\nTail.", |editing, blocks| {
        insert(editing, &format!("replace:{}", blocks.last().unwrap()), "New tail.")
    });
    assert!(reply.get("error").is_none(), "{reply}");
    assert!(!body.contains("Tail.<") && body.contains("New tail."), "{body}");
}

#[test]
fn kept_names_the_replaced_paragraph_by_the_id_it_was_stamped_with() {
    let (reply, _) = replaced(false, "Alpha body.\n\nAfter.", |editing, blocks| {
        format(editing, &[&blocks[0]], json!({"keep_with_next": true}));
        insert(editing, &format!("replace:{}", blocks[0]), "New.")
    });
    let from = reply["data"]["kept"][0]["from"].as_str().unwrap();
    assert!(from.starts_with("p:"), "{reply}");
}
