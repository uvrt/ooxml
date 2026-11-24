//! A section break splits a section as Word 16.106 writes one: the new section (the one
//! before the break) takes the split section's header and footer references, and the split
//! section names none from then on and inherits them -- tracked or not. Two sections naming
//! one header holding a chart stopped Word opening the file (the Word suite's
//! `chart-places.docx` case, `word-own.docx` the form Word wrote at that place).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{InsertSectionBreak, RevisionPolicy, Tracking};

fn body_after(policy: RevisionPolicy) -> String {
    let editor = editor(&fixture("generated/charts/chart-places.docx"));
    let command = InsertSectionBreak {
        after: Some("p@body/0".to_owned()),
        before: None,
        kind: "nextPage".to_owned(),
        policy,
    };
    let done = editor.run(&Caller::anonymous(), &command).unwrap();
    assert!(done.out.changed);
    let head = editor.store().snapshot();
    String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap()
}

fn check(body: &str) {
    let sects: Vec<&str> = body.split("<w:sectPr").skip(1).collect();
    assert_eq!(sects.len(), 2, "{body}");
    let first = &sects[0][..sects[0].find("</w:sectPr>").unwrap()];
    let last = &sects[1][..sects[1].find("</w:sectPr>").unwrap()];
    assert!(
        first.contains(r#"<w:headerReference w:type="default" r:id="rId10"/>"#),
        "the new section keeps the header: {first}"
    );
    assert!(!last.contains("Reference"), "the split section inherits: {last}");
    assert_eq!(body.matches(r#"r:id="rId10""#).count(), 1, "one section names the header");
}

#[test]
fn a_tracked_break_gives_the_split_sections_header_to_the_new_one() {
    let body = body_after(RevisionPolicy::Tracked(Tracking {
        author: "Writer A".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    }));
    check(&body);
    assert!(body.contains("<w:ins "), "the break's mark is inserted: {body}");
}

#[test]
fn an_untracked_break_gives_the_split_sections_header_to_the_new_one() {
    check(&body_after(RevisionPolicy::Untracked));
}
