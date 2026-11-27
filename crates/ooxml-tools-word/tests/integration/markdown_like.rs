//! `word_insert_markdown` `like` (round 60, R141; docx-agent 0.1's round 42 rules): new list
//! items take the look of the paragraph given for their level -- its style, its list at its
//! level, its indents and page settings -- a level below the given ones the list a level
//! deeper, a style map's list role naming a paragraph read as like; what a sample cannot give
//! said, a table's frame still refused; in a one-level list a level below is the item moved
//! as Word's list indent moves it (F-2c). docx-agent's own cases are `test_like.py`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_format, word_insert_markdown, CallEditor, Editing, WordFormat, WordInsertMarkdown};
use ooxml_word_edit::Editor;
use serde_json::{json, Value};
use std::sync::Arc;

const FIXTURE: &[u8] = include_bytes!("../../../../crates/ooxml-oracle/fixtures/generated/lists-and-styles.docx");

/// The fixture with a made-up nested list and a paragraph at its end (`Point one` with its
/// example a level down, `Point two`, then `Plain.`), then `call(editing, blocks)` made: the
/// reply, and the body's XML after it.
fn listed(call: impl Fn(&Editing<'_>, &[String]) -> Value) -> (Value, String) {
    let package = Package::open(Arc::from(FIXTURE), &Limits::default()).unwrap();
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-26T10:00:00Z");
    let made = insert(
        &editing,
        json!({"at": "end", "markdown": "- Point one\n  - Example one\n- Point two\n\nPlain.\n"}),
    );
    let blocks: Vec<String> = made["data"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(blocks.len(), 4, "{made}");
    let reply = call(&editing, &blocks);
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let body = String::from_utf8(docx.body().unwrap().root().write()).unwrap();
    (reply, body)
}

fn insert(editing: &Editing<'_>, arguments: Value) -> Value {
    let mut args = arguments;
    args["doc"] = json!("d1");
    Value::Object(word_insert_markdown(&WordInsertMarkdown::accept(args, Some("docx")).unwrap().0, editing).to_json())
}

fn format(editing: &Editing<'_>, targets: &[&str], settings: Value) {
    let mut args = settings;
    args["doc"] = json!("d1");
    args["targets"] = json!(targets);
    let result = word_format(&WordFormat::accept(args, Some("docx")).unwrap().0, editing);
    assert!(result.to_json().get("error").is_none(), "{:?}", result.to_json());
}

/// The `w:pPr` of the paragraph whose text holds `text`, as written (empty: none).
fn properties<'a>(body: &'a str, text: &str) -> &'a str {
    let at = body.find(text).unwrap_or_else(|| panic!("{text} not in the body"));
    let start = body[..at].rfind("<w:p ").or_else(|| body[..at].rfind("<w:p>")).unwrap();
    let paragraph = &body[start..at];
    match (paragraph.find("<w:pPr>"), paragraph.find("</w:pPr>")) {
        (Some(open), Some(close)) => &paragraph[open..close],
        _ => "",
    }
}

/// The value of the `w:<local>` element's `w:val` in `properties`.
fn val(properties: &str, local: &str) -> Option<String> {
    let at = properties.find(&format!("<w:{local} "))?;
    let rest = &properties[at..];
    let value = rest.split("w:val=\"").nth(1)?;
    Some(value.split('"').next()?.to_owned())
}

#[test]
fn new_items_take_each_given_levels_look() {
    let (reply, body) = listed(|editing, blocks| {
        format(
            editing,
            &[&blocks[1]],
            json!({"keep_with_next": true, "indent_left": 40}),
        );
        insert(
            editing,
            json!({"at": format!("after:{}", blocks[2]), "markdown": "- Point three\n  - Example two\n",
                   "like": [blocks[0], blocks[1]]}),
        )
    });
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(
        properties(&body, "Point three"),
        properties(&body, "Point one"),
        "{body}"
    );
    assert_eq!(
        properties(&body, "Example two"),
        properties(&body, "Example one"),
        "{body}"
    );
    assert!(properties(&body, "Example two").contains("<w:keepNext/>"), "{body}");
    // like is not a replacement's keeps: nothing is said kept
    assert!(reply["data"].get("kept").is_none(), "{reply}");
}

