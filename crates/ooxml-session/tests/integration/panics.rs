//! A panic inside a tool is the session's own reply (lane Z1): an `internal` failure saying
//! whether the call changed the document, the call again as its way out, `details.trace`
//! naming the tool and what the panic said, the message in the application part, and
//! `panics{tool}` counted. The session goes on serving calls.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::editing::{context, docx};
use ooxml_base::Cancel;
use ooxml_session::{CallContext, DocKind, EditHandler, ReadHandler, Registry, Session, SessionConfig};
use ooxml_tools::{Done, ToolError, ToolResult};
use ooxml_tools_word::{word_set_text, Describe, Editing, Version, WordSetText};
use serde_json::{json, Value};
use std::sync::Arc;

fn boom_read(_: &Describe, _: &Version<'_>, _: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    panic!("a read that panics")
}

/// An edit that panics before it changes anything.
fn boom_edit(_: &WordSetText, _: &Editing<'_>) -> ToolResult<Value> {
    panic!("an edit that panics")
}

/// An edit that panics after it changed the document.
fn boom_after(args: &WordSetText, editing: &Editing<'_>) -> ToolResult<Value> {
    let done = word_set_text(args, editing);
    assert!(!done.is_error());
    panic!("after the edit")
}

fn with(register: impl FnOnce(&mut Registry)) -> Session {
    let mut registry = Registry::empty();
    register(&mut registry);
    Session::with_registry(SessionConfig::default(), context(), registry)
}

#[test]
fn a_read_that_panics_is_an_internal_failure_and_counted() {
    let s = with(|r| r.register(Arc::new(ReadHandler::<Describe>::new(&[DocKind::Docx], boom_read))));
    s.open(docx("<w:p/>"), DocKind::Docx, "a.docx").unwrap();
    let result = s.call("describe", json!({"doc": "d1"}), &CallContext::default());
    assert_eq!(result.app.panic.as_deref(), Some("a read that panics"));
    let reply = Value::Object(result.to_json());
    assert_eq!(reply["error"]["code"], json!("internal"));
    let message = reply["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("the call failed inside the library; nothing changed"),
        "{message}"
    );
    assert_eq!(
        reply["error"]["details"]["trace"],
        json!("panic:describe:1: a read that panics")
    );
    assert_eq!(
        reply["error"]["try_call"],
        json!({"tool": "describe", "arguments": {"doc": "d1"}})
    );
    // arguments as JSON text, as providers send them: the same way out; the count goes on
    let again = s.call(
        "describe",
        Value::String(r#"{"doc":"d1"}"#.into()),
        &CallContext::default(),
    );
    assert_eq!(
        Value::Object(again.to_json())["error"]["try_call"]["arguments"],
        json!({"doc": "d1"})
    );
    let stats = s.stats();
    assert_eq!(stats.panics.get("describe"), Some(&2));
    assert_eq!(stats.internal.get("describe"), Some(&2));
    assert_eq!((stats.calls, stats.failed), (2, 2));
}

/// An edit whose core finds its own model and tree disagreeing.
fn mismatched(_: &WordSetText, _: &Editing<'_>) -> ToolResult<Value> {
    let error = ooxml_word_edit::EditError::no_element("word/document.xml");
    ToolResult::failed(ToolError::internal(error.to_string(), "word:edit"))
}

#[test]
fn an_internal_failure_is_counted_and_a_refusal_is_not() {
    let s = with(|r| {
        r.register(Arc::new(EditHandler::<WordSetText>::new(&[DocKind::Docx], mismatched)));
    });
    let body = r#"<w:p w14:paraId="11111111" w14:textId="77777777"><w:r><w:t>one</w:t></w:r></w:p>"#;
    s.open(docx(body), DocKind::Docx, "a.docx").unwrap();
    let asked = json!({"doc": "d1", "target": "p:11111111", "text": "two"});
    let done = s.call("word_set_text", asked, &CallContext::default());
    let reply = Value::Object(done.to_json());
    assert_eq!(reply["error"]["code"], "internal");
    assert_eq!(
        reply["error"]["message"],
        "no element at the place found in word/document.xml"
    );
    let stats = s.stats();
    assert_eq!(stats.internal.get("word_set_text"), Some(&1));
    assert!(stats.panics.is_empty());
    // a refusal is the caller's: failed, not internal
    let refused = s.call(
        "word_set_text",
        json!({"doc": "nope", "target": "p:11111111", "text": "x"}),
        &CallContext::default(),
    );
    assert_eq!(Value::Object(refused.to_json())["error"]["code"], "not_found");
    let stats = s.stats();
    assert_eq!((stats.failed, stats.internal.get("word_set_text")), (2, Some(&1)));
}

#[test]
fn an_edit_that_panics_says_whether_it_changed_the_document() {
    let s = with(|r| {
        r.register(Arc::new(EditHandler::<WordSetText>::new(&[DocKind::Docx], boom_edit)));
    });
    let body = r#"<w:p w14:paraId="11111111" w14:textId="77777777"><w:r><w:t>one</w:t></w:r></w:p>"#;
    let d = s.open(docx(body), DocKind::Docx, "a.docx").unwrap();
    let asked = json!({"doc": "d1", "target": "p:11111111", "text": "two"});
    let (result, made) = s.edit("word_set_text", asked.clone(), &CallContext::default());
    assert!(made.is_none());
    let reply = Value::Object(result.to_json());
    assert_eq!(reply["error"]["code"], json!("internal"));
    assert!(reply["error"]["message"].as_str().unwrap().contains("nothing changed"));
    assert_eq!(s.snapshot(&d).unwrap().version(), 0);

    let s = with(|r| {
        r.register(Arc::new(EditHandler::<WordSetText>::new(&[DocKind::Docx], boom_after)));
    });
    let d = s.open(docx(body), DocKind::Docx, "a.docx").unwrap();
    let (result, made) = s.edit("word_set_text", asked, &CallContext::default());
    // what it changed stays, as its step, and the reply says so
    assert!(made.is_some());
    assert_eq!(s.snapshot(&d).unwrap().version(), 1);
    let reply = Value::Object(result.to_json());
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .contains("what it changed before it failed stays"));
    assert_eq!(s.stats().panics.get("word_set_text"), Some(&1));
}
