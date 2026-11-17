//! Owns: a value axis' scale -- its extent and major unit ([`Scale`], `nice_axis_scale`) and
//! how many intervals an axis has room for (`side_axis_intervals`, `radial_axis_intervals`,
//! `bottom_axis_intervals`) -- as `chart/layout.py` measured them, with every comparison
//! and slack the Python makes.
//!
//! Must not: know which axis it scales, or place anything.

use super::consts::{
    AXIS_EDGE_RESERVE_PT, AXIS_END_LABEL_LINES, AXIS_HEADROOM, AXIS_MAX_INTERVALS, AXIS_ZERO_ANCHOR_RATIO,
    BOTTOM_AXIS_LABEL_GAP_EM, BOTTOM_AXIS_RESERVE_PT, BOTTOM_AXIS_RUNG_EM, DECADE_SLACK, EXTENT_SLACK,
    RADIAL_AXIS_SLACK_EM,
};
use crate::model::Axis;

/// A value axis' scale: its extent and major unit, and whether it runs reversed
/// (`orientation="maxMin"`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    /// The minimum.
    pub minimum: f64,
    /// The maximum.
    pub maximum: f64,
    /// The major unit.
    pub unit: f64,
    /// Whether it runs from its maximum.
    pub reversed: bool,
}

impl Scale {
    /// An unreversed scale.
    pub fn new(minimum: f64, maximum: f64, unit: f64) -> Scale {
        Scale {
            minimum,
            maximum,
            unit,
            reversed: false,
        }
    }

    /// The scale reversed where `axis` runs `maxMin` (and not already).
    pub fn reversed_for(self, axis: Option<&Axis>) -> Scale {
        match axis {
            Some(axis) if axis.orientation == "maxMin" => Scale { reversed: true, ..self },
            _ => self,
        }
    }
}

/// Python's `math.floor` of a finite float as an integer.
fn floor_int(value: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let int = value.floor() as i64;
    int
}

/// How many intervals a vertical value axis has room for.
pub fn side_axis_intervals(available_pt: f64, pitch_pt: f64) -> i64 {
    if !(available_pt.is_finite() && pitch_pt.is_finite()) || pitch_pt <= 0.0 {
        return AXIS_MAX_INTERVALS;
    }
    let rungs = (available_pt - AXIS_EDGE_RESERVE_PT) / pitch_pt;
    if !rungs.is_finite() {
        return AXIS_MAX_INTERVALS;
    }
    (floor_int(rungs) - AXIS_END_LABEL_LINES).min(AXIS_MAX_INTERVALS).max(1)
}

/// How many intervals a radar's radial axis has room for.
pub fn radial_axis_intervals(radial_pt: f64, pitch_pt: f64) -> i64 {
    if !(radial_pt.is_finite() && pitch_pt.is_finite()) || pitch_pt <= 0.0 {
        return AXIS_MAX_INTERVALS;
    }
    let rungs = radial_pt / pitch_pt + RADIAL_AXIS_SLACK_EM;
    if !rungs.is_finite() {
        return AXIS_MAX_INTERVALS;
    }
    floor_int(rungs).min(AXIS_MAX_INTERVALS).max(1)
}

/// How many intervals a horizontal value axis has room for.
pub fn bottom_axis_intervals(available_pt: f64, font_size_pt: f64, widest_label_pt: f64) -> i64 {
    if !(available_pt.is_finite() && font_size_pt.is_finite()) || font_size_pt <= 0.0 {
        return AXIS_MAX_INTERVALS;
    }
    let rung = py_max(
        BOTTOM_AXIS_RUNG_EM * font_size_pt,
        widest_label_pt + BOTTOM_AXIS_LABEL_GAP_EM * font_size_pt,
    );
    if rung <= 0.0 || !rung.is_finite() {
        return AXIS_MAX_INTERVALS;
    }
    let rungs = floor_int((available_pt - BOTTOM_AXIS_RESERVE_PT) / rung);
    rungs.min(AXIS_MAX_INTERVALS).max(1)
}

