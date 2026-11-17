//! Owns: reading a colour choice -- `a:srgbClr`, `a:schemeClr`, `a:sysClr`, `a:prstClr`,
//! `a:scrgbClr` and `a:hslClr`, each with its transforms -- as ooxml-common's
//! `parse_color` reads it.
//!
//! Must not: apply a transform or look a slot up (`ooxml-colour`).

use super::or;
use ooxml_colour::named::{hsl_hex, preset_colour_hex, scrgb_hex};
use ooxml_colour::{SourceColour, Transform, TransformKind};
use ooxml_schema::a;
use ooxml_xml::view::{attr_num, elements};
use ooxml_xml::Element;

/// The first colour choice among `parent`'s children (`a:solidFill`, `a:buClr`...).
pub fn colour(parent: Option<&Element>) -> Option<SourceColour> {
    elements(parent?).find_map(colour_node)
}

/// One colour element read, or `None` where it is not one or states no value.
pub fn colour_node(node: &Element) -> Option<SourceColour> {
    if let Some(c) = a::SrgbColour::of(node) {
        return Some(SourceColour::Srgb {
            hex: c.val()?.to_uppercase(),
            transforms: transforms(node),
            linear: None,
        });
    }
    if let Some(c) = a::SchemeColour::of(node) {
        return Some(SourceColour::Scheme {
            slot: c.val()?.to_owned(),
            transforms: transforms(node),
        });
    }
    if let Some(c) = a::SystemColour::of(node) {
        return Some(SourceColour::System {
            value: c.val()?.to_owned(),
            last: c.last().filter(|v| !v.is_empty()).map(str::to_uppercase),
            transforms: transforms(node),
        });
    }
    if let Some(c) = a::PresetColour::of(node) {
        return Some(SourceColour::Srgb {
            hex: preset_colour_hex(c.val().unwrap_or(""))?.to_owned(),
            transforms: transforms(node),
            linear: None,
        });
    }
    if let Some(c) = a::ScrgbColour::of(node) {
        let (hex, channels) = scrgb_hex(or(c.r(), 0.0), or(c.g(), 0.0), or(c.b(), 0.0));
        return Some(SourceColour::Srgb {
            hex,
            transforms: transforms(node),
            linear: Some(channels),
        });
    }
    let c = a::HslColour::of(node)?;
    let hex = hsl_hex(
        or(c.hue(), 0.0) / 60000.0 / 360.0,
        or(c.sat(), 0.0) / 100_000.0,
        or(c.lum(), 0.0) / 100_000.0,
    );
    Some(SourceColour::Srgb {
        hex,
        transforms: transforms(node),
        linear: None,
    })
}

/// A colour element's transforms, in document order; a valueless one without a value is
/// carried with 0, any other without a value is passed over.
pub fn transforms(node: &Element) -> Vec<Transform> {
    elements(node)
        .filter_map(|item| {
            let kind = TransformKind::from_name(item.name().local())?;
            let value = attr_num(item, "val").or_else(|| kind.valueless().then_some(0.0))?;
            Some(Transform { kind, value })
        })
        .collect()
}
