//! Owns: [`FormatEdit`], `word_format`'s edits on its targets as docx-agent's tool makes
//! them (`tools/text.py`, `word_format`; `edit/formatops.py`): each target's paragraphs
//! given a sample's look first, then per target its keeps (a paragraph's lines, or a span's
//! blocks on one page), its direct formatting cleared, its paragraph style, then a range's
//! character style and run formatting (a paragraph's or span's: on every paragraph and
//! run) -- each paragraph's edit a step, stamping once, the edit's end (text boxes synced,
//! revision ids made unique) once after the last; and the outline before and after where
//! a heading's level may change.
//!
//! Must not: write but through the sink, or know a tool's JSON. Every call reads the
//! whole document (a list a look leaves unused is removed when no paragraph names it).

mod colour;
mod edit;
mod found;
pub(crate) mod keeps;
mod like;
mod outline;
pub(crate) mod values;

pub(crate) use colour::ThemeColours;
pub use found::{occurrences, range_text, Occurrence};
pub use keeps::{body_blocks, keep_writes, kept_blocks, kept_package};
pub use like::{differs, ParagraphLook, NOT_TAKEN, PARTS};
pub use values::{check, theme_color_name, Property, Setting, Value};
pub(crate) use values::{properties_of, set_child, write_paragraph, write_run};

use crate::lists::paragraphs_of;
use crate::policy::RevisionPolicy;
use crate::text::atoms;
use crate::text_range::{range, Span};
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use edit::{ParagraphOp, RangeOp, Run};
use ooxml_base::pyfmt::repr_float;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::{is_text_range, resolve, span_ends};
use ooxml_word_model::docx::numbering::{membership, Lists};
use ooxml_word_model::docx::outline::Heading;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use std::collections::{BTreeMap, BTreeSet};

/// A sample's look given to the targets, less the parts the call sets itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Like {
    /// The look.
    pub look: ParagraphLook,
    /// Parts of [`PARTS`] not taken.
    pub skip: Vec<&'static str>,
}

/// What `word_format` sets on each target.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatPlan {
    /// A sample's look.
    pub like: Option<Like>,
    /// Keep each target whole on one page (or not).
    pub keep_together: Option<bool>,
    /// Remove direct formatting first.
    pub clear_direct: bool,
    /// The paragraph style's name.
    pub paragraph_style: Option<String>,
    /// The character style's name.
    pub character_style: Option<String>,
    /// Paragraph settings, in order.
    pub paragraph: Vec<Setting>,
    /// Run settings, in order.
    pub run: Vec<Setting>,
    /// The targets kept together that the layout before the change draws taller than a
    /// page, by address: their height and the least room a page gives the body, in points
    /// (the host measures them; the keep says so).
    pub taller: BTreeMap<String, (f64, f64)>,
}

impl FormatPlan {
    /// Whether a heading's level may change (docx-agent reads the outline before then).
    fn moves_outline(&self) -> bool {
        self.paragraph_style.is_some() || self.clear_direct || self.like.is_some()
    }
}

/// `word_format`'s edits.
#[derive(Debug, Clone)]
pub struct FormatEdit {
    /// The targets: paragraphs, ranges, spans, tables, cells, sections.
    pub targets: Vec<String>,
    /// What to set.
    pub plan: FormatPlan,
    /// Whether `clear` was given (docx-agent reads the outline then).
    pub cleared: bool,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What one edit did (docx-agent's `EditResult`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormatStep {
    /// The block's id after the edit.
    pub id: String,
    /// Whether it changed anything.
    pub changed: bool,
    /// Ids its stamping renamed, old to new.
    pub renamed: Vec<(String, String)>,
    /// What it removed (`num:3`).
    pub removed: Vec<String>,
    /// What it notes.
    pub warnings: Vec<String>,
}

/// What the look did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LikeDone {
    /// Each paragraph that took something, with the parts it took.
    pub set: Vec<(String, Vec<String>)>,
    /// Paragraphs already alike (and the sample).
    pub alike: Vec<String>,
}

/// What a [`FormatEdit`] did.
#[derive(Debug, Clone, Default)]
pub struct Formatted {
    /// Each edit, in order.
    pub steps: Vec<FormatStep>,
    /// The look's, when one was given.
    pub like: Option<LikeDone>,
    /// The body's headings before and after, when a level may have changed.
    pub outline: Option<(Vec<Heading>, Vec<Heading>)>,
}

