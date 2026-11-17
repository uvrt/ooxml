//! Owns: path gradients -- shaded triangle by triangle (the centre and one boundary edge,
//! barycentric between their colours, the blend or preset applied from the boundary to the
//! centre) into a bitmap in device pixels, clipped to the shape, as Windows' PDF export
//! draws them; past the shading cap, the centre colour.
//!
//! Must not: fill anything but a path gradient.

use super::objects::{flatten, lerp_argb, PathGradient};
use super::paint::colour;
use super::Plus;
use crate::error::{ReadError, RecordError};
use crate::player::transform_of;
use crate::png;
use crate::pymath;
use ooxml_scene::{Figure, Group, Image, ImageData, Matrix, Node, Path, PathData};

/// The box shaded: device pixels `x0..x1`, `y0..y1`.
struct Area {
    x0: f64,
    y0: f64,
    pw: usize,
    ph: usize,
    step_x: f64,
    step_y: f64,
}

impl Plus<'_, '_> {
    /// Fills `figures` with a path gradient; a box or span beyond finite numbers makes the
    /// record unreadable, as it does the reference's.
    pub fn path_gradient(
        &mut self,
        brush: &PathGradient,
        figures: Vec<Figure>,
        m: &Matrix,
        even_odd: bool,
    ) -> Result<(), RecordError> {
        let bm = brush.matrix.then(m);
        let polygons: Vec<Vec<(f64, f64)>> = brush
            .boundary
            .iter()
            .filter(|p| p.len() >= 3)
            .map(|p| p.iter().map(|&(x, y)| bm.apply(x, y)).collect())
            .collect();
        if polygons.is_empty() {
            return Ok(());
        }
        let centre = bm.apply(brush.center.0, brush.center.1);
        let shape: Vec<(f64, f64)> = figures.iter().flat_map(|f| flatten(&f.mapped(m), 4)).collect();
        let bounds: Vec<(f64, f64)> = polygons.iter().flatten().copied().collect();
        let (cx, cy, cw, ch) = self.p.canvas;
        let lo = |v: f64, w: f64, c: f64| -> Result<f64, RecordError> {
            Ok(pymath::floor(largest(v, w, c))?.max(-1_000_000.0))
        };
        let hi = |v: f64, w: f64, c: f64| -> Result<f64, RecordError> {
            Ok(pymath::ceil(smallest(v, w, c))?.min(1_000_000.0))
        };
        let x0 = lo(min(shape.iter().map(|p| p.0)), min(bounds.iter().map(|p| p.0)), cx)?;
        let y0 = lo(min(shape.iter().map(|p| p.1)), min(bounds.iter().map(|p| p.1)), cy)?;
        let x1 = hi(max(shape.iter().map(|p| p.0)), max(bounds.iter().map(|p| p.0)), cx + cw)?;
        let y1 = hi(max(shape.iter().map(|p| p.1)), max(bounds.iter().map(|p| p.1)), cy + ch)?;
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        let area = (x1 - x0) * (y1 - y0);
        let cap = self.p.limits.max_gradient_image as f64;
        // One sample a device pixel, as Windows' own bitmap of it; fewer for a big one.
        let k = if area <= cap { 1.0 } else { (cap / area).sqrt() };
        let (pw, ph) = ((((x1 - x0) * k) as i64).max(1), (((y1 - y0) * k) as i64).max(1));
        let pixels = u64::try_from(pw * ph).unwrap_or(u64::MAX);
        let transform = transform_of(m);
        if self.pp.gradient_pixels.saturating_add(pixels) > self.p.limits.max_gradient_pixels {
            self.p
                .skip("EmfPlus path gradients beyond the shading cap (drawn in their centre colour)");
            self.write(Node::Path(Path {
                data: PathData::new(figures),
                transform,
                fill: Some(colour(brush.center_color).fill(even_odd)),
                ..Path::default()
            }))?;
            return Ok(());
        }
        self.pp.gradient_pixels += pixels;
        let (pw, ph) = (pw as usize, ph as usize);
        let area = Area {
            x0,
            y0,
            pw,
            ph,
            step_x: (x1 - x0) / pw as f64,
            step_y: (y1 - y0) / ph as f64,
        };
        let rgba = shade(brush, &polygons, centre, &area)?;
        let png = png::encode(pw as u32, ph as u32, &rgba, true);
        let id = self.clip_path(PathData::new(figures), transform, even_odd)?;
        self.write(Node::Group(Group {
            clip: Some(id),
            children: vec![Node::Image(Image {
                x: Some(x0),
                y: Some(y0),
                width: x1 - x0,
                height: y1 - y0,
                stretch: true,
                pixelated: false,
                data: ImageData {
                    mime: "image/png".to_owned(),
                    bytes: png,
                },
            })],
            ..Group::default()
        }))?;
        Ok(())
    }
}

