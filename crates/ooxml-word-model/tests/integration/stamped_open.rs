//! A package is stamped in memory when it is opened (stamp-on-open design, 2026-11-26): every
//! story block has a durable id from the first read, the ids are a function of the bytes,
//! a save before any part is written gives the bytes back, and a part written and saved
//! opens again with the same ids.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use super::support::{package_bytes, raw_package, Parts};
use ooxml_base::Limits;
use ooxml_opc::{Package, PartVersion};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{IdState, PartIds};
use ooxml_word_model::WordPackage;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixtures(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            fixtures(&path, out);
        } else if path.extension().is_some_and(|e| e == "docx") {
            out.push(path);
        }
    }
}

/// Every fixture and corpus document of the oracle.
fn documents() -> Vec<PathBuf> {
    let oracle = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ooxml-oracle");
    let mut files = Vec::new();
    fixtures(&oracle.join("fixtures"), &mut files);
    fixtures(&oracle.join("corpus"), &mut files);
    files.sort();
    files
}

fn opened(bytes: &[u8]) -> Option<Package> {
    Package::open(Arc::from(bytes), &Limits::default()).ok()
}

/// Every story block's id, part by part, as the reading names them.
fn ids(docx: &Docx) -> Vec<(String, Vec<String>)> {
    docx.parts
        .iter()
        .map(|p| {
            let mut ids: Vec<String> = p.ids.paragraphs().iter().map(|q| q.id).collect();
            for table in p.ids.tables().iter() {
                ids.push(table.id.clone());
                ids.extend(table.rows.iter().map(|(_, r)| r.clone()));
            }
            (p.part.clone(), ids)
        })
        .collect()
}

#[test]
fn every_document_opens_stamped_the_same_each_time_and_saves_as_it_was() {
    let (mut opened_count, mut stamped) = (0, 0);
    for file in documents() {
        let bytes = std::fs::read(&file).unwrap();
        let Some(package) = opened(&bytes) else { continue };
        let (word, _) = WordPackage::open(package);
        let Ok(docx) = Docx::open(&word, std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX))) else {
            continue;
        };
        let what = file.display();
        opened_count += 1;
        // no block without a valid and unique id, in any story
        for part in &docx.parts {
            assert!(
                part.ids.paragraphs().iter().all(|p| p.state == IdState::Ok),
                "{what}: {}",
                part.part
            );
            for table in part.ids.tables().iter() {
                assert!(
                    table.rows.iter().all(|(_, r)| r.contains("tr:")),
                    "{what}: {}",
                    table.id
                );
            }
        }
        // the ids are the bytes': opened again, the same
        let (again, _) = WordPackage::open(opened(&bytes).unwrap());
        assert_eq!(
            ids(&docx),
            ids(&Docx::open(&again, std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX))).unwrap()),
            "{what}"
        );
        // nothing written: the save is the bytes
        let unwritten = word.unwritten();
        stamped += usize::from(!unwritten.is_empty());
        assert!(word.save().unwrap() == bytes, "{what}: saved bytes differ");
        // every part written and saved, then opened: the same ids, none given at open
        let written = word.written().save().unwrap();
        let (reopened, _) = WordPackage::open(opened(&written).unwrap());
        assert!(reopened.unwritten().is_empty(), "{what}: {:?}", reopened.unwritten());
        assert_eq!(
            ids(&docx),
            ids(&Docx::open(&reopened, std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX))).unwrap()),
            "{what}"
        );
    }
    assert!(
        opened_count > 200 && stamped > 200,
        "{opened_count} documents, {stamped} stamped"
    );
}

#[test]
fn a_partly_written_document_opens_again_with_the_same_ids() {
    let parts = Parts {
        body: "<w:p><w:r><w:t>one</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>".into(),
        styles: Some(String::new()),
        ..Parts::default()
    };
    let (word, _) = WordPackage::open(raw_package(&parts));
    assert_eq!(word.unwritten(), vec!["word/document.xml".to_owned()]);
    let docx = Docx::open(&word, std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX))).unwrap();
    // the body written, the rest as it was
    let tree = word.tree("word/document.xml").unwrap().unwrap();
    let partly = word.with_tree("word/document.xml", tree).unwrap().save().unwrap();
    let (reopened, _) = WordPackage::open(opened(&partly).unwrap());
    assert!(reopened.unwritten().is_empty());
    assert_eq!(
        ids(&docx),
        ids(&Docx::open(&reopened, std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX))).unwrap())
    );
    assert!(ids(&docx)[0]
        .1
        .iter()
        .all(|id| id.starts_with("p:") || id.starts_with("t:") || id.starts_with("tr:")));
}

