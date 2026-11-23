//! A made-up package through open, read, parse, new versions and save.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_opc::{Limits, OpcError, Package, PartVersion};
use ooxml_xml::{Cursor, Node};
use ooxml_zip::{Archive, Method, NewEntry, Writer};
use std::sync::Arc;

const TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="PNG" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="urn:image" Target="media/a.png"/><Relationship Id="rId2" Type="urn:link" Target="https://example.com/" TargetMode="External"/><Relationship Id="rId3" Type="urn:custom" Target="../customXml/item1.xml"/><Relationship Type="urn:broken" Target="x"/></Relationships>"#;
const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>One</w:t></w:r></w:p></w:body></w:document>"#;

fn made_up() -> Vec<u8> {
    let mut w = Writer::new();
    let entry = |name: &str, method| NewEntry {
        name: name.into(),
        method,
        dos_time: 0x6000,
        dos_date: 0x5a21,
        external_attr: 0,
        create_system: 0,
    };
    for (name, data, method) in [
        ("[Content_Types].xml", TYPES.as_bytes(), Method::Deflated),
        ("_rels/.rels", ROOT_RELS.as_bytes(), Method::Deflated),
        ("word/", b"".as_slice(), Method::Stored),
        ("word/document.xml", DOC.as_bytes(), Method::Deflated),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes(), Method::Deflated),
        ("word/media/a.png", b"\x89PNG\r\n".as_slice(), Method::Stored),
    ] {
        w.add(&entry(name, method), data).unwrap();
    }
    w.finish(b"").unwrap()
}

fn open(bytes: &[u8]) -> Package {
    Package::open(Arc::from(bytes), &Limits::default()).unwrap()
}

#[test]
fn reads_parts_relationships_and_types() {
    let p = open(&made_up());
    assert_eq!(
        p.part_names(),
        [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/media/a.png"
        ]
    );
    assert!(p.has_part("/word/document.xml") && p.has_part("word\\media\\a.png"));
    assert_eq!(&*p.read("word/media/a.png").unwrap().unwrap(), b"\x89PNG\r\n");
    let main = p.relationships("").unwrap();
    assert_eq!(
        main.targets_of_type("http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument")
            .collect::<Vec<_>>(),
        ["word/document.xml"]
    );
    let rels = p.relationships("word/document.xml").unwrap();
    assert_eq!(rels.len(), 3, "the relationship without an id is skipped");
    assert_eq!(
        rels.get("rId1").unwrap().target_part.as_deref(),
        Some("word/media/a.png")
    );
    assert!(rels.get("rId2").unwrap().external && rels.get("rId2").unwrap().target_part.is_none());
    assert_eq!(
        rels.get("rId3").unwrap().target_part.as_deref(),
        Some("customXml/item1.xml")
    );
    let types = p.content_types().unwrap();
    assert_eq!(
        types.of("word/document.xml"),
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml")
    );
    assert_eq!(types.of("word/media/a.png"), Some("image/png"));
    assert_eq!(types.of("word/settings.xml"), Some("application/xml"));
    assert_eq!(types.of("word/x.bin"), None);
}

#[test]
fn parts_are_parsed_once_and_shared_by_versions() {
    let p = open(&made_up());
    assert!(matches!(p.part("word/document.xml"), Some(PartVersion::Raw(_))));
    let v1 = p.with_bytes("word/media/b.png", Arc::from(&b"png"[..]));
    let (t0, t1) = (
        p.tree("word/document.xml").unwrap().unwrap(),
        v1.tree("word/document.xml").unwrap().unwrap(),
    );
    assert!(
        t0.ptr_eq(&t1),
        "an untouched part's tree is parsed once for every version"
    );
    // an edit through a cursor makes a new version; the old one is unchanged
    let text = Cursor::root(&t0)
        .child(0)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap();
    let edited = text.replace(Node::Text("Two".into())).unwrap().apply(&t0).unwrap();
    let v2 = v1.with_tree("word/document.xml", edited).unwrap();
    assert_eq!(
        v2.tree("word/document.xml").unwrap().unwrap().root().text_content(),
        "Two"
    );
    assert_eq!(
        p.tree("word/document.xml").unwrap().unwrap().root().text_content(),
        "One"
    );
    assert!(matches!(p.with_tree("word/missing.xml", t0), Err(OpcError::NoPart(_))));
    assert!(v2
        .without("word/media/a.png")
        .unwrap()
        .read("word/media/a.png")
        .unwrap()
        .is_none());
}

