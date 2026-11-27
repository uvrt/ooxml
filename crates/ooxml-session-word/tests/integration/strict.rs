//! A strict Open XML document in the session (phase 3 RD1a): every tool but `render`
//! refuses it, reading and changing alike, with the one refusal whose way out is the
//! document saved as a Word Document (.docx) in Word; no change applies to it, so its bytes
//! save as they came. Before RD1a the reading tools saw an empty document and an edit wrote
//! a paragraph Word refuses to open (Word 16.106: the repair prompt,
//! `crates/ooxml-oracle/word/observations/strict-word.json`, "edited").

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_session::{CallContext, DocKind};
use ooxml_tools_word::strict::STRICT;
use serde_json::{json, Value};
use std::sync::Arc;

const PROBES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ooxml-oracle/tests/data/word-probes/make_strict_probe"
);

fn twin(name: &str) -> Vec<u8> {
    let path = format!("{PROBES}/{name}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The not-supported form a call's reply refuses, if it refuses one.
fn refused_form(reply: &serde_json::Map<String, Value>) -> Option<String> {
    reply
        .get("error")?
        .get("details")?
        .get("not_supported")?
        .as_str()
        .map(str::to_owned)
}

#[test]
fn every_tool_but_render_refuses_a_strict_document_and_it_saves_as_it_came() {
    let bytes = twin("strict.docx");
    let session = crate::editing::session();
    let id = session
        .open(Arc::from(bytes.clone()), DocKind::Docx, "strict.docx")
        .unwrap();
    let cx = CallContext::default();
    let calls = [
        ("describe", json!({"doc": "d1"})),
        ("word_read", json!({"doc": "d1"})),
        ("word_inspect", json!({"doc": "d1", "target": "body"})),
        ("find_text", json!({"doc": "d1", "text": "Lists"})),
        ("check", json!({"doc": "d1"})),
        (
            "word_insert_markdown",
            json!({"doc": "d1", "at": "end", "markdown": "Added **here**."}),
        ),
    ];
    for (tool, arguments) in calls {
        let reply = session.call(tool, arguments, &cx).to_json();
        assert_eq!(refused_form(&reply).as_deref(), Some(STRICT), "{tool}: {reply:?}");
        let message = reply["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("strict Open XML"), "{tool}: {message}");
        let way_out = reply["error"]["valid_options"].to_string();
        assert!(way_out.contains("Word Document (.docx)"), "{tool}: {way_out}");
    }
    // render lays it out: whatever it answers here (no raster in this session), it is not
    // the strict refusal
    let render = session
        .call("render", json!({"doc": "d1", "pages": [1]}), &cx)
        .to_json();
    assert_ne!(refused_form(&render).as_deref(), Some(STRICT), "{render:?}");
    // the insert that used to write a file Word refuses to open wrote nothing
    assert_eq!(session.save(&id).unwrap(), bytes);
}

#[test]
fn its_transitional_twin_is_read_and_edited() {
    let session = crate::editing::session();
    session
        .open(Arc::from(twin("transitional.docx")), DocKind::Docx, "transitional.docx")
        .unwrap();
    let cx = CallContext::default();
    let described = session.call("describe", json!({"doc": "d1"}), &cx).to_json();
    assert!(described["ok"].as_bool().unwrap_or_default(), "{described:?}");
    assert!(!described["data"]["headings"].as_array().unwrap().is_empty());
    let inserted = session
        .call(
            "word_insert_markdown",
            json!({"doc": "d1", "at": "end", "markdown": "Added."}),
            &cx,
        )
        .to_json();
    assert!(inserted["ok"].as_bool().unwrap_or_default(), "{inserted:?}");
}
