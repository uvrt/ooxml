//! Steps 6 to 8 of a call (ooxml-edit's `test_tools_sources`, `test_tools_datasets` and
//! `test_tools_textfacts`, their generic parts, on the toy format): a declared source's
//! dataset checked, its rows handed to the format; one value of it handed as a value
//! written; a reference in a declared text replaced by its value; an unknown dataset refused
//! by the host with the ones there are.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, session};
use ooxml_session::datasets::DatasetEntry;
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::Cell;
use serde_json::json;
use std::sync::Arc;

fn with_sales() -> ooxml_host::HostSession {
    let s = session();
    let row = |region: &str, units: i64| vec![Cell::Text(region.into()), Cell::Int(units.to_string())];
    let sales = Dataset::new(
        vec!["region".into(), "units".into()],
        vec![row("North", 3), row("South", 5)],
    )
    .unwrap()
    .with_keys(1)
    .unwrap();
    s.keep_datasets(vec![DatasetEntry::new("sales", Arc::new(sales), None)])
        .unwrap();
    s
}

#[test]
fn a_declared_sources_rows_are_handed_to_the_format() {
    let s = with_sales();
    let reply = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales"}));
    assert_eq!(
        reply["data"]["rows"],
        json!([{"given": "d:sales", "address": "d:sales", "rows": 2}]),
        "{reply}"
    );
    assert_eq!(reply["data"]["written"], json!([]));
    let run = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales/#2:#2"}));
    assert_eq!(run["data"]["rows"][0]["rows"], 1, "{run}");
}

#[test]
fn a_source_naming_one_value_is_handed_as_a_value_written_and_where_it_is() {
    let s = with_sales();
    let reply = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales/South/units"}));
    assert_eq!(
        reply["data"]["written"],
        json!([{"field": "from", "text": "5", "start": 0, "end": 1}]),
        "{reply}"
    );
    assert_eq!(
        reply["data"]["one"],
        json!([{"field": "from", "shown": "d:sales/South/units", "row": 1, "column": 1}])
    );
    let missing = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sales/East/units"}));
    assert_eq!(missing["data"]["refused"][0]["field"], "from", "{missing}");
    assert_eq!(missing["data"]["written"], json!([]));
}

#[test]
fn a_reference_in_a_declared_text_is_written_with_its_value() {
    let s = with_sales();
    let reply = call(
        &s,
        "toy_fill",
        json!({"doc": "d1", "text": "North sold {{d:sales/North/units}} units"}),
    );
    assert_eq!(reply["data"]["text"], "North sold 3 units", "{reply}");
    assert_eq!(
        reply["data"]["written"],
        json!([{"field": "text", "text": "3", "start": 11, "end": 12}])
    );
}

#[test]
fn an_unknown_dataset_is_refused_by_the_host_with_the_ones_there_are() {
    let s = with_sales();
    let reply = call(&s, "toy_fill", json!({"doc": "d1", "from": "d:sale"}));
    assert_eq!(reply["error"]["code"], "not_found", "{reply}");
    assert_eq!(reply["error"]["field"], "from");
    let text = call(&s, "toy_fill", json!({"doc": "d1", "text": "{{d:nope/North/units}}"}));
    assert_eq!(text["ok"], false, "{text}");
}

#[test]
fn describe_lists_the_sessions_datasets_never_their_values() {
    let s = with_sales();
    let described = call(&s, "describe", json!({"doc": "d1"}));
    let listed = described["data"]["datasets"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{described}");
    assert!(!listed[0].to_string().contains("North"), "{described}");
    let none = call(&session(), "describe", json!({"doc": "d1"}));
    assert!(none["data"].get("datasets").is_none(), "{none}");
}

#[test]
fn a_dataset_the_application_kept_reads_back_with_its_serial_and_goes_stale_with_its_document() {
    use ooxml_session::datasets::DocMeta;
    let s = session();
    let made =
        |units: i64| Arc::new(Dataset::new(vec!["units".into()], vec![vec![Cell::Int(units.to_string())]]).unwrap());
    let meta = DocMeta {
        doc: "d1".into(),
        kind: ooxml_session::DocKind::Pptx,
        name: "deck.pptx".into(),
    };
    let serials = s
        .keep_datasets(vec![DatasetEntry::new("units", made(3), Some(&meta))])
        .unwrap();
    let kept = s.kept_dataset("units").unwrap();
    assert_eq!(serials, vec![kept.serial]);
    assert!(!kept.stale && !kept.closed);
    assert_eq!(kept.entry.version(), Some(0), "of the document's head when kept");
    let reply = call(&s, "toy_set", json!({"doc": "d1", "target": "a", "text": "changed"}));
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert!(s.kept_dataset("units").unwrap().stale, "its document changed since");
    s.keep_datasets(vec![DatasetEntry::new("units", made(4), None)])
        .unwrap();
    let again = s.kept_dataset("units").unwrap();
    assert_ne!(again.serial, kept.serial, "kept again: another serial");
    assert!(!again.stale);
    s.drop_dataset("d:units").unwrap();
    assert!(s.kept_dataset("units").is_none());
    assert_eq!(s.drop_dataset("d:units").unwrap_err().code().as_str(), "not_found");
}

#[test]
fn the_applications_images_and_outputs_are_the_sessions() {
    let s = session();
    assert_eq!(s.images_remaining(), Some(40));
    s.images(39, true).unwrap();
    assert_eq!(s.images(2, false).unwrap_err().code().as_str(), "limit");
    s.release_images(9);
    assert_eq!(s.images_remaining(), Some(10));
    s.add_output(ooxml_session::outputs::Output {
        name: "notes.txt".into(),
        format: "txt".into(),
        data: Arc::from(&b"hello"[..]),
        doc: None,
        validate: None,
        state: None,
    });
    let taken = s.take_outputs();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].name, "notes.txt");
    assert!(s.take_outputs().is_empty());
}