#[test]
fn saves_raw_unchanged_and_written_when_touched() {
    let bytes = made_up();
    let p = open(&bytes);
    assert_eq!(p.save().unwrap(), bytes, "an unchanged package saves byte for byte");
    let tree = p.tree("word/document.xml").unwrap().unwrap();
    let saved = p
        .with_tree("word/document.xml", tree.clone())
        .unwrap()
        .with_bytes("word/media/new.xml", Arc::from(&b"<n/>"[..]))
        .save()
        .unwrap();
    let again = Archive::read(Arc::from(saved), &Default::default()).unwrap();
    let names: Vec<&str> = again.entries().iter().map(|e| e.meta().name.as_str()).collect();
    assert_eq!(
        names,
        [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/media/a.png",
            "word/media/new.xml"
        ]
    );
    let doc = &again.entries()[3];
    assert_eq!(again.read_entry(doc).unwrap(), tree.write());
    assert_eq!(
        (doc.meta().dos_time, doc.meta().dos_date, doc.meta().external_attr),
        (0x6000, 0x5a21, 0o600 << 16)
    );
    let new = again.entries()[6].meta();
    assert_eq!(
        (new.method, new.dos_date, new.version_made_by >> 8),
        (Method::Deflated, 33, 3)
    );
}

#[test]
fn refuses_two_entries_for_one_part() {
    let mut w = Writer::new();
    for name in ["word/a.xml", "/word/a.xml"] {
        w.add(
            &NewEntry {
                name: name.into(),
                method: Method::Stored,
                dos_time: 0,
                dos_date: 33,
                external_attr: 0,
                create_system: 3,
            },
            b"<a/>",
        )
        .unwrap();
    }
    let bytes = w.finish(b"").unwrap();
    assert!(matches!(
        Package::open(Arc::from(bytes), &Limits::default()),
        Err(OpcError::DuplicatePart(_))
    ));
}

#[test]
fn a_restored_part_is_exactly_as_it_was() {
    let bytes = made_up();
    let first = open(&bytes);
    let doc = first.tree("word/document.xml").unwrap().unwrap();
    let changed = first
        .with_bytes("word/document.xml", Arc::from(&b"<x/>"[..]))
        .with_new_tree("word/extra.xml", doc.clone())
        .without("word/media/a.png")
        .unwrap();
    assert!(!changed
        .part("word/document.xml")
        .unwrap()
        .same(first.part("word/document.xml").unwrap()));
    assert!(changed.part("word/extra.xml").unwrap().same(&PartVersion::Tree(doc)));
    let mut back = changed.clone();
    for name in ["word/document.xml", "word/extra.xml", "word/media/a.png"] {
        back = back.restored(name, &first).unwrap();
    }
    assert_eq!(back.part_names(), first.part_names());
    for name in first.part_names() {
        assert!(back.part(&name).unwrap().same(first.part(&name).unwrap()), "{name}");
    }
    // the raw entries come back raw: an unchanged package saves as it was read
    assert_eq!(back.save().unwrap(), bytes);
    // and forward again
    let again = back.restored("word/extra.xml", &changed).unwrap();
    assert!(again.has_part("word/extra.xml"));
    let other = open(&made_up());
    assert!(matches!(
        back.restored("word/document.xml", &other),
        Err(OpcError::OtherPackage(_))
    ));
}

#[test]
fn the_parts_changed_are_found_by_identity_else_by_bytes() {
    let bytes = made_up();
    let first = open(&bytes);
    // opened apart from the same bytes: no part is the same by identity, every one by bytes
    assert!(open(&bytes).changed_parts(&first).is_empty());
    let edited = first
        .with_bytes("word/media/a.png", Arc::from(&b"\x89PNG\r\nnew"[..]))
        .without("word/_rels/document.xml.rels")
        .unwrap()
        .with_bytes("word/media/b.png", Arc::from(&b"\x89PNG"[..]));
    assert_eq!(
        edited.changed_parts(&first),
        ["word/media/a.png", "word/media/b.png", "word/_rels/document.xml.rels"]
    );
    // a part put anew with the same bytes is the same
    let again = first.with_bytes("word/media/a.png", Arc::from(&b"\x89PNG\r\n"[..]));
    assert!(again.changed_parts(&first).is_empty());
}

