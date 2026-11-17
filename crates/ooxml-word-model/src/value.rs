//! Owns: a property as one level of the cascade declared it ([`Value`]), and a level's
//! declared properties ([`Props`]): flat keys (`ind.left`, `rFonts.ascii`, `tcBorders.top`)
//! to typed values, so that "absent" and "declared false" stay apart -- what toggle
//! resolution turns on. The vocabulary is docx2svg's (`docx2svg.parse.properties`).
//!
//! Must not: merge levels (`crate::resolve` does), or read XML (`crate::props` does).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Shared text.
pub type Text = Arc<str>;

/// One `w:rFonts` slot as one element declared it: an explicit face and a theme reference,
/// read together (the theme one wins on the same element; a level declaring either
/// replaces both as inherited).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontRef {
    /// The explicit family name.
    pub face: Option<Text>,
    /// The `ST_Theme` value (`minorHAnsi`...).
    pub theme: Option<Text>,
}

/// A tab stop (`w:tab` in `w:tabs`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabStop {
    /// Its position, twips.
    pub position: i64,
    /// `left`, `center`, `right`, `decimal`, `bar`, `num` or `clear` (`left` unstated).
    pub alignment: Text,
    /// Its leader, as written.
    pub leader: Option<Text>,
}

/// A border side: style, width in eighths of a point, distance in points, colour.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Border {
    /// `w:val`.
    pub style: Option<Text>,
    /// `w:sz`, eighths of a point (0 unstated).
    pub size: i64,
    /// `w:space`, points (0 unstated).
    pub space: i64,
    /// `w:color` as written (`None` for a run's border, which keeps three fields).
    pub color: Option<Text>,
}

/// Shading: pattern, pattern colour and fill (the theme's fill where it names one).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Shading {
    /// `w:val`: the pattern.
    pub kind: Option<Text>,
    /// `w:color`.
    pub color: Option<Text>,
    /// The fill.
    pub fill: Option<Text>,
}

/// A declared property value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Value {
    /// An on/off property.
    Bool(bool),
    /// A number (twips, half points, a count...).
    Int(i64),
    /// A string value as written (`w:jc`, `w:color`...).
    Str(Text),
    /// A font slot.
    Font(FontRef),
    /// A run's border (`w:bdr`: three fields; `color` is `None`).
    RunBorder(Border),
    /// A paragraph, table or cell border side (four fields).
    Border(Border),
    /// Shading.
    Shading(Shading),
    /// Tab stops, `clear` ones included, in document order.
    Tabs(Arc<[TabStop]>),
    /// An element's attributes by local name, as written (`w:framePr`, `w:tblpPr`).
    Attrs(Arc<[(Text, Text)]>),
    /// A table width: the value and its type (`dxa` unstated).
    Width(i64, Text),
    /// A row height (`w:trHeight`): the value and its rule (`atLeast` unstated).
    Height(i64, Text),
    /// A Word 2010 text effect (`w14:textFill`, `w14:textOutline`): DrawingML, read by the
    /// DrawingML model (phase P3); here only that it is declared.
    TextEffect(Text),
}

impl Value {
    /// The value as a boolean the way Python's `bool()` reads it (a toggle's
    /// accumulated value): `false`, `0`, an empty string or no tab stops are false.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Str(s) => !s.is_empty(),
            Value::Tabs(t) => !t.is_empty(),
            Value::Attrs(a) => !a.is_empty(),
            _ => true,
        }
    }

    /// The number, if this is one (a `Bool` counts as 0 or 1, as in Python).
    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(i64::from(*b)),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// A property's flat key (`ind.left`): borrowed for the vocabulary's own names, owned for
/// the few made from what a document states (a border side's local name).
pub type Key = Cow<'static, str>;

/// A level's declared properties by flat key.
pub type Props = BTreeMap<Key, Value>;

/// Shared text from a string (a style id, a path).
pub fn key(name: &str) -> Text {
    Text::from(name)
}

/// The property names the vocabulary has, so that a key read from an element's local name
/// borrows the vocabulary's spelling rather than allocating its own.
const NAMES: &[&str] = &[
    "b",
    "bCs",
    "i",
    "iCs",
    "caps",
    "smallCaps",
    "strike",
    "dstrike",
    "outline",
    "shadow",
    "emboss",
    "imprint",
    "vanish",
    "webHidden",
    "specVanish",
    "noProof",
    "snapToGrid",
    "rtl",
    "cs",
    "oMath",
    "sz",
    "szCs",
    "kern",
    "spacing",
    "w",
    "position",
    "color",
    "u",
    "vertAlign",
    "highlight",
    "effect",
    "em",
    "bdr",
    "shd",
    "textFill",
    "textOutline",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "widowControl",
    "contextualSpacing",
    "bidi",
    "suppressAutoHyphens",
    "mirrorIndents",
    "suppressLineNumbers",
    "adjustRightInd",
    "wordWrap",
    "kinsoku",
    "overflowPunct",
    "autoSpaceDE",
    "autoSpaceDN",
    "jc",
    "textAlignment",
    "outlineLvl",
    "tabs",
    "framePr",
    "tblW",
    "tblInd",
    "tblCellSpacing",
    "tcW",
    "wBefore",
    "wAfter",
    "vAlign",
    "textDirection",
    "hMerge",
    "gridSpan",
    "gridBefore",
    "gridAfter",
    "tblStyleRowBandSize",
    "tblStyleColBandSize",
    "cantSplit",
    "tblHeader",
    "noWrap",
    "hidden",
    "bidiVisual",
    "tcFitText",
    "hideMark",
    "tblLayout",
    "trHeight",
    "vMerge",
    "tblpPr",
];

/// The key for a property named `name`: the vocabulary's spelling where it has one.
pub fn known(name: &str) -> Key {
    NAMES
        .iter()
        .find(|n| **n == name)
        .map_or_else(|| Cow::Owned(name.to_owned()), |n| Cow::Borrowed(*n))
}

/// A key made from parts (`pBdr.top`).
pub fn joined(prefix: &str, name: &str) -> Key {
    Cow::Owned(format!("{prefix}.{name}"))
}

/// Shared text from a string.
pub fn text(s: &str) -> Text {
    Text::from(s)
}
