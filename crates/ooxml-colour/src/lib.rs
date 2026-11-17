//! Colour (docs/architecture/README.md, section 3 `ooxml-colour`; phase P3): one colour
//! engine for Word and PowerPoint, equal to ooxml-common's `drawingml/color.py` and
//! `themetint.py`.
//!
//! Owns:
//! - a colour choice as stated ([`SourceColour`], [`Transform`]);
//! - each application's measured rules for the transforms ([`ColourRules`]: [`POWERPOINT`],
//!   [`WORD`]) and the arithmetic ([`transform`]);
//! - colour maps, the theme's colours with the Office fallback, and resolution
//!   ([`ColourContext`]);
//! - the named and other-space colours ([`named`]) and Word's theme tint and shade
//!   ([`word_theme_tint`]).
//!
//! Must not: read XML (`ooxml-drawingml` reads colour choices, and maps and themes, into
//! these values) or draw anything. Every value is immutable and `Send + Sync`; nothing here
//! depends on an interpreter lock.
#![forbid(unsafe_code)]

mod context;
pub mod named;
mod rules;
mod source;
pub mod transform;
mod word;

pub use context::{
    apply_linear, apply_transforms, default_colour_map, effective_colour_map, fallback, ColourContext, ColourMap,
    ThemeColours, DEFAULT_COLOUR_MAP, FALLBACK_SCHEME,
};
pub use rules::{
    ColourRules, Luma, Rounding, POWERPOINT, POWERPOINT_TRANSFORMS, REC_601, REC_709, WORD, WORD_TRANSFORMS,
};
pub use source::{SourceColour, Transform, TransformKind};
pub use word::word_theme_tint;

/// A concrete colour: sRGB levels and an opacity 0-1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedColour {
    /// Red, green and blue levels.
    pub rgb: [u8; 3],
    /// The opacity, 0-1.
    pub alpha: f64,
}

impl ResolvedColour {
    /// Black, opaque.
    pub const BLACK: ResolvedColour = ResolvedColour {
        rgb: [0, 0, 0],
        alpha: 1.0,
    };

    /// `#rrggbb`, lower case.
    pub fn hex(&self) -> String {
        let [r, g, b] = self.rgb;
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

/// Why a colour could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ColourError {
    /// A stated hex value is not one (Python's reader raises on it too).
    #[error("not a hex colour: {0:?}")]
    Hex(String),
}
