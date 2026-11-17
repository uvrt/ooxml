//! Owns: the arithmetic the reference player gets from its language, reproduced so that the
//! same input gives the same numbers -- a compensated float sum (Neumaier's, which `sum`
//! uses), `hypot` as a correctly scaled two-term norm, rounding half to even, and floor and
//! ceiling that refuse what is not finite.
//!
//! Must not: be used for anything but reproducing those results.

use crate::error::ReadError;

/// The sum of `values` as the reference sums floats: the first value, then Neumaier's
/// compensated additions, the compensation added at the end; `0` for none.
pub fn fsum<I: IntoIterator<Item = f64>>(values: I) -> f64 {
    let mut iter = values.into_iter();
    let Some(mut total) = iter.next() else {
        return 0.0;
    };
    let mut c = 0.0;
    for x in iter {
        let t = total + x;
        if total.abs() >= x.abs() {
            c += (total - t) + x;
        } else {
            c += (x - t) + total;
        }
        total = t;
    }
    if c != 0.0 && c.is_finite() {
        total += c;
    }
    total
}

fn dl_mul(x: f64, y: f64) -> (f64, f64) {
    let z = x * y;
    (z, x.mul_add(y, -z))
}

fn dl_fast_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    (x, (a - x) + b)
}

/// `sqrt(x * x + y * y)`, computed as the reference computes it (a scaled sum of exact
/// squares with a correction step), so that the last bit agrees.
pub fn hypot(x: f64, y: f64) -> f64 {
    let (ax, ay) = (x.abs(), y.abs());
    if ax.is_infinite() || ay.is_infinite() {
        return f64::INFINITY;
    }
    if ax.is_nan() || ay.is_nan() {
        return f64::NAN;
    }
    let max = ax.max(ay);
    if max == 0.0 {
        return max;
    }
    let max_e = frexp_exponent(max);
    if max_e < -1023 {
        let tiny = f64::MIN_POSITIVE;
        return tiny * hypot(ax / tiny, ay / tiny);
    }
    let scale = 2f64.powi(-max_e);
    let (mut csum, mut frac1, mut frac2) = (1.0f64, 0.0f64, 0.0f64);
    for v in [ax, ay] {
        let (hi, lo) = dl_mul(v * scale, v * scale);
        let (s, slo) = dl_fast_sum(csum, hi);
        csum = s;
        frac1 += lo;
        frac2 += slo;
    }
    let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
    let (hi, lo) = dl_mul(-h, h);
    let (s, slo) = dl_fast_sum(csum, hi);
    csum = s;
    frac1 += lo;
    frac2 += slo;
    let x = csum - 1.0 + (frac1 + frac2);
    h += x / (2.0 * h);
    h / scale
}

/// The exponent `e` of `frexp(v)`: `v = m * 2^e` with `0.5 <= |m| < 1`.
fn frexp_exponent(v: f64) -> i32 {
    let bits = v.to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i32;
    if exp == 0 {
        // Subnormal: normalise.
        let lead = (bits << 12).leading_zeros() as i32;
        return -1022 - lead;
    }
    exp - 1022
}

/// A float rounded to the nearest integer, ties to even (the reference's `round(x)`).
pub fn round_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// `floor(x)`, refused for a value that is not finite (the reference raises).
pub fn floor(x: f64) -> Result<f64, ReadError> {
    if x.is_finite() {
        Ok(x.floor())
    } else {
        Err(ReadError::NotFinite)
    }
}

/// `ceil(x)`, refused for a value that is not finite.
pub fn ceil(x: f64) -> Result<f64, ReadError> {
    if x.is_finite() {
        Ok(x.ceil())
    } else {
        Err(ReadError::NotFinite)
    }
}

/// Degrees to radians, as the reference converts them.
pub fn radians(deg: f64) -> f64 {
    deg * (std::f64::consts::PI / 180.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_and_norms() {
        assert_eq!(fsum([0.1, 0.2, 0.3]), 0.6);
        assert_eq!(fsum(Vec::<f64>::new()), 0.0);
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, -2.0), 2.0);
        assert_eq!(round_even(2.5), 2.0);
        assert_eq!(round_even(3.5), 4.0);
        assert!(floor(f64::INFINITY).is_err());
    }
}
