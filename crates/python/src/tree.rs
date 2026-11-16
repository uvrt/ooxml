//! Lane J: digests of an lxml tree's parts, read from libxml2's nodes, for writing out a
//! part from its last bytes with only the changed top-level elements written again.
//!
//! An lxml element (`lxml.etree._Element`) is, in lxml's public C API (`lxml.etree.h`,
//! `struct LxmlElement`), the object header, the document and the libxml2 node; the node is
//! libxml2's public `xmlNode` (`tree.h`).  Both layouts have been the same for every lxml and
//! libxml2 version in use.  The walk only reads, under the interpreter lock (only Python code
//! holding it changes a tree), and hashes everything libxml2's serializer writes: each
//! node's type, name, namespace prefix, namespace declarations, attributes and text.
//!
//! The digests are 128 bits, two foldhash hashes seeded at random once per process: equal
//! digests are taken as equal bytes.  They are compared only within the process.
use foldhash::quality::RandomState;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::hash::BuildHasher;
use std::sync::OnceLock;

#[repr(C)]
struct XmlNs {
    next: *const XmlNs,
    kind: c_int,
    href: *const c_char,
    prefix: *const c_char,
    private: *const c_void,
    context: *const c_void,
}

#[repr(C)]
struct XmlNode {
    private: *const c_void,
    kind: c_int,
    name: *const c_char,
    children: *const XmlNode,
    last: *const XmlNode,
    parent: *const XmlNode,
    next: *const XmlNode,
    prev: *const XmlNode,
    doc: *const XmlNode,
    ns: *const XmlNs,
    content: *const c_char,
    properties: *const XmlAttr,
    ns_def: *const XmlNs,
}

#[repr(C)]
struct XmlAttr {
    private: *const c_void,
    kind: c_int,
    name: *const c_char,
    children: *const XmlNode,
    last: *const XmlNode,
    parent: *const XmlNode,
    next: *const XmlAttr,
    prev: *const XmlAttr,
    doc: *const XmlNode,
    ns: *const XmlNs,
}

const ELEMENT: c_int = 1;
const TEXT: c_int = 3;
const CDATA: c_int = 4;
/// libxml2's node of a reference (`&name;`).
const REF_NODE: c_int = 5;
const PI: c_int = 7;
const COMMENT: c_int = 8;
const DOCUMENT: c_int = 9;

/// The libxml2 node of an lxml element (`struct LxmlElement`: the header, `_doc`,
/// `_c_node`).
fn node_of(element: &Bound<'_, PyAny>) -> PyResult<*const XmlNode> {
    let module = element.py().import("lxml.etree")?;
    let kind = module.getattr("_Element")?;
    if !element.is_instance(&kind)? {
        return Err(pyo3::exceptions::PyTypeError::new_err("not an lxml element"));
    }
    let at = std::mem::size_of::<pyo3::ffi::PyObject>() + std::mem::size_of::<*const c_void>();
    // SAFETY: an instance of lxml.etree._Element is a `struct LxmlElement`, whose
    // `_c_node` follows the object header and the `_doc` pointer.
    let node = unsafe { *((element.as_ptr() as *const u8).add(at) as *const *const XmlNode) };
    if node.is_null() {
        return Err(pyo3::exceptions::PyValueError::new_err("an element without a node"));
    }
    // SAFETY: a live element's node.
    if unsafe { (*node).kind } != ELEMENT {
        return Err(pyo3::exceptions::PyValueError::new_err("not an element's node"));
    }
    Ok(node)
}

fn keys() -> &'static (RandomState, RandomState) {
    static KEYS: OnceLock<(RandomState, RandomState)> = OnceLock::new();
    KEYS.get_or_init(|| (RandomState::default(), RandomState::default()))
}

/// What a digest is made of, as a stream of bytes: libxml2's strings (never holding a NUL)
/// each ended by a NUL, a null string by a marker, kinds and marks as single bytes; hashed
/// at the end.
struct Digest {
    buf: Vec<u8>,
}

impl Digest {
    fn new() -> Self {
        Digest {
            buf: Vec::with_capacity(1 << 12),
        }
    }
    fn tag(&mut self, value: u8) {
        self.buf.push(value);
    }
    fn kind(&mut self, value: c_int) {
        self.buf.push(value as u8);
    }
    fn text(&mut self, text: *const c_char) {
        if text.is_null() {
            self.buf.push(0xFF);
        } else {
            // SAFETY: libxml2's strings are NUL-terminated.
            self.buf
                .extend_from_slice(unsafe { CStr::from_ptr(text) }.to_bytes_with_nul());
        }
    }
    fn finish(&mut self) -> [u8; 16] {
        let (a, b) = keys();
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&a.hash_one(&self.buf[..]).to_le_bytes());
        out[8..].copy_from_slice(&b.hash_one(&self.buf[..]).to_le_bytes());
        self.buf.clear();
        out
    }
}

