//! Owns: the problems in some blocks (docx-agent's `validate.problems_in`, round 35, R18):
//! of a whole package's problems, those one of the blocks' elements carries (a repeated
//! paraId, a missing style a paragraph names, an unpaired bookmark's mark...) or that the
//! check finds again in the blocks alone (children out of order, text in a deletion, a cell
//! not ending in a paragraph), and how many are elsewhere ([`Scoped`]).
//!
//! Must not: find problems (it sorts those the whole check found), or cap a list (the tool
//! layer does).

use super::Problem;
use crate::names::{R, W, W14, WP};
use crate::WordDoc;
use ooxml_word_model::docx::casefold;
use ooxml_xml::Element;
use std::collections::HashSet;
use std::sync::Arc;

/// Revision records (docx-agent's `_RECORDS`).
const RECORDS: [&str; 14] = [
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "trPrChange",
    "tcPrChange",
    "numberingChange",
    "cellIns",
    "cellDel",
    "cellMerge",
];

/// Which elements a problem's `detail` names.
#[derive(Clone, Copy)]
enum Tags {
    /// Any of these `w:` elements.
    Word(&'static [&'static str]),
    /// `wp:docPr`.
    DocPr,
    /// Any element: the value of an attribute in the relationships namespace.
    AnyRelationship,
}

/// Per code, the elements and attribute (namespace, local name) a problem's `detail` is the
/// value of (docx-agent's `_HELD_BY`).
const HELD_BY: [(&str, Tags, Option<&str>, &str); 17] = [
    ("paraid-repeated", Tags::Word(&["p", "tr"]), Some(W14), "paraId"),
    ("paraid-out-of-range", Tags::Word(&["p", "tr"]), Some(W14), "paraId"),
    ("textid-without-paraid", Tags::Word(&["p", "tr"]), Some(W14), "textId"),
    ("textid-out-of-range", Tags::Word(&["p", "tr"]), Some(W14), "textId"),
    (
        "bookmark-unpaired",
        Tags::Word(&["bookmarkStart", "bookmarkEnd"]),
        Some(W),
        "id",
    ),
    ("bookmark-name-invalid", Tags::Word(&["bookmarkStart"]), Some(W), "name"),
    (
        "bookmark-name-repeated",
        Tags::Word(&["bookmarkStart"]),
        Some(W),
        "name",
    ),
    (
        "comment-range-unpaired",
        Tags::Word(&["commentRangeStart", "commentRangeEnd"]),
        Some(W),
        "id",
    ),
    (
        "comment-reference-without-comment",
        Tags::Word(&["commentReference"]),
        Some(W),
        "id",
    ),
    (
        "footnote-reference-without-note",
        Tags::Word(&["footnoteReference"]),
        Some(W),
        "id",
    ),
    (
        "endnote-reference-without-note",
        Tags::Word(&["endnoteReference"]),
        Some(W),
        "id",
    ),
    ("docpr-id-repeated", Tags::DocPr, None, "id"),
    ("numbering-num-missing", Tags::Word(&["numId"]), Some(W), "val"),
    ("numbering-level-out-of-range", Tags::Word(&["ilvl"]), Some(W), "val"),
    ("revision-id-repeated", Tags::Word(&RECORDS), Some(W), "id"),
    (
        "move-unpaired",
        Tags::Word(&["moveFromRangeStart", "moveToRangeStart"]),
        Some(W),
        "name",
    ),
    ("relationship-id-unknown", Tags::AnyRelationship, None, ""),
];

/// Codes reported against the main part although their marks may be in any story: looked
/// for in the blocks all the same (docx-agent's `_BODY_WIDE`).
const BODY_WIDE: [&str; 9] = [
    "bookmark-unpaired",
    "bookmark-name-invalid",
    "bookmark-name-repeated",
    "comment-range-unpaired",
    "comment-reference-without-comment",
    "footnote-reference-without-note",
    "endnote-reference-without-note",
    "docpr-id-repeated",
    "revision-id-repeated",
];

/// The problems in some blocks, and how many are elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scoped {
    /// The problems in the blocks, in the order given.
    pub inside: Vec<Problem>,
    /// How many of the problems given are not.
    pub elsewhere: usize,
}

