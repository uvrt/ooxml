//! Owns: [`Work`], an editable copy of one element's subtree (a paragraph) in the shape the
//! edit rules are written in -- elements with their text and each node's tail, a parent
//! for every node, siblings by place -- and its conversion back to green nodes, every
//! subtree the edit did not change shared with the version it came from.
//!
//! An edit reads the version's immutable tree, copies the one block it changes into a
//! `Work`, applies its rules there, and hands the result to the sink as one patch: the
//! version's path copy (design section 4, "Edits are path copies").
//!
//! Must not: outlive its edit, reach outside the subtree it was made from (the namespace
//! declarations in scope above it are given, read only), or know any Word rule.

use ooxml_xml::{Atom, Attribute, Element, Name, NamespaceDecl, Node, Pi, QName};
use std::sync::Arc;

/// A node of a [`Work`] tree.
pub(crate) type Id = usize;

/// What a node is.
#[derive(Debug, Clone)]
enum Kind {
    Element {
        name: Name,
        namespaces: Vec<NamespaceDecl>,
        attributes: Vec<Attribute>,
        /// The text before the first child.
        text: String,
    },
    Comment(Atom),
    Pi(Arc<Pi>),
    Entity(Atom),
}

#[derive(Debug, Clone)]
struct Slot {
    kind: Kind,
    parent: Option<Id>,
    children: Vec<Id>,
    /// The text after this node, up to its next sibling (part of its parent's content).
    tail: String,
    /// The green node this one was read from, while nothing below it changed.
    origin: Option<Node>,
    changed: bool,
    /// A key an edit gives the node to know it again after splits (copies carry it); not
    /// part of the XML (docx-agent's temporary tracking attribute).
    key: Option<u32>,
}

/// An editable subtree.
#[derive(Debug, Clone)]
pub(crate) struct Work {
    slots: Vec<Slot>,
    root: Id,
    /// Namespace declarations in scope above the root, outermost first.
    scope: Vec<NamespaceDecl>,
}

impl Work {
    /// An editable copy of `element`, under the declarations `scope` (outermost first).
    pub(crate) fn of(element: &Arc<Element>, scope: Vec<NamespaceDecl>) -> Work {
        let mut work = Work {
            slots: Vec::new(),
            root: 0,
            scope,
        };
        work.root = work.read(&Node::Element(Arc::clone(element)), None).unwrap_or(0);
        work
    }

    /// Read `node` and what is below it; `None` for text, which is its element's text or
    /// a node's tail, never a node.
    fn read(&mut self, node: &Node, parent: Option<Id>) -> Option<Id> {
        let kind = match node {
            Node::Element(e) => Kind::Element {
                name: e.name().clone(),
                namespaces: e.namespaces().to_vec(),
                attributes: e.attributes().to_vec(),
                text: String::new(),
            },
            Node::Comment(c) => Kind::Comment(Arc::clone(c)),
            Node::Pi(p) => Kind::Pi(Arc::clone(p)),
            Node::EntityRef(r) => Kind::Entity(Arc::clone(r)),
            Node::Text(_) => return None,
        };
        let id = self.push(kind, Some(node.clone()));
        if let Some(slot) = self.slots.get_mut(id) {
            slot.parent = parent;
        }
        if let Node::Element(e) = node {
            let mut last: Option<Id> = None;
            for child in e.children().iter() {
                match child {
                    Node::Text(t) => {
                        let into = match last {
                            Some(previous) => self.tail_mut(previous),
                            None => self.text_mut(id),
                        };
                        if let Some(into) = into {
                            into.push_str(t);
                        }
                    }
                    other => {
                        if let Some(c) = self.read(other, Some(id)) {
                            if let Some(slot) = self.slots.get_mut(id) {
                                slot.children.push(c);
                            }
                            last = Some(c);
                        }
                    }
                }
            }
        }
        Some(id)
    }

    fn push(&mut self, kind: Kind, origin: Option<Node>) -> Id {
        let changed = origin.is_none();
        self.slots.push(Slot {
            kind,
            parent: None,
            children: Vec::new(),
            tail: String::new(),
            origin,
            changed,
            key: None,
        });
        self.slots.len() - 1
    }

    // -- reading -------------------------------------------------------------------------

    /// The key the node was given ([`Work::set_key`]), carried by its copies.
    pub(crate) fn key(&self, id: Id) -> Option<u32> {
        self.slot(id).and_then(|s| s.key)
    }

