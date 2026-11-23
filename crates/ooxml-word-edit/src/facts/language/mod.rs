//! Owns: `check`'s `text_language` (docx-agent's `text_language.facts`, round 49 R94): the
//! paragraphs whose words are another language than the tag Word proofs them in (consecutive
//! top-level ones alike as one range), and the document's language by its words next to the
//! one its default paragraph style gives, with the `set_language` that retags it.
//!
//! Must not: lay out (the check's pages are the caller's `keep`), cap the paragraphs listed
//! or word the fact (the tool layer does).

mod detect;
mod detections;
mod judged;

pub use detect::{detect, words_of, Detected, LANGUAGES, MIN_WORDS};
use detections::Detecting;
pub use detections::Detections;

use super::proofing::{all_text, paragraphs, primary, Placed, Proofing};
use super::py_space;
use crate::names::W;
use crate::text::walk;
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use detect::two_places;
use ooxml_word_model::docx::text::leading_text;
use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_word_model::typography::RunContext;
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A paragraph is listed as another language than its tag at least this sure.
pub const MIN_CONFIDENCE: f64 = 0.5;

/// The tag a fix proposes for a language the document names nowhere, by [`LANGUAGES`].
const DEFAULT_TAGS: [&str; 7] = ["en-US", "fr-FR", "nl-NL", "de-DE", "es-ES", "it-IT", "pt-PT"];

/// What a `text_language` check reads.
#[derive(Clone, Copy)]
pub struct LanguageScope<'a> {
    /// Whether a paragraph (by id) is in the check's pages and range; `None`: all are.
    pub keep: Option<&'a dyn Fn(&str) -> bool>,
    /// Whether that is the whole document: else the language found is of the words checked
    /// and says nothing of the document's (no paragraph folded into it, no fix).
    pub whole: bool,
    /// Whether consecutive top-level paragraphs alike are one entry (the layout's check).
    pub ranges: bool,
    /// The session's language tag: the tag a fix sets where it is the language found.
    pub locale: Option<&'a str>,
    /// The texts' languages the document's checks keep (`None`: each text detected).
    pub detections: Option<&'a Detections>,
}

impl<'a> LanguageScope<'a> {
    /// Every paragraph read, each text detected afresh, no session tag; `whole` and `ranges`
    /// as on the fields.
    #[must_use]
    pub fn new(whole: bool, ranges: bool) -> Self {
        Self {
            keep: None,
            whole,
            ranges,
            locale: None,
            detections: None,
        }
    }

    /// Only the paragraphs (by id) `keep` holds.
    #[must_use]
    pub fn keeping(self, keep: &'a dyn Fn(&str) -> bool) -> Self {
        Self {
            keep: Some(keep),
            ..self
        }
    }

    /// The texts' languages the document's checks keep, read instead of detected again.
    #[must_use]
    pub fn with_detections(self, detections: &'a Detections) -> Self {
        Self {
            detections: Some(detections),
            ..self
        }
    }
}

/// `check`'s `text_language`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLanguage {
    /// Whether it is of the whole document (`document`), else of the words checked.
    pub whole: bool,
    /// The language the default paragraph style gives, and the one found.
    pub document: DocumentLanguage,
    /// The paragraphs whose words are another language than their tag, in document order.
    pub mismatched: Vec<Mistagged>,
    /// The paragraphs with text too short to judge.
    pub too_short: usize,
    /// Those with words enough but too few function words.
    pub unclear: usize,
}

/// The document's (or the checked words') language.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentLanguage {
    /// The default paragraph style's tag.
    pub tag: Option<String>,
    /// Where it comes from (`default`, `style:<name>`).
    pub source: Option<String>,
    /// The language most of the surely judged paragraphs' words are in; `None` when none is
    /// that sure.
    pub detected: Option<Dominant>,
    /// The paragraphs judged.
    pub paragraphs: usize,
}

/// The language most words are in.
#[derive(Debug, Clone, PartialEq)]
pub struct Dominant {
    /// The language.
    pub language: &'static str,
    /// Its share of the words, rounded to two decimals.
    pub share: f64,
    /// The words of the paragraphs surely judged.
    pub words: usize,
    /// Where it is not the tag's (the whole document only): what retags it.
    pub retag: Option<Retag>,
}

/// The document's language set to the one its words are in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retag {
    /// The paragraphs in that language under the document's tag (not listed one by one).
    pub mismatched: usize,
    /// The tag `set_language` sets.
    pub language: String,
}

