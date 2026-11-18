//! Owns: the green tree (design ADR-003) -- immutable, lossless nodes shared between
//! versions: [`Tree`], [`Element`] (a [`Head`] of name, namespace declarations and
//! attributes, plus [`Children`]), and the leaf kinds of [`Node`].
//!
//! Lossless means: what libxml2 keeps of a part is kept here, in order -- names as written,
//! namespace declarations before attributes, text decoded, comments, processing
//! instructions, opaque entity references, the document type's name and external id.
//! What libxml2 drops is dropped too (CDATA sections become text; whitespace outside the
//! root; the XML declaration, which the writer always writes in one form).
//!
//! Must not: mutate a node once built (an edit makes new nodes along a path and shares the
//! rest), or know any OOXML name.

use crate::name::{Atom, Name};
use crate::rope::Children;
use std::sync::Arc;

/// A namespace declaration: `xmlns="uri"` (no prefix) or `xmlns:p="uri"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamespaceDecl {
    /// The prefix declared, or `None` for the default namespace.
    pub prefix: Option<Atom>,
    /// The URI (empty only for `xmlns=""`, which undeclares the default).
    pub uri: Atom,
}

/// An attribute: its name and its value as decoded and normalised.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Attribute {
    /// The name.
    pub name: Name,
    /// The value.
    pub value: Atom,
}

/// What an element is apart from its children. Shared between elements that are alike
/// (every `<w:r>` without attributes has one head).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Head {
    /// The element's name.
    pub name: Name,
    /// Namespace declarations, in document order.
    pub namespaces: Box<[NamespaceDecl]>,
    /// Attributes, in document order.
    pub attributes: Box<[Attribute]>,
}

/// A processing instruction: `<?target data?>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pi {
    /// The target.
    pub target: Atom,
    /// What follows the target and the whitespace after it, up to `?>` (may be empty).
    pub data: Atom,
}

/// One child of an element (or a node before or after the root).
#[derive(Debug, Clone)]
pub enum Node {
    /// An element.
    Element(Arc<Element>),
    /// Text, decoded (CDATA sections included, as libxml2 reads them with `strip_cdata`).
    Text(Atom),
    /// A comment's content.
    Comment(Atom),
    /// A processing instruction.
    Pi(Arc<Pi>),
    /// A reference to an entity the parser did not expand (only in a document whose DTD is
    /// external, as libxml2 keeps them): its name.
    EntityRef(Atom),
}

impl Node {
    /// The element, if this is one.
    pub fn as_element(&self) -> Option<&Arc<Element>> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }

    /// The text, if this is a text node.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Node::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Whether `self` is the node a patch made elsewhere means to remove: the same node
    /// ([`Node::same`]) or an equal one -- small identical subtrees share one allocation
    /// anyway, and a change made again on another version (an undo's inverse, its ids drawn
    /// the same way) builds equal ones.
    pub fn removable_as(&self, other: &Node) -> bool {
        self.same(other) || self == other
    }

    /// Whether `self` and `other` are the same node: the same element allocation, or leaves
    /// with equal content.
    pub fn same(&self, other: &Node) -> bool {
        match (self, other) {
            (Node::Element(a), Node::Element(b)) => Arc::ptr_eq(a, b),
            (Node::Pi(a), Node::Pi(b)) => Arc::ptr_eq(a, b) || a == b,
            (Node::Text(a), Node::Text(b))
            | (Node::Comment(a), Node::Comment(b))
            | (Node::EntityRef(a), Node::EntityRef(b)) => a == b,
            _ => false,
        }
    }
}

impl PartialEq for Node {
    /// Structural equality, short-circuiting on shared allocations.
    fn eq(&self, other: &Node) -> bool {
        match (self, other) {
            (Node::Element(a), Node::Element(b)) => Arc::ptr_eq(a, b) || **a == **b,
            _ => self.same(other),
        }
    }
}

impl Eq for Node {}

/// An element: a shared [`Head`] and its [`Children`].
#[derive(Debug, Clone)]
pub struct Element {
    head: Arc<Head>,
    children: Children,
}

impl PartialEq for Element {
    fn eq(&self, other: &Element) -> bool {
        (Arc::ptr_eq(&self.head, &other.head) || self.head == other.head) && self.children == other.children
    }
}

impl Eq for Element {}

impl Element {
    /// An element from its parts.
    pub fn new(name: Name, namespaces: Vec<NamespaceDecl>, attributes: Vec<Attribute>, children: Vec<Node>) -> Element {
        Element {
            head: Arc::new(Head {
                name,
                namespaces: namespaces.into(),
                attributes: attributes.into(),
            }),
            children: Children::from_vec(children),
        }
    }

