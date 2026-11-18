//! Owns: [`Compact`], the one form every exact length is kept in where many are kept (a
//! measure's pieces, lines' heights, cells' stacked lines): the same value in 16 bytes
//! instead of a [`Q`]'s 32, and [`ExactLength`], what a length type gives to be kept so.
//!
//! Must not: round, use floating point, or do arithmetic: a value is taken back out with
//! [`Compact::get`] and computed with as its own type.

use crate::units::Q;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroU32;

/// An exact length a [`Compact`] can keep: a newtype over a [`Q`].
pub trait ExactLength: Copy {
    /// The exact value.
    fn exact(self) -> Q;
    /// The length of an exact value.
    fn from_exact(value: Q) -> Self;
}

/// A length `L` as it is kept, in 16 bytes rather than 32: inline where its numerator fits
/// an `i64` and its denominator a `u32` -- every length a layout makes from twips, layout
/// units and a face's units per em -- and the exact value boxed otherwise. Exact either
/// way, and one form per value (the inline one wherever it fits), so equal values compare
/// and hash equal. Arithmetic is done on `L`: [`Compact::get`] gives the value back.
pub struct Compact<L> {
    repr: Repr,
    length: PhantomData<fn() -> L>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    /// `numerator / denominator`, in lowest terms.
    Inline { numerator: i64, denominator: NonZeroU32 },
    /// A value whose parts do not fit inline.
    Boxed(Box<Q>),
}

impl<L> Compact<L> {
    /// Zero.
    pub const ZERO: Compact<L> = Compact {
        repr: Repr::Inline {
            numerator: 0,
            denominator: NonZeroU32::MIN,
        },
        length: PhantomData,
    };

    /// Whether the value is kept inline (not boxed).
    pub fn is_inline(&self) -> bool {
        matches!(self.repr, Repr::Inline { .. })
    }

    /// The exact value.
    pub fn exact(&self) -> Q {
        match &self.repr {
            Repr::Inline { numerator, denominator } => {
                Q::new_raw(i128::from(*numerator), i128::from(denominator.get()))
            }
            Repr::Boxed(value) => **value,
        }
    }

    /// Whether it is zero.
    pub fn is_zero(&self) -> bool {
        matches!(self.repr, Repr::Inline { numerator: 0, .. })
    }
}

impl<L: ExactLength> Compact<L> {
    /// The value, for arithmetic.
    pub fn get(&self) -> L {
        L::from_exact(self.exact())
    }
}

impl<L: ExactLength> From<L> for Compact<L> {
    fn from(value: L) -> Compact<L> {
        let value = value.exact();
        let repr = match (
            i64::try_from(*value.numer()),
            u32::try_from(*value.denom()).ok().and_then(NonZeroU32::new),
        ) {
            (Ok(numerator), Some(denominator)) => Repr::Inline { numerator, denominator },
            _ => Repr::Boxed(Box::new(value)),
        };
        Compact {
            repr,
            length: PhantomData,
        }
    }
}

impl<L> Clone for Compact<L> {
    fn clone(&self) -> Compact<L> {
        Compact {
            repr: self.repr.clone(),
            length: PhantomData,
        }
    }
}

impl<L> PartialEq for Compact<L> {
    fn eq(&self, other: &Compact<L>) -> bool {
        self.repr == other.repr
    }
}

impl<L> Eq for Compact<L> {}

impl<L> Hash for Compact<L> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.repr.hash(state);
    }
}

impl<L> Default for Compact<L> {
    fn default() -> Compact<L> {
        Compact::ZERO
    }
}

impl<L: ExactLength + fmt::Display> fmt::Debug for Compact<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl<L: ExactLength + fmt::Display> fmt::Display for Compact<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Length(Q);

    impl ExactLength for Length {
        fn exact(self) -> Q {
            self.0
        }
        fn from_exact(value: Q) -> Length {
            Length(value)
        }
    }

    fn length(n: i128, d: i128) -> Length {
        Length(Q::new(n, d))
    }

    #[test]
    fn exact_inline_or_boxed_and_half_the_room() {
        assert_eq!(std::mem::size_of::<Compact<Length>>(), 16);
        assert_eq!(std::mem::size_of::<Option<Compact<Length>>>(), 16);
        let inline = [
            length(0, 1),
            length(-205, 1),
            length(1_843_200, 2_048),
            length(-7, 3),
            length(i128::from(i64::MAX), 1),
            length(i128::from(i64::MIN), 1),
            length(1, i128::from(u32::MAX)),
            length(25, 24_576),
        ];
        for value in inline {
            let compact = Compact::from(value);
            assert!(compact.is_inline(), "{value:?}");
            assert_eq!(compact.get(), value);
        }
        let wide = [
            length(i128::from(i64::MAX) + 1, 1),
            length(i128::from(i64::MIN) - 1, 1),
            length(1, i128::from(u32::MAX) + 2),
            length(i128::MAX, 3),
        ];
        for value in wide {
            let compact = Compact::from(value);
            assert!(!compact.is_inline(), "{value:?}");
            assert_eq!(compact.get(), value);
            assert!(compact == Compact::from(value));
        }
        assert!(Compact::from(length(0, 1)) == Compact::ZERO);
        assert!(Compact::<Length>::ZERO.is_zero());
        assert!(Compact::from(length(1, 1)) != Compact::from(wide[0]));
        assert!(Compact::from(length(9, 3)) == Compact::from(length(3, 1)));
    }
}