/// A paragraph, or consecutive top-level ones alike, in another language than its tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Mistagged {
    /// The (first) paragraph.
    pub para: String,
    /// For consecutive ones: the last, and how many top-level paragraphs the range spans.
    pub through: Option<(String, usize)>,
    /// The tag most of the text has.
    pub tag: Option<String>,
    /// Where it comes from (`run`: the runs' own `w:lang`).
    pub source: Option<String>,
    /// The language its words are in.
    pub detected: &'static str,
    /// How sure, by words, rounded to two decimals.
    pub confidence: f64,
    /// Its words.
    pub words: usize,
}

/// A paragraph judged.
struct Judged {
    id: String,
    tag: Option<String>,
    source: Option<String>,
    language: usize,
    confidence: f64,
    words: usize,
}

/// Why a paragraph is not judged.
enum Unjudged {
    TooShort,
    Unclear,
    NoText,
}

/// `check`'s `text_language` of `doc`; `None` when no paragraph has words enough to judge.
///
/// # Errors
/// When the version cannot be read.
pub fn text_language(doc: &WordDoc, scope: LanguageScope<'_>) -> Result<Option<TextLanguage>> {
    let docx = doc.docx()?;
    let mut proofing = Proofing::new(&docx)?;
    let Reading { judged, short, unclear } = read(&docx, &mut proofing, scope);
    if judged.is_empty() {
        return Ok(None);
    }
    let default = proofing.of(&RunContext::default());
    let mut words = [0_usize; LANGUAGES.len()];
    for item in judged.iter().filter(|j| j.confidence >= MIN_CONFIDENCE) {
        if let Some(count) = words.get_mut(item.language) {
            *count += item.words;
        }
    }
    let total: usize = words.iter().sum();
    let mut out = TextLanguage {
        whole: scope.whole,
        document: DocumentLanguage {
            tag: default.language.clone(),
            source: default.source.clone(),
            detected: None,
            paragraphs: judged.len(),
        },
        mismatched: Vec::new(),
        too_short: short.len(),
        unclear,
    };
    if total == 0 {
        return Ok(Some(out));
    }
    // the first of the most (Python's `max` over LANGUAGES)
    let dominant = (0..LANGUAGES.len()).fold(0, |best, k| if words.get(k) > words.get(best) { k } else { best });
    let own = primary(default.language.as_deref());
    let profiled = |tag: Option<&str>| tag.is_some_and(|t| LANGUAGES.contains(&t));
    let differs = scope.whole && profiled(own.as_deref()) && own.as_deref() != LANGUAGES.get(dominant).copied();
    let mut folded = 0;
    let mut listed = Vec::new();
    for item in judged {
        let tag = primary(item.tag.as_deref());
        let mismatched = profiled(tag.as_deref())
            && tag.as_deref() != LANGUAGES.get(item.language).copied()
            && item.confidence >= MIN_CONFIDENCE;
        if !mismatched {
            continue;
        }
        if differs && item.language == dominant && tag == own && item.source.as_deref() != Some("run") {
            folded += 1; // the document's own mismatch
            continue;
        }
        listed.push(item);
    }
    #[allow(clippy::cast_precision_loss)] // word counts
    let share = two_places(words.get(dominant).copied().unwrap_or(0) as f64 / total as f64);
    let language = LANGUAGES.get(dominant).copied().unwrap_or("en");
    out.document.detected = Some(Dominant {
        language,
        share,
        words: total,
        retag: differs.then(|| Retag {
            mismatched: folded,
            language: tag_for(&proofing, scope.locale, dominant),
        }),
    });
    let blocks = if scope.ranges {
        top_level(docx.body().ok_or_else(|| EditError::NoPart(docx.main.clone()))?)
    } else {
        Vec::new()
    };
    out.mismatched = grouped(listed, &blocks, &short);
    Ok(Some(out))
}

/// What the paragraphs checked are: those judged, the ids of those too short to judge, the
/// count of those unclear.
struct Reading {
    judged: Vec<Judged>,
    short: HashSet<String>,
    unclear: usize,
}

