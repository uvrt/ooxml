//! Owns: one face of a font file, read lazily from its bytes -- its table directory
//! (read-fonts' `TableDirectory`), units per em, style, names, the line and script metrics
//! Word's layout takes, the decorations a painter takes, its character map, advances and
//! legacy kern pairs. The one sfnt reader of the workspace (design section 2.2); a port of
//! ooxml-common's `office.Face` and docx2svg's `fonts.Face`, reading each field at the
//! offset and under the length checks they read it with, so a face measures the same.
//!
//! Must not: look a face up by name (that is [`crate::catalogue`] and [`crate::word`]),
//! shape, or read outlines.

use crate::be::{i16_at, slice, u16_at, u32_at};
use crate::cmap::{read_cmap, CharMap};
use crate::kern::{read_legacy_kern, KernPairs};
use crate::names::{name_records, NameRecord};
use crate::FontError;
use read_fonts::{FontData, FontRead, TableDirectory};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

/// A table tag.
pub type Tag = [u8; 4];

/// The integers Word's vertical model takes from a face, in font units
/// (docx2svg's `vertical.FaceMetrics`): `hhea`'s ascent, descent (positive) and line gap,
/// or `OS/2`'s typographic ones under `USE_TYPO_METRICS`; and `OS/2`'s script sizes and
/// offsets where the table is long enough to hold them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceMetrics {
    /// `head.unitsPerEm`.
    pub units_per_em: u16,
    /// The ascent.
    pub ascent: i32,
    /// The descent, positive below the baseline.
    pub descent: i32,
    /// The line gap.
    pub line_gap: i32,
    /// `OS/2.ySuperscriptYSize`.
    pub superscript_size: Option<i16>,
    /// `OS/2.ySubscriptYSize`.
    pub subscript_size: Option<i16>,
    /// `OS/2.ySuperscriptYOffset`, positive up.
    pub superscript_offset: Option<i16>,
    /// `OS/2.ySubscriptYOffset`, positive down.
    pub subscript_offset: Option<i16>,
}

/// What drawing a run takes from its face beyond its metrics, in font units
/// (docx2svg's `fonts.Decorations`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Decorations {
    /// `head.unitsPerEm`.
    pub units_per_em: u16,
    /// `OS/2.ySuperscriptYOffset`.
    pub superscript_offset: Option<i16>,
    /// `OS/2.ySubscriptYOffset`.
    pub subscript_offset: Option<i16>,
    /// `OS/2.yStrikeoutPosition`.
    pub strikeout_position: Option<i16>,
    /// `OS/2.yStrikeoutSize`.
    pub strikeout_size: Option<i16>,
    /// `post.underlinePosition`.
    pub underline_position: Option<i16>,
    /// `post.underlineThickness`.
    pub underline_thickness: Option<i16>,
    /// `hhea.ascender`.
    pub hhea_ascent: i32,
    /// `hhea.descender`, positive below the baseline.
    pub hhea_descent: i32,
}

/// The four styles Word asks a family for; a typographic family (name ID 16) counts as a
/// family name only for a face whose typographic subfamily is one of them (or absent).
const STYLES: [&str; 4] = ["Regular", "Bold", "Italic", "Bold Italic"];

/// The offset of every face in a font file: one, or each of a collection's.
pub fn face_offsets(data: &[u8]) -> Vec<usize> {
    if data.get(..4) != Some(b"ttcf".as_slice()) {
        return vec![0];
    }
    let count = u32_at(data, 8).unwrap_or_default() as usize;
    (0..count)
        .map_while(|i| u32_at(data, 12 + 4 * i).map(|v| v as usize))
        .collect()
}

/// A table directory: tag -> (offset, length) in the file; a later record of a tag wins.
pub(crate) fn table_directory(
    data: &[u8],
    offset: usize,
    source: &str,
) -> Result<HashMap<Tag, (usize, usize)>, FontError> {
    let head = data.get(offset..).filter(|rest| rest.len() >= 12);
    let head = head.ok_or_else(|| FontError::malformed(source, "too short for an offset table"))?;
    let directory =
        TableDirectory::read(FontData::new(head)).map_err(|e| FontError::malformed(source, e.to_string()))?;
    Ok(directory
        .table_records()
        .iter()
        .map(|record| {
            (
                record.tag().into_bytes(),
                (record.offset() as usize, record.length() as usize),
            )
        })
        .collect())
}