#[test]
fn a_level_below_the_given_ones_is_the_samples_list_a_level_deeper() {
    let (reply, body) = listed(|editing, blocks| {
        insert(
            editing,
            json!({"at": format!("after:{}", blocks[2]), "markdown": "- Point three\n  - Deeper\n",
                   "like": blocks[1]}),
        )
    });
    assert!(reply.get("error").is_none(), "{reply}");
    let sample = properties(&body, "Example one");
    let deeper = properties(&body, "Deeper");
    let level = |p: &str| val(p, "ilvl").unwrap().parse::<i64>().unwrap();
    assert_eq!(level(deeper), level(sample) + 1, "{body}");
    assert_eq!(val(deeper, "numId"), val(sample, "numId"), "{body}");
    // Word's Tab: the style stays, the level's own indent (no direct one)
    assert_eq!(val(deeper, "pStyle"), val(sample, "pStyle"), "{body}");
    assert!(!deeper.contains("<w:ind "), "{body}");
}

#[test]
fn a_sample_in_no_list_and_one_of_the_other_kind_are_said() {
    let (reply, body) = listed(|editing, blocks| {
        insert(
            editing,
            json!({"at": format!("after:{}", blocks[3]), "markdown": "- Item\n", "like": blocks[3]}),
        )
    });
    let warnings = reply["warnings"].to_string();
    assert!(
        warnings.contains("is in no list: the level 1 items are written like it"),
        "{reply}"
    );
    assert!(!properties(&body, "Item").contains("<w:numPr>"), "{body}");
    let (reply, _) = listed(|editing, blocks| {
        insert(
            editing,
            json!({"at": format!("after:{}", blocks[3]), "markdown": "1. Item\n", "like": [blocks[0]]}),
        )
    });
    let warnings = reply["warnings"].to_string();
    assert!(
        warnings.contains("level 1 items are numbers and") && warnings.contains("is a bullet item"),
        "{reply}"
    );
}

#[test]
fn a_numbered_list_starting_past_one_continues_the_samples_numbering_and_says_so() {
    let (reply, _) = listed(|editing, blocks| {
        insert(
            editing,
            json!({"at": format!("after:{}", blocks[3]), "markdown": "3. Item\n", "like": [blocks[0]]}),
        )
    });
    assert!(
        reply["warnings"]
            .to_string()
            .contains("like: a numbered list starting at 3 continues"),
        "{reply}"
    );
}

#[test]
fn a_style_map_naming_a_paragraph_for_a_list_role_is_read_as_like() {
    let (reply, body) = listed(|editing, blocks| {
        insert(
            editing,
            json!({"at": format!("after:{}", blocks[2]), "markdown": "- Point three\n  - Example two\n",
                   "style_map": [{"element": "bullet2", "style": blocks[1]}]}),
        )
    });
    assert!(reply.get("error").is_none(), "{reply}");
    let first = reply["warnings"][0].as_str().unwrap();
    assert!(
        first.starts_with("style_map bullet2: ")
            && first.ends_with(" is a paragraph: read as like, its items looking like it"),
        "{reply}"
    );
    assert_eq!(
        properties(&body, "Example two"),
        properties(&body, "Example one"),
        "{body}"
    );
}

#[test]
fn an_empty_level_is_written_without_like() {
    let (reply, body) = listed(|editing, blocks| {
        insert(
            editing,
            json!({"at": format!("after:{}", blocks[3]), "markdown": "- Top\n  - Nested\n",
                   "like": ["", blocks[1]]}),
        )
    });
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(properties(&body, "Nested"), properties(&body, "Example one"), "{body}");
}

#[test]
fn a_sample_of_several_paragraphs_is_refused_with_them() {
    let (reply, _) = listed(|editing, blocks| {
        insert(
            editing,
            json!({"at": "end", "markdown": "- Item\n", "like": format!("{}..{}", blocks[0], blocks[2])}),
        )
    });
    let error = &reply["error"];
    assert_eq!(error["code"], "invalid_arguments", "{reply}");
    assert_eq!(error["field"], "like", "{reply}");
    assert!(
        error["message"].as_str().unwrap().contains("holds 3: give one of them"),
        "{reply}"
    );
    assert_eq!(error["valid_options"].as_array().unwrap().len(), 3, "{reply}");
}

#[test]
fn a_table_or_a_sample_by_reference_is_refused_as_not_supported_yet() {
    for like in ["t:1A2B3C4D", "$sample"] {
        let (reply, body) =
            listed(|editing, _| insert(editing, json!({"at": "end", "markdown": "- Item\n", "like": like})));
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("is not supported yet"),
            "{reply}"
        );
        assert!(!body.contains("Item<"), "{body}");
    }
}

#[test]
fn like_wins_over_a_replaced_items_kept_settings() {
    let (reply, body) = listed(|editing, blocks| {
        format(editing, &[&blocks[1]], json!({"keep_with_next": true}));
        insert(
            editing,
            json!({"at": format!("replace:{}", blocks[1]), "markdown": "- Now a point\n", "like": blocks[0]}),
        )
    });
    assert!(reply.get("error").is_none(), "{reply}");
    assert!(reply["data"].get("kept").is_none(), "{reply}");
    assert_eq!(
        properties(&body, "Now a point"),
        properties(&body, "Point one"),
        "{body}"
    );
}

