//! Owns: the record of where a chart's values came from, as ooxml-edit keeps it in the chart
//! part itself (`charts/record.py`): the `json` attribute of a `uvp:record` in the chart
//! space's own extension (`c:extLst/c:ext uri=`[`RECORD_URI`]) -- read ([`record_text`]),
//! and written or taken away ([`with_record`], `write_record`: an existing record replaced
//! where it stands, a new one in an extension after the others, the extension list made
//! last where there is none, and taken away with the last extension).
//!
//! Must not: read or write the record's JSON (the caller does), or know which document
//! holds the chart.

use super::path::{appended, indexed, replaced, without};
use ooxml_xml::view::local;
use ooxml_xml::{Atom, Attribute, Element, NamespaceDecl, Node, QName};
use std::sync::Arc;

/// The extension's `uri`, and the namespace of the element holding the record.
pub const RECORD_URI: &str = "urn:uvrt:ooxml-edit:provenance:v1";

/// The record's text (its `json` attribute) in the chart space `root`; `None` where it has
/// none or an empty one.
#[must_use]
pub fn record_text(root: &Element) -> Option<&str> {
    let (_, ext) = extension(root)?;
    let node = ext.child_elements().find(|e| is_record(e))?;
    node.attribute(None, "json").filter(|t| !t.is_empty())
}

/// `root` with its record's text `text` (`None`: the record taken away, with an extension
/// list it leaves empty); `None` where nothing changes.
#[must_use]
pub fn with_record(root: &Element, text: Option<&str>) -> Option<Element> {
    let list = indexed(root).find(|(_, e)| local(e) == "extLst");
    let found = extension(root);
    let Some(text) = text else {
        let ((list_at, list), (ext_at, _)) = (list?, found?);
        let left = without(list, ext_at);
        return Some(if left.child_elements().next().is_none() {
            without(root, list_at)
        } else {
            replaced(root, list_at, left)
        });
    };
    let record = Node::Element(Arc::new(record(text)));
    if let (Some((list_at, list)), Some((ext_at, ext))) = (list, found) {
        let only = ext.children().len() == 1 && ext.child_elements().next().is_some_and(|e| is_record(e));
        if only && record_text(root) == Some(text) {
            return None;
        }
        let ext = ext.with_children(ooxml_xml::Children::from_vec(vec![record]));
        return Some(replaced(root, list_at, replaced(list, ext_at, ext)));
    }
    let ext = Element::new(
        c_name(root, "ext"),
        Vec::new(),
        vec![Attribute {
            name: Arc::new(QName::new(None, "uri", None)),
            value: Atom::from(RECORD_URI),
        }],
        vec![record],
    );
    Some(match list {
        Some((list_at, list)) => replaced(root, list_at, appended(list, ext)),
        // CT_ChartSpace ends with extLst: whatever is there comes before it
        None => appended(
            root,
            Element::new(
                c_name(root, "extLst"),
                Vec::new(),
                Vec::new(),
                vec![Node::Element(Arc::new(ext))],
            ),
        ),
    })
}

/// The record's extension: its place in the extension list, and itself.
fn extension(root: &Element) -> Option<(usize, &Element)> {
    let (_, list) = indexed(root).find(|(_, e)| local(e) == "extLst")?;
    indexed(list).find(|(_, e)| local(e) == "ext" && e.attribute(None, "uri") == Some(RECORD_URI))
}

fn is_record(element: &Element) -> bool {
    element.name().is(Some(RECORD_URI), "record")
}

/// `<uvp:record xmlns:uvp=RECORD_URI json=text/>`.
fn record(text: &str) -> Element {
    Element::new(
        Arc::new(QName::new(Some("uvp"), "record", Some(RECORD_URI))),
        vec![NamespaceDecl {
            prefix: Some(Atom::from("uvp")),
            uri: Atom::from(RECORD_URI),
        }],
        vec![Attribute {
            name: Arc::new(QName::new(None, "json", None)),
            value: Atom::from(text),
        }],
        Vec::new(),
    )
}

/// A chart element `local_name`, named as the chart space is.
fn c_name(root: &Element, local_name: &str) -> ooxml_xml::Name {
    Arc::new(QName::new(root.name().prefix(), local_name, root.name().namespace()))
}

#[cfg(test)]
mod tests {
    use super::{record_text, with_record};
    use ooxml_xml::{parse, Limits};

    const C: &str = r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart""#;

    fn root(xml: &str) -> std::sync::Arc<ooxml_xml::Element> {
        std::sync::Arc::clone(parse(xml.as_bytes(), &Limits::default()).expect("xml").root())
    }

    fn xml(element: ooxml_xml::Element) -> String {
        let tree = ooxml_xml::Tree::from_root(std::sync::Arc::new(element));
        String::from_utf8(tree.write()).expect("utf-8")
    }

    #[test]
    fn a_record_made_last_replaced_where_it_stands_and_taken_away() {
        let bare = root(&format!(r#"<c:chartSpace {C}><c:chart/></c:chartSpace>"#));
        assert_eq!(record_text(&bare), None);
        assert_eq!(with_record(&bare, None), None);
        let made = with_record(&bare, Some(r#"{"v":1}"#)).expect("made");
        let text = xml(made.clone());
        assert!(text.contains(r#"<c:extLst><c:ext uri="urn:uvrt:ooxml-edit:provenance:v1"><uvp:record xmlns:uvp="urn:uvrt:ooxml-edit:provenance:v1" json="{&quot;v&quot;:1}"/></c:ext></c:extLst></c:chartSpace>"#), "{text}");
        assert_eq!(record_text(&made), Some(r#"{"v":1}"#));
        assert_eq!(with_record(&made, Some(r#"{"v":1}"#)), None);
        let gone = with_record(&made, None).expect("taken away");
        assert_eq!(xml(gone), xml((*bare).clone()));
    }

    #[test]
    fn other_extensions_keep_their_place() {
        let other = r#"<c:ext uri="x"><y/></c:ext>"#;
        let kept = root(&format!(
            r#"<c:chartSpace {C}><c:chart/><c:extLst>{other}<c:ext uri="urn:uvrt:ooxml-edit:provenance:v1"><uvp:record xmlns:uvp="urn:uvrt:ooxml-edit:provenance:v1" json="a"/></c:ext></c:extLst></c:chartSpace>"#
        ));
        let changed = xml(with_record(&kept, Some("b")).expect("replaced"));
        assert!(
            changed.contains(&format!(
                r#"<c:extLst>{other}<c:ext uri="urn:uvrt:ooxml-edit:provenance:v1"><uvp:record"#
            )),
            "{changed}"
        );
        assert!(changed.contains(r#"json="b""#));
        let gone = xml(with_record(&kept, None).expect("taken away"));
        assert!(
            gone.ends_with(&format!(r#"<c:extLst>{other}</c:extLst></c:chartSpace>"#)),
            "{gone}"
        );
    }
}
