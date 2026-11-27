//! A stand-in refuses every operation of a format with an internal error naming it, and
//! answers a call's reads only with what the application resolved for it.

use super::*;
use crate::StoredDoc;
use ooxml_tools::{ErrorCode, Failure};

fn stand_in() -> Arc<HeldElsewhere> {
    Arc::new(HeldElsewhere::new(DocKind::Xlsx, 1200, 3).unwrap())
}

fn names_the_stand_in(message: &str) {
    assert!(
        message.contains("xlsx document the application's own host holds (a stand-in here)"),
        "{message}"
    );
}

#[test]
fn its_head_is_the_state_the_application_said_and_moves_as_told() {
    let held = stand_in();
    assert_eq!(held.head().number(), 3);
    let seq = held.head().seq();
    held.moved(5);
    assert_eq!((held.head().number(), held.highest()), (5, 5));
    assert!(held.head().seq() > seq, "another version");
    held.moved(5);
    assert_eq!(held.numbered(5).map(|v| v.number()), Some(5));
    assert!(held.numbered(4).is_none());
    assert_eq!(held.size(), 1200);
    assert!(held
        .head()
        .package()
        .part_names()
        .iter()
        .all(|p| p == "[Content_Types].xml"));
}

#[test]
fn every_operation_of_a_format_on_it_is_refused_naming_it() {
    let held = stand_in();
    let at = held.head();
    let error = |e: SessionError| names_the_stand_in(&e.to_string());
    error(held.save(&at, false).unwrap_err());
    error(held.history(1, None, Direction::Undo).unwrap_err());
    error(held.continue_numbers(1, 1).unwrap_err());
    error(held.open_group(TxnOwner::new(1)).unwrap_err());
    error(held.group_mark(TxnOwner::new(1)).unwrap_err());
    error(held.commit_group(TxnOwner::new(1), &Caller::named("x")).unwrap_err());
    let read = held.source(&at, "t:Sales").unwrap().unwrap_err();
    assert_eq!(read.code(), ErrorCode::Failure(Failure::Internal));
    names_the_stand_in(read.message());
    assert!(read.message().contains("t:Sales"));
    assert!(!held.drop_group(TxnOwner::new(1)));
    let format = ElsewhereFormat {
        kind: DocKind::Xlsx,
        prefixes: vec!["t:".into()],
    };
    assert!(!format.detect(b"PK", "a.xlsx"));
    assert_eq!(format.source_prefixes(), vec!["t:".to_owned()]);
}

#[test]
fn as_a_call_reads_it_it_answers_what_the_application_resolved_and_nothing_else() {
    let held = stand_in();
    let rows = Dataset::from_json(&serde_json::json!({"columns": ["Region"], "rows": [["North"]]})).unwrap();
    let values = HeldValues {
        doc: "d2".into(),
        answers: vec![("t:Sales".into(), Ok(rows.clone()))],
        value_sources: Some(vec!["t:Sales".into()]),
        ..HeldValues::default()
    };
    let missed = values.missed.clone();
    let read = held.answering(values);
    let at = read.stored().head();
    assert_eq!(read.source(&at, "t:Sales").unwrap().unwrap(), rows);
    let other = read.source(&at, "t:Notes").unwrap().unwrap_err();
    names_the_stand_in(other.message());
    assert_eq!(
        missed.addresses(),
        vec!["t:Notes".to_owned()],
        "noted for the application to resolve"
    );
    assert_eq!(read.value_sources(&at), Some(vec!["t:Sales".to_owned()]));
    assert!(read.save(&at, false).is_err());
    assert_eq!(read.stored().head().number(), 3);
}
