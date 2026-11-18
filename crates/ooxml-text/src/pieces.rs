//! Owns: a paragraph's content as pieces -- every character, tab, break, hyphen and inline
//! object with its width in 1/4096 pt -- and the kern pair each glyph is charged with
//! (docx2svg's `linebreak.pieces`, `label_pieces`, `_units`, `_kern`): Word's advance is
//! `advance x half points x 2048 / unitsPerEm` units, exactly, then `w:w` and `w:spacing`;
//! the legacy kern of a pair is charged to its left glyph.
//!
//! A measure keeps a piece per character, so a piece is kept small (about 100 bytes): its
//! lengths as [`CompactUnits`], its format shared by the pieces of a run, and what a pair
//! kerns under (face and drawn size) derived from its run and format when kerning rather
//! than kept.
//!
//! Must not: decide where lines break (that is [`crate::breaks`]) or hyphenate words (that
//! is the `auto` module).

use crate::glyphs::{script_half_points, Glyphs};
use crate::input::{BreakKind, CharFormat, FaceKey, Label, LabelAlign, LabelBody, LabelSuffix, Mark, Paragraph, Run};
use crate::units::{twips, CompactUnits, Units};
use crate::TextError;
use num_integer::Integer;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a piece is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PieceKind {
    /// A character drawn (or the label's shift, or a note separator's line).
    Glyph,
    /// A breaking space.
    Space,
    /// A tab.
    Tab,
    /// A line, page or column break.
    Break,
    /// A soft hyphen: a place a line may end, drawing a hyphen.
    SoftHyphen,
    /// An inline picture.
    Object,
}

/// What a kern pair is looked up in: pieces kern only with a neighbour of the same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KernKey<'a> {
    /// The face.
    face: &'a FaceKey,
    /// The size the glyphs are drawn at, half points.
    half_points: u32,
}

/// One piece of a paragraph. Its lengths and format are read through [`Piece::width`],
/// [`Piece::kern_next`], [`Piece::hyphen`] and [`Piece::format`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// What it is.
    pub kind: PieceKind,
    /// The character drawn: `\t` a tab, `\n` `\u{c}` `\u{b}` a line, page or column break,
    /// `\u{ad}` a soft hyphen, `\u{2011}` a non-breaking hyphen, `\u{fffc}` an object or a
    /// picture bullet, `\u{0}` the label's shift, `\u{e000}` `\u{e001}` a note
    /// separator's and continuation separator's line.
    pub ch: char,
    /// Its advance, tracking included (0 for a tab, a break or a soft hyphen).
    width: CompactUnits,
    /// The kern pair with the next piece, charged to this one.
    kern_next: CompactUnits,
    /// Its index in the paragraph's text (the runs' text concatenated); -1 for the label.
    pub source: i64,
    /// For a soft hyphen: what the hyphen drawn costs when the line ends at it.
    hyphen: CompactUnits,
    /// For an object: its height, whole twips.
    pub height: i64,
    /// The run it comes from; -1 for the label.
    pub run: i64,
    /// A soft hyphen automatic hyphenation put at a word's break point.
    pub auto: bool,
    /// For such a hyphen in a word that runs into a `w:noProof` run: where that run starts.
    pub proof_end: Option<i64>,
    /// The format it is drawn with, where it is a character of a run or label (shared by
    /// the pieces of a run that have it).
    format: Option<Arc<CharFormat>>,
}

impl Piece {
    fn new(kind: PieceKind, ch: char, width: Units, source: i64) -> Piece {
        Piece {
            kind,
            ch,
            width: width.into(),
            kern_next: CompactUnits::ZERO,
            source,
            hyphen: CompactUnits::ZERO,
            height: 0,
            run: -1,
            auto: false,
            proof_end: None,
            format: None,
        }
    }

    /// An automatic soft hyphen before the character at `source` of run `run`, whose hyphen
    /// is `hyphen` wide; `proof_end`: where a `w:noProof` run its word runs into starts.
    pub(crate) fn auto_hyphen(source: i64, hyphen: Units, run: i64, proof_end: Option<i64>) -> Piece {
        let mut piece = Piece::new(PieceKind::SoftHyphen, '\u{ad}', Units::ZERO, source);
        piece.hyphen = hyphen.into();
        piece.run = run;
        piece.auto = true;
        piece.proof_end = proof_end;
        piece
    }

