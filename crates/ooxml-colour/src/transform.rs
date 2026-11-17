//! Owns: the colour arithmetic -- sRGB and linear light, HLS by the textbook formula with the
//! saturation unbounded, each transform in document order with the channels clamped between
//! steps and kept to the rules' precision, and the levels at the end -- equal, operation for
//! operation, to ooxml-common's `drawingml/color.py` (`colorsys` as Python 3.12 has it).
//!
//! Must not: look up a scheme colour (`crate::context`) or know which application a rule set
//! stands for.

use crate::rules::ColourRules;
use crate::source::{Transform, TransformKind};
use crate::{ColourError, ResolvedColour};

/// Python's float `%`: the result takes the sign of the divisor.
pub(crate) fn py_mod(x: f64, y: f64) -> f64 {
    let m = x % y;
    if m != 0.0 {
        if (y < 0.0) != (m < 0.0) {
            m + y
        } else {
            m
        }
    } else {
        0.0_f64.copysign(y)
    }
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// A linear-light channel, 0-1, as an sRGB level on 0-255 (unrounded).
pub fn linear_to_srgb(value: f64) -> f64 {
    let value = clamp01(value);
    if value <= 0.003_130_8 {
        value * 12.92 * 255.0
    } else {
        (1.055 * value.powf(1.0 / 2.4) - 0.055) * 255.0
    }
}

/// An sRGB channel, 0-1, in linear light, 0-1.
pub fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// `colorsys.rgb_to_hls` (Python 3.12).
fn rgb_to_hls(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let sumc = maxc + minc;
    let rangec = maxc - minc;
    let l = sumc / 2.0;
    if minc == maxc {
        return (0.0, l, 0.0);
    }
    let s = if l <= 0.5 {
        rangec / sumc
    } else {
        rangec / (2.0 - maxc - minc)
    };
    let rc = (maxc - r) / rangec;
    let gc = (maxc - g) / rangec;
    let bc = (maxc - b) / rangec;
    let h = if r == maxc {
        bc - gc
    } else if g == maxc {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    (py_mod(h / 6.0, 1.0), l, s)
}

/// HLS to RGB by the textbook formula, the saturation not clamped (both applications let it
/// pass 1 and clamp the channels instead; PowerPoint lets it go below 0).
fn hls_to_rgb_unbounded(hue: f64, lum: f64, sat: f64) -> [f64; 3] {
    if sat == 0.0 || lum == 0.0 || lum == 1.0 {
        return [lum, lum, lum];
    }
    let high = if lum < 0.5 {
        lum * (1.0 + sat)
    } else {
        lum + sat - lum * sat
    };
    let low = 2.0 * lum - high;
    let channel = |t: f64| {
        let t = py_mod(t, 1.0);
        if t < 1.0 / 6.0 {
            low + (high - low) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            high
        } else if t < 2.0 / 3.0 {
            low + (high - low) * (2.0 / 3.0 - t) * 6.0
        } else {
            low
        }
    };
    [channel(hue + 1.0 / 3.0), channel(hue), channel(hue - 1.0 / 3.0)]
}

/// What a channel becomes before the next step: itself, or its linear-light value in
/// `1 / precision`, as sRGB again.
fn keep(precision: Option<u32>, channel: f64) -> f64 {
    match precision {
        None => channel,
        Some(p) => {
            let p = f64::from(p);
            let linear = srgb_to_linear(clamp01(channel));
            linear_to_srgb((linear * p + 0.5).floor() / p) / 255.0
        }
    }
}

/// One HLS transform on clamped channels.
fn hls_step(rgb: [f64; 3], kind: TransformKind, value: f64, rules: &ColourRules) -> [f64; 3] {
    let amount = value / 100_000.0;
    let [r, g, b] = rgb;
    let achromatic = r.max(g).max(b) == r.min(g).min(b);
    let (mut hue, mut lum, mut sat) = rgb_to_hls(r, g, b);
    match kind {
        TransformKind::LumMod => lum = clamp01(lum * amount),
        TransformKind::LumOff => lum = clamp01(lum + amount),
        TransformKind::SatMod if rules.negative_saturation => sat *= amount,
        TransformKind::SatMod => sat = (sat * amount).max(0.0),
        TransformKind::SatOff if rules.negative_saturation => sat += amount,
        TransformKind::SatOff => sat = (sat + amount).max(0.0),
        TransformKind::HueMod => hue = py_mod(hue * amount, 1.0),
        TransformKind::HueOff => hue = py_mod(hue + value / 60_000.0 / 360.0, 1.0),
        _ => hue = py_mod(hue + 0.5, 1.0),
    }
    let mut out = hls_to_rgb_unbounded(hue, lum, sat);
    if achromatic && sat != 0.0 && rules.achromatic_extrapolates {
        // PowerPoint's grey is red at hue 0, its blue a third of a turn back on the ramp,
        // unwrapped.
        out[2] = 3.0 * out[1] - 2.0 * out[0];
    }
    out
}

/// One transform that is not HLS; `alpha` is the opacity so far.
fn other_step(rgb: [f64; 3], alpha: &mut f64, transform: Transform, rules: &ColourRules) -> [f64; 3] {
    let amount = transform.value / 100_000.0;
    let each = |f: &dyn Fn(f64) -> f64| rgb.map(f);
    match transform.kind {
        TransformKind::Tint => each(&|c| linear_to_srgb(srgb_to_linear(c) * amount + (1.0 - amount)) / 255.0),
        TransformKind::Shade => each(&|c| linear_to_srgb(srgb_to_linear(c) * amount) / 255.0),
        TransformKind::Inv => each(&|c| linear_to_srgb(1.0 - srgb_to_linear(c)) / 255.0),
        TransformKind::Gray => {
            let [r, g, b] = rgb;
            let luma = 0.0 + rules.luma[0] * r + rules.luma[1] * g + rules.luma[2] * b;
            [luma, luma, luma]
        }
        TransformKind::Gamma => each(&|c| linear_to_srgb(c) / 255.0),
        TransformKind::InvGamma => each(&srgb_to_linear),
        TransformKind::Alpha => {
            *alpha = amount;
            rgb
        }
        TransformKind::AlphaMod => {
            *alpha = clamp01(*alpha * amount);
            rgb
        }
        TransformKind::AlphaOff => {
            *alpha = clamp01(*alpha + amount);
            rgb
        }
        _ => rgb,
    }
}

/// Each transform in document order on sRGB channels 0-1, clamped between steps and kept to
/// the rules' precision except within a run of HLS transforms, rounded to levels at the end.
pub fn apply_sequential(rgb: [f64; 3], transforms: &[Transform], rules: &ColourRules) -> ResolvedColour {
    let mut alpha = 1.0;
    let mut rgb = rgb.map(|c| keep(rules.precision, c));
    let applied: Vec<Transform> = transforms.iter().copied().filter(|t| rules.applies(t.kind)).collect();
    for (index, transform) in applied.iter().enumerate() {
        let clamped = rgb.map(clamp01);
        rgb = if transform.kind.is_hls() {
            hls_step(clamped, transform.kind, transform.value, rules)
        } else {
            other_step(clamped, &mut alpha, *transform, rules)
        };
        let following = applied.get(index + 1).map(|t| t.kind);
        if !(transform.kind.is_hls() && following.is_some_and(TransformKind::is_hls)) {
            // A run of HLS transforms stays on sRGB channels; it is kept where it ends.
            rgb = rgb.map(|c| keep(rules.precision, c));
        }
    }
    let level = |c: f64| {
        let v = rules.rounding.level(c * 255.0).clamp(0.0, 255.0);
        // A level clamped to 0-255 is a whole number in range.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = v as u8;
        byte
    };
    ResolvedColour {
        rgb: rgb.map(level),
        alpha,
    }
}

/// Python's `int(text, 16)` on a short slice: surrounding whitespace, an optional sign, hex
/// digits.
fn py_hex(text: &str) -> Option<i64> {
    let s = text.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c));
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, s.get(1..)?),
        Some(b'+') => (false, s.get(1..)?),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = i64::from_str_radix(digits, 16).ok()?;
    Some(if negative { -value } else { value })
}

