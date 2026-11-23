//! Owns: [`Judgements`], each paragraph's judgement for `text_language` (its runs' tag and
//! the language detected in its text, or why it is not judged) kept across a document's
//! checks -- the block tier of the caching policy (ADR-008): by the paragraph's element,
//! the `w:tblPr` of the table it is in and the styles part's root, each by identity (a
//! version that shares them judges the paragraph as before). A check after a local edit
//! judges only the paragraphs the edit made anew.
//!
//! Must not: keep an element alive (they are held weakly; a judgement no version can ask
//! for is swept), or keep anything a paragraph's judgement does not read.

use super::{Judged, Unjudged};
use ooxml_xml::Element;
use std::collections::HashMap;
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
#[derive(Debug)]
struct Kept {
    paragraph: Weak<Element>,
    table: Option<Option<Weak<Element>>>,
    styles: Option<Weak<Element>>,
    judgement: Judgement,
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

/// The judgements, and how many were alive when last swept.
#[derive(Debug, Default)]
struct Map {
    kept: HashMap<usize, Kept>,
    alive: usize,
}

/// Each paragraph's judgement kept across a document's checks, by the paragraph element's
/// address (a weak reference keeps the address its own while it is kept).
#[derive(Debug, Default)]
pub struct Judgements {
    map: Mutex<Map>,
}

impl Judgements {
    /// The judgement of the paragraph `of` names, where a check before made it of the same.
    pub(super) fn get(&self, of: &Of<'_>) -> Option<Judgement> {
        let map = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = map.kept.get(&(Arc::as_ptr(of.paragraph) as usize))?;
        kept.of(of).then(|| kept.judgement.clone())
    }

    /// Keep `judgement` as the paragraph `of` names. Judgements of paragraphs no version
    /// holds are swept once the memo has doubled since its last sweep.
    pub(super) fn keep(&self, of: &Of<'_>, judgement: Judgement) {
        let kept = Kept {
            paragraph: Arc::downgrade(of.paragraph),
            table: of.table.map(|t| t.map(Arc::downgrade)),
            styles: of.styles.map(Arc::downgrade),
            judgement,
        };
        let mut map = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        map.kept.insert(Arc::as_ptr(of.paragraph) as usize, kept);
        if map.kept.len() > 2 * map.alive + 64 {
            map.kept.retain(|_, k| k.paragraph.strong_count() > 0);
            map.alive = map.kept.len();
        }
    }
}
