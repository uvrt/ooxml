//! The session tools and the application's documents (ooxml-edit's `test_tools_doc_ids`,
//! `test_tools_open_state` and `test_tools_shared`'s session-tool tests, their generic parts
//! ported): opening under the application's ids and with a saved state, `open_document`,
//! `new_document`, `close_document`, `read_blob`, and what the host adds to a format tool's
//! reply (the save gate and its output, check's validation, describe's problems).

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, empty_session, package, package_with};
use ooxml_host::{HostSettings, OpenRefused, Opening};
use ooxml_session::{DocId, DocKind};
use serde_json::{json, Value};
use std::sync::Arc;

fn opening<'a>(doc_id: Option<&'a str>, state: Option<&'a str>) -> Opening<'a> {
    Opening {
        doc_id,
        state,
        ..Opening::default()
    }
}

fn id(doc: &str) -> DocId {
    DocId::parse(doc).unwrap()
}

#[test]
fn a_resumed_run_reopens_its_documents_under_their_earlier_ids() {
    let s = empty_session(HostSettings::default());
    assert_eq!(
        s.open_with(package(), "text.pptx", &opening(Some("d2"), None)).unwrap(),
        id("d2")
    );
    assert_eq!(
        s.open_with(package(), "deck.pptx", &opening(Some("d1"), None)).unwrap(),
        id("d1")
    );
    let set = call(&s, "toy_set", json!({"doc": "d1", "target": "a", "text": "x"}));
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(
        s.open_with(package(), "more.pptx", &Opening::default()).unwrap(),
        id("d3")
    );
}

#[test]
fn an_id_in_use_is_refused_naming_its_document_and_the_way_out() {
    let s = empty_session(HostSettings::default());
    s.open_with(package(), "deck.pptx", &Opening::default()).unwrap();
    s.open_with(package(), "text.pptx", &Opening::default()).unwrap();
    let refused = s
        .open_with(package(), "other.pptx", &opening(Some("d1"), None))
        .unwrap_err();
    assert_eq!(
        refused,
        OpenRefused::Argument(
            "doc_id 'd1' is in use: deck.pptx (pptx) is open as d1; close it first (session.close('d1'), after \
             saving it if it changed), or open this one without doc_id for the next free id (open now: d1, d2)"
                .into()
        )
    );
    assert_eq!(s.session().documents().len(), 2);
}

#[test]
fn closing_frees_an_id_and_the_counter_never_gives_a_closed_or_given_one() {
    let s = empty_session(HostSettings::default());
    assert_eq!(
        s.open_with(package(), "c.pptx", &opening(Some("d3"), None)).unwrap(),
        id("d3")
    );
    assert_eq!(s.open_with(package(), "a.pptx", &Opening::default()).unwrap(), id("d1"));
    assert_eq!(s.open_with(package(), "b.pptx", &Opening::default()).unwrap(), id("d2"));
    assert_eq!(s.open_with(package(), "d.pptx", &Opening::default()).unwrap(), id("d4"));
    s.close(&id("d3")).unwrap();
    assert_eq!(s.open_with(package(), "e.pptx", &Opening::default()).unwrap(), id("d5"));
    s.close(&id("d5")).unwrap();
    assert_eq!(
        s.open_with(package(), "c.pptx", &opening(Some("d3"), None)).unwrap(),
        id("d3")
    );
}

#[test]
fn a_malformed_id_is_refused_with_the_accepted_form() {
    let s = empty_session(HostSettings::default());
    for bad in ["1", "d0", "d01", "D1", "d1 ", "doc1", "d-1", "", "b1", "d1/p:A"] {
        let refused = s
            .open_with(package(), "deck.pptx", &opening(Some(bad), None))
            .unwrap_err();
        assert!(
            refused.to_string().contains("'d' and a whole number from 1"),
            "{bad}: {refused}"
        );
    }
    assert!(s.session().documents().is_empty());
}

