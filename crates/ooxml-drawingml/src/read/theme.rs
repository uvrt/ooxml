//! Owns: reading a theme's colour scheme and format scheme, and colour maps (`p:clrMap`,
//! `p:clrMapOvr`, a chart's `c:clrMapOvr`), as ooxml-common (`parse_color_scheme`,
//! `parse_format_scheme`) and pptx2svg (`parse_color_map`) read them.
//!
//! Must not: resolve the scheme (`crate::resolve`, `ooxml-colour`) or read the font scheme
//! (the Word model reads it for layout; a slide's reader will).

use super::colour::colour;
use super::fill::{effects, fill_element, line};
use crate::source::FormatScheme;
use ooxml_colour::{ColourMap, SourceColour, ThemeColours};
use ooxml_schema::a;
use ooxml_xml::view::elements;
use ooxml_xml::Element;

/// The colour scheme's slots, in schema order.
pub const SCHEME_SLOTS: [&str; 12] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink",
];

/// The colour-map slots.
pub const MAP_SLOTS: [&str; 12] = [
    "bg1", "tx1", "bg2", "tx2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink",
];

/// A theme as stated: its colour scheme (slot -> colour choice, in schema order, the slots
/// with a readable colour only) and its format scheme.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Theme {
    /// The colour scheme.
    pub colours: Vec<(String, SourceColour)>,
    /// The format scheme.
    pub formats: FormatScheme,
}

impl Theme {
    /// The colour scheme as the colour engine takes it.
    pub fn colour_map(&self) -> ThemeColours {
        self.colours.iter().cloned().collect()
    }
}

/// A theme part's root (`a:theme`) read.
pub fn theme(root: &Element) -> Theme {
    let elements = a::Theme::new(root).elements();
    Theme {
        colours: colour_scheme(elements.and_then(a::ThemeElements::colours)),
        formats: format_scheme(elements.and_then(a::ThemeElements::formats)),
    }
}

/// `a:clrScheme` as slot -> colour choice.
pub fn colour_scheme(scheme: Option<a::ColourScheme<'_>>) -> Vec<(String, SourceColour)> {
    let Some(scheme) = scheme else {
        return Vec::new();
    };
    SCHEME_SLOTS
        .iter()
        .filter_map(|slot| {
            let holder = ooxml_xml::view::child(scheme.element(), slot);
            Some(((*slot).to_owned(), colour(holder)?))
        })
        .collect()
}

fn bare_fills(list: Option<a::Any<'_>>) -> Vec<crate::source::Fill> {
    list.map(|l| elements(l.element()).filter_map(fill_element).collect())
        .unwrap_or_default()
}

/// `a:fmtScheme`: what a style reference's index selects (empty where absent).
pub fn format_scheme(scheme: Option<a::FormatScheme<'_>>) -> FormatScheme {
    let Some(scheme) = scheme else {
        return FormatScheme::default();
    };
    FormatScheme {
        fill_styles: bare_fills(scheme.fill_styles()),
        line_styles: scheme
            .line_styles()
            .into_iter()
            .flat_map(a::LineStyleList::lines)
            .filter_map(|ln| line(Some(ln)))
            .collect(),
        effect_styles: scheme
            .effect_styles()
            .into_iter()
            .flat_map(a::EffectStyleList::styles)
            .map(|style| effects(style.effects()))
            .collect(),
        background_fill_styles: bare_fills(scheme.background_fill_styles()),
    }
}

/// A colour map (`p:clrMap`, `a:overrideClrMapping`): only the slots it states; `None`
/// where it states none.
pub fn colour_map(map: Option<&Element>) -> Option<ColourMap> {
    let map = map?;
    let mapping: ColourMap = MAP_SLOTS
        .iter()
        .filter_map(|slot| Some(((*slot).to_owned(), ooxml_xml::view::attr(map, slot)?.to_owned())))
        .collect();
    (!mapping.is_empty()).then_some(mapping)
}

/// `p:clrMapOvr`: `None` for `a:masterClrMapping` ("the master's map") or no override.
pub fn colour_map_override(override_: Option<&Element>) -> Option<ColourMap> {
    let mapping = a::ColourMapOverride::new(override_?).mapping()?;
    colour_map(Some(mapping.element()))
}