    /// Its advance, tracking included (0 for a tab, a break or a soft hyphen).
    pub fn width(&self) -> Units {
        self.width.get()
    }

    /// The kern pair with the next piece, charged to this one.
    pub fn kern_next(&self) -> Units {
        self.kern_next.get()
    }

    /// For a soft hyphen: what the hyphen drawn costs when the line ends at it.
    pub fn hyphen(&self) -> Units {
        self.hyphen.get()
    }

    /// The format it is drawn with, where it is a character of a run or label.
    pub fn format(&self) -> Option<&CharFormat> {
        self.format.as_deref()
    }
}

/// The formats of a paragraph's pieces, one `Arc` for a run of pieces with equal formats.
#[derive(Default)]
struct SharedFormats {
    last: Option<Arc<CharFormat>>,
}

impl SharedFormats {
    fn share(&mut self, format: &CharFormat) -> Arc<CharFormat> {
        match &self.last {
            Some(last) if **last == *format => Arc::clone(last),
            _ => {
                let shared = Arc::new(format.clone());
                self.last = Some(Arc::clone(&shared));
                shared
            }
        }
    }
}

/// Characters that are a breaking space: the space, the en, em and four-per-em spaces and
/// the zero-width space.
pub const SPACES: [char; 5] = [' ', '\u{2002}', '\u{2003}', '\u{2005}', '\u{200b}'];
/// The character a note separator's line stands as.
pub const NOTE_SEPARATOR: char = '\u{e000}';
/// The character a continuation separator's line stands as.
pub const NOTE_CONTINUATION: char = '\u{e001}';
/// The character a picture bullet and an inline object stand as.
pub const OBJECT_CHAR: char = '\u{fffc}';
/// The character of the piece that moves the pen back before a centred or right label.
pub const LABEL_SHIFT: char = '\u{0}';
/// The face a picture bullet's space is as wide as a space of.
pub const PICTURE_BULLET_SPACE_FACE: &str = "Arial";

fn unmeasurable(message: String) -> TextError {
    TextError::Unmeasurable(message)
}

/// `advance x half points x 2048 / units per em` units.
pub fn advance_units(advance: u32, units_per_em: u32, half_points: u32) -> Result<Units, TextError> {
    let numerator = i128::from(advance) * i128::from(half_points) * 2048;
    Units::ratio(numerator, i128::from(units_per_em)).ok_or_else(|| unmeasurable("a face of 0 units per em".into()))
}

/// A note number's glyph's advance: its advance at the size it is drawn at (whole device
/// px at 300 dpi), rounded to whole device px (docx2svg's `note_number_units`).
pub fn note_number_units(advance: u32, units_per_em: u32, half_points: u32) -> Result<Units, TextError> {
    let upm = i128::from(units_per_em);
    if upm == 0 {
        return Err(unmeasurable("a face of 0 units per em".into()));
    }
    // ppem = floor(half_points / 2 x 300 / 72 + 1/2) = floor((300 hp + 72) / 144)
    let ppem = Integer::div_floor(&(300 * i128::from(half_points) + 72), &144);
    // px = floor(advance x ppem / upm + 1/2)
    let px = Integer::div_floor(&(2 * i128::from(advance) * ppem + upm), &(2 * upm));
    Units::ratio(px * 72 * 4096, 300).ok_or_else(|| unmeasurable("note number".into()))
}

/// A picture bullet's width, 1/4096 pt: `9 (s - 4) / h` pt for a label of `s` half points
/// and a picture `h` px high.
pub fn picture_bullet_units(half_points: u32, pixel_height: u32) -> Result<Units, TextError> {
    let value = 9 * (i128::from(half_points) - 4) * 4096;
    Units::ratio(value, i128::from(pixel_height)).ok_or_else(|| unmeasurable("a picture bullet 0 px high".into()))
}

