//! Owns: EMF+ record names and the handlers of its drawing, transform, state and clip
//! records.
//!
//! Must not: paint itself (`paint`, `stroke`, `shade`, `text`, `image` do).

use super::objects::{Object, StringFormat};
use super::reader::Data;
use super::{Plus, SaveKey, State, U_DISPLAY, U_WORLD};
use crate::error::Played;
use crate::geom;
use crate::pymath::radians;
use ooxml_scene::{Figure, Matrix, Segment};

const NAMES: [&str; 59] = [
    "",
    "Header",
    "EndOfFile",
    "Comment",
    "GetDC",
    "MultiFormatStart",
    "MultiFormatSection",
    "MultiFormatEnd",
    "Object",
    "Clear",
    "FillRects",
    "DrawRects",
    "FillPolygon",
    "DrawLines",
    "FillEllipse",
    "DrawEllipse",
    "FillPie",
    "DrawPie",
    "DrawArc",
    "FillRegion",
    "FillPath",
    "DrawPath",
    "FillClosedCurve",
    "DrawClosedCurve",
    "DrawCurve",
    "DrawBeziers",
    "DrawImage",
    "DrawImagePoints",
    "DrawString",
    "SetRenderingOrigin",
    "SetAntiAliasMode",
    "SetTextRenderingHint",
    "SetTextContrast",
    "SetInterpolationMode",
    "SetPixelOffsetMode",
    "SetCompositingMode",
    "SetCompositingQuality",
    "Save",
    "Restore",
    "BeginContainer",
    "BeginContainerNoParams",
    "EndContainer",
    "SetWorldTransform",
    "ResetWorldTransform",
    "MultiplyWorldTransform",
    "TranslateWorldTransform",
    "ScaleWorldTransform",
    "RotateWorldTransform",
    "SetPageTransform",
    "ResetClip",
    "SetClipRect",
    "SetClipPath",
    "SetClipRegion",
    "OffsetClip",
    "DrawDriverString",
    "StrokeFillPath",
    "SerializableObject",
    "SetTSGraphics",
    "SetTSClip",
];

/// A record type's name, or `Record 0x....`.
pub fn name(kind: u16) -> String {
    match kind.checked_sub(0x4000).and_then(|i| NAMES.get(usize::from(i))) {
        Some(n) if !n.is_empty() => (*n).to_owned(),
        _ => format!("Record 0x{kind:04X}"),
    }
}

/// Whether a handler reads the record type.
pub fn handled(kind: u16) -> bool {
    matches!(kind, 0x4008..=0x401D | 0x4021 | 0x4023 | 0x4025..=0x4036)
}

fn rect_figure(r: [f64; 4]) -> Figure {
    let [x, y, w, h] = r;
    geom::rect(x, y, x + w, y + h)
}

impl Plus<'_, '_> {
    pub(super) fn handle(&mut self, kind: u16, flags: u16, data: &[u8]) -> Played {
        let d = &mut Data::new(data);
        match kind {
            0x4008 => self.object(flags, data),
            0x4009 => self.clear(d),
            0x400A | 0x400B => self.rects(kind == 0x400A, flags, d),
            0x400C | 0x400D => self.poly(kind == 0x400C, flags, d),
            0x400E | 0x400F => self.ellipse(kind == 0x400E, flags, d),
            0x4010..=0x4012 => self.pie(kind, flags, d),
            // The reference keeps a region object as its node, which no handler accepts:
            // a region is never drawn or clipped to (kept equal).
            0x4013 => d.u32().map(|_| false).map_err(Into::into),
            0x4034 => Ok(false),
            0x4014 | 0x4015 => self.path(kind == 0x4014, flags, d),
            0x4016..=0x4018 => self.curve(kind, flags, d),
            0x4019 => self.beziers(flags, d),
            0x401A | 0x401B => self.image_record(kind == 0x401B, flags, d),
            0x401C => self.string_record(flags, d),
            0x4036 => self.driver_string_record(flags, d),
            0x401D => {
                let (x, y) = (d.i32()?, d.i32()?);
                self.pp.state.origin = (f64::from(x), f64::from(y));
                Ok(true)
            }
            0x4021 => {
                self.pp.state.interpolation = u32::from(flags & 0xFF);
                Ok(true)
            }
            0x4023 => {
                self.pp.state.compositing = u32::from(flags & 0xFF);
                Ok(true)
            }
            0x4025..=0x4029 => self.save_restore(kind, flags, d),
            0x402A..=0x4030 => self.transform(kind, flags, d),
            0x4031..=0x4033 | 0x4035 => self.clip_record(kind, flags, d),
            _ => Ok(false),
        }
    }