    /// Give the node a key (or take it away), which its copies carry; the XML is unchanged.
    pub(crate) fn set_key(&mut self, id: Id, key: Option<u32>) {
        if let Some(slot) = self.slots.get_mut(id) {
            slot.key = key;
        }
    }

    /// The root.
    pub(crate) fn root(&self) -> Id {
        self.root
    }

    fn slot(&self, id: Id) -> Option<&Slot> {
        self.slots.get(id)
    }

    /// The element's name, `None` for another kind of node.
    pub(crate) fn name(&self, id: Id) -> Option<&Name> {
        match &self.slot(id)?.kind {
            Kind::Element { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Whether the node is the element `{namespace}local`.
    pub(crate) fn is(&self, id: Id, namespace: &str, local: &str) -> bool {
        self.name(id).is_some_and(|n| n.is(Some(namespace), local))
    }

    /// Whether the node is an element in `namespace` with one of `locals`.
    pub(crate) fn is_any(&self, id: Id, namespace: &str, locals: &[&str]) -> bool {
        self.name(id)
            .is_some_and(|n| n.namespace() == Some(namespace) && locals.contains(&n.local()))
    }

    /// Whether the node is an element.
    pub(crate) fn is_element(&self, id: Id) -> bool {
        self.name(id).is_some()
    }

    /// The parent.
    pub(crate) fn parent(&self, id: Id) -> Option<Id> {
        self.slot(id)?.parent
    }

    /// The children (every kind of node but text), in order.
    pub(crate) fn children(&self, id: Id) -> &[Id] {
        self.slot(id).map_or(&[], |s| s.children.as_slice())
    }

    /// The first child element named `{namespace}local`.
    pub(crate) fn find(&self, id: Id, namespace: &str, local: &str) -> Option<Id> {
        self.children(id)
            .iter()
            .copied()
            .find(|c| self.is(*c, namespace, local))
    }

    /// The node's place among its parent's children.
    pub(crate) fn index(&self, id: Id) -> Option<usize> {
        let parent = self.parent(id)?;
        self.children(parent).iter().position(|c| *c == id)
    }

    /// The sibling before.
    pub(crate) fn previous(&self, id: Id) -> Option<Id> {
        let parent = self.parent(id)?;
        let at = self.index(id)?;
        self.children(parent).get(at.checked_sub(1)?).copied()
    }

    /// The sibling after.
    pub(crate) fn next(&self, id: Id) -> Option<Id> {
        let parent = self.parent(id)?;
        let at = self.index(id)?;
        self.children(parent).get(at + 1).copied()
    }

    /// The element's text before its first child.
    pub(crate) fn text(&self, id: Id) -> &str {
        match self.slot(id).map(|s| &s.kind) {
            Some(Kind::Element { text, .. }) => text,
            _ => "",
        }
    }

    /// The text after the node.
    pub(crate) fn tail(&self, id: Id) -> &str {
        self.slot(id).map_or("", |s| s.tail.as_str())
    }

    /// The value of the attribute `{namespace}local` (`namespace` `None`: no namespace).
    pub(crate) fn get(&self, id: Id, namespace: Option<&str>, local: &str) -> Option<&str> {
        match &self.slot(id)?.kind {
            Kind::Element { attributes, .. } => attributes
                .iter()
                .find(|a| a.name.is(namespace, local))
                .map(|a| &*a.value),
            _ => None,
        }
    }

    /// The attributes, in order.
    pub(crate) fn attributes(&self, id: Id) -> &[Attribute] {
        match self.slot(id).map(|s| &s.kind) {
            Some(Kind::Element { attributes, .. }) => attributes,
            _ => &[],
        }
    }

    /// The node and every node below it, in document order.
    pub(crate) fn descendants(&self, id: Id) -> Vec<Id> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            out.push(at);
            stack.extend(self.children(at).iter().rev());
        }
        out
    }

    /// The elements named `{namespace}local` (one of `locals`) at or below `id`, in
    /// document order (lxml's `iter(tag, ...)`).
    pub(crate) fn iter(&self, id: Id, namespace: &str, locals: &[&str]) -> Vec<Id> {
        self.descendants(id)
            .into_iter()
            .filter(|n| self.is_any(*n, namespace, locals))
            .collect()
    }

    /// The declaration a name in `namespace` is written with at `id`: the nearest one
    /// (lxml's search from the node up); a default declaration only where `default` allows
    /// it (an attribute's name takes a prefix). `Some(None)`: the default namespace.
    fn declared(&self, id: Id, namespace: &str, default: bool) -> Option<Option<String>> {
        let fits = |d: &&NamespaceDecl| &*d.uri == namespace && (default || d.prefix.is_some());
        let mut at = Some(id);
        while let Some(node) = at {
            if let Some(Kind::Element { namespaces, .. }) = self.slot(node).map(|s| &s.kind) {
                if let Some(d) = namespaces.iter().find(fits) {
                    return Some(d.prefix.as_deref().map(str::to_owned));
                }
            }
            at = self.parent(node);
        }
        self.scope
            .iter()
            .rev()
            .find(fits)
            .map(|d| d.prefix.as_deref().map(str::to_owned))
    }

    /// An attribute's name in `namespace`, with the prefix declared for it at the root
    /// (`fallback` where nothing declares it: the declaration is then added at the root).
    pub(crate) fn qname(&self, namespace: &str, local: &str, fallback: &str) -> Name {
        let prefix = self.declared(self.root, namespace, false).flatten();
        let prefix = prefix.unwrap_or_else(|| fallback.to_owned());
        Arc::new(QName::new(Some(&prefix), local, Some(namespace)))
    }

    /// An element's name in `namespace` as an element made under `parent` is written: the
    /// nearest declaration's prefix, none for the default namespace.
    pub(crate) fn element_name(&self, parent: Id, namespace: &str, local: &str, fallback: &str) -> Name {
        let prefix = match self.declared(parent, namespace, true) {
            Some(prefix) => prefix,
            None => Some(fallback.to_owned()),
        };
        Arc::new(QName::new(prefix.as_deref(), local, Some(namespace)))
    }

    // -- changing ------------------------------------------------------------------------

    /// Mark `id` and everything above it as changed.
    fn touch(&mut self, id: Id) {
        let mut at = Some(id);
        while let Some(node) = at {
            let Some(slot) = self.slots.get_mut(node) else { return };
            slot.changed = true;
            at = slot.parent;
        }
    }

    fn text_mut(&mut self, id: Id) -> Option<&mut String> {
        self.slots.get_mut(id).and_then(|s| match &mut s.kind {
            Kind::Element { text, .. } => Some(text),
            _ => None,
        })
    }

    fn tail_mut(&mut self, id: Id) -> Option<&mut String> {
        self.slots.get_mut(id).map(|s| &mut s.tail)
    }

    /// A new element, detached.
    pub(crate) fn new_element(&mut self, name: Name) -> Id {
        self.push(
            Kind::Element {
                name,
                namespaces: Vec::new(),
                attributes: Vec::new(),
                text: String::new(),
            },
            None,
        )
    }

    /// Set the element's text.
    pub(crate) fn set_text(&mut self, id: Id, value: &str) {
        if let Some(text) = self.text_mut(id) {
            value.clone_into(text);
        }
        self.touch(id);
    }

    /// Set the node's tail.
    pub(crate) fn set_tail(&mut self, id: Id, value: &str) {
        if let Some(tail) = self.tail_mut(id) {
            value.clone_into(tail);
        }
        if let Some(parent) = self.parent(id) {
            self.touch(parent);
        }
    }

    /// Rename the element (lxml's `element.tag = ...`).
    pub(crate) fn rename(&mut self, id: Id, to: Name) {
        if let Some(Kind::Element { name, .. }) = self.slots.get_mut(id).map(|s| &mut s.kind) {
            *name = to;
        }
        self.touch(id);
    }

    /// Set an attribute: in its place when the element has it, else after the others.
    pub(crate) fn set(&mut self, id: Id, name: Name, value: &str) {
        if let Some(Kind::Element { attributes, .. }) = self.slots.get_mut(id).map(|s| &mut s.kind) {
            let value: Atom = Atom::from(value);
            match attributes.iter_mut().find(|a| same_attr(&a.name, &name)) {
                Some(a) => a.value = value,
                None => attributes.push(Attribute { name, value }),
            }
        }
        self.touch(id);
    }

    /// Remove the attribute `{namespace}local`, if the element has it.
    pub(crate) fn remove_attr(&mut self, id: Id, namespace: Option<&str>, local: &str) {
        if let Some(Kind::Element { attributes, .. }) = self.slots.get_mut(id).map(|s| &mut s.kind) {
            attributes.retain(|a| !a.name.is(namespace, local));
        }
        self.touch(id);
    }

    /// Detach `id` from its parent, its tail going with it (lxml's `parent.remove`, which
    /// the callers below wrap to keep the tail where it was).
    fn unlink(&mut self, id: Id) {
        let Some(parent) = self.parent(id) else { return };
        if let Some(slot) = self.slots.get_mut(parent) {
            slot.children.retain(|c| *c != id);
        }
        if let Some(slot) = self.slots.get_mut(id) {
            slot.parent = None;
        }
        self.touch(parent);
    }

    /// Remove `id`, its tail kept where it was: added to the previous sibling's tail, or
    /// to the parent's text (ooxml-edit's `remove`).
    pub(crate) fn remove(&mut self, id: Id) {
        let Some(parent) = self.parent(id) else { return };
        let tail = self.tail(id).to_owned();
        if !tail.is_empty() {
            let into = match self.previous(id) {
                Some(previous) => self.tail_mut(previous),
                None => self.text_mut(parent),
            };
            if let Some(into) = into {
                into.push_str(&tail);
            }
            if let Some(own) = self.tail_mut(id) {
                own.clear();
            }
        }
        self.unlink(id);
    }

    /// Put `node` (moved from wherever it is, with its tail) as `parent`'s child at `index`.
    pub(crate) fn insert(&mut self, parent: Id, index: usize, node: Id) {
        self.unlink(node);
        if let Some(slot) = self.slots.get_mut(parent) {
            let at = index.min(slot.children.len());
            slot.children.insert(at, node);
        }
        if let Some(slot) = self.slots.get_mut(node) {
            slot.parent = Some(parent);
        }
        self.touch(parent);
        self.strip_redundant(node);
    }

    /// The prefix of the nearest declaration of `uri` from `id` up (through the scope above
    /// the root when `id` is attached), `Some(None)` for a default declaration.
    fn in_scope(&self, id: Id, uri: &str) -> Option<Option<Atom>> {
        let mut at = Some(id);
        let mut last = id;
        while let Some(node) = at {
            if let Some(Kind::Element { namespaces, .. }) = self.slot(node).map(|s| &s.kind) {
                if let Some(d) = namespaces.iter().find(|d| &*d.uri == uri) {
                    return Some(d.prefix.clone());
                }
            }
            last = node;
            at = self.parent(node);
        }
        if last != self.root {
            return None; // a detached subtree: nothing above it
        }
        self.scope
            .iter()
            .rev()
            .find(|d| &*d.uri == uri)
            .map(|d| d.prefix.clone())
    }

    /// lxml's handling of a moved subtree (`moveNodeToDocument`): a namespace declaration
    /// in it whose namespace is declared already where it now is goes, the names that used
    /// it taking the declaration in scope.
    fn strip_redundant(&mut self, node: Id) {
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            stack.extend(self.children(id).iter().rev());
            let Some(parent) = self.parent(id) else { continue };
            let declared = match self.slot(id).map(|s| &s.kind) {
                Some(Kind::Element { namespaces, .. }) if !namespaces.is_empty() => namespaces.clone(),
                _ => continue,
            };
            let mut keep = Vec::new();
            let mut remap = Vec::new();
            for d in declared.iter() {
                match self.in_scope(parent, &d.uri) {
                    Some(prefix) if prefix.is_some() || d.prefix.is_none() => {
                        if prefix != d.prefix {
                            remap.push((d.prefix.clone(), prefix, d.uri.clone()));
                        }
                    }
                    _ => keep.push(d.clone()),
                }
            }
            if keep.len() == declared.len() {
                continue;
            }
            if let Some(Kind::Element { namespaces, .. }) = self.slots.get_mut(id).map(|s| &mut s.kind) {
                *namespaces = keep;
            }
            self.touch(id);
            for (from, to, uri) in remap {
                self.reprefix(id, from.as_deref(), to.as_deref(), &uri);
            }
        }
    }