/// The fixture's body paragraphs used below: its one-level list's items (`Bullet from a
/// style`, `Another bullet from the style`) and the paragraph after them.
const BULLET: &str = "p:5A000009";
const ANOTHER: &str = "p:5A00000A";
const AFTER: &str = "p:5A00000B";

/// The fixture with `call(editing)` made: the reply, and the body's XML after it.
fn on_fixture(call: impl Fn(&Editing<'_>) -> Value) -> (Value, String) {
    let package = Package::open(Arc::from(FIXTURE), &Limits::default()).unwrap();
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-27T10:00:00Z");
    let reply = call(&editing);
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let body = String::from_utf8(docx.body().unwrap().root().write()).unwrap();
    (reply, body)
}

/// The `w:<local>` element's attribute `name` in `properties`.
fn attribute(properties: &str, local: &str, name: &str) -> Option<String> {
    let at = properties.find(&format!("<w:{local} "))?;
    let element = &properties[at..at + properties[at..].find('>')?];
    let value = element.split(&format!("w:{name}=\"")).nth(1)?;
    Some(value.split('"').next()?.to_owned())
}

/// The fixture's one-level list (Word's built-in List Bullet, Dutch `Lijstopsomteken`: a
/// single-level definition, left 360 hanging 360; defaultTabStop 708), items a level and two
/// levels down written after it `like` its first item: the reply and each new item's
/// properties.
fn one_level(before_next: Option<Value>) -> (Value, Vec<String>) {
    let (reply, body) = on_fixture(|editing| {
        if let Some(settings) = &before_next {
            format(editing, &[AFTER], settings.clone());
        }
        insert(
            editing,
            json!({"at": format!("after:{ANOTHER}"), "markdown": "- Third\n  - Below\n    - Lower\n",
                   "like": [BULLET]}),
        )
    });
    let items = ["Third", "Below", "Lower"]
        .map(|t| properties(&body, t).to_owned())
        .to_vec();
    (reply, items)
}

#[test]
fn a_level_below_a_one_level_list_keeps_its_style_and_moves_as_words_list_indent() {
    let (reply, items) = one_level(None);
    assert!(reply.get("error").is_none(), "{reply}");
    // Word's answer, not 0.1's (List Bullet 2 and a warning)
    assert_eq!(reply["warnings"], json!([]), "{reply}");
    for item in &items {
        assert_eq!(val(item, "pStyle").as_deref(), Some("Lijstopsomteken"), "{item}");
        assert!(!item.contains("<w:numPr>"), "{item}");
    }
    assert!(
        !items[0].contains("<w:ind ") && !items[0].contains("<w:tabs>"),
        "{}",
        items[0]
    );
    // measured: left 720 then 1068 (the default stop 708 plus the hanging), a num stop there,
    // the hanging the level's
    for (item, left) in items[1..].iter().zip(["720", "1068"]) {
        assert_eq!(attribute(item, "ind", "left").as_deref(), Some(left), "{item}");
        assert_eq!(attribute(item, "ind", "hanging"), None, "{item}");
        assert_eq!(attribute(item, "tab", "val").as_deref(), Some("num"), "{item}");
        assert_eq!(attribute(item, "tab", "pos").as_deref(), Some(left), "{item}");
    }
    // the tabs before the indent, in schema order
    let tabs = items[1].find("<w:tabs>").unwrap();
    assert!(tabs < items[1].find("<w:ind ").unwrap(), "{}", items[1]);
}

#[test]
fn the_paragraph_after_the_new_items_is_a_stop_at_its_first_line() {
    // its first line at 600 twips (30 pt): the item two levels down stops there
    let (reply, items) = one_level(Some(json!({"indent_left": 30})));
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(
        attribute(&items[1], "ind", "left").as_deref(),
        Some("720"),
        "{}",
        items[1]
    );
    assert_eq!(
        attribute(&items[2], "ind", "left").as_deref(),
        Some("960"),
        "{}",
        items[2]
    );
}

#[test]
fn a_sample_with_its_own_indent_in_a_one_level_list_is_still_said() {
    // not measured: 0.1's answer and its warning
    let (reply, body) = on_fixture(|editing| {
        format(editing, &[BULLET], json!({"indent_left": 30}));
        insert(
            editing,
            json!({"at": format!("after:{ANOTHER}"), "markdown": "- Third\n  - Below\n", "like": [BULLET]}),
        )
    });
    assert!(
        reply["warnings"].to_string().contains("has no level 1 below"),
        "{reply}"
    );
    assert!(!properties(&body, "Below").contains("<w:tabs>"), "{body}");
}