    /// An element from a head (possibly shared) and children.
    pub fn from_head(head: Arc<Head>, children: Children) -> Element {
        Element { head, children }
    }

    /// The head.
    pub fn head(&self) -> &Arc<Head> {
        &self.head
    }

    /// The name.
    pub fn name(&self) -> &Name {
        &self.head.name
    }

    /// The namespace declarations.
    pub fn namespaces(&self) -> &[NamespaceDecl] {
        &self.head.namespaces
    }

    /// The attributes.
    pub fn attributes(&self) -> &[Attribute] {
        &self.head.attributes
    }

    /// The value of the attribute `{namespace}local`, if present.
    pub fn attribute(&self, namespace: Option<&str>, local: &str) -> Option<&str> {
        self.head
            .attributes
            .iter()
            .find(|a| a.name.is(namespace, local))
            .map(|a| &*a.value)
    }

    /// The children.
    pub fn children(&self) -> &Children {
        &self.children
    }

    /// The element children, in order.
    pub fn child_elements(&self) -> impl Iterator<Item = &Arc<Element>> {
        self.children.iter().filter_map(Node::as_element)
    }

    /// The first element child named `{namespace}local`.
    pub fn find(&self, namespace: Option<&str>, local: &str) -> Option<&Arc<Element>> {
        self.child_elements().find(|e| e.name().is(namespace, local))
    }

    /// This element with other children (the head shared).
    pub fn with_children(&self, children: Children) -> Element {
        Element {
            head: Arc::clone(&self.head),
            children,
        }
    }

    /// This element with another head (the children shared).
    pub fn with_head(&self, head: Head) -> Element {
        Element {
            head: Arc::new(head),
            children: self.children.clone(),
        }
    }

    /// The text of every text node below this element, in document order.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        let mut stack = vec![self.children.iter()];
        while let Some(top) = stack.last_mut() {
            match top.next() {
                Some(Node::Text(t)) => out.push_str(t),
                Some(Node::Element(e)) => stack.push(e.children.iter()),
                Some(_) => {}
                None => {
                    stack.pop();
                }
            }
        }
        out
    }
}

/// A document type declaration's external id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalId {
    /// `SYSTEM "uri"`.
    System(Atom),
    /// `PUBLIC "public id" "uri"`.
    Public(Atom, Atom),
}

/// A document type declaration, kept opaque: its name and external id. Internal subsets
/// are refused by the parser (their declarations are not reproduced in libxml2's form, and
/// entity declarations are never expanded).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Doctype {
    /// The root element name it declares.
    pub name: Atom,
    /// The external DTD's id, if any.
    pub external: Option<ExternalId>,
}

/// A parsed part: the root element, with what libxml2 keeps around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    doctype: Option<Arc<Doctype>>,
    prolog: Arc<[Node]>,
    root: Arc<Element>,
    epilog: Arc<[Node]>,
}

impl Tree {
    /// A tree from its parts. `prolog` and `epilog` hold only comments and processing
    /// instructions (the writer writes whatever they hold).
    pub fn new(doctype: Option<Doctype>, prolog: Vec<Node>, root: Arc<Element>, epilog: Vec<Node>) -> Tree {
        Tree {
            doctype: doctype.map(Arc::new),
            prolog: prolog.into(),
            root,
            epilog: epilog.into(),
        }
    }

    /// A tree of just a root element.
    pub fn from_root(root: Arc<Element>) -> Tree {
        Tree::new(None, Vec::new(), root, Vec::new())
    }

    /// The document type declaration, if any.
    pub fn doctype(&self) -> Option<&Doctype> {
        self.doctype.as_deref()
    }

    /// Comments and processing instructions before the root.
    pub fn prolog(&self) -> &[Node] {
        &self.prolog
    }

    /// The root element.
    pub fn root(&self) -> &Arc<Element> {
        &self.root
    }

    /// Comments and processing instructions after the root.
    pub fn epilog(&self) -> &[Node] {
        &self.epilog
    }

    /// This tree with another root (everything around it shared).
    pub fn with_root(&self, root: Arc<Element>) -> Tree {
        Tree {
            doctype: self.doctype.clone(),
            prolog: Arc::clone(&self.prolog),
            root,
            epilog: Arc::clone(&self.epilog),
        }
    }

    /// Whether two trees are the same version (their roots one allocation).
    pub fn ptr_eq(&self, other: &Tree) -> bool {
        Arc::ptr_eq(&self.root, &other.root)
    }
}
