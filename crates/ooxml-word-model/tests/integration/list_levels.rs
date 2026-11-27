//! A paragraph at a level its list's definition lacks reads as Word fills that level: Word
//! 16.106 gives a multi-level definition all nine levels when it opens the file, a missing one
//! `w:numFmt="decimal"` with an empty `w:lvlText` (no label), and a single-level definition
//! none (measured: `style-document.docx`'s one-level bullet list, a paragraph set to level 1;
//! the Word suite's `style:1b2189e4005924c7`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{package, Parts};
use ooxml_word_model::docx::numbering::{membership, Lists};
use ooxml_word_model::docx::Docx;

/// Three paragraphs of list 1, at levels 0, 1 and 9.
fn parts(kind: &str) -> Parts {
    let item = |level: u32| {
        format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>item</w:t></w:r></w:p>"#
        )
    };
    Parts {
        body: [item(0), item(1), item(9)].concat(),
        numbering: Some(format!(
            r#"<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="{kind}"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="o"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#
        )),
        ..Parts::default()
    }
}

/// Each paragraph's level, format and text as the model reads them.
fn read(kind: &str) -> Vec<(i64, Option<String>, Option<String>)> {
    let docx = Docx::open(
        &package(&parts(kind)),
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap();
    let lists = Lists::read(&docx);
    let body = docx.part(&docx.main).unwrap();
    body.ids
        .paragraphs()
        .iter()
        .map(|p| {
            let found = membership(&docx, &lists, &p.element).unwrap();
            (found.level, found.format, found.text)
        })
        .collect()
}

#[test]
fn a_missing_level_of_a_multi_level_list_is_decimal_with_no_text() {
    for kind in ["hybridMultilevel", "multilevel"] {
        let found = read(kind);
        assert_eq!(found[0], (0, Some("bullet".to_owned()), Some("o".to_owned())), "{kind}");
        assert_eq!(found[1], (1, Some("decimal".to_owned()), Some(String::new())), "{kind}");
        // past Word's nine levels there is nothing to fill
        assert_eq!(found[2], (9, None, None), "{kind}");
    }
}

#[test]
fn a_single_level_list_has_no_level_filled() {
    let found = read("singleLevel");
    assert_eq!(found[1], (1, None, None));
}
