//! Fields in a text box (lane P5s): a `PAGE` or `NUMPAGES` field inserted in a paragraph of a
//! body text box (docx-agent's `generated/charts/chart-places.docx`, whose box the Rust
//! layout lays out since lane P5n) gets its result from the page the box's lines are drawn
//! on, as docx-agent's `_page_field` reads `layout().where` and as Word shows it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{engine, Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_fields, CallEditor, Editing, EnginePages, WordFields};
use ooxml_word_edit::Editor;
use serde_json::{json, Value};
use std::sync::Arc;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../python/docx-agent/tests/fixtures/generated/charts/chart-places.docx"
);

/// `word_fields` called with `arguments` on the fixture, laid out by a page engine; the
/// reply, and the fields of the version it made (keyword, result).
fn fields_after(arguments: Value) -> (Value, Vec<(String, String)>) {
    let package = Package::open(Arc::from(std::fs::read(FIXTURE).unwrap()), &Limits::default()).unwrap();
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let layout = EnginePages::of(engine());
    let caller = Caller::anonymous();
    let reply = {
        let call_editor = CallEditor::new(&editor);
        let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z")
            .with_layout(Some(&layout));
        let (args, _) = WordFields::accept(arguments, Some("docx")).unwrap();
        Value::Object(word_fields(&args, &editing).to_json())
    };
    let head = editor.store().snapshot();
    let found = ooxml_word_edit::fields(head.doc()).unwrap();
    (reply, found.into_iter().map(|f| (f.keyword, f.result)).collect())
}

#[test]
fn a_page_number_in_a_text_box_is_the_page_the_box_is_drawn_on() {
    // The box's first paragraph (p@body/2), at its start and inside its text; and the count.
    for (at, instruction, result) in [
        ("p@body/2@2", "PAGE", "1"),
        ("p@body/2@0", "PAGE", "1"),
        ("p@body/2", "NUMPAGES", "2"),
    ] {
        let (reply, fields) = fields_after(json!({"doc": "d1", "action": "insert_field", "at": at,
                                                  "instruction": instruction}));
        assert_eq!(reply["warnings"], json!([]), "{reply}");
        // The box is written twice (its DrawingML choice and its VML fallback), the field too.
        let wanted = (instruction.to_owned(), result.to_owned());
        assert_eq!(fields, [wanted.clone(), wanted], "{reply}");
    }
}