/// Whether `keep_together` keeps `address`'s blocks on one page rather than one
/// paragraph's lines (docx-agent's `_keeps_blocks`).
#[must_use]
pub fn keeps_blocks(doc: &WordDoc, docx: &Docx, address: &str) -> bool {
    if is_text_range(address) || address.contains('/') {
        return false;
    }
    if !matches!(span_ends(address), Ok(None)) {
        return true;
    }
    let followed = doc.ids().follow(address);
    !matches!(resolve(docx, &followed), Ok((_, Entry::Paragraph(_))))
}

impl Command<WordDoc> for FormatEdit {
    type Out = Formatted;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        check(&self.plan.paragraph)?;
        check(&self.plan.run)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Formatted> {
        let mut sink = crate::sink::EditSink::new(tx, self.policy.clone());
        sink.read_everything();
        let docx = sink.docx()?;
        let kept: Vec<bool> = self
            .targets
            .iter()
            .map(|a| self.plan.keep_together.is_some() && keeps_blocks(sink.doc(), &docx, a))
            .collect();
        let before = (self.plan.moves_outline() || self.cleared).then(|| outline::Outline::read(&docx));
        let colours = colour::ThemeColours::read(sink.doc(), &docx)?;
        let mut run = Run::new(&mut sink, colours);
        let mut out = Formatted::default();
        if let Some(like) = &self.plan.like {
            out.like = Some(like_targets(&mut run, &self.targets, like, &mut out.steps)?);
        }
        for (address, kept) in self.targets.iter().zip(kept) {
            target(&mut run, &self.plan, address, kept, &mut out.steps)?;
        }
        if out.steps.iter().any(|s| s.changed) {
            crate::textbox::sync(&mut sink)?;
            crate::revision_ids::make_unique(&mut sink)?;
        }
        if let Some(before) = before {
            out.outline = Some((before.headings(), before.after(&*sink.docx()?)));
        }
        Ok(out)
    }
}

/// The style `name` names for `kind` (a built-in one Word would add counts), or the
/// refusal docx-agent's `Styles.resolve` gives.
///
/// # Errors
/// The style is another kind's, or neither the document nor Word has it.
pub fn known_style(docx: &Docx, name: &str, kind: &str) -> Result<String> {
    crate::builtin::resolve(docx, name, kind)
}

/// The paragraphs `address` holds now, as `(part, index)`.
fn places(run: &Run<'_, '_, '_>, address: &str) -> Result<Vec<(String, usize)>> {
    let docx = run.sink.docx()?;
    if is_text_range(address) {
        // a range's ends followed where stamping renamed them (`Document.range`)
        let span = Span::of(run.sink.doc(), &docx, &range(run.sink.doc(), &docx, address)?)?;
        let story = docx
            .part(&span.part)
            .ok_or_else(|| EditError::NoPart(span.part.clone()))?;
        return Ok(span
            .paragraphs
            .iter()
            .filter_map(|p| story.ids.paragraph_at(&p.path).map(|q| (span.part.clone(), q.index)))
            .collect());
    }
    paragraphs_of(run.sink.doc(), &docx, address)
}

