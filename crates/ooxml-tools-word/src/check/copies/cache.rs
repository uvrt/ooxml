//! Owns: the comparisons a document's checks keep (docx-agent's `tools/scoped.py`
//! `COPY_CACHE`), kept by what both sides hold -- their blocks (ids and content), each
//! document's styles, numbering and theme -- and by the list items before and in each
//! side's blocks, which decide the numbers its lists show; not by the document's version,
//! so an edit elsewhere in either document finds the comparison kept. The least recently
//! used is dropped first.
//!
//! Must not: compare (`super::compare` does).

use super::compare::CopyBlock;
use ooxml_base::{Digest, DigestKey, Digester};
use ooxml_word_model::digest::feed;
use ooxml_word_model::docx::{find_path, Docx};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

/// Comparisons kept per document.
const KEPT: usize = 128;

const KEY: DigestKey = DigestKey::new(0x636f_7069_6573_2d63, 0x6f6d_7061_7265_6421);

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

/// One comparison: `{"follows", "pairs", "differences", "example_text"}`.
pub(super) type Comparison = Arc<Map<String, Value>>;

/// The comparisons a document's checks keep, by their key.
#[derive(Debug, Default)]
pub struct CopyCache {
    kept: Mutex<VecDeque<(Digest, Comparison)>>,
}

impl CopyCache {
    /// The comparison kept for `key`, made the most recently used.
    pub(super) fn get(&self, key: &Digest) -> Option<Comparison> {
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        let at = kept.iter().position(|(k, _)| k == key)?;
        let found = kept.remove(at)?;
        let value = Arc::clone(&found.1);
        kept.push_back(found);
        Some(value)
    }

    /// `found` kept for `key`, the least recently used dropped past [`KEPT`].
    pub(super) fn put(&self, key: Digest, found: Comparison) {
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.retain(|(k, _)| *k != key);
        kept.push_back((key, found));
        while kept.len() > KEPT {
            kept.pop_front();
        }
    }
}

/// What a document's comparisons are kept by besides their blocks: its styles, numbering
/// and theme, and the list skeleton of its body (each paragraph's style, its own
/// `w:numPr` and whether its mark is deleted, as prefix digests).
pub(super) struct Context {
    styles: Digest,
    skeleton: Vec<Digest>,
    upto: HashMap<Vec<u32>, usize>,
}

impl Context {
    /// `docx`'s.
    pub(super) fn of(docx: &Docx) -> Context {
        let mut d = Digester::new(KEY);
        for kind in ["styles", "numbering", "theme"] {
            let parts = ooxml_word_model::parts::related_parts(&docx.package, &docx.main, &format!("{REL}{kind}"))
                .unwrap_or_default();
            for part in parts {
                d.field(part.as_bytes());
                if let Ok(Some(tree)) = docx.package.tree(&part) {
                    feed(&mut d, tree.root());
                }
            }
        }
        let styles = d.finish();
        let mut skeleton = vec![Digester::new(KEY).finish()];
        let mut upto = HashMap::new();
        let mut running = Digester::new(KEY);
        if let Some(body) = docx.body() {
            let mut paragraphs = Vec::new();
            every_paragraph(body.root(), &mut Vec::new(), &mut paragraphs);
            for (path, paragraph) in paragraphs {
                let style = find_path(&paragraph, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
                running.field(style.unwrap_or("").as_bytes());
                match find_path(&paragraph, &["pPr", "numPr"]) {
                    Some(own) => feed(&mut running, own),
                    None => running.field(b"-"),
                }
                let deleted = find_path(&paragraph, &["pPr", "rPr", "del"]).is_some();
                running.field(if deleted { b"d" } else { b"k" });
                skeleton.push(running.clone().finish());
                upto.insert(path, skeleton.len() - 1);
            }
        }
        Context { styles, skeleton, upto }
    }

    /// The list skeleton up to the last paragraph of `blocks` (the first digest where they
    /// hold none).
    fn list_context(&self, blocks: &[CopyBlock]) -> Digest {
        let last = blocks.iter().rev().find_map(|b| {
            let mut found = Vec::new();
            every_paragraph(&b.element, &mut b.path.clone(), &mut found);
            found.pop().map(|(path, _)| path)
        });
        let at = last.and_then(|p| self.upto.get(&p).copied()).unwrap_or(0);
        self.skeleton.get(at).copied().unwrap_or_default()
    }
}

/// Every paragraph at and below `element` (at `path`), in document order.
fn every_paragraph(element: &Arc<Element>, path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, Arc<Element>)>) {
    if element.name().is(Some(W), "p") {
        out.push((path.clone(), Arc::clone(element)));
    }
    for (k, node) in element.children().iter().enumerate() {
        if let Some(child) = node.as_element() {
            path.push(u32::try_from(k).unwrap_or(u32::MAX));
            every_paragraph(child, path, out);
            path.pop();
        }
    }
}

fn blocks_digest(d: &mut Digester, blocks: &[CopyBlock]) {
    for block in blocks {
        d.field(block.id.as_bytes());
        feed(d, &block.element);
        d.field(b"\x01");
    }
}

/// One side of a comparison as its key reads it: its document's name and context, and its
/// blocks.
pub(super) struct Keyed<'k> {
    pub(super) doc: &'k str,
    pub(super) context: &'k Context,
    pub(super) blocks: &'k [CopyBlock],
}

/// The key of one copy's comparison with its reference; `runs_on` the blocks of the copy
/// before it in a stated run (`Err(first)`: none, the run's first copy or no run).
pub(super) fn key(copy: &Keyed<'_>, reference: &Keyed<'_>, runs_on: Result<&[CopyBlock], bool>) -> Digest {
    let mut d = Digester::new(KEY);
    for side in [copy, reference] {
        d.field(side.doc.as_bytes());
        blocks_digest(&mut d, side.blocks);
        d.update(&side.context.styles.0);
        d.update(&side.context.list_context(side.blocks).0);
    }
    match runs_on {
        Ok(before) => {
            d.field(b"after");
            blocks_digest(&mut d, before);
        }
        Err(first) => d.field(if first { b"first" } else { b"none" }),
    }
    d.finish()
}
