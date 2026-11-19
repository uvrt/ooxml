//! Owns: a part's fields found where they are (docx-agent's `edit/fields.py` `scan`): each
//! complex field's begin, separator and end and each simple field by their paths in the
//! part's tree, its instruction and the fields nested in it, those inside a deletion left
//! out; a field's id (`fld:<story>/<n>`, its place among its story's fields), the paragraph
//! it begins in, its cached result as text (`_result_text`), and the document's fields in
//! story order (`_all_fields`). A block's fields are scanned once while it is unchanged
//! ([`FieldScans`], the document's).
//!
//! Must not: change a field or compute its value.

use crate::names::W;
use crate::WordDoc;
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_word_model::docx::text::leading_text;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node};
use std::sync::{Arc, Mutex, PoisonError};

/// A field as the scan finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    /// Its place among its part's fields.
    pub(crate) number: usize,
    /// A `w:fldSimple`.
    pub(crate) simple: bool,
    /// Its instruction as written (`" PAGE "`).
    pub(crate) instruction: String,
    /// The path of its begin `w:fldChar`, or of its `w:fldSimple`.
    pub(crate) begin: Vec<u32>,
    /// The path of its separator.
    pub(crate) separate: Option<Vec<u32>>,
    /// The path of its end.
    pub(crate) end: Option<Vec<u32>>,
    /// The numbers of the fields nested in it.
    pub(crate) nested: Vec<usize>,
}

impl Found {
    /// Its kind: the instruction's first word, upper case (`PAGE`, `TOC`).
    pub(crate) fn keyword(&self) -> String {
        self.instruction
            .split_whitespace()
            .next()
            .map(str::to_uppercase)
            .unwrap_or_default()
    }
}

/// A field of the document: its part and story, and the field.
#[derive(Debug, Clone)]
pub(crate) struct Located {
    /// The story part.
    pub(crate) part: String,
    /// Its story's name.
    pub(crate) story: String,
    /// The field.
    pub(crate) found: Found,
}

impl Located {
    /// Its id: `fld:<story>/<n>`.
    pub(crate) fn id(&self) -> String {
        format!("fld:{}/{}", self.story, self.found.number)
    }
}

/// The fields of one block scanned on its own (no field open where it starts): its fields
/// numbered from 0, their paths below it, and those still open where it ends.
#[derive(Debug, Default)]
struct BlockFields {
    found: Vec<Found>,
    open: Vec<usize>,
}

/// A memo entry: the block (held, so its address is not given to another) and its fields.
#[derive(Clone)]
struct Held {
    element: Arc<Element>,
    fields: Arc<BlockFields>,
}

impl Weigh for Held {
    fn weight(&self) -> usize {
        64 + self
            .fields
            .found
            .iter()
            .map(|f| 128 + f.instruction.len() + 12 * (f.begin.len() + f.nested.len()))
            .sum::<usize>()
    }
}

/// The fields of one document's blocks (a part root's children, the body's), memoised by
/// the block (an element's identity: green nodes are immutable): a version's scan walks
/// only the blocks its edits made.
pub struct FieldScans {
    blocks: Mutex<Memo<usize, Held>>,
}

impl std::fmt::Debug for FieldScans {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldScans").finish_non_exhaustive()
    }
}

impl FieldScans {
    /// Scans kept within `budget`.
    #[must_use]
    pub fn new(budget: Arc<dyn CacheBudget>) -> FieldScans {
        FieldScans {
            blocks: Mutex::new(Memo::new(budget)),
        }
    }
}

/// One scan: the fields found so far, the complex fields open, the path walked.
struct Scanner<'m> {
    out: Vec<Found>,
    stack: Vec<usize>,
    path: Vec<u32>,
    memo: &'m mut Memo<usize, Held>,
}

