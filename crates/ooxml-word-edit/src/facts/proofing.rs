//! Owns: what `punctuation_spacing` and `text_language` read of a document's proofing: the
//! styles' language cascade (docx-agent's `typography.Cascade`) with each style combination
//! read once, the styles part (its `w:lang` elements, the template's own languages) and
//! every story's paragraphs with what their runs are under.
//!
//! Must not: lay out, or word a fact.

use crate::names::W;
use crate::spacing::table_of;
use crate::styles::REL_STYLES;
use crate::Result;
use ooxml_word_model::docx::text::leading_text;
use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_word_model::typography::{Cascade, RunContext, RunLanguage, StyleKey};
use ooxml_xml::{Element, Node, Tree};
use std::collections::HashMap;
use std::sync::Arc;

/// A document's language cascade, each style combination read once.
pub(super) struct Proofing {
    cascade: Cascade,
    styled: HashMap<StyleKey, RunLanguage>,
    /// The styles part's tree, where there is one.
    pub(super) styles: Option<Tree>,
}

impl Proofing {
    /// The cascade of `docx`'s styles.
    pub(super) fn new(docx: &Docx) -> Result<Proofing> {
        let part = ooxml_word_model::parts::related(&docx.package, &docx.main, REL_STYLES)?;
        let styles = match part {
            Some(part) => docx.package.tree(&part)?,
            None => None,
        };
        let defaults = styles.as_ref().and_then(|tree| {
            tree.root()
                .find(Some(W), "docDefaults")
                .and_then(|d| d.find(Some(W), "rPrDefault"))
                .and_then(|d| d.find(Some(W), "rPr"))
                .cloned()
        });
        Ok(Proofing {
            cascade: Cascade::new(docx.styles(), defaults),
            styled: HashMap::new(),
            styles,
        })
    }

    /// The language, proofing, code-ness and source of a run under `context`.
    pub(super) fn of(&mut self, context: &RunContext<'_>) -> RunLanguage {
        let key = self.cascade.key(context);
        let cascade = &self.cascade;
        self.styled
            .entry(key)
            .or_insert_with_key(|key| cascade.styled(key))
            .clone()
            .with_direct(context.run_properties)
    }

    /// The values of the styles part's `w:lang` elements' `w:val`, in document order.
    pub(super) fn stated_languages(&self) -> Vec<&str> {
        fn walk<'a>(element: &'a Element, out: &mut Vec<&'a str>) {
            for child in element.child_elements() {
                if child.name().is(Some(W), "lang") {
                    out.extend(child.attribute(Some(W), "val"));
                }
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        if let Some(tree) = &self.styles {
            walk(tree.root(), &mut out);
        }
        out
    }
}

/// A paragraph of a story with what its runs are under, its id named only when asked.
pub(super) struct Placed<'a> {
    /// Its story: the part's place among the document's parts, and the part.
    pub(super) part: (usize, &'a StoryPart),
    /// Its ordinal among the part's paragraphs.
    pub(super) ordinal: usize,
    /// The paragraph.
    pub(super) element: &'a Arc<Element>,
    /// `Some` when it is in a table: the innermost one's `w:tblPr`.
    pub(super) table: Option<Option<Arc<Element>>>,
}

impl Placed<'_> {
    /// The context of a run with `run_properties` (its `w:rPr`) in the paragraph.
    pub(super) fn context<'e>(&'e self, run_properties: Option<&'e Element>) -> RunContext<'e> {
        RunContext {
            paragraph_properties: self.element.find(Some(W), "pPr").map(|p| &**p),
            table: self.table.as_ref().map(|t| t.as_deref()),
            run_properties,
        }
    }

    /// Which paragraph of the document it is: its part's place and its ordinal there.
    pub(super) fn who(&self) -> Who {
        Who {
            part: self.part.0,
            ordinal: self.ordinal,
        }
    }

    /// Its id.
    pub(super) fn id(&self) -> String {
        name(self.part.1, self.ordinal)
    }
}

/// A paragraph of a document: its part's place among the document's parts and its ordinal
/// among the part's paragraphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Who {
    pub(super) part: usize,
    pub(super) ordinal: usize,
}

impl Who {
    /// Its id in `docx`.
    pub(super) fn id(self, docx: &Docx) -> String {
        docx.parts
            .get(self.part)
            .map(|part| name(part, self.ordinal))
            .unwrap_or_default()
    }
}

/// The id of the paragraph with ordinal `ordinal` in `part`.
fn name(part: &StoryPart, ordinal: usize) -> String {
    part.ids.paragraphs().get(ordinal).map(|p| p.id).unwrap_or_default()
}

/// Every story's paragraphs in document order (docx-agent's `_parts()`, each part's index's
/// paragraphs), none named.
pub(super) fn paragraphs(docx: &Docx) -> impl Iterator<Item = Placed<'_>> + '_ {
    paragraphs_where(docx, |_| true)
}

/// [`paragraphs`] whose element `keep` takes: the others' places and tables are not read.
pub(super) fn paragraphs_where<F>(docx: &Docx, keep: F) -> impl Iterator<Item = Placed<'_>> + '_
where
    F: Fn(&Element) -> bool + Copy + 'static,
{
    docx.parts.iter().enumerate().flat_map(move |(k, part)| {
        part.ids
            .paragraphs()
            .placed_where(keep)
            .map(move |(ordinal, element, path)| Placed {
                part: (k, &**part),
                ordinal,
                element,
                table: table_of(part.root(), &path),
            })
    })
}

/// The text of every `w:t` below `element`, joined (deleted text, `w:delText`, is not).
pub(super) fn all_text(element: &Element) -> String {
    fn walk(element: &Element, out: &mut String) {
        for child in element.child_elements() {
            if child.name().is(Some(W), "t") {
                out.push_str(&leading_text(child));
            } else {
                walk(child, out);
            }
        }
    }
    let mut out = String::new();
    walk(element, &mut out);
    out
}

/// Whether the text of some `w:t` below `element` (what [`all_text`] joins) passes
/// `test`, read in place: for a test of single characters, the same as testing the joined
/// text.
pub(super) fn any_text(element: &Element, test: &impl Fn(&str) -> bool) -> bool {
    element.child_elements().any(|child| {
        if child.name().is(Some(W), "t") {
            child.children().iter().map_while(Node::as_text).any(test)
        } else {
            any_text(child, test)
        }
    })
}

/// A tag's language subtag, lower case (`fr` of `fr-BE` or `fr_BE`).
pub(super) fn primary(tag: Option<&str>) -> Option<String> {
    let tag = tag.filter(|t| !t.is_empty())?;
    Some(
        tag.replace('_', "-")
            .split('-')
            .next()
            .unwrap_or_default()
            .to_lowercase(),
    )
}
