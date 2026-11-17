//! Owns: [`BaseError`], the errors of units, limits, locales and ports.
//! Must not: carry another crate's errors (each crate has its own enum).

use thiserror::Error;

/// What went wrong in the base layer.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BaseError {
    /// A division that must be exact was not.
    #[error("inexact division: {numerator}/{denominator}")]
    Inexact {
        /// The dividend.
        numerator: i128,
        /// The divisor.
        denominator: i128,
    },
    /// A rational number with a zero denominator was asked for.
    #[error("zero denominator")]
    ZeroDenominator,
    /// An arithmetic result does not fit its type.
    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),
    /// A locale tag is not a well-formed BCP 47 tag.
    #[error("not a locale tag: {0:?}")]
    InvalidLocale(String),
    /// The work was cancelled through the [`crate::Cancel`] port.
    #[error("cancelled")]
    Cancelled,
    /// A port (font source, part store, ...) failed.
    #[error("{port}: {message}")]
    Port {
        /// Which port.
        port: &'static str,
        /// What it said.
        message: String,
    },
}
