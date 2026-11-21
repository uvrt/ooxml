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
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::ParagraphId;
use ooxml_word_model::typography::{Cascade, RunContext, RunLanguage, StyleKey};
use ooxml_xml::{Element, Tree};
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

/// A paragraph of a story with what its runs are under.
pub(super) struct Placed {
    /// The paragraph, its place and id.
    pub(super) paragraph: ParagraphId,
    /// `Some` when it is in a table: the innermost one's `w:tblPr`.
    pub(super) table: Option<Option<Arc<Element>>>,
}

impl Placed {
    /// The context of a run with `run_properties` (its `w:rPr`) in the paragraph.
    pub(super) fn context<'e>(&'e self, run_properties: Option<&'e Element>) -> RunContext<'e> {
        RunContext {
            paragraph_properties: self.paragraph.element.find(Some(W), "pPr").map(|p| &**p),
            table: self.table.as_ref().map(|t| t.as_deref()),
            run_properties,
        }
    }
}

/// Every story's paragraphs in document order (docx-agent's `_parts()`, each part's index's
/// paragraphs).
pub(super) fn paragraphs(docx: &Docx) -> impl Iterator<Item = Placed> + '_ {
    docx.parts.iter().flat_map(|part| {
        part.ids.paragraphs().iter().map(move |paragraph| Placed {
            table: table_of(part.root(), &paragraph.path),
            paragraph,
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
