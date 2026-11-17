//! Owns: what turns a colour choice into a colour -- the colour map (a slot such as `tx1`
//! names a theme colour such as `dk1`), layered from the schema's default through a master,
//! a layout and a slide ([`effective_colour_map`]); the theme's colour scheme with the
//! Office fallback for what it lacks; and resolution under an application's rules
//! ([`ColourContext::resolve`]), equal to ooxml-common's `resolve_color`.
//!
//! Must not: read XML, or know where a colour map or a theme came from.

use crate::rules::{ColourRules, POWERPOINT};
use crate::source::{SourceColour, Transform};
use crate::transform::{apply_sequential, hex_channels, linear_to_srgb};
use crate::{ColourError, ResolvedColour};
use std::collections::BTreeMap;

/// A colour map: slot (`bg1`, `tx1`, `accent1`...) -> theme colour (`lt1`, `dk1`...).
pub type ColourMap = BTreeMap<String, String>;

/// The schema's default colour map (`p:clrMap` when a master declares none), slot by slot.
pub const DEFAULT_COLOUR_MAP: [(&str, &str); 12] = [
    ("bg1", "lt1"),
    ("tx1", "dk1"),
    ("bg2", "lt2"),
    ("tx2", "dk2"),
    ("accent1", "accent1"),
    ("accent2", "accent2"),
    ("accent3", "accent3"),
    ("accent4", "accent4"),
    ("accent5", "accent5"),
    ("accent6", "accent6"),
    ("hlink", "hlink"),
    ("folHlink", "folHlink"),
];

/// The Office theme's colours used where a theme is missing or lacks a slot (as
/// ooxml-common's `FALLBACK_SCHEME_COLORS` has them), `#rrggbb`.
pub const FALLBACK_SCHEME: [(&str, &str); 12] = [
    ("dk1", "#000000"),
    ("lt1", "#ffffff"),
    ("dk2", "#44546a"),
    ("lt2", "#e7e6e6"),
    ("accent1", "#4472c4"),
    ("accent2", "#ed7d31"),
    ("accent3", "#a5a5a5"),
    ("accent4", "#ffc000"),
    ("accent5", "#5b9bd5"),
    ("accent6", "#70ad47"),
    ("hlink", "#0563c1"),
    ("folHlink", "#954f72"),
];

/// The fallback colour of theme slot `slot`.
pub fn fallback(slot: &str) -> Option<&'static str> {
    FALLBACK_SCHEME.iter().find(|(k, _)| *k == slot).map(|(_, v)| *v)
}

/// The default colour map.
pub fn default_colour_map() -> ColourMap {
    DEFAULT_COLOUR_MAP
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// The colour maps layered: the schema default < a master's < a layout's override < a
/// slide's override (each as stated: only its own slots).
pub fn effective_colour_map(layers: &[Option<&ColourMap>]) -> ColourMap {
    let mut map = default_colour_map();
    for layer in layers.iter().flatten() {
        map.extend(layer.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    map
}

/// A theme's colour scheme as stated: theme colour (`dk1`...) -> its colour choice.
pub type ThemeColours = BTreeMap<String, SourceColour>;

/// Everything that turns a [`SourceColour`] into a colour: the theme's colour scheme (if
/// any), the colour map and the application's rules.
#[derive(Debug, Clone)]
pub struct ColourContext {
    theme: Option<ThemeColours>,
    colour_map: ColourMap,
    rules: ColourRules,
    scheme: BTreeMap<String, String>,
}

impl ColourContext {
    /// A context for `theme` under `colour_map` and `rules`; the flattened scheme is built
    /// once, as ooxml-common's `build_color_scheme` builds it.
    pub fn new(theme: Option<ThemeColours>, colour_map: ColourMap, rules: ColourRules) -> ColourContext {
        let mut context = ColourContext {
            theme,
            colour_map,
            rules,
            scheme: FALLBACK_SCHEME
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        };
        let entries: Vec<(String, SourceColour)> = context
            .theme
            .iter()
            .flat_map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect();
        for (name, colour) in entries {
            if let Ok(Some(resolved)) = context.resolve(&colour) {
                context.scheme.insert(name, resolved.hex());
            }
        }
        context
    }

    /// PowerPoint's rules, the default colour map and no theme.
    pub fn unthemed() -> ColourContext {
        ColourContext::new(None, default_colour_map(), POWERPOINT)
    }

    /// The rules.
    pub fn rules(&self) -> &ColourRules {
        &self.rules
    }

    /// The colour map.
    pub fn colour_map(&self) -> &ColourMap {
        &self.colour_map
    }

    /// The theme's colours flattened, `#rrggbb` by theme colour, the fallback where the theme
    /// is silent.
    pub fn scheme(&self) -> &BTreeMap<String, String> {
        &self.scheme
    }

    /// The colour `colour` comes to, or `None` where it names nothing (a slot neither the
    /// theme nor the fallback has, `phClr` included). An error where a stated hex value is
    /// not one.
    pub fn resolve(&self, colour: &SourceColour) -> Result<Option<ResolvedColour>, ColourError> {
        self.resolve_visiting(colour, &mut Vec::new())
    }

    fn resolve_visiting(
        &self,
        colour: &SourceColour,
        visited: &mut Vec<String>,
    ) -> Result<Option<ResolvedColour>, ColourError> {
        if let SourceColour::Srgb {
            linear: Some(linear),
            transforms,
            ..
        } = colour
        {
            return Ok(Some(apply_linear(*linear, transforms, &self.rules)));
        }
        let Some(base) = self.base_hex(colour, visited)? else {
            return Ok(None);
        };
        apply_transforms(&base, colour.transforms(), &self.rules).map(Some)
    }

    fn base_hex(&self, colour: &SourceColour, visited: &mut Vec<String>) -> Result<Option<String>, ColourError> {
        let slot = match colour {
            SourceColour::Srgb { hex, .. } => return Ok(Some(hex.clone())),
            SourceColour::System { last, .. } => return Ok(Some(last.clone().unwrap_or_else(|| "000000".into()))),
            SourceColour::Scheme { slot, .. } => slot,
        };
        let mapped = self.colour_map.get(slot).unwrap_or(slot);
        if visited.contains(mapped) {
            // A colour-map cycle (tx1 -> dk1 -> tx1): stop rather than recurse.
            return Ok(Some("#000000".into()));
        }
        if let Some(entry) = self.theme.as_ref().and_then(|t| t.get(mapped)) {
            visited.push(mapped.clone());
            let resolved = self.resolve_visiting(entry, visited);
            visited.pop();
            return Ok(resolved?.map(|c| c.hex()));
        }
        Ok(self
            .scheme
            .get(mapped)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
            .or_else(|| fallback(mapped))
            .map(str::to_owned))
    }
}

/// A concrete colour (`RRGGBB` or `#rrggbb`, any case) after its transforms: for a caller
/// that looks the base colour up itself (a style reference's `phClr`, Word's theme slot).
pub fn apply_transforms(
    value: &str,
    transforms: &[Transform],
    rules: &ColourRules,
) -> Result<ResolvedColour, ColourError> {
    Ok(apply_sequential(hex_channels(value)?, transforms, rules))
}

/// As [`apply_transforms`], from channels in linear light (an `a:scrgbClr`'s).
pub fn apply_linear(linear: [f64; 3], transforms: &[Transform], rules: &ColourRules) -> ResolvedColour {
    apply_sequential(linear.map(|v| linear_to_srgb(v) / 255.0), transforms, rules)
}