#[test]
fn a_part_stamped_again_from_the_same_content_keeps_its_tree() {
    let parts = Parts {
        body: "<w:p/><w:p/>".into(),
        ..Parts::default()
    };
    let raw = raw_package(&parts);
    let (first, _) = WordPackage::open(raw.clone());
    let (again, _) = WordPackage::open_after(raw, Some(&first));
    let (Some(PartVersion::Unwritten(a)), Some(PartVersion::Unwritten(b))) =
        (first.part("word/document.xml"), again.part("word/document.xml"))
    else {
        panic!("the body is unwritten");
    };
    assert!(a.tree().ptr_eq(b.tree()));
    // other bytes: stamped anew
    let other = opened(&package_bytes(&Parts {
        body: "<w:p/>".into(),
        ..Parts::default()
    }))
    .unwrap();
    let (other, _) = WordPackage::open_after(other, Some(&first));
    let Some(PartVersion::Unwritten(c)) = other.part("word/document.xml") else {
        panic!("the body is unwritten");
    };
    assert!(!a.tree().ptr_eq(c.tree()));
}

#[test]
fn twenty_thousand_paragraphs_are_stamped_at_open_with_one_draw_per_seed() {
    let mut body = String::new();
    for k in 0..20_000 {
        body.push_str(&format!("<w:p><w:r><w:t>Paragraph {k}</w:t></w:r></w:p>"));
    }
    body.push_str("<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
    let raw = raw_package(&Parts {
        body,
        ..Parts::default()
    });
    let started = Instant::now();
    let (word, pool) = WordPackage::open(raw);
    let took = started.elapsed();
    eprintln!("stamped at open in {took:?}");
    // one cursor per seed drawn from: the body's paragraphs and its rows
    let seeds: Vec<&String> = pool.cursors().keys().collect();
    assert_eq!(seeds, ["word/document.xml", "word/document.xml\0row"]);
    let tree = word.tree("word/document.xml").unwrap().unwrap();
    let ids = PartIds::new("body", tree.root());
    assert_eq!(
        ids.paragraphs().iter().filter(|p| p.state == IdState::Ok).count(),
        20_002
    );
    // generous (a debug build under load)
    assert!(took < Duration::from_secs(60), "stamping took {took:?}");
}

#[test]
fn one_value_written_two_ways_is_a_repeat_and_the_second_gets_an_id_of_its_own() {
    // repeats are compared by value, whatever the spelling (`1A`, `0000001A`): the first
    // keeps its id, the second is stamped at open, as Word keeps the first of a repeat
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let w14 = "http://schemas.microsoft.com/office/word/2010/wordml";
    let xml = format!(
        r#"<w:document xmlns:w="{w}" xmlns:w14="{w14}"><w:body><w:p w14:paraId="1A" w14:textId="1B"/><w:p w14:paraId="0000001A" w14:textId="1C"/></w:body></w:document>"#
    );
    let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap();
    let ids = PartIds::new("body", tree.root());
    let states: Vec<IdState> = ids.paragraphs().iter().map(|p| p.state).collect();
    assert_eq!(states, [IdState::Ok, IdState::Repeat]);
    assert!(!ids.fully_stamped());
    assert_eq!(ids.get("p:1A"), Some(ooxml_word_model::ids::Entry::Paragraph(0)));
    assert_eq!(ids.get("p:0000001A"), None, "the repeat is no block's id");
    let found = ooxml_word_model::stamp::unstamped("word/document.xml", tree.root(), &ids, false).unwrap();
    let mut pool = ooxml_word_model::ids::IdPool::of(ooxml_word_model::ids::used_long_hex(&[&**tree.root()]));
    let mut draw = |seed: &str, count: usize| Ok::<_, std::convert::Infallible>(pool.generate(seed, count));
    let Ok(stamp) = found.stamp(&mut draw);
    let after = PartIds::new("body", &stamp.root);
    assert!(after.fully_stamped());
    let named: Vec<String> = after.paragraphs().iter().map(|p| p.id).collect();
    assert_eq!(named[0], "p:1A");
    assert_ne!(named[1], "p:0000001A");
}
