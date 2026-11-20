//! The write contract on the corners of XML: each case's expected bytes are what
//! `ooxml_edit.xml.serialize(parse_xml(raw))` wrote (lxml 6.1.3, libxml2 2.14.6), and each
//! refused case is one lxml refuses too (apart from the internal DTD subset, which this
//! crate refuses where lxml would write it in libxml2's own form).
//! `tools/rust_oracles.py xml-pairs` and `examples/xml_contract.rs` run the same check over
//! every part of every package in the repository.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_xml::{parse, Limits, XmlError};

const DECL: &[u8] = b"<?xml version='1.0' encoding='UTF-8' standalone='yes'?>\n";

fn written(raw: &[u8]) -> Vec<u8> {
    parse(raw, &Limits::default())
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(raw)))
        .write()
}

fn same_as_lxml(raw: &[u8], body: &[u8]) {
    let mut want = DECL.to_vec();
    want.extend_from_slice(body);
    assert_eq!(
        String::from_utf8_lossy(&written(raw)),
        String::from_utf8_lossy(&want),
        "input {:?}",
        String::from_utf8_lossy(raw)
    );
}

#[test]
fn elements_attributes_and_namespaces() {
    same_as_lxml(b"<a></a>", b"<a/>");
    same_as_lxml(b"<a  b = \"1\" ></a >", b"<a b=\"1\"/>");
    same_as_lxml(
        b"<a xmlns:w=\"u\" w:x=\"1\" xmlns=\"d\" c=\"2\"/>",
        b"<a xmlns:w=\"u\" xmlns=\"d\" w:x=\"1\" c=\"2\"/>",
    );
    same_as_lxml(
        b"<a xmlns:x=\"u\"><x:b xmlns:x=\"v\"/></a>",
        b"<a xmlns:x=\"u\"><x:b xmlns:x=\"v\"/></a>",
    );
    same_as_lxml(b"<a xml:space=\"preserve\"/>", b"<a xml:space=\"preserve\"/>");
    same_as_lxml(b"<a xmlns=\"\"/>", b"<a xmlns=\"\"/>");
    same_as_lxml(b"<a xmlns:xml=\"http://www.w3.org/XML/1998/namespace\"/>", b"<a/>");
    same_as_lxml(b"<a b=\"\"/>", b"<a b=\"\"/>");
}

#[test]
fn text_and_attribute_values() {
    same_as_lxml(b"<a b=\"x\r\ny\tz\n\"/>", b"<a b=\"x y z \"/>");
    same_as_lxml(b"<a>x\r\ny\rz</a>", b"<a>x\ny\nz</a>");
    same_as_lxml(b"<a><![CDATA[<x>&]]>t</a>", b"<a>&lt;x&gt;&amp;t</a>");
    same_as_lxml(
        b"<a b=\"&#10;&#9;&#13;&quot;'&lt;&gt;&amp;\"/>",
        b"<a b=\"&#10;&#9;&#13;&quot;'&lt;&gt;&amp;\"/>",
    );
    same_as_lxml(b"<a b='\"'/>", b"<a b=\"&quot;\"/>");
    same_as_lxml(b"<a>&#xD;</a>", b"<a>&#13;</a>");
    same_as_lxml(b"<a>\t</a>", b"<a>\t</a>");
    same_as_lxml(
        "<a>\u{e9} &#169; &#x1F600;</a>".as_bytes(),
        "<a>\u{e9} \u{a9} \u{1f600}</a>".as_bytes(),
    );
    same_as_lxml(
        "<a b=\"\u{85}\">\u{85}\u{2028}</a>".as_bytes(),
        "<a b=\"\u{85}\">\u{85}\u{2028}</a>".as_bytes(),
    );
}

#[test]
fn around_the_root() {
    same_as_lxml(
        b"<?xml version=\"1.0\"?>\n<!-- c --><?pi  x  y ?><a/><!--after--><?p2?>",
        b"<!-- c --><?pi x  y ?><a/><!--after--><?p2?>",
    );
    same_as_lxml(b"<a/>\n<!--c-->\n", b"<a/><!--c-->");
    same_as_lxml(
        b"<a><?xml-stylesheet href=\"x\"?></a>",
        b"<a><?xml-stylesheet href=\"x\"?></a>",
    );
    same_as_lxml(b"<?xml version = \"1.0\" encoding=\"utf-8\" ?>\r\n<a/>", b"<a/>");
    same_as_lxml(b"<!DOCTYPE a><a/>", b"<!DOCTYPE a>\n<a/>");
    same_as_lxml(
        b"<!DOCTYPE a SYSTEM \"x.dtd\"><a/>",
        b"<!DOCTYPE a SYSTEM \"x.dtd\">\n<a/>",
    );
    same_as_lxml(
        b"<!DOCTYPE a PUBLIC \"p\" \"s\"><a/>",
        b"<!DOCTYPE a PUBLIC \"p\" \"s\">\n<a/>",
    );
    same_as_lxml(b"<!DOCTYPE a SYSTEM 'x\"y'><a/>", b"<!DOCTYPE a SYSTEM 'x\"y'>\n<a/>");
    same_as_lxml(
        b"<!DOCTYPE a SYSTEM \"x.dtd\"><a>x&e;y&amp;</a>",
        b"<!DOCTYPE a SYSTEM \"x.dtd\">\n<a>x&e;y&amp;</a>",
    );
}

