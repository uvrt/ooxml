//! Owns: [`SetParagraphStyle`], a paragraph's style set as docx-agent's
//! `Document.set_paragraph_style` sets it: the style found by name, alias or id -- a
//! built-in one the document lacks added as Word writes it ([`crate::builtin::ensure`]) --
//! as the paragraph's `w:pStyle`, the default style (or none) as no `w:pStyle`; tracked,
//! the paragraph's old properties recorded in a `w:pPrChange`.
//!
//! Must not: write but through the sink, or read outside what it records reading: the
//! paragraph's own block, the styles (and numbering) part when it adds a style, the
//! ledger seeds the stamping draws from, the annotation ids when tracked.

use crate::doc::Key;
use crate::marks::{clean_properties, insert_in_order};
use crate::names::{W, W16DU};
use crate::policy::RevisionPolicy;
use crate::set_text::{block_key, find, paragraph_has_annotations};
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::stamp_document;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::at_path;

/// Set a paragraph's style.
#[derive(Debug, Clone)]
pub struct SetParagraphStyle {
    /// The paragraph (an id, an alias, a cell's first paragraph).
    pub target: String,
    /// The paragraph style's name, alias or id; `None` for the default style.
    pub style: Option<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a set_paragraph_style did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSet {
    /// The paragraph's id after the edit.
    pub id: String,
    /// Ids the edit's stamping renamed, old to new, in the order made.
    pub renamed: Vec<(String, String)>,
    /// The paragraph's part.
    pub part: String,
}

/// Give the paragraph the `w:pStyle` `style` (none for `None`), as docx-agent's
/// `_formatting._set_child` writes it: an existing one keeps its place, a new one (and a
/// new `w:pPr`) goes where the schema wants it; a `w:pPr` left empty goes.
pub(crate) fn write_style(work: &mut Work, paragraph: Id, style: Option<&str>, orders: &crate::validate::Orders) {
    let properties = work.find(paragraph, W, "pPr");
    let Some(style) = style else {
        if let Some(properties) = properties {
            if let Some(node) = work.find(properties, W, "pStyle") {
                work.remove(node);
                if work.children(properties).is_empty() {
                    work.remove(properties);
                }
            }
        }
        return;
    };
    let properties = properties.unwrap_or_else(|| {
        let p = work.new_element(work.element_name(paragraph, W, "pPr", "w"));
        insert_in_order(work, paragraph, p, orders);
        p
    });
    let node = work.find(properties, W, "pStyle").unwrap_or_else(|| {
        let s = work.new_element(work.element_name(properties, W, "pStyle", "w"));
        insert_in_order(work, properties, s, orders);
        s
    });
    let val = work.qname(W, "val", "w");
    work.set(node, val, style);
}

impl Command<WordDoc> for SetParagraphStyle {
    type Out = StyleSet;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        find(doc, &docx, &self.target)?;
        if let Some(name) = &self.style {
            crate::builtin::resolve(&docx, name, "paragraph")?;
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<StyleSet> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = set_style(&mut sink, &self.target, self.style.as_deref())?;
        crate::textbox::sync(&mut sink)?;
        crate::revision_ids::make_unique(&mut sink)?;
        Ok(out)
    }
}

/// docx-agent's `set_paragraph_style` inside an edit, without what the edit does once it
/// ends (text boxes synced, revision ids made unique), which the command's caller does.
///
/// # Errors
/// A refusal for a target that is not a paragraph, or a style neither the document nor
/// Word has.
pub(crate) fn set_style(sink: &mut EditSink<'_, '_>, target: &str, style: Option<&str>) -> Result<StyleSet> {
    let docx = sink.docx()?;
    let found = find(sink.doc(), &docx, target)?;
    if let Some(name) = style {
        crate::builtin::resolve(&docx, name, "paragraph")?;
    }
    sink.read(block_key(&found.part, &found.id));
    let renamed = stamp_document(sink, &docx)?;
    let id = crate::ranges::renamed_id(&renamed, &found.id);
    let style = match style {
        Some(name) => Some(crate::builtin::ensure(sink, name, "paragraph")?),
        None => None,
    };
    let default = sink
        .docx()?
        .default_style("paragraph")
        .and_then(|s| s.attribute(Some(W), "styleId"))
        .map(str::to_owned);
    let style = style.filter(|s| Some(s) != default.as_ref());
    let tree = sink
        .doc()
        .tree(&found.part)?
        .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
    let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::Path(found.part.clone()))?;
    let mut work = Work::of(element, scope_above(tree.root(), &found.path));
    let paragraph = work.root();
    let properties = work.find(paragraph, W, "pPr");
    let old = clean_properties(&mut work, properties, paragraph);
    let shared = sink.doc().shared_handle();
    write_style(&mut work, paragraph, style.as_deref(), shared.facts.orders());
    let wrote = sink.paragraph_properties_changed(&mut work, paragraph, old)?;
    let new = work.build().ok_or_else(|| EditError::Path(found.part.clone()))?;
    // a paragraph that has that style already is not written (rule 3)
    if *new != **element {
        let draws = wrote
            .annotations
            .map(|n| annotation_draws(&new, n, 0))
            .unwrap_or_default();
        let mut keys = vec![block_key(&found.part, &id)];
        if paragraph_has_annotations(&found.element) || paragraph_has_annotations(&new) {
            keys.push(Key::Annotations);
        }
        sink.replace_drawing(&found.part, &found.path, new, keys, draws)?;
        if wrote.dated {
            sink.ensure_w14(&found.part, false, &[("w16du", W16DU)])?;
        }
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    }
    Ok(StyleSet {
        id,
        renamed,
        part: found.part,
    })
}
