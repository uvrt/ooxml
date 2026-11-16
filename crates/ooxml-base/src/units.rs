//! Owns: OOXML's length units as distinct types, their exact conversions, and the exact
//! rational arithmetic layout uses ([`Q`], [`PxScale`]).
//!
//! Must not: round silently. A conversion that is not exact returns
//! [`BaseError::Inexact`]; no floating point enters a value here.
//!
//! Refactored from the prototype's `crates/core/src/units.rs` (which stays as it is until
//! P1/P2 replace it): the same arithmetic, with checked results instead of panics.

use crate::BaseError;
use num_integer::Integer;
use num_rational::Ratio;
use num_traits::{CheckedAdd, CheckedMul, ToPrimitive};

/// An exact rational: what docx2svg's `Fraction` is.
pub type Q = Ratio<i128>;

/// `n / d`, normalised; refuses a zero denominator.
pub fn q(n: i128, d: i128) -> Result<Q, BaseError> {
    if d == 0 {
        return Err(BaseError::ZeroDenominator);
    }
    Ok(Q::new(n, d))
}

/// The greatest integer not above `x`.
pub fn floor(x: &Q) -> i128 {
    x.numer().div_floor(x.denom())
}

/// `a / b`, which must be exact and fit an `i64`.
pub fn exact_div(a: i128, b: i128) -> Result<i64, BaseError> {
    if b == 0 {
        return Err(BaseError::ZeroDenominator);
    }
    if a % b != 0 {
        return Err(BaseError::Inexact { numerator: a, denominator: b });
    }
    i64::try_from(a / b).map_err(|_| BaseError::Overflow("exact_div"))
}

/// `a + b`, or an overflow error.
pub fn checked_add(a: &Q, b: &Q) -> Result<Q, BaseError> {
    a.checked_add(b).ok_or(BaseError::Overflow("Q addition"))
}

/// `a * b`, or an overflow error.
pub fn checked_mul(a: &Q, b: &Q) -> Result<Q, BaseError> {
    a.checked_mul(b).ok_or(BaseError::Overflow("Q multiplication"))
}

macro_rules! unit {
    ($(#[$doc:meta])* $name:ident, $per_point_num:expr, $per_point_den:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(pub i64);

        impl $name {
            /// How many of this unit make `per_point_den` points' worth of `per_point_num`.
            const PER_POINT: (i128, i128) = ($per_point_num, $per_point_den);

            /// The length in points, exactly.
            pub fn points(self) -> Q {
                Q::new(i128::from(self.0) * Self::PER_POINT.1, Self::PER_POINT.0)
            }

            /// A length in points in this unit; refuses one that is not a whole number of it.
            pub fn from_points(points: &Q) -> Result<Self, BaseError> {
                let scaled = points * Q::new(Self::PER_POINT.0, Self::PER_POINT.1);
                if !scaled.is_integer() {
                    return Err(BaseError::Inexact {
                        numerator: *scaled.numer(),
                        denominator: *scaled.denom(),
                    });
                }
                scaled.to_integer().to_i64().map(Self).ok_or(BaseError::Overflow(stringify!($name)))
            }
        }
    };
}

unit!(
    /// English Metric Units: 12,700 to the point (DrawingML's lengths).
    Emu, 12_700, 1
);
unit!(
    /// Twentieths of a point (WordprocessingML's page and indent lengths).
    Twips, 20, 1
);
unit!(
    /// Half points (WordprocessingML's font sizes).
    HalfPoints, 2, 1
);
unit!(
    /// 1/4096 point: Word's layout unit, in which character advances are kept.
    Pt4096, 4096, 1
);

/// A length converted between units, which must be exact.
pub fn convert<A, B>(from: A, to_points: fn(A) -> Q, from_points: fn(&Q) -> Result<B, BaseError>) -> Result<B, BaseError> {
    from_points(&to_points(from))
}

/// How docx2svg's px lengths are kept: integers over one denominator `den` per document, so
/// sums and comparisons are integer operations and pagination decisions are exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxScale {
    /// The common denominator of every px length.
    pub den: i64,
    /// The layout unit (1/4096 pt) in `1/den` px.
    pub lu: i64,
    /// Pixels per inch.
    pub dpi: i64,
}

impl PxScale {
    /// `vertical.twips_to_px`: `t / 20` pt in px, half up to the layout unit:
    /// `floor(204.8 t + 1/2)` layout units, whatever the dpi.
    pub fn twips(&self, t: Twips) -> Result<i64, BaseError> {
        let units = (2048_i64.checked_mul(t.0).and_then(|v| v.checked_add(5)))
            .ok_or(BaseError::Overflow("PxScale::twips"))?
            .div_euclid(10);
        units.checked_mul(self.lu).ok_or(BaseError::Overflow("PxScale::twips"))
    }

    /// Pixels per point.
    pub fn px_per_pt(&self) -> Q {
        Q::new(i128::from(self.dpi), 72)
    }

    /// One layout unit in px.
    pub fn lu_px(&self) -> Q {
        Q::new(i128::from(self.dpi), 72 * 4096)
    }

    /// `vertical.quantise`: half up to the layout unit.
    pub fn quantise(&self, x: &Q) -> Result<Q, BaseError> {
        let lu = self.lu_px();
        if lu.numer() == &0 {
            return Err(BaseError::ZeroDenominator);
        }
        let units = floor(&checked_add(&(x / lu), &Q::new(1, 2))?);
        checked_mul(&Q::from_integer(units), &lu)
    }

    /// A rational px length on `den`, which must be exact.
    pub fn of(&self, x: &Q) -> Result<i64, BaseError> {
        let v = checked_mul(x, &Q::from_integer(i128::from(self.den)))?;
        if !v.is_integer() {
            return Err(BaseError::Inexact { numerator: *v.numer(), denominator: *v.denom() });
        }
        v.to_integer().to_i64().ok_or(BaseError::Overflow("PxScale::of"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_convert_exactly() {
        assert_eq!(Twips(20).points(), Q::from_integer(1));
        assert_eq!(convert(Twips(1), Twips::points, Emu::from_points), Ok(Emu(635)));
        assert_eq!(convert(HalfPoints(24), HalfPoints::points, Pt4096::from_points), Ok(Pt4096(49_152)));
        assert!(matches!(convert(Emu(1), Emu::points, Twips::from_points), Err(BaseError::Inexact { .. })));
    }

    #[test]
    fn exact_division_refuses_remainders() {
        assert_eq!(exact_div(10, 5), Ok(2));
        assert_eq!(exact_div(10, 3), Err(BaseError::Inexact { numerator: 10, denominator: 3 }));
        assert_eq!(exact_div(1, 0), Err(BaseError::ZeroDenominator));
        assert_eq!(q(1, 0), Err(BaseError::ZeroDenominator));
    }

    #[test]
    fn px_scale_matches_the_prototype() {
        let s = PxScale { den: 4096 * 72, lu: 96, dpi: 96 };
        // floor(204.8 * 3 + 1/2) = 614 layout units
        assert_eq!(s.twips(Twips(3)), Ok(614 * 96));
        // one layout unit is 1/3072 px at 96 dpi: 1/1000 px is 3.072 units, half up 3
        assert_eq!(s.quantise(&q(1, 1000).unwrap()), Ok(q(3, 3072).unwrap()));
        assert_eq!(s.quantise(&q(1, 3).unwrap()), Ok(q(1, 3).unwrap()));
        assert!(s.of(&Q::new(1, 7)).is_err());
        assert_eq!(s.of(&Q::from_integer(2)), Ok(2 * 4096 * 72));
    }
}
