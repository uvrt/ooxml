//! Owns: a numbered heading's entry in a table of contents (docx-agent's `_toc_numbered`,
//! `_toc_labels_read` and `_TocLooks`, round 32 R12, measured with Word's own update): the
//! number each body paragraph draws, counted as the layout counts lists (a text box's
//! paragraphs apart; a paragraph whose style's level is tied to another style draws none),
//! with what follows it ([`labels`]), and, where a tab follows it, the entry's left stop
//! after the number ([`NumberStops`]).
//!
//! **The stop after the number**: the number's end -- the line's start (left indent less
//! hanging, plus first line, of the level's `toc N` style) plus the number's advance in the
//! entry's face (the style's size, bold and italic; the Hyperlink style's face where the
//! entries link; capitals where either says so) -- plus [`NUMBER_GAP`]; the stop is the left
//! indent of the first of `toc 1` ... `toc 9`, in level order, at or past that (a style the
//! document lacks: [`LEVEL_STEP`] per level), else that point itself, rounded.
//!
//! Must not: measure fonts (the layout port says a character's advance), or write anything.

use super::select::Heading;
use crate::names::W;
use ooxml_word_model::docx::formatting::{EffectiveParagraph, EffectiveRun, Formatting};
use ooxml_word_model::docx::numbering::{membership, style_numbering, Lists};
use ooxml_word_model::docx::{find_path, Docx};
use ooxml_word_model::labels::Counters;
use ooxml_xml::Element;
use std::collections::HashMap;

use super::super::pages::LaidFields;

/// Between a numbered entry's number and its left stop, at least (twips, measured).
const NUMBER_GAP: f64 = 240.0;
/// A `toc N` style's left indent where the document lacks it: this per level past the
/// first (twips).
const LEVEL_STEP: f64 = 240.0;
/// A character's advance where no font says (of an em).
const UNKNOWN_ADVANCE: f64 = 0.5;

/// A numbered heading's number as its entry shows it.
pub(crate) struct Label {
    /// The number as the heading draws it.
    pub(crate) text: String,
    /// Whether a tab follows it (`w:suff` `tab`, Word's default); else a space or nothing,
    /// which the entry writes as a space.
    pub(crate) tab: bool,
}

/// Whether the paragraph draws a list number in a table of contents' eyes: in a list whose
/// level has a format, its own `w:numId` or its style's list.
fn listed(docx: &Docx, lists: &Lists, paragraph: &Element) -> Option<ooxml_word_model::docx::numbering::Membership> {
    let found = membership(docx, lists, paragraph)?;
    if found.format.as_deref().is_none_or(|f| f == "none") {
        return None;
    }
    let style = find_path(paragraph, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
    (!found.from_style || style_numbering(docx, style).0.is_some()).then_some(found)
}

/// Each heading's number (`None`: it draws none), in order; all `None` without counting
/// where no heading listed may be numbered (`_toc_numbered`).
pub(crate) fn labels(docx: &Docx, headings: &[Heading]) -> Vec<Option<Label>> {
    let lists = Lists::read(docx);
    if !headings
        .iter()
        .any(|h| listed(docx, &lists, &h.paragraph.element).is_some())
    {
        return headings.iter().map(|_| None).collect();
    }
    let Some(body) = docx.body() else {
        return headings.iter().map(|_| None).collect();
    };
    let formatting = Formatting::read(docx);
    let numbering = formatting.numbering();
    let root = body.root();
    let (mut outside, mut inside) = (Counters::default(), Counters::default());
    let mut found: HashMap<Vec<u32>, Label> = HashMap::new();
    for paragraph in body.ids.paragraphs().iter() {
        let Some(item) = listed(docx, &lists, &paragraph.element) else {
            continue;
        };
        let counters = if in_text_box(root, &paragraph.path) {
            &mut inside
        } else {
            &mut outside
        };
        let Some(text) = counters.item(numbering, item.num_id, item.level) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let tab = numbering
            .level(item.num_id, item.level)
            .and_then(|l| l.suffix.as_deref())
            .is_none_or(|s| s == "tab");
        found.insert(paragraph.path.to_vec(), Label { text, tab });
    }
    headings.iter().map(|h| found.remove(&*h.paragraph.path)).collect()
}

/// Whether the element at `path` of `root` is inside a `w:txbxContent`.
fn in_text_box(root: &Element, path: &[u32]) -> bool {
    let mut node = root;
    for k in path.iter().take(path.len().saturating_sub(1)) {
        let Some(next) = node.children().get(*k as usize).and_then(|n| n.as_element()) else {
            return false;
        };
        if next.name().is(Some(W), "txbxContent") {
            return true;
        }
        node = next;
    }
    false
}

/// A `toc N` style's look: its paragraph, a run in it, and that run in the Hyperlink style.
type Look = Option<(EffectiveParagraph, EffectiveRun, EffectiveRun)>;

/// The left stops after numbers, per level, measured through the layout port.
pub(crate) struct NumberStops<'a> {
    docx: &'a Docx,
    formatting: Formatting,
    link_style: Option<&'a str>,
    laid: &'a dyn LaidFields,
    looks: HashMap<i64, Look>,
}

