//! Owns: the port the engine reads faces through ([`Glyphs`]: an advance and its units per
//! em, a legacy kern pair, a face's script metrics, the fonts epoch), and what Word derives
//! from a face's script metrics -- the size a raised or lowered run is drawn at
//! (docx2svg's `resolve.script_half_points` and `script_fields_taken`).
//!
//! Must not: find faces (the fonts layer does; L3 adapts it to this port), or know a
//! paragraph.

use crate::input::{FaceKey, VerticalAlign};
use num_integer::Integer;
use ooxml_base::Digest;

/// An advance in font units and the units per em of the face it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Advance {
    /// The advance.
    pub advance: u32,
    /// Units per em.
    pub units_per_em: u32,
}

/// What Word takes from a face to size a `w:vertAlign` run: `OS/2`'s script sizes and
/// offsets, font units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptMetrics {
    /// Units per em.
    pub units_per_em: u32,
    /// `ySuperscriptYSize`.
    pub superscript_size: Option<i32>,
    /// `ySubscriptYSize`.
    pub subscript_size: Option<i32>,
    /// `ySuperscriptYOffset`.
    pub superscript_offset: Option<i32>,
    /// `ySubscriptYOffset` (positive down).
    pub subscript_offset: Option<i32>,
}

/// Where the engine reads faces: the face Word lays a [`FaceKey`] out with.
pub trait Glyphs {
    /// The advance of `ch` in `face`, or `None` where it is unknown.
    fn advance(&self, face: &FaceKey, ch: char) -> Option<Advance>;
    /// The legacy kern adjustment between two characters in `face`, font units; `None`
    /// where the face is unknown.
    fn kern(&self, face: &FaceKey, left: char, right: char) -> Option<i32>;
    /// The face's script metrics, or `None` where the face is unknown.
    fn script_metrics(&self, face: &FaceKey) -> Option<ScriptMetrics>;
    /// What a measure made with these faces is cached under: it changes whenever any
    /// answer above may.
    fn epoch(&self) -> Digest;
}

/// The fraction of the size a script run is drawn at when the face's own is out of range:
/// 3/5.
pub const SCRIPT_SIZE_FALLBACK: (i128, i128) = (3, 5);
/// Where Word takes a face's own script size: from 1/4 to 1 of the size (inclusive).
pub const SCRIPT_SIZE_RANGE: [(i128, i128); 2] = [(1, 4), (1, 1)];
/// The least superscript offset (of an em) at which Word takes a face's script fields: 1/5.
pub const SCRIPT_OFFSET_MINIMUM: (i128, i128) = (1, 5);

fn in_range(size: i32, upm: u32) -> bool {
    let (size, upm) = (i128::from(size), i128::from(upm));
    let [(lo_n, lo_d), (hi_n, hi_d)] = SCRIPT_SIZE_RANGE;
    size * lo_d >= lo_n * upm && size * hi_d <= hi_n * upm
}

/// Whether Word draws a script run with the face's own script size and offsets.
pub fn script_fields_taken(face: &ScriptMetrics) -> bool {
    if face
        .superscript_size
        .is_some_and(|size| !in_range(size, face.units_per_em))
    {
        return false;
    }
    let (min_n, min_d) = SCRIPT_OFFSET_MINIMUM;
    if face
        .superscript_offset
        .is_some_and(|o| i128::from(o) * min_d < min_n * i128::from(face.units_per_em))
    {
        return false;
    }
    !face.subscript_offset.is_some_and(|o| o < 0)
}

/// The size, half points, a run of `half_points` raised or lowered is drawn at: the face's
/// script size over its em, rounded half up, at least 2; `None` where the face gives no
/// script size.
pub fn script_half_points(half_points: u32, align: VerticalAlign, face: &ScriptMetrics) -> Option<u32> {
    let size = match align {
        VerticalAlign::Superscript => face.superscript_size,
        VerticalAlign::Subscript => face.subscript_size,
    }?;
    let (mut num, mut den) = (i128::from(size), i128::from(face.units_per_em));
    if den == 0 || !in_range(size, face.units_per_em) || !script_fields_taken(face) {
        (num, den) = SCRIPT_SIZE_FALLBACK;
    }
    // floor(half_points x num / den + 1/2)
    let value = Integer::div_floor(&(2 * i128::from(half_points) * num + den), &(2 * den));
    u32::try_from(value.max(2)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(size: i32, sup: i32, sub: i32) -> ScriptMetrics {
        ScriptMetrics {
            units_per_em: 2048,
            superscript_size: Some(size),
            subscript_size: Some(size),
            superscript_offset: Some(sup),
            subscript_offset: Some(sub),
        }
    }

    #[test]
    fn script_sizes() {
        // Calibri: 1331/2048 = 0.65; 22 half points -> 14.3 -> 14.
        assert_eq!(
            script_half_points(22, VerticalAlign::Superscript, &face(1331, 976, 286)),
            Some(14)
        );
        // An offset under 1/5 em: 3/5 of 22 = 13.2 -> 13.
        assert_eq!(
            script_half_points(22, VerticalAlign::Superscript, &face(1331, 200, 286)),
            Some(13)
        );
        // A negative subscript offset: the fallback too.
        assert_eq!(
            script_half_points(22, VerticalAlign::Subscript, &face(1331, 976, -1)),
            Some(13)
        );
        assert_eq!(
            script_half_points(2, VerticalAlign::Subscript, &face(1331, 976, 286)),
            Some(2)
        );
    }
}