#[test]
fn two_opens_of_one_id_at_once_give_it_to_one_of_them() {
    let s = empty_session(HostSettings::default());
    let results: Vec<Result<DocId, OpenRefused>> = std::thread::scope(|scope| {
        let opens: Vec<_> = (0..4)
            .map(|n| {
                let s = &s;
                scope.spawn(move || s.open_with(package(), &format!("copy{n}.pptx"), &opening(Some("d5"), None)))
            })
            .collect();
        opens.into_iter().map(|t| t.join().unwrap()).collect()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results
        .iter()
        .filter_map(|r| r.as_ref().err())
        .all(|e| e.to_string().contains("is in use")));
    assert_eq!(s.session().documents(), vec![id("d5")]);
}

#[test]
fn the_room_for_documents_is_refused_in_todays_words() {
    let s = empty_session(HostSettings::default());
    for n in 0..4 {
        s.open_with(package(), &format!("{n}.pptx"), &Opening::default())
            .unwrap();
    }
    let refused = s.open_with(package(), "more.pptx", &Opening::default()).unwrap_err();
    assert_eq!(
        refused,
        OpenRefused::Limit(
            "4 documents are open; at most 4: close_document one you are done with (save it first if it changed)"
                .into()
        )
    );
}

#[test]
fn save_gives_bytes_and_state_of_one_version_and_open_takes_them_back() {
    let s = empty_session(HostSettings::default());
    s.open_with(package(), "deck.pptx", &Opening::default()).unwrap();
    call(&s, "toy_set", json!({"doc": "d1", "target": "a", "text": "x"}));
    call(&s, "toy_set", json!({"doc": "d1", "target": "b", "text": "y"}));
    let saved = s.save(&id("d1")).unwrap();
    assert_eq!(
        (saved.version, saved.kind, saved.name.as_str()),
        (2, DocKind::Pptx, "deck.pptx")
    );
    let state = saved.state.clone().unwrap();
    assert_eq!(state, "toy-state:3:2:2");
    let again = empty_session(HostSettings::default());
    let how = opening(Some("d1"), Some(&state));
    again.open_with(Arc::from(saved.data), "deck.pptx", &how).unwrap();
    // its history goes on from the saved numbers
    let set = call(&again, "toy_set", json!({"doc": "d1", "target": "c", "text": "z"}));
    assert_eq!(set["version"], 3, "{set}");
}

#[test]
fn a_format_keeping_no_state_refuses_one_and_a_state_of_other_bytes_is_refused() {
    let s = empty_session(HostSettings::default());
    let workbook = Opening {
        kind: Some(DocKind::Xlsx),
        state: Some("anything"),
        ..Opening::default()
    };
    assert_eq!(
        s.open_with(package(), "book.xlsx", &workbook).unwrap_err(),
        OpenRefused::State(
            "book.xlsx is a xlsx document, and that format keeps no state beside its bytes: open it without state"
                .into()
        )
    );
    let other = s
        .open_with(package(), "deck.pptx", &opening(None, Some("toy-state:9:1:1")))
        .unwrap_err();
    assert_eq!(
        other,
        OpenRefused::State("the state is another deck's: open these bytes without it".into())
    );
    assert!(s.session().documents().is_empty());
    // an empty state is none
    s.open_with(package(), "deck.pptx", &opening(None, Some(""))).unwrap();
}

#[test]
fn open_document_opens_a_blob_with_its_formats_summary() {
    let s = empty_session(HostSettings::default());
    let bytes = package_with(&[("toy/title.xml", "<t/>"), ("toy/problem-odd.xml", "<p/>")]);
    s.add_blob(bytes, "deck.pptx", None).unwrap();
    let reply = call(&s, "open_document", json!({"blob": "b1"}));
    assert_eq!(reply["summary"], "Opened deck.pptx as d1 (pptx)", "{reply}");
    assert_eq!(reply["created"], json!(["d1"]));
    assert_eq!(reply["doc"], "d1");
    assert_eq!(
        reply["data"],
        json!({"doc": "d1", "kind": "pptx", "name": "deck.pptx", "source": "b1", "version": 0,
               "baseline_problems": 1, "summary": {"parts": 3}})
    );
}

#[test]
fn open_document_on_a_text_or_image_blob_points_where_it_belongs() {
    let s = empty_session(HostSettings::default());
    s.add_blob(Arc::from(&b"a,b\n1,2\n"[..]), "data.csv", None).unwrap();
    let text = call(&s, "open_document", json!({"blob": "b1"}));
    assert_eq!(text["error"]["code"], "invalid_arguments");
    assert_eq!(text["error"]["field"], "blob");
    assert_eq!(
        text["error"]["message"],
        "data.csv is text (text/csv), not a document: read it with read_blob"
    );
    assert_eq!(text["error"]["valid_options"], json!(["read_blob"]));
    let missing = call(&s, "open_document", json!({"blob": "b9"}));
    assert_eq!(missing["error"]["code"], "not_found");
    assert_eq!(missing["error"]["valid_options"], json!(["b1 (data.csv)"]));
}

#[test]
fn new_document_runs_the_kinds_format_and_its_refusals() {
    let s = empty_session(HostSettings::default());
    let made = call(&s, "new_document", json!({"kind": "pptx", "title": "Plan"}));
    assert_eq!(made["summary"], "Made new.pptx as d1", "{made}");
    assert_eq!(made["doc"], "d1");
    assert_eq!(made["data"]["source"], "new");
    assert_eq!(made["data"]["summary"], json!({"parts": 2}));
    let read = call(&s, "toy_read", json!({"doc": "d1", "target": "title"}));
    assert_eq!(read["data"], "<t>Plan at 2026-01-05T09:00:00Z</t>", "{read}");
    let size = call(&s, "new_document", json!({"kind": "pptx", "size": "A4"}));
    assert_eq!(size["error"]["field"], "size");
    assert_eq!(size["error"]["valid_options"], json!(["16:9", "4:3"]));
    s.add_blob(package(), "template.potx", None).unwrap();
    let from = call(
        &s,
        "new_document",
        json!({"kind": "pptx", "template_blob": "b1", "name": "t.pptx"}),
    );
    assert_eq!(from["data"]["source"], "b1", "{from}");
    let none = call(&s, "new_document", json!({"kind": "docx"}));
    assert_eq!(
        none["error"]["message"],
        "new_document cannot make a docx document here: no library for it is loaded"
    );
    assert_eq!(none["error"]["valid_options"], json!(["pptx", "xlsx"]));
}

#[test]
fn close_document_closes_and_frees_its_room() {
    let s = empty_session(HostSettings::default());
    s.open_with(package(), "deck.pptx", &Opening::default()).unwrap();
    let closed = call(&s, "close_document", json!({"doc": "d1"}));
    assert_eq!(closed["summary"], "Closed d1");
    assert_eq!(closed["removed"], json!(["d1"]));
    assert_eq!(closed["data"], Value::Null);
    let again = call(&s, "close_document", json!({"doc": "d1"}));
    assert_eq!(again["error"]["code"], "not_found");
    assert!(s.stores().docs.is_empty());
}

#[test]
fn read_blob_reads_a_csv_a_page_at_a_time() {
    let s = empty_session(HostSettings {
        limits: ooxml_tools::Limits {
            result_chars: 2500,
            ..ooxml_tools::Limits::DEFAULT
        },
        ..HostSettings::default()
    });
    let rows: String = (0..200).map(|n| format!("row {n:03},{n}\r\n")).collect();
    s.add_blob(Arc::from(rows.as_bytes()), "rows.csv", None).unwrap();
    let first = call(&s, "read_blob", json!({"blob": "b1"}));
    let data = &first["data"];
    assert_eq!(
        (data["chars"].as_u64(), data["lines"].as_u64()),
        (Some(2290), Some(200)),
        "{first}"
    );
    assert_eq!(data["from"], 0);
    let text = data["text"].as_str().unwrap();
    assert!(text.len() <= 1000 && text.ends_with('\n'), "{text}");
    let cursor = first["next_cursor"].as_str().unwrap();
    assert_eq!(
        first["summary"],
        format!("Read {} of 2,290 characters of rows.csv", text.len())
    );
    let second = call(&s, "read_blob", json!({"blob": "b1", "cursor": cursor}));
    assert_eq!(second["data"]["from"], cursor[1..].parse::<u64>().unwrap());
    assert!(second["data"]["text"].as_str().unwrap().starts_with("row "));
}

#[test]
fn read_blob_refuses_images_packages_and_binary() {
    let s = empty_session(HostSettings::default());
    s.add_blob(package(), "deck.pptx", None).unwrap();
    s.add_blob(Arc::from(&b"\x00\x01\x02"[..]), "raw.bin", None).unwrap();
    let deck = call(&s, "read_blob", json!({"blob": "b1"}));
    assert_eq!(
        deck["error"]["message"],
        "b1 is a package (deck.pptx); open it with open_document."
    );
    let raw = call(&s, "read_blob", json!({"blob": "b2"}));
    assert_eq!(
        raw["error"]["message"],
        "b2 (application/octet-stream) is binary, not text"
    );
}

#[test]
fn a_save_hands_the_application_the_file_with_its_state_and_the_model_its_description() {
    let s = empty_session(HostSettings::default());
    s.open_with(package(), "deck.pptx", &Opening::default()).unwrap();
    let saved = call(
        &s,
        "save_document",
        json!({"doc": "d1", "name": "out.pptx", "format": "pptx"}),
    );
    assert_eq!(saved["ok"], true, "{saved}");
    let size = saved["data"]["size"].clone();
    assert_eq!(
        saved["data"],
        json!({"name": "out.pptx", "format": "pptx", "size": size, "doc": "d1",
               "validate": {"new": [], "fixed": [], "baseline": 0}, "coverage": {"complete": true}})
    );
    let outputs = s.take_outputs();
    assert_eq!(outputs.len(), 1);
    let state = outputs[0].state.as_ref().unwrap().downcast_ref::<String>().unwrap();
    assert_eq!(state, "toy-state:1:0:0");
    let outline = call(
        &s,
        "save_document",
        json!({"doc": "d1", "name": "out.txt", "format": "outline"}),
    );
    assert_eq!(outline["ok"], true);
    assert!(s.take_outputs()[0].state.is_none(), "a text copy keeps no state");
}

#[test]
fn the_save_gate_refuses_new_problems_unless_the_application_allows_them() {
    let s = empty_session(HostSettings::default());
    s.open_with(package(), "deck.pptx", &Opening::default()).unwrap();
    call(
        &s,
        "toy_set",
        json!({"doc": "d1", "target": "problem-odd", "text": "x"}),
    );
    let refused = call(
        &s,
        "save_document",
        json!({"doc": "d1", "name": "out.pptx", "format": "pptx"}),
    );
    let problem = "[odd] toy/problem-odd.xml: made up";
    assert_eq!(refused["error"]["code"], "refused", "{refused}");
    assert_eq!(
        refused["error"]["message"],
        "saving would add 1 validation problem(s); fix them first"
    );
    assert_eq!(refused["error"]["valid_options"], json!([problem]));
    // (what blocked it, in the one problems shape: user decision 2026-11-25)
    let shaped = json!({"code": "odd", "part": "toy/problem-odd.xml", "element": "made up", "message": problem,
        "severity": "problem"});
    assert_eq!(
        refused["error"]["details"]["validate"],
        json!({"new": [shaped], "fixed": [], "baseline": 0})
    );
    assert_eq!(refused["version"], 1);
    assert!(s.take_outputs().is_empty());
    let checked = call(&s, "check", json!({"doc": "d1"}));
    assert_eq!(
        checked["summary"],
        "Checked; saving would refuse 1 new validation problem(s): see validation"
    );
    assert_eq!(
        checked["data"],
        json!({"ok": true, "include": null, "validation": {"new": [shaped], "baseline": 0},
            "timing": {"total": 0.0, "validation": 0.0}})
    );
    let allowing = empty_session(HostSettings {
        allow_new_problems: true,
        ..HostSettings::default()
    });
    allowing.open_with(package(), "deck.pptx", &Opening::default()).unwrap();
    call(
        &allowing,
        "toy_set",
        json!({"doc": "d1", "target": "problem-odd", "text": "x"}),
    );
    let saved = call(
        &allowing,
        "save_document",
        json!({"doc": "d1", "name": "out.pptx", "format": "pptx"}),
    );
    assert_eq!(saved["data"]["validate"]["new"], json!([shaped]), "{saved}");
}

#[test]
fn describe_lists_the_problems_the_document_opened_with_after_its_controls() {
    let s = empty_session(HostSettings::default());
    let bytes = package_with(&[("toy/problem-odd.xml", "<p/>")]);
    s.open_with(bytes, "deck.pptx", &Opening::default()).unwrap();
    let described = call(&s, "describe", json!({"doc": "d1"}));
    let keys: Vec<&String> = described["data"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["parts", "controls", "problems", "after"]);
    assert_eq!(
        described["data"]["problems"],
        json!(["[odd] toy/problem-odd.xml: made up"])
    );
}

#[test]
fn the_applications_undo_takes_back_the_refs_of_what_it_undoes() {
    let s = empty_session(HostSettings::default());
    s.open_with(package(), "deck.pptx", &Opening::default()).unwrap();
    call(
        &s,
        "toy_set",
        json!({"doc": "d1", "target": "a", "text": "x", "ref": "intro"}),
    );
    assert_eq!(s.history(&id("d1"), 1, false, None).unwrap(), 1);
    let read = call(&s, "toy_read", json!({"doc": "d1", "target": "$intro"}));
    assert_eq!(read["error"]["code"], "not_found", "{read}");
    assert_eq!(s.history(&id("d1"), 1, true, None).unwrap(), 1);
    let read = call(&s, "toy_read", json!({"doc": "d1", "target": "$intro"}));
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(s.history(&id("d1"), 3, true, None).unwrap(), 0);
}
