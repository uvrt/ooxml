//! The session's own page engines (lane H7b): a document laid out by the engine the session
//! keeps for it ([`SessionPages`]), made once and remade when the host sets other fonts for
//! it; `word_inspect`'s placements from that layout where the host gives no placer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::locale::{docx, session_with, test_fonts};
use ooxml_session::{CallContext, DocKind, MemoryBudget, SessionConfig, SessionPages};
use ooxml_tools_word::PageSetup;
use ooxml_word_pages::EngineOptions;
use serde_json::{json, Value};
use std::sync::Arc;

fn pages() -> SessionPages {
    SessionPages {
        setup: PageSetup::new(test_fonts(), EngineOptions::default()),
        memory: MemoryBudget::new(64 << 20),
    }
}

fn inspect(session: &ooxml_session::Session) -> Value {
    let arguments = json!({"doc": "d1", "target": "body", "layout": true});
    Value::Object(
        session
            .call("word_inspect", arguments, &CallContext::default())
            .to_json(),
    )
}

#[test]
fn a_session_without_pages_keeps_no_engine_and_refuses_placements() {
    let s = session_with(SessionConfig::default());
    let d = s.open(docx(None), DocKind::Docx, "d.docx").unwrap();
    assert!(s.pages(&d).unwrap().is_none());
    assert_eq!(inspect(&s)["error"]["code"], json!("refused"));
}

#[test]
fn the_session_keeps_one_engine_per_document_and_places_blocks_with_it() {
    let s = session_with(SessionConfig {
        pages: Some(pages()),
        ..SessionConfig::default()
    });
    let d = s.open(docx(None), DocKind::Docx, "d.docx").unwrap();
    let first = s.pages(&d).unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &s.pages(&d).unwrap().unwrap()));
    let read = inspect(&s);
    assert!(read.get("error").is_none(), "{read}");
    let blocks = read["data"]["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 3);
    let mut last_bottom = 0.0;
    for block in blocks {
        let placed = block["placements"].as_array().unwrap();
        assert_eq!(placed.len(), 1, "{block}");
        let one = &placed[0];
        assert_eq!(
            (one["page"].clone(), one["story"].clone(), one["column"].clone()),
            (json!(1), json!("body"), json!(0))
        );
        assert_eq!(one["lines"], json!(1));
        let (top, bottom) = (one["top"].as_f64().unwrap(), one["bottom"].as_f64().unwrap());
        assert!(top >= last_bottom && bottom > top, "{block}");
        last_bottom = bottom;
    }
    // other fonts for this document: its engine made again; none set, the session's again
    s.set_pages(&d, Some(pages())).unwrap();
    let second = s.pages(&d).unwrap().unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    s.set_pages(&d, None).unwrap();
    assert!(!Arc::ptr_eq(&second, &s.pages(&d).unwrap().unwrap()));
}

#[test]
fn a_host_placer_comes_before_the_session_s_layout() {
    struct Nowhere;
    impl ooxml_tools_word::Placer for Nowhere {
        fn place(&self, asked: &[Vec<String>]) -> Option<Vec<serde_json::Map<String, Value>>> {
            let unknown = json!({"unknown": {"reason": "host", "after_page": null}});
            Some(asked.iter().map(|_| unknown.as_object().cloned().unwrap()).collect())
        }
    }
    let s = session_with(SessionConfig {
        pages: Some(pages()),
        ..SessionConfig::default()
    });
    s.open(docx(None), DocKind::Docx, "d.docx").unwrap();
    let context = CallContext {
        placer: Some(Arc::new(Nowhere)),
        ..CallContext::default()
    };
    let arguments = json!({"doc": "d1", "target": "body", "layout": true});
    let read = Value::Object(s.call("word_inspect", arguments, &context).to_json());
    assert_eq!(read["data"]["blocks"][0]["unknown"]["reason"], json!("host"));
}