impl Scanner<'_> {
    /// Walk the children of `element`; `blocks`: they are blocks (a root's, the body's),
    /// taken from the memo where no field is open before them.
    fn walk(&mut self, element: &Element, deleted: bool, blocks: bool) {
        for (k, node) in element.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            self.path.push(u32::try_from(k).unwrap_or(u32::MAX));
            if blocks && !deleted && self.stack.is_empty() {
                let fields = self.block(child);
                self.take(&fields);
            } else {
                self.visit(child, deleted);
                self.walk(child, inside(child, deleted), child.name().is(Some(W), "body"));
            }
            self.path.pop();
        }
    }

    /// `element`'s fields scanned on their own, from the memo when it is unchanged.
    fn block(&mut self, element: &Arc<Element>) -> Arc<BlockFields> {
        let key = Arc::as_ptr(element) as usize;
        if let Some(held) = self.memo.get(&key).filter(|h| Arc::ptr_eq(&h.element, element)) {
            return held.fields;
        }
        let mut own = Scanner {
            out: Vec::new(),
            stack: Vec::new(),
            path: Vec::new(),
            memo: &mut *self.memo,
        };
        own.visit(element, false);
        own.walk(element, inside(element, false), element.name().is(Some(W), "body"));
        let fields = Arc::new(BlockFields {
            found: own.out,
            open: own.stack,
        });
        let held = Held {
            element: Arc::clone(element),
            fields: Arc::clone(&fields),
        };
        self.memo.insert(key, held);
        fields
    }

    /// A block's fields, placed at the path walked now and numbered after those found.
    fn take(&mut self, block: &BlockFields) {
        let offset = self.out.len();
        let at = |p: &Vec<u32>| [self.path.as_slice(), p].concat();
        let placed: Vec<Found> = block
            .found
            .iter()
            .map(|f| Found {
                number: f.number + offset,
                simple: f.simple,
                instruction: f.instruction.clone(),
                begin: at(&f.begin),
                separate: f.separate.as_ref().map(at),
                end: f.end.as_ref().map(at),
                nested: f.nested.iter().map(|n| n + offset).collect(),
            })
            .collect();
        self.out.extend(placed);
        self.stack.extend(block.open.iter().map(|n| n + offset));
    }

    /// One element met in document order.
    fn visit(&mut self, node: &Element, deleted: bool) {
        if deleted {
            return;
        }
        let path = self.path.as_slice();
        let name = node.name();
        if name.is(Some(W), "fldSimple") {
            let number = self.out.len();
            self.out.push(found(
                number,
                true,
                node.attribute(Some(W), "instr").unwrap_or(""),
                path,
            ));
        } else if name.is(Some(W), "instrText") {
            if let Some(field) = self.stack.last().and_then(|k| self.out.get_mut(*k)) {
                if field.separate.is_none() {
                    field.instruction.push_str(&leading_text(node));
                }
            }
        } else if name.is(Some(W), "fldChar") {
            match node.attribute(Some(W), "fldCharType") {
                Some("begin") => {
                    let number = self.out.len();
                    if let Some(outer) = self.stack.last().and_then(|k| self.out.get_mut(*k)) {
                        outer.nested.push(number);
                    }
                    self.stack.push(number);
                    self.out.push(found(number, false, "", path));
                }
                Some("separate") => {
                    if let Some(field) = self.stack.last().and_then(|k| self.out.get_mut(*k)) {
                        field.separate = Some(path.to_vec());
                    }
                }
                Some("end") => {
                    if let Some(field) = self.stack.pop().and_then(|k| self.out.get_mut(k)) {
                        field.end = Some(path.to_vec());
                    }
                }
                _ => {}
            }
        }
    }
}

/// Whether what is below `element` is deleted (it is, or is in, a `w:del` or `w:moveFrom`).
fn inside(element: &Element, deleted: bool) -> bool {
    deleted || element.name().is(Some(W), "del") || element.name().is(Some(W), "moveFrom")
}

/// Every field of the part whose root is `root`, in document order, nested ones after
/// their outer one; the blocks unchanged since `scans` last met them not walked again.
pub(crate) fn scan(scans: &FieldScans, root: &Element) -> Vec<Found> {
    let mut memo = scans.blocks.lock().unwrap_or_else(PoisonError::into_inner);
    let mut scanner = Scanner {
        out: Vec::new(),
        stack: Vec::new(),
        path: Vec::new(),
        memo: &mut memo,
    };
    scanner.walk(root, false, true);
    scanner.out
}

fn found(number: usize, simple: bool, instruction: &str, path: &[u32]) -> Found {
    Found {
        number,
        simple,
        instruction: instruction.to_owned(),
        begin: path.to_vec(),
        separate: None,
        end: None,
        nested: Vec::new(),
    }
}