impl<'a> NumberStops<'a> {
    /// The stops of `docx` (its `toc N` styles as they are now), the entries' text in
    /// `link_style` where they link.
    pub(crate) fn new(docx: &'a Docx, link_style: Option<&'a str>, laid: &'a dyn LaidFields) -> NumberStops<'a> {
        NumberStops {
            docx,
            formatting: Formatting::read(docx),
            link_style,
            laid,
            looks: HashMap::new(),
        }
    }

    fn look(&mut self, level: i64) -> &Look {
        let (docx, formatting, link_style) = (self.docx, &self.formatting, self.link_style);
        self.looks.entry(level).or_insert_with(|| {
            let style = docx.find_style(&format!("toc {level}"), "paragraph")?;
            let id = style.attribute(Some(W), "styleId").unwrap_or("");
            let own = formatting.style_look(Some(id), None, None);
            let linked = match link_style {
                Some(link) => formatting.style_look(Some(id), None, Some(link)).run,
                None => own.run.clone(),
            };
            Some((own.paragraph, own.run, linked))
        })
    }

    /// A level's left indent, twips.
    fn indent(&mut self, level: i64) -> f64 {
        match self.look(level) {
            Some((paragraph, ..)) => paragraph.indent_left * 20.0,
            None => (level - 1) as f64 * LEVEL_STEP,
        }
    }

    /// The left stop after `label` in a level `level` entry, twips.
    pub(crate) fn stop(&mut self, level: i64, label: &str) -> i64 {
        let (start, width) = match self.look(level).clone() {
            Some((paragraph, run, linked)) => (
                (paragraph.indent_left - paragraph.hanging + paragraph.first_line) * 20.0,
                self.width(label, &run, &linked),
            ),
            None => (0.0, label.chars().count() as f64 * 120.0),
        };
        let end = start + width + NUMBER_GAP;
        for other in 1..=9 {
            let indent = self.indent(other);
            if indent >= end {
                return indent.round_ties_even() as i64;
            }
        }
        end.round_ties_even() as i64
    }

    /// `text`'s advance in twips: the style's size, bold and italic; the face the Hyperlink
    /// style gives it (in a table of contents the Hyperlink style keeps only its fonts and
    /// capitals).
    fn width(&self, text: &str, run: &EffectiveRun, linked: &EffectiveRun) -> f64 {
        let named = |r: &EffectiveRun| r.font.clone().filter(|f| !f.is_empty());
        let face = named(linked)
            .or_else(|| named(run))
            .unwrap_or_else(|| "Calibri".to_owned());
        let capitals = run.caps || run.small_caps || linked.caps || linked.small_caps;
        let text = if capitals { text.to_uppercase() } else { text.to_owned() };
        let ems: f64 = text
            .chars()
            .map(|ch| {
                self.laid
                    .advance(&face, run.bold, run.italic, ch)
                    .filter(|a| a.units_per_em > 0)
                    .map_or(UNKNOWN_ADVANCE, |a| f64::from(a.advance) / f64::from(a.units_per_em))
            })
            .sum();
        ems * run.size * 20.0
    }
}