/// A stated hex value as the channels resolution starts from (0-1, before clamping):
/// `#` dropped, padded on the left with zeros to six characters and cut to six, each pair
/// read as Python's `int(pair, 16)` reads it.
pub fn hex_channels(value: &str) -> Result<[f64; 3], ColourError> {
    let normalized: Vec<char> = value.trim_start_matches('#').to_lowercase().chars().collect();
    let padded: Vec<char> = std::iter::repeat_n('0', 6usize.saturating_sub(normalized.len()))
        .chain(normalized)
        .take(6)
        .collect();
    let pair = |k: usize| -> Result<f64, ColourError> {
        let text: String = padded.iter().skip(k).take(2).collect();
        // Channels are at most 255 in magnitude: exact in f64.
        #[allow(clippy::cast_precision_loss)]
        py_hex(&text)
            .map(|v| v as f64 / 255.0)
            .ok_or_else(|| ColourError::Hex(value.to_owned()))
    };
    Ok([pair(0)?, pair(2)?, pair(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{POWERPOINT, WORD};

    fn t(kind: TransformKind, value: f64) -> Transform {
        Transform { kind, value }
    }

    fn hex(rgb: [f64; 3], transforms: &[Transform], rules: &ColourRules) -> String {
        apply_sequential(rgb, transforms, rules).hex()
    }

    #[test]
    fn the_measured_swatches_named_in_the_python_docstring() {
        let blue = hex_channels("4472C4").unwrap();
        // Word: lumOff 40000 before lumMod 60000.
        let word = hex(
            blue,
            &[t(TransformKind::LumOff, 40000.0), t(TransformKind::LumMod, 60000.0)],
            &WORD,
        );
        assert_eq!(word, "#517cc8");
        let black = hex_channels("000000").unwrap();
        let half = [t(TransformKind::LumMod, 50000.0), t(TransformKind::LumOff, 50000.0)];
        assert_eq!(hex(black, &half, &WORD), "#7f7f7f");
        assert_eq!(hex(blue, &[t(TransformKind::SatMod, 200_000.0)], &WORD), "#0460ff");
        let orange = hex_channels("ED7D31").unwrap();
        let tint = [t(TransformKind::Tint, 95000.0), t(TransformKind::SatMod, 170_000.0)];
        assert_eq!(hex(orange, &tint, &POWERPOINT), "#ff7818");
        assert_eq!(hex(orange, &[t(TransformKind::Gray, 0.0)], &POWERPOINT), "#8f8f8f");
        let grey = hex_channels("808080").unwrap();
        assert_eq!(hex(grey, &[t(TransformKind::SatOff, 25000.0)], &POWERPOINT), "#a06000");
        assert_eq!(hex(black, &[t(TransformKind::LumOff, 50000.0)], &POWERPOINT), "#7f7f7f");
    }

    #[test]
    fn hex_values_read_as_python_reads_them() {
        assert_eq!(hex_channels("#fff").unwrap(), [0.0, 15.0 / 255.0, 1.0]);
        assert!(hex_channels("zzzzzz").is_err());
        assert_eq!(py_mod(-0.25, 1.0), 0.75);
    }
}
