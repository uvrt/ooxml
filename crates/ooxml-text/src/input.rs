//! Owns: what the engine measures -- a paragraph's runs of text with every character's
//! resolved format, the marks between them (tabs, breaks, hyphens, inline objects), its
//! list label, and whether it is hyphenated -- and its content digest, which a measure is
//! memoised under.
//!
//! Must not: know styles, numbering or the document: a caller (the Word model) resolves
//! all of it before handing it here.

use ooxml_base::Digester;
use std::sync::Arc;

/// A face as a run asks for it: family, bold, italic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceKey {
    /// The family name.
    pub family: Arc<str>,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
}

/// `w:vertAlign`: a raised or lowered run is drawn at its face's script size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerticalAlign {
    /// Superscript.
    Superscript,
    /// Subscript.
    Subscript,
}

/// One character's resolved format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharFormat {
    /// The face, or `None` where no face is resolved (the character is unmeasurable).
    pub face: Option<FaceKey>,
    /// `w:sz` (or `w:szCs`), half points.
    pub half_points: u32,
    /// `w:vertAlign`.
    pub vertical_align: Option<VerticalAlign>,
}

/// What ends a line where a `w:br` or `w:cr` stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BreakKind {
    /// A line break (`w:br`, `textWrapping`, `w:cr`): `\n`.
    Line,
    /// A page break: `\f`.
    Page,
    /// A column break: `\v`.
    Column,
}

/// Something in a run that is not a character of its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mark {
    /// `w:tab`.
    Tab,
    /// A break.
    Break(BreakKind),
    /// `w:softHyphen`.
    SoftHyphen,
    /// `w:noBreakHyphen`.
    NoBreakHyphen,
    /// An inline picture: its extent and effect extent, EMU.
    Drawing {
        /// `wp:extent/@cx`.
        cx: i64,
        /// `wp:extent/@cy`.
        cy: i64,
        /// The effect extent's four sides: left, top, right, bottom.
        effect: [i64; 4],
    },
    /// A drawing the layout does not lay out (the paragraph is unmeasurable).
    UnmeasuredDrawing,
    /// A note separator's line.
    Separator,
    /// A note continuation separator's line.
    ContinuationSeparator,
    /// Anything else (an anchor's place): no piece.
    Other,
}

/// A run's characters' formats: one for every character, or each its own.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RunFormats {
    /// Every character the same.
    Uniform(CharFormat),
    /// One per character, in order.
    PerChar(Vec<CharFormat>),
}

impl RunFormats {
    /// The format of character `index`.
    pub fn at(&self, index: usize) -> Option<&CharFormat> {
        match self {
            RunFormats::Uniform(format) => Some(format),
            RunFormats::PerChar(formats) => formats.get(index),
        }
    }
}

/// One run of a paragraph, resolved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Run {
    /// Its text.
    pub text: String,
    /// Its marks, by the index of the character they stand before (the text's length:
    /// after it), in document order.
    pub marks: Vec<(usize, Mark)>,
    /// `w:vanish`: the run takes no room (its text still counts in the indices).
    pub vanish: bool,
    /// Every character's format (empty text: any).
    pub formats: RunFormats,
    /// The format of a hyphen in the run (for soft and non-breaking hyphens).
    pub hyphen_format: CharFormat,
    /// `w:spacing`, twips added to every character's advance.
    pub tracking: i32,
    /// `w:w`, percent.
    pub scale: u32,
    /// `w:kern`: kern pairs at this size and above (half points; 0: off).
    pub kern: u32,
    /// `w:caps`.
    pub caps: bool,
    /// A note's number (its glyphs advance by whole device px).
    pub note_number: bool,
    /// `w:lang/@w:val`.
    pub lang: Option<String>,
    /// `w:noProof`.
    pub no_proof: bool,
}

/// How a list label stands against its start (`w:lvlJc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LabelAlign {
    /// From its start.
    Left,
    /// Centred on it.
    Center,
    /// Ending at it (`right`, `end`).
    Right,
}

/// What follows a list label (`w:suff`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LabelSuffix {
    /// A tab.
    Tab,
    /// A space.
    Space,
    /// Nothing.
    Nothing,
}

/// What a label shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LabelBody {
    /// Text, every character with its format.
    Text(Vec<(char, CharFormat)>),
    /// A picture bullet: the label's size, half points, and the picture's height, px.
    Picture {
        /// The label's size.
        half_points: u32,
        /// The picture's height in pixels.
        pixel_height: u32,
    },
    /// A picture bullet whose picture was not read (the paragraph is unmeasurable).
    UnreadPicture,
}

