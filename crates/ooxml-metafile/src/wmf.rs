//! Owns: a Windows Metafile's header and records (MS-WMF 2.3), the subset EMF shares.
//!
//! A placeable WMF states its box in logical units and how many make an inch; the box is
//! mapped onto the picture. The player stands where Windows' does when it plays a
//! placeable file into a rectangle: `MM_ANISOTROPIC`, the window the box, the viewport the
//! picture -- so the file's own window records map their window onto the picture, and its
//! `SETMAPMODE` and viewport records are not followed. A WMF without the placeable header
//! takes its first window as the box. Measured on Windows: pen widths are honoured,
//! `PS_NULL` draws nothing, and an ellipse leaves out its right and bottom pixel.
//!
//! Must not: draw (the player does).

use crate::bytes::{cp1252, strip, Bytes};
use crate::dc::{self, colorref, Brush, BrushImage, Font, Object, Pen};
use crate::dib;
use crate::emf::shapes::{draw_arc, read_points, ArcKind};
use crate::error::{Halt, Played, RecordError};
use crate::fonts::Faces;
use crate::geom;
use crate::player::image::{Blend, Blit};
use crate::player::text::TextCall;
use crate::player::Player;
use ooxml_base::MetafileLimits;
use ooxml_scene::Rgb;
use std::collections::BTreeSet;
use std::sync::Arc;

const PLACEABLE_KEY: u32 = 0x9AC6_CDD7;
const IGNORED: [u16; 11] = [
    0x0035, 0x0037, 0x0105, 0x0107, 0x0108, 0x0139, 0x020A, 0x0231, 0x0234, 0x0436, 0x0626,
];

/// Whether `data` is a WMF, placeable or not.
pub fn is_wmf(data: &[u8]) -> bool {
    let b = Bytes(data);
    if data.len() >= 22 && b.u32(0) == Ok(PLACEABLE_KEY) {
        return true;
    }
    data.len() >= 18 && matches!(b.u16(0), Ok(1 | 2)) && b.u16(2) == Ok(9) && matches!(b.u16(4), Ok(0x0100 | 0x0300))
}

fn name(function: u16) -> String {
    let n = match function {
        0x0000 => "EOF",
        0x0035 => "REALIZEPALETTE",
        0x0037 => "SETPALENTRIES",
        0x0102 => "SETBKMODE",
        0x0103 => "SETMAPMODE",
        0x0104 => "SETROP2",
        0x0105 => "SETRELABS",
        0x0106 => "SETPOLYFILLMODE",
        0x0107 => "SETSTRETCHBLTMODE",
        0x0108 => "SETTEXTCHAREXTRA",
        0x0127 => "RESTOREDC",
        0x012A => "INVERTREGION",
        0x012B => "PAINTREGION",
        0x012C => "SELECTCLIPREGION",
        0x012D => "SELECTOBJECT",
        0x012E => "SETTEXTALIGN",
        0x0139 => "RESIZEPALETTE",
        0x0142 => "DIBCREATEPATTERNBRUSH",
        0x014C => "SETLAYOUT",
        0x01F0 => "DELETEOBJECT",
        0x01F9 => "CREATEPATTERNBRUSH",
        0x0201 => "SETBKCOLOR",
        0x0209 => "SETTEXTCOLOR",
        0x020A => "SETTEXTJUSTIFICATION",
        0x020B => "SETWINDOWORG",
        0x020C => "SETWINDOWEXT",
        0x020D => "SETVIEWPORTORG",
        0x020E => "SETVIEWPORTEXT",
        0x020F => "OFFSETWINDOWORG",
        0x0211 => "OFFSETVIEWPORTORG",
        0x0213 => "LINETO",
        0x0214 => "MOVETO",
        0x0220 => "OFFSETCLIPRGN",
        0x0228 => "FILLREGION",
        0x0231 => "SETMAPPERFLAGS",
        0x0234 => "SELECTPALETTE",
        0x02FA => "CREATEPENINDIRECT",
        0x02FB => "CREATEFONTINDIRECT",
        0x02FC => "CREATEBRUSHINDIRECT",
        0x0324 => "POLYGON",
        0x0325 => "POLYLINE",
        0x0410 => "SCALEWINDOWEXT",
        0x0412 => "SCALEVIEWPORTEXT",
        0x0415 => "EXCLUDECLIPRECT",
        0x0416 => "INTERSECTCLIPRECT",
        0x0418 => "ELLIPSE",
        0x0419 => "FLOODFILL",
        0x041B => "RECTANGLE",
        0x041F => "SETPIXEL",
        0x0429 => "FRAMEREGION",
        0x0436 => "ANIMATEPALETTE",
        0x0521 => "TEXTOUT",
        0x0538 => "POLYPOLYGON",
        0x0548 => "EXTFLOODFILL",
        0x061C => "ROUNDRECT",
        0x061D => "PATBLT",
        0x0626 => "ESCAPE",
        0x06FF => "CREATEREGION",
        0x0817 => "ARC",
        0x081A => "PIE",
        0x0830 => "CHORD",
        0x0922 => "BITBLT",
        0x0940 => "DIBBITBLT",
        0x0A32 => "EXTTEXTOUT",
        0x0B23 => "STRETCHBLT",
        0x0B41 => "DIBSTRETCHBLT",
        0x0D33 => "SETDIBTODEV",
        0x0F43 => "STRETCHDIB",
        0x001E => "SAVEDC",
        0x00F7 => "CREATEPALETTE",
        _ => return format!("0x{function:04X}"),
    };
    n.to_owned()
}