/// One face, over the bytes of the file it is in.
pub struct Face {
    source: String,
    data: Arc<[u8]>,
    offset: usize,
    tables: HashMap<Tag, (usize, usize)>,
    units_per_em: u16,
    cmap: OnceLock<CharMap>,
    advances: OnceLock<Vec<u16>>,
    kern: OnceLock<KernPairs>,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Face")
            .field("source", &self.source)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl Face {
    /// Face `number` of the file `data` (0 for a single font). `source` names where the
    /// bytes came from, for errors and reports.
    pub fn new(data: Arc<[u8]>, number: usize, source: impl Into<String>) -> Result<Face, FontError> {
        let source = source.into();
        let offset = *face_offsets(&data)
            .get(number)
            .ok_or_else(|| FontError::malformed(&source, format!("no face {number} in the file")))?;
        Face::at_offset(data, offset, source)
    }

    /// The face whose table directory is at `offset` of `data`.
    pub fn at_offset(data: Arc<[u8]>, offset: usize, source: impl Into<String>) -> Result<Face, FontError> {
        let source = source.into();
        let tables = table_directory(&data, offset, &source)?;
        let mut face = Face {
            source,
            data,
            offset,
            tables,
            units_per_em: 0,
            cmap: OnceLock::new(),
            advances: OnceLock::new(),
            kern: OnceLock::new(),
        };
        face.units_per_em =
            u16_at(face.required(b"head")?, 18).ok_or_else(|| FontError::malformed(&face.source, "head too short"))?;
        Ok(face)
    }

    /// Where the bytes came from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The bytes of the file the face is in.
    pub fn data(&self) -> &Arc<[u8]> {
        &self.data
    }

    /// The offset of the face's table directory in its file.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// A table's bytes (cut short at the end of the file), or `None`.
    pub fn table(&self, tag: &Tag) -> Option<&[u8]> {
        self.tables
            .get(tag)
            .map(|&(offset, length)| slice(&self.data, offset, length))
    }

    fn required(&self, tag: &Tag) -> Result<&[u8], FontError> {
        self.table(tag)
            .ok_or_else(|| FontError::malformed(&self.source, format!("no {} table", String::from_utf8_lossy(tag))))
    }

    /// Whether the face has a table.
    pub fn has_table(&self, tag: &Tag) -> bool {
        self.tables.contains_key(tag)
    }

    /// `head.unitsPerEm`.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// `(bold, italic)`: `OS/2.fsSelection`'s bold and italic bits, or `head.macStyle`'s.
    pub fn style(&self) -> Result<(bool, bool), FontError> {
        let selection = self
            .table(b"OS/2")
            .filter(|t| t.len() >= 64)
            .and_then(|t| u16_at(t, 62))
            .unwrap_or(0);
        let mac_style =
            u16_at(self.required(b"head")?, 44).ok_or_else(|| FontError::malformed(&self.source, "head too short"))?;
        Ok((
            selection & 0x20 != 0 || mac_style & 1 != 0,
            selection & 0x01 != 0 || mac_style & 2 != 0,
        ))
    }

    /// Every decodable record of the `name` table.
    pub fn names(&self) -> Vec<NameRecord> {
        self.table(b"name").map(name_records).unwrap_or_default()
    }

    /// The family names Word may ask for this face by: name ID 1, and 16 where the
    /// typographic subfamily (17) is one of the four styles, or absent.
    pub fn families(&self) -> BTreeSet<String> {
        families_of(&self.names())
    }

    /// The line and script metrics Word's layout takes ([`FaceMetrics`]).
    pub fn metrics(&self) -> Result<FaceMetrics, FontError> {
        let os2 = self.table(b"OS/2");
        let script = |offset: usize| os2.filter(|t| t.len() >= 26).and_then(|t| i16_at(t, offset));
        let typo = os2.filter(|t| t.len() >= 74 && u16_at(t, 62).is_some_and(|s| s & 0x80 != 0));
        let source = typo
            .map(|t| (t, 68))
            .map_or_else(|| self.required(b"hhea").map(|t| (t, 4)), Ok)?;
        let read = |k: usize| i16_at(source.0, source.1 + 2 * k).map(i32::from);
        let (Some(ascent), Some(descender), Some(line_gap)) = (read(0), read(1), read(2)) else {
            return Err(FontError::malformed(&self.source, "line metrics cut short"));
        };
        Ok(FaceMetrics {
            units_per_em: self.units_per_em,
            ascent,
            descent: -descender,
            line_gap,
            superscript_size: script(20),
            subscript_size: script(12),
            superscript_offset: script(24),
            subscript_offset: script(16),
        })
    }

    /// The decorations' geometry ([`Decorations`]).
    pub fn decorations(&self) -> Result<Decorations, FontError> {
        let hhea = self.required(b"hhea")?;
        let os2 = self.table(b"OS/2").filter(|t| t.len() >= 30);
        let post = self.table(b"post").filter(|t| t.len() >= 12);
        let at = |table: Option<&[u8]>, offset: usize| table.and_then(|t| i16_at(t, offset));
        Ok(Decorations {
            units_per_em: self.units_per_em,
            superscript_offset: at(os2, 24),
            subscript_offset: at(os2, 16),
            strikeout_position: at(os2, 28),
            strikeout_size: at(os2, 26),
            underline_position: at(post, 8),
            underline_thickness: at(post, 10),
            hhea_ascent: i16_at(hhea, 4).map(i32::from).unwrap_or_default(),
            hhea_descent: -i16_at(hhea, 6).map(i32::from).unwrap_or_default(),
        })
    }

    /// The glyph `ch` maps to, if any.
    pub fn glyph(&self, ch: char) -> Option<u32> {
        self.cmap
            .get_or_init(|| self.table(b"cmap").map(read_cmap).unwrap_or_default())
            .get(&u32::from(ch))
            .copied()
    }

    /// Every glyph's advance up to `hhea.numberOfHMetrics`.
    fn advances(&self) -> &[u16] {
        self.advances.get_or_init(|| {
            let count = self.table(b"hhea").and_then(|t| u16_at(t, 34)).unwrap_or_default();
            let hmtx = self.table(b"hmtx").unwrap_or_default();
            (0..usize::from(count)).map_while(|i| u16_at(hmtx, 4 * i)).collect()
        })
    }

    /// A glyph's advance in font units: a glyph past `numberOfHMetrics` takes the last.
    pub fn glyph_advance(&self, glyph: u32) -> Option<u16> {
        let advances = self.advances();
        advances.get(glyph as usize).or_else(|| advances.last()).copied()
    }

    /// The advance of `ch` in font units, or `None` where the face lacks it.
    pub fn advance(&self, ch: char) -> Option<u16> {
        self.glyph(ch).and_then(|glyph| self.glyph_advance(glyph))
    }

    /// The legacy `kern` table's adjustment between two characters, font units (0 where
    /// either is not mapped or the pair is not kerned).
    pub fn kern(&self, left: char, right: char) -> i16 {
        let (Some(a), Some(b)) = (self.glyph(left), self.glyph(right)) else {
            return 0;
        };
        let (Ok(a), Ok(b)) = (u16::try_from(a), u16::try_from(b)) else {
            return 0;
        };
        let pairs = self
            .kern
            .get_or_init(|| self.table(b"kern").map(read_legacy_kern).unwrap_or_default());
        pairs.get(&(a, b)).copied().unwrap_or(0)
    }
}

/// The family names of a face's name records ([`Face::families`]).
pub fn families_of(names: &[NameRecord]) -> BTreeSet<String> {
    let subfamilies: Vec<&str> = names
        .iter()
        .filter(|r| r.name_id == 17)
        .map(|r| r.text.as_str())
        .collect();
    let typographic = subfamilies.is_empty() || subfamilies.iter().any(|s| STYLES.contains(s));
    names
        .iter()
        .filter(|r| r.name_id == 1 || (r.name_id == 16 && typographic))
        .map(|r| r.text.clone())
        .collect()
}