/// Every story's paragraphs in `scope` judged (in document order).
fn read(docx: &Docx, proofing: &mut Proofing, scope: LanguageScope<'_>) -> Reading {
    let mut out = Reading {
        judged: Vec::new(),
        short: HashSet::new(),
        unclear: 0,
    };
    let mut detecting = Detecting::new(scope.detections, scope.whole);
    let styles = proofing.styles.as_ref().map(|t| Arc::clone(t.root()));
    let memo = scope.detections.map(|d| &d.paragraphs);
    for placed in paragraphs(docx) {
        if scope.keep.is_some_and(|keep| !keep(&placed.paragraph.id)) {
            continue;
        }
        let of = judged::Of {
            paragraph: &placed.paragraph.element,
            table: placed.table.as_ref().map(Option::as_ref),
            styles: styles.as_ref(),
        };
        let found = match memo.and_then(|m| m.get(&of)) {
            Some(kept) => kept.named(&placed.paragraph.id),
            None => {
                let found = judge(proofing, &placed, &mut detecting);
                if let Some(memo) = memo {
                    memo.keep(&of, judged::Judgement::of(&found));
                }
                found
            }
        };
        match found {
            Ok(found) => out.judged.push(found),
            Err(Unjudged::TooShort) => {
                out.short.insert(placed.paragraph.id.clone());
            }
            Err(Unjudged::Unclear) => out.unclear += 1,
            Err(Unjudged::NoText) => {}
        }
    }
    detecting.finish();
    out
}

/// A run's tag and where it comes from.
type Tagged = (Option<String>, Option<String>);

/// A paragraph's tag and detected language (`_judge`), or why it is not judged. Its runs'
/// text (`w:t`; a text box's paragraphs left to themselves), without the text Word does not
/// proof or that is code.
fn judge(
    proofing: &mut Proofing,
    placed: &Placed,
    detecting: &mut Detecting<'_>,
) -> std::result::Result<Judged, Unjudged> {
    let element = &placed.paragraph.element;
    let raw = all_text(element);
    if words_of(&raw).len() < MIN_WORDS {
        return Err(if raw.chars().all(py_space) {
            Unjudged::NoText
        } else {
            Unjudged::TooShort
        });
    }
    let mut chars: Vec<(Tagged, usize)> = Vec::new();
    let mut text = String::new();
    let mut take = |written: String, direct: Option<&Element>| {
        if written.is_empty() {
            return;
        }
        let found = proofing.of(&placed.context(direct));
        if !found.proofed || found.code {
            text.push(' ');
            return;
        }
        let count = written.chars().count();
        text.push_str(&written);
        let key = (found.language, found.source);
        match chars.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 += count,
            None => chars.push((key, count)),
        }
    };
    if has_text_box(element) {
        let work = Work::of(element, Vec::new());
        for run in walk(&work, work.root()).runs {
            let written: String = work
                .children(run)
                .iter()
                .filter(|c| work.is(**c, W, "t"))
                .map(|c| work.text(*c))
                .collect();
            let direct = work.find(run, W, "rPr").and_then(|r| work.build_element(r));
            take(written, direct.as_deref());
        }
    } else {
        for run in runs_below(element) {
            let written: String = run
                .child_elements()
                .filter(|c| c.name().is(Some(W), "t"))
                .map(|c| leading_text(c))
                .collect();
            take(written, run.find(Some(W), "rPr").map(|r| &**r));
        }
    }
    let found = detecting.detect(&text);
    let Some(language) = found.language.filter(|_| !chars.is_empty()) else {
        return Err(if found.words < MIN_WORDS {
            Unjudged::TooShort
        } else {
            Unjudged::Unclear
        });
    };
    // the tag most of the text has: the first of the most
    let ((tag, source), _) = chars
        .into_iter()
        .fold(None, |best: Option<(_, usize)>, item| match best {
            Some(b) if b.1 >= item.1 => Some(b),
            _ => Some(item),
        })
        .unwrap_or_default();
    Ok(Judged {
        id: placed.paragraph.id.clone(),
        tag,
        source,
        language,
        confidence: found.confidence,
        words: found.words,
    })
}

/// Whether a text box is below `element`.
fn has_text_box(element: &Element) -> bool {
    element
        .child_elements()
        .any(|c| c.name().is(Some(W), "txbxContent") || has_text_box(c))
}

/// Every `w:r` below `element`, in document order.
fn runs_below(element: &Element) -> Vec<&Element> {
    fn visit<'a>(element: &'a Element, out: &mut Vec<&'a Element>) {
        for child in element.child_elements() {
            if child.name().is(Some(W), "r") {
                out.push(child);
            }
            visit(child, out);
        }
    }
    let mut out = Vec::new();
    visit(element, &mut out);
    out
}

/// The tag a fix sets for `language` (`tag_for`): the locale's when it is that language with
/// a region, else a tag of it with a region the styles state, else [`DEFAULT_TAGS`]'.
fn tag_for(proofing: &Proofing, locale: Option<&str>, language: usize) -> String {
    let wanted = LANGUAGES.get(language).copied();
    let of = |tag: &str| primary(Some(tag)).as_deref() == wanted && tag.contains('-');
    if let Some(locale) = locale.filter(|l| of(l)) {
        return locale.to_owned();
    }
    proofing
        .stated_languages()
        .into_iter()
        .find(|v| of(v))
        .or_else(|| DEFAULT_TAGS.get(language).copied())
        .unwrap_or("en-US")
        .to_owned()
}

