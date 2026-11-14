//! Exact arithmetic.  docx2svg holds every length as an exact `Fraction`; here:
//!
//! * **widths** (1/4096 pt) are `i64` over one scale `S` per document -- the least common
//!   multiple of the exporter's denominator, 200 (a centred label's half, a `w:w` percent)
//!   and every face's `unitsPerEm / gcd(unitsPerEm, 2048)` -- so each width docx2svg makes
//!   (`advance x half points x 2048 / upm`, x `w:w`/100, + tracking, + kern) is an integer
//!   multiple of `1/S`; every construction checks it is (`exact_div`), and a value that is
//!   not panics rather than rounds;
//! * **px** (page lengths) are `i64` over `px_den`, the least common multiple of every
//!   denominator in the export and of the layout unit's (Word's 1/4096 pt in px): sums,
//!   differences and comparisons of rationals on one denominator are integer operations,
//!   so the paginator's `<=` tests are docx2svg's exactly;
//! * **line heights** are computed in `Ratio<i128>` (num-rational, always normalised) as
//!   docx2svg computes them, then put on `px_den`, checked exact.

use num_integer::Integer;
use num_rational::Ratio;
use num_traits::{ToPrimitive, Zero};

pub type Q = Ratio<i128>;

pub fn q(n: i128, d: i128) -> Q {
    Ratio::new(n, d)
}

pub fn floor(x: Q) -> i128 {
    x.numer().div_floor(x.denom())
}

/// `a / b`, which must be exact.
pub fn exact_div(a: i128, b: i128) -> i64 {
    assert!(a % b == 0, "inexact: {a}/{b}");
    (a / b) as i64
}

/// What docx2svg's px lengths are measured in.
#[derive(Clone, Copy, Debug)]
pub struct Px {
    pub den: i64,
    /// The layout unit in `1/den` px.
    pub lu: i64,
    pub dpi: i64,
}

impl Px {
    /// `vertical.twips_to_px`: `t / 20` pt in px, half up to the layout unit:
    /// `floor(204.8 t + 1/2)` layout units, whatever the dpi.
    #[inline]
    pub fn twips(&self, t: i64) -> i64 {
        (2048 * t + 5).div_euclid(10) * self.lu
    }
    pub fn px_per_pt(&self) -> Q {
        q(self.dpi as i128, 72)
    }
    pub fn lu_px(&self) -> Q {
        q(self.dpi as i128, 72 * 4096)
    }
    /// `vertical.quantise`: half up to the layout unit.
    pub fn quantise(&self, x: Q) -> Q {
        let lu = self.lu_px();
        Q::from_integer(floor(x / lu + q(1, 2))) * lu
    }
    /// A rational px length on `den`, checked exact.
    pub fn of(&self, x: Q) -> i64 {
        let v = x * Q::from_integer(self.den as i128);
        assert!(v.is_integer(), "px {x} not on 1/{}", self.den);
        v.to_integer().to_i64().unwrap()
    }
    pub fn zero() -> Q {
        Q::zero()
    }
}