/// Every field of the document `docx` (a reading of `doc`), story by story in the
/// reading's order.
pub(crate) fn all_fields(doc: &WordDoc, docx: &Docx) -> Vec<Located> {
    let scans = &doc.shared().fields;
    let mut out = Vec::new();
    for part in &docx.parts {
        for found in scan(scans, part.root()) {
            out.push(Located {
                part: part.part.clone(),
                story: part.story.clone(),
                found,
            });
        }
    }
    out
}

/// The path of the paragraph holding the node at `path` (its nearest `w:p` above it).
pub(crate) fn paragraph_of(root: &Arc<Element>, path: &[u32]) -> Option<Vec<u32>> {
    let mut found = None;
    let mut node = root;
    for (k, step) in path.iter().enumerate() {
        node = node.children().get(*step as usize)?.as_element()?;
        if node.name().is(Some(W), "p") {
            found = Some(path.get(..=k)?.to_vec());
        }
    }
    found
}

/// The path of the run holding the node at `path` (its parent when that is a `w:r`, else
/// the node: `_run_of`).
pub(crate) fn run_of(root: &Arc<Element>, path: &[u32]) -> Vec<u32> {
    let (_, above) = path.split_last().unwrap_or((&0, &[]));
    match at_path(root, above) {
        Some(parent) if !above.is_empty() && parent.name().is(Some(W), "r") => above.to_vec(),
        _ => path.to_vec(),
    }
}

/// Whether `a` comes before `b` in document order (an ancestor before what it holds).
pub(crate) fn before(a: &[u32], b: &[u32]) -> bool {
    a < b
}

/// The cached result's text (`_result_text`): a simple field's text; a complex one's
/// between its separator and its end -- each paragraph a line, a run's tab a tab,
/// deleted text left out -- without the lines at its ends.
pub(crate) fn result_text(root: &Arc<Element>, found: &Found) -> String {
    if found.simple {
        return at_path(root, &found.begin).map_or_else(String::new, |e| texts(e));
    }
    let (Some(separate), Some(end)) = (&found.separate, &found.end) else {
        return String::new();
    };
    let (first, last) = (run_of(root, separate), run_of(root, end));
    let mut out = String::new();
    let mut path = Vec::new();
    between(
        root,
        &mut path,
        false,
        (&first, &last),
        &mut |node, _, deleted, parent_run| {
            let name = node.name();
            if name.is(Some(W), "t") && !deleted {
                out.push_str(&leading_text(node));
            } else if name.is(Some(W), "tab") && parent_run {
                out.push('\t');
            } else if name.is(Some(W), "p") {
                out.push('\n');
            }
        },
    );
    out.trim_matches('\n').to_owned()
}

/// Every `w:t`'s text below `element`, in order.
fn texts(element: &Element) -> String {
    let mut out = String::new();
    for node in element.children().iter() {
        let Node::Element(child) = node else { continue };
        if child.name().is(Some(W), "t") {
            out.push_str(&leading_text(child));
        } else {
            out.push_str(&texts(child));
        }
    }
    out
}

