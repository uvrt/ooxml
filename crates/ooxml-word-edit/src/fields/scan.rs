//! Owns: a part's fields found where they are (docx-agent's `edit/fields.py` `scan`): each
//! complex field's begin, separator and end and each simple field by their paths in the
//! part's tree, its instruction and the fields nested in it, those inside a deletion left
//! out; a field's id (`fld:<story>/<n>`, its place among its story's fields), the paragraph
//! it begins in, its cached result as text (`_result_text`), and the document's fields in
//! story order (`_all_fields`). A block's fields are scanned once while it is unchanged,
//! and a version's fields once while its parts are ([`FieldScans`], the document's); the
//! bookmarks the fields refer to, each block's markers read once while it is unchanged
//! ([`Marks`]).
//!
//! Must not: change a field or compute its value.

use crate::doc::Shared;
use crate::names::W;
use crate::WordDoc;
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_word_model::address::marks::{bookmark_markers_with, bookmark_with, BlockMarks, Bookmark, Bookmarks};
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
/// only the blocks its edits made; and the last version's fields, by its parts' roots.
pub struct FieldScans {
    blocks: Mutex<Memo<usize, Held>>,
    last: Mutex<Option<Scanned>>,
    marks: Mutex<Memo<usize, HeldMarks>>,
}

/// A memo entry of the bookmark markers: the block (held, so its address is not given to
/// another) and its markers.
#[derive(Clone)]
struct HeldMarks {
    element: Arc<Element>,
    marks: Arc<BlockMarks>,
}

impl Weigh for HeldMarks {
    fn weight(&self) -> usize {
        self.marks.weight()
    }
}

/// A version's fields, held with the story parts they were scanned from (each part's name,
/// story and root: the roots held, so their addresses are not given to others).
struct Scanned {
    parts: Vec<(String, String, Arc<Element>)>,
    fields: Arc<[Located]>,
}

