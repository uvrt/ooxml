//! Owns: the records' item made where a document has none (docx-agent's
//! `_add_provenance_item`): `customXml/itemN.xml` and `customXml/itemPropsN.xml` at the first
//! number neither name takes, the item an empty `prov:sources`, its properties a
//! `ds:datastoreItem` whose `ds:itemID` is made from the first key it is made for (`uuid5`
//! of the URL namespace and `NS#key`: the same edit writes the same bytes) and whose
//! `ds:schemaRef` names [`NS`]; their content types declared as ooxml-edit's `add_part`
//! declares them (the properties' with `override=True`), a `customXmlProps` relationship
//! from the item and a `customXml` relationship from the main part.
//!
//! Must not: make a second item (the caller looks for the one there is first), or write
//! but through the sink.

use crate::new_part::{add_internal, declare_type, declare_type_override};
use crate::sink::EditSink;
use crate::Result;
use ooxml_base::uuid::{braced_upper, uuid5, NAMESPACE_URL};
use ooxml_opc::name::rels_for;
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_word_model::docx::provenance::{DS, ITEM_TYPE, NS, PROPS_TYPE, REL_CUSTOM_XML, REL_CUSTOM_XML_PROPS};
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Attribute, Element, NamespaceDecl, Node, QName, Tree};
use std::sync::Arc;

/// The item made for `docx` through `sink`, its first entry to be keyed `first`; its name.
///
/// # Errors
/// When the content types or a relationships part are not XML.
pub(super) fn make_item(sink: &mut EditSink<'_, '_>, docx: &Docx, first: &str) -> Result<String> {
    let package = sink.doc().package().clone();
    let number = (1..)
        .find(|n: &u32| {
            !package.has_part(&format!("customXml/item{n}.xml"))
                && !package.has_part(&format!("customXml/itemProps{n}.xml"))
        })
        .unwrap_or(1);
    let (item, props) = (
        format!("customXml/item{number}.xml"),
        format!("customXml/itemProps{number}.xml"),
    );
    let mut made = package.with_new_tree(&item, tree(sources()));
    made = declare_type(&made, &item, ITEM_TYPE)?.unwrap_or(made);
    made = made.with_new_tree(&props, tree(properties(first)));
    made = declare_type_override(&made, &props, PROPS_TYPE)?.unwrap_or(made);
    made = add_internal(&made, &item, &props, REL_CUSTOM_XML_PROPS)?
        .0
        .unwrap_or(made);
    made = add_internal(&made, &docx.main, &item, REL_CUSTOM_XML)?
        .0
        .unwrap_or(made);
    for name in [
        item.clone(),
        props,
        rels_for(&item),
        CONTENT_TYPES_PART.to_owned(),
        rels_for(&docx.main),
    ] {
        put_changed(sink, &package, &made, &name)?;
    }
    Ok(item)
}

/// `name` put through `sink` where `made` has it otherwise than `before`.
fn put_changed(sink: &mut EditSink<'_, '_>, before: &Package, made: &Package, name: &str) -> Result<()> {
    let Some(now) = made.tree(name)? else {
        return Ok(());
    };
    let unchanged = before
        .tree(name)
        .ok()
        .flatten()
        .is_some_and(|then| Arc::ptr_eq(then.root(), now.root()));
    if !unchanged {
        sink.put(name, now)?;
    }
    Ok(())
}

fn tree(root: Element) -> Tree {
    Tree::from_root(Arc::new(root))
}

fn declared(prefix: &str, uri: &str) -> NamespaceDecl {
    NamespaceDecl {
        prefix: Some(Arc::from(prefix)),
        uri: Arc::from(uri),
    }
}

/// `<prov:sources xmlns:prov=NS/>`.
fn sources() -> Element {
    Element::new(
        Arc::new(QName::new(Some("prov"), "sources", Some(NS))),
        vec![declared("prov", NS)],
        Vec::new(),
        Vec::new(),
    )
}

/// The item's properties: `<ds:datastoreItem ds:itemID="{...}"><ds:schemaRefs><ds:schemaRef
/// ds:uri=NS/></ds:schemaRefs></ds:datastoreItem>`.
fn properties(first: &str) -> Element {
    let ds = |local: &str| Arc::new(QName::new(Some("ds"), local, Some(DS)));
    let item_id = braced_upper(&uuid5(&NAMESPACE_URL, &format!("{NS}#{first}")));
    let reference = Element::new(
        ds("schemaRef"),
        Vec::new(),
        vec![Attribute {
            name: ds("uri"),
            value: Arc::from(NS),
        }],
        Vec::new(),
    );
    let references = Element::new(
        ds("schemaRefs"),
        Vec::new(),
        Vec::new(),
        vec![Node::Element(Arc::new(reference))],
    );
    Element::new(
        ds("datastoreItem"),
        vec![declared("ds", DS)],
        vec![Attribute {
            name: ds("itemID"),
            value: Arc::from(item_id.as_str()),
        }],
        vec![Node::Element(Arc::new(references))],
    )
}