    fn clear(&mut self, d: &mut Data<'_>) -> Played {
        let colour = d.u32()?;
        let (x, y, w, h) = self.p.canvas;
        let node = super::paint::colour_path(vec![geom::rect(x, y, x + w, y + h)], None, colour, "fill");
        self.write(node)?;
        Ok(true)
    }

    fn rects(&mut self, fill: bool, flags: u16, d: &mut Data<'_>) -> Played {
        let brush = if fill { self.brush(flags, d.u32()?) } else { None };
        let compressed = flags & 0x4000 != 0;
        let n = d.count(if compressed { 8 } else { 16 })?;
        self.p.count_points(4 * n as i64)?;
        let figures: Vec<Figure> = d.rects(n, compressed)?.into_iter().map(rect_figure).collect();
        if fill {
            self.fill(figures, brush, false)?;
        } else {
            let pen = self.pen(u32::from(flags));
            self.stroke(figures, pen)?;
        }
        Ok(true)
    }

    fn poly_points(&mut self, flags: u16, d: &mut Data<'_>) -> Result<Vec<(f64, f64)>, crate::error::RecordError> {
        let (compressed, relative) = (flags & 0x4000 != 0, flags & 0x0800 != 0);
        let n = d.count(if relative {
            1
        } else if compressed {
            4
        } else {
            8
        })?;
        self.p.count_points(n as i64)?;
        Ok(d.points(n, compressed, relative)?)
    }

    fn poly(&mut self, fill: bool, flags: u16, d: &mut Data<'_>) -> Played {
        let brush = if fill { self.brush(flags, d.u32()?) } else { None };
        let points = self.poly_points(flags, d)?;
        if points.len() >= 2 {
            if fill {
                self.fill(geom::polyline(&points, true).into_iter().collect(), brush, true)?;
            } else {
                let pen = self.pen(u32::from(flags));
                self.stroke(geom::polyline(&points, flags & 0x2000 != 0).into_iter().collect(), pen)?;
            }
        }
        Ok(true)
    }

    fn ellipse(&mut self, fill: bool, flags: u16, d: &mut Data<'_>) -> Played {
        let brush = if fill { self.brush(flags, d.u32()?) } else { None };
        let [x, y, w, h] = d.rects(1, flags & 0x4000 != 0)?.first().copied().unwrap_or_default();
        let fig = geom::ellipse(x, y, x + w, y + h);
        if fill {
            self.fill(vec![fig], brush, false)?;
        } else {
            let pen = self.pen(u32::from(flags));
            self.stroke(vec![fig], pen)?;
        }
        Ok(true)
    }

    fn pie(&mut self, kind: u16, flags: u16, d: &mut Data<'_>) -> Played {
        let brush = if kind == 0x4010 {
            self.brush(flags, d.u32()?)
        } else {
            None
        };
        let [start, sweep] = d.floats_n::<2>()?;
        let rect = d.rects(1, flags & 0x4000 != 0)?.first().copied().unwrap_or_default();
        let fig = pie_figure(start, sweep, rect, kind != 0x4012)?;
        if kind == 0x4010 {
            self.fill(vec![fig], brush, false)?;
        } else {
            let pen = self.pen(u32::from(flags));
            self.stroke(vec![fig], pen)?;
        }
        Ok(true)
    }

    fn path_object(&self, flags: u16) -> Option<Vec<Figure>> {
        match self.pp.objects.get(&u32::from(flags & 0xFF)) {
            Some(Object::Path(figures)) => Some(figures.clone()),
            _ => None,
        }
    }

    fn path(&mut self, fill: bool, flags: u16, d: &mut Data<'_>) -> Played {
        let brush = if fill { self.brush(flags, d.u32()?) } else { None };
        let Some(figures) = self.path_object(flags) else {
            return Ok(false);
        };
        if fill {
            let closed = figures.into_iter().map(|f| Figure { closed: true, ..f }).collect();
            self.fill(closed, brush, true)?;
        } else {
            let pen = self.pen(d.u32()?);
            self.stroke(figures, pen)?;
        }
        Ok(true)
    }

