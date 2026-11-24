//! Owns: one paragraph's formatting edit as docx-agent's `FormatOps` makes it -- the
//! document stamped (once, by the first edit that changes something), the paragraph copied
//! into a work, its formatting watched (tracked: the property tracker), the change made,
//! the old formatting recorded, the paragraph put back -- and the changes `word_format`
//! makes there ([`ParagraphOp`]: direct formatting, its clearing, a paragraph style, a
//! sample's look) and on a range's runs ([`RangeOp`]).
//!
//! Must not: write but through the sink, or read a tool's JSON.

use super::colour::ThemeColours;
use super::like::{take, ParagraphLook};
use super::values::{clear_paragraph, clear_run, properties_of, set_child, write_paragraph, write_run, Setting};
use super::FormatStep;
use crate::doc::Key;
use crate::inline::{isolate, merge_around, prune};
use crate::merge::merge_runs;
use crate::names::{W, W16DU};
use crate::paragraph_style::write_style;
use crate::policy::RevisionPolicy;
use crate::ranges::renamed_id;
use crate::set_text::{block_key, paragraph_has_annotations};
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::stamp_document;
use crate::text::runs;
use crate::validate::Orders;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::at_path;

/// A change to a whole paragraph.
pub(crate) enum ParagraphOp<'a> {
    /// Direct paragraph settings, then run settings on every run and the mark
    /// (`format_paragraph`).
    Settings(&'a [Setting], &'a [Setting]),
    /// Its direct formatting removed, its runs' and mark's too (`clear_formatting`).
    Clear,
    /// The paragraph style with this id (`None`: the default; `set_paragraph_style`).
    Style(Option<&'a str>),
    /// A sample's look, the parts named (`format_like`).
    Like(&'a ParagraphLook, &'a [&'a str]),
}

/// A change to a range's runs, from a character to another, the marks of every paragraph
/// but the last too where it says so (`_format_range`).
pub(crate) enum RangeOp<'a> {
    /// Run settings (marks too).
    Settings(&'a [Setting]),
    /// Direct formatting removed (`clear_formatting` of a range).
    Clear,
    /// The character style with this id (`None`: none; `set_character_style`).
    Style(Option<&'a str>),
}

impl RangeOp<'_> {
    fn marks(&self) -> bool {
        matches!(self, Self::Settings(_))
    }
}

/// What every edit of one `word_format` call shares.
pub(crate) struct Run<'s, 't, 'x> {
    pub(crate) sink: &'s mut EditSink<'t, 'x>,
    pub(crate) colours: ThemeColours,
    stamped: bool,
}

impl<'s, 't, 'x> Run<'s, 't, 'x> {
    pub(crate) fn new(sink: &'s mut EditSink<'t, 'x>, colours: ThemeColours) -> Self {
        Run {
            sink,
            colours,
            stamped: false,
        }
    }

    /// Stamp the document if no edit did yet; the renames.
    pub(crate) fn stamp(&mut self) -> Result<Vec<(String, String)>> {
        if self.stamped {
            return Ok(Vec::new());
        }
        self.stamped = true;
        let docx = self.sink.docx()?;
        stamp_document(self.sink, &docx)
    }

    /// Edit the paragraph `index` of `part` by `change` in a work; the step (its id after
    /// the edit, the renames its stamping made).
    pub(crate) fn edit(
        &mut self,
        part: &str,
        index: usize,
        change: impl FnOnce(&mut Work, Id, &Orders, &ThemeColours) -> Result<()>,
    ) -> Result<FormatStep> {
        let renamed = self.stamp()?;
        let docx = self.sink.docx()?;
        let story = docx.part(part).ok_or_else(|| EditError::no_part(part.to_owned()))?;
        let paragraph = story
            .ids
            .paragraphs()
            .get(index)
            .ok_or_else(|| EditError::no_element(part.to_owned()))?;
        let tree = self
            .sink
            .doc()
            .tree(part)?
            .ok_or_else(|| EditError::no_part(part.to_owned()))?;
        let element = at_path(tree.root(), &paragraph.path).ok_or_else(|| EditError::no_element(part.to_owned()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &paragraph.path));
        let root = work.root();
        let tracker = self.sink.watch_properties(&mut work, &[root]);
        let shared = self.sink.doc().shared_handle();
        change(&mut work, root, shared.facts.orders(), &self.colours)?;
        let wrote = self.sink.properties_changed(&mut work, tracker)?;
        let new = work.build().ok_or_else(|| EditError::unbuilt(part.to_owned()))?;
        if *new != **element {
            let draws = wrote
                .annotations
                .map(|n| annotation_draws(&new, n, 0))
                .unwrap_or_default();
            let mut keys = vec![block_key(part, &paragraph.id)];
            if paragraph_has_annotations(element) || paragraph_has_annotations(&new) {
                keys.push(Key::Annotations);
            }
            self.sink.replace_drawing(part, &paragraph.path, new, keys, draws)?;
            if wrote.dated {
                self.sink.ensure_w14(part, false, &[("w16du", W16DU)])?;
            }
        }
        if let RevisionPolicy::Tracked(tracking) = self.sink.policy().clone() {
            let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
            self.sink
                .people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
        }
        Ok(FormatStep {
            id: renamed_id(&renamed, &paragraph.id),
            changed: true,
            renamed,
            ..FormatStep::default()
        })
    }

    /// `op` on the paragraph `index` of `part`.
    pub(crate) fn paragraph(&mut self, part: &str, index: usize, op: &ParagraphOp<'_>) -> Result<FormatStep> {
        self.edit(part, index, |work, p, orders, colours| {
            let colour = |name: &str| colours.value(name);
            match op {
                ParagraphOp::Settings(paragraph, run) => {
                    write_paragraph(work, p, paragraph, orders);
                    if !run.is_empty() {
                        let found = runs(work, p);
                        for &r in &found {
                            write_run(work, r, run, &colour, orders);
                        }
                        if let Some(owner) = properties_of(work, p, "pPr", true, orders) {
                            write_run(work, owner, run, &colour, orders);
                            if work.children(owner).is_empty() {
                                work.remove(owner);
                            }
                        }
                        merge_runs(work, &found);
                    }
                }
                ParagraphOp::Clear => {
                    for r in runs(work, p) {
                        clear_run(work, r);
                    }
                    clear_paragraph(work, p);
                    let found = runs(work, p);
                    merge_runs(work, &found);
                }
                ParagraphOp::Style(style) => write_style(work, p, *style, orders),
                ParagraphOp::Like(look, parts) => take(work, p, look, parts, orders),
            }
            Ok(())
        })
    }

    /// `op` on the characters `[start, end)` of the paragraph `index` of `part`; `mark`:
    /// its mark too (a range going on past it).
    pub(crate) fn range(
        &mut self,
        part: &str,
        index: usize,
        (start, end, mark): (usize, usize, bool),
        op: &RangeOp<'_>,
    ) -> Result<FormatStep> {
        self.edit(part, index, |work, p, orders, colours| {
            let colour = |name: &str| colours.value(name);
            let apply = |work: &mut Work, owner: Id| match op {
                RangeOp::Settings(run) => write_run(work, owner, run, &colour, orders),
                RangeOp::Clear => clear_run(work, owner),
                RangeOp::Style(style) => character_style(work, owner, *style, orders),
            };
            let found = isolate(work, p, start, end)?;
            for &r in &found {
                apply(work, r);
            }
            if mark && op.marks() {
                if let Some(owner) = properties_of(work, p, "pPr", true, orders) {
                    apply(work, owner);
                    if work.children(owner).is_empty() {
                        work.remove(owner);
                    }
                }
            }
            merge_around(work, &found);
            prune(work, p);
            Ok(())
        })
    }
}

/// A run's character style as docx-agent's `set_character_style` writes it.
fn character_style(work: &mut Work, owner: Id, style: Option<&str>, orders: &Orders) {
    let Some(properties) = properties_of(work, owner, "rPr", style.is_some(), orders) else {
        return;
    };
    let value = style.map(|s| [("val", Some(s))]);
    set_child(work, properties, "rStyle", value.as_ref().map(|v| &v[..]), orders);
    if work.is(owner, W, "r") && work.children(properties).is_empty() {
        work.remove(properties);
    }
}
