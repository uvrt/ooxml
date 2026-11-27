//! A part's relationships read once, each id found at once, equal the relationships read as
//! a dictionary by id keeps them: each id once, at the place it first appears, with its last
//! relationship.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{package_bytes, Parts};
use ooxml_base::Limits;
use ooxml_opc::{Package, Relationship};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::parts::{relationships, PartRelationships};
use std::sync::Arc;

const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// What a dictionary by id keeps, found the slow way: each relationship's id looked for
/// among those kept so far.
fn by_scanning(package: &Package, source: &str) -> Vec<Relationship> {
    let mut out: Vec<Relationship> = Vec::new();
    for rel in package.relationships(source).unwrap().iter() {
        match out.iter_mut().find(|r| r.id == rel.id) {
            Some(slot) => *slot = rel.clone(),
            None => out.push(rel.clone()),
        }
    }
    out
}

fn with_relationships(rels: &[(&str, &str, &str)]) -> Package {
    let bytes = package_bytes(&Parts {
        body: "<w:p/>".into(),
        ..Parts::default()
    });
    let package = Package::open(Arc::from(bytes), &Limits::default()).unwrap();
    let items: String = rels
        .iter()
        .map(|(id, kind, target)| format!(r#"<Relationship Id="{id}" Type="urn:{kind}" Target="{target}"/>"#))
        .collect();
    let xml = format!(r#"<Relationships xmlns="{RELS}">{items}</Relationships>"#);
    package.with_bytes("word/_rels/document.xml.rels", Arc::from(xml.as_bytes()))
}

#[test]
fn each_id_once_at_its_first_place_with_its_last_relationship() {
    let package = with_relationships(&[
        ("rId1", "a", "media/one.png"),
        ("rId2", "b", "media/two.png"),
        ("rId1", "c", "media/three.png"),
        ("rId3", "d", "media/four.png"),
        ("rId2", "e", "media/five.png"),
        ("rId2", "f", "media/six.png"),
    ]);
    let source = "word/document.xml";
    let expected = by_scanning(&package, source);
    let read = PartRelationships::read(&package, source).unwrap();
    assert_eq!(read.items(), expected.as_slice());
    assert_eq!(relationships(&package, source).unwrap(), expected);
    let ids: Vec<&str> = expected.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["rId1", "rId2", "rId3"]);
    for rel in &expected {
        assert_eq!(read.get(&rel.id), Some(rel));
    }
    assert_eq!(read.get("rId9"), None);
    assert_eq!(read.get("rId2").unwrap().rel_type, "urn:f");

    // A version's reading reads them once, the same.
    let docx = Docx::open(
        &ooxml_word_model::WordPackage::open(package).0,
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap();
    let first = docx.relationships(source).unwrap();
    let again = docx.relationships(source).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(*first, read);
    assert!(docx.relationships("word/none.xml").unwrap().items().is_empty());
}