    fn curve(&mut self, kind: u16, flags: u16, d: &mut Data<'_>) -> Played {
        let brush = if kind == 0x4016 {
            self.brush(flags, d.u32()?)
        } else {
            None
        };
        let tension = d.f32()?;
        let (offset, segments) = if kind == 0x4018 {
            (d.u32()? as usize, d.u32()? as usize)
        } else {
            (0, 0)
        };
        let points = self.poly_points(flags, d)?;
        if kind == 0x4018 {
            if points.len() < 2 {
                return Ok(true);
            }
            let beziers = cardinal(&points, tension, false);
            let offset = offset.min(beziers.len());
            let chosen: Vec<Segment> = beziers.into_iter().skip(offset).take(segments).collect();
            let Some(&(x, y)) = points.get(offset).filter(|_| !chosen.is_empty()) else {
                return Ok(true);
            };
            let fig = Figure {
                start: (x, y),
                segments: chosen,
                closed: false,
            };
            let pen = self.pen(u32::from(flags));
            self.stroke(vec![fig], pen)?;
            return Ok(true);
        }
        let Some(&(x, y)) = points.first().filter(|_| points.len() >= 3) else {
            return Ok(true);
        };
        let fig = Figure {
            start: (x, y),
            segments: cardinal(&points, tension, true),
            closed: true,
        };
        if kind == 0x4016 {
            self.fill(vec![fig], brush, flags & 0x2000 == 0)?;
        } else {
            let pen = self.pen(u32::from(flags));
            self.stroke(vec![fig], pen)?;
        }
        Ok(true)
    }

    fn beziers(&mut self, flags: u16, d: &mut Data<'_>) -> Played {
        let points = self.poly_points(flags, d)?;
        if let Some(fig) = geom::beziers(&points).filter(|f| !f.segments.is_empty()) {
            let pen = self.pen(u32::from(flags));
            self.stroke(vec![fig], pen)?;
        }
        Ok(true)
    }

    fn string_record(&mut self, flags: u16, d: &mut Data<'_>) -> Played {
        let font = self.font_object(flags);
        let brush = self.brush(flags, d.u32()?);
        let (fmt_id, length) = (d.u32()?, d.u32()? as usize);
        let rect = d.floats_n::<4>()?;
        let length = length.min(d.left() / 2).min(self.p.limits.max_text_chars);
        let text = crate::bytes::utf16le(d.bytes(2 * length)?);
        let fmt = match self.pp.objects.get(&(fmt_id & 0xFF)).filter(|_| fmt_id < 64) {
            Some(Object::Format(f)) => *f,
            _ => StringFormat::default(),
        };
        let Some(font) = font else {
            return Ok(false);
        };
        self.draw_string(&font, brush, fmt, rect, &text)?;
        Ok(true)
    }

    fn driver_string_record(&mut self, flags: u16, d: &mut Data<'_>) -> Played {
        let font = self.font_object(flags);
        let brush = self.brush(flags, d.u32()?);
        let [options, matrix_present, count] = d.u32s::<3>()?;
        let count = (count as usize).min(d.left() / 10).min(self.p.limits.max_text_chars);
        let glyphs: Vec<u16> = (0..count).map(|_| d.u16()).collect::<Result<_, _>>()?;
        let positions = d.points(count, false, false)?;
        let matrix = if matrix_present != 0 && d.left() >= 24 {
            let [a, b, c, e, f, g] = d.floats_n::<6>()?;
            Some(Matrix::new(a, b, c, e, f, g))
        } else {
            None
        };
        let Some(font) = font else {
            return Ok(false);
        };
        self.draw_driver_string(&font, brush, options, &glyphs, &positions, matrix)
    }

    fn font_object(&self, flags: u16) -> Option<super::objects::FontObj> {
        match self.pp.objects.get(&u32::from(flags & 0xFF)) {
            Some(Object::Font(f)) => Some(f.clone()),
            _ => None,
        }
    }