fn min(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(f64::INFINITY, |a, b| if b < a { b } else { a })
}

fn max(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(f64::NEG_INFINITY, |a, b| if b > a { b } else { a })
}

/// The largest of three, the first of equals.
fn largest(a: f64, b: f64, c: f64) -> f64 {
    let ab = if b > a { b } else { a };
    if c > ab {
        c
    } else {
        ab
    }
}

/// The smallest of three, the first of equals.
fn smallest(a: f64, b: f64, c: f64) -> f64 {
    let ab = if b < a { b } else { a };
    if c < ab {
        c
    } else {
        ab
    }
}

/// A float truncated toward zero, refused when it is not finite.
fn truncate(v: f64) -> Result<i64, ReadError> {
    if v.is_finite() {
        Ok(v.trunc() as i64)
    } else {
        Err(ReadError::NotFinite)
    }
}

fn channels(v: u32) -> [f64; 4] {
    [(v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF, (v >> 24) & 0xFF].map(f64::from)
}

/// A piecewise-linear value of `(position, value)` stops at `t`.
fn at(stops: &[(f64, f64)], t: f64) -> f64 {
    let Some(&(p0, v0)) = stops.first() else {
        return t;
    };
    if t <= p0 {
        return v0;
    }
    for pair in stops.windows(2) {
        if let [(p0, v0), (p1, v1)] = *pair {
            if t <= p1 {
                let f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
                return v0 + (v1 - v0) * f;
            }
        }
    }
    stops.last().map_or(t, |s| s.1)
}

/// The colour of ARGB stops at `t`.
fn at_colour(stops: &[(f64, u32)], t: f64) -> u32 {
    let Some(&(p0, v0)) = stops.first() else {
        return 0xFF00_0000;
    };
    if t <= p0 {
        return v0;
    }
    for pair in stops.windows(2) {
        if let [(p0, v0), (p1, v1)] = *pair {
            if t <= p1 {
                let f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
                return lerp_argb(v0, v1, f);
            }
        }
    }
    stops.last().map_or(0xFF00_0000, |s| s.1)
}

/// The gradient's pixels: each triangle of the centre and a boundary edge shaded from the
/// centre colour to the edge's colours, transparent outside the boundary; row by row, over
/// the span of each row inside the triangle.
fn shade(
    brush: &PathGradient,
    polygons: &[Vec<(f64, f64)>],
    centre: (f64, f64),
    a: &Area,
) -> Result<Vec<u8>, ReadError> {
    let mut rgba = vec![0u8; a.pw * a.ph * 4];
    let surround: Vec<u32> = if brush.surround.is_empty() {
        vec![brush.center_color]
    } else {
        brush.surround.clone()
    };
    let cc = channels(brush.center_color);
    let plain = brush.blend.is_none() && brush.preset.is_none() && brush.focus <= 0.0;
    for poly in polygons {
        let n = poly.len();
        let colours: Vec<[f64; 4]> = (0..n)
            .map(|i| channels(surround.get(i.min(surround.len() - 1)).copied().unwrap_or(0)))
            .collect();
        for i in 0..n {
            let (Some(&pa), Some(&pb)) = (poly.get(i), poly.get((i + 1) % n)) else {
                continue;
            };
            let (Some(&ca), Some(&cb)) = (colours.get(i), colours.get((i + 1) % n)) else {
                continue;
            };
            let tri = Triangle {
                a: pa,
                b: pb,
                c: centre,
                ca,
                cb,
                cc,
            };
            tri.shade(brush, plain, a, &mut rgba)?;
        }
    }
    Ok(rgba)
}

struct Triangle {
    a: (f64, f64),
    b: (f64, f64),
    c: (f64, f64),
    ca: [f64; 4],
    cb: [f64; 4],
    cc: [f64; 4],
}

impl Triangle {
    fn shade(&self, brush: &PathGradient, plain: bool, area: &Area, rgba: &mut [u8]) -> Result<(), ReadError> {
        let ((ax, ay), (bx, by), (cxp, cyp)) = (self.a, self.b, self.c);
        let denominator = (by - cyp) * (ax - cxp) + (cxp - bx) * (ay - cyp);
        if denominator.abs() < 1e-12 {
            return Ok(());
        }
        // l1 (corner a) and l2 (corner b) are linear in the pixel: l = gx * x + gy * y + k.
        let (g1x, g1y) = ((by - cyp) / denominator, (cxp - bx) / denominator);
        let (g2x, g2y) = ((cyp - ay) / denominator, (ax - cxp) / denominator);
        let k1 = -g1x * cxp - g1y * cyp;
        let k2 = -g2x * cxp - g2y * cyp;
        let lo_y = truncate((ay.min(by).min(cyp) - area.y0) / area.step_y)?;
        let hi_y = truncate((ay.max(by).max(cyp) - area.y0) / area.step_y)?
            .saturating_add(1)
            .min(area.ph as i64);
        for j in lo_y.max(0)..hi_y {
            let py = area.y0 + (j as f64 + 0.5) * area.step_y;
            let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
            for (gx, c) in [
                (g1x, g1y * py + k1),
                (g2x, g2y * py + k2),
                (-g1x - g2x, 1.0 - (g1y + g2y) * py - k1 - k2),
            ] {
                if gx.abs() < 1e-15 {
                    if c < -1e-9 {
                        (lo, hi) = (1.0, 0.0);
                    }
                    continue;
                }
                let bound = -c / gx;
                if gx > 0.0 {
                    lo = if bound > lo { bound } else { lo };
                } else {
                    hi = if bound < hi { bound } else { hi };
                }
            }
            let first = pymath::ceil((lo - area.x0) / area.step_x - 0.5 - 1e-9)?.max(0.0);
            let last = pymath::floor((hi - area.x0) / area.step_x - 0.5 + 1e-9)?.min(area.pw as f64 - 1.0);
            if last < first {
                continue;
            }
            let (first, last) = (first as usize, last as usize);
            let row = j as usize * area.pw;
            let px = area.x0 + (first as f64 + 0.5) * area.step_x;
            let mut l1 = g1x * px + g1y * py + k1;
            let mut l2 = g2x * px + g2y * py + k2;
            let (d1, d2) = (g1x * area.step_x, g2x * area.step_x);
            for i2 in first..=last {
                let colour = self.colour(brush, plain, l1, l2);
                if let Some(slot) = rgba.get_mut((row + i2) * 4..(row + i2) * 4 + 4) {
                    for (s, v) in slot.iter_mut().zip(colour) {
                        *s = ((v + 0.5) as i64).clamp(0, 255) as u8;
                    }
                }
                l1 += d1;
                l2 += d2;
            }
        }
        Ok(())
    }

    fn colour(&self, brush: &PathGradient, plain: bool, l1: f64, l2: f64) -> [f64; 4] {
        let a1 = if l1 > 0.0 { l1 } else { 0.0 };
        let a2 = if l2 > 0.0 { l2 } else { 0.0 };
        let mut l0 = 1.0 - a1 - a2;
        if l0 < 0.0 {
            l0 = 0.0;
        }
        let (ca, cb, cc) = (self.ca, self.cb, self.cc);
        let mut out = [0.0; 4];
        if plain {
            for (((o, c), a), b) in out.iter_mut().zip(cc).zip(ca).zip(cb) {
                *o = c * l0 + a * a1 + b * a2;
            }
            return out;
        }
        let edge = a1 + a2;
        let u = if edge > 1e-12 { a2 / edge } else { 0.0 };
        let mut t = l0;
        if brush.focus > 0.0 {
            t = if t >= 1.0 - brush.focus {
                1.0
            } else {
                t / (1.0 - brush.focus)
            };
        }
        if let Some(preset) = &brush.preset {
            return channels(at_colour(preset, t));
        }
        let f = brush.blend.as_ref().map_or(t, |b| at(b, t));
        for (((o, c), a), b) in out.iter_mut().zip(cc).zip(ca).zip(cb) {
            let bc = a + (b - a) * u;
            *o = bc + (c - bc) * f;
        }
        out
    }
}
