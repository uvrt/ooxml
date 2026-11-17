//! Owns: EMF+ objects -- brushes (solid, hatch, linear and path gradients, textures), pens,
//! paths, regions, fonts and string formats -- read from their serialised form, and the
//! object record that stores them in the 64-slot table (continued objects joined).
//!
//! Must not: draw; images are read in `image`.

use super::image::{parse_image, Image};
use super::reader::Data;
use super::Plus;
use crate::error::{Played, ReadError, RecordError};
use crate::pymath::round_even;
use ooxml_scene::{Figure, Matrix, Segment};
use std::sync::Arc;

// Line caps.
pub const CAP_FLAT: i32 = 0;
pub const CAP_SQUARE: i32 = 1;
pub const CAP_ROUND: i32 = 2;
pub const CAP_TRIANGLE: i32 = 3;
pub const CAP_SQUARE_ANCHOR: i32 = 0x11;
pub const CAP_ROUND_ANCHOR: i32 = 0x12;
pub const CAP_DIAMOND_ANCHOR: i32 = 0x13;
pub const CAP_ARROW_ANCHOR: i32 = 0x14;
pub const CAP_CUSTOM: i32 = 0xFF;

/// A linear gradient brush.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Linear {
    pub rect: [f64; 4],
    pub start: u32,
    pub end: u32,
    pub matrix: Matrix,
    /// `(position, ARGB)` stops along the rectangle's width.
    pub stops: Vec<(f64, u32)>,
}

/// A path gradient brush.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PathGradient {
    pub center_color: u32,
    pub center: (f64, f64),
    pub surround: Vec<u32>,
    /// The boundary in brush space: closed polygons.
    pub boundary: Vec<Vec<(f64, f64)>>,
    pub matrix: Matrix,
    /// `(position, factor)` from the boundary (0) to the centre (1).
    pub blend: Option<Vec<(f64, f64)>>,
    /// `(position, ARGB)`, replacing both colours.
    pub preset: Option<Vec<(f64, u32)>>,
    pub focus: f64,
}

/// A texture brush.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Texture {
    pub image: Arc<Image>,
    pub wrap: i32,
    pub matrix: Matrix,
}

/// A brush.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Brush {
    Solid(u32),
    Hatch { style: u32, fore: u32, back: u32 },
    Linear(Linear),
    PathGradient(PathGradient),
    Texture(Texture),
}

/// A pen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pen {
    pub width: f64,
    pub unit: u32,
    pub brush: Brush,
    pub start_cap: i32,
    pub end_cap: i32,
    pub join: u32,
    pub miter: f64,
    pub dash: i32,
    pub dash_cap: i32,
    pub dash_offset: f64,
    pub dash_array: Vec<f64>,
    pub align: i32,
    pub compound: Vec<f64>,
    pub matrix: Matrix,
}

/// A region's node.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Region {
    Combine(u32, Box<Region>, Box<Region>),
    Rect([f64; 4]),
    Path(Vec<Figure>),
    Empty,
    Infinite,
}

/// A font.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FontObj {
    pub em: f64,
    pub unit: u32,
    pub style: i32,
    pub family: String,
}

/// A string format.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct StringFormat {
    pub flags: u32,
    pub align: u32,
    pub line_align: u32,
    pub leading: f64,
    pub trailing: f64,
}

/// What a slot of the object table holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Object {
    Brush(Brush),
    Pen(Box<Pen>),
    Path(Vec<Figure>),
    Region(Region),
    Image(Arc<Image>),
    Font(FontObj),
    Format(StringFormat),
    /// One the player keeps a slot for and never draws with, or could not read.
    Other,
}

/// What a parser returns: an object, or `None` for one it cannot use.
type Parsed = Result<Option<Object>, RecordError>;