/// Python's `max(a, b)` of floats: `a` unless `b` is greater.
pub fn py_max(a: f64, b: f64) -> f64 {
    if b > a {
        b
    } else {
        a
    }
}

/// Python's `min(a, b)`: `a` unless `b` is less.
pub fn py_min(a: f64, b: f64) -> f64 {
    if b < a {
        b
    } else {
        a
    }
}

/// The smallest extent containing `low..high` with `clearance` of it free beyond the data.
fn clearance_extent(low: f64, high: f64, data_minimum: f64, data_maximum: f64, clearance: f64) -> (f64, f64) {
    if clearance <= 0.0 || !clearance.is_finite() {
        return (low, high);
    }
    let clearance = py_min(clearance, 0.45);
    let mut candidates = vec![(low, high)];
    if clearance < 1.0 {
        candidates.push((low, (data_maximum - clearance * low) / (1.0 - clearance)));
        candidates.push(((data_minimum - clearance * high) / (1.0 - clearance), high));
    }
    let span = (data_maximum - data_minimum) / (1.0 - 2.0 * clearance);
    candidates.push((data_minimum - clearance * span, data_maximum + clearance * span));
    let mut best: Option<(f64, f64)> = None;
    for (candidate_low, candidate_high) in candidates {
        let bottom = py_min(candidate_low, low);
        let top = py_max(candidate_high, high);
        let width = top - bottom;
        if !width.is_finite() || width <= 0.0 {
            continue;
        }
        let room = clearance * width;
        let slack = EXTENT_SLACK * py_max(width, 1.0);
        if data_minimum - room < bottom - slack || data_maximum + room > top + slack {
            continue;
        }
        if best.is_none_or(|(b, t)| width < t - b) {
            best = Some((bottom, top));
        }
    }
    best.unwrap_or((low, high))
}

/// `nice_axis_scale`'s options.
#[derive(Debug, Clone, Copy)]
pub struct NiceOptions {
    /// The most intervals.
    pub intervals: i64,
    /// Whether the extent is padded by the headroom.
    pub strict: bool,
    /// Whether zero is kept in the extent.
    pub anchor_zero: bool,
    /// The fraction of the extent kept free beyond the data.
    pub clearance: f64,
}

impl Default for NiceOptions {
    fn default() -> Self {
        NiceOptions {
            intervals: AXIS_MAX_INTERVALS,
            strict: true,
            anchor_zero: true,
            clearance: 0.0,
        }
    }
}

/// A value axis' extent and unit for data from `data_minimum` to `data_maximum`.
pub fn nice_axis_scale(data_minimum: f64, data_maximum: f64, options: NiceOptions) -> Scale {
    let fallback = Scale::new(0.0, 1.0, 1.0);
    let (mut low, mut high) = (py_min(0.0, data_minimum), py_max(0.0, data_maximum));
    if !options.anchor_zero && floats_away_from_zero(data_minimum, data_maximum) {
        (low, high) = (data_minimum, data_maximum);
    }
    let anchored = low <= 0.0 && 0.0 <= high;
    let span = high - low;
    if span <= 0.0 || !span.is_finite() {
        return fallback;
    }
    let headroom = if options.strict { AXIS_HEADROOM * span } else { 0.0 };
    let padded_low = if anchored && low >= 0.0 { low } else { low - headroom };
    let padded_high = if anchored && high <= 0.0 { high } else { high + headroom };
    let (padded_low, padded_high) =
        clearance_extent(padded_low, padded_high, data_minimum, data_maximum, options.clearance);
    #[allow(clippy::cast_precision_loss)]
    let intervals = options.intervals.max(1) as f64;
    let unit = nice_unit((padded_high - padded_low) / intervals);
    if !unit.is_finite() || unit <= 0.0 {
        return fallback;
    }
    let (ceiling, floor) = if options.strict {
        (padded_high, padded_low)
    } else {
        (high, low)
    };
    let mut maximum = (ceiling / unit - EXTENT_SLACK).ceil() * unit;
    let mut minimum = (floor / unit + EXTENT_SLACK).floor() * unit;
    if !(minimum.is_finite() && maximum.is_finite() && maximum > minimum) {
        return fallback;
    }
    if options.clearance > 0.0 {
        for _ in 0..AXIS_MAX_INTERVALS {
            let extent = maximum - minimum;
            let room = py_min(options.clearance, 0.45) * extent;
            let slack = EXTENT_SLACK * py_max(extent, 1.0);
            let below = data_minimum - room < minimum - slack;
            let above = data_maximum + room > maximum + slack;
            if !(below || above) {
                break;
            }
            if below {
                minimum -= unit;
            }
            if above {
                maximum += unit;
            }
        }
    }
    Scale::new(minimum, maximum, unit)
}

