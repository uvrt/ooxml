//! A tracked level change writes what Word 16.106's own does: the paragraph's `w:numPr` with
//! the new level and a `w:pPrChange` holding its old properties, nothing in the numbering
//! part (measured on `style-document.docx`, p@body/11 set to level 1 by hand). A paragraph of
//! a single-level list Word sets to no level but 0 (its `list level number` refused,
//! measured): refused here, the way out level 0.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_word_edit::{EditError, Editor, ListAction, ListEdit, RevisionPolicy, Tracking};
use std::sync::Arc;

fn tracked() -> RevisionPolicy {
    RevisionPolicy::Tracked(Tracking {
        author: "Writer A".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    })
}

fn level_one() -> ListEdit {
    ListEdit {
        targets: vec![ooxml_word_model::address::Address::parse("p@body/11")],
        action: ListAction::Level(1),
        policy: tracked(),
    }
}

fn part(editor: &Editor, name: &str) -> String {
    let head = editor.store().snapshot();
    String::from_utf8(head.doc().tree(name).unwrap().unwrap().write()).unwrap()
}

#[test]
fn a_tracked_level_change_writes_the_level_and_the_old_properties_only() {
    let bytes = fixture("docx2svg/style-document.docx");
    let editor = editor(&bytes);
    let numbering = part(&editor, "word/numbering.xml");
    editor.run(&Caller::anonymous(), &level_one()).unwrap();
    let body = part(&editor, "word/document.xml");
    let at = body.find("and list paragraphs").unwrap();
    let start = body[..at].rfind("<w:p ").unwrap();
    let properties = &body[body[start..].find("<w:pPr>").unwrap() + start
        ..body[start..].find("</w:pPrChange>").unwrap() + start + "</w:pPrChange>".len()];
    assert!(
        properties.contains(r#"<w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr>"#),
        "{properties}"
    );
    assert!(
        properties.contains(r#"<w:pPr><w:pStyle w:val="ListBullet"/></w:pPr></w:pPrChange>"#),
        "{properties}"
    );
    // as Word's own change, nothing in the numbering part (Word fills the missing level when
    // it opens the file: a decimal level with no text)
    assert_eq!(part(&editor, "word/numbering.xml"), numbering);
}

/// `style-document.docx` with its list definition made single-level (the only change).
fn single_level_editor() -> Editor {
    let bytes = fixture("docx2svg/style-document.docx");
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).unwrap();
    let numbering = String::from_utf8(package.read("word/numbering.xml").unwrap().unwrap().to_vec()).unwrap();
    let single = numbering.replace(r#"w:val="hybridMultilevel""#, r#"w:val="singleLevel""#);
    assert_ne!(single, numbering);
    let package = package.with_bytes("word/numbering.xml", Arc::from(single.into_bytes()));
    Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(crate::common::Monotonic(std::time::Instant::now())),
        Arc::new(crate::common::Unlimited),
    )
    .unwrap()
}

#[test]
fn a_level_past_0_in_a_single_level_list_is_refused_with_level_0() {
    let editor = single_level_editor();
    let error = editor.run(&Caller::anonymous(), &level_one()).unwrap_err();
    let EditError::Argument {
        field,
        message,
        options,
    } = error
    else {
        panic!("not an argument refusal: {error:?}")
    };
    assert_eq!(field, "level");
    assert!(message.contains("single-level"), "{message}");
    assert_eq!(options, ["0"]);
    // level 0 is the way out (a no-op here: the paragraph is at 0 already)
    let zero = ListEdit {
        action: ListAction::Level(0),
        ..level_one()
    };
    editor.run(&Caller::anonymous(), &zero).unwrap();
}

#[test]
fn a_target_with_a_paragraph_in_no_list_is_refused_as_not_in_a_list_first() {
    // level 0 would not get past the paragraph in no list: that refusal comes first, wherever
    // it stands in the target
    let editor = single_level_editor();
    let mixed = ListEdit {
        targets: vec![
            ooxml_word_model::address::Address::parse("p@body/11"),
            ooxml_word_model::address::Address::parse("p@body/0"),
        ],
        ..level_one()
    };
    let error = editor.run(&Caller::anonymous(), &mixed).unwrap_err();
    let EditError::Refused(message) = error else {
        panic!("not the not-in-a-list refusal: {error:?}")
    };
    assert!(message.ends_with("is not in a list"), "{message}");
}