impl Scanned {
    /// Whether `docx` has these parts, each with the same root.
    fn of(&self, docx: &Docx) -> bool {
        self.parts.len() == docx.parts.len()
            && self.parts.iter().zip(&docx.parts).all(|((part, story, root), now)| {
                *part == now.part && *story == now.story && Arc::ptr_eq(root, now.root())
            })
    }
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
            blocks: Mutex::new(Memo::new(Arc::clone(&budget))),
            last: Mutex::new(None),
            marks: Mutex::new(Memo::new(budget)),
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
/// reading's order; scanned once while its parts are the same.
pub(crate) fn all_fields(doc: &WordDoc, docx: &Docx) -> Arc<[Located]> {
    let scans = &doc.shared().fields;
    let mut last = scans.last.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(known) = last.as_ref().filter(|l| l.of(docx)) {
        return Arc::clone(&known.fields);
    }
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
    let fields: Arc<[Located]> = Arc::from(out);
    *last = Some(Scanned {
        parts: docx
            .parts
            .iter()
            .map(|p| (p.part.clone(), p.story.clone(), Arc::clone(p.root())))
            .collect(),
        fields: Arc::clone(&fields),
    });
    fields
}

/// How many bookmark names a pass looks up one by one before it lists every bookmark of the
/// version: a lookup alone walks the version's blocks once (their markers held while they are
/// unchanged); the list does that and copies out every bookmark, which a long document's
/// table of contents makes thousands.
const LOOKED_UP_ALONE: usize = 4;

/// A version's bookmarks as a pass asks for them by name: a few looked up alone, more from
/// one list of every bookmark.
pub(crate) enum Marks<'d> {
    /// Every bookmark, listed once.
    Listed(Bookmarks<'d>),
    /// Each name looked up alone, each block's markers from the document's memo.
    Alone { docx: &'d Docx, shared: Arc<Shared> },
}

impl<'d> Marks<'d> {
    /// The bookmarks of `docx` (a reading of `doc`) for a pass that looks up `names` names.
    pub(crate) fn of(doc: &WordDoc, docx: &'d Docx, names: usize) -> Marks<'d> {
        if names <= LOOKED_UP_ALONE {
            return Marks::Alone {
                docx,
                shared: doc.shared_handle(),
            };
        }
        let shared = doc.shared_handle();
        let mut memo = shared.fields.marks.lock().unwrap_or_else(PoisonError::into_inner);
        Marks::Listed(Bookmarks::of_markers(bookmark_markers_with(docx, &mut |block| {
            held_marks(&mut memo, block)
        })))
    }

    /// The bookmark `name` names (as [`Bookmarks::get`] finds it).
    pub(crate) fn get(&self, name: &str) -> Option<Bookmark<'d>> {
        match self {
            Marks::Listed(all) => all.get(name).ok().cloned(),
            Marks::Alone { docx, shared } => {
                let mut memo = shared.fields.marks.lock().unwrap_or_else(PoisonError::into_inner);
                bookmark_with(docx, name, &mut |block| held_marks(&mut memo, block)).ok()
            }
        }
    }
}

/// The markers of `block`, read once while it is unchanged.
fn held_marks(memo: &mut Memo<usize, HeldMarks>, block: &Arc<Element>) -> Arc<BlockMarks> {
    let key = Arc::as_ptr(block) as usize;
    if let Some(held) = memo.get(&key).filter(|h| Arc::ptr_eq(&h.element, block)) {
        return held.marks;
    }
    let marks = Arc::new(BlockMarks::of(block));
    memo.insert(
        key,
        HeldMarks {
            element: Arc::clone(block),
            marks: Arc::clone(&marks),
        },
    );
    marks
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

/// The path of the run a field's text is drawn for (`_style_reference`'s target): a simple
/// field's `w:fldSimple`, a complex one's run holding its instruction's first piece (the
/// first `w:instrText` after its begin and before its separator, else its end); `None`
/// where it has none.
pub(crate) fn instruction_run(root: &Arc<Element>, found: &Found) -> Option<Vec<u32>> {
    if found.simple {
        return Some(found.begin.clone());
    }
    let stop = found.separate.as_ref().or(found.end.as_ref())?;
    let mut first = None;
    between(
        root,
        &mut Vec::new(),
        false,
        (&found.begin, stop),
        &mut |node, path, _, _| {
            if first.is_none() && node.name().is(Some(W), "instrText") {
                first = Some(path.to_vec());
            }
        },
    );
    first.map(|path| run_of(root, &path))
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

/// Whether the result of the field `found` (at the same paths in both parts) is read from
/// the same elements of `old` and `new`: the elements `result_text` visits, below the
/// deepest element holding both its separator's and its end's runs, and that element's
/// ancestors' names (a deletion above it, a run).
pub(crate) fn same_result(old: &Arc<Element>, new: &Arc<Element>, found: &Found) -> bool {
    let (first, last) = if found.simple {
        (found.begin.clone(), found.begin.clone())
    } else {
        let (Some(separate), Some(end)) = (&found.separate, &found.end) else {
            return true;
        };
        let first = run_of(new, separate);
        let last = run_of(new, end);
        if run_of(old, separate) != first || run_of(old, end) != last {
            return false;
        }
        (first, last)
    };
    let shared = first.iter().zip(&last).take_while(|(a, b)| a == b).count();
    let depth = shared.min(first.len().min(last.len()).saturating_sub(1));
    let (mut a, mut b) = (Arc::clone(old), Arc::clone(new));
    for step in first.iter().take(depth) {
        if Arc::ptr_eq(&a, &b) {
            return true;
        }
        if a.name() != b.name() {
            return false;
        }
        let child = |e: &Arc<Element>| e.children().get(*step as usize).and_then(|n| n.as_element()).cloned();
        let (Some(x), Some(y)) = (child(&a), child(&b)) else {
            return false;
        };
        a = x;
        b = y;
    }
    if Arc::ptr_eq(&a, &b) {
        return true;
    }
    if a.name() != b.name() {
        return false;
    }
    let (Some(from), Some(to)) = (first.get(depth), last.get(depth)) else {
        return false;
    };
    (*from..=*to).all(|k| {
        let (x, y) = (a.children().get(k as usize), b.children().get(k as usize));
        match (x, y) {
            (Some(Node::Element(x)), Some(Node::Element(y))) => Arc::ptr_eq(x, y),
            // `result_text` reads elements only
            (Some(Node::Element(_)), _) | (_, Some(Node::Element(_))) => false,
            _ => true,
        }
    })
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

    #[test]
    fn a_result_read_from_the_same_elements_is_the_same_result() {
        let root = root();
        let block = |k: usize| {
            Node::Element(Arc::clone(
                root.find(Some(W), "body").unwrap().child_elements().nth(k).unwrap(),
            ))
        };
        let fields = fresh(&root);
        assert!(fields.iter().all(|f| same_result(&root, &root, f)));
        // The PAGE's paragraph (body child 2, inside the REF) replaced: the REF and the PAGE
        // are read anew; the first PAGE and the NUMPAGES in the cell are not.
        let edited = replaced(&root, 2, block(0));
        let same: Vec<bool> = fields.iter().map(|f| same_result(&root, &edited, f)).collect();
        assert_eq!(same, vec![true, false, false, true]);
        for (field, same) in fields.iter().zip(same) {
            if same {
                assert_eq!(result_text(&root, field), result_text(&edited, field));
            }
        }
    }
}
