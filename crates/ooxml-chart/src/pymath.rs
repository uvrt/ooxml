//! Owns: Python's arithmetic where it is not IEEE's plain operation -- `sum()` of floats
//! (CPython 3.12's compensated Neumaier sum), `math.hypot` (CPython's correctly scaled
//! vector norm), `math.radians` and `x ** y` (C's `pow`) -- so the layout's numbers are the
//! reference's to the last bit.
//!
//! Must not: hold layout rules.

/// `sum(values)` of floats, as CPython 3.12 adds them (Neumaier's compensation, the
/// compensation added at the end only when finite).
pub fn sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut iter = values.into_iter();
    let Some(first) = iter.next() else {
        return 0.0;
    };
    let mut total = 0.0 + first;
    let mut compensation = 0.0;
    for x in iter {
        let t = total + x;
        if total.abs() >= x.abs() {
            compensation += (total - t) + x;
        } else {
            compensation += (x - t) + total;
        }
        total = t;
    }
    if compensation != 0.0 && compensation.is_finite() {
        total += compensation;
    }
    total
}

/// A double-length product: `x * y` and its exact error.
fn dl_mul(x: f64, y: f64) -> (f64, f64) {
    let z = x * y;
    (z, x.mul_add(y, -z))
}

/// A double-length sum of `a + b` where `|a| >= |b|`.
fn dl_fast_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let z = x - a;
    (x, b - z)
}

/// `frexp`'s exponent.
fn exponent(value: f64) -> i32 {
    if value == 0.0 || !value.is_finite() {
        return 0;
    }
    let bits = value.to_bits();
    let raw = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    if raw == 0 {
        // Subnormal: scale into the normal range first.
        return exponent(value * 2f64.powi(64)) - 64;
    }
    raw - 1022
}

/// CPython's `vector_norm` of non-negative finite values whose largest is `max`.
fn vector_norm(values: &mut [f64], max: f64) -> f64 {
    if max == 0.0 || values.len() <= 1 {
        return max;
    }
    let max_e = exponent(max);
    if max_e < -1023 {
        for v in values.iter_mut() {
            *v /= f64::MIN_POSITIVE;
        }
        return f64::MIN_POSITIVE * vector_norm(values, max / f64::MIN_POSITIVE);
    }
    let scale = 2f64.powi(-max_e);
    let (mut csum, mut frac1, mut frac2) = (1.0, 0.0, 0.0);
    for v in values.iter() {
        let x = v * scale;
        let (hi, lo) = dl_mul(x, x);
        let (s_hi, s_lo) = dl_fast_sum(csum, hi);
        csum = s_hi;
        frac1 += lo;
        frac2 += s_lo;
    }
    let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
    let (hi, lo) = dl_mul(-h, h);
    let (s_hi, s_lo) = dl_fast_sum(csum, hi);
    csum = s_hi;
    frac1 += lo;
    frac2 += s_lo;
    let x = csum - 1.0 + (frac1 + frac2);
    h += x / (2.0 * h);
    h / scale
}

/// `math.hypot(*values)`.
pub fn hypot(values: &[f64]) -> f64 {
    let mut coordinates: Vec<f64> = values.iter().map(|v| v.abs()).collect();
    let mut max = 0.0_f64;
    let mut nan = false;
    for &x in &coordinates {
        nan |= x.is_nan();
        if x > max {
            max = x;
        }
    }
    if max.is_infinite() {
        return max;
    }
    if nan {
        return f64::NAN;
    }
    vector_norm(&mut coordinates, max)
}

/// `x ** y` of floats (C's `pow`).
pub fn pow(x: f64, y: f64) -> f64 {
    x.powf(y)
}

/// `math.floor` as a float.
pub fn floor(x: f64) -> f64 {
    x.floor()
}

#[cfg(test)]
mod tests {
    use super::{hypot, sum};

    #[test]
    fn sums_and_norms_are_cpythons() {
        assert_eq!(sum(vec![0.1; 10]), 1.0);
        assert_eq!(sum(vec![1e100, 1.0, -1e100]), 1.0);
        assert_eq!(hypot(&[3.0, 4.0]), 5.0);
        assert_eq!(hypot(&[1.0, 1.0, 1.0]), 3.0_f64.sqrt());
    }
}
