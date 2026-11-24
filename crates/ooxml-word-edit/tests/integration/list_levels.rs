//! A tracked level change writes what Word 16.106's own does: the paragraph's `w:numPr` with
//! the new level and a `w:pPrChange` holding its old properties, nothing in the numbering
//! part (measured on `style-document.docx`, p@body/11 set to level 1 by hand).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{Editor, ListAction, ListEdit, RevisionPolicy, Tracking};

fn tracked() -> RevisionPolicy {
    RevisionPolicy::Tracked(Tracking {
        author: "Writer A".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    })
}

fn level_one() -> ListEdit {
    ListEdit {
        targets: vec!["p@body/11".to_owned()],
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
