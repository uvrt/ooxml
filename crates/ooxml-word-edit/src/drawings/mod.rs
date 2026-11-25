//! Owns: drawings edited as docx-agent's `edit/drawings.py` and `edit/pictures.py` edit
//! them (as Word 16.106 writes each): a drawing floated ([`FloatDrawing`]), put in the line
//! again ([`InlineDrawing`]), its place, wrapping, order, size and alternative text set
//! ([`SetDrawing`]), a text box or a shape inserted ([`InsertShape`]), a chart inserted
//! ([`InsertChart`]), the parts behind a
//! chart edited ([`EditGraphic`]) -- and what they share:
//! the drawing a `d:` id names, Word's measured constants, the document's z-order heights,
//! VML lengths.
//!
//! Word tracks none of the frame edits (they are applied untracked, and the result says
//! so); a new text box or shape is an inserted run.
//!
//! Must not: write but through the sink, or read a part an edit does not need but for the
//! z-order heights every story holds.

mod chart;
mod frame;
mod graphic;
mod picture;
mod settings;
mod shape;

pub use chart::{ChartInserted, ChartParts, InsertChart};
pub use frame::{FloatDrawing, InlineDrawing, SetDrawing};
pub use graphic::{EditGraphic, GraphicEdit, GraphicEdited, GraphicParts, PartWrite, RecordChange};
pub use picture::{image_format, picture_size, written_size, InsertPicture};
pub use settings::{DrawingValues, ZOrder};
pub use shape::InsertShape;

use crate::controls::{at_place, rewrite, InParagraph, Rewrite};
pub(crate) use crate::imported::python_int;
use crate::names::WP;
use crate::sink::{EditSink, TextWrite};
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::address::AddressError;
use ooxml_word_model::docx::drawings::{all_drawings, pictures, Drawing};
use ooxml_word_model::docx::Docx;

/// EMU per point.
pub(crate) const EMU: f64 = 12700.0;
/// Word's first `relativeHeight` and its step (measured: 251658240, then +1024 each).
pub(crate) const FIRST_HEIGHT: i64 = 251_658_240;
pub(crate) const HEIGHT_STEP: i64 = 1024;
/// Word's default horizontal distance of a floating drawing from the text (9 pt).
pub(crate) const SIDE_DISTANCE: i64 = 114_300;
/// Word's tight and through polygon round a rectangular frame (measured).
pub(crate) const POLYGON: [(i64, i64); 5] = [(0, 0), (0, 21000), (21300, 21000), (21300, 0), (0, 0)];
/// Word's 2010 shapes.
pub(crate) const WPS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";
/// DrawingML's main namespace.
pub(crate) const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// VML.
pub(crate) const V: &str = "urn:schemas-microsoft-com:vml";
/// VML's Word extensions.
pub(crate) const W10: &str = "urn:schemas-microsoft-com:office:word";

/// What a drawing edit did, for the tool's reply.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrawingEdited {
    /// The drawing (a new one's id).
    pub id: String,
    /// Whether it changed anything (a drawing already as asked is no change).
    pub changed: bool,
    /// What it made: the drawing, a text box's paragraphs.
    pub created: Vec<String>,
    /// Ids the stamping gave.
    pub renamed: ooxml_word_model::ids::Renames,
    /// What the edit says (applied untracked, already as asked).
    pub warnings: Vec<String>,
}

/// A length in points as EMU (docx-agent's `emu`: rounded half to even).
pub(crate) fn emu(points: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let value = (points * EMU).round_ties_even() as i64;
    value
}

/// A VML length as Word writes one (docx-agent's `_points_text`): whole inches in inches
/// (144 pt is `2in`), the rest in points with at most two decimals.
pub(crate) fn points_text(value: f64) -> String {
    let fixed = format!("{value:.2}");
    let text = fixed.trim_end_matches('0').trim_end_matches('.');
    if matches!(text, "" | "-0" | "0") {
        return "0".to_owned();
    }
    let inches = value / 72.0;
    if value.abs() >= 72.0 && (inches - inches.round()).abs() < 1e-9 {
        #[allow(clippy::cast_possible_truncation)]
        let whole = inches.round() as i64;
        return format!("{whole}in");
    }
    format!("{text}pt")
}

/// The drawing `id` names (docx-agent's `_frame`: every drawing with a `docPr` id, a
/// picture under its picture id); refused as docx-agent refuses one that is not there.
pub(crate) fn located(docx: &Docx, id: &str) -> Result<Drawing> {
    all_drawings(docx)
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| EditError::Refused(format!("no drawing {}", ooxml_base::pyfmt::repr_str(id))))
}

/// The picture `id` names (docx-agent's `picture`): `not_found` for any other.
pub(crate) fn picture(docx: &Docx, id: &str) -> Result<Drawing> {
    pictures(docx)
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| EditError::Address(AddressError::not_found(format!("no picture {id}"), Vec::new())))
}

/// Every floating drawing's `relativeHeight` in the document (docx-agent's `all_heights`);
/// reads every story.
pub(crate) fn all_heights(sink: &mut EditSink<'_, '_>) -> Result<Vec<i64>> {
    sink.read_everything();
    let docx = sink.docx()?;
    Ok(all_drawings(&docx)
        .iter()
        .filter(|d| d.element.name().is(Some(WP), "anchor"))
        .filter_map(|d| d.element.attribute(None, "relativeHeight").and_then(python_int))
        .collect())
}

/// A step above every drawing's height (Word's first when there is none).
pub(crate) fn height_above(heights: &[i64]) -> i64 {
    heights.iter().copied().max().unwrap_or(FIRST_HEIGHT - HEIGHT_STEP) + HEIGHT_STEP
}

/// Rewrite the paragraph holding `drawing`'s frame, `write` given the frame in it.
pub(crate) fn edit_frame<R>(
    sink: &mut EditSink<'_, '_>,
    drawing: &Drawing,
    how: Rewrite,
    write: impl FnOnce(&mut EditSink<'_, '_>, &mut Work, Id) -> Result<R>,
) -> Result<R> {
    let docx = sink.docx()?;
    let at = InParagraph::above(&docx, &drawing.part, &drawing.path)
        .ok_or_else(|| EditError::no_element(drawing.part.clone()))?;
    let inside = drawing.path.get(at.path.len()..).unwrap_or_default().to_vec();
    rewrite(sink, &at, how, |sink, work| {
        let frame = at_place(work, &inside).ok_or_else(|| EditError::no_element(at.part.clone()))?;
        let out = write(sink, work, frame)?;
        Ok((
            out,
            TextWrite {
                changed: true,
                ..TextWrite::default()
            },
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vml_lengths_as_word_writes_them() {
        assert_eq!(points_text(144.0), "2in");
        assert_eq!(points_text(72.0), "1in");
        assert_eq!(points_text(0.0), "0");
        assert_eq!(points_text(-0.001), "0");
        assert_eq!(points_text(9.0), "9pt");
        assert_eq!(points_text(-30.5), "-30.5pt");
        assert_eq!(points_text(12.345_67), "12.35pt");
    }

    #[test]
    fn emu_rounds_half_to_even() {
        assert_eq!(emu(1.0), 12700);
        assert_eq!(emu(-30.5), -387_350);
    }
}