fn upper(ch: char) -> Result<char, TextError> {
    let mut upper = ch.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(one), None) => Ok(one),
        _ => Err(unmeasurable(format!("{ch:?} in capitals is more than one character"))),
    }
}

fn advance_of(glyphs: &dyn Glyphs, face: &FaceKey, ch: char) -> Result<crate::glyphs::Advance, TextError> {
    glyphs.advance(face, ch).ok_or_else(|| {
        unmeasurable(format!(
            "no advance for {ch:?} in {} b={} i={}",
            face.family, face.bold, face.italic
        ))
    })
}

/// The size a character of `face` in `format` is drawn at, half points: its script size
/// where it is raised or lowered.
fn drawn_half_points(glyphs: &dyn Glyphs, face: &FaceKey, format: &CharFormat) -> Result<u32, TextError> {
    match format.vertical_align {
        None => Ok(format.half_points),
        Some(align) => glyphs
            .script_metrics(face)
            .and_then(|m| script_half_points(format.half_points, align, &m))
            .ok_or_else(|| unmeasurable(format!("no script size for {}", face.family))),
    }
}

/// A character of a run as drawn (in capitals where the run is) and its width, as
/// docx2svg's `pieces.glyph` makes it.
fn glyph_width(glyphs: &dyn Glyphs, run: &Run, format: &CharFormat, ch: char) -> Result<(char, Units), TextError> {
    let drawn = if run.caps { upper(ch)? } else { ch };
    let face = format
        .face
        .as_ref()
        .ok_or_else(|| unmeasurable(format!("no face for {ch:?}")))?;
    let half_points = drawn_half_points(glyphs, face, format)?;
    let advance = advance_of(glyphs, face, drawn)?;
    let mut width = if run.note_number {
        note_number_units(advance.advance, advance.units_per_em, half_points)?
    } else {
        advance_units(advance.advance, advance.units_per_em, half_points)?
    };
    if run.scale != 100 {
        width = width.scale(i128::from(run.scale), 100).unwrap_or(width);
    }
    if run.tracking != 0 {
        width += twips(i64::from(run.tracking));
    }
    Ok((drawn, width))
}

/// A glyph of a run, drawn with `format`.
fn glyph(
    glyphs: &dyn Glyphs,
    run: &Run,
    format: Arc<CharFormat>,
    ch: char,
    kind: PieceKind,
    index: i64,
) -> Result<Piece, TextError> {
    let (drawn, width) = glyph_width(glyphs, run, &format, ch)?;
    let mut piece = Piece::new(kind, drawn, width, index);
    piece.format = Some(format);
    Ok(piece)
}

/// The width of a hyphen in a run, or `None` where it cannot be measured.
pub(crate) fn hyphen_width(glyphs: &dyn Glyphs, run: &Run) -> Option<Units> {
    glyph_width(glyphs, run, &run.hyphen_format, '-')
        .ok()
        .map(|(_, width)| width)
}

fn mark(
    glyphs: &dyn Glyphs,
    run: &Run,
    mark: Mark,
    source: i64,
    formats: &mut SharedFormats,
    out: &mut Vec<Piece>,
) -> Result<(), TextError> {
    match mark {
        Mark::Tab => out.push(Piece::new(PieceKind::Tab, '\t', Units::ZERO, source)),
        Mark::Break(kind) => {
            let ch = match kind {
                BreakKind::Line => '\n',
                BreakKind::Page => '\u{c}',
                BreakKind::Column => '\u{b}',
            };
            out.push(Piece::new(PieceKind::Break, ch, Units::ZERO, source));
        }
        Mark::NoBreakHyphen => {
            let format = formats.share(&run.hyphen_format);
            let mut piece = glyph(glyphs, run, format, '-', PieceKind::Glyph, source)?;
            piece.ch = '\u{2011}';
            out.push(piece);
        }
        Mark::SoftHyphen => {
            let (_, hyphen) = glyph_width(glyphs, run, &run.hyphen_format, '-')?;
            let mut piece = Piece::new(PieceKind::SoftHyphen, '\u{ad}', Units::ZERO, source);
            piece.hyphen = hyphen.into();
            out.push(piece);
        }
        Mark::Drawing {
            cx,
            cy,
            effect: [left, top, right, bottom],
        } => {
            let mut piece = Piece::new(
                PieceKind::Object,
                OBJECT_CHAR,
                twips(Integer::div_floor(&(cx + left + right), &635)),
                source,
            );
            piece.height = Integer::div_floor(&(cy + top + bottom), &635);
            out.push(piece);
        }
        Mark::UnmeasuredDrawing => return Err(unmeasurable("a drawing".into())),
        Mark::Separator => out.push(Piece::new(PieceKind::Glyph, NOTE_SEPARATOR, twips(2880), source)),
        Mark::ContinuationSeparator => out.push(Piece::new(PieceKind::Glyph, NOTE_CONTINUATION, Units::ZERO, source)),
        Mark::Other => {}
    }
    Ok(())
}

