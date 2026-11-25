//! Owns: [`InsertPicture`], an inline picture inserted at a position as docx-agent's
//! `insert_picture` writes one (as Word writes an inline picture): a `wp:inline` with its
//! extent, a `docPr` numbered past every drawing the document has had, `wp14:anchorId` and
//! `wp14:editId` drawn from `{part}\0picture`, and a `pic:pic` stretching the picture's
//! blip over it, in a run of its own formatted as text there would be; the picture's bytes
//! a media part (one holding the same bytes reused); its size the natural one (pixels at
//! the density the file states, else 96 to the inch) or a width keeping the aspect ratio.
//! Tracked, the run is an insertion. And what a caller checks before: the picture's format
//! and size ([`picture_size`]).
//!
//! Must not: write but through the sink; draw an SVG (its PNG fallback is a later step:
//! the caller leaves an SVG to the host).

use super::{emu, DrawingEdited, A, EMU};
use crate::controls::{rewrite, InParagraph, Rewrite};
use crate::names::{W, WP, WP14};
use crate::policy::RevisionPolicy;
use crate::ranges::renamed_id;
use crate::sink::{EditSink, TextWrite};
use crate::stamping::stamp_document;
use crate::work::scope_above;
use crate::{EditError, Result, WordDoc};
use ooxml_base::Limits;
use ooxml_store::{Command, Txn};
use ooxml_word_model::imagemeta::{pixels_and_density, states_density};
use std::sync::Arc;

/// DrawingML's pictures.
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
/// Relationships.
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Word's density for a picture that states none (pixels per inch).
const DEFAULT_DPI: f64 = 96.0;

/// An inline picture inserted at a position.
#[derive(Debug, Clone)]
pub struct InsertPicture {
    /// Where: a position, or a range (at its start).
    pub at: ooxml_word_model::address::Address,
    /// The picture's bytes: PNG, JPEG, GIF or BMP.
    pub image: Arc<[u8]>,
    /// Points; the height keeps the aspect ratio (the natural size without one).
    pub width: Option<f64>,
    /// Its text for screen readers (`descr`).
    pub alt_text: Option<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// A picture's extension and content type by its signature (docx-agent's `image_format`).
///
/// # Errors
/// A refusal for any other format.
pub fn image_format(data: &[u8]) -> Result<(&'static str, &'static str)> {
    const FORMATS: [(&[u8], &str, &str); 5] = [
        (b"\x89PNG\r\n\x1a\n", "png", "image/png"),
        (b"\xff\xd8", "jpeg", "image/jpeg"),
        (b"GIF87a", "gif", "image/gif"),
        (b"GIF89a", "gif", "image/gif"),
        (b"BM", "bmp", "image/bmp"),
    ];
    FORMATS
        .iter()
        .find(|(signature, _, _)| data.starts_with(signature))
        .map(|(_, extension, kind)| (*extension, *kind))
        .ok_or_else(|| EditError::Refused("not a PNG, JPEG, GIF or BMP picture".to_owned()))
}

/// A picture's natural size in points (docx-agent's `natural_size`): its pixels at the
/// density it states in whole dots an inch (as Word takes a density in pixels per metre),
/// else at 96 dpi.
///
/// # Errors
/// A refusal when its size cannot be read.
pub fn natural_size(data: &[u8]) -> Result<(f64, f64)> {
    let unread = || EditError::Refused("the picture's size cannot be read".to_owned());
    let (width, height, mut dx, mut dy) = pixels_and_density(data).ok_or_else(unread)?;
    if width == 0 || height == 0 {
        return Err(unread());
    }
    if !states_density(data) {
        (dx, dy) = (DEFAULT_DPI, DEFAULT_DPI);
    }
    let whole = |d: f64| {
        let d = d.round_ties_even();
        if d == 0.0 {
            DEFAULT_DPI
        } else {
            d
        }
    };
    Ok((
        f64::from(width) * 72.0 / whole(dx),
        f64::from(height) * 72.0 / whole(dy),
    ))
}

/// The size a picture of `data` takes with `width` (docx-agent's `_fit`): its natural size,
/// or the width with the height keeping the aspect ratio; refused for a format other than
/// PNG, JPEG, GIF or BMP or a size that cannot be read.
///
/// # Errors
/// As [`image_format`] and [`natural_size`].
pub fn picture_size(data: &[u8], width: Option<f64>) -> Result<(f64, f64)> {
    image_format(data)?;
    let natural = natural_size(data)?;
    Ok(match width {
        Some(w) => (w, natural.1 * w / natural.0),
        None => natural,
    })
}

/// `text` as an XML attribute's value, double-quoted (`xml.sax.saxutils.quoteattr`'s
/// escapes: a line break or tab kept as a character reference).
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What the drawing's XML takes.
struct Frame<'a> {
    rid: &'a str,
    number: u64,
    size: (f64, f64),
    anchor_id: &'a str,
    edit_id: &'a str,
}

