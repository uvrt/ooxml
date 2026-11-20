use super::facts::{facts_after_edit, facts_found, labels_after_edit};
use super::{kind_of, Kind, TextEntry};
use serde_json::{json, Value};

fn fact(p: u64, at: u64, text: &str) -> Value {
    json!({"source": "d:x/a", "text": text, "p": p, "at": at})
}

#[test]
fn keys_are_read_as_docx_agent_reads_them() {
    assert_eq!(kind_of("t:04C2BB95"), Some(Kind::Table));
    assert_eq!(kind_of("header1/t:04C2BB95"), Some(Kind::Table));
    assert_eq!(kind_of("t:04C2BB95#1"), None);
    assert_eq!(kind_of("t@body/2"), None);
    assert_eq!(kind_of("d:1"), Some(Kind::Chart));
    assert_eq!(kind_of("d:3#1"), None);
    assert_eq!(kind_of("p:3B212964"), Some(Kind::Text));
    assert_eq!(kind_of("footnotes/p:3B212964"), Some(Kind::Text));
    assert_eq!(kind_of("p:80000000"), None);
}

#[test]
fn facts_follow_an_edit_away_from_them() {
    let facts = [fact(0, 4, "12"), fact(0, 13, "3.5")];
    // an edit before both moves them; the text between kept
    let after = facts_after_edit(&facts, "Got 12 and a 3.5.", "We got 12 and a 3.5.");
    assert_eq!(after, vec![fact(0, 7, "12"), fact(0, 16, "3.5")]);
    // an edit touching the first drops it
    let after = facts_after_edit(&facts, "Got 12 and a 3.5.", "Got 123 and a 3.5.");
    assert_eq!(after, vec![fact(0, 14, "3.5")]);
    // a fact not where it says is dropped
    assert!(facts_found(&[fact(0, 0, "12")], "Got 12").is_empty());
}

#[test]
fn places_count_characters() {
    let facts = [fact(0, 2, "€5")];
    let after = facts_after_edit(&facts, "ä €5", "äää €5");
    assert_eq!(after, vec![fact(0, 4, "€5")]);
}

#[test]
fn labels_stretch_over_an_edit_of_their_own_text() {
    let label = json!({"text": "Low", "value": 1.5, "p": 0, "at": 6});
    let after = labels_after_edit(std::slice::from_ref(&label), "Risk: Low.", "Risk: Lower.");
    assert_eq!(after, vec![json!({"text": "Lower", "value": 1.5, "p": 0, "at": 6})]);
    let whole = json!({"text": "Low", "p": 0, "at": 0});
    let after = labels_after_edit(std::slice::from_ref(&whole), "Low", "High");
    assert_eq!(after, vec![json!({"text": "High", "p": 0, "at": 0})]);
    let after = labels_after_edit(&[label], "Risk: Low.", "Risk: L");
    assert!(after.is_empty());
}

#[test]
fn a_text_entry_is_read_and_written_back() {
    let text = r#"{"v":1,"paragraph":"Got 12","facts":[{"text":"12","p":0,"at":4}]}"#;
    let entry = TextEntry::read(text).expect("a text record");
    assert_eq!(super::dumps(&entry.record()), text);
    assert!(TextEntry::read(r#"{"v":0,"paragraph":"x","facts":[]}"#).is_none());
    assert!(TextEntry::read(r#"{"v":1,"paragraph":"x","facts":[],"labels":3}"#).is_none());
}