/// The pieces of one run, appended to `out`; `source` is the index of its first character.
fn run_pieces(
    glyphs: &dyn Glyphs,
    run: &Run,
    run_index: i64,
    source: i64,
    formats: &mut SharedFormats,
    out: &mut Vec<Piece>,
) -> Result<(), TextError> {
    let first_new = out.len();
    let mut marks: BTreeMap<usize, Vec<Mark>> = BTreeMap::new();
    for (at, m) in &run.marks {
        marks.entry(*at).or_default().push(*m);
    }
    let mut length = 0_usize;
    for (offset, ch) in run.text.chars().enumerate() {
        for m in marks.get(&offset).into_iter().flatten() {
            mark(glyphs, run, *m, source, formats, out)?;
        }
        let format = run
            .formats
            .at(offset)
            .ok_or_else(|| unmeasurable("a character without a format".into()))?;
        let kind = if SPACES.contains(&ch) {
            PieceKind::Space
        } else {
            PieceKind::Glyph
        };
        out.push(glyph(
            glyphs,
            run,
            formats.share(format),
            ch,
            kind,
            source + offset as i64,
        )?);
        length = offset + 1;
    }
    for m in marks.get(&length).into_iter().flatten() {
        mark(glyphs, run, *m, source, formats, out)?;
    }
    for piece in out.iter_mut().skip(first_new) {
        piece.run = run_index;
    }
    Ok(())
}

/// The label's pieces (docx2svg's `label_pieces`).
pub fn label_pieces(glyphs: &dyn Glyphs, label: &Label) -> Result<Vec<Piece>, TextError> {
    let mut out = Vec::new();
    let mut formats = SharedFormats::default();
    match &label.body {
        LabelBody::UnreadPicture => return Err(unmeasurable("a picture bullet whose picture is not read".into())),
        LabelBody::Picture {
            half_points,
            pixel_height,
        } => {
            out.push(Piece::new(
                PieceKind::Glyph,
                OBJECT_CHAR,
                picture_bullet_units(*half_points, *pixel_height)?,
                -1,
            ));
        }
        LabelBody::Text(chars) => {
            for (ch, format) in chars {
                let drawn = if label.caps { upper(*ch)? } else { *ch };
                let face = format
                    .face
                    .as_ref()
                    .ok_or_else(|| unmeasurable(format!("no advance for label {drawn:?}")))?;
                let advance = advance_of(glyphs, face, drawn)?;
                let mut piece = Piece::new(
                    PieceKind::Glyph,
                    drawn,
                    advance_units(advance.advance, advance.units_per_em, format.half_points)?,
                    -1,
                );
                piece.format = Some(formats.share(format));
                out.push(piece);
            }
        }
    }
    let width = out.iter().fold(Units::ZERO, |sum, p| sum + p.width());
    let shift = match label.alignment {
        LabelAlign::Center => width.scale(1, 2).unwrap_or(width),
        LabelAlign::Right => width,
        LabelAlign::Left => Units::ZERO,
    };
    if shift != Units::ZERO {
        out.insert(0, Piece::new(PieceKind::Glyph, LABEL_SHIFT, -shift, -1));
    }
    match label.suffix {
        LabelSuffix::Tab => out.push(Piece::new(PieceKind::Tab, '\t', Units::ZERO, -1)),
        LabelSuffix::Space => {
            let format = &label.space_format;
            let face = match label.body {
                LabelBody::Picture { .. } => Some(FaceKey {
                    family: PICTURE_BULLET_SPACE_FACE.into(),
                    bold: false,
                    italic: false,
                }),
                _ => format.face.clone(),
            };
            let face = face.ok_or_else(|| unmeasurable("no advance for the label's space".into()))?;
            let advance = glyphs
                .advance(&face, ' ')
                .ok_or_else(|| unmeasurable("no advance for the label's space".into()))?;
            out.push(Piece::new(
                PieceKind::Glyph,
                ' ',
                advance_units(advance.advance, advance.units_per_em, format.half_points)?,
                -1,
            ));
        }
        LabelSuffix::Nothing => {}
    }
    Ok(out)
}

