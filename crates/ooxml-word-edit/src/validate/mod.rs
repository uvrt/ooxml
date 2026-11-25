//! Owns: validation per change (design sections 5 and 7): docx-agent's problems of a
//! version ([`problems`], its `validate.py` `check`) read from the version's `Tally` --
//! each block's facts memoised by content (`ElementFacts`, one walk of the block, kept
//! while the block is unchanged), held in persistent per-part trees and whole-document
//! counts that a new version takes from the previous one and changes by its changed
//! blocks; the delta against a baseline as the save gate compares it ([`Delta`]); the
//! largest annotation id in use (`annotation_ceiling`), which tracked edits number their
//! revisions from; and who still has revisions or comments (`author_in_use`). Of a
//! version's problems, those in some blocks ([`problems_in`]); and the problems only a save
//! finds, of the file name written to ([`extension_problems`]).
//!
//! No block's XML is read again and no version folds every block. The full check from
//! scratch ([`full_problems`]) is the test oracle.
//!
//! Must not: change a document, or hold anything but caches that belong to the document.

mod counts;
mod extension;
mod facts;
pub(crate) mod grid;
mod kept;
mod order;
mod rules;
mod scoped;
mod tally;

pub use extension::{extension_problems, WordKind, WORD_KINDS};
pub(crate) use facts::{python_int, ElementFacts};
pub(crate) use order::Orders;
pub use scoped::{problems_in, Scoped};
pub(crate) use tally::Tally;

use crate::doc::WordDoc;
use crate::Result;
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_word_model::ids::W;
use ooxml_xml::{Element, Node};
use rules::{body_problem, Inputs, PartFacts};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

/// A validation problem, as docx-agent reports it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Problem {
    /// What is wrong (`paraid-repeated`, `bookmark-unpaired`...).
    pub code: String,
    /// The part it is in.
    pub part: String,
    /// The value or place it concerns.
    pub detail: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.code, self.part, self.detail)
    }
}

/// The codes validation reports as warnings, not problems: what Word opens and repairs
/// itself the same way each time (`grid-after-differs`: a `w:gridAfter` Word does not read,
/// writing back the columns the row leaves). The save gate does not hold a new one against
/// a version.
pub const WARNINGS: &[&str] = &["grid-after-differs"];

impl Problem {
    /// Whether it is a warning ([`WARNINGS`]), not a problem.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        WARNINGS.contains(&self.code.as_str())
    }
}

/// The problems of a version against a baseline (the save gate's comparison); warnings
/// ([`Problem::is_warning`]) apart.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    /// Each problem now that the baseline lacks, in order (no warning).
    pub new: Vec<String>,
    /// Each baseline problem gone now (no warning).
    pub fixed: Vec<String>,
    /// How many problems the baseline has (no warning).
    pub baseline: usize,
    /// Each warning now that the baseline lacks, in order.
    pub warnings: Vec<String>,
}

impl Delta {
    /// `now` against `baseline`, compared as `(code, part, text)`.
    #[must_use]
    pub fn of(baseline: &[Problem], now: &[Problem]) -> Delta {
        let before: std::collections::HashSet<&Problem> = baseline.iter().collect();
        let after: std::collections::HashSet<&Problem> = now.iter().collect();
        let came = |warning: bool| -> Vec<String> {
            now.iter()
                .filter(|p| p.is_warning() == warning && !before.contains(p))
                .map(ToString::to_string)
                .collect()
        };
        Delta {
            new: came(false),
            fixed: baseline
                .iter()
                .filter(|p| !p.is_warning() && !after.contains(p))
                .map(ToString::to_string)
                .collect(),
            baseline: baseline.iter().filter(|p| !p.is_warning()).count(),
            warnings: came(true),
        }
    }
}

/// A memo entry: the element (held, so its address is not given to another) and its facts.
struct Held<T> {
    element: Arc<Element>,
    facts: Arc<T>,
}

