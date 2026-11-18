//! Owns: [`Px`], a vertical or page length in device pixels at docx2svg's 300 dpi, held
//! exactly (docx2svg's `Fraction` px), and Word's roundings into it: an authored twip
//! length half up to the layout unit ([`Px::twips`]), a computed length half up or down
//! to it ([`Px::quantise`], [`Px::quantise_down`]), and whole-pixel rounding
//! ([`Px::round_half_up`]) -- docx2svg's `vertical.twips_to_px`, `quantise`,
//! `quantise_down` and `round_half_up` -- and [`CompactPx`], a `Px` as the measures keep it.
//!
//! Must not: use floating point, or round anywhere but in the functions named above.

use num_integer::Integer;
use ooxml_base::{Compact, ExactLength, Q};
use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Device pixels per inch (Word's export: whole 1/300-inch pixels).
pub const DPI: i128 = 300;
/// Points per inch.
const POINTS_PER_INCH: i128 = 72;
/// Word's layout unit: 1/4096 pt.
const UNITS_PER_POINT: i128 = 4096;

/// A length in device px, exact.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Px(Q);

/// A [`Px`] as the measures keep it (lines' heights, cells' stacked lines): ooxml-base's
/// [`Compact`], 16 bytes rather than 32, exact. Arithmetic is done on [`Px`]:
/// [`Compact::get`] gives the value back.
pub type CompactPx = Compact<Px>;

impl ExactLength for Px {
    fn exact(self) -> Q {
        self.0
    }
    fn from_exact(value: Q) -> Px {
        Px(value)
    }
}

impl fmt::Debug for Px {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Px {
    /// `n` or `n/d`, as docx2svg's differential prints a `Fraction`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_integer() {
            write!(f, "{}", self.0.numer())
        } else {
            write!(f, "{}/{}", self.0.numer(), self.0.denom())
        }
    }
}

impl Px {
    /// Zero.
    pub const ZERO: Px = Px(Q::new_raw(0, 1));

    /// A whole number of px.
    pub fn int(value: i64) -> Px {
        Px(Q::from_integer(i128::from(value)))
    }

    /// `numerator / denominator` px; `None` for a zero denominator.
    pub fn ratio(numerator: i128, denominator: i128) -> Option<Px> {
        (denominator != 0).then(|| Px(Q::new(numerator, denominator)))
    }

    /// The exact rational.
    pub fn value(self) -> Q {
        self.0
    }

    /// Px per point: 300/72.
    pub fn per_point() -> Q {
        Q::new(DPI, POINTS_PER_INCH)
    }

    /// One layout unit (1/4096 pt) in px.
    pub fn layout_unit() -> Q {
        Q::new(DPI, POINTS_PER_INCH * UNITS_PER_POINT)
    }

    /// `x` px, rounded half up to the layout unit (`vertical.quantise`).
    pub fn quantise_q(x: Q) -> Px {
        let lu = Px::layout_unit();
        let units = (x / lu + Q::new(1, 2)).floor();
        Px(units * lu)
    }

    /// Rounded half up to the layout unit.
    pub fn quantise(self) -> Px {
        Px::quantise_q(self.0)
    }

    /// Truncated to the layout unit (`vertical.quantise_down`).
    pub fn quantise_down(self) -> Px {
        let lu = Px::layout_unit();
        Px((self.0 / lu).floor() * lu)
    }

    /// An authored twip length as Word holds it: `t / 20` pt in px, half up to the layout
    /// unit -- `floor(204.8 t + 1/2)` units (`vertical.twips_to_px`).
    pub fn twips(t: i64) -> Px {
        let units = Integer::div_floor(&(2048 * i128::from(t) + 5), &10);
        Px(Q::from_integer(units) * Px::layout_unit())
    }

    /// A length in points, exact (no rounding).
    pub fn points(points: Q) -> Px {
        Px(points * Px::per_point())
    }

    /// The nearest whole pixel, halves up (`vertical.round_half_up`).
    pub fn round_half_up(self) -> i64 {
        let v = (self.0 + Q::new(1, 2)).floor().to_integer();
        i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
    }

    /// The greatest whole pixel not above it.
    pub fn floor(self) -> i64 {
        let v = self.0.floor().to_integer();
        i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
    }

    /// Its fractional part, in `[0, 1)`.
    pub fn fract(self) -> Px {
        Px(self.0 - self.0.floor())
    }

    /// Scaled by `numerator / denominator`; `None` for a zero denominator.
    pub fn scale(self, numerator: i128, denominator: i128) -> Option<Px> {
        (denominator != 0).then(|| Px(self.0 * Q::new(numerator, denominator)))
    }

    /// Whether it is zero.
    pub fn is_zero(self) -> bool {
        *self.0.numer() == 0
    }
}

impl From<Q> for Px {
    fn from(value: Q) -> Px {
        Px(value)
    }
}

impl Add for Px {
    type Output = Px;
    fn add(self, other: Px) -> Px {
        Px(self.0 + other.0)
    }
}

impl AddAssign for Px {
    fn add_assign(&mut self, other: Px) {
        self.0 += other.0;
    }
}

impl Sub for Px {
    type Output = Px;
    fn sub(self, other: Px) -> Px {
        Px(self.0 - other.0)
    }
}

impl SubAssign for Px {
    fn sub_assign(&mut self, other: Px) {
        self.0 -= other.0;
    }
}

impl Neg for Px {
    type Output = Px;
    fn neg(self) -> Px {
        Px(-self.0)
    }
}

impl Mul<Q> for Px {
    type Output = Px;
    fn mul(self, other: Q) -> Px {
        Px(self.0 * other)
    }
}

impl Div<Q> for Px {
    type Output = Px;
    fn div(self, other: Q) -> Px {
        Px(self.0 / other)
    }
}

impl std::iter::Sum for Px {
    fn sum<I: Iterator<Item = Px>>(iter: I) -> Px {
        iter.fold(Px::ZERO, |a, b| a + b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twips_round_half_up_to_the_unit() {
        // 240 twips = 12 pt = 50 px exactly
        assert_eq!(Px::twips(240), Px::int(50));
        // 3 twips: floor(614.4 + 0.5) = 614 units
        assert_eq!(Px::twips(3), Px::layout_unit().mul(Q::from_integer(614)).into());
        assert_eq!(Px::twips(-3), Px::layout_unit().mul(Q::from_integer(-614)).into());
    }

    #[test]
    fn rounding() {
        assert_eq!(Px::ratio(5, 2).map(Px::round_half_up), Some(3));
        assert_eq!(Px::ratio(-5, 2).map(Px::round_half_up), Some(-2));
        assert_eq!(Px::ratio(7, 3).map(|p| p.fract()), Px::ratio(1, 3));
        assert_eq!(Px::ratio(1, 1_000_000).map(Px::quantise_down), Some(Px::ZERO));
        assert_eq!(Px::int(3).to_string(), "3");
        assert_eq!(Px::ratio(3, 6).map(|p| p.to_string()).as_deref(), Some("1/2"));
    }
}