/// One target's edits, in docx-agent's order.
fn target(
    run: &mut Run<'_, '_, '_>,
    plan: &FormatPlan,
    address: &str,
    kept: bool,
    steps: &mut Vec<FormatStep>,
) -> Result<()> {
    let is_range = address.contains('@') && !address.starts_with("p@");
    let paragraphs = places(run, address)?;
    if let Some(on) = plan.keep_together {
        if kept {
            let mut step = keep_blocks(run, address, on)?;
            if let (true, Some((height, room))) = (on, plan.taller.get(address)) {
                step.warnings.push(format!(
                    "{address} is {} pt tall and a page holds {} pt: it cannot be kept on one page (Word starts \
                     it on a new page and breaks it where the page is full); keep shorter spans together",
                    repr_float(*height),
                    repr_float(*room)
                ));
            }
            steps.push(step);
        } else {
            let keep = [Setting {
                property: Property::KeepTogether,
                value: Some(Value::Flag(on)),
            }];
            for (part, k) in &paragraphs {
                steps.push(run.paragraph(part, *k, &ParagraphOp::Settings(&keep, &[]))?);
            }
        }
    }
    if plan.clear_direct {
        if is_range {
            steps.push(on_range(run, address, &RangeOp::Clear)?);
        } else {
            for (part, k) in &paragraphs {
                steps.push(run.paragraph(part, *k, &ParagraphOp::Clear)?);
            }
        }
    }
    if let Some(name) = &plan.paragraph_style {
        let id = if paragraphs.is_empty() {
            None
        } else {
            paragraph_style(run, name)?
        };
        for (part, k) in &paragraphs {
            steps.push(run.paragraph(part, *k, &ParagraphOp::Style(id.as_deref()))?);
        }
    }
    if is_range {
        // docx-agent reads the range here, whatever it then sets on it
        let docx = run.sink.docx()?;
        range(run.sink.doc(), &docx, address)?;
        if let Some(name) = &plan.character_style {
            let id = character_style(run, name)?;
            steps.push(on_range(run, address, &RangeOp::Style(id.as_deref()))?);
        }
        if !plan.run.is_empty() {
            steps.push(on_range(run, address, &RangeOp::Settings(&plan.run))?);
        }
        if !plan.paragraph.is_empty() {
            for (part, k) in &paragraphs {
                steps.push(run.paragraph(part, *k, &ParagraphOp::Settings(&plan.paragraph, &[]))?);
            }
        }
        return Ok(());
    }
    if let Some(name) = &plan.character_style {
        for (part, k) in &paragraphs {
            let length = paragraph_length(run, part, *k)?;
            if length > 0 {
                let id = character_style(run, name)?;
                steps.push(run.range(part, *k, (0, length, false), &RangeOp::Style(id.as_deref()))?);
            }
        }
    }
    if !plan.run.is_empty() || !plan.paragraph.is_empty() {
        for (part, k) in &paragraphs {
            steps.push(run.paragraph(part, *k, &ParagraphOp::Settings(&plan.paragraph, &plan.run))?);
        }
    }
    Ok(())
}

fn paragraph_length(run: &Run<'_, '_, '_>, part: &str, index: usize) -> Result<usize> {
    let docx = run.sink.docx()?;
    let paragraph = docx
        .part(part)
        .and_then(|s| s.ids.paragraphs().get(index))
        .ok_or_else(|| EditError::Path(part.to_owned()))?;
    let work = Work::of(&paragraph.element, Vec::new());
    Ok(crate::text::paragraph_text(&work, work.root()).chars().count())
}

/// The paragraph style `name` names (a built-in one the document lacks added); `None` for
/// the default style.
fn paragraph_style(run: &mut Run<'_, '_, '_>, name: &str) -> Result<Option<String>> {
    let id = crate::builtin::ensure(run.sink, name, "paragraph")?;
    let docx = run.sink.docx()?;
    let default = docx
        .default_style("paragraph")
        .map(|s| ooxml_word_model::docx::style_id(s).to_owned());
    Ok((Some(&id) != default.as_ref()).then_some(id))
}

/// The character style `name` names; `None` for the default one.
fn character_style(run: &mut Run<'_, '_, '_>, name: &str) -> Result<Option<String>> {
    let id = crate::builtin::ensure(run.sink, name, "character")?;
    let docx = run.sink.docx()?;
    let default = docx
        .default_style("character")
        .map(|s| ooxml_word_model::docx::style_id(s).to_owned());
    Ok((Some(&id) != default.as_ref()).then_some(id))
}

/// `op` on the range `address` names: each paragraph's part of it, checked before any
/// changes (docx-agent's `_format_range`); the step of its first paragraph.
fn on_range(run: &mut Run<'_, '_, '_>, address: &str, op: &RangeOp<'_>) -> Result<FormatStep> {
    let docx = run.sink.docx()?;
    let found = range(run.sink.doc(), &docx, address)?;
    let span = Span::of(run.sink.doc(), &docx, &found)?;
    let id = crate::text_range::range_id(&found);
    let story = docx
        .part(&span.part)
        .ok_or_else(|| EditError::NoPart(span.part.clone()))?;
    let last = span.paragraphs.len().saturating_sub(1);
    let mut pieces = Vec::new();
    for (k, paragraph) in span.paragraphs.iter().enumerate() {
        let work = Work::of(&paragraph.element, Vec::new());
        let items = atoms(&work, work.root());
        let start = if k == 0 { span.start } else { 0 };
        let end = if k == last { span.end } else { items.len() };
        if !(start <= end && end <= items.len()) {
            return Err(EditError::Refused(format!("{id} is outside its paragraphs' text")));
        }
        crate::inline::check_span(&work, &items, start, end, false)
            .map_err(|e| EditError::Refused(format!("{id}: {}", e.to_string().trim_start_matches("span: "))))?;
        let index = story
            .ids
            .paragraph_at(&paragraph.path)
            .map(|p| p.index)
            .ok_or_else(|| EditError::Path(span.part.clone()))?;
        pieces.push((index, start, end, k < last));
    }
    let part = span.part.clone();
    let mut first = None;
    for (index, start, end, mark) in pieces {
        let step = run.range(&part, index, (start, end, mark), op)?;
        first.get_or_insert(step);
    }
    first.ok_or_else(|| EditError::Refused(format!("{id} holds no paragraphs")))
}