/// Call `f` on every element after `first` (not below it) and before `last` in document
/// order, with whether it is deleted and whether its parent is a run. The children before
/// `first`'s own line are not visited (a result is read in time of its length, not of
/// its place in the part).
fn between(
    element: &Element,
    path: &mut Vec<u32>,
    deleted: bool,
    (first, last): (&[u32], &[u32]),
    f: &mut impl FnMut(&Element, &[u32], bool, bool),
) {
    let is_run = element.name().is(Some(W), "r");
    let children = element.children();
    let start = if first.starts_with(path) {
        first.get(path.len()).map_or(0, |k| *k as usize)
    } else {
        0
    };
    for k in start..children.len() {
        let Some(Node::Element(child)) = children.get(k) else {
            continue;
        };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        let here: &[u32] = path;
        // past `last`: nothing further is wanted; `first` and below: skipped
        if !before(here, last) {
            path.pop();
            return;
        }
        let ends_before_first = before(here, first) && !first.starts_with(here);
        if !ends_before_first && !here.starts_with(first) {
            let inside = deleted || child.name().is(Some(W), "del") || child.name().is(Some(W), "moveFrom");
            if !before(here, first) {
                f(child, here, inside, is_run);
            }
            between(child, path, inside, (first, last), f);
        }
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use ooxml_base::Limits;

    struct Unlimited;

    impl CacheBudget for Unlimited {
        fn reserve(&self, _bytes: usize) -> bool {
            true
        }
        fn release(&self, _bytes: usize) {}
        fn used(&self) -> usize {
            0
        }
    }

    const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn chars(kind: &str) -> String {
        format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#)
    }

    fn instr(text: &str) -> String {
        format!(r"<w:r><w:instrText>{text}</w:instrText></w:r>")
    }

    /// A body: a simple field; a field across three paragraphs with one nested in it; a
    /// deleted field; a field in a cell.
    fn root() -> Arc<Element> {
        let (b, s, e) = (chars("begin"), chars("separate"), chars("end"));
        let body = format!(
            r#"<w:p><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p>
<w:p>{b}{} REF x {s}<w:r><w:t>a</w:t></w:r></w:p>
<w:p>{b}{}{s}<w:r><w:t>2</w:t></w:r>{e}</w:p>
<w:p><w:r><w:t>b</w:t></w:r>{e}</w:p>
<w:p><w:del>{b}{}{s}{e}</w:del></w:p>
<w:tbl><w:tr><w:tc><w:p>{b}{}{s}{e}</w:p></w:tc></w:tr></w:tbl>"#,
            instr(" REF x "),
            instr(" PAGE "),
            instr(" SEQ x "),
            instr(" NUMPAGES "),
        );
        let xml = format!(r#"<w:document xmlns:w="{W_NS}"><w:body>{body}</w:body></w:document>"#);
        let tree = ooxml_xml::parse(xml.as_bytes(), &Limits::default().xml).unwrap();
        Arc::clone(tree.root())
    }

    /// `root` with its body's child `k` replaced by `with` (the rest shared).
    fn replaced(root: &Element, k: usize, with: Node) -> Arc<Element> {
        let body = root.find(Some(W), "body").unwrap();
        let at = body
            .children()
            .iter()
            .position(|n| {
                n.as_element()
                    .is_some_and(|e| Arc::ptr_eq(e, body.child_elements().nth(k).unwrap()))
            })
            .unwrap();
        let children = body.children().splice(at..at + 1, vec![with]).unwrap();
        let body = Arc::new(body.with_children(children));
        let place = root
            .children()
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "body")))
            .unwrap();
        let outer = root
            .children()
            .splice(place..place + 1, vec![Node::Element(body)])
            .unwrap();
        Arc::new(root.with_children(outer))
    }

    fn fresh(root: &Element) -> Vec<Found> {
        scan(&FieldScans::new(Arc::new(Unlimited)), root)
    }

    #[test]
    fn a_scan_from_the_memo_is_the_scan_from_scratch() {
        let root = root();
        let scans = FieldScans::new(Arc::new(Unlimited));
        let first = scan(&scans, &root);
        assert_eq!(first, fresh(&root));
        assert_eq!(scan(&scans, &root), first);
        let words: Vec<(String, Vec<usize>)> = first
            .iter()
            .map(|f| (f.instruction.clone(), f.nested.clone()))
            .collect();
        assert_eq!(
            words,
            vec![
                (" PAGE ".to_owned(), vec![]),
                (" REF x ".to_owned(), vec![2]),
                (" PAGE ".to_owned(), vec![]),
                (" NUMPAGES ".to_owned(), vec![]),
            ]
        );
        assert_eq!(
            first[1].end.as_deref().map(|p| p[1]),
            Some(6),
            "the REF ends in the fourth paragraph (the text between blocks kept)"
        );
        // A paragraph inside the open field changed, then one outside it: each scan as from scratch.
        let block = |k: usize| {
            Node::Element(Arc::clone(
                root.find(Some(W), "body").unwrap().child_elements().nth(k).unwrap(),
            ))
        };
        let edited = replaced(&root, 2, block(0));
        assert_eq!(scan(&scans, &edited), fresh(&edited));
        let again = replaced(&edited, 0, block(4));
        assert_eq!(scan(&scans, &again), fresh(&again));
        assert_ne!(fresh(&again), first);
    }
}