fn channels(v: u32) -> [u32; 4] {
    [(v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF, (v >> 24) & 0xFF]
}

/// An ARGB colour between two, `t` clamped to 0..1, each channel rounded half to even.
pub(crate) fn lerp_argb(c0: u32, c1: u32, t: f64) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = (channels(c0), channels(c1));
    let ch = |i: usize| {
        let (x, y) = (
            f64::from(a.get(i).copied().unwrap_or(0)),
            f64::from(b.get(i).copied().unwrap_or(0)),
        );
        round_even(x + (y - x) * t) as u32
    };
    (ch(3) << 24) | (ch(0) << 16) | (ch(1) << 8) | ch(2)
}

fn matrix(d: &mut Data<'_>) -> Result<Matrix, ReadError> {
    let [a, b, c, e, f, g] = d.floats_n::<6>()?;
    Ok(Matrix::new(a, b, c, e, f, g))
}

fn blend<T>(
    d: &mut Data<'_>,
    read: impl Fn(&mut Data<'_>, usize) -> Result<Vec<T>, ReadError>,
) -> Result<Vec<(f64, T)>, ReadError> {
    let n = d.count(8)?;
    let positions = d.floats(n)?;
    let values = read(d, n)?;
    Ok(positions.into_iter().zip(values).collect())
}

fn colours(d: &mut Data<'_>, n: usize) -> Result<Vec<u32>, ReadError> {
    (0..n).map(|_| d.u32()).collect()
}

fn sorted<T: Copy + PartialOrd>(mut v: Vec<(f64, T)>) -> Vec<(f64, T)> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v
}

/// Bézier and line figures flattened to points (Béziers in `steps` straight pieces).
pub(crate) fn flatten(fig: &Figure, steps: u32) -> Vec<(f64, f64)> {
    let mut pts = vec![fig.start];
    for seg in &fig.segments {
        match *seg {
            Segment::Line(x, y) => pts.push((x, y)),
            Segment::Cubic(a, b, c, e, f, g) => {
                let (x0, y0) = pts.last().copied().unwrap_or(fig.start);
                for j in 1..=steps {
                    let t = f64::from(j) / f64::from(steps);
                    let u = 1.0 - t;
                    pts.push((
                        u * u * u * x0 + 3.0 * u * u * t * a + 3.0 * u * t * t * c + t * t * t * f,
                        u * u * u * y0 + 3.0 * u * u * t * b + 3.0 * u * t * t * e + t * t * t * g,
                    ));
                }
            }
        }
    }
    pts
}

impl Plus<'_, '_> {
    pub(crate) fn parse_brush(&mut self, d: &mut Data<'_>) -> Result<Option<Brush>, RecordError> {
        let [_version, kind] = d.u32s::<2>()?;
        Ok(Some(match kind {
            0 => Brush::Solid(d.u32()?),
            1 => {
                let [style, fore, back] = d.u32s::<3>()?;
                Brush::Hatch { style, fore, back }
            }
            2 => {
                let (flags, wrap) = (d.u32()?, d.i32()?);
                let m = if flags & 0x02 != 0 {
                    matrix(d)?
                } else {
                    Matrix::IDENTITY
                };
                let Some(image) = parse_image(self, d)? else {
                    return Ok(None);
                };
                Brush::Texture(Texture { image, wrap, matrix: m })
            }
            3 => self.parse_path_gradient(d)?,
            4 => linear(self, d)?,
            _ => return Ok(None),
        }))
    }

