//! Owns: [`Judgements`], each paragraph's judgement for `text_language` (its runs' tag and
//! the language detected in its text, or why it is not judged) kept across a document's
//! checks -- the block tier of the caching policy (ADR-008): by the paragraph's element,
//! the `w:tblPr` of the table it is in and the styles part's root, each by identity (a
//! version that shares them judges the paragraph as before). A check after a local edit
//! judges only the paragraphs the edit made anew. They are held within a budget (the
//! document's account in a session; a fixed bound otherwise).
//!
//! Must not: keep an element alive (they are held weakly; a judgement no version can ask
//! for is swept), or keep anything a paragraph's judgement does not read.

use super::{Judged, Unjudged};
use ooxml_derive::{CacheBudget, Memo, MemoStats, Weigh};
use ooxml_xml::Element;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// A paragraph's judgement, without its id (its version's to name).
#[derive(Debug, Clone)]
pub(super) enum Judgement {
    /// Judged: its tag, the tag's source, its language, the detection's confidence, its words.
    Judged {
        tag: Option<String>,
        source: Option<String>,
        language: usize,
        confidence: f64,
        words: usize,
    },
    /// Too few words.
    TooShort,
    /// No language clear enough.
    Unclear,
    /// No text.
    NoText,
}

impl Judgement {
    /// `found` without its id.
    pub(super) fn of(found: &Result<Judged, Unjudged>) -> Judgement {
        match found {
            Ok(j) => Judgement::Judged {
                tag: j.tag.clone(),
                source: j.source.clone(),
                language: j.language,
                confidence: j.confidence,
                words: j.words,
            },
            Err(Unjudged::TooShort) => Judgement::TooShort,
            Err(Unjudged::Unclear) => Judgement::Unclear,
            Err(Unjudged::NoText) => Judgement::NoText,
        }
    }

    /// It, the paragraph's id `id` named.
    pub(super) fn named(&self, id: &str) -> Result<Judged, Unjudged> {
        match self {
            Judgement::Judged {
                tag,
                source,
                language,
                confidence,
                words,
            } => Ok(Judged {
                id: id.to_owned(),
                tag: tag.clone(),
                source: source.clone(),
                language: *language,
                confidence: *confidence,
                words: *words,
            }),
            Judgement::TooShort => Err(Unjudged::TooShort),
            Judgement::Unclear => Err(Unjudged::Unclear),
            Judgement::NoText => Err(Unjudged::NoText),
        }
    }
}

/// What a paragraph's judgement reads: the paragraph, its table's `w:tblPr` (`Some` in a
/// table), the styles part's root.
pub(super) struct Of<'a> {
    pub(super) paragraph: &'a Arc<Element>,
    pub(super) table: Option<Option<&'a Arc<Element>>>,
    pub(super) styles: Option<&'a Arc<Element>>,
}

/// One judgement, and what it was made of, held weakly.
#[derive(Debug, Clone)]
struct Kept {
    paragraph: Weak<Element>,
    table: Option<Option<Weak<Element>>>,
    styles: Option<Weak<Element>>,
    judgement: Judgement,
}

impl Weigh for Kept {
    fn weight(&self) -> usize {
        let text = |t: &Option<String>| t.as_ref().map_or(0, String::len);
        std::mem::size_of::<Kept>()
            + match &self.judgement {
                Judgement::Judged { tag, source, .. } => text(tag) + text(source),
                _ => 0,
            }
    }
}

fn same(kept: Option<&Weak<Element>>, now: Option<&Arc<Element>>) -> bool {
    match (kept, now) {
        (Some(k), Some(n)) => std::ptr::eq(k.as_ptr(), Arc::as_ptr(n)),
        (None, None) => true,
        _ => false,
    }
}

impl Kept {
    fn of(&self, of: &Of<'_>) -> bool {
        let table = match (&self.table, of.table) {
            (Some(k), Some(n)) => same(k.as_ref(), n),
            (None, None) => true,
            _ => false,
        };
        same(Some(&self.paragraph), Some(of.paragraph)) && table && same(self.styles.as_ref(), of.styles)
    }
}

/// What the judgements hold at most where no budget is given, bytes: a long report's
/// paragraphs (~70,000 at ~100 B) with room to spare.
pub const JUDGEMENT_BYTES: usize = 16 << 20;

/// A budget of a fixed number of bytes.
struct Limited {
    limit: usize,
    used: AtomicUsize,
}

impl CacheBudget for Limited {
    fn reserve(&self, bytes: usize) -> bool {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|after| *after <= self.limit)
            })
            .is_ok()
    }
    fn release(&self, bytes: usize) {
        let _ = self.used.try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            Some(used.saturating_sub(bytes))
        });
    }
    fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
}

/// Each paragraph's judgement kept across a document's checks, by the paragraph element's
/// address (a weak reference keeps the address its own while it is kept), within a budget:
/// the least recently used let go first, and those of paragraphs no version holds swept.
pub struct Judgements {
    memo: Mutex<Memo<usize, Kept>>,
}

impl std::fmt::Debug for Judgements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("Judgements").field("stats", &memo.stats()).finish()
    }
}

impl Default for Judgements {
    fn default() -> Judgements {
        Judgements::new(Arc::new(Limited {
            limit: JUDGEMENT_BYTES,
            used: AtomicUsize::new(0),
        }))
    }
}

impl Judgements {
    /// Judgements counted on `budget`.
    pub fn new(budget: Arc<dyn CacheBudget>) -> Judgements {
        Judgements {
            memo: Mutex::new(Memo::new(budget)),
        }
    }

    /// What it holds: its entries and the bytes reserved for them.
    pub fn stats(&self) -> MemoStats {
        self.memo.lock().unwrap_or_else(PoisonError::into_inner).stats()
    }

    /// The judgement of the paragraph `of` names, where a check before made it of the same.
    pub(super) fn get(&self, of: &Of<'_>) -> Option<Judgement> {
        let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = memo.get(&(Arc::as_ptr(of.paragraph) as usize))?;
        kept.of(of).then_some(kept.judgement)
    }

    /// Keep `judgement` as the paragraph `of` names; drop those of paragraphs no version
    /// holds (`sweep`: after a check that met every paragraph).
    pub(super) fn keep(&self, of: &Of<'_>, judgement: Judgement) {
        let kept = Kept {
            paragraph: Arc::downgrade(of.paragraph),
            table: of.table.map(|t| t.map(Arc::downgrade)),
            styles: of.styles.map(Arc::downgrade),
            judgement,
        };
        let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        memo.insert(Arc::as_ptr(of.paragraph) as usize, kept);
    }

    /// Drop the judgements of paragraphs no version holds that no check asked for since the
    /// last sweep.
    pub(super) fn sweep(&self) {
        let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        memo.sweep(|kept| kept.paragraph.strong_count() > 0);
    }
}
