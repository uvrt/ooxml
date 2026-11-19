//! Owns: [`PartTexts`], each paragraph's text in the current view of one story part's
//! version ([`super::text::paragraph_text`], `View::Current`), in the order of the part's
//! paragraphs (`PartIds::paragraphs`), read once per version and kept with the part: a later
//! version shares each run (chunk) of paragraphs it holds as an earlier one held them, takes
//! the text of every other paragraph it shares with it (the same element, by pointer) and
//! walks only the paragraphs that changed. A find reads these
//! texts instead of walking every paragraph again (`find_text`, `replace_text`, the edit
//! core's finds).
//!
//! [`KeptTexts`] holds, for one document, each part's texts as the latest version that read
//! them left them: shared by all its versions, so a version reads from them whatever the
//! versions between it and that one read.
//!
//! Must not: keep anything a part's tree does not determine, or a text of any other view.

use crate::docx::text::{paragraph_text, View};
use crate::ids::PartIds;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// One paragraph: its element (held, so its pointer is never another's) and its text.
#[derive(Debug, Clone)]
struct Line {
    element: Arc<Element>,
    text: Arc<str>,
}

/// How many paragraphs a chunk made fresh holds at most.
const CHUNK: usize = 64;

/// A story part's paragraph texts at one version, in document order: runs of lines
/// (chunks), each shared with every version that holds the same paragraphs there, so a
/// version costs its changed chunks and a pointer per other chunk, not a line per paragraph.
#[derive(Debug, Default)]
pub struct PartTexts {
    chunks: Vec<Arc<[Line]>>,
    /// Each chunk's first paragraph's number.
    starts: Vec<usize>,
    len: usize,
}

/// The paragraphs of an earlier version, found where a later one holds them.
struct Earlier<'a> {
    texts: Option<&'a PartTexts>,
    /// Each held paragraph's number, by pointer: made on the second paragraph not found at
    /// a guess (an edit of one paragraph makes one new element, found nowhere).
    by_pointer: Option<HashMap<*const Element, usize>>,
    missed: usize,
}

impl Earlier<'_> {
    /// The number of the earlier paragraph that is `element`, tried at `guesses` first.
    fn find(&mut self, element: &Arc<Element>, guesses: &[Option<usize>]) -> Option<usize> {
        let texts = self.texts?;
        let same = |n: usize| texts.line(n).is_some_and(|line| Arc::ptr_eq(&line.element, element));
        if let Some(n) = guesses.iter().flatten().copied().find(|n| same(*n)) {
            return Some(n);
        }
        self.missed += 1;
        if self.missed < 2 {
            return None;
        }
        let map = self.by_pointer.get_or_insert_with(|| {
            texts
                .lines()
                .enumerate()
                .map(|(n, line)| (Arc::as_ptr(&line.element), n))
                .collect()
        });
        map.get(&Arc::as_ptr(element)).copied()
    }
}

impl PartTexts {
    /// The texts of the paragraphs `ids` lists; `earlier`, the texts of an earlier version
    /// of the part: a chunk of its paragraphs this version holds in the same order is shared
    /// as it is, a paragraph it holds elsewhere has its text taken, and only the paragraphs
    /// it does not hold are walked.
    #[must_use]
    pub fn read(ids: &PartIds, earlier: Option<&PartTexts>) -> PartTexts {
        let elements: Vec<&Arc<Element>> = ids.paragraph_elements().collect();
        let mut old = Earlier {
            texts: earlier,
            by_pointer: None,
            missed: 0,
        };
        let before = earlier.map_or(0, |e| e.len);
        let shift = before.abs_diff(elements.len());
        let grown = elements.len() >= before;
        let mut out = PartTexts::default();
        let mut fresh: Vec<Line> = Vec::new();
        // the earlier number of the next paragraph, as the last one found says
        let mut next: Option<usize> = None;
        let mut k = 0;
        while let Some(&element) = elements.get(k) {
            let shifted = if grown {
                k.checked_sub(shift)
            } else {
                k.checked_add(shift)
            };
            let at = old.find(element, &[next, Some(k), shifted]);
            let shared = at.and_then(|n| Some((n, earlier?.shared_from(n, elements.get(k..)?)?)));
            if let Some((n, chunk)) = shared {
                out.push_fresh(&mut fresh);
                k += chunk.len();
                next = Some(n + chunk.len());
                out.push(chunk);
                continue;
            }
            let text = at.and_then(|n| earlier?.line(n)).map_or_else(
                || Arc::from(paragraph_text(element, View::Current)),
                |line| Arc::clone(&line.text),
            );
            fresh.push(Line {
                element: Arc::clone(element),
                text,
            });
            if fresh.len() == CHUNK {
                out.push_fresh(&mut fresh);
            }
            next = at.map(|n| n + 1);
            k += 1;
        }
        out.push_fresh(&mut fresh);
        out
    }

    /// The chunk that starts at paragraph `n`, when `elements` begin with its paragraphs.
    fn shared_from(&self, n: usize, elements: &[&Arc<Element>]) -> Option<Arc<[Line]>> {
        let c = self.starts.binary_search(&n).ok()?;
        let chunk = self.chunks.get(c)?;
        let held = chunk.len() <= elements.len()
            && chunk
                .iter()
                .zip(elements)
                .all(|(line, element)| Arc::ptr_eq(&line.element, element));
        held.then(|| Arc::clone(chunk))
    }

    fn push(&mut self, chunk: Arc<[Line]>) {
        self.starts.push(self.len);
        self.len += chunk.len();
        self.chunks.push(chunk);
    }

    fn push_fresh(&mut self, fresh: &mut Vec<Line>) {
        if !fresh.is_empty() {
            self.push(Arc::from(std::mem::take(fresh)));
        }
    }

    fn line(&self, k: usize) -> Option<&Line> {
        let c = self.starts.partition_point(|s| *s <= k).checked_sub(1)?;
        self.chunks.get(c)?.get(k - self.starts.get(c)?)
    }

    fn lines(&self) -> impl Iterator<Item = &Line> {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }

    /// The text of the `k`-th paragraph (as `PartIds::paragraphs` numbers them).
    #[must_use]
    pub fn get(&self, k: usize) -> Option<&str> {
        self.line(k).map(|line| &*line.text)
    }

    /// How many paragraphs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// No paragraphs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every paragraph's text, in document order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.lines().map(|line| &*line.text)
    }
}

/// One document's latest paragraph texts per story part, shared by its versions.
#[derive(Debug, Default)]
pub struct KeptTexts {
    parts: Mutex<HashMap<String, Arc<PartTexts>>>,
}

impl KeptTexts {
    /// The texts of `part` that a version of it read last.
    #[must_use]
    pub fn latest(&self, part: &str) -> Option<Arc<PartTexts>> {
        self.parts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(part)
            .cloned()
    }

    /// Keep `texts` as `part`'s latest.
    pub fn keep(&self, part: &str, texts: &Arc<PartTexts>) {
        self.parts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(part.to_owned(), Arc::clone(texts));
    }
}
