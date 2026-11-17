//! Owns: a 3-D chart's scene as drawn -- bar prisms painted back to front with their faces
//! shaded, line and area ribbons as slabs, a surface's faces cut into value bands and lit,
//! the band legend's series, and the scene's gridlines and floor.
//!
//! Must not: lay out the scene (`layout::three_d`).

use super::{at, blanks_as, Builder, Series};
use crate::layout::consts::{
    face_shade, surface_light, BAND_COLOR_CYCLE_SLACK, DEFAULT_AXIS_LINE_EMU, DEFAULT_GAP_DEPTH,
    SURFACE_SEAM_WIDTH_EMU, SURFACE_WIREFRAME_WIDTH_EMU, VIEW_3D_FLOOR_COLOR, VIEW_3D_RIBBON_THICKNESS,
};
use crate::layout::paths::{split_runs, Point};
use crate::layout::scale::{nice_axis_scale, py_max, NiceOptions, Scale};
use crate::layout::three_d::{clip_to_band, lambert, lambert_default, surface_normal, Point3, Rect};
use crate::model::Axis;
use crate::pymath;
use ooxml_drawingml::drawable::{Colour, Fill, Outline};

/// A face of a slab: its sort key, polygon and normal.
type SlabFace = (f64, Vec<Point>, Point3);

/// A surface face: its polygon, band, shade and depth key.
type SurfaceFace = (Vec<Point>, usize, f64, f64);

/// `copysign(1.0, value)`.
fn sign_of(value: f64) -> f64 {
    1.0_f64.copysign(value)
}

