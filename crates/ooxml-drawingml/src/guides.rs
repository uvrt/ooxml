//! Owns: the guide-formula language (ECMA-376 20.1.9.11) -- the built-in names, a formula's
//! value, a guide list evaluated in order -- and `a:arcTo` turned into SVG arcs, equal to
//! ooxml-common's `drawingml/guides.py` operation for operation (Python's float semantics:
//! `round` to even, `min`/`max` keeping the first of equals, `math.radians` and
//! `math.degrees` as Python computes them).
//!
//! Must not: draw (painting is phase P3c's), or round where the Python does not.

use ooxml_xml::view::py_float;
use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

/// One degree in DrawingML's 1/60000-degree unit.
pub const DEGREE: f64 = 60000.0;

const ANGLES: [(&str, f64); 7] = [
    ("cd8", 2_700_000.0),
    ("cd4", 5_400_000.0),
    ("cd2", 10_800_000.0),
    ("3cd8", 8_100_000.0),
    ("3cd4", 16_200_000.0),
    ("5cd8", 13_500_000.0),
    ("7cd8", 18_900_000.0),
];

const DIVISORS: [u8; 12] = [2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24, 32];

/// Guide values by name.
pub type Variables = HashMap<String, f64>;

/// Python's `min(a, b)`: `b` only where it is smaller.
fn py_min(a: f64, b: f64) -> f64 {
    if b < a {
        b
    } else {
        a
    }
}

/// Python's `max(a, b)`: `b` only where it is larger.
fn py_max(a: f64, b: f64) -> f64 {
    if b > a {
        b
    } else {
        a
    }
}

/// Python's `x ** 2`: the platform's `pow`, which is not always `x * x` rounded once (on
/// macOS `28.148751785860103 ** 2` is a unit in the last place below the product). The
/// exponent is hidden from the optimiser, which would otherwise rewrite the call as `x * x`.
fn squared(x: f64) -> f64 {
    x.powf(std::hint::black_box(2.0))
}

/// `math.radians`.
fn radians(degrees: f64) -> f64 {
    degrees * (PI / 180.0)
}

/// `math.degrees`.
fn degrees(radians: f64) -> f64 {
    radians * (180.0 / PI)
}

/// The names every formula may use without declaring them, for a `width` x `height` shape.
pub fn builtin_variables(width: f64, height: f64) -> Variables {
    let shortest = py_min(width, height);
    let mut v: Variables = [
        ("w", width),
        ("h", height),
        ("l", 0.0),
        ("t", 0.0),
        ("r", width),
        ("b", height),
        ("hc", width / 2.0),
        ("vc", height / 2.0),
        ("ss", shortest),
        ("ls", py_max(width, height)),
    ]
    .into_iter()
    .map(|(k, x)| (k.to_owned(), x))
    .collect();
    for d in DIVISORS {
        let d64 = f64::from(d);
        v.insert(format!("wd{d}"), width / d64);
        v.insert(format!("hd{d}"), height / d64);
        v.insert(format!("ssd{d}"), shortest / d64);
    }
    v.extend(ANGLES.iter().map(|(k, x)| ((*k).to_owned(), *x)));
    v
}

/// A token's value: a literal as Python's `float()` reads it, else a name (0 where unknown).
pub fn resolve_value(token: &str, variables: &Variables) -> f64 {
    py_float(token).unwrap_or_else(|| variables.get(token).copied().unwrap_or(0.0))
}

/// Guide lists evaluated in order into one namespace, each guide seeing those before it.
/// `precise` keeps every value unrounded (presets, in pixels); otherwise the operators that
/// divide or take a root or an angle round to whole units, as Office does in EMU.
pub fn evaluate_guides(lists: &[&[(String, String)]], width: f64, height: f64, precise: bool) -> Variables {
    let mut variables = builtin_variables(width, height);
    for list in lists {
        for (name, formula) in *list {
            let value = evaluate_formula(formula, &variables, precise);
            variables.insert(name.clone(), value);
        }
    }
    variables
}

/// `text.split()` as Python splits on whitespace.
pub(crate) fn py_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|t| !t.is_empty())
}