/// One namespace (a declaration, or an element's or attribute's own).
///
/// # Safety
/// `ns` is null or a live libxml2 namespace.
unsafe fn ns(d: &mut Digest, ns: *const XmlNs) {
    if ns.is_null() {
        d.tag(0);
    } else {
        d.tag(1);
        d.text((*ns).prefix);
        d.text((*ns).href);
    }
}

/// An element's start: its name, namespace, declarations and attributes (each attribute's
/// value as its text and reference nodes).
///
/// # Safety
/// `node` is a live libxml2 element.
unsafe fn start(d: &mut Digest, node: *const XmlNode) {
    d.text((*node).name);
    ns(d, (*node).ns);
    let mut def = (*node).ns_def;
    while !def.is_null() {
        d.tag(2);
        ns(d, def);
        def = (*def).next;
    }
    d.tag(3);
    let mut attr = (*node).properties;
    while !attr.is_null() {
        d.tag(4);
        d.kind((*attr).kind);
        d.text((*attr).name);
        ns(d, (*attr).ns);
        let mut child = (*attr).children;
        while !child.is_null() {
            d.kind((*child).kind);
            d.text((*child).name);
            d.text((*child).content);
            child = (*child).next;
        }
        d.tag(5);
        attr = (*attr).next;
    }
    d.tag(6);
}

/// A node and everything under it, in document order (no recursion: the walk follows the
/// nodes' own links).
///
/// `false` where a node is of a kind the walk does not read (a document type, say).
///
/// # Safety
/// `top` is a live libxml2 node of a tree no other thread changes.
unsafe fn subtree(d: &mut Digest, top: *const XmlNode) -> bool {
    let mut node = top;
    loop {
        let kind = (*node).kind;
        d.kind(kind);
        match kind {
            ELEMENT => {
                start(d, node);
                if !(*node).children.is_null() {
                    node = (*node).children;
                    continue;
                }
                d.tag(7);
            }
            TEXT | CDATA | REF_NODE | PI | COMMENT => {
                d.text((*node).name);
                d.text((*node).content);
            }
            _ => return false,
        }
        // Up until a node has a next sibling, closing each element left.
        loop {
            if node == top {
                return true;
            }
            if !(*node).next.is_null() {
                node = (*node).next;
                break;
            }
            node = (*node).parent;
            d.tag(7);
        }
    }
}

/// Whether a child of an element is one lxml counts as a child (an element, a comment, a
/// processing instruction, a reference node), not part of a text or tail.
fn counted(kind: c_int) -> bool {
    matches!(kind, ELEMENT | COMMENT | PI | REF_NODE)
}

/// The digests of `root`'s tree around `body`, one of its children: `(skeleton, units,
/// elements)` -- `skeleton` everything but `body`'s children (the document's other nodes,
/// the root's start, its other children, `body`'s start and the text before its first
/// child), `units` one per child of `body` as lxml counts them (the child, its subtree and
/// its tail, 16 bytes each, joined), `elements` whether every such child is an element.
/// `None` where a node is of a kind the walk does not read.
#[pyfunction]
#[allow(clippy::type_complexity)]
pub fn tree_digests<'py>(
    root: &Bound<'py, PyAny>,
    body: &Bound<'py, PyAny>,
) -> PyResult<Option<(Bound<'py, PyBytes>, Bound<'py, PyBytes>, bool)>> {
    let py = root.py();
    let root = node_of(root)?;
    let body = node_of(body)?;
    // SAFETY: two live elements of one tree, held by the caller under the interpreter lock.
    unsafe {
        if (*body).parent != root {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "the body is not a child of the root",
            ));
        }
        let mut skeleton = Digest::new();
        let parent = (*root).parent;
        if !parent.is_null() && (*parent).kind == DOCUMENT {
            let mut node = (*parent).children;
            while !node.is_null() {
                if node == root {
                    skeleton.tag(8);
                } else if !subtree(&mut skeleton, node) {
                    return Ok(None);
                }
                node = (*node).next;
            }
        }
        start(&mut skeleton, root);
        let mut firsts: Vec<usize> = Vec::new();
        let mut elements = true;
        let mut child = (*root).children;
        while !child.is_null() {
            if child != body {
                if !subtree(&mut skeleton, child) {
                    return Ok(None);
                }
                child = (*child).next;
                continue;
            }
            skeleton.tag(9);
            start(&mut skeleton, body);
            let mut node = (*body).children;
            while !node.is_null() && !counted((*node).kind) {
                if !subtree(&mut skeleton, node) {
                    return Ok(None);
                }
                node = (*node).next;
            }
            while !node.is_null() {
                elements &= (*node).kind == ELEMENT;
                firsts.push(node as usize);
                node = (*node).next;
                while !node.is_null() && !counted((*node).kind) {
                    node = (*node).next;
                }
            }
            skeleton.tag(10);
            child = (*child).next;
        }
        let skeleton = skeleton.finish();
        // The children's digests side by side (each child's walk reads its own nodes only;
        // nothing changes the tree meanwhile: its document's lock is the caller's).
        let found: Vec<Option<[u8; 16]>> = py.detach(|| {
            use rayon::prelude::*;
            firsts.par_iter().map(|&first| unit(first as *const XmlNode)).collect()
        });
        let mut units = Vec::with_capacity(16 * found.len());
        for digest in found {
            match digest {
                Some(digest) => units.extend_from_slice(&digest),
                None => return Ok(None),
            }
        }
        Ok(Some((PyBytes::new(py, &skeleton), PyBytes::new(py, &units), elements)))
    }
}

