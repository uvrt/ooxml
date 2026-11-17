//! Owns: a 3-D chart's scene as `chart/layout.py` models it -- the camera that applies
//! ([`camera`]), the scene's shape and depth, its front face inside the plot region
//! with the depth as a fixed offset ([`scene`]), Lambert shading of a face, and the
//! geometry a surface's faces are cut and lit with.
//!
//! Must not: draw (the builder does), or model a perspective scene (it is not measured).

use super::consts::{
    DEFAULT_GAP_DEPTH, DEFAULT_GAP_WIDTH, MIN_BAR_SLOTS, VIEW_3D_AMBIENT, VIEW_3D_DIFFUSE, VIEW_3D_LIGHT,
    VIEW_3D_SCENE_MARGIN,
};
use super::scale::{py_max, py_min};
use crate::model::View3D;
use crate::pymath;

/// A rectangle by its edges, points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left.
    pub left: f64,
    /// Top.
    pub top: f64,
    /// Right.
    pub right: f64,
    /// Bottom.
    pub bottom: f64,
}

impl Rect {
    /// A rectangle.
    pub fn new(left: f64, top: f64, right: f64, bottom: f64) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Its width.
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    /// Its height.
    pub fn height(&self) -> f64 {
        self.bottom - self.top
    }
}

/// A point in 3-D.
pub type Point3 = (f64, f64, f64);

/// How lit a face with `normal` is.
pub fn lambert(normal: Point3, light: [f64; 3]) -> f64 {
    let [a, b, c] = light;
    let lit = pymath::sum([normal.0 * a, normal.1 * b, normal.2 * c]);
    py_min(1.0, VIEW_3D_AMBIENT + VIEW_3D_DIFFUSE * py_max(0.0, lit))
}

/// Lambert under the default light.
pub fn lambert_default(normal: Point3) -> f64 {
    lambert(normal, VIEW_3D_LIGHT)
}

/// A scene's aspect factor and row count, or `None` for a kind with no scene.
pub fn scene_shape(
    kind: &str,
    series: usize,
    grouping: Option<&str>,
    categories: usize,
    across: Option<usize>,
) -> Option<(f64, Option<f64>)> {
    if kind == "bar3DChart" {
        return Some((1.0, None));
    }
    if !matches!(kind, "line3DChart" | "area3DChart" | "surfaceChart" | "surface3DChart") {
        return None;
    }
    if matches!(grouping.unwrap_or(""), "stacked" | "percentStacked") {
        return None;
    }
    let count = categories.max(1);
    let span = across.map_or(count, |a| a.max(1));
    let rows = series.max(1);
    #[allow(clippy::cast_precision_loss)]
    let shape = ((span + rows) / 2) as f64 / count as f64;
    #[allow(clippy::cast_precision_loss)]
    Some((shape, Some(rows as f64)))
}

/// The camera a 3-D chart is drawn through, where one applies.
pub fn camera<'a>(view: Option<&'a View3D>, kind: &str, series: usize, grouping: Option<&str>) -> Option<&'a View3D> {
    let view = view?;
    if view.right_angle_axes == Some(false) {
        return None;
    }
    scene_shape(kind, series, grouping, 5, None)?;
    Some(view)
}

/// How many bar slots a 3-D bar chart has.
fn bar_slots(series: usize, grouping: Option<&str>) -> usize {
    if matches!(grouping.unwrap_or(""), "stacked" | "percentStacked") {
        1
    } else {
        series.max(1)
    }
}

/// What a scene is laid out with.
#[derive(Debug, Clone, Copy)]
pub struct SceneSpec<'a> {
    /// The plot's kind.
    pub kind: &'a str,
    /// Its series' count.
    pub series: usize,
    /// `c:gapDepth`.
    pub gap_depth: Option<f64>,
    /// `c:grouping`.
    pub grouping: Option<&'a str>,
    /// Its category count.
    pub categories: usize,
    /// `c:gapWidth`.
    pub gap_width: Option<f64>,
    /// `c:barDir`.
    pub bar_direction: Option<&'a str>,
    /// The categories across the face.
    pub across: Option<usize>,
}

/// The scene's depth as a fraction of the face's width.
fn scene_depth(spec: &SceneSpec<'_>, view: &View3D, aspect: f64) -> f64 {
    let depth = view.depth_percent.unwrap_or(100.0) / 100.0;
    let gap = py_max(spec.gap_depth.unwrap_or(DEFAULT_GAP_DEPTH) / 100.0, 0.0);
    #[allow(clippy::cast_precision_loss)]
    let categories = spec.categories.max(1) as f64;
    let rows = scene_shape(spec.kind, spec.series, spec.grouping, spec.categories, None).and_then(|s| s.1);
    if let Some(rows) = rows {
        return depth * rows / categories;
    }
    let width = py_max(spec.gap_width.unwrap_or(DEFAULT_GAP_WIDTH), 0.0) / 100.0;
    #[allow(clippy::cast_precision_loss)]
    let slots = bar_slots(spec.series, spec.grouping) as f64;
    let divisor = categories * py_max(slots + width, MIN_BAR_SLOTS);
    let span = if spec.bar_direction.unwrap_or("col") == "bar" {
        aspect
    } else {
        1.0
    };
    depth * span * (1.0 + gap) / divisor
}