impl<T> Clone for Held<T> {
    fn clone(&self) -> Self {
        Held {
            element: Arc::clone(&self.element),
            facts: Arc::clone(&self.facts),
        }
    }
}

impl Weigh for Held<ElementFacts> {
    fn weight(&self) -> usize {
        std::mem::size_of::<ElementFacts>() + 32 * (self.facts.para_ids.len() + self.facts.r_values.len() + 4)
    }
}

/// The facts of one document's blocks and parts, memoised by content (an element's
/// identity: green nodes are immutable).
pub struct Facts {
    orders: Orders,
    /// The facts of a node that holds none (text between blocks), shared.
    empty: Arc<ElementFacts>,
    blocks: Mutex<Memo<usize, Held<ElementFacts>>>,
}

impl fmt::Debug for Facts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Facts").finish_non_exhaustive()
    }
}

/// How a part is walked: the main part by its body's children, a story part by its root's,
/// any other part whole.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Main,
    Story,
    Other,
}

impl Facts {
    /// Facts kept within `budget`.
    #[must_use]
    pub fn new(budget: Arc<dyn CacheBudget>) -> Facts {
        Facts {
            orders: Orders::load(),
            empty: Arc::new(ElementFacts::default()),
            blocks: Mutex::new(Memo::new(budget)),
        }
    }

    /// The child orders validation checks against.
    pub(crate) fn orders(&self) -> &Orders {
        &self.orders
    }

    /// The facts of one element (a block), from the memo when it is unchanged.
    pub(crate) fn element(&self, element: &Arc<Element>) -> Arc<ElementFacts> {
        let key = Arc::as_ptr(element) as usize;
        let held = self.blocks.lock().unwrap_or_else(PoisonError::into_inner).get(&key);
        if let Some(held) = held.filter(|h| Arc::ptr_eq(&h.element, element)) {
            return held.facts;
        }
        let facts = Arc::new(ElementFacts::of(element, &self.orders));
        let held = Held {
            element: Arc::clone(element),
            facts: Arc::clone(&facts),
        };
        self.blocks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, held);
        facts
    }

    /// A part read whole, for the oracle (every block's facts looked up or read).
    fn read_part(&self, root: &Arc<Element>, kind: Kind) -> PartFacts {
        let body = (kind == Kind::Main).then(|| root.find(Some(W), "body")).flatten();
        let holder: Option<&Arc<Element>> = match kind {
            Kind::Main => body,
            Kind::Story => Some(root),
            Kind::Other => None,
        };
        let elements: Vec<Arc<Element>> = match holder {
            Some(holder) => holder.children().iter().filter_map(Node::as_element).cloned().collect(),
            None => vec![Arc::clone(root)],
        };
        let blocks = elements.iter().map(|block| self.element(block)).collect();
        let rest = match kind {
            Kind::Other => ElementFacts::default(),
            Kind::Story => ElementFacts::of_rest(root, None, &self.orders, true),
            Kind::Main => ElementFacts::of_rest(root, body.map(|b| &**b), &self.orders, true),
        };
        PartFacts {
            blocks,
            rest,
            root: Arc::clone(root),
            body: (kind == Kind::Main).then(|| body_problem(root)).flatten(),
        }
    }
}

/// A version's parts as validation reads them.
struct Read {
    main: String,
    stories: Vec<String>,
    parts: Vec<(String, std::result::Result<Arc<PartFacts>, String>)>,
}

/// Every XML part of `doc` read for validation from scratch (the oracle): its facts, or
/// why it is not XML.
fn read_parts(doc: &WordDoc) -> Result<Read> {
    let docx = doc.docx()?;
    let stories: Vec<String> = docx.parts.iter().map(|p| p.part.clone()).collect();
    let facts = Facts::new(Arc::new(Unlimited));
    let mut parts = Vec::new();
    for name in doc.package().part_names() {
        if !(name.ends_with(".xml") || name.ends_with(".rels")) {
            continue;
        }
        let kind = if name == docx.main {
            Kind::Main
        } else if stories.contains(&name) {
            Kind::Story
        } else {
            Kind::Other
        };
        let found = match doc.package().tree(&name) {
            Ok(Some(tree)) => Ok(Arc::new(facts.read_part(tree.root(), kind))),
            Ok(None) => continue,
            Err(error) => Err(error.to_string()),
        };
        parts.push((name, found));
    }
    Ok(Read {
        main: docx.main.clone(),
        stories,
        parts,
    })
}

