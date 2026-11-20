//! Owns: `word_format_table`'s `width_of: "body"` (round 21, T5) as docx-agent's
//! `body_lines` and `_body_width` read it: where the body's lines run beside a table -- the
//! width it sits in (its section's text width, or its cell's) less the effective indents of
//! the paragraph around it (the nearest before it in its container, else after it, else the
//! default paragraph style's) -- and the table's width, indent and columns that put it on
//! those lines.
//!
//! Must not: write anything (the tool hands [`super::FormatTable`] the edits).

use super::{find_table, room_for, shape};
use crate::format::keeps::Keeps;
use crate::names::W;
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::{at_path, find_path};
use ooxml_xml::Element;
use std::sync::Arc;

/// Where the body's lines run beside a table, points (docx-agent's `body_lines`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyLines {
    /// The paragraph whose indents they are, by id (`None`: the default style's).
    pub paragraph: Option<String>,
    /// The text width the table sits in.
    pub text_width: f64,
    /// The left indent (a negative one runs the lines past the margin).
    pub left: f64,
    /// The right indent.
    pub right: f64,
    /// The lines' width.
    pub width: f64,
}

/// A table put on the body's lines (docx-agent's `_body_width` and the columns shared
/// again): its width and indent from the margin in points (aligned left), each grid column's
/// new width in points, and the lines.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyWidth {
    /// The table's width.
    pub width: f64,
    /// Its indent.
    pub indent: f64,
    /// Each grid column's width.
    pub columns: Vec<f64>,
    /// The lines it is put on.
    pub lines: BodyLines,
}

fn is_w(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

/// The nearest paragraph before the element at `path` among its siblings, else after it
/// (docx-agent's `_previous_block` / `_next_block`, past tables and controls): its place.
fn around(root: &Arc<Element>, path: &[u32]) -> Option<Vec<u32>> {
    let (&at, above) = path.split_last()?;
    let parent = at_path(root, above)?;
    let paragraph = |k: usize| {
        let node = parent.children().get(k)?.as_element()?;
        let k = u32::try_from(k).ok()?;
        is_w(node, "p").then(|| [above, &[k]].concat())
    };
    let at = at as usize;
    (0..at)
        .rev()
        .find_map(paragraph)
        .or_else(|| (at + 1..parent.children().len()).find_map(paragraph))
}

/// Where the body's lines run beside the table `table` names.
///
/// # Errors
/// A `table` that names no table.
pub fn body_lines(doc: &WordDoc, table: &str) -> Result<BodyLines> {
    let docx = doc.docx()?;
    let found = find_table(doc, &docx, table)?;
    let story = docx
        .part(&found.part)
        .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
    let mut look = Keeps::new(&docx, story);
    let placed = around(story.root(), &found.path).and_then(|place| Some((at_path(story.root(), &place)?, place)));
    let (effective, paragraph) = match placed {
        Some((element, place)) => (
            look.effective(element, &place, true),
            story.ids.paragraph_at(&place).map(|p| p.id),
        ),
        None => {
            // a paragraph with no formatting of its own: the default paragraph style's look
            let name = Arc::new(ooxml_xml::QName::new(Some("w"), "p", Some(W)));
            let empty = Element::new(name, Vec::new(), Vec::new(), Vec::new());
            (look.effective(&empty, &[], true), None)
        }
    };
    #[allow(clippy::cast_precision_loss)]
    let text_width = room_for(doc, &docx, &found) as f64 / 20.0;
    let (left, right) = (effective.indent_left, effective.indent_right);
    Ok(BodyLines {
        paragraph,
        text_width,
        left,
        right,
        width: text_width - left - right,
    })
}

/// `x` rounded to two decimals as Python's `round(x, 2)` rounds it (the exact value, ties
/// to even).
fn two_places(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

/// The table `table` names put on the body's lines (docx-agent's `_body_width`), `percent`
/// of their width, placed by `alignment` (else its own: left where it states none, `start`
/// or `both`); `None` where that comes to under a point (the lines' width said by the
/// caller).
///
/// # Errors
/// A `table` that names no table.
pub fn body_width(doc: &WordDoc, table: &str, percent: f64, alignment: Option<&str>) -> Result<Option<BodyWidth>> {
    let lines = body_lines(doc, table)?;
    let width = two_places(lines.width * percent / 100.0);
    if width < 1.0 {
        return Ok(None);
    }
    let docx = doc.docx()?;
    let found = find_table(doc, &docx, table)?;
    let current = find_path(&found.element, &["tblPr", "jc"]).and_then(|j| j.attribute(Some(W), "val"));
    let placed = alignment.unwrap_or(match current {
        None | Some("start" | "both") => "left",
        Some(other) => other,
    });
    let room = lines.width - width;
    let offset = match placed {
        "center" => room / 2.0,
        "right" => room,
        _ => 0.0,
    };
    let work = Work::of(&found.element, Vec::new());
    let grid = shape::grid_widths(&work, work.root());
    #[allow(clippy::cast_precision_loss)]
    let columns = shape::share(super::props::twips(width), &grid)
        .into_iter()
        .map(|w| w as f64 / 20.0)
        .collect();
    Ok(Some(BodyWidth {
        width,
        indent: two_places(lines.left + offset),
        columns,
        lines,
    }))
}