/// Whether `at` names a table cell (docx-agent's `_CELL`: `t:X/c<row>,<column>`).
fn is_cell(at: &str) -> bool {
    let Some((table, cell)) = at.rsplit_once("/c") else {
        return false;
    };
    let numbers = cell.split_once(',').is_some_and(|(r, c)| {
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        digits(r) && digits(c)
    });
    (table.starts_with("t:") || table.starts_with("t@")) && table.len() > 2 && numbers
}

/// Where the picture goes: `at`'s paragraph and offset, a cell's first paragraph at its
/// start.
fn place(
    doc: &WordDoc,
    docx: &ooxml_word_model::docx::Docx,
    at: &ooxml_word_model::address::Address,
) -> Result<crate::fields::Place> {
    let text = at.as_str();
    if is_cell(text) {
        let found =
            crate::set_text::cell_paragraph(docx, text).map_err(|error| no_cell(doc, docx, text).unwrap_or(error))?;
        return crate::fields::place(
            doc,
            docx,
            &ooxml_word_model::address::Address::from(format!("{}@0", found.id)),
            true,
        );
    }
    crate::fields::place(doc, docx, at, true)
}

/// The refusal docx-agent's `Table.cell` gives for a cell its table does not have (`None`:
/// no such table).
fn no_cell(doc: &WordDoc, docx: &ooxml_word_model::docx::Docx, at: &str) -> Option<EditError> {
    let (table, cell) = at.rsplit_once("/c")?;
    let (row, column) = cell.split_once(',')?;
    let (row, column) = (row.parse().ok()?, column.parse().ok()?);
    let (story, found) = crate::set_cells::found_table(doc, docx, table).ok()?;
    let grid = ooxml_word_model::docx::tables::TableGrid::new(story, &found);
    Some(EditError::Address(grid.no_cell(&found.id, row, column)))
}

