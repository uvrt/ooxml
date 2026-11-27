//! A bookmark looked up by name alone (`bookmark_named`) is the one every bookmark listed
//! finds (`Bookmarks::get`): written as asked first, else in any case, the first start of a
//! name, the last end of its id in its part.
#![allow(clippy::unwrap_used)]

use crate::support::{package, Parts};
use ooxml_word_model::address::marks::{bookmark_named, Bookmark, Bookmarks};
use ooxml_word_model::docx::Docx;

fn start(name: &str, id: u32) -> String {
    format!(r#"<w:bookmarkStart w:id="{id}" w:name="{name}"/>"#)
}

fn end(id: u32) -> String {
    format!(r#"<w:bookmarkEnd w:id="{id}"/>"#)
}

fn paragraph(inner: &str) -> String {
    format!("<w:p>{inner}<w:r><w:t>text</w:t></w:r></w:p>")
}

/// A bookmark's name, part, start and end.
type Place = (String, String, Box<[u32]>, Option<Box<[u32]>>);

fn place(found: &Bookmark<'_>) -> Place {
    (
        found.name.clone(),
        found.part.part.clone(),
        found.start.clone(),
        found.end.clone(),
    )
}

#[test]
fn one_name_found_as_all_find_it() {
    let body = [
        paragraph(&start("Intro", 1)),
        paragraph(&format!("{}{}", end(1), start("Twice", 2))),
        paragraph(&start("twice", 3)),
        paragraph(&format!("{}{}", start("Twice", 4), end(2))),
        paragraph(&format!("{}{}", end(3), end(2))),
        format!(
            "<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w=\"2000\"/></w:tblGrid><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>",
            paragraph(&format!("{}{}", start("InCell", 5), end(5)))
        ),
        paragraph(&start("NoEnd", 6)),
    ]
    .concat();
    let docx = Docx::open(
        &package(&Parts {
            body,
            ..Parts::default()
        }),
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap();
    let all = Bookmarks::of(&docx);
    for name in [
        "Intro",
        "Twice",
        "twice",
        "TWICE",
        "intro",
        "bm:InCell",
        "NoEnd",
        "Missing",
        "",
    ] {
        let alone = bookmark_named(&docx, name).map(|b| place(&b)).ok();
        let listed = all.get(name).map(place).ok();
        assert_eq!(alone, listed, "{name}");
    }
    assert!(bookmark_named(&docx, "TWICE").is_ok_and(|b| b.name == "Twice"));
}