    fn save_restore(&mut self, kind: u16, flags: u16, d: &mut Data<'_>) -> Played {
        match kind {
            0x4025 => {
                let index = d.u32()?;
                if self.pp.saves.len() >= self.p.limits.max_save_depth {
                    self.p.skip("EmfPlusSave beyond the depth cap");
                    return Ok(true);
                }
                self.pp.order += 1;
                let entry = (SaveKey::Save(index), self.pp.order, self.pp.state.clone());
                self.put_save(entry);
            }
            0x4026 => {
                let index = d.u32()?;
                let Some((_, order, state)) = self.pp.saves.iter().find(|s| s.0 == SaveKey::Save(index)).cloned()
                else {
                    return Ok(true);
                };
                self.pp.state = state;
                self.pp.saves.retain(|s| s.1 < order);
            }
            0x4027 => {
                let dst = d.floats_n::<4>()?;
                let src = d.floats_n::<4>()?;
                let index = d.u32()?;
                let unit = u32::from(flags >> 8);
                let outer = self.pp.state.clone();
                let mut base = self.matrix();
                let [sx, sy, sw, sh] = src;
                if unit != U_WORLD && unit != U_DISPLAY && sw != 0.0 && sh != 0.0 {
                    let k = self.unit(unit, 0);
                    let [dx, dy, dw, dh] = dst.map(|v| v * k);
                    let mapping = Matrix::new(dw / sw, 0.0, 0.0, dh / sh, dx - sx * dw / sw, dy - sy * dh / sh);
                    base = mapping.then(&base);
                }
                self.push_container(index, outer, base);
            }
            0x4028 => {
                let index = d.u32()?;
                let (outer, base) = (self.pp.state.clone(), self.matrix());
                self.push_container(index, outer, base);
            }
            _ => {
                let index = d.u32()?;
                if let Some(k) = self.pp.containers.iter().rposition(|c| c.0 == index) {
                    if let Some((_, state)) = self.pp.containers.get(k).cloned() {
                        self.pp.state = state;
                    }
                    self.pp.containers.truncate(k);
                    let key = SaveKey::Container(index);
                    let order = self.pp.saves.iter().find(|s| s.0 == key).map_or(0, |s| s.1);
                    self.pp.saves.retain(|s| s.1 < order);
                }
            }
        }
        Ok(true)
    }

    fn put_save(&mut self, entry: (SaveKey, u64, State)) {
        match self.pp.saves.iter_mut().find(|s| s.0 == entry.0) {
            Some(slot) => *slot = entry,
            None => self.pp.saves.push(entry),
        }
    }

    fn push_container(&mut self, index: u32, outer: State, base: Matrix) {
        if self.pp.containers.len() >= self.p.limits.max_save_depth {
            self.p.skip("EmfPlusBeginContainer beyond the depth cap");
            return;
        }
        self.pp.order += 1;
        self.pp.containers.push((index, outer.clone()));
        let entry = (SaveKey::Container(index), self.pp.order, outer.clone());
        self.put_save(entry);
        self.pp.state = State {
            base,
            clip: outer.clip.clone(),
            base_clip: outer.clip,
            interpolation: outer.interpolation,
            compositing: outer.compositing,
            origin: outer.origin,
            ..State::default()
        };
    }

    fn transform(&mut self, kind: u16, flags: u16, d: &mut Data<'_>) -> Played {
        let op = match kind {
            0x402A => {
                let [a, b, c, e, f, g] = d.floats_n::<6>()?;
                self.pp.state.world = Matrix::new(a, b, c, e, f, g);
                return Ok(true);
            }
            0x402B => {
                self.pp.state.world = Matrix::IDENTITY;
                return Ok(true);
            }
            0x402C => {
                let [a, b, c, e, f, g] = d.floats_n::<6>()?;
                Matrix::new(a, b, c, e, f, g)
            }
            0x402D => {
                let [x, y] = d.floats_n::<2>()?;
                Matrix::translate(x, y)
            }
            0x402E => {
                let [x, y] = d.floats_n::<2>()?;
                Matrix::scale(x, y)
            }
            0x402F => {
                let angle = radians(d.f32()?);
                let (c, s) = (angle.cos(), angle.sin());
                Matrix::new(c, s, -s, c, 0.0, 0.0)
            }
            _ => {
                let scale = d.f32()?;
                if scale <= 0.0 {
                    return Ok(false);
                }
                self.pp.state.page_scale = scale;
                self.pp.state.page_unit = u32::from(flags >> 8);
                return Ok(true);
            }
        };
        // Prepend (the operation first, in world space) or, with the `A` flag, append.
        let world = self.pp.state.world;
        self.pp.state.world = if flags & 0x2000 != 0 {
            world.then(&op)
        } else {
            op.then(&world)
        };
        Ok(true)
    }