    fn parse_path_gradient(&mut self, d: &mut Data<'_>) -> Result<Brush, RecordError> {
        let (flags, _wrap, center_color) = (d.u32()?, d.i32()?, d.u32()?);
        let [cx, cy] = d.floats_n::<2>()?;
        let n = d.count(4)?;
        let surround = colours(d, n)?;
        let boundary = if flags & 0x01 != 0 {
            let size = d.i32()?;
            let bytes = d.bytes_signed(size)?;
            let path = self.parse_path(&mut Data::new(bytes))?;
            path.iter().map(|f| flatten(f, 8)).collect()
        } else {
            let m = d.count(8)?;
            vec![d.points(m, false, false)?]
        };
        let m = if flags & 0x02 != 0 {
            matrix(d)?
        } else {
            Matrix::IDENTITY
        };
        let (mut blend_v, mut preset) = (None, None);
        if flags & 0x04 != 0 {
            preset = Some(sorted(blend(d, colours)?));
        } else if flags & 0x18 != 0 {
            blend_v = Some(blend(d, |d, n| d.floats(n))?);
        }
        let mut focus = 0.0;
        if flags & 0x40 != 0 {
            d.u32()?;
            let [fx, fy] = d.floats_n::<2>()?;
            focus = ((fx + fy) / 2.0).clamp(0.0, 0.999);
        }
        Ok(Brush::PathGradient(PathGradient {
            center_color,
            center: (cx, cy),
            surround,
            boundary,
            matrix: m,
            blend: blend_v,
            preset,
            focus,
        }))
    }

    pub(crate) fn parse_pen(&mut self, d: &mut Data<'_>) -> Result<Option<Pen>, RecordError> {
        let [_version, _kind, flags, unit] = d.u32s::<4>()?;
        let width = d.f32()?;
        let mut pen = Pen {
            width: width.abs(),
            unit,
            brush: Brush::Solid(0xFF00_0000),
            start_cap: CAP_FLAT,
            end_cap: CAP_FLAT,
            join: 0,
            miter: 10.0,
            dash: 0,
            dash_cap: CAP_FLAT,
            dash_offset: 0.0,
            dash_array: Vec::new(),
            align: 0,
            compound: Vec::new(),
            matrix: Matrix::IDENTITY,
        };
        let mut dash_set = false;
        if flags & 0x0001 != 0 {
            pen.matrix = matrix(d)?;
        }
        if flags & 0x0002 != 0 {
            pen.start_cap = d.i32()?;
        }
        if flags & 0x0004 != 0 {
            pen.end_cap = d.i32()?;
        }
        if flags & 0x0008 != 0 {
            pen.join = d.u32()?;
        }
        if flags & 0x0010 != 0 {
            pen.miter = d.f32()?;
        }
        if flags & 0x0020 != 0 {
            pen.dash = d.i32()?;
            dash_set = true;
        }
        if flags & 0x0040 != 0 {
            pen.dash_cap = d.i32()?;
        }
        if flags & 0x0080 != 0 {
            pen.dash_offset = d.f32()?;
        }
        if flags & 0x0100 != 0 {
            let n = d.count(4)?;
            pen.dash_array = d.floats(n)?;
            if !dash_set {
                pen.dash = 5;
            }
        }
        if flags & 0x0200 != 0 {
            pen.align = d.i32()?;
        }
        if flags & 0x0400 != 0 {
            let n = d.count(4)?;
            pen.compound = d.floats(n)?;
        }
        for bit in [0x0800, 0x1000] {
            if flags & bit != 0 {
                let size = d.u32()? as usize;
                d.bytes(size)?;
                if bit == 0x0800 {
                    pen.start_cap = CAP_CUSTOM;
                } else {
                    pen.end_cap = CAP_CUSTOM;
                }
            }
        }
        let Some(brush) = self.parse_brush(d)? else {
            return Ok(None);
        };
        pen.brush = brush;
        Ok(Some(pen))
    }

    pub(crate) fn parse_path(&mut self, d: &mut Data<'_>) -> Result<Vec<Figure>, RecordError> {
        let [_version, count, flags] = d.u32s::<3>()?;
        let count = count as usize;
        if count > d.left() {
            return Err(ReadError::CountPastEnd.into());
        }
        self.p.count_points(count as i64)?;
        let points = d.points(count, flags & 0x4000 != 0, flags & 0x0800 != 0)?;
        let mut types: Vec<u8> = Vec::new();
        if flags & 0x1000 != 0 {
            while types.len() < count && d.left() >= 2 {
                let (run, kind) = (d.u8()?, d.u8()?);
                types.extend(std::iter::repeat_n(kind, usize::from((run & 0x3F).max(1))));
            }
            types.truncate(count);
        } else {
            types = d.bytes(count.min(d.left()))?.to_vec();
        }
        Ok(figures_of(&points, &types))
    }

