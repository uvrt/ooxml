//! Owns: [`Owned`], an element opened for changes the way lxml changes one -- children put
//! in, taken out and moved, attributes and text set -- over the immutable tree: a child is
//! opened only when it is changed (every other subtree stays shared), and the element is
//! closed into an [`Element`] again when the change is done. A child is put among its
//! siblings where a schema sequence wants it ([`Owned::insert_ordered`], ooxml-edit's
//! `insert_in_order`), and a child taken out leaves the text around it (`remove` keeps a
//! tail).
//!
//! Must not: know what a chart is.

use ooxml_xml::view::local;
use ooxml_xml::{Atom, Attribute, Element, NamespaceDecl, Node, QName};
use std::sync::Arc;

/// A schema sequence: the local names of each step (a choice lists several).
pub(crate) type Order = &'static [&'static [&'static str]];

/// One child of an [`Owned`] element: as it was, or opened.
#[derive(Debug, Clone)]
enum Child {
    Kept(Node),
    Open(Owned),
}

/// An element opened for changes.
#[derive(Debug, Clone)]
pub(crate) struct Owned {
    name: ooxml_xml::Name,
    namespaces: Vec<NamespaceDecl>,
    attributes: Vec<Attribute>,
    children: Vec<Child>,
}

impl Owned {
    /// `element` opened (its children shared until they are opened in turn).
    pub(crate) fn open(element: &Element) -> Owned {
        Owned {
            name: Arc::clone(element.name()),
            namespaces: element.namespaces().to_vec(),
            attributes: element.attributes().to_vec(),
            children: element.children().iter().cloned().map(Child::Kept).collect(),
        }
    }

    /// A new element `local_name` named in the namespace and with the prefix of `like`.
    pub(crate) fn like(like: &ooxml_xml::Name, local_name: &str) -> Owned {
        Owned::named(like.prefix(), local_name, like.namespace())
    }

    /// A new element `prefix:local_name` in `namespace`.
    pub(crate) fn named(prefix: Option<&str>, local_name: &str, namespace: Option<&str>) -> Owned {
        Owned {
            name: Arc::new(QName::new(prefix, local_name, namespace)),
            namespaces: Vec::new(),
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The element closed again.
    pub(crate) fn close(self) -> Element {
        let children = self
            .children
            .into_iter()
            .map(|child| match child {
                Child::Kept(node) => node,
                Child::Open(owned) => Node::Element(Arc::new(owned.close())),
            })
            .collect();
        Element::new(self.name, self.namespaces, self.attributes, children)
    }

    /// The element's name.
    pub(crate) fn name(&self) -> &ooxml_xml::Name {
        &self.name
    }

    /// Its local name.
    pub(crate) fn local(&self) -> &str {
        self.name.local()
    }

    /// Declare `prefix` for `uri` on this element.
    pub(crate) fn declare(&mut self, prefix: &str, uri: &str) {
        self.namespaces.push(NamespaceDecl {
            prefix: Some(Atom::from(prefix)),
            uri: Atom::from(uri),
        });
    }

    /// An attribute without a namespace.
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name.namespace().is_none() && a.name.local() == name)
            .map(|a| &*a.value)
    }

    /// Set the attribute `name` (no namespace) to `value`, in its place where it is there,
    /// last otherwise (as lxml sets one).
    pub(crate) fn set_attr(&mut self, name: &str, value: &str) {
        let found = self
            .attributes
            .iter_mut()
            .find(|a| a.name.namespace().is_none() && a.name.local() == name);
        match found {
            Some(attribute) => attribute.value = Atom::from(value),
            None => self.attributes.push(Attribute {
                name: Arc::new(QName::new(None, name, None)),
                value: Atom::from(value),
            }),
        }
    }

    /// `self` with the attribute set (for building).
    pub(crate) fn with_attr(mut self, name: &str, value: &str) -> Owned {
        self.set_attr(name, value);
        self
    }

    /// `self` with the attribute `prefix:local` of `namespace` set last (for building).
    pub(crate) fn with_ns_attr(mut self, (prefix, namespace): (&str, &str), local: &str, value: &str) -> Owned {
        self.attributes.push(Attribute {
            name: Arc::new(QName::new(Some(prefix), local, Some(namespace))),
            value: Atom::from(value),
        });
        self
    }

    /// `self` with `prefix` declared for `uri` (for building).
    pub(crate) fn declaring(mut self, prefix: &str, uri: &str) -> Owned {
        self.declare(prefix, uri);
        self
    }

    /// `self` with `child` last (for building).
    pub(crate) fn with(mut self, child: Owned) -> Owned {
        self.push(child);
        self
    }

    /// The text before the first child (lxml's `.text`).
    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            match child {
                Child::Kept(Node::Text(text)) => out.push_str(text),
                Child::Kept(Node::EntityRef(_)) => {}
                _ => break,
            }
        }
        out
    }

    /// Set the text before the first child.
    pub(crate) fn set_text(&mut self, value: &str) {
        let first = self
            .children
            .iter()
            .position(|c| !matches!(c, Child::Kept(Node::Text(_))))
            .unwrap_or(self.children.len());
        self.children.drain(..first);
        if !value.is_empty() {
            self.children.insert(0, Child::Kept(Node::Text(Atom::from(value))));
        }
    }

    /// `self` with its text set (for building; an empty text is written, as lxml writes one
    /// set to `""`, `<c:v></c:v>`).
    pub(crate) fn with_text(mut self, value: &str) -> Owned {
        self.set_text("");
        self.children.insert(0, Child::Kept(Node::Text(Atom::from(value))));
        self
    }

    /// The places of the element children, with their local names.
    pub(crate) fn elements(&self) -> Vec<(usize, String)> {
        self.children
            .iter()
            .enumerate()
            .filter_map(|(i, c)| match c {
                Child::Kept(Node::Element(e)) => Some((i, local(e).to_owned())),
                Child::Open(o) => Some((i, o.local().to_owned())),
                Child::Kept(_) => None,
            })
            .collect()
    }

    /// The places of the element children named `name` (local).
    pub(crate) fn places(&self, name: &str) -> Vec<usize> {
        self.elements()
            .into_iter()
            .filter(|(_, n)| n == name)
            .map(|(i, _)| i)
            .collect()
    }

    /// The place of the first element child named `name`.
    pub(crate) fn place(&self, name: &str) -> Option<usize> {
        self.places(name).into_iter().next()
    }

    /// The element child at `place`, read (closed again where it was opened).
    pub(crate) fn read(&self, place: usize) -> Option<Arc<Element>> {
        match self.children.get(place)? {
            Child::Kept(Node::Element(e)) => Some(Arc::clone(e)),
            Child::Open(o) => Some(Arc::new(o.clone().close())),
            Child::Kept(_) => None,
        }
    }

    /// The element child at `place`, opened for changes.
    pub(crate) fn at(&mut self, place: usize) -> Option<&mut Owned> {
        let child = self.children.get_mut(place)?;
        if let Child::Kept(Node::Element(e)) = child {
            *child = Child::Open(Owned::open(e));
        }
        match child {
            Child::Open(o) => Some(o),
            Child::Kept(_) => None,
        }
    }

    /// The first element child named `name`, opened.
    pub(crate) fn child(&mut self, name: &str) -> Option<&mut Owned> {
        let place = self.place(name)?;
        self.at(place)
    }

    /// The first child named `name`, made in schema order (`order`) where there is none,
    /// in the namespace of `like` (ooxml-edit's `subelement`).
    pub(crate) fn sub(&mut self, like: &ooxml_xml::Name, name: &str, order: Option<Order>) -> &mut Owned {
        let place = match self.place(name) {
            Some(place) => place,
            None => self.insert_ordered(order, Owned::like(like, name)),
        };
        // the place was found or made just now
        self.at(place).unwrap_or_else(|| unreachable!("an element child"))
    }

    /// Take out the child at `place` (the text around it stays).
    pub(crate) fn remove(&mut self, place: usize) -> Option<Owned> {
        if place >= self.children.len() {
            return None;
        }
        match self.children.remove(place) {
            Child::Kept(Node::Element(e)) => Some(Owned::open(&e)),
            Child::Open(o) => Some(o),
            Child::Kept(other) => {
                self.children.insert(place, Child::Kept(other));
                None
            }
        }
    }

    /// Put `child` in before the child at `place`.
    pub(crate) fn insert(&mut self, place: usize, child: Owned) {
        let place = place.min(self.children.len());
        self.children.insert(place, Child::Open(child));
    }

    /// Put a text node in before the child at `place`.
    pub(crate) fn insert_text(&mut self, place: usize, text: &str) {
        let place = place.min(self.children.len());
        self.children.insert(place, Child::Kept(Node::Text(Atom::from(text))));
    }

    /// The text right after the element child at `place` (lxml's `.tail`).
    pub(crate) fn tail(&self, place: usize) -> String {
        let mut out = String::new();
        for child in self.children.iter().skip(place + 1) {
            match child {
                Child::Kept(Node::Text(text)) => out.push_str(text),
                _ => break,
            }
        }
        out
    }

    /// The place right after the element child at `place` and its tail (where lxml's
    /// `addnext` puts an element).
    pub(crate) fn after(&self, place: usize) -> usize {
        (place + 1..self.children.len())
            .find(|&i| !matches!(self.children.get(i), Some(Child::Kept(Node::Text(_)))))
            .unwrap_or(self.children.len())
    }

    /// Move `from`'s element children here, last, each with the text after it (lxml's
    /// `append` of an element takes its tail along).
    pub(crate) fn take_elements(&mut self, from: &mut Owned) {
        let mut moving = false;
        for child in std::mem::take(&mut from.children) {
            match child {
                Child::Kept(Node::Element(_)) | Child::Open(_) => {
                    moving = true;
                    self.children.push(child);
                }
                Child::Kept(Node::Text(_)) if moving => self.children.push(child),
                other => {
                    moving = matches!(other, Child::Kept(Node::Comment(_)) | Child::Kept(Node::Pi(_)));
                    if moving {
                        self.children.push(other);
                    } else {
                        from.children.push(other);
                    }
                }
            }
        }
    }

    /// Put `child` last.
    pub(crate) fn push(&mut self, child: Owned) {
        self.children.push(Child::Open(child));
    }

    /// Put `child` where `order` wants it: before the first sibling ranked after it
    /// (siblings it does not name rank last), else last; last where `order` does not name it
    /// or there is none. Its place.
    pub(crate) fn insert_ordered(&mut self, order: Option<Order>, child: Owned) -> usize {
        let rank = |name: &str| order.and_then(|o| o.iter().position(|step| step.contains(&name)));
        let at = match rank(child.local()) {
            Some(own) => self
                .elements()
                .into_iter()
                .find(|(_, name)| rank(name).is_none_or(|other| other > own))
                .map_or(self.children.len(), |(i, _)| i),
            None => self.children.len(),
        };
        self.insert(at, child);
        at
    }
}