    /// Names in `uri` written with prefix `from` at or below `id`, written with `to`.
    fn reprefix(&mut self, id: Id, from: Option<&str>, to: Option<&str>, uri: &str) {
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            stack.extend(self.children(at).iter());
            let Some(Kind::Element { name, attributes, .. }) = self.slots.get_mut(at).map(|s| &mut s.kind) else {
                continue;
            };
            let renamed = |n: &Name| -> Option<Name> {
                (n.namespace() == Some(uri) && n.prefix() == from)
                    .then(|| Arc::new(QName::new(to, n.local(), Some(uri))))
            };
            if let Some(new) = renamed(name) {
                *name = new;
            }
            for a in attributes.iter_mut() {
                if let Some(new) = renamed(&a.name).filter(|_| to.is_some()) {
                    a.name = new;
                }
            }
            self.touch(at);
        }
    }

    /// Put `node` last among `parent`'s children.
    pub(crate) fn append(&mut self, parent: Id, node: Id) {
        self.insert(parent, usize::MAX, node);
    }

    /// Put `node` right before `anchor` (lxml's `addprevious`).
    pub(crate) fn add_previous(&mut self, anchor: Id, node: Id) {
        self.unlink(node);
        if let (Some(parent), Some(at)) = (self.parent(anchor), self.index(anchor)) {
            self.insert(parent, at, node);
        }
    }

    /// Put `node` right after `anchor` (lxml's `addnext`).
    pub(crate) fn add_next(&mut self, anchor: Id, node: Id) {
        self.unlink(node);
        if let (Some(parent), Some(at)) = (self.parent(anchor), self.index(anchor)) {
            self.insert(parent, at + 1, node);
        }
    }

    /// A detached copy of `id` and everything below it, its tail included (lxml's
    /// `copy.deepcopy`). A copy of a subtree that did not change shares its green nodes.
    pub(crate) fn deep_copy(&mut self, id: Id) -> Id {
        let copy = self.copy_subtree(id);
        // lxml's copy is a document of its own: a namespace its names use that nothing in
        // it declares is declared on the copy (where it is inserted, a declaration its new
        // place has already goes)
        let mut declared: Vec<Atom> = Vec::new();
        let mut used: Vec<(Option<String>, Atom)> = Vec::new();
        for node in self.descendants(copy) {
            if let Some(Kind::Element {
                name,
                namespaces,
                attributes,
                ..
            }) = self.slot(node).map(|s| &s.kind)
            {
                declared.extend(namespaces.iter().map(|d| Atom::clone(&d.uri)));
                for n in std::iter::once(name).chain(attributes.iter().map(|a| &a.name)) {
                    if let Some(uri) = n.namespace().filter(|u| *u != crate::names::XML) {
                        let prefix = n.prefix().map(str::to_owned);
                        if !used.iter().any(|(p, u)| *p == prefix && &**u == uri) {
                            used.push((prefix, Atom::from(uri)));
                        }
                    }
                }
            }
        }
        let missing: Vec<NamespaceDecl> = used
            .into_iter()
            .filter(|(_, uri)| !declared.contains(uri))
            .map(|(prefix, uri)| NamespaceDecl {
                prefix: prefix.map(|p| Atom::from(p.as_str())),
                uri,
            })
            .collect();
        if !missing.is_empty() {
            if let Some(Kind::Element { namespaces, .. }) = self.slots.get_mut(copy).map(|s| &mut s.kind) {
                for d in missing {
                    if !namespaces.iter().any(|n| n.prefix == d.prefix) {
                        namespaces.push(d);
                    }
                }
            }
            self.touch(copy);
        }
        copy
    }

    /// A detached copy of `id` and everything below it, its tail included.
    fn copy_subtree(&mut self, id: Id) -> Id {
        let Some(slot) = self.slot(id).cloned() else { return id };
        let copy = self.push(slot.kind.clone(), if slot.changed { None } else { slot.origin.clone() });
        if let Some(s) = self.slots.get_mut(copy) {
            s.tail = slot.tail.clone();
            s.changed = slot.changed;
            s.key = slot.key;
        }
        for child in slot.children {
            let c = self.copy_subtree(child);
            if let Some(s) = self.slots.get_mut(c) {
                s.parent = Some(copy);
            }
            if let Some(s) = self.slots.get_mut(copy) {
                s.children.push(c);
            }
        }
        copy
    }

    /// A detached shallow copy of the element: its name and attributes, no children
    /// (lxml's `makeelement(tag, attrib)`).
    pub(crate) fn shallow_copy(&mut self, id: Id) -> Id {
        let (name, attributes) = match self.slot(id).map(|s| &s.kind) {
            Some(Kind::Element { name, attributes, .. }) => (name.clone(), attributes.clone()),
            _ => return id,
        };
        let key = self.key(id);
        let copy = self.new_element(name);
        if let Some(slot) = self.slots.get_mut(copy) {
            slot.key = key;
            if let Kind::Element { attributes: a, .. } = &mut slot.kind {
                *a = attributes;
            }
        }
        copy
    }

    // -- back to green -------------------------------------------------------------------

    /// Whether anything at or below the root changed.
    pub(crate) fn changed(&self) -> bool {
        self.slot(self.root).is_some_and(|s| s.changed)
    }

    /// The root as a green element: unchanged subtrees are the version's own nodes.
    pub(crate) fn build(&self) -> Option<Arc<Element>> {
        match self.build_node(self.root) {
            Some(Node::Element(e)) => Some(e),
            _ => None,
        }
    }

    /// The element's content as green nodes (its text, its children, their tails): what
    /// replaces a stretch of a parent's children when the root stands for that stretch.
    pub(crate) fn build_content(&self, id: Id) -> Option<Vec<Node>> {
        let slot = self.slot(id)?;
        let mut out = Vec::new();
        if !self.text(id).is_empty() {
            out.push(Node::Text(Atom::from(self.text(id))));
        }
        for child in &slot.children {
            out.push(self.build_node(*child)?);
            let tail = self.tail(*child);
            if !tail.is_empty() {
                out.push(Node::Text(Atom::from(tail)));
            }
        }
        Some(out)
    }

    /// Whether two elements are the same XML as canonical serialisation compares them
    /// (lxml's c14n): names and attributes by namespace (attributes in any order), text,
    /// and children in order with their tails; prefixes do not matter.
    pub(crate) fn same_xml(&self, a: Id, b: Id) -> bool {
        let (Some(x), Some(y)) = (self.slot(a), self.slot(b)) else {
            return false;
        };
        match (&x.kind, &y.kind) {
            (
                Kind::Element {
                    name: n1,
                    attributes: a1,
                    text: t1,
                    ..
                },
                Kind::Element {
                    name: n2,
                    attributes: a2,
                    text: t2,
                    ..
                },
            ) => {
                let key = |attrs: &[Attribute]| {
                    let mut v: Vec<(Option<String>, String, String)> = attrs
                        .iter()
                        .map(|t| {
                            (
                                t.name.namespace().map(str::to_owned),
                                t.name.local().to_owned(),
                                t.value.to_string(),
                            )
                        })
                        .collect();
                    v.sort();
                    v
                };
                n1.namespace() == n2.namespace()
                    && n1.local() == n2.local()
                    && t1 == t2
                    && key(a1) == key(a2)
                    && x.children.len() == y.children.len()
                    && x.children
                        .iter()
                        .zip(&y.children)
                        .all(|(c, d)| self.tail(*c) == self.tail(*d) && self.same_xml(*c, *d))
            }
            (Kind::Comment(c), Kind::Comment(d)) | (Kind::Entity(c), Kind::Entity(d)) => c == d,
            (Kind::Pi(c), Kind::Pi(d)) => c == d,
            _ => false,
        }
    }

    fn build_node(&self, id: Id) -> Option<Node> {
        let slot = self.slot(id)?;
        if !slot.changed {
            if let Some(origin) = &slot.origin {
                return Some(origin.clone());
            }
        }
        Some(match &slot.kind {
            Kind::Element {
                name,
                namespaces,
                attributes,
                text,
            } => {
                let mut children = Vec::with_capacity(slot.children.len() * 2 + 1);
                if !text.is_empty() {
                    children.push(Node::Text(Atom::from(text.as_str())));
                }
                for child in &slot.children {
                    children.push(self.build_node(*child)?);
                    let tail = self.tail(*child);
                    if !tail.is_empty() {
                        children.push(Node::Text(Atom::from(tail)));
                    }
                }
                Node::Element(Arc::new(Element::new(
                    name.clone(),
                    namespaces.clone(),
                    attributes.clone(),
                    children,
                )))
            }
            Kind::Comment(c) => Node::Comment(Arc::clone(c)),
            Kind::Pi(p) => Node::Pi(Arc::clone(p)),
            Kind::Entity(r) => Node::EntityRef(Arc::clone(r)),
        })
    }
}

/// The namespace declarations in scope above the element at `path` of `root` (child
/// indexes from the root), outermost first: what [`Work::of`] takes for that element.
pub(crate) fn scope_above(root: &Element, path: &[u32]) -> Vec<NamespaceDecl> {
    let mut out = root.namespaces().to_vec();
    let mut at = root;
    for k in path.iter().take(path.len().saturating_sub(1)) {
        let Some(child) = at.children().get(*k as usize).and_then(Node::as_element) else {
            break;
        };
        out.extend_from_slice(child.namespaces());
        at = child;
    }
    out
}

/// Two attribute names for the same attribute: the same namespace and local name.
fn same_attr(a: &QName, b: &QName) -> bool {
    a.namespace() == b.namespace() && a.local() == b.local()
}
