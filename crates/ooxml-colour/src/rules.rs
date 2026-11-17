//! Owns: how one application applies a colour's transforms and turns the result into levels
//! ([`ColourRules`]): PowerPoint's ([`POWERPOINT`], the default) and Word's ([`WORD`]), each
//! measured against its own application (ooxml-common's `drawingml/color.py` holds the
//! measurements: pptx2svg's 3,108-swatch colour probe and docx2svg's 54-swatch DrawingML
//! probe).
//!
//! Must not: hold the arithmetic (`crate::transform`) or anything an application was not
//! measured to do.

use crate::source::TransformKind;

/// How a channel computed on 0-255 becomes the level drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// The nearest level, a tie to the even one (PowerPoint's).
    HalfEven,
    /// The nearest level, a tie to the lower one, with a 1e-9 tolerance either side of an
    /// exact half (Word's, measured: 127.5 is 127).
    HalfDown,
}

impl Rounding {
    /// `value` (a channel on 0-255) as a level, before clamping.
    pub fn level(self, value: f64) -> f64 {
        match self {
            Rounding::HalfEven => value.round_ties_even(),
            Rounding::HalfDown => (value - 0.5 - 1e-9).ceil(),
        }
    }
}

/// Luma weights of the sRGB channels for `gray`.
pub type Luma = [f64; 3];

/// Rec. 601's weights.
pub const REC_601: Luma = [0.299, 0.587, 0.114];
/// Rec. 709's weights.
pub const REC_709: Luma = [0.2126, 0.7152, 0.0722];

/// How one application applies a colour's transforms (in document order, on channels
/// clamped to 0-1 between steps) and rounds the result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourRules {
    /// The application's name (`powerpoint`, `word`).
    pub name: &'static str,
    /// How a channel becomes a level.
    pub rounding: Rounding,
    /// The quantum the channels are kept in between steps, as linear-light fractions
    /// (100000: an `a:scrgbClr`'s percentages), or `None` to keep them unrounded.
    pub precision: Option<u32>,
    /// The transform kinds applied; any other is passed over.
    pub transforms: &'static [TransformKind],
    /// The luma weights for `gray`.
    pub luma: Luma,
    /// Whether `satMod` and `satOff` may take the saturation below 0.
    pub negative_saturation: bool,
    /// Whether a grey given a saturation is drawn as PowerPoint draws it (red at hue 0, its
    /// blue channel extrapolated).
    pub achromatic_extrapolates: bool,
}

impl ColourRules {
    /// Whether these rules apply transforms of `kind`.
    pub fn applies(&self, kind: TransformKind) -> bool {
        self.transforms.contains(&kind)
    }
}

/// The transforms Word's probe measured, and so the ones [`WORD`] applies.
pub const WORD_TRANSFORMS: &[TransformKind] = &[
    TransformKind::LumMod,
    TransformKind::LumOff,
    TransformKind::SatMod,
    TransformKind::SatOff,
    TransformKind::HueMod,
    TransformKind::HueOff,
    TransformKind::Comp,
    TransformKind::Tint,
    TransformKind::Shade,
    TransformKind::Inv,
    TransformKind::Gray,
    TransformKind::Alpha,
];

/// PowerPoint's, measured: Word's and `gamma`, `invGamma`, `alphaMod`, `alphaOff`.
pub const POWERPOINT_TRANSFORMS: &[TransformKind] = &[
    TransformKind::LumMod,
    TransformKind::LumOff,
    TransformKind::SatMod,
    TransformKind::SatOff,
    TransformKind::HueMod,
    TransformKind::HueOff,
    TransformKind::Comp,
    TransformKind::Tint,
    TransformKind::Shade,
    TransformKind::Inv,
    TransformKind::Gray,
    TransformKind::Alpha,
    TransformKind::Gamma,
    TransformKind::InvGamma,
    TransformKind::AlphaMod,
    TransformKind::AlphaOff,
];

/// PowerPoint, as pptx2svg's colour probe measured it. The default.
pub const POWERPOINT: ColourRules = ColourRules {
    name: "powerpoint",
    rounding: Rounding::HalfEven,
    precision: Some(100_000),
    transforms: POWERPOINT_TRANSFORMS,
    luma: REC_709,
    negative_saturation: true,
    achromatic_extrapolates: true,
};

/// Word, as docx2svg measured it: every transform in document order on unrounded channels,
/// the result rounded a half down.
pub const WORD: ColourRules = ColourRules {
    name: "word",
    rounding: Rounding::HalfDown,
    precision: None,
    transforms: WORD_TRANSFORMS,
    luma: REC_601,
    negative_saturation: false,
    achromatic_extrapolates: false,
};

impl Default for ColourRules {
    fn default() -> ColourRules {
        POWERPOINT
    }
}