/// A scene: its front face, the offset one unit of depth moves a point, and its depth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene {
    /// The front face.
    pub face: Rect,
    /// The offset of a full unit of depth.
    pub depth: (f64, f64),
    /// The depth as a fraction of the face's width.
    pub span: f64,
}

impl Scene {
    /// Where `(x, y)` at depth `z` (0 front, 1 back) is drawn.
    pub fn at(&self, x: f64, y: f64, z: f64) -> (f64, f64) {
        (x + self.depth.0 * z, y + self.depth.1 * z)
    }
}

/// The scene a 3-D chart's plot region holds.
pub fn scene(region: Rect, view: &View3D, spec: &SceneSpec<'_>) -> Option<Scene> {
    let rot_x = view.rot_x.unwrap_or(0.0).to_radians();
    let rot_y = view.rot_y.unwrap_or(0.0).to_radians();
    let scale = scene_shape(spec.kind, spec.series, spec.grouping, spec.categories, spec.across).map_or(1.0, |s| s.0);
    if region.width() <= 0.0 || region.height() <= 0.0 {
        return None;
    }
    let aspect = scale * view.h_percent.map_or(region.height() / region.width(), |h| h / 100.0);
    let depth = scene_depth(spec, view, aspect);
    if !(aspect.is_finite() && aspect > 0.0 && depth.is_finite() && depth >= 0.0) {
        return None;
    }
    let run = depth * rot_y.sin().abs();
    let rise = depth * rot_x.sin().abs();
    let width = py_min(
        region.width() / (1.0 + VIEW_3D_SCENE_MARGIN + run),
        region.height() / (aspect + VIEW_3D_SCENE_MARGIN + rise),
    );
    if !width.is_finite() || width <= 0.0 {
        return None;
    }
    let height = width * aspect;
    let margin = width * VIEW_3D_SCENE_MARGIN / 2.0;
    let slack_x = (region.width() - width * (1.0 + VIEW_3D_SCENE_MARGIN + run)) / 2.0;
    let slack_y = (region.height() - height - width * (VIEW_3D_SCENE_MARGIN + rise)) / 2.0;
    let reversed = view.rot_y.unwrap_or(0.0).rem_euclid(360.0) >= 180.0;
    let leads_left = rot_y.sin() < 0.0;
    let leads_up = (rot_x.sin() > 0.0) != reversed;
    let left = region.left + slack_x + margin + if leads_left { width * run } else { 0.0 };
    let top = region.top + slack_y + margin + if leads_up { width * rise } else { 0.0 };
    Some(Scene {
        face: Rect::new(left, top, left + width, top + height),
        depth: (
            if leads_left { -width * run } else { width * run },
            if leads_up { -width * rise } else { width * rise },
        ),
        span: depth,
    })
}

/// A triangle's unit normal facing the viewer, or `None` for a degenerate one.
pub fn surface_normal(points: &[Point3], depth: f64, view: Point3) -> Option<Point3> {
    if depth <= 0.0 || points.len() < 3 {
        return None;
    }
    let lift = |p: &Point3| (p.0, -p.1, -p.2 * depth);
    let (a, b, c) = (lift(points.first()?), lift(points.get(1)?), lift(points.get(2)?));
    let u = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
    let v = (c.0 - a.0, c.1 - a.1, c.2 - a.2);
    let mut normal = (u.1 * v.2 - u.2 * v.1, u.2 * v.0 - u.0 * v.2, u.0 * v.1 - u.1 * v.0);
    let length = pymath::hypot(&[normal.0, normal.1, normal.2]);
    if length <= 0.0 {
        return None;
    }
    if pymath::sum([normal.0 * view.0, normal.1 * view.1, normal.2 * view.2]) < 0.0 {
        normal = (-normal.0, -normal.1, -normal.2);
    }
    Some((normal.0 / length, normal.1 / length, normal.2 / length))
}

/// A polygon with values at its corners clipped to the band `low..high` of those values.
pub fn clip_to_band(points: &[Point3], values: &[f64], low: f64, high: f64) -> Vec<Point3> {
    let mut polygon: Vec<(Point3, f64)> = points.iter().copied().zip(values.iter().copied()).collect();
    for (keep_above, limit) in [(true, low), (false, high)] {
        let inside = |value: f64| if keep_above { value >= limit } else { value <= limit };
        let mut clipped = Vec::new();
        let count = polygon.len();
        for index in 0..count {
            let Some(&(point, value)) = polygon.get(index) else {
                continue;
            };
            let Some(&(previous_point, previous_value)) = polygon.get((index + count - 1) % count) else {
                continue;
            };
            if inside(value) {
                if !inside(previous_value) {
                    clipped.push(lerp(previous_point, previous_value, point, value, limit));
                }
                clipped.push((point, value));
            } else if inside(previous_value) {
                clipped.push(lerp(previous_point, previous_value, point, value, limit));
            }
        }
        polygon = clipped;
        if polygon.is_empty() {
            return Vec::new();
        }
    }
    polygon.into_iter().map(|(p, _)| p).collect()
}

fn lerp(first: Point3, first_value: f64, second: Point3, second_value: f64, limit: f64) -> (Point3, f64) {
    let span = second_value - first_value;
    let t = if span == 0.0 { 0.0 } else { (limit - first_value) / span };
    let t = py_min(py_max(t, 0.0), 1.0);
    (
        (
            first.0 + (second.0 - first.0) * t,
            first.1 + (second.1 - first.1) * t,
            first.2 + (second.2 - first.2) * t,
        ),
        limit,
    )
}