mod held_elsewhere {
    //! The values channel (design phase 3): a document the application's own host holds is a
    //! stand-in in the session -- one id counter, one room -- whose rows a call reads only as
    //! the application resolved them for it.

    use super::*;
    use crate::toy::session_holding_elsewhere;
    use ooxml_host::{CallAs, Opening};
    use ooxml_tools::written::{CallValues, HeldValues};
    use serde_json::Value;

    fn sales() -> Dataset {
        let row = |region: &str, units: i64| vec![Cell::Text(region.into()), Cell::Int(units.to_string())];
        Dataset::new(
            vec!["Region".into(), "Units".into()],
            vec![row("North", 3), row("South", 5)],
        )
        .unwrap()
    }

    fn holding() -> ooxml_host::HostSession {
        let s = session_holding_elsewhere();
        let how = Opening {
            source: Some("app"),
            ..Opening::default()
        };
        let id = s
            .hold(ooxml_session::DocKind::Xlsx, "sales.xlsx", 2_000, 4, &how)
            .unwrap();
        assert_eq!(id.to_string(), "d2", "the next id of the session's one counter");
        s
    }

    fn with(held: Vec<HeldValues>) -> CallAs {
        CallAs {
            values: CallValues {
                held,
                ..CallValues::default()
            },
            ..CallAs::default()
        }
    }

    #[test]
    fn a_held_documents_rows_are_read_as_the_application_resolved_them() {
        let s = holding();
        let values = HeldValues {
            doc: "d2".into(),
            answers: vec![("t:Sales".into(), Ok(sales()))],
            value_sources: Some(vec!["t:Sales".into()]),
            ..HeldValues::default()
        };
        let reply = Value::Object(
            s.call("toy_fill", json!({"doc": "d1", "from": "t:Sales"}), &with(vec![values]))
                .unwrap()
                .to_json(),
        );
        assert_eq!(
            reply["data"]["rows"],
            json!([{"given": "d2/t:Sales", "address": "d2/t:Sales", "rows": 2}]),
            "{reply}"
        );
    }

    #[test]
    fn rows_not_resolved_for_the_call_are_an_internal_error_naming_the_stand_in() {
        let s = holding();
        let reply = Value::Object(
            s.call("toy_fill", json!({"doc": "d1", "from": "t:Sales"}), &CallAs::default())
                .unwrap()
                .to_json(),
        );
        assert_eq!(reply["error"]["code"], "internal", "{reply}");
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("the application's own host holds (a stand-in here)"),
            "{message}"
        );
    }

    #[test]
    fn a_held_documents_version_is_what_the_application_says_and_closing_frees_its_room() {
        let s = holding();
        let id = ooxml_session::DocId::parse("d2").unwrap();
        assert_eq!(s.session().snapshot(&id).unwrap().number(), 4);
        s.held_moved(&id, 7).unwrap();
        assert_eq!(s.session().snapshot(&id).unwrap().number(), 7);
        let d1 = ooxml_session::DocId::parse("d1").unwrap();
        assert!(s.held_moved(&d1, 1).is_err(), "the session holds d1 itself");
        assert!(s
            .session()
            .format_doc(&id)
            .unwrap()
            .save(&s.session().snapshot(&id).unwrap().any().clone(), false)
            .is_err());
        s.close(&id).unwrap();
        assert!(s.held_moved(&id, 8).is_err());
        let again = s
            .hold(ooxml_session::DocKind::Xlsx, "other.xlsx", 10, 0, &Opening::default())
            .unwrap();
        assert_eq!(again.to_string(), "d3", "ids are never given twice");
        assert!(s
            .hold(ooxml_session::DocKind::Docx, "x.docx", 10, 0, &Opening::default())
            .is_err());
    }
}