/// A value's sort order.
fn by_key(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

impl<'a> Builder<'a> {
    /// Whether this plot is drawn as a 3-D scene.
    pub fn draws_a_scene(&self) -> bool {
        if self.group.scene.is_none() {
            return false;
        }
        if matches!(self.plot.kind.as_str(), "line3DChart" | "area3DChart") || self.is_surface() {
            return true;
        }
        if self.plot.kind != "bar3DChart" || self.grouping_or("clustered") == "standard" {
            return false;
        }
        self.plot.shape.as_deref().filter(|s| !s.is_empty()).unwrap_or("box") == "box"
    }

    fn gap_depth(&self) -> f64 {
        py_max(self.plot.gap_depth.unwrap_or(DEFAULT_GAP_DEPTH) / 100.0, 0.0)
    }

    fn scene_depth_span(&self) -> (f64, f64) {
        let gap = self.gap_depth();
        let near = gap / 2.0 / (1.0 + gap);
        (near, near + 1.0 / (1.0 + gap))
    }

    fn scene_rows(&self, count: usize) -> Vec<(f64, f64)> {
        let gap = self.gap_depth();
        let rows = count.max(1);
        #[allow(clippy::cast_precision_loss)]
        let span = 1.0 / rows as f64;
        let near = span * gap / 2.0 / (1.0 + gap);
        let deep = span / (1.0 + gap);
        #[allow(clippy::cast_precision_loss)]
        (0..rows)
            .map(|i| (i as f64 * span + near, i as f64 * span + near + deep))
            .collect()
    }

    /// Line and area series as slabs, back row first.
    pub fn draw_scene_ribbons(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        let Some(scene) = self.group.scene else { return };
        if series.is_empty() || categories.is_empty() {
            return;
        }
        let xs = self.category_positions(rect, categories.len());
        let zero = self.value_to_y(rect, 0.0, scale);
        let rows = self.scene_rows(series.len());
        let thickness = scene.face.width() * VIEW_3D_RIBBON_THICKNESS;
        let ribbon = self.is_line();
        let blanks = blanks_as(self.chart.display_blanks_as.as_deref(), !ribbon);
        for order in (0..series.len()).rev() {
            let Some(item) = series.get(order) else { continue };
            let mut run: Vec<Option<(f64, f64, f64)>> = Vec::new();
            for index in 0..categories.len() {
                let value = match at(&item.values, index) {
                    Some(v) => v,
                    None if blanks == "zero" => 0.0,
                    None if blanks == "span" => continue,
                    None => {
                        run.push(None);
                        continue;
                    }
                };
                let y = self.value_to_y(rect, value, scale);
                run.push(Some((
                    xs.get(index).copied().unwrap_or(0.0),
                    y,
                    if ribbon { y + thickness } else { zero },
                )));
            }
            for stretch in split_runs(&run) {
                if stretch.len() < 2 {
                    continue;
                }
                let top: Vec<Point> = stretch.iter().map(|&(x, t, _)| (x, t)).collect();
                let bottom: Vec<Point> = stretch.iter().map(|&(x, _, b)| (x, b)).collect();
                let span = rows.get(order).copied().unwrap_or((0.0, 1.0));
                self.paint_slab(&top, &bottom, span, item.fill.clone(), &item.colour);
            }
        }
    }

    /// The value bands between ticks, with their legend names (`low-high`).
    fn value_bands(&self, scale: Scale, axis: Option<&Axis>) -> Vec<(f64, f64, String)> {
        let texts = self.tick_texts(scale, axis);
        texts
            .windows(2)
            .filter_map(|w| match w {
                [(first, first_text), (second, second_text)] => {
                    Some((*first, *second, format!("{first_text}-{second_text}")))
                }
                _ => None,
            })
            .collect()
    }

    fn band_fill(&self, index: usize, count: usize) -> Option<Fill> {
        let key = i64::try_from(index).unwrap_or(i64::MAX);
        if let Some(stated) = self.plot.band_fills.iter().find(|(k, _)| *k == key).map(|(_, f)| f) {
            if let Some(fill) = self.resolve.fill(Some(stated)) {
                return Some(fill);
            }
        }
        if self.style.accents.is_empty() {
            return None;
        }
        Some(Fill::Solid(self.cycle_accent(index, count + BAND_COLOR_CYCLE_SLACK)))
    }

    fn band_colour(&self, index: usize, count: usize) -> Colour {
        match self.band_fill(index, count) {
            Some(Fill::Solid(c)) => c,
            _ => Colour::opaque("#4472C4"),
        }
    }

    fn band_outline(&self, index: usize, count: usize) -> Option<Outline> {
        let key = i64::try_from(index).unwrap_or(i64::MAX);
        if let Some(stated) = self.plot.band_outlines.iter().find(|(k, _)| *k == key).map(|(_, o)| o) {
            if let Some(outline) = self.resolve.outline(Some(stated)) {
                return Some(outline);
            }
        }
        Some(Outline::solid(
            SURFACE_WIREFRAME_WIDTH_EMU,
            self.band_colour(index, count),
        ))
    }

    /// A surface's bands as legend series.
    pub fn band_series(&self, scale: Scale, axis: Option<&Axis>) -> Vec<Series> {
        let bands = self.value_bands(scale, axis);
        let wire = self.plot.wireframe == Some(true);
        let count = bands.len();
        bands
            .into_iter()
            .enumerate()
            .map(|(index, (_, _, name))| {
                let fill = if wire { None } else { self.band_fill(index, count) };
                let outline = if wire { self.band_outline(index, count) } else { None };
                Series::new(Some(name), self.band_colour(index, count), fill, outline)
            })
            .collect()
    }

    /// The scale a surface's legend bands are cut on.
    pub fn band_scale(&self) -> Scale {
        if let Some(scale) = self.group.bands_scale {
            return scale;
        }
        let mut numbers = self.axis_reach(&self.series());
        if numbers.is_empty() {
            numbers = vec![0.0];
        }
        let (low, high) = super::series::min_max(&numbers);
        nice_axis_scale(
            low,
            high,
            NiceOptions {
                strict: false,
                ..NiceOptions::default()
            },
        )
    }

    fn surface_view(&self) -> Point3 {
        let Some(scene) = self.group.scene else {
            return (0.0, 0.0, 1.0);
        };
        let depth = scene.span * scene.face.width();
        if depth <= 0.0 {
            return (0.0, 0.0, 1.0);
        }
        (scene.depth.0 / depth, -scene.depth.1 / depth, 1.0)
    }

    /// A surface: its grid's triangles cut into bands, lit, painted back to front.
    pub fn draw_scene_surface(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        if self.group.scene.is_none() || series.len() < 2 || categories.len() < 2 {
            return;
        }
        let bands = self.value_bands(scale, None);
        let mut faces = self.surface_faces(rect, series, categories.len(), scale, &bands);
        faces.sort_by(|a, b| by_key(a.3, b.3));
        let count = bands.len();
        let wire = self.plot.wireframe == Some(true);
        for (polygon, index, shade, _) in faces {
            if wire {
                let outline = self.band_outline(index, count);
                self.polygon(&polygon, None, outline);
                continue;
            }
            let fill = self.shade_factor(self.band_fill(index, count), shade, &self.band_colour(index, count));
            let outline = match &fill {
                Some(Fill::Solid(c)) => Some(Outline::solid(SURFACE_SEAM_WIDTH_EMU, c.clone())),
                _ => None,
            };
            self.polygon(&polygon, fill, outline);
        }
    }

    /// A surface grid's triangles cut into the value bands: each piece's polygon, band, shade
    /// and depth key.
    fn surface_faces(
        &self,
        rect: Rect,
        series: &[Series],
        count: usize,
        scale: Scale,
        bands: &[(f64, f64, String)],
    ) -> Vec<SurfaceFace> {
        let Some(scene) = self.group.scene else {
            return Vec::new();
        };
        let xs = self.category_positions(rect, count);
        let blanks = self
            .chart
            .display_blanks_as
            .as_deref()
            .filter(|b| !b.is_empty())
            .unwrap_or("gap");
        let rows = series.len();
        let depth = scene.span * scene.face.width();
        let view = self.surface_view();
        let value_at = |row: usize, col: usize| {
            let value = series.get(row).and_then(|s| at(&s.values, col));
            if value.is_none() && blanks == "zero" {
                Some(0.0)
            } else {
                value
            }
        };
        let mut faces: Vec<SurfaceFace> = Vec::new();
        for row in 0..rows - 1 {
            for col in 0..count - 1 {
                let corners = [(row, col), (row, col + 1), (row + 1, col + 1), (row + 1, col)];
                let values: Vec<f64> = corners.iter().filter_map(|&(r, c)| value_at(r, c)).collect();
                if values.len() < corners.len() {
                    continue;
                }
                let zs = [row, row, row + 1, row + 1];
                #[allow(clippy::cast_precision_loss)]
                let points: Vec<Point3> = corners
                    .iter()
                    .zip(values.iter())
                    .zip(zs.iter())
                    .map(|((&(_, c), &v), &z)| {
                        (
                            xs.get(c).copied().unwrap_or(0.0),
                            self.value_to_y(rect, v, scale),
                            z as f64 / (rows - 1) as f64,
                        )
                    })
                    .collect();
                for triangle in [[0, 1, 2], [0, 2, 3]] {
                    let corner_points: Vec<Point3> = triangle.iter().filter_map(|&i| points.get(i).copied()).collect();
                    let corner_values: Vec<f64> = triangle.iter().filter_map(|&i| values.get(i).copied()).collect();
                    let Some(normal) = surface_normal(&corner_points, depth, view) else {
                        continue;
                    };
                    let shade = lambert(normal, surface_light());
                    let key = pymath::sum(
                        corner_points
                            .iter()
                            .map(|p| p.0 * view.0 - p.1 * view.1 - p.2 * depth * view.2),
                    ) / 3.0;
                    for (index, (low, high, _)) in bands.iter().enumerate() {
                        let piece = clip_to_band(&corner_points, &corner_values, *low, *high);
                        if piece.len() >= 3 {
                            faces.push((
                                piece.iter().map(|p| scene.at(p.0, p.1, p.2)).collect(),
                                index,
                                shade,
                                key,
                            ));
                        }
                    }
                }
            }
        }
        faces
    }

    fn paint_slab(&mut self, top: &[Point], bottom: &[Point], span: (f64, f64), fill: Option<Fill>, base: &Colour) {
        let Some(scene) = self.group.scene else { return };
        if top.len() < 2 {
            return;
        }
        let (near, far) = span;
        let mut faces: Vec<SlabFace> = Vec::new();
        let mut quad = |a: Point, b: Point, normal: Point3| {
            let facing = -normal.0 * scene.depth.0 + normal.1 * scene.depth.1;
            if facing.partial_cmp(&0.0) != Some(std::cmp::Ordering::Less) {
                return;
            }
            faces.push((
                (a.0 + b.0) / 2.0 * sign_of(scene.depth.0),
                vec![
                    scene.at(a.0, a.1, near),
                    scene.at(b.0, b.1, near),
                    scene.at(b.0, b.1, far),
                    scene.at(a.0, a.1, far),
                ],
                normal,
            ));
        };
        let (Some(&t0), Some(&b0), Some(&t1), Some(&b1)) = (top.first(), bottom.first(), top.last(), bottom.last())
        else {
            return;
        };
        quad(t0, b0, (-1.0, 0.0, 0.0));
        quad(t1, b1, (1.0, 0.0, 0.0));
        for index in 0..top.len() - 1 {
            for (edge, upper) in [(top, true), (bottom, false)] {
                let (Some(&first), Some(&second)) = (edge.get(index), edge.get(index + 1)) else {
                    continue;
                };
                let (run, rise) = (second.0 - first.0, second.1 - first.1);
                let length = pymath::hypot(&[run, rise]);
                if length <= 0.0 {
                    continue;
                }
                let normal = (rise / length, run / length, 0.0);
                quad(
                    first,
                    second,
                    if upper {
                        normal
                    } else {
                        (-normal.0, -normal.1, -normal.2)
                    },
                );
            }
        }
        faces.sort_by(|a, b| by_key(a.0, b.0));
        for (_, points, normal) in faces {
            let shaded = self.shade_factor(fill.clone(), lambert_default(normal), base);
            self.polygon(&points, shaded, None);
        }
        let mut front: Vec<Point> = top.iter().map(|p| scene.at(p.0, p.1, near)).collect();
        front.extend(bottom.iter().rev().map(|p| scene.at(p.0, p.1, near)));
        self.polygon(&front, fill, None);
    }

    /// The value gridlines across the scene's back and side walls.
    pub fn draw_scene_gridlines(&mut self, rect: Rect, scale: Scale, axis: Option<&Axis>) {
        let Some(scene) = self.group.scene else { return };
        let Some(axis) = axis.filter(|a| a.major_gridlines) else {
            return;
        };
        let Some(outline) = self.axis_outline(axis.major_gridline_outline.as_ref()) else {
            return;
        };
        let near_x = if scene.depth.0 >= 0.0 { rect.left } else { rect.right };
        let far_x = if scene.depth.0 >= 0.0 { rect.right } else { rect.left };
        let near_y = if scene.depth.1 <= 0.0 { rect.bottom } else { rect.top };
        let far_y = if scene.depth.1 <= 0.0 { rect.top } else { rect.bottom };
        let horizontal = self.horizontal();
        for value in super::series::tick_values(scale) {
            let points = if horizontal {
                let x = self.value_to_x(rect, value, scale);
                vec![(x, near_y), scene.at(x, near_y, 1.0), scene.at(x, far_y, 1.0)]
            } else {
                let y = self.value_to_y(rect, value, scale);
                vec![(near_x, y), scene.at(near_x, y, 1.0), scene.at(far_x, y, 1.0)]
            };
            self.polyline(&points, &outline, false);
        }
    }

    /// The scene's floor along the category axis.
    pub fn draw_scene_floor(&mut self, rect: Rect, scale: Scale, axis: Option<&Axis>) {
        let Some(scene) = self.group.scene else { return };
        let Some(axis) = axis.filter(|a| !a.delete) else { return };
        let crossing = self.category_axis_position(rect, scale, Some(axis), true);
        let (near, far) = if self.horizontal() {
            ((crossing, rect.top), (crossing, rect.bottom))
        } else {
            ((rect.left, crossing), (rect.right, crossing))
        };
        let points = [near, scene.at(near.0, near.1, 1.0), scene.at(far.0, far.1, 1.0), far];
        let outline = Outline::solid(DEFAULT_AXIS_LINE_EMU, Colour::opaque(VIEW_3D_FLOOR_COLOR));
        self.polygon(&points, Some(Fill::None), Some(outline));
    }

    fn shade(&self, fill: Option<Fill>, face: &str, base: &Colour) -> Option<Fill> {
        self.shade_factor(fill, face_shade(face).unwrap_or(1.0), base)
    }

    /// A fill darkened by `factor` (the solid colour's, else `base`'s).
    pub fn shade_factor(&self, fill: Option<Fill>, factor: f64, base: &Colour) -> Option<Fill> {
        if factor >= 1.0 {
            return fill;
        }
        let colour = match &fill {
            Some(Fill::Solid(c)) => c.clone(),
            _ => base.clone(),
        };
        let level = |index: usize| {
            let channel = colour
                .hex
                .get(index..index + 2)
                .and_then(|c| u8::from_str_radix(c, 16).ok())
                .unwrap_or(0);
            let shaded = crate::pyfmt::round_half_even(f64::from(channel) * factor);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let level = shaded.clamp(0.0, 255.0) as u8;
            level
        };
        Some(Fill::Solid(Colour {
            hex: format!("#{:02X}{:02X}{:02X}", level(1), level(3), level(5)),
            alpha: colour.alpha,
        }))
    }

    fn paint_prism(&mut self, prism: &super::Prism, span: (f64, f64)) {
        let Some(scene) = self.group.scene else { return };
        let (near, far) = span;
        let r = prism.rect;
        let (cap_y, cap_face) = if scene.depth.1 < 0.0 {
            (r.top, "top")
        } else {
            (r.bottom, "bottom")
        };
        let (side_x, side_face) = if scene.depth.0 > 0.0 {
            (r.right, "right")
        } else {
            (r.left, "left")
        };
        let faces = [
            (
                side_face,
                [
                    scene.at(side_x, r.top, near),
                    scene.at(side_x, r.top, far),
                    scene.at(side_x, r.bottom, far),
                    scene.at(side_x, r.bottom, near),
                ],
            ),
            (
                cap_face,
                [
                    scene.at(r.left, cap_y, near),
                    scene.at(r.right, cap_y, near),
                    scene.at(r.right, cap_y, far),
                    scene.at(r.left, cap_y, far),
                ],
            ),
            (
                "front",
                [
                    scene.at(r.left, r.top, near),
                    scene.at(r.right, r.top, near),
                    scene.at(r.right, r.bottom, near),
                    scene.at(r.left, r.bottom, near),
                ],
            ),
        ];
        for (face, points) in faces {
            let [p0, _, p2, _] = points;
            if (p0.0 - p2.0).abs() < 1e-9 || (p0.1 - p2.1).abs() < 1e-9 {
                continue;
            }
            let fill = self.shade(prism.fill.clone(), face, &prism.base);
            self.polygon(&points, fill, prism.outline.clone());
        }
    }

    /// The queued prisms, painted in depth order.
    pub fn paint_prisms(&mut self) {
        let Some(mut prisms) = self.group.prisms.take() else {
            return;
        };
        if prisms.is_empty() {
            return;
        }
        let span = self.scene_depth_span();
        prisms.sort_by(|a, b| {
            by_key(a.order.0, b.order.0)
                .then(by_key(a.order.1, b.order.1))
                .then(by_key(a.order.2, b.order.2))
        });
        for prism in &prisms {
            self.paint_prism(prism, span);
        }
    }

    /// A prism's place in the painting order.
    pub fn prism_order(&self, rect: Rect) -> (f64, f64, f64) {
        let Some(scene) = self.group.scene else {
            return (0.0, 0.0, 0.0);
        };
        (
            0.0,
            (rect.left + rect.right) / 2.0 * sign_of(scene.depth.0),
            (rect.top + rect.bottom) / 2.0 * sign_of(scene.depth.1),
        )
    }
}
