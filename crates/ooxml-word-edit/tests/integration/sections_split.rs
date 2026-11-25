//! A section break splits a section as Word 16.106 writes one: the new section (the one
//! before the break) takes the split section's header and footer references, and the split
//! section names none from then on and inherits them -- tracked or not. Two sections naming
//! one header holding a chart stopped Word opening the file (the Word suite's
//! `chart-places.docx` case, `word-own.docx` the form Word wrote at that place). Rejecting
//! the tracked break gives the header back to the section left (Word 16.106's Reject All of
//! its own break and of this one).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{InsertSectionBreak, Review, RevisionPolicy, Selection, Tracking};

fn body_after(policy: RevisionPolicy) -> String {
    body_reviewed(policy, None)
}

/// The body after the break, and after reviewing everything (`accept`) if asked.
fn body_reviewed(policy: RevisionPolicy, accept: Option<bool>) -> String {
    let editor = editor(&fixture("generated/charts/chart-places.docx"));
    let command = InsertSectionBreak {
        after: Some("p@body/0".to_owned()).map(ooxml_word_model::address::Address::from),
        before: None,
        kind: ooxml_word_model::docx::sections::SectionBreakKind::NextPage,
        policy,
    };
    let done = editor.run(&Caller::anonymous(), &command).unwrap();
    assert!(done.out.changed);
    if let Some(accept) = accept {
        let review = Review {
            accept,
            selection: Selection::All,
        };
        editor.run(&Caller::anonymous(), &review).unwrap();
    }
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
    assert_eq!(
        body.matches(r#"r:id="rId10""#).count(),
        1,
        "one section names the header"
    );
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

fn tracked() -> RevisionPolicy {
    RevisionPolicy::Tracked(Tracking {
        author: "Writer A".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    })
}

#[test]
fn rejecting_a_tracked_break_gives_the_header_back() {
    let body = body_reviewed(tracked(), Some(false));
    let sects: Vec<&str> = body.split("<w:sectPr").skip(1).collect();
    assert_eq!(sects.len(), 1, "the break is gone: {body}");
    assert!(
        sects[0].contains(r#"<w:headerReference w:type="default" r:id="rId10"/>"#),
        "the section left names the header again: {body}"
    );
}

#[test]
fn accepting_a_tracked_break_keeps_the_new_sections_header() {
    check(&body_reviewed(tracked(), Some(true)));
}

/// `touched` for the body: one entry for the heading section holding the break (with its
/// blocks), and one plain `{"part"}` entry for the last section's properties, which lost
/// their references -- a change in no block, as `touched` has named one since round 16
/// (F12). Tracked and untracked alike; the reject that hands the references back the same.
fn document_entries(found: &[ooxml_word_edit::touched::Touched]) -> (Vec<[String; 2]>, usize) {
    let body: Vec<_> = found.iter().filter(|t| t.part == "word/document.xml").collect();
    let blocks = body
        .iter()
        .filter_map(|t| t.section.as_ref())
        .map(|s| s.blocks.clone())
        .collect();
    (blocks, body.iter().filter(|t| t.section.is_none()).count())
}

#[test]
fn a_break_after_a_paragraph_touches_its_section_and_the_last_sections_properties() {
    for policy in [RevisionPolicy::Untracked, tracked()] {
        let editor = editor(&fixture("generated/charts/chart-places.docx"));
        let command = InsertSectionBreak {
            after: Some("p@body/0".to_owned()).map(ooxml_word_model::address::Address::from),
            before: None,
            kind: ooxml_word_model::docx::sections::SectionBreakKind::NextPage,
            policy: policy.clone(),
        };
        let done = editor.run(&Caller::anonymous(), &command).unwrap();
        let (blocks, plain) = document_entries(&done.touched);
        assert_eq!(blocks.len(), 1, "{policy:?}: {:?}", done.touched);
        assert_eq!(blocks[0][0], "p:5FCB534E", "{policy:?}: {:?}", done.touched);
        assert_eq!(
            plain, 1,
            "{policy:?}: the last section's properties: {:?}",
            done.touched
        );
        if matches!(policy, RevisionPolicy::Tracked(_)) {
            let review = Review {
                accept: false,
                selection: Selection::All,
            };
            let rejected = editor.run(&Caller::anonymous(), &review).unwrap();
            let (blocks, plain) = document_entries(&rejected.touched);
            assert_eq!(blocks.len(), 1, "reject: {:?}", rejected.touched);
            assert_eq!(plain, 1, "reject: the header handed back: {:?}", rejected.touched);
        }
    }
}