/// The problems of `doc`, sorted, from its memoised facts.
///
/// # Errors
/// When the document cannot be read as a Word document.
pub fn problems(doc: &WordDoc) -> Result<Vec<Problem>> {
    Ok(kept::problems(&*doc.tally()?, doc.package()))
}

/// The problems of `doc` found with no memo at all: the oracle [`problems`] is tested
/// against.
///
/// # Errors
/// As [`problems`].
pub fn full_problems(doc: &WordDoc) -> Result<Vec<Problem>> {
    let read = read_parts(doc)?;
    Ok(rules::problems(&Inputs {
        package: doc.package(),
        main: &read.main,
        stories: &read.stories,
        parts: &read.parts,
    }))
}

/// Whether a story part of `doc` has a live drawing without its anchorId and editId.
///
/// # Errors
/// When the document cannot be read as a Word document.
pub(crate) fn unstamped_drawings(doc: &WordDoc, part: &str) -> Result<bool> {
    Ok(kept::unstamped_drawings(&*doc.tally()?, part))
}

/// The largest annotation id in use in any story of `doc`, `-1` when none is.
///
/// # Errors
/// When the document cannot be read as a Word document.
pub(crate) fn annotation_ceiling(doc: &WordDoc) -> Result<i64> {
    Ok(kept::annotation_ceiling(&*doc.tally()?))
}

/// Whether `author` made a revision or a comment still in `doc` (docx-agent's
/// `_authors_in_use`).
///
/// # Errors
/// When the document cannot be read as a Word document.
pub(crate) fn author_in_use(doc: &WordDoc, author: &str) -> Result<bool> {
    Ok(doc.tally()?.counts.author_in_use(author))
}

/// A budget with no limit, for the oracle's memo-free check and for scratch versions that
/// are dropped after one reading (`facts`' settled table copies).
pub(crate) struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::rules::bookmark_name_ok;
    use super::{Delta, Problem};

    fn problem(code: &str, detail: &str) -> Problem {
        Problem {
            code: code.to_owned(),
            part: "word/document.xml".to_owned(),
            detail: detail.to_owned(),
        }
    }

    #[test]
    fn a_warning_is_kept_apart_from_the_problems_the_save_gate_counts() {
        let baseline = [problem("grid-row-mismatch", "a"), problem("grid-after-differs", "b")];
        let now = [problem("grid-after-differs", "c"), problem("paraid-repeated", "d")];
        let delta = Delta::of(&baseline, &now);
        assert_eq!(delta.new, ["[paraid-repeated] word/document.xml: d"]);
        assert_eq!(delta.warnings, ["[grid-after-differs] word/document.xml: c"]);
        assert_eq!(delta.fixed, ["[grid-row-mismatch] word/document.xml: a"]);
        assert_eq!(delta.baseline, 1);
    }

    #[test]
    fn bookmark_names_as_pythons_re_reads_them() {
        // `[^\W\d][\w]{0,39}` in a `str` pattern: `\d` is a decimal digit, `\w` `isalnum` or `_`
        for name in ["_a", "a1", "\u{b2}a", "\u{2170}x", "x\u{2170}", "\u{4e94}"] {
            assert!(bookmark_name_ok(name), "{name:?}");
        }
        for name in ["", "1a", "\u{663}a", "a\u{93e}", "\u{93e}a", &"a".repeat(41)] {
            assert!(!bookmark_name_ok(name), "{name:?}");
        }
        assert!(bookmark_name_ok(&"a".repeat(40)));
    }
}