/// One playback's result: the player and the picture's natural size (pixels at 96 to the
/// inch).
pub(crate) struct Played2<'a> {
    pub player: Player<'a>,
    pub size: (f64, f64),
}

/// Plays WMF bytes: `None` when they are not a WMF (or are larger than the cap).
pub(crate) fn play<'a>(data: &[u8], faces: &'a Faces, limits: MetafileLimits) -> Option<Played2<'a>> {
    if !is_wmf(data) || data.len() > limits.max_input_bytes {
        return None;
    }
    let b = Bytes(data);
    let mut start = 0usize;
    let mut box_ = None;
    let mut inch = 1440u16;
    if b.u32(0) == Ok(PLACEABLE_KEY) {
        let [left, top, right, bottom] = [6, 8, 10, 12].map(|at| i32::from(b.i16(at).unwrap_or(0)));
        inch = b.u16(14).ok()?;
        if right != left && bottom != top {
            box_ = Some(
                [
                    left.min(right),
                    top.min(bottom),
                    (right - left).abs(),
                    (bottom - top).abs(),
                ]
                .map(f64::from),
            );
        }
        if inch == 0 {
            inch = 1440;
        }
        start = 22;
    }
    if data.len() < start + 18 {
        return None;
    }
    let first = start + usize::from(b.u16(start + 2).ok()?) * 2;
    let [x, y, w, h] = match box_ {
        Some(bx) => bx,
        None => first_window(data, first, limits.max_records)?,
    };
    let mut player = Player::new(faces, limits, (96.0 / 25.4, 96.0 / 25.4), (x, y, w, h));
    player.dc.map_mode = dc::MM_ANISOTROPIC;
    player.dc.window_org = (x, y);
    player.dc.window_ext = (w, h);
    player.dc.viewport_org = (x, y);
    player.dc.viewport_ext = (w, h);
    let mut reader = Reader {
        p: player,
        slots: BTreeSet::new(),
    };
    let _ = reader.run(data, first);
    let inch = f64::from(inch);
    Some(Played2 {
        player: reader.p,
        size: (w / inch * 96.0, h / inch * 96.0),
    })
}

fn first_window(data: &[u8], mut offset: usize, max_records: usize) -> Option<[f64; 4]> {
    let b = Bytes(data);
    let mut org = (0.0, 0.0);
    for _ in 0..max_records {
        if offset + 6 > data.len() {
            break;
        }
        let (words, function) = (b.u32(offset).ok()?, b.u16(offset + 4).ok()?);
        if words < 3 || function == 0 {
            break;
        }
        if (function == 0x020B || function == 0x020C) && offset + 10 <= data.len() {
            let (yy, xx) = (f64::from(b.i16(offset + 6).ok()?), f64::from(b.i16(offset + 8).ok()?));
            if function == 0x020B {
                org = (xx, yy);
            } else if xx != 0.0 && yy != 0.0 {
                return Some([org.0, org.1, xx, yy]);
            }
        }
        offset = offset.saturating_add(words as usize * 2);
    }
    None
}

