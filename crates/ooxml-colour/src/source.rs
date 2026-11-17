//! Owns: a colour choice as the XML states it ([`SourceColour`]) -- an sRGB value, a
//! colour-map slot or theme colour, or a system colour -- with its transforms ([`Transform`])
//! unapplied, in document order.
//!
//! Must not: read XML (`ooxml-drawingml` reads a colour choice into these) or resolve one
//! (`crate::context`).

/// A DrawingML colour transform's kind (ECMA-376 20.1.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransformKind {
    /// `a:lumMod`.
    LumMod,
    /// `a:lumOff`.
    LumOff,
    /// `a:tint`.
    Tint,
    /// `a:shade`.
    Shade,
    /// `a:alpha`.
    Alpha,
    /// `a:satMod`.
    SatMod,
    /// `a:satOff`.
    SatOff,
    /// `a:hueMod`.
    HueMod,
    /// `a:hueOff`.
    HueOff,
    /// `a:gray` (no value).
    Gray,
    /// `a:inv` (no value).
    Inv,
    /// `a:comp` (no value).
    Comp,
    /// `a:gamma` (no value).
    Gamma,
    /// `a:invGamma` (no value).
    InvGamma,
    /// `a:alphaMod`.
    AlphaMod,
    /// `a:alphaOff`.
    AlphaOff,
}

impl TransformKind {
    /// Every kind, in the order the names are listed above.
    pub const ALL: [TransformKind; 16] = [
        TransformKind::LumMod,
        TransformKind::LumOff,
        TransformKind::Tint,
        TransformKind::Shade,
        TransformKind::Alpha,
        TransformKind::SatMod,
        TransformKind::SatOff,
        TransformKind::HueMod,
        TransformKind::HueOff,
        TransformKind::Gray,
        TransformKind::Inv,
        TransformKind::Comp,
        TransformKind::Gamma,
        TransformKind::InvGamma,
        TransformKind::AlphaMod,
        TransformKind::AlphaOff,
    ];

    /// The element's local name (`lumMod`).
    pub fn name(self) -> &'static str {
        match self {
            TransformKind::LumMod => "lumMod",
            TransformKind::LumOff => "lumOff",
            TransformKind::Tint => "tint",
            TransformKind::Shade => "shade",
            TransformKind::Alpha => "alpha",
            TransformKind::SatMod => "satMod",
            TransformKind::SatOff => "satOff",
            TransformKind::HueMod => "hueMod",
            TransformKind::HueOff => "hueOff",
            TransformKind::Gray => "gray",
            TransformKind::Inv => "inv",
            TransformKind::Comp => "comp",
            TransformKind::Gamma => "gamma",
            TransformKind::InvGamma => "invGamma",
            TransformKind::AlphaMod => "alphaMod",
            TransformKind::AlphaOff => "alphaOff",
        }
    }

    /// The kind an element's local name names, if it is a transform.
    pub fn from_name(name: &str) -> Option<TransformKind> {
        TransformKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Whether the transform takes no value (`gray`, `inv`, `comp`, `gamma`, `invGamma`):
    /// such a transform is carried with 0.
    pub fn valueless(self) -> bool {
        matches!(
            self,
            TransformKind::Gray
                | TransformKind::Inv
                | TransformKind::Comp
                | TransformKind::Gamma
                | TransformKind::InvGamma
        )
    }

    /// Whether the transform works in HLS on the sRGB channels.
    pub fn is_hls(self) -> bool {
        matches!(
            self,
            TransformKind::LumMod
                | TransformKind::LumOff
                | TransformKind::SatMod
                | TransformKind::SatOff
                | TransformKind::HueMod
                | TransformKind::HueOff
                | TransformKind::Comp
        )
    }
}

/// One transform: its kind and its value as stated (1/1000 percent; 1/60000 degree for
/// `hueOff`; 0 for a valueless one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// The kind.
    pub kind: TransformKind,
    /// The value as stated.
    pub value: f64,
}

/// A colour choice as the XML states it.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceColour {
    /// `a:srgbClr` (and the spellings read as one: `a:prstClr`, `a:scrgbClr`, `a:hslClr`).
    Srgb {
        /// The value as stated, upper-cased (`4472C4`).
        hex: String,
        /// Its transforms, in document order.
        transforms: Vec<Transform>,
        /// For an `a:scrgbClr`, its channels as stated, linear light 0-1: resolution starts
        /// from these, where `hex` approximates them read as sRGB.
        linear: Option<[f64; 3]>,
    },
    /// `a:schemeClr`: a colour-map slot (`tx1`) or a theme colour (`accent1`, `phClr`).
    Scheme {
        /// The slot as stated.
        slot: String,
        /// Its transforms, in document order.
        transforms: Vec<Transform>,
    },
    /// `a:sysClr`: a system colour and the value it last had.
    System {
        /// The system colour's name (`windowText`).
        value: String,
        /// `lastClr`, upper-cased, where stated.
        last: Option<String>,
        /// Its transforms, in document order.
        transforms: Vec<Transform>,
    },
}

impl SourceColour {
    /// The colour's transforms, in document order.
    pub fn transforms(&self) -> &[Transform] {
        match self {
            SourceColour::Srgb { transforms, .. }
            | SourceColour::Scheme { transforms, .. }
            | SourceColour::System { transforms, .. } => transforms,
        }
    }

    /// A scheme colour of `slot` with no transforms.
    pub fn scheme(slot: &str) -> SourceColour {
        SourceColour::Scheme {
            slot: slot.to_owned(),
            transforms: Vec::new(),
        }
    }
}