#[test]
fn encodings() {
    let utf16: Vec<u8> = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-16\"?><a>\u{e9}</a>"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    same_as_lxml(&utf16, "<a>\u{e9}</a>".as_bytes());
    let be: Vec<u8> = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a/>"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    same_as_lxml(&be, b"<a/>");
    same_as_lxml(b"\xef\xbb\xbf<a/>", b"<a/>");
    same_as_lxml(
        b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\xe9</a>",
        "<a>\u{e9}</a>".as_bytes(),
    );
}

fn refused(raw: &[u8]) -> XmlError {
    match parse(raw, &Limits::default()) {
        Err(e) => e,
        Ok(_) => panic!("accepted {:?}", String::from_utf8_lossy(raw)),
    }
}

#[test]
fn what_lxml_refuses_is_refused() {
    for raw in [
        &b"<w:p/>"[..],
        b"<a x:b=\"1\"/>",
        b"<a>]]></a>",
        b"<a><!-- x -- y --></a>",
        b"<a>\x01</a>",
        b"<a>&#1;</a>",
        b"<a>&#0;</a>",
        b"<a>&#x110000;</a>",
        b"<a>\xef\xbf\xbe</a>",
        b"<a>\xed\xa0\x80</a>",
        b" <?xml version=\"1.0\"?><a/>",
        b"<a b=\"1\" b=\"2\"/>",
        b"<a b=\"<\"/>",
        b"<1a/>",
        b"<a:b:c xmlns:a=\"u\"/>",
        b"<a xmlns:p=\"\"/>",
        b"",
        b"<a",
        b"<a><b/></a><c/>",
        b"<?XML version=\"1.0\"?><a/>",
        b"<a><?xml x?></a>",
        b"<!DOCTYPE a PUBLIC \"p\"><a/>",
        b"<?xml version=\"1.0\" encoding=\"US-ASCII\"?><a>\xc3</a>",
        b"<a xmlns:x=\"u\" xmlns:y=\"u\" x:b=\"1\" y:b=\"2\"/>",
    ] {
        refused(raw);
    }
    assert!(matches!(refused(b"<a>&e;</a>"), XmlError::UndefinedEntity { .. }));
    assert!(matches!(refused(b"<w:p/>"), XmlError::UndefinedPrefix { .. }));
    let standalone = b"<?xml version=\"1.0\" standalone=\"yes\"?><!DOCTYPE a SYSTEM \"x\"><a>&e;</a>";
    assert!(matches!(refused(standalone), XmlError::UndefinedEntity { .. }));
}

#[test]
fn what_this_crate_refuses_rather_than_approximates() {
    // lxml writes these in libxml2's own forms; the core refuses them (design section 11)
    assert!(matches!(
        refused(b"<!DOCTYPE a [<!ENTITY e \"x\">]><a>&e;</a>"),
        XmlError::Unsupported { .. }
    ));
    assert!(matches!(
        refused(b"<!DOCTYPE a SYSTEM \"x\"><a b=\"&e;\"/>"),
        XmlError::Unsupported { .. }
    ));
    assert!(matches!(
        refused(b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><a/>"),
        XmlError::Encoding(_)
    ));
}

#[test]
fn limits() {
    let deep = |n: usize| [b"<a>".repeat(n), b"</a>".repeat(n)].concat();
    assert!(parse(&deep(256), &Limits::default()).is_ok());
    assert!(matches!(
        parse(&deep(257), &Limits::default()),
        Err(XmlError::Limit {
            what: "element depth",
            ..
        })
    ));
    let tight = Limits {
        max_text_len: 10,
        max_attribute_len: 10,
        max_name_len: 5,
        max_attributes: 2,
        max_nodes: 5,
        ..Limits::default()
    };
    for raw in [
        &b"<a>12345678901</a>"[..],
        b"<a b=\"12345678901\"/>",
        b"<abcdef/>",
        b"<a b=\"1\" c=\"2\" d=\"3\"/>",
        b"<a><b/><b/><b/><b/><b/></a>",
    ] {
        assert!(
            matches!(parse(raw, &tight), Err(XmlError::Limit { .. })),
            "{:?}",
            String::from_utf8_lossy(raw)
        );
    }
}

#[test]
fn equal_small_subtrees_are_shared() {
    let tree = parse(
        b"<r><p><b/><s v=\"2\"/></p><p><b/><s v=\"2\"/></p></r>",
        &Limits::default(),
    )
    .unwrap();
    let ps: Vec<_> = tree.root().child_elements().collect();
    assert_eq!(ps.len(), 2);
    assert!(std::sync::Arc::ptr_eq(ps[0], ps[1]));
    assert_eq!(
        tree.write(),
        [DECL, b"<r><p><b/><s v=\"2\"/></p><p><b/><s v=\"2\"/></p></r>"].concat()
    );
}
