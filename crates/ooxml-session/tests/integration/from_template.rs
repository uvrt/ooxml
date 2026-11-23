//! A new document from a template's bytes (docx-agent's `new_document` with a
//! `template_blob`): the content left out, one empty paragraph before the last section, the
//! properties started again, a `.docx`; the template's bytes as they were.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::editing::{docx, session};
use ooxml_opc::Package;
use ooxml_word_edit::{FileKind, NewFromTemplate};

const BODY: &str = r#"<w:p w14:paraId="0A000001"><w:r><w:t>Template text.</w:t></w:r></w:p><w:p w14:paraId="0A000002"/><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>"#;

fn part(package: &Package, name: &str) -> String {
    String::from_utf8(package.read(name).unwrap().unwrap().to_vec()).unwrap()
}

#[test]
fn a_template_s_content_goes_and_its_properties_start_again() {
    let s = session();
    let template = docx(BODY);
    let kept = template.clone();
    let made = NewFromTemplate {
        page: Some((11906, 16838)),
        title: Some("Plan".to_owned()),
        author: Some("Ann".to_owned()),
        created: "2026-11-23T09:00:00Z".to_owned(),
    };
    let bytes = s.new_from_template(template, &made).unwrap();
    assert_eq!(*kept, *docx(BODY), "the template's bytes are as they were");
    let package = Package::open(bytes.into(), &ooxml_base::Limits::default()).unwrap();
    assert_eq!(FileKind::of(&package).unwrap(), Some(FileKind::Docx));
    let main = part(&package, "word/document.xml");
    assert!(!main.contains("Template text."), "{main}");
    assert_eq!(main.matches("<w:p ").count(), 1, "{main}");
    assert!(main.contains(r#"<w:pgSz w:w="11906" w:h="16838"/>"#), "{main}");
    let core = part(&package, "docProps/core.xml");
    for wanted in ["Plan", "Ann", ">1<", "2026-11-23T09:00:00Z", "dcterms:W3CDTF"] {
        assert!(core.contains(wanted), "{wanted}: {core}");
    }
}

#[test]
fn bytes_that_are_no_word_document_are_refused() {
    let s = session();
    let made = NewFromTemplate {
        page: None,
        title: None,
        author: None,
        created: "2026-11-23T09:00:00Z".to_owned(),
    };
    assert!(s
        .new_from_template(std::sync::Arc::from(&b"not a zip"[..]), &made)
        .is_err());
}