/// A child's digest: the child and the nodes after it up to the next child lxml counts.
///
/// # Safety
/// `first` is a live libxml2 node of a tree no other thread changes.
unsafe fn unit(first: *const XmlNode) -> Option<[u8; 16]> {
    let mut d = Digest::new();
    if !subtree(&mut d, first) {
        return None;
    }
    let mut node = (*first).next;
    while !node.is_null() && !counted((*node).kind) {
        if !subtree(&mut d, node) {
            return None;
        }
        node = (*node).next;
    }
    Some(d.finish())
}

/// Where each child of a part's body begins in its bytes as libxml2 writes them, the body's
/// content being `data[start..end]`: a child begins at each tag that opens at the body's
/// level (its tail runs to the next).  `None` where a comment, a processing instruction or
/// a CDATA section is at that level, or the tags do not nest.
#[pyfunction]
pub fn unit_starts(py: Python<'_>, data: &[u8], start: usize, end: usize) -> Option<Vec<usize>> {
    if start > end || end > data.len() {
        return None;
    }
    py.detach(|| scan(&data[..end], start))
}

fn scan(data: &[u8], from: usize) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    let mut depth: usize = 0;
    let mut at = from;
    while let Some(k) = memchr(b'<', &data[at..]) {
        let lt = at + k;
        let rest = &data[lt + 1..];
        if rest.starts_with(b"!--") {
            if depth == 0 {
                return None;
            }
            at = lt + 4 + find(&data[lt + 4..], b"-->")? + 3;
        } else if rest.starts_with(b"![CDATA[") {
            if depth == 0 {
                return None;
            }
            at = lt + 9 + find(&data[lt + 9..], b"]]>")? + 3;
        } else if rest.starts_with(b"?") {
            if depth == 0 {
                return None;
            }
            at = lt + 2 + find(&data[lt + 2..], b"?>")? + 2;
        } else if rest.starts_with(b"/") {
            depth = depth.checked_sub(1)?;
            at = lt + 2 + memchr(b'>', &data[lt + 2..])? + 1;
        } else {
            if depth == 0 {
                out.push(lt);
            }
            let gt = lt + 1 + memchr(b'>', rest)?;
            if data[gt - 1] != b'/' {
                depth += 1;
            }
            at = gt + 1;
        }
    }
    if depth != 0 {
        return None;
    }
    Some(out)
}

fn memchr(byte: u8, data: &[u8]) -> Option<usize> {
    data.iter().position(|&b| b == byte)
}

fn find(data: &[u8], what: &[u8]) -> Option<usize> {
    data.windows(what.len()).position(|w| w == what)
}

const W_NS: &[u8] = b"http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const W14_NS: &[u8] = b"http://schemas.microsoft.com/office/word/2010/wordml";
const MC_NS: &[u8] = b"http://schemas.openxmlformats.org/markup-compatibility/2006";

/// # Safety
/// `ns` is null or a live libxml2 namespace.
unsafe fn href_is(ns: *const XmlNs, href: &[u8]) -> bool {
    !ns.is_null() && !(*ns).href.is_null() && CStr::from_ptr((*ns).href).to_bytes() == href
}

/// # Safety
/// `text` is a live NUL-terminated string.
unsafe fn named(text: *const c_char, name: &[u8]) -> bool {
    !text.is_null() && CStr::from_ptr(text).to_bytes() == name
}