/// A paragraph's list label, its numbers already substituted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    /// What it shows.
    pub body: LabelBody,
    /// `w:caps` of its format.
    pub caps: bool,
    /// `w:lvlJc`.
    pub alignment: LabelAlign,
    /// `w:suff`.
    pub suffix: LabelSuffix,
    /// The format of a space in the label (its suffix's).
    pub space_format: CharFormat,
}

/// Automatic hyphenation of a paragraph (`w:autoHyphenation`, not suppressed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hyphenation {
    /// The document is in compatibility mode 15 or later.
    pub mode15: bool,
    /// `w:doNotHyphenateCaps`.
    pub no_caps: bool,
}

/// A paragraph's text as the engine measures it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Paragraph {
    /// Its runs.
    pub runs: Vec<Run>,
    /// Its list label, if it shows one.
    pub label: Option<Label>,
    /// Its automatic hyphenation, if any.
    pub hyphenation: Option<Hyphenation>,
}

/// Feeding a value's content into a digest, field by field, each length-prefixed.
pub(crate) trait Digestible {
    fn feed(&self, d: &mut Digester);
}

fn text(d: &mut Digester, value: &str) {
    d.update(&(value.len() as u64).to_le_bytes());
    d.update(value.as_bytes());
}

fn num(d: &mut Digester, value: i64) {
    d.update(&value.to_le_bytes());
}

impl Digestible for CharFormat {
    fn feed(&self, d: &mut Digester) {
        match &self.face {
            Some(face) => {
                text(d, &face.family);
                d.update(&[1, u8::from(face.bold), u8::from(face.italic)]);
            }
            None => d.update(&[0]),
        }
        num(d, i64::from(self.half_points));
        d.update(&[match self.vertical_align {
            None => 0,
            Some(VerticalAlign::Superscript) => 1,
            Some(VerticalAlign::Subscript) => 2,
        }]);
    }
}

impl Digestible for Mark {
    fn feed(&self, d: &mut Digester) {
        let tag: u8 = match self {
            Mark::Tab => 0,
            Mark::Break(BreakKind::Line) => 1,
            Mark::Break(BreakKind::Page) => 2,
            Mark::Break(BreakKind::Column) => 3,
            Mark::SoftHyphen => 4,
            Mark::NoBreakHyphen => 5,
            Mark::Drawing { .. } => 6,
            Mark::UnmeasuredDrawing => 7,
            Mark::Separator => 8,
            Mark::ContinuationSeparator => 9,
            Mark::Other => 10,
        };
        d.update(&[tag]);
        if let Mark::Drawing { cx, cy, effect } = self {
            for value in [*cx, *cy].iter().chain(effect) {
                num(d, *value);
            }
        }
    }
}

impl Digestible for Run {
    fn feed(&self, d: &mut Digester) {
        text(d, &self.text);
        num(d, self.marks.len() as i64);
        for (at, mark) in &self.marks {
            num(d, *at as i64);
            mark.feed(d);
        }
        d.update(&[
            u8::from(self.vanish),
            u8::from(self.caps),
            u8::from(self.note_number),
            u8::from(self.no_proof),
        ]);
        match &self.formats {
            RunFormats::Uniform(format) => {
                d.update(&[0]);
                format.feed(d);
            }
            RunFormats::PerChar(formats) => {
                d.update(&[1]);
                num(d, formats.len() as i64);
                formats.iter().for_each(|f| f.feed(d));
            }
        }
        self.hyphen_format.feed(d);
        for value in [i64::from(self.tracking), i64::from(self.scale), i64::from(self.kern)] {
            num(d, value);
        }
        text(d, self.lang.as_deref().unwrap_or("\u{0}"));
    }
}

impl Digestible for Label {
    fn feed(&self, d: &mut Digester) {
        match &self.body {
            LabelBody::Text(chars) => {
                d.update(&[0]);
                num(d, chars.len() as i64);
                for (ch, format) in chars {
                    num(d, i64::from(u32::from(*ch)));
                    format.feed(d);
                }
            }
            LabelBody::Picture {
                half_points,
                pixel_height,
            } => {
                d.update(&[1]);
                num(d, i64::from(*half_points));
                num(d, i64::from(*pixel_height));
            }
            LabelBody::UnreadPicture => d.update(&[2]),
        }
        d.update(&[u8::from(self.caps), self.alignment as u8, self.suffix as u8]);
        self.space_format.feed(d);
    }
}

impl Digestible for Paragraph {
    fn feed(&self, d: &mut Digester) {
        num(d, self.runs.len() as i64);
        self.runs.iter().for_each(|r| r.feed(d));
        match &self.label {
            Some(label) => {
                d.update(&[1]);
                label.feed(d);
            }
            None => d.update(&[0]),
        }
        match self.hyphenation {
            Some(h) => d.update(&[1, u8::from(h.mode15), u8::from(h.no_caps)]),
            None => d.update(&[0]),
        }
    }
}