    fn clip_record(&mut self, kind: u16, flags: u16, d: &mut Data<'_>) -> Played {
        let mode = u32::from((flags >> 8) & 0x0F);
        match kind {
            0x4031 => {
                self.pp.state.clip = self.pp.state.base_clip.clone();
                Ok(true)
            }
            0x4032 => {
                let r = d.floats_n::<4>()?;
                let fig = rect_figure(r).mapped(&self.matrix());
                Ok(self.combine(vec![fig], mode, false)?)
            }
            0x4033 => {
                let Some(figures) = self.path_object(flags) else {
                    return Ok(false);
                };
                let m = self.matrix();
                let figures = figures
                    .iter()
                    .map(|f| Figure {
                        closed: true,
                        ..f.mapped(&m)
                    })
                    .collect();
                Ok(self.combine(figures, mode, true)?)
            }
            _ => self.offset_clip(d),
        }
    }
}

/// An arc (or pie) of the ellipse in `rect` from `start` by `sweep` degrees.
fn pie_figure(start: f64, sweep: f64, rect: [f64; 4], pie: bool) -> Result<Figure, crate::error::ReadError> {
    let [x, y, w, h] = rect;
    let (first, segments) = arc(x, y, w, h, start, sweep)?;
    let mut fig = Figure {
        start: first,
        segments,
        closed: false,
    };
    if pie {
        fig.segments.push(Segment::Line(x + w / 2.0, y + h / 2.0));
        fig.closed = true;
    }
    Ok(fig)
}

/// The parametric angle of the ellipse for a geometric one (GDI+ measures arcs as the angle
/// of the ray from the centre).
fn param(angle: f64, rx: f64, ry: f64) -> f64 {
    if rx == 0.0 || ry == 0.0 {
        return angle;
    }
    (angle.sin() * rx).atan2(angle.cos() * ry)
}

fn arc(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    start_deg: f64,
    sweep_deg: f64,
) -> Result<((f64, f64), Vec<Segment>), crate::error::ReadError> {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let sweep_deg = sweep_deg.clamp(-360.0, 360.0);
    let a0 = radians(start_deg);
    let a1 = radians(start_deg + sweep_deg);
    let t0 = param(a0, rx.abs(), ry.abs());
    let t1 = param(a1, rx.abs(), ry.abs());
    let mut sweep = t1 - t0;
    let full = radians(sweep_deg);
    // Keep the parametric sweep on the side and the number of turns the geometric one has.
    while full > 0.0 && sweep < 0.0 {
        sweep += 2.0 * std::f64::consts::PI;
    }
    while full < 0.0 && sweep > 0.0 {
        sweep -= 2.0 * std::f64::consts::PI;
    }
    if sweep_deg.abs() >= 360.0 {
        sweep = (2.0 * std::f64::consts::PI).copysign(sweep_deg);
    }
    let start = (cx + rx * t0.cos(), cy + ry * t0.sin());
    Ok((start, geom::arc_segments(cx, cy, rx, ry, t0, sweep)?))
}

/// Béziers through `points` (a cardinal spline): each control point a third of the tension
/// times the chord of its neighbours from its point.
fn cardinal(points: &[(f64, f64)], tension: f64, closed: bool) -> Vec<Segment> {
    let n = points.len();
    let k = tension / 3.0;
    let at = |i: usize| points.get(i).copied().unwrap_or_default();
    let last = if closed { n } else { n.saturating_sub(1) };
    (0..last)
        .map(|i| {
            let (p0, p1) = (at(i), at((i + 1) % n));
            let (pm, p2) = if closed {
                (at((i + n - 1) % n), at((i + 2) % n))
            } else {
                (at(i.saturating_sub(1)), at((i + 2).min(n - 1)))
            };
            let c1 = (p0.0 + k * (p1.0 - pm.0), p0.1 + k * (p1.1 - pm.1));
            let c2 = (p1.0 - k * (p2.0 - p0.0), p1.1 - k * (p2.1 - p0.1));
            Segment::Cubic(c1.0, c1.1, c2.0, c2.1, p1.0, p1.1)
        })
        .collect()
}