impl InsertPicture {
    /// The `w:drawing` Word writes for an inline picture (docx-agent's `_drawing`).
    fn xml(&self, frame: &Frame<'_>) -> String {
        let (cx, cy) = (emu(frame.size.0), emu(frame.size.1));
        let name = quoted(&format!("Picture {}", frame.number));
        let descr = match self.alt_text.as_deref() {
            Some(alt) if !alt.is_empty() => format!(" descr={}", quoted(alt)),
            _ => String::new(),
        };
        let (number, rid, a, e) = (frame.number, frame.rid, frame.anchor_id, frame.edit_id);
        format!(
            "<w:drawing xmlns:w=\"{W}\"><wp:inline xmlns:wp=\"{WP}\" xmlns:wp14=\"{WP14}\" distT=\"0\" distB=\"0\" \
             distL=\"0\" distR=\"0\" wp14:anchorId=\"{a}\" wp14:editId=\"{e}\">\
             <wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
             <wp:docPr id=\"{number}\" name={name}{descr}/>\
             <wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a=\"{A}\" noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
             <a:graphic xmlns:a=\"{A}\"><a:graphicData uri=\"{PIC}\">\
             <pic:pic xmlns:pic=\"{PIC}\"><pic:nvPicPr><pic:cNvPr id=\"{number}\" name={name}{descr}/>\
             <pic:cNvPicPr><a:picLocks noChangeAspect=\"1\" noChangeArrowheads=\"1\"/></pic:cNvPicPr></pic:nvPicPr>\
             <pic:blipFill><a:blip xmlns:r=\"{R}\" r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
             <pic:spPr bwMode=\"auto\"><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
             <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
             </a:graphicData></a:graphic></wp:inline></w:drawing>"
        )
    }

    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<DrawingEdited> {
        let size = picture_size(&self.image, self.width)?;
        let format = image_format(&self.image)?;
        let docx = sink.docx()?;
        let place = place(sink.doc(), &docx, &self.at)?;
        let renamed = stamp_document(sink, &docx)?;
        let docx = sink.docx()?;
        let id = renamed_id(&renamed, &place.id);
        let found = crate::set_text::find(sink.doc(), &docx, &id)?;
        let part = found.part.clone();
        let rid = sink.media(&part, &self.image, format)?;
        let number = sink.draw_drawings(1)?.first().copied().unwrap_or(1);
        let ids = sink.draw(&format!("{part}\0picture"), 2)?;
        let frame = Frame {
            rid: &rid,
            number,
            size,
            anchor_id: ids.first().map_or("", String::as_str),
            edit_id: ids.get(1).map_or("", String::as_str),
        };
        let parsed = ooxml_xml::parse(self.xml(&frame).as_bytes(), &Limits::default().xml)
            .map_err(|e| EditError::Refused(e.to_string()))?;
        let tree = sink
            .doc()
            .tree(&part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let mut scope = scope_above(tree.root(), &found.path);
        scope.extend_from_slice(found.element.namespaces());
        let drawing = crate::namespaces::strip_redundant(parsed.root(), &mut scope);
        let at = InParagraph {
            part: part.clone(),
            path: found.path.clone(),
            id: found.id.clone(),
        };
        let how = Rewrite {
            renew: true,
            ensure_w14: false,
        };
        rewrite(sink, &at, how, |sink, work| {
            let drawing = work
                .adopt(&drawing)
                .ok_or_else(|| EditError::no_element(part.clone()))?;
            let run = crate::inline::new_run(work, work.root(), place.offset, &[drawing])?;
            let wrote = sink.track(work, |work, stamp, _| {
                if let (Some(parent), Some(index)) = (work.parent(run), work.index(run)) {
                    work.remove(run);
                    crate::track::insert_nodes(work, parent, index, &[run], stamp);
                }
                Vec::new()
            })?;
            Ok(((), TextWrite { changed: true, ..wrote }))
        })?;
        sink.ensure_w14(&part, true, &[])?;
        Ok(DrawingEdited {
            id: format!("d:{number}"),
            changed: true,
            created: vec![format!("d:{number}")],
            renamed,
            warnings: Vec::new(),
        })
    }
}

impl Command<WordDoc> for InsertPicture {
    type Out = DrawingEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        picture_size(&self.image, self.width)?;
        let docx = doc.docx()?;
        place(doc, &docx, &self.at).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<DrawingEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// A size in points as the extent written holds it (EMU, whole), as docx-agent's
/// `Picture.size` reads it back.
#[must_use]
pub fn written_size((width, height): (f64, f64)) -> (f64, f64) {
    #[allow(clippy::cast_precision_loss)]
    let back = |points: f64| emu(points) as f64 / EMU;
    (back(width), back(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_values_keep_breaks() {
        assert_eq!(quoted("a\"b\n<c>"), "\"a&quot;b&#10;&lt;c&gt;\"");
    }

    #[test]
    fn cells_told_from_positions() {
        assert!(is_cell("t:1A2B3C4D/c0,1"));
        assert!(is_cell("t@body/0/c12,3"));
        assert!(!is_cell("p:1A2B3C4D@0"));
        assert!(!is_cell("t:1A2B3C4D/c0"));
    }

    #[test]
    fn other_formats_refused() {
        assert!(image_format(b"<svg/>").is_err());
        assert_eq!(image_format(b"GIF89a....").ok(), Some(("gif", "image/gif")));
    }
}