struct Reader<'a> {
    p: Player<'a>,
    slots: BTreeSet<u32>,
}

impl Reader<'_> {
    fn run(&mut self, data: &[u8], mut offset: usize) -> Result<(), Halt> {
        let b = Bytes(data);
        for _ in 0..self.p.limits.max_records {
            if offset + 6 > data.len() {
                return Ok(());
            }
            let (Ok(words), Ok(function)) = (b.u32(offset), b.u16(offset + 4)) else {
                return Ok(());
            };
            let size = words as usize * 2;
            if size < 6 || offset.saturating_add(size) > data.len() {
                self.p
                    .skip("a record whose size runs past the file (the rest is not read)");
                return Ok(());
            }
            if function == 0 {
                return Ok(());
            }
            self.record(function, data.get(offset + 6..offset + size).unwrap_or(&[]))?;
            offset += size;
        }
        self.p.truncated = true;
        Ok(())
    }

    fn record(&mut self, function: u16, params: &[u8]) -> Result<(), Halt> {
        if IGNORED.contains(&function) {
            return Ok(());
        }
        let name = format!("META_{}", name(function));
        if matches!(function, 0x00F7 | 0x01F9 | 0x06FF) {
            self.add(Object::Slot);
            return Ok(());
        }
        match self.handle(function, Bytes(params)) {
            Some(Ok(true)) => Ok(()),
            Some(Ok(false)) | None => {
                self.p.skip(name);
                Ok(())
            }
            Some(Err(RecordError::Halt(h))) => Err(h),
            Some(Err(RecordError::Read(_))) => {
                self.p.skip(format!("{name} (unreadable)"));
                Ok(())
            }
        }
    }

    /// A WMF object takes the lowest free slot.
    fn add(&mut self, obj: Object) {
        let mut index = 0u32;
        while self.slots.contains(&index) {
            index += 1;
        }
        if index >= self.p.limits.max_objects {
            return;
        }
        self.slots.insert(index);
        self.p.create(index, obj);
    }

    fn handle(&mut self, function: u16, p: Bytes<'_>) -> Option<Played> {
        Some(match function {
            0x0103 => Ok(true),
            0x020B | 0x020C | 0x020F | 0x0410 => self.window(function, p),
            0x020D | 0x020E | 0x0211 | 0x0412 => Ok(false),
            0x0102 | 0x0106 | 0x0104 | 0x012E | 0x0201 | 0x0209 | 0x014C => self.state(function, p),
            0x001E => {
                self.p.save_dc();
                Ok(true)
            }
            0x0127 | 0x012D | 0x01F0 => self.slot_record(function, p),
            0x02FA | 0x02FC | 0x0142 | 0x02FB => self.object(function, p),
            0x0214 | 0x0213 => self.move_or_line(function, p),
            0x041B | 0x0418 | 0x061C => self.box_shape(function, p),
            0x0324 | 0x0325 | 0x0538 => self.poly(function, p),
            0x0817 | 0x0830 | 0x081A => self.arc(function, p),
            0x041F => self.pixel(p),
            0x0416 | 0x0415 => self.clip(function, p),
            0x061D | 0x0F43 | 0x0B41 | 0x0940 => self.bitmap(function, p),
            0x0521 | 0x0A32 => self.text(function, p),
            _ => return None,
        })
    }

    fn slot_record(&mut self, function: u16, p: Bytes<'_>) -> Played {
        match function {
            0x0127 => self.p.restore_dc(i64::from(p.i16(0)?))?,
            0x012D => self.p.select(u32::from(p.u16(0)?))?,
            _ => {
                let index = u32::from(p.u16(0)?);
                self.slots.remove(&index);
                self.p.delete(index);
            }
        }
        Ok(true)
    }

    fn h(p: Bytes<'_>, at: usize) -> Result<f64, crate::error::ReadError> {
        Ok(f64::from(p.i16(at)?))
    }

    fn rect(p: Bytes<'_>, at: usize) -> Result<[f64; 4], crate::error::ReadError> {
        let (bottom, right, top, left) = (
            Self::h(p, at)?,
            Self::h(p, at + 2)?,
            Self::h(p, at + 4)?,
            Self::h(p, at + 6)?,
        );
        Ok([left, top, right, bottom])
    }

    fn window(&mut self, function: u16, p: Bytes<'_>) -> Played {
        let dc = &mut self.p.dc;
        if function == 0x0410 {
            let (yd, yn, xd, xn) = (Self::h(p, 0)?, Self::h(p, 2)?, Self::h(p, 4)?, Self::h(p, 6)?);
            if xd != 0.0 && yd != 0.0 && xn != 0.0 && yn != 0.0 {
                let (ex, ey) = dc.window_ext;
                dc.window_ext = (ex * xn / xd, ey * yn / yd);
            }
            return Ok(true);
        }
        let (y, x) = (Self::h(p, 0)?, Self::h(p, 2)?);
        match function {
            0x020B => dc.window_org = (x, y),
            0x020C if x != 0.0 && y != 0.0 => dc.window_ext = (x, y),
            0x020F => dc.window_org = (dc.window_org.0 + x, dc.window_org.1 + y),
            _ => {}
        }
        Ok(true)
    }

    fn state(&mut self, function: u16, p: Bytes<'_>) -> Played {
        match function {
            0x0201 | 0x0209 => {
                let c = colorref(p.u32(0)?);
                if function == 0x0201 {
                    self.p.dc.bk_color = c;
                } else {
                    self.p.dc.text_color = c;
                }
                return Ok(true);
            }
            _ => {}
        }
        let v = u32::from(p.u16(0)?);
        match function {
            0x0102 => self.p.dc.bk_mode = v,
            0x0106 => self.p.dc.fill_mode = if v == 2 { dc::WINDING } else { dc::ALTERNATE },
            0x0104 => {
                self.p.flush()?;
                self.p.dc.rop2 = v;
                if ![1, 11, 13, 16].contains(&v) {
                    self.p.skip(format!("META_SETROP2 mode {v} (drawn as R2_COPYPEN)"));
                }
            }
            0x012E => self.p.dc.text_align = v,
            _ => return Ok(v == 0),
        }
        Ok(true)
    }

    fn object(&mut self, function: u16, p: Bytes<'_>) -> Played {
        match function {
            0x02FA => {
                let (style, width, color) = (u32::from(p.u16(0)?), i32::from(p.i16(2)?), p.u32(6)?);
                let width = width.abs();
                self.add(Object::Pen(Pen {
                    style,
                    width: f64::from(width),
                    color: colorref(color),
                    geometric: width > 1,
                    plain: true,
                    user_style: Vec::new(),
                    null: style & 0x0F == dc::PS_NULL,
                }));
                Ok(true)
            }
            0x02FC => {
                let (style, color) = (p.u16(0)?, colorref(p.u32(2)?));
                let hatch = if p.len() >= 8 { u32::from(p.u16(6)?) } else { 0 };
                let (brush, known) = match style {
                    1 => (Brush::Null, true),
                    2 => (Brush::Hatch { color, hatch }, true),
                    s => (Brush::Solid(color), s == 0),
                };
                self.add(Object::Brush(brush));
                Ok(known)
            }
            0x0142 => Ok(self.pattern_brush(p)),
            _ => self.font(p),
        }
    }

    fn pattern_brush(&mut self, p: Bytes<'_>) -> bool {
        let mut brush = None;
        if let Some((header, bits)) = dib::split_packed(p.slice(4, p.len())) {
            if let Some(image) = dib::to_image(header, bits).filter(|i| i.mime == "image/png") {
                let head = Bytes(header);
                if let (Ok(w), Ok(h)) = (head.i32(4), head.i32(8)) {
                    brush = Some(Brush::Pattern(Arc::new(BrushImage {
                        serial: self.p.serial(),
                        png: image.data,
                        width: i64::from(w).abs(),
                        height: i64::from(h).abs(),
                    })));
                }
            }
        }
        let found = brush.is_some();
        self.add(Object::Brush(brush.unwrap_or(Brush::Solid(Rgb(0x80, 0x80, 0x80)))));
        found
    }

    fn font(&mut self, p: Bytes<'_>) -> Played {
        let v = [0, 2, 4, 6, 8].map(|at| p.i16(at));
        let [height, _w, escapement, _o, weight] = [v[0]?, v[1]?, v[2]?, v[3]?, v[4]?];
        let flags = p.many(10, 8, 1, Bytes::u8)?;
        let raw = p.slice(18, 50);
        let raw = raw.split(|&c| c == 0).next().unwrap_or(&[]);
        let face = cp1252(raw);
        let flag = |i: usize| flags.get(i).copied().unwrap_or(0);
        self.add(Object::Font(Font {
            height: f64::from(height),
            escapement: f64::from(escapement),
            weight: i64::from(weight),
            italic: flag(0) != 0,
            underline: flag(1) != 0,
            strike: flag(2) != 0,
            face: strip(&face).to_owned(),
            pitch_family: flag(7),
        }));
        Ok(true)
    }

    fn move_or_line(&mut self, function: u16, p: Bytes<'_>) -> Played {
        let (y, x) = (Self::h(p, 0)?, Self::h(p, 2)?);
        if function == 0x0214 {
            self.p.move_to(x, y);
        } else {
            self.p.line_to(x, y)?;
        }
        Ok(true)
    }

    fn box_shape(&mut self, function: u16, p: Bytes<'_>) -> Played {
        let fig = match function {
            0x041B => {
                let [l, t, r, b] = Self::rect(p, 0)?;
                geom::rect(l, t, r, b)
            }
            0x0418 => {
                // Measured on Windows: a WMF ellipse leaves out its right and bottom pixel.
                let [l, t, r, b] = Self::rect(p, 0)?;
                let r = r + if r > l { -1.0 } else { 1.0 };
                let b = b + if b > t { -1.0 } else { 1.0 };
                geom::ellipse(l, t, r, b)
            }
            _ => {
                let (h, w) = (Self::h(p, 0)?, Self::h(p, 2)?);
                let [l, t, r, b] = Self::rect(p, 4)?;
                geom::round_rect(l, t, r, b, w, h)
            }
        };
        self.p.shape(vec![Some(fig)], true)?;
        Ok(true)
    }

    fn points(&mut self, p: Bytes<'_>, at: usize, count: usize) -> Result<Vec<(f64, f64)>, RecordError> {
        let count = (count as i64).min((p.len() as i64 - at as i64).div_euclid(4));
        self.p.count_points(count)?;
        Ok(read_points(p, at, usize::try_from(count).unwrap_or(0), true)?)
    }

    fn poly(&mut self, function: u16, p: Bytes<'_>) -> Played {
        if function == 0x0538 {
            let polys = usize::from(p.u16(0)?).min(p.len().saturating_sub(2) / 2);
            let counts = p.many(2, polys, 2, Bytes::u16)?;
            let total = counts.iter().map(|&c| usize::from(c)).sum();
            let points = self.points(p, 2 + 2 * polys, total)?;
            let mut figures = Vec::new();
            let mut k = 0usize;
            for n in counts {
                let end = k + usize::from(n);
                let chunk = points.get(k.min(points.len())..end.min(points.len())).unwrap_or(&[]);
                if !chunk.is_empty() {
                    figures.push(geom::polyline(chunk, true));
                }
                k = end;
            }
            self.p.shape(figures, true)?;
            return Ok(true);
        }
        let count = usize::from(p.u16(0)?);
        let points = self.points(p, 2, count)?;
        if function == 0x0324 {
            self.p.shape(vec![geom::polyline(&points, true)], true)?;
        } else {
            self.p.paint(
                geom::polyline(&points, false).into_iter().collect(),
                false,
                true,
                None,
                None,
            )?;
        }
        Ok(true)
    }

    fn arc(&mut self, function: u16, p: Bytes<'_>) -> Played {
        let (ye, xe, ys, xs) = (Self::h(p, 0)?, Self::h(p, 2)?, Self::h(p, 4)?, Self::h(p, 6)?);
        let kind = match function {
            0x0817 => ArcKind::Arc,
            0x0830 => ArcKind::Chord,
            _ => ArcKind::Pie,
        };
        draw_arc(&mut self.p, kind, Self::rect(p, 8)?, [xs, ys, xe, ye])?;
        Ok(true)
    }

    fn pixel(&mut self, p: Bytes<'_>) -> Played {
        let color = p.u32(0)?;
        let (y, x) = (Self::h(p, 4)?, Self::h(p, 6)?);
        self.p.pixel(x, y, color)?;
        Ok(true)
    }

    fn clip(&mut self, function: u16, p: Bytes<'_>) -> Played {
        let [l, t, r, b] = Self::rect(p, 0)?;
        self.p.clip_rect(l, t, r, b, function == 0x0415)?;
        Ok(true)
    }

    fn bitmap(&mut self, function: u16, p: Bytes<'_>) -> Played {
        let rop = p.u32(0)?;
        let h = |at| Self::h(p, at);
        let (blit, dib_at) = match function {
            0x061D => {
                let (hh, w, y, x) = (h(4)?, h(6)?, h(8)?, h(10)?);
                let blit = Blit {
                    dest: (x, y, w, hh),
                    src: (0.0, 0.0, 0.0, 0.0),
                };
                self.p.image(blit, &[], &[], rop, Blend::default())?;
                return Ok(true);
            }
            0x0F43 | 0x0B41 => {
                let base = if function == 0x0F43 { 6 } else { 4 };
                let v: Vec<f64> = (0..8).map(|k| h(base + 2 * k)).collect::<Result<_, _>>()?;
                let [sh, sw, sy, sx, dh, dw, dy, dx] = <[f64; 8]>::try_from(v).unwrap_or([0.0; 8]);
                let blit = Blit {
                    dest: (dx, dy, dw, dh),
                    src: (sx, sy, sw, sh),
                };
                (blit, if function == 0x0F43 { 22 } else { 20 })
            }
            _ => {
                let v: Vec<f64> = (0..6).map(|k| h(4 + 2 * k)).collect::<Result<_, _>>()?;
                let [sy, sx, hh, w, dy, dx] = <[f64; 6]>::try_from(v).unwrap_or([0.0; 6]);
                let blit = Blit {
                    dest: (dx, dy, w, hh),
                    src: (sx, sy, w, hh),
                };
                (blit, 16)
            }
        };
        let (header, bits) = dib::split_packed(p.slice(dib_at, p.len())).unwrap_or((&[], &[]));
        self.p.image(blit, header, bits, rop, Blend::default())?;
        Ok(true)
    }

    fn text(&mut self, function: u16, p: Bytes<'_>) -> Played {
        if function == 0x0521 {
            let count = usize::from(p.u16(0)?);
            let raw = p.slice(2, 2 + count);
            let at = 2 + count + (count & 1);
            let (y, x) = (Self::h(p, at)?, Self::h(p, at + 2)?);
            let text = cp1252(raw);
            self.p.text(TextCall {
                x,
                y,
                text: &text,
                dx: None,
                options: 0,
                rect: None,
            })?;
            return Ok(true);
        }
        let (y, x, count, options) = (
            Self::h(p, 0)?,
            Self::h(p, 2)?,
            i64::from(p.i16(4)?),
            u32::from(p.u16(6)?),
        );
        let mut at = 8usize;
        let mut rect = None;
        if options & (dc::ETO_OPAQUE | dc::ETO_CLIPPED) != 0 {
            let (l, t, r, b) = (
                Self::h(p, at)?,
                Self::h(p, at + 2)?,
                Self::h(p, at + 4)?,
                Self::h(p, at + 6)?,
            );
            rect = Some((l, t, r, b));
            at += 8;
        }
        let count = usize::try_from(count.min(p.len() as i64 - at as i64).max(0)).unwrap_or(0);
        let raw = p.slice(at, at + count);
        at += count + (count & 1);
        let mut dx = None;
        if p.len() >= at + 2 * count && count > 0 {
            dx = Some(p.many(at, count, 2, |b, o| Ok(i64::from(b.i16(o)?)))?);
        }
        let text = cp1252(raw);
        self.p.text(TextCall {
            x,
            y,
            text: &text,
            dx,
            options,
            rect,
        })?;
        Ok(true)
    }
}
