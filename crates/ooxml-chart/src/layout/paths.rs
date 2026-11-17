//! Owns: the path data and small geometry the layout draws with -- polylines and Catmull-Rom
//! smoothing, pie and doughnut slices as cubic arcs, a path moved by an offset, a tangent
//! from a point to a circle, the colour shift a long accent cycle takes, whole percentages
//! that add to 100, and runs of points split at blanks -- each written with the digits
//! `chart/layout.py` writes (`f"{x:.4f}"`).
//!
//! Must not: know about series, axes or scenes.

use super::consts::{MAX_ARC_DEGREES, VARY_COLOR_CYCLE_SHADE, VARY_COLOR_CYCLE_TINT};
use super::scale::{py_max, py_min};
use crate::pyfmt::fixed;
use crate::pymath;

/// A point.
pub type Point = (f64, f64);

/// `f"{x:.4f} {y:.4f}"`.
fn pair(x: f64, y: f64) -> String {
    format!("{} {}", fixed(x, 4), fixed(y, 4))
}

/// A polyline through `points` (at least one), smoothed as Catmull-Rom cubics.
pub fn path_commands(points: &[Point], smooth: bool) -> String {
    let Some(&(x0, y0)) = points.first() else {
        return String::new();
    };
    let mut parts = vec![format!("M {}", pair(x0, y0))];
    if !smooth || points.len() < 3 {
        parts.extend(points.iter().skip(1).map(|&(x, y)| format!("L {}", pair(x, y))));
        return parts.join(" ");
    }
    let reflect = |inner: Point, edge: Point| (2.0 * edge.0 - inner.0, 2.0 * edge.1 - inner.1);
    for (index, window) in points.windows(2).enumerate() {
        let [p1, p2] = [
            window.first().copied().unwrap_or_default(),
            window.get(1).copied().unwrap_or_default(),
        ];
        let p0 = index
            .checked_sub(1)
            .and_then(|i| points.get(i))
            .copied()
            .unwrap_or_else(|| reflect(p2, p1));
        let p3 = points.get(index + 2).copied().unwrap_or_else(|| reflect(p1, p2));
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        parts.push(format!(
            "C {} {} {}",
            pair(c1.0, c1.1),
            pair(c2.0, c2.1),
            pair(p2.0, p2.1)
        ));
    }
    parts.join(" ")
}

/// A circular arc as cubic control points (three a piece), at most 90 degrees a piece;
/// angles clockwise from twelve o'clock.
fn arc_points(cx: f64, cy: f64, radius: f64, start: f64, sweep: f64) -> Vec<Point> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pieces = ((sweep.abs() / MAX_ARC_DEGREES).ceil() as usize).max(1);
    #[allow(clippy::cast_precision_loss)]
    let step = sweep / pieces as f64;
    let mut points = Vec::with_capacity(pieces * 3);
    for piece in 0..pieces {
        #[allow(clippy::cast_precision_loss)]
        let (k0, k1) = (piece as f64, (piece + 1) as f64);
        let a0 = (start + k0 * step).to_radians();
        let a1 = (start + k1 * step).to_radians();
        let alpha = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan();
        let p0 = (cx + radius * a0.sin(), cy - radius * a0.cos());
        let p1 = (cx + radius * a1.sin(), cy - radius * a1.cos());
        let t0 = (radius * a0.cos(), radius * a0.sin());
        let t1 = (radius * a1.cos(), radius * a1.sin());
        points.push((p0.0 + alpha * t0.0, p0.1 + alpha * t0.1));
        points.push((p1.0 - alpha * t1.0, p1.1 - alpha * t1.1));
        points.push(p1);
    }
    points
}

fn cubics(parts: &mut Vec<String>, points: &[Point]) {
    for chunk in points.chunks(3) {
        if let [c1, c2, end] = chunk {
            parts.push(format!(
                "C {} {} {}",
                pair(c1.0, c1.1),
                pair(c2.0, c2.1),
                pair(end.0, end.1)
            ));
        }
    }
}

/// A slice of a ring (a pie's where `inner` is 0) as a closed path.
pub fn ring_path(cx: f64, cy: f64, inner: f64, outer: f64, start: f64, sweep: f64) -> String {
    let sweep = py_max(py_min(sweep, 360.0), -360.0);
    let begin = (
        cx + outer * start.to_radians().sin(),
        cy - outer * start.to_radians().cos(),
    );
    let mut parts = vec![format!("M {}", pair(begin.0, begin.1))];
    cubics(&mut parts, &arc_points(cx, cy, outer, start, sweep));
    if inner <= 0.0 {
        parts.push(format!("L {}", pair(cx, cy)));
    } else {
        let end = (start + sweep).to_radians();
        parts.push(format!("L {}", pair(cx + inner * end.sin(), cy - inner * end.cos())));
        cubics(&mut parts, &arc_points(cx, cy, inner, start + sweep, -sweep));
    }
    parts.push("Z".into());
    parts.join(" ")
}