/// What the blocks hold that a problem may name.
#[derive(Default)]
struct Held {
    /// `(code, detail)` of the problems found again in a block alone.
    local: HashSet<(String, String)>,
    /// `(code, value)` per [`HELD_BY`] rule, the value as the rule compares it.
    values: HashSet<(&'static str, String)>,
    /// `pStyle Name`, `rStyle Name`, `tblStyle Name` (`None` for a missing value).
    styles: HashSet<String>,
    /// The kinds of revision records without an author.
    authorless: HashSet<String>,
    /// Move range marks: `from 3`, `to 4`.
    moves: HashSet<String>,
}

/// Of `problems` (the whole package's, sorted), those in `roots`: blocks or paragraphs of
/// `part` of `doc`. A problem of the package, of another part or of the part as a whole (a
/// field begun and never ended) is in none.
#[must_use]
pub fn problems_in(doc: &WordDoc, problems: &[Problem], part: &str, roots: &[Arc<Element>]) -> Scoped {
    if roots.is_empty() || problems.is_empty() {
        return Scoped {
            inside: Vec::new(),
            elsewhere: problems.len(),
        };
    }
    let mut held = Held::default();
    let facts = &doc.shared().facts;
    for root in roots {
        let found = facts.element(root);
        let local = found
            .order
            .iter()
            .chain(&found.revision)
            .chain(&found.cells)
            .chain(&found.grids);
        held.local
            .extend(local.map(|(code, detail)| ((*code).to_owned(), detail.clone())));
        walk(root, &mut held);
    }
    let inside: Vec<Problem> = problems
        .iter()
        .filter(|p| (p.part == part || BODY_WIDE.contains(&p.code.as_str())) && holds(&held, p))
        .cloned()
        .collect();
    Scoped {
        elsewhere: problems.len() - inside.len(),
        inside,
    }
}

/// Whether the blocks hold `problem`.
fn holds(held: &Held, problem: &Problem) -> bool {
    let code = problem.code.as_str();
    if held.local.contains(&(problem.code.clone(), problem.detail.clone())) {
        return true;
    }
    match code {
        "style-missing" => return held.styles.contains(&problem.detail),
        "revision-without-author" => return held.authorless.contains(&problem.detail),
        "move-range-unpaired" => return held.moves.contains(&problem.detail),
        _ => {}
    }
    let Some((code, _, _, _)) = HELD_BY.iter().find(|(c, ..)| *c == code) else {
        return false;
    };
    held.values.contains(&(*code, compared(code, &problem.detail)))
}

/// A value as a rule compares it: bookmark names as Word compares them, relationship ids as
/// written, the rest upper-cased.
fn compared(code: &str, value: &str) -> String {
    match code {
        "bookmark-name-repeated" => casefold(value),
        "relationship-id-unknown" => value.to_owned(),
        _ => value.to_uppercase(),
    }
}

/// Everything at or below `element` a problem may name, into `held`.
fn walk(element: &Element, held: &mut Held) {
    let name = element.name();
    let word = name.namespace() == Some(W);
    let local = name.local();
    for (code, tags, namespace, attribute) in &HELD_BY {
        let carries = match tags {
            Tags::Word(locals) => word && locals.contains(&local),
            Tags::DocPr => name.is(Some(WP), "docPr"),
            Tags::AnyRelationship => {
                for a in element.attributes().iter().filter(|a| a.name.namespace() == Some(R)) {
                    held.values.insert((code, compared(code, &a.value)));
                }
                false
            }
        };
        if carries {
            if let Some(value) = element.attribute(*namespace, attribute) {
                held.values.insert((code, compared(code, value)));
            }
        }
    }
    if word {
        if matches!(local, "pStyle" | "rStyle" | "tblStyle") {
            let value = element.attribute(Some(W), "val").unwrap_or("None");
            held.styles.insert(format!("{local} {value}"));
        }
        if RECORDS.contains(&local) && element.attribute(Some(W), "author").is_none() {
            held.authorless.insert(local.to_owned());
        }
        if let Some(side) = match local {
            "moveFromRangeStart" | "moveFromRangeEnd" => Some("from"),
            "moveToRangeStart" | "moveToRangeEnd" => Some("to"),
            _ => None,
        } {
            let id = element.attribute(Some(W), "id").unwrap_or("");
            held.moves.insert(format!("{side} {id}"));
        }
    }
    for child in element.child_elements() {
        walk(child, held);
    }
}