/// The smallest of 1, 2 and 5 times a power of ten at least `value`.
fn nice_unit(value: f64) -> f64 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    let decade = decade(value);
    if decade <= 0.0 || !decade.is_finite() {
        return 0.0;
    }
    for step in [1.0, 2.0, 5.0] {
        let candidate = step * decade;
        if value <= candidate * (1.0 + DECADE_SLACK) {
            return candidate;
        }
    }
    10.0 * decade
}

/// Whether data all on one side of zero sits far enough from it to float.
fn floats_away_from_zero(data_minimum: f64, data_maximum: f64) -> bool {
    if !(data_minimum.is_finite() && data_maximum.is_finite()) {
        return false;
    }
    if data_minimum > 0.0 && data_maximum > 0.0 {
        return data_minimum > AXIS_ZERO_ANCHOR_RATIO * data_maximum;
    }
    if data_minimum < 0.0 && data_maximum < 0.0 {
        return data_maximum < AXIS_ZERO_ANCHOR_RATIO * data_minimum;
    }
    false
}

/// `float(f"1e{exponent}")`: the power of ten, correctly rounded.
fn power_of_ten(exponent: i64) -> f64 {
    format!("1e{exponent}").parse().unwrap_or(f64::INFINITY)
}

/// The power of ten `value` reaches.
fn decade(value: f64) -> f64 {
    let mut exponent = floor_int(value.log10());
    while !reaches_decade(value, exponent) {
        exponent -= 1;
    }
    while reaches_decade(value, exponent + 1) {
        exponent += 1;
    }
    power_of_ten(exponent)
}

fn reaches_decade(value: f64, exponent: i64) -> bool {
    value >= power_of_ten(exponent) * (1.0 - DECADE_SLACK)
}

/// A scale with an axis' stated limits applied (unreversed).
pub fn apply_axis_limits(scale: Scale, axis: Option<&Axis>) -> Scale {
    let (mut minimum, mut maximum, mut unit) = (scale.minimum, scale.maximum, scale.unit);
    if let Some(axis) = axis {
        if let Some(m) = axis.minimum {
            minimum = m;
        }
        if let Some(m) = axis.maximum {
            maximum = m;
        }
        if let Some(u) = axis.major_unit.filter(|u| *u > 0.0) {
            unit = u;
        }
    }
    if maximum <= minimum {
        maximum = minimum + if unit != 0.0 { unit } else { 1.0 };
    }
    Scale::new(minimum, maximum, unit)
}

#[cfg(test)]
mod tests {
    use super::{nice_axis_scale, NiceOptions, Scale};

    #[test]
    fn scales_are_nice() {
        assert_eq!(
            nice_axis_scale(0.0, 87.0, NiceOptions::default()),
            Scale::new(0.0, 100.0, 10.0)
        );
        assert_eq!(
            nice_axis_scale(3.0, 3.0, NiceOptions::default()),
            Scale::new(0.0, 3.5, 0.5)
        );
    }
}