    pub(crate) fn parse_region(&mut self, d: &mut Data<'_>, depth: u32) -> Result<Region, RecordError> {
        if depth == 0 {
            d.u32s::<2>()?;
        }
        if depth > 64 {
            return Err(ReadError::TooDeep.into());
        }
        let kind = d.u32()?;
        Ok(match kind {
            1..=5 => {
                let left = self.parse_region(d, depth + 1)?;
                let right = self.parse_region(d, depth + 1)?;
                Region::Combine(kind, Box::new(left), Box::new(right))
            }
            0x1000_0000 => Region::Rect(d.floats_n::<4>()?),
            0x1000_0001 => {
                let size = d.i32()?;
                let bytes = d.bytes_signed(size)?;
                Region::Path(self.parse_path(&mut Data::new(bytes))?)
            }
            0x1000_0002 => Region::Empty,
            0x1000_0003 => Region::Infinite,
            _ => return Err(ReadError::UnknownNode.into()),
        })
    }

    fn parse_object(&mut self, kind: u32, data: &[u8]) -> Parsed {
        let mut d = Data::new(data);
        let d = &mut d;
        Ok(match kind {
            1 => self.parse_brush(d)?.map(Object::Brush),
            2 => self.parse_pen(d)?.map(|p| Object::Pen(Box::new(p))),
            3 => Some(Object::Path(self.parse_path(d)?)),
            4 => Some(Object::Region(self.parse_region(d, 0)?)),
            // An image that cannot be drawn is kept as such, not named unreadable.
            5 => Some(parse_image(self, d)?.map_or(Object::Other, Object::Image)),
            6 => {
                d.u32()?;
                let em = d.f32()?;
                let [unit, style, _reserved, length] = d.u32s::<4>()?;
                let raw = d.bytes((length as usize).min(64) * 2)?;
                let family = crate::bytes::utf16le(raw);
                let family = family.split('\0').next().unwrap_or("");
                Some(Object::Font(FontObj {
                    em,
                    unit,
                    style: style as i32,
                    family: crate::bytes::strip(family).to_owned(),
                }))
            }
            7 => {
                let [_version, flags, _language, align, line_align] = d.u32s::<5>()?;
                d.u32s::<2>()?;
                d.f32()?;
                d.i32()?;
                let [leading, trailing] = d.floats_n::<2>()?;
                Some(Object::Format(StringFormat {
                    flags,
                    align,
                    line_align,
                    leading,
                    trailing,
                }))
            }
            _ => Some(Object::Other),
        })
    }

    /// `EmfPlusObject`: an object into slot `flags & 0xFF`, its continuations joined.
    pub(crate) fn object(&mut self, flags: u16, data: &[u8]) -> Played {
        let index = u32::from(flags & 0xFF);
        let kind = u32::from((flags >> 8) & 0x7F);
        if index >= 64 {
            return Ok(false);
        }
        let joined: Vec<u8>;
        let data: &[u8] = if flags & 0x8000 != 0 {
            let total = crate::bytes::Bytes(data).u32(0)? as usize;
            let chunk = data.get(4..).unwrap_or(&[]);
            let held = self.pp.partial.get(&index).filter(|h| h.0 == kind).is_some();
            if !held {
                if total > self.p.limits.max_plus_object_bytes {
                    self.p.skip("an EmfPlus object larger than the cap");
                    return Ok(true);
                }
                self.pp.partial.insert(index, (kind, total, Vec::new()));
            }
            let Some(entry) = self.pp.partial.get_mut(&index) else {
                return Ok(true);
            };
            entry.2.extend_from_slice(chunk);
            if entry.2.len() < entry.1 {
                return Ok(true);
            }
            let (_, total, bytes) = self.pp.partial.remove(&index).unwrap_or_default();
            joined = bytes.get(..total).unwrap_or(&bytes).to_vec();
            &joined
        } else if let Some((_, _, held)) = self.pp.partial.remove(&index) {
            joined = [held.as_slice(), data].concat();
            &joined
        } else {
            data
        };
        if !(1..=7).contains(&kind) {
            self.pp.objects.insert(index, Object::Other);
            return Ok(true);
        }
        let parsed = match self.parse_object(kind, data) {
            Err(RecordError::Halt(h)) => return Err(h.into()),
            Err(RecordError::Read(_)) => None,
            Ok(obj) => obj,
        };
        match parsed {
            Some(obj) => {
                self.pp.objects.insert(index, obj);
            }
            None => {
                self.pp.objects.insert(index, Object::Other);
                let what = ["brush", "pen", "path", "region", "image", "font", "string format"];
                let what = what.get(kind as usize - 1).copied().unwrap_or("object");
                self.p.skip(format!("EmfPlusObject {what} (unreadable)"));
            }
        }
        Ok(true)
    }
}