/// Whether a token is a number as the Python matches it (`-?\d+\.?\d*`).
fn numeric(token: &str) -> bool {
    let body = token.strip_prefix('-').unwrap_or(token);
    let (whole, fraction) = body.split_once('.').map_or((body, ""), |(w, f)| (w, f));
    !whole.is_empty() && whole.chars().all(|c| c.is_ascii_digit()) && fraction.chars().all(|c| c.is_ascii_digit())
}

/// A path's numbers moved by `(dx, dy)`, alternately.
pub fn translate_path(commands: &str, dx: f64, dy: f64) -> String {
    let mut position = 0;
    commands
        .split(' ')
        .map(|token| {
            if !numeric(token) {
                return token.to_owned();
            }
            let value = token.parse::<f64>().unwrap_or(0.0) + if position % 2 == 0 { dx } else { dy };
            position += 1;
            fixed(value, 4)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn to_linear(channel: f64) -> f64 {
    let value = channel / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        pymath::pow((value + 0.055) / 1.055, 2.4)
    }
}

fn to_srgb(value: f64) -> u8 {
    let value = py_max(0.0, py_min(1.0, value));
    let encoded = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * pymath::pow(value, 1.0 / 2.4) - 0.055
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let level = crate::pyfmt::round_half_even(encoded * 255.0) as u8;
    level
}

/// An accent's colour on its `cycle`th repetition: shaded on the first, tinted after.
pub fn cycle_shift(hex: &str, cycle: usize) -> String {
    let text = hex.trim_start_matches('#');
    if text.chars().count() != 6 {
        return hex.to_owned();
    }
    let mut out = String::from("#");
    for index in [0, 2, 4] {
        let channel = text
            .get(index..index + 2)
            .and_then(|c| u8::from_str_radix(c, 16).ok())
            .unwrap_or(0);
        let mut linear = to_linear(f64::from(channel));
        if cycle == 0 {
            linear *= VARY_COLOR_CYCLE_SHADE;
        } else {
            linear += VARY_COLOR_CYCLE_TINT * (1.0 - linear);
        }
        out.push_str(&format!("{:02x}", to_srgb(linear)));
    }
    out
}

/// Where a tangent from `origin` touches the circle round `centre`: the upper or lower one;
/// `None` from inside it.
pub fn tangent_point(origin: Point, centre: Point, radius: f64, upper: bool) -> Option<Point> {
    let (dx, dy) = (centre.0 - origin.0, centre.1 - origin.1);
    let distance = pymath::hypot(&[dx, dy]);
    if distance <= radius || radius <= 0.0 {
        return None;
    }
    let length = (distance * distance - radius * radius).sqrt();
    let base = dy.atan2(dx);
    let spread = (radius / distance).min(1.0).asin();
    let mut candidates: Vec<Point> = [1.0, -1.0]
        .iter()
        .map(|sign| {
            (
                origin.0 + length * (base + sign * spread).cos(),
                origin.1 + length * (base + sign * spread).sin(),
            )
        })
        .collect();
    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    if upper {
        candidates.first().copied()
    } else {
        candidates.last().copied()
    }
}

/// Whole percentages of `values` that add to 100 (largest remainders first).
pub fn percent_shares(values: &[f64]) -> Vec<i64> {
    let total = pymath::sum(values.iter().copied());
    if total <= 0.0 {
        return vec![0; values.len()];
    }
    let exact: Vec<f64> = values.iter().map(|v| v / total * 100.0).collect();
    #[allow(clippy::cast_possible_truncation)]
    let mut floors: Vec<i64> = exact.iter().map(|e| e.floor() as i64).collect();
    let remainder = 100 - floors.iter().sum::<i64>();
    let mut order: Vec<usize> = (0..values.len()).collect();
    #[allow(clippy::cast_precision_loss)]
    let key = |i: usize| -(exact.get(i).copied().unwrap_or(0.0) - floors.get(i).copied().unwrap_or(0) as f64);
    order.sort_by(|&a, &b| {
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    let take = usize::try_from(remainder.max(0)).unwrap_or(0);
    for index in order.into_iter().take(take) {
        if let Some(f) = floors.get_mut(index) {
            *f += 1;
        }
    }
    floors
}

/// Points split into runs at each blank.
pub fn split_runs<T: Clone>(points: &[Option<T>]) -> Vec<Vec<T>> {
    let mut runs = Vec::new();
    let mut current = Vec::new();
    for point in points {
        match point {
            None => {
                if !current.is_empty() {
                    runs.push(std::mem::take(&mut current));
                }
            }
            Some(p) => current.push(p.clone()),
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// A closed ring of points rotated to start at its last blank.
pub fn rotate_past_blank<T: Clone>(points: &[Option<T>]) -> Vec<Option<T>> {
    let blanks: Vec<usize> = points
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_none())
        .map(|(i, _)| i)
        .collect();
    match blanks.last() {
        Some(&start) if blanks.len() != points.len() => {
            let (head, tail) = points.split_at(start);
            tail.iter().chain(head.iter()).cloned().collect()
        }
        _ => points.to_vec(),
    }
}
