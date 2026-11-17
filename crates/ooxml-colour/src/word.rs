//! Owns: the colour Word draws for a WordprocessingML theme colour with a tint or shade
//! (`w:themeTint`, `w:themeShade`, `w:themeFillTint`...): Windows' integer HLS on a scale of
//! 1530, every step rounded down but the hue's per-channel terms, which round up -- measured
//! in Word 16.106 on 560 swatches (ooxml-common's `themetint.py` holds the measurement and
//! the rule). Word draws the theme colour, not the `w:val` beside it.
//!
//! Must not: implement DrawingML transforms (`crate::transform`): these attributes are not
//! DrawingML transforms, and Word does not compute them as one.

const SCALE: i64 = 1530;
const SECTOR: i64 = SCALE / 6;

fn hex_byte(value: Option<&str>) -> Option<i64> {
    let v = value?;
    let n = i64::from_str_radix(v.trim(), 16).ok()?;
    (0..=255).contains(&n).then_some(n)
}

fn floor_div(a: i64, b: i64) -> i64 {
    a.div_euclid(b)
}

/// `base` (`RRGGBB`, a `#` allowed) with Word's `w:themeTint` / `w:themeShade` applied as
/// Word draws it, upper-case; with both, the tint alone counts. `None` for a base that is
/// not six hex digits.
pub fn word_theme_tint(base: &str, tint: Option<&str>, shade: Option<&str>) -> Option<String> {
    let base = base.trim_start_matches('#').to_uppercase();
    let channel = |k: usize| {
        base.get(k..k + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(i64::from)
    };
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let tinting = hex_byte(tint).is_some();
    let amount = if tinting { hex_byte(tint) } else { hex_byte(shade) };
    let Some(amount) = amount.filter(|a| *a != 255) else {
        return Some(base);
    };
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let lum = 3 * (high + low);
    let lum2 = if tinting {
        floor_div(lum * amount + SCALE * (255 - amount), 255)
    } else {
        floor_div(lum * amount, 255)
    };
    if high == low {
        let grey = floor_div(lum2 * 255, SCALE);
        return Some(format!("{grey:02X}{grey:02X}{grey:02X}"));
    }
    let span = high - low;
    let sat = if high + low <= 255 {
        floor_div(span * SCALE, high + low)
    } else {
        floor_div(span * SCALE, 510 - high - low)
    };
    let term = |c: i64| -floor_div(-(high - c) * SECTOR, span);
    let (rd, gd, bd) = (term(r), term(g), term(b));
    let hue = if r == high {
        bd - gd
    } else if g == high {
        SCALE / 3 + rd - bd
    } else {
        2 * SCALE / 3 + gd - rd
    }
    .rem_euclid(SCALE);
    let m2 = if 2 * lum2 <= SCALE {
        floor_div(lum2 * (SCALE + sat), SCALE)
    } else {
        lum2 + sat - floor_div(lum2 * sat, SCALE)
    };
    let m1 = 2 * lum2 - m2;
    let ramp = |h: i64| {
        let h = h.rem_euclid(SCALE);
        let v = if h < SECTOR {
            m1 + floor_div((m2 - m1) * h, SECTOR)
        } else if h < SCALE / 2 {
            m2
        } else if h < 2 * SCALE / 3 {
            m1 + floor_div((m2 - m1) * (2 * SCALE / 3 - h), SECTOR)
        } else {
            m1
        };
        floor_div(v * 255, SCALE).clamp(0, 255)
    };
    Some(format!(
        "{:02X}{:02X}{:02X}",
        ramp(hue + SCALE / 3),
        ramp(hue),
        ramp(hue - SCALE / 3)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tints_and_shades_as_word_draws_them() {
        assert_eq!(word_theme_tint("4472C4", None, None).as_deref(), Some("4472C4"));
        assert_eq!(word_theme_tint("4472C4", None, Some("FF")).as_deref(), Some("4472C4"));
        assert_eq!(word_theme_tint("808080", Some("00"), None).as_deref(), Some("FFFFFF"));
        assert_eq!(word_theme_tint("808080", None, Some("00")).as_deref(), Some("000000"));
        assert_eq!(word_theme_tint("zz", None, None), None);
    }
}