fn linear(pp: &mut Plus<'_, '_>, d: &mut Data<'_>) -> Result<Brush, RecordError> {
    let (flags, _wrap) = (d.u32()?, d.i32()?);
    let rect = d.floats_n::<4>()?;
    let [start, end, c0, c1] = d.u32s::<4>()?;
    let m = if flags & 0x02 != 0 {
        matrix(d)?
    } else {
        Matrix::IDENTITY
    };
    // Windows draws the Reserved colours (GDI+ writes the start and end again there).
    let mut stops = vec![(0.0, c0), (1.0, c1)];
    if flags & 0x04 != 0 {
        stops = sorted(blend(d, colours)?);
    } else if flags & 0x18 != 0 {
        stops = sorted(blend(d, |d, n| d.floats(n))?)
            .into_iter()
            .map(|(p, f)| (p, lerp_argb(c0, c1, f)))
            .collect();
    }
    if flags & 0x80 != 0 {
        pp.p.skip("EmfPlus gamma-corrected gradients (drawn without gamma)");
    }
    let _ = (start, end);
    Ok(Brush::Linear(Linear {
        rect,
        start: c0,
        end: c1,
        matrix: m,
        stops,
    }))
}

/// A path's figures from its points and point types.
fn figures_of(points: &[(f64, f64)], types: &[u8]) -> Vec<Figure> {
    let mut figures: Vec<Figure> = Vec::new();
    let mut open = false;
    let mut k = 0;
    let n = points.len().min(types.len());
    while k < n {
        let kind = types.get(k).copied().unwrap_or(0) & 0x07;
        let (x, y) = points.get(k).copied().unwrap_or_default();
        if kind == 0 || !open {
            figures.push(Figure::at(x, y));
            open = true;
            k += 1;
        } else if kind == 3 && k + 2 < points.len() {
            let (a, b, c) = (
                points.get(k).copied(),
                points.get(k + 1).copied(),
                points.get(k + 2).copied(),
            );
            if let (Some(fig), Some(a), Some(b), Some(c)) = (figures.last_mut(), a, b, c) {
                fig.segments.push(Segment::Cubic(a.0, a.1, b.0, b.1, c.0, c.1));
            }
            k += 2;
            if types
                .get(k.min(types.len().saturating_sub(1)))
                .is_some_and(|t| t & 0x80 != 0)
            {
                if let Some(fig) = figures.last_mut() {
                    fig.closed = true;
                }
                open = false;
            }
            k += 1;
            continue;
        } else {
            if let Some(fig) = figures.last_mut() {
                fig.segments.push(Segment::Line(x, y));
            }
            k += 1;
        }
        if open && types.get(k - 1).is_some_and(|t| t & 0x80 != 0) {
            if let Some(fig) = figures.last_mut() {
                fig.closed = true;
            }
            open = false;
        }
    }
    figures
}