/// The paragraph's pieces, the label's first, before automatic hyphenation and kerning.
pub(crate) fn paragraph_pieces(glyphs: &dyn Glyphs, paragraph: &Paragraph) -> Result<Vec<Piece>, TextError> {
    let mut out = match &paragraph.label {
        Some(label) => label_pieces(glyphs, label)?,
        None => Vec::new(),
    };
    let mut source = 0_i64;
    let mut formats = SharedFormats::default();
    for (index, run) in paragraph.runs.iter().enumerate() {
        let length = run.text.chars().count() as i64;
        if !run.vanish {
            run_pieces(glyphs, run, index as i64, source, &mut formats, &mut out)?;
        }
        source += length;
    }
    Ok(out)
}

/// What a piece kerns under: the face and drawn size of a character of a run that kerns at
/// its size (`w:kern`); `None` for anything else (a label, a mark, kerning off).
fn kern_key<'a>(
    glyphs: &dyn Glyphs,
    paragraph: &Paragraph,
    piece: &'a Piece,
) -> Result<Option<KernKey<'a>>, TextError> {
    let run = usize::try_from(piece.run).ok().and_then(|r| paragraph.runs.get(r));
    let (Some(run), Some(format)) = (run, piece.format()) else {
        return Ok(None);
    };
    if run.kern == 0 || format.half_points < run.kern {
        return Ok(None);
    }
    let Some(face) = format.face.as_ref() else {
        return Ok(None);
    };
    let half_points = drawn_half_points(glyphs, face, format)?;
    Ok(Some(KernKey { face, half_points }))
}

/// Charge every glyph of `paragraph`'s pieces with the kern pair it makes with the piece
/// after it.
pub(crate) fn kern(glyphs: &dyn Glyphs, paragraph: &Paragraph, pieces: &mut [Piece]) -> Result<(), TextError> {
    // The keys borrow the pieces' formats: the pairs are found first, then charged.
    let mut kerns = Vec::new();
    let mut right_key = match pieces.first() {
        Some(first) => kern_key(glyphs, paragraph, first)?,
        None => None,
    };
    for (left_index, pair) in pieces.windows(2).enumerate() {
        let [left, right] = pair else { continue };
        let left_key = right_key;
        right_key = kern_key(glyphs, paragraph, right)?;
        let (Some(key), Some(left_key)) = (right_key, left_key) else {
            continue;
        };
        if key != left_key {
            continue;
        }
        let value = glyphs
            .kern(key.face, left.ch, right.ch)
            .ok_or_else(|| unmeasurable(format!("no kerning for {}", key.face.family)))?;
        if value != 0 {
            let advance = advance_of(glyphs, key.face, right.ch)?;
            let magnitude = value.unsigned_abs();
            let units = advance_units(magnitude, advance.units_per_em, key.half_points)?;
            kerns.push((left_index, if value < 0 { -units } else { units }));
        }
    }
    for (index, units) in kerns {
        if let Some(piece) = pieces.get_mut(index) {
            piece.kern_next = units.into();
        }
    }
    Ok(())
}