/// Which of a paragraph, a row and a table an element is (1, 2, 3), else 0.
///
/// # Safety
/// `node` is a live libxml2 element.
unsafe fn walked_kind(node: *const XmlNode) -> u8 {
    if !href_is((*node).ns, W_NS) {
        return 0;
    }
    if named((*node).name, b"p") {
        1
    } else if named((*node).name, b"tr") {
        2
    } else if named((*node).name, b"tbl") {
        3
    } else {
        0
    }
}

/// One element of an index's walk: its kind, its node, its `w14:paraId`, a row's table.
/// `false` where the paraId is not plain text.
///
/// # Safety
/// `node` is a live libxml2 element of kind `kind` (not 0).
unsafe fn walked(d: &mut Digest, node: *const XmlNode, kind: u8) -> bool {
    d.tag(kind);
    d.buf.extend_from_slice(&(node as usize).to_le_bytes());
    let mut attr = (*node).properties;
    let mut found: *const c_char = std::ptr::null();
    while !attr.is_null() {
        if href_is((*attr).ns, W14_NS) && named((*attr).name, b"paraId") {
            let child = (*attr).children;
            if child.is_null() {
                found = c"".as_ptr();
            } else if !(*child).next.is_null() || (*child).kind != TEXT {
                return false;
            } else {
                found = (*child).content;
            }
            break;
        }
        attr = (*attr).next;
    }
    d.text(found);
    if kind == 2 {
        let mut up = (*node).parent;
        while !up.is_null() && !((*up).kind == ELEMENT && walked_kind(up) == 3) {
            up = (*up).parent;
        }
        d.buf.extend_from_slice(&(up as usize).to_le_bytes());
    }
    true
}

/// The walk of `top`'s subtree, `top` included where `with_top`: every paragraph, row and
/// table not under an `mc:Fallback`, in document order.  `false` as :func:`walked` says.
///
/// # Safety
/// `top` is a live libxml2 node of a tree no other thread changes.
unsafe fn walk(d: &mut Digest, top: *const XmlNode, with_top: bool) -> bool {
    let mut node = top;
    loop {
        let mut descend = false;
        if (*node).kind == ELEMENT && !(href_is((*node).ns, MC_NS) && named((*node).name, b"Fallback")) {
            if node != top || with_top {
                let kind = walked_kind(node);
                if kind != 0 && !walked(d, node, kind) {
                    return false;
                }
            }
            descend = !(*node).children.is_null();
        }
        if descend {
            node = (*node).children;
            continue;
        }
        loop {
            if node == top {
                return true;
            }
            if !(*node).next.is_null() {
                node = (*node).next;
                break;
            }
            node = (*node).parent;
        }
    }
}

/// The digest of what docx-agent's index of a story reads of its tree (lane J): every
/// paragraph, row and table under `root` (not `root` itself, none under an
/// `mc:Fallback`), in document order -- each element's node (the node itself: an index holds
/// every one of them, so none is freed while it is kept), its kind, its `w14:paraId`, and a
/// row's table (the nearest table above it).  Equal digests: the walk an index was made
/// from, unchanged (`docx_agent.edit.ids.same_walk`).  A body's children are walked side by
/// side, their records joined in order and hashed as one.  `None` where a paraId is not
/// plain text.
#[pyfunction]
pub fn index_walk<'py>(root: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyBytes>>> {
    let py = root.py();
    let top = node_of(root)?;
    let mut d = Digest::new();
    // SAFETY: a live element's tree, read under the interpreter lock (the walks side by side
    // read nodes of their own only).
    unsafe {
        if href_is((*top).ns, MC_NS) && named((*top).name, b"Fallback") {
            return Ok(Some(PyBytes::new(py, &d.finish())));
        }
        let mut child = (*top).children;
        while !child.is_null() {
            let body = (*child).kind == ELEMENT && href_is((*child).ns, W_NS) && named((*child).name, b"body");
            if !body {
                if !walk(&mut d, child, true) {
                    return Ok(None);
                }
                child = (*child).next;
                continue;
            }
            let mut firsts: Vec<usize> = Vec::new();
            let mut node = (*child).children;
            while !node.is_null() {
                firsts.push(node as usize);
                node = (*node).next;
            }
            // Each child's records walked side by side, then hashed in order as one stream:
            // the digest of the walk in document order, whatever the children holding it.
            let found: Vec<Option<Vec<u8>>> = py.detach(|| {
                use rayon::prelude::*;
                firsts
                    .par_iter()
                    .map(|&first| {
                        let mut part = Digest::new();
                        if walk(&mut part, first as *const XmlNode, true) {
                            Some(part.buf)
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            for records in found {
                match records {
                    Some(records) => d.buf.extend_from_slice(&records),
                    None => return Ok(None),
                }
            }
            child = (*child).next;
        }
    }
    Ok(Some(PyBytes::new(py, &d.finish())))
}