/// What a refusal to keep blocks together says they are (docx-agent's `_keep_blocks`).
pub const KEEPS_BLOCKS: &str = "keep_together keeps the body's blocks: give a span p:A..t:B, a table or a section";

/// Keep `address`'s blocks on one page, or release them (docx-agent's
/// `keep_blocks_together`): one step, a no-op said in its warnings.
fn keep_blocks(run: &mut Run<'_, '_, '_>, address: &str, on: bool) -> Result<FormatStep> {
    let docx = run.sink.docx()?;
    let plan = keeps::keep_plan(run.sink.doc(), &docx, address, on).map_err(|e| {
        let said = match e {
            EditError::Refused(m) => m,
            EditError::Address(a) => a.message,
            other => other.to_string(),
        };
        EditError::Address(ooxml_word_model::address::AddressError::invalid(
            format!("{address}: {said}; {KEEPS_BLOCKS}"),
            Vec::new(),
        ))
    })?;
    let mut warnings = plan.warnings;
    if plan.changes.is_empty() {
        warnings.push(if on {
            format!("{address} was already kept together")
        } else {
            format!("{address} was not kept together")
        });
        return Ok(FormatStep {
            id: plan.first,
            warnings,
            ..FormatStep::default()
        });
    }
    let renamed = run.stamp()?;
    let main = docx.main.clone();
    for (path, settings) in &plan.changes {
        let docx = run.sink.docx()?;
        let index = docx
            .part(&main)
            .and_then(|s| s.ids.paragraph_at(path))
            .map(|p| p.index)
            .ok_or_else(|| EditError::Path(main.clone()))?;
        run.paragraph(&main, index, &ParagraphOp::Settings(settings, &[]))?;
    }
    Ok(FormatStep {
        id: crate::ranges::renamed_id(&renamed, &plan.first),
        changed: true,
        renamed,
        warnings,
        ..FormatStep::default()
    })
}

/// Each target paragraph given the look (docx-agent's `_like_targets`): the sample itself
/// and paragraphs already alike listed, not changed.
fn like_targets(
    run: &mut Run<'_, '_, '_>,
    targets: &[String],
    like: &Like,
    steps: &mut Vec<FormatStep>,
) -> Result<LikeDone> {
    let mut done = LikeDone::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for address in targets {
        // the ids as listed before this target's edits (docx-agent's `paragraphs_of`)
        let listed: Vec<(String, usize, String)> = {
            let docx = run.sink.docx()?;
            places(run, address)?
                .into_iter()
                .filter_map(|(part, k)| {
                    let id = docx.part(&part)?.ids.paragraphs().get(k)?.id;
                    Some((part, k, id))
                })
                .collect()
        };
        for (part, k, listed_id) in listed {
            if !seen.insert(listed_id.clone()) {
                continue;
            }
            if listed_id == like.look.sample {
                done.alike.push(listed_id);
                continue;
            }
            let docx = run.sink.docx()?;
            let paragraph = docx
                .part(&part)
                .and_then(|s| s.ids.paragraphs().get(k))
                .ok_or_else(|| EditError::Path(part.clone()))?;
            let parts = differs(&paragraph.element, &like.look, &like.skip);
            if parts.is_empty() {
                done.alike.push(paragraph.id.clone());
                steps.push(FormatStep {
                    id: paragraph.id,
                    ..FormatStep::default()
                });
                continue;
            }
            let before = membership(&docx, &Lists::read(&docx), &paragraph.element);
            let mut step = run.paragraph(&part, k, &ParagraphOp::Like(&like.look, &parts))?;
            if let Some(before) = before.filter(|_| parts.contains(&"list")) {
                let main = docx.main.clone();
                step.removed = crate::lists::reap(run.sink, &main, &BTreeSet::from([before.num_id]))?;
            }
            done.set
                .push((step.id.clone(), parts.iter().map(|p| (*p).to_owned()).collect()));
            steps.push(step);
        }
    }
    Ok(done)
}
