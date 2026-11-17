//! Owns: how a number is written in the SVG -- at most three decimals, no trailing zeros,
//! integers without a point, `0` for a value that is not finite (ooxml-common's
//! `metafile.gdi.num`, which every Python SVG writer of the metafiles uses).
//!
//! Must not: round anything that is not written.

/// `value` as the SVG writes it: rounded to three decimals (ties to even on the exact binary
/// value, as Python's `round` and `format` do), trailing zeros and a bare point removed, `-0`
/// written `0`, and a value that is not finite written `0`.
pub fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let text = format!("{value:.3}");
    let trimmed = match text.strip_suffix(".000") {
        Some(whole) => whole,
        None => text.trim_end_matches('0'),
    };
    if trimmed == "-0" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::num;

    #[test]
    fn numbers_are_written_as_the_python_writes_them() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.0004), "0");
        assert_eq!(num(0.0625), "0.062");
        assert_eq!(num(123.4565), "123.457");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-12.25), "-12.25");
        assert_eq!(num(1e20), "100000000000000000000");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(f64::INFINITY), "0");
    }
}
