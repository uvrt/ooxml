//! Owns: [`Units`], a horizontal length in Word's layout unit, 1/4096 pt, held exactly (an
//! integer wherever it is one, as docx2svg's `int | Fraction`), and the conversions into
//! it Word rounds (twips half up to the unit).
//!
//! Must not: round anything else, or use floating point.

use num_integer::Integer;
use num_rational::Ratio;
use num_traits::{Signed, Zero};
use std::fmt;
use std::ops::{Add, AddAssign, Mul, Neg, Sub};

type Q = Ratio<i128>;

/// A length in 1/4096 pt, exact.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Units(Q);

impl Units {
    /// Zero.
    pub const ZERO: Units = Units(Q::new_raw(0, 1));

    /// A whole number of units.
    pub fn int(value: i64) -> Units {
        Units(Q::from_integer(i128::from(value)))
    }

    /// `numerator / denominator` units; `None` for a zero denominator.
    pub fn ratio(numerator: i128, denominator: i128) -> Option<Units> {
        (denominator != 0).then(|| Units(Q::new(numerator, denominator)))
    }

    /// The numerator and denominator, in lowest terms (denominator positive).
    pub fn parts(self) -> (i128, i128) {
        (*self.0.numer(), *self.0.denom())
    }

    /// Whether the length is a whole number of units.
    pub fn is_integer(self) -> bool {
        self.0.is_integer()
    }

    /// The greatest whole number of units not above it.
    pub fn floor(self) -> i128 {
        self.0.numer().div_floor(self.0.denom())
    }

    /// `self / divisor`, exactly; `None` for a zero divisor.
    pub fn checked_div(self, divisor: Units) -> Option<Units> {
        (!divisor.0.is_zero()).then(|| Units(self.0 / divisor.0))
    }

    /// `self * numerator / denominator`, exactly; `None` for a zero denominator.
    pub fn scale(self, numerator: i128, denominator: i128) -> Option<Units> {
        (denominator != 0).then(|| Units(self.0 * Q::new(numerator, denominator)))
    }

    /// Whether the length is below zero.
    pub fn is_negative(self) -> bool {
        self.0.is_negative()
    }
}

impl Add for Units {
    type Output = Units;
    fn add(self, other: Units) -> Units {
        if self.0.is_integer() && other.0.is_integer() {
            Units(Q::from_integer(self.0.numer() + other.0.numer()))
        } else {
            Units(self.0 + other.0)
        }
    }
}

impl AddAssign for Units {
    fn add_assign(&mut self, other: Units) {
        *self = *self + other;
    }
}

impl Sub for Units {
    type Output = Units;
    fn sub(self, other: Units) -> Units {
        self + -other
    }
}

impl Neg for Units {
    type Output = Units;
    fn neg(self) -> Units {
        Units(-self.0)
    }
}

impl Mul<i64> for Units {
    type Output = Units;
    fn mul(self, k: i64) -> Units {
        Units(self.0 * Q::from_integer(i128::from(k)))
    }
}

impl fmt::Debug for Units {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for Units {
    /// `n` or `n/d`, as docx2svg's differential writes a `Fraction`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_integer() {
            write!(f, "{}", self.0.numer())
        } else {
            write!(f, "{}/{}", self.0.numer(), self.0.denom())
        }
    }
}

impl std::str::FromStr for Units {
    type Err = String;
    /// `n` or `n/d`.
    fn from_str(text: &str) -> Result<Units, String> {
        let bad = || format!("not a length: {text:?}");
        let (n, d) = text.split_once('/').unwrap_or((text, "1"));
        let n: i128 = n.parse().map_err(|_| bad())?;
        let d: i128 = d.parse().map_err(|_| bad())?;
        Units::ratio(n, d).ok_or_else(bad)
    }
}

/// A twip length as Word holds it: in 1/4096 pt, half up (`linebreak.twips_to_units`).
pub fn twips_to_units(twips: Units) -> Units {
    // twips x 4096/20, plus 1/2, floored.
    let (n, d) = twips.parts();
    let value = Q::new(n * 4096, d * 20) + Q::new(1, 2);
    Units(Q::from_integer(value.numer().div_floor(value.denom())))
}

/// [`twips_to_units`] of a whole number of twips.
pub fn twips(value: i64) -> Units {
    twips_to_units(Units::int(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_rounded() {
        assert_eq!(twips(708), Units::int(144_998));
        assert_eq!(twips(1), Units::int(205));
        assert_eq!(twips(-1), Units::int(-205));
        let third = Units::ratio(1, 3).unwrap();
        assert_eq!((third + third + third).to_string(), "1");
        assert_eq!("7/2".parse::<Units>().unwrap().floor(), 3);
        assert_eq!("-7/2".parse::<Units>().unwrap().floor(), -4);
        assert!(Units::int(2) > third);
    }
}
