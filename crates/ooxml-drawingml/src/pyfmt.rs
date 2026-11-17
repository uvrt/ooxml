//! Owns: writing a coordinate as ooxml-common writes it into path data (`_fmt` in
//! `drawingml/read.py`): rounded to three decimals as Python's `round(value, 3)` rounds
//! (the exact binary value, ties to even), a whole number without a point, anything else
//! as `%g` writes it (six significant digits).
//!
//! Must not: write any other number format.

/// `round(value, 3)`: the decimal nearest the exact value, ties to even, read back.
fn round3(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    format!("{value:.3}").parse().unwrap_or(value)
}

/// `%g` of a finite, non-zero value.
fn general(value: f64) -> String {
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if (-4..6).contains(&exponent) {
        let decimals = usize::try_from(5 - exponent).unwrap_or(0);
        let fixed = format!("{value:.decimals$}");
        if fixed.contains('.') {
            fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            fixed
        }
    } else {
        let mantissa = if mantissa.contains('.') {
            mantissa.trim_end_matches('0').trim_end_matches('.')
        } else {
            mantissa
        };
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exponent.abs())
    }
}

/// A coordinate as ooxml-common's `_fmt` writes it: `0`, `12.346`, `1.23457e+08`.
pub fn coordinate(value: f64) -> String {
    let rounded = round3(value);
    if !rounded.is_finite() {
        return if rounded.is_nan() {
            "nan".into()
        } else if rounded > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    if rounded == rounded.trunc() {
        let whole = format!("{rounded:.0}");
        return if whole == "-0" { "0".into() } else { whole };
    }
    general(rounded)
}

#[cfg(test)]
mod tests {
    use super::coordinate;

    #[test]
    fn coordinates_write_as_python_writes_them() {
        for (value, written) in [
            (0.0, "0"),
            (-0.0, "0"),
            (12.3456, "12.346"),
            (0.0625, "0.062"),
            (1.0005, "1"),
            (12345.6785, "12345.7"),
            (0.0005, "0.001"),
            (123_456_789.25, "1.23457e+08"),
            (-2.5, "-2.5"),
            (1e17 + 0.5, "100000000000000000"),
        ] {
            assert_eq!(coordinate(value), written, "{value}");
        }
    }
}