/// One formula's value; an unknown operator is 0.
pub fn evaluate_formula(formula: &str, variables: &Variables, precise: bool) -> f64 {
    let tokens: Vec<&str> = py_words(formula).collect();
    let operand = |i: usize| tokens.get(i).map_or(0.0, |t| resolve_value(t, variables));
    let q = |v: f64| if precise { v } else { v.round_ties_even() };
    let nonzero = |v: f64| if v == 0.0 { 1.0 } else { v };
    let (o1, o2, o3) = (operand(1), operand(2), operand(3));
    match tokens.first().copied().unwrap_or("") {
        "val" => o1,
        "+-" => o1 + o2 - o3,
        "*/" => q((o1 * o2) / nonzero(o3)),
        "+/" => q((o1 + o2) / nonzero(o3)),
        "pin" => py_max(o1, py_min(o2, o3)),
        "min" => py_min(o1, o2),
        "max" => py_max(o1, o2),
        "abs" => o1.abs(),
        "sqrt" => q(py_max(0.0, o1).sqrt()),
        "sin" => q(o1 * radians(o2 / DEGREE).sin()),
        "cos" => q(o1 * radians(o2 / DEGREE).cos()),
        "tan" => q(o1 * radians(o2 / DEGREE).tan()),
        "at2" => q(degrees(o2.atan2(o1)) * DEGREE),
        "mod" => q((squared(o1) + squared(o2) + squared(o3)).sqrt()),
        "cat2" => q(o1 * o3.atan2(o2).cos()),
        "sat2" => q(o1 * o3.atan2(o2).sin()),
        "?:" if o1 > 0.0 => o2,
        "?:" => o3,
        _ => 0.0,
    }
}

/// A geometric angle (radians) as the ellipse's parametric angle, the revolution kept.
fn parametric_angle(geometric: f64, width_radius: f64, height_radius: f64) -> f64 {
    if width_radius == height_radius {
        return geometric;
    }
    let t = (width_radius * geometric.sin()).atan2(height_radius * geometric.cos());
    t + TAU * ((geometric - t) / TAU).round_ties_even()
}

/// One SVG arc: its end point, and the large-arc and sweep flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcSegment {
    /// End x.
    pub x: f64,
    /// End y.
    pub y: f64,
    /// The large-arc flag.
    pub large: u8,
    /// The sweep flag.
    pub sweep: u8,
}

/// An `a:arcTo` from the current point as SVG arcs of at most half a turn each (a full
/// circle is one `arcTo`, which a single SVG arc cannot draw). Angles in 1/60000 degree.
pub fn arc_segments(x: f64, y: f64, width_radius: f64, height_radius: f64, start: f64, swing: f64) -> Vec<ArcSegment> {
    let start_parametric = parametric_angle(radians(start / DEGREE), width_radius, height_radius);
    let end_parametric = parametric_angle(radians((start + swing) / DEGREE), width_radius, height_radius);
    let centre_x = x - width_radius * start_parametric.cos();
    let centre_y = y - height_radius * start_parametric.sin();
    let total = end_parametric - start_parametric;
    if total == 0.0 {
        return Vec::new();
    }
    let count = (total.abs() / PI - 1e-9).ceil().max(1.0);
    let step = total / count;
    // `count` is a small whole number of half turns.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pieces = count as u32;
    (1..=pieces)
        .map(|index| {
            let angle = start_parametric + step * f64::from(index);
            ArcSegment {
                x: centre_x + width_radius * angle.cos(),
                y: centre_y + height_radius * angle.sin(),
                large: u8::from(step.abs() > PI),
                sweep: u8::from(step > 0.0),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_evaluate_as_the_python_evaluates_them() {
        let v = builtin_variables(100.0, 50.0);
        assert_eq!(evaluate_formula("*/ w 1 3", &v, false), 33.0);
        assert_eq!(evaluate_formula("*/ w 1 3", &v, true), 100.0 / 3.0);
        assert_eq!(evaluate_formula("pin 0 200 100", &v, false), 100.0);
        assert_eq!(evaluate_formula("?: -1 2 3", &v, false), 3.0);
        assert_eq!(evaluate_formula("cos wd2 2700000", &v, false), 35.0);
        assert_eq!(evaluate_formula("nonsense 1", &v, false), 0.0);
        assert_eq!(arc_segments(0.0, 0.0, 10.0, 10.0, 0.0, 21_600_000.0).len(), 2);
    }
}
