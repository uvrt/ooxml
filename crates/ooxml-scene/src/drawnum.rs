//! Owns: how the DrawingML painter writes a number -- ooxml-common's `drawingml.svg.num`:
//! rounded to three decimals, an integer without a point, anything else in Python's `%g`
//! (six significant digits) -- and the other forms its markup uses: two fixed decimals
//! (`f"{v:.2f}"`, a text line's advance) and a multiplier's ten significant digits
//! (`geometry._factor`).
//!
//! Must not: round anything that is not written.

/// `value` as the DrawingML painter writes it: `round(value, 3)`, then `str(int(r))` where
/// that is an integer and `f"{r:g}"` where it is not. Both roundings are to nearest, ties to
/// even on the exact binary value, as Python's are. A value that is not finite (which the
/// Python cannot write at all) is written `0`.
pub fn drawing_num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let rounded: f64 = format!("{value:.3}").parse().unwrap_or(0.0);
    if rounded == rounded.trunc() {
        return integer(rounded);
    }
    general(rounded, 6)
}

/// `f"{value:.2f}"`.
pub fn fixed2(value: f64) -> String {
    format!("{value:.2}")
}

/// `geometry._factor`: an integer as one, anything else in `%.10g`.
pub fn factor(value: f64) -> String {
    if value.is_finite() && value == value.trunc() {
        return integer(value);
    }
    general(value, 10)
}

/// The value [`drawing_num`] writes, read back (`float(num(v))`): what the Python carries
/// on where it keeps a written number as a string and later adds to it.
pub fn written(value: f64) -> f64 {
    drawing_num(value).parse().unwrap_or(0.0)
}

/// Python's `f"{value:g}"`: six significant digits, trailing zeros dropped, an exponent
/// below 1e-4 and from 1e6.
pub fn python_g(value: f64) -> String {
    general(value, 6)
}

/// `str(int(v))` for an integral float.
fn integer(value: f64) -> String {
    let text = format!("{value:.0}");
    if text == "-0" {
        "0".to_owned()
    } else {
        text
    }
}

/// Python's `%.{precision}g`.
fn general(value: f64, precision: usize) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_owned();
    }
    if !value.is_finite() {
        return if value.is_nan() {
            "nan".to_owned()
        } else if value > 0.0 {
            "inf".to_owned()
        } else {
            "-inf".to_owned()
        };
    }
    let scientific = format!("{:.*e}", precision.saturating_sub(1), value);
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific.as_str(), "0"));
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let limit = i64::try_from(precision).unwrap_or(6);
    if exponent < -4 || exponent >= limit {
        let mantissa = trim_zeros(mantissa);
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exponent.abs());
    }
    let decimals = usize::try_from(limit - 1 - exponent).unwrap_or(0);
    trim_zeros(&format!("{value:.decimals$}")).to_owned()
}

fn trim_zeros(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{drawing_num, factor, fixed2};

    #[test]
    fn numbers_are_written_as_the_drawing_python_writes_them() {
        assert_eq!(drawing_num(1.0), "1");
        assert_eq!(drawing_num(-0.0004), "0");
        assert_eq!(drawing_num(0.0625), "0.062");
        assert_eq!(drawing_num(35.268_4), "35.268");
        assert_eq!(drawing_num(1234.5678), "1234.57");
        assert_eq!(drawing_num(1_000_000.5), "1e+06");
        assert_eq!(drawing_num(-12.25), "-12.25");
        assert_eq!(drawing_num(1e20), "100000000000000000000");
        assert_eq!(fixed2(24.414_5), "24.41");
        assert_eq!(fixed2(-0.0), "-0.00");
        assert_eq!(factor(1.0499e-4), "0.00010499");
        assert_eq!(factor(2.0), "2");
        assert_eq!(factor(1.0 / 3.0), "0.3333333333");
    }
}