/// A top-level block of the body: whether it is a paragraph, its element and id.
struct Block<'a> {
    paragraph: bool,
    element: &'a Element,
    id: String,
}

/// The body's blocks in order, a block-level content control opened (`_top_level`): those
/// the index names.
fn top_level(body: &StoryPart) -> Vec<Block<'_>> {
    fn visit<'a>(body: &'a StoryPart, container: &'a Element, at: &[u32], out: &mut Vec<Block<'a>>) {
        for (k, child) in container.children().iter().enumerate() {
            let Some(child) = child.as_element() else { continue };
            let place = [at, &[u32::try_from(k).unwrap_or(u32::MAX)]].concat();
            if child.name().is(Some(W), "sdt") {
                let content = child
                    .children()
                    .iter()
                    .enumerate()
                    .find(|(_, n)| n.as_element().is_some_and(|e| e.name().is(Some(W), "sdtContent")));
                if let Some((j, content)) = content.and_then(|(j, n)| n.as_element().map(|e| (j, e))) {
                    let inner = [place.as_slice(), &[u32::try_from(j).unwrap_or(u32::MAX)]].concat();
                    visit(body, content, &inner, out);
                }
            } else if child.name().is(Some(W), "p") {
                if let Some(found) = body.ids.paragraph_at(&place) {
                    out.push(Block {
                        paragraph: true,
                        element: child,
                        id: found.id,
                    });
                }
            } else if child.name().is(Some(W), "tbl") {
                if let Some(found) = body.ids.table_at(&place) {
                    out.push(Block {
                        paragraph: false,
                        element: child,
                        id: found.id,
                    });
                }
            }
        }
    }
    let root = body.root();
    let mut out = Vec::new();
    for (k, child) in root.children().iter().enumerate() {
        if let Some(element) = child.as_element().filter(|e| e.name().is(Some(W), "body")) {
            visit(body, element, &[u32::try_from(k).unwrap_or(u32::MAX)], &mut out);
        }
    }
    out
}

/// `listed` as entries (`_grouped`), consecutive top-level paragraphs alike -- only
/// paragraphs too short to judge or without text between them -- as one range.
fn grouped(listed: Vec<Judged>, blocks: &[Block<'_>], short: &HashSet<String>) -> Vec<Mistagged> {
    let order: HashMap<&str, usize> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.paragraph)
        .map(|(k, b)| (b.id.as_str(), k))
        .collect();
    let alike = |a: &Judged, b: &Judged| {
        let (Some(&from), Some(&to)) = (order.get(a.id.as_str()), order.get(b.id.as_str())) else {
            return false;
        };
        if (primary(a.tag.as_deref()), &a.source, a.language) != (primary(b.tag.as_deref()), &b.source, b.language) {
            return false;
        }
        blocks
            .get(from + 1..to)
            .unwrap_or_default()
            .iter()
            .all(|block| block.paragraph && (short.contains(&block.id) || !has_text(block.element)))
    };
    let mut out = Vec::new();
    let mut run: Vec<Judged> = Vec::new();
    let flush = |run: &mut Vec<Judged>, out: &mut Vec<Mistagged>| {
        let (Some(first), Some(last)) = (run.first(), run.last()) else {
            return;
        };
        let words: usize = run.iter().map(|j| j.words).sum();
        #[allow(clippy::cast_precision_loss)] // word counts
        let weighted: f64 = run.iter().map(|j| j.confidence * j.words as f64).sum::<f64>() / words as f64;
        let through = (run.len() > 1).then(|| {
            let span = order.get(last.id.as_str()).copied().unwrap_or(0)
                - order.get(first.id.as_str()).copied().unwrap_or(0)
                + 1;
            (last.id.clone(), span)
        });
        out.push(Mistagged {
            para: first.id.clone(),
            through,
            tag: first.tag.clone(),
            source: first.source.clone(),
            detected: LANGUAGES.get(first.language).copied().unwrap_or("en"),
            confidence: two_places(weighted),
            words,
        });
        run.clear();
    };
    for item in listed {
        if run.last().is_some_and(|last| !alike(last, &item)) {
            flush(&mut run, &mut out);
        }
        run.push(item);
    }
    flush(&mut run, &mut out);
    out
}

/// Whether `element` has a `w:t` with text that is not white space (`_has_text`).
fn has_text(element: &Element) -> bool {
    element
        .child_elements()
        .any(|c| (c.name().is(Some(W), "t") && !leading_text(c).chars().all(py_space)) || has_text(c))
}
