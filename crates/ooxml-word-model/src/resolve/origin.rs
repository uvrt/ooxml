//! Owns: where a resolved value came from ([`Origin`]): the cascade's level that decided it
//! and, at a style level, the style of the `w:basedOn` chain that declared it, the region
//! (`w:tblStylePr`) it was declared in and the properties element that holds it -- enough
//! to find the declaring element (a theme colour's name is read there). [`Trace`] is how
//! the cascade records them: [`Origins`] keeps them, `()` keeps nothing and costs nothing,
//! so a resolution that does not ask for origins runs the same code without the record.
//!
//! Must not: read XML, or resolve (`crate::resolve::cascade`, `run` and `table` do).

use crate::styles::{Style, StyleSheet};
use crate::value::{Key, Props, Text};
use std::collections::BTreeMap;

/// The cascade's levels, in precedence order (lowest first): docx2svg's `Origin.level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    /// `w:docDefaults`.
    DocDefaults,
    /// The table style (its chain and the cell's regions).
    TableStyle,
    /// The paragraph's list level.
    Numbering,
    /// The paragraph style.
    ParagraphStyle,
    /// The run's character style.
    CharacterStyle,
    /// Direct formatting.
    Direct,
    /// A value the cascade sets itself (an autospacing's space).
    Application,
}

/// Which properties element of its holder a value was declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Holder {
    /// `w:pPr`.
    Paragraph,
    /// `w:rPr`.
    Run,
    /// `w:tblPr`.
    Table,
    /// `w:trPr`.
    Row,
    /// `w:tcPr`.
    Cell,
}

impl Holder {
    /// The element's local name.
    #[must_use]
    pub fn local(self) -> &'static str {
        match self {
            Holder::Paragraph => "pPr",
            Holder::Run => "rPr",
            Holder::Table => "tblPr",
            Holder::Row => "trPr",
            Holder::Cell => "tcPr",
        }
    }
}

/// Where one resolved value came from. Small and `Copy`: a style is its position in the
/// sheet ([`StyleSheet::at`]) and a region its position among the style's regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Origin {
    /// The level that decided the value.
    pub level: Level,
    /// The style of the chain that declared it (a style level), by position.
    pub style: Option<u32>,
    /// The region of that style it was declared in, by position among its regions.
    pub region: Option<u16>,
    /// The properties element it was declared in.
    pub holder: Holder,
    /// A cell border side read from the region's inside line (`insideH` for `top` and
    /// `bottom`, `insideV` for `left` and `right`): where the cell is inside the region.
    pub inside: bool,
}

impl Origin {
    /// A level with no style.
    #[must_use]
    pub fn at(level: Level, holder: Holder) -> Origin {
        Origin {
            level,
            style: None,
            region: None,
            holder,
            inside: false,
        }
    }

    /// `style`'s own properties at `level`.
    #[must_use]
    pub fn style(level: Level, style: &Style, holder: Holder) -> Origin {
        Origin {
            style: Some(style.position),
            ..Origin::at(level, holder)
        }
    }

    /// `style`'s region `condition` at `level` (`None` where the style has no such region).
    #[must_use]
    pub fn region(level: Level, style: &Style, condition: &str, holder: Holder) -> Origin {
        let region = style
            .conditional
            .keys()
            .position(|k| &**k == condition)
            .and_then(|p| u16::try_from(p).ok());
        Origin {
            region,
            ..Origin::style(level, style, holder)
        }
    }

    /// The declaring style, in `sheet`.
    #[must_use]
    pub fn declared_in<'s>(&self, sheet: &'s StyleSheet) -> Option<&'s Style> {
        sheet.at(self.style?).map(|s| &**s)
    }

    /// The region's `w:type`, in its style.
    #[must_use]
    pub fn region_type<'s>(&self, style: &'s Style) -> Option<&'s Text> {
        style.conditional.keys().nth(usize::from(self.region?))
    }
}

/// How a resolution records origins.
pub trait Trace: Default {
    /// Whether it keeps anything: work done only for the record is skipped when not.
    const RECORDS: bool;

    /// Every key of `layer` now comes from `origin` (made only when recorded).
    fn declare(&mut self, layer: &Props, origin: impl FnOnce() -> Origin);

    /// `key` now comes from `origin`.
    fn set(&mut self, key: &Key, origin: Origin);

    /// The keys of `layer` now come from where `level` says they came from (a level
    /// resolved on its own, then merged).
    fn absorb(&mut self, level: &Self, layer: &Props);
}

/// No record: the resolution as it runs when no one asks where values came from.
impl Trace for () {
    const RECORDS: bool = false;

    #[inline]
    fn declare(&mut self, _layer: &Props, _origin: impl FnOnce() -> Origin) {}

    #[inline]
    fn set(&mut self, _key: &Key, _origin: Origin) {}

    #[inline]
    fn absorb(&mut self, _level: &Self, _layer: &Props) {}
}

/// Each resolved key's origin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origins(BTreeMap<Key, Origin>);

impl Origins {
    /// Where `key`'s value came from (`None` for a key no level declared).
    #[must_use]
    pub fn of(&self, key: &str) -> Option<&Origin> {
        self.0.get(key)
    }

    /// Every key's origin.
    pub fn iter(&self) -> impl Iterator<Item = (&Key, &Origin)> {
        self.0.iter()
    }
}

impl Trace for Origins {
    const RECORDS: bool = true;

    fn declare(&mut self, layer: &Props, origin: impl FnOnce() -> Origin) {
        if layer.is_empty() {
            return;
        }
        let origin = origin();
        for k in layer.keys() {
            self.0.insert(k.clone(), origin);
        }
    }

    fn set(&mut self, key: &Key, origin: Origin) {
        self.0.insert(key.clone(), origin);
    }

    fn absorb(&mut self, level: &Self, layer: &Props) {
        for k in layer.keys() {
            if let Some(origin) = level.0.get(k) {
                self.0.insert(k.clone(), *origin);
            }
        }
    }
}