#[test]
fn two_trees_are_the_same_part_by_structure_as_by_the_bytes_they_write() {
    let first = open(&made_up());
    let parse = |xml: &str| ooxml_xml::parse(xml.as_bytes(), &Limits::default().xml).unwrap();
    let name = "word/document.xml";
    let a = first.with_tree(name, parse(DOC)).unwrap();
    // parsed apart: another allocation of the same structure
    let b = first.with_tree(name, parse(DOC)).unwrap();
    assert!(!a.part(name).unwrap().same(b.part(name).unwrap()));
    assert!(b.changed_parts(&a).is_empty());
    assert_eq!(a.read(name).unwrap(), b.read(name).unwrap());
    // one letter of the text changed, the rest shared
    let edited = parse(&DOC.replace("One", "Ono"));
    let c = a.with_tree(name, edited).unwrap();
    assert_eq!(c.changed_parts(&a), [name]);
    assert_ne!(a.read(name).unwrap(), c.read(name).unwrap());
    // an attribute added where the text is the same
    let d = a
        .with_tree(name, parse(&DOC.replace("<w:p>", r#"<w:p w:rsidR="00A1">"#)))
        .unwrap();
    assert_eq!(d.changed_parts(&a), [name]);
    // a stored part, read, against a tree: by the tree it reads as
    let stored = first.tree(name).unwrap().unwrap();
    let same = first.with_tree(name, parse(DOC)).unwrap();
    assert!(same.changed_parts(&first).is_empty() && first.changed_parts(&same).is_empty());
    let edited = first
        .with_tree(name, stored.with_root(c.tree(name).unwrap().unwrap().root().clone()))
        .unwrap();
    assert_eq!(edited.changed_parts(&first), [name]);
}

#[test]
fn a_touched_part_with_the_bytes_read_is_its_entry_as_read() {
    // co-authoring request 1: written again unchanged, a part keeps its entry -- local
    // header and central record as read, so its MS-FSSHTTPD chunk is the one a host holds
    let mut bytes = made_up();
    // mark every record as another writer does: "version made by" 4.5 on FAT, "version
    // needed" 1.0 (central records at 4 and 6, local headers at 4)
    let directory = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    let mut at = 0;
    while at + 4 <= bytes.len() {
        match &bytes[at..at + 4] {
            b"PK\x03\x04" => bytes[at + 4..at + 6].copy_from_slice(&10u16.to_le_bytes()),
            b"PK\x01\x02" if at >= directory => {
                bytes[at + 4..at + 6].copy_from_slice(&45u16.to_le_bytes());
                bytes[at + 6..at + 8].copy_from_slice(&10u16.to_le_bytes());
            }
            _ => {}
        }
        at += 1;
    }
    let p = open(&bytes);
    let png = p.read("word/media/a.png").unwrap().unwrap();
    let same = p.with_bytes("word/media/a.png", png).save().unwrap();
    assert_eq!(same, bytes, "the same bytes: the entry as read");
    let other = p
        .with_bytes("word/media/a.png", Arc::from(&b"\x89PNG\r\nx"[..]))
        .save()
        .unwrap();
    let again = Archive::read(Arc::from(other), &Default::default()).unwrap();
    let png = again
        .entries()
        .iter()
        .find(|e| e.meta().name == "word/media/a.png")
        .unwrap();
    assert_eq!((png.meta().version_needed, png.meta().version_made_by & 0xff), (20, 20));
    assert!(again.header_copyable(png));
}

/// A package whose every entry is stored with a data descriptor (flag bit 3, zero sizes in
/// the local header, the descriptor after the data), as writers that cannot seek make one.
fn with_descriptors(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let crc = |data: &[u8]| {
        let mut c = !0u32;
        for b in data {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { c >> 1 ^ 0xEDB8_8320 } else { c >> 1 };
            }
        }
        !c
    };
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in parts {
        let offset = out.len() as u32;
        let (crc, size, name_len) = (crc(data), data.len() as u32, name.len() as u16);
        let fields = |v: &mut Vec<u8>, sizes: bool| {
            for x in [20u16, 0x8, 0, 0x6000, 0x5a21] {
                v.extend_from_slice(&x.to_le_bytes());
            }
            for x in if sizes { [crc, size, size] } else { [0, 0, 0] } {
                v.extend_from_slice(&x.to_le_bytes());
            }
            v.extend_from_slice(&name_len.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
        };
        out.extend_from_slice(b"PK\x03\x04");
        fields(&mut out, false);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        out.extend_from_slice(b"PK\x07\x08");
        for x in [crc, size, size] {
            out.extend_from_slice(&x.to_le_bytes());
        }
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes());
        fields(&mut central, true);
        central.extend_from_slice(&[0; 6]); // comment length, disk, internal attributes
        central.extend_from_slice(&(0o600u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let (at, size, count) = (out.len() as u32, central.len() as u32, parts.len() as u16);
    out.append(&mut central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    for x in [count, count] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    for x in [size, at] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

#[test]
fn an_untouched_entry_with_a_data_descriptor_is_copied_as_it_is() {
    // co-authoring request 1: an untouched entry keeps its bytes, descriptor and all (its
    // chunk is what a host holds); a touched one is written anew, without a descriptor,
    // even with the bytes read -- its header does not hold its sizes
    let bytes = with_descriptors(&[
        ("[Content_Types].xml", TYPES.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", DOC.as_bytes()),
    ]);
    let p = open(&bytes);
    let read = Archive::read(Arc::from(bytes.clone()), &Default::default()).unwrap();
    assert!(read.entries().iter().all(|e| !read.header_copyable(e)));
    assert_eq!(p.save().unwrap(), bytes, "an unchanged package saves byte for byte");
    let doc = p.read("word/document.xml").unwrap().unwrap();
    let saved = p.with_bytes("word/document.xml", doc).save().unwrap();
    let again = Archive::read(Arc::from(saved.clone()), &Default::default()).unwrap();
    let flags: Vec<u16> = again.entries().iter().map(|e| e.meta().flags).collect();
    assert_eq!(flags, [0x8, 0x8, 0]);
    let doc_at = bytes.windows(17).position(|w| w == b"word/document.xml").unwrap() - 30;
    assert_eq!(saved[..doc_at], bytes[..doc_at], "the other entries as read");
    assert_eq!(again.read_entry(&again.entries()[2]).unwrap(), DOC.as_bytes());
}
