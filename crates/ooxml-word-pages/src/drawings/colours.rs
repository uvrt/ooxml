//! Owns: a Word drawing's colours, fills, outlines and effects resolved -- docx2svg's
//! `drawing.Colours`: a theme colour through `w:clrSchemeMapping` (Word's spellings of the
//! colour map's slots), `phClr` as a style reference's colour, a shape's theme style
//! references (`wps:style`: 1-based indexes, from 1001 the background fills), a group's
//! fill for a member that says `a:grpFill`, under Word's colour rules -- and what cannot be
//! drawn said as a warning code (`drawing-colour-unknown:*`, `drawing-not-drawn:*`,
//! `drawing-outline-approximate:*`).
//!
//! Must not: draw, or read the document's XML.

use ooxml_colour::{apply_linear, apply_transforms, ResolvedColour, SourceColour, WORD};
use ooxml_drawingml::drawable::{
    ArrowEnd, Colour, Effects, Fill, Glow, GradientFill, GradientStop, InnerShadow, OuterShadow, Outline,
};
use ooxml_drawingml::source::{self, FormatScheme, GradientKind, LineEnd, StyleRef};
use std::collections::BTreeMap;

/// `a:ln` without `@w`: Word's default outline width, 0.75 pt.
pub const DEFAULT_LINE_EMU: f64 = 9525.0;

/// `w:clrSchemeMapping`'s attributes as the colour map's slots (`_MAPPED_KEYS`).
fn mapped_key(key: &str) -> &str {
    match key {
        "t1" => "tx1",
        "t2" => "tx2",
        "followedHyperlink" => "folHlink",
        "hyperlink" => "hlink",
        other => other,
    }
}

/// `w:clrSchemeMapping`'s values as theme slots (`_MAPPED`).
fn mapped_value(value: &str) -> &str {
    match value {
        "light1" => "lt1",
        "dark1" => "dk1",
        "light2" => "lt2",
        "dark2" => "dk2",
        "followedHyperlink" => "folHlink",
        other => other,
    }
}

/// `w:clrSchemeMapping` as `{colour-map slot: theme slot}` over Word's default map
/// (`color_map_slots`).
pub fn colour_map_slots<'a>(map: impl IntoIterator<Item = (&'a str, &'a str)>) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = [("bg1", "lt1"), ("tx1", "dk1"), ("bg2", "lt2"), ("tx2", "dk2")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    for (key, value) in map {
        out.insert(mapped_key(key).to_owned(), mapped_value(value).to_owned());
    }
    out
}

/// A colour as this package has always written one: `#RRGGBB` (`_upper`).
fn upper(colour: &ResolvedColour) -> Colour {
    Colour {
        hex: format!("#{}", colour.hex().trim_start_matches('#').to_ascii_uppercase()),
        alpha: colour.alpha,
    }
}

/// One document's colour resolution (`Colours`).
#[derive(Debug, Clone)]
pub struct WordColours<'a> {
    /// The theme's colours by slot.
    pub theme: &'a BTreeMap<ooxml_word_model::value::Text, ooxml_word_model::value::Text>,
    /// The colour map's slots to theme slots.
    pub slots: BTreeMap<String, String>,
    /// The theme's format scheme.
    pub formats: Option<&'a FormatScheme>,
}

/// What a resolution could not draw, said as warning codes in the order met.
pub type Said = Vec<String>;

impl WordColours<'_> {
    /// A colour choice resolved, `None` where it names nothing (warned); `ph_colour` is
    /// what `phClr` means.
    pub fn resolve(&self, choice: &SourceColour, ph_colour: Option<&Colour>, said: &mut Said) -> Option<Colour> {
        let base = match choice {
            SourceColour::Srgb {
                linear: Some(linear),
                transforms,
                ..
            } => return Some(upper(&apply_linear(*linear, transforms, &WORD))),
            SourceColour::Srgb { hex, .. } => Some(hex.clone()),
            SourceColour::System { value, last, .. } => last
                .clone()
                .or_else(|| (value.chars().count() == 6).then(|| value.clone())),
            SourceColour::Scheme { slot, transforms } if slot == "phClr" => {
                let Some(reference) = ph_colour else {
                    said.push("drawing-colour-unknown:phClr".into());
                    return None;
                };
                let mut resolved = apply_transforms(&reference.hex, transforms, &WORD).ok()?;
                if reference.alpha < 1.0 && !transforms.iter().any(|t| t.kind == ooxml_colour::TransformKind::Alpha) {
                    resolved.alpha = reference.alpha;
                }
                return Some(upper(&resolved));
            }
            SourceColour::Scheme { slot, .. } => {
                let mapped = self.slots.get(slot).map_or(slot.as_str(), String::as_str);
                let Some(found) = self.theme.get(mapped) else {
                    said.push(format!("drawing-colour-unknown:{slot}"));
                    return None;
                };
                Some(found.to_string())
            }
        };
        let kind = match choice {
            SourceColour::Srgb { .. } => "srgb",
            SourceColour::System { .. } => "system",
            SourceColour::Scheme { .. } => "scheme",
        };
        match base.filter(|b| b.trim_start_matches('#').chars().count() == 6) {
            Some(base) => apply_transforms(&base, choice.transforms(), &WORD)
                .ok()
                .map(|c| upper(&c)),
            None => {
                said.push(format!("drawing-colour-unknown:{kind}"));
                None
            }
        }
    }

    /// A stated fill as the drawable's, `None` where it cannot be drawn (warned); `group`
    /// is what `a:grpFill` means (no fill where no group states one).
    pub fn fill(
        &self,
        fill: &source::Fill,
        ph_colour: Option<&Colour>,
        group: Option<&Fill>,
        said: &mut Said,
    ) -> Option<Fill> {
        match fill {
            source::Fill::None => Some(Fill::None),
            source::Fill::Solid(c) => self.resolve(c, ph_colour, said).map(Fill::Solid),
            source::Fill::Gradient(g) => {
                let stops: Vec<GradientStop> = g
                    .stops
                    .iter()
                    .filter_map(|s| {
                        Some(GradientStop {
                            position: s.position,
                            colour: self.resolve(&s.colour, ph_colour, said)?,
                        })
                    })
                    .collect();
                (!stops.is_empty()).then(|| {
                    Fill::Gradient(GradientFill {
                        stops,
                        angle: g.angle / 60000.0,
                        gradient_type: if g.kind == GradientKind::Radial {
                            "radial"
                        } else {
                            "linear"
                        }
                        .into(),
                        center_x: g.centre.map(|c| c.0),
                        center_y: g.centre.map(|c| c.1),
                        path: g.path.clone(),
                        focus: g.focus,
                        scaled: g.scaled,
                        rotate_with_shape: g.rotate_with_shape,
                    })
                })
            }
            source::Fill::Pattern(p) => {
                let foreground = self.resolve(&p.foreground, ph_colour, said);
                let background = self.resolve(&p.background, ph_colour, said);
                Some(Fill::Pattern {
                    preset: p.preset.clone(),
                    foreground: foreground?,
                    background: background?,
                })
            }
            source::Fill::Group => Some(group.cloned().unwrap_or(Fill::None)),
            source::Fill::Image(_) => {
                said.push("drawing-not-drawn:blipFill".into());
                None
            }
        }
    }

    /// A `wps:style` `a:fillRef`: the theme's fill style `idx` (1-based; from 1001 the
    /// background fill styles), `phClr` its colour.
    pub fn style_fill(&self, reference: Option<&StyleRef>, said: &mut Said) -> Option<Fill> {
        let (reference, formats) = (reference?, self.formats?);
        if reference.idx == 0 {
            return None;
        }
        let (styles, index) = if reference.idx >= 1000 {
            (&formats.background_fill_styles, reference.idx - 1001)
        } else {
            (&formats.fill_styles, reference.idx - 1)
        };
        let style = usize::try_from(index).ok().and_then(|i| styles.get(i))?;
        let ph_colour = self.reference_colour(reference, said);
        self.fill(style, ph_colour.as_ref(), None, said)
    }

    fn reference_colour(&self, reference: &StyleRef, said: &mut Said) -> Option<Colour> {
        reference.colour.as_ref().and_then(|c| self.resolve(c, None, said))
    }

    /// A shape's `a:ln` over its `a:lnRef` (the theme's line style `idx`, `phClr` its
    /// colour) as the drawable's outline, `None` for none.
    pub fn outline(
        &self,
        line: Option<&source::Outline>,
        reference: Option<&StyleRef>,
        said: &mut Said,
    ) -> Option<Outline> {
        let base = reference
            .filter(|r| r.idx > 0)
            .and_then(|r| self.formats?.line_styles.get(usize::try_from(r.idx - 1).ok()?));
        if line.is_none() && base.is_none() {
            return None;
        }
        let ph_colour = match (base, reference) {
            (Some(_), Some(r)) => self.reference_colour(r, said),
            _ => None,
        };
        let mut merged = source::Outline::default();
        for layer in [base, line].into_iter().flatten() {
            merged.width = layer.width.or(merged.width);
            merged.fill = layer.fill.clone().or(merged.fill);
            merged.dash = layer.dash.clone().or(merged.dash);
            merged.custom_dash = layer.custom_dash.clone().or(merged.custom_dash);
            merged.cap = layer.cap.clone().or(merged.cap);
            merged.join = layer.join.clone().or(merged.join);
            merged.head = layer.head.clone().or(merged.head);
            merged.tail = layer.tail.clone().or(merged.tail);
            merged.compound = layer.compound.clone().or(merged.compound);
        }
        let paint = merged
            .fill
            .as_ref()
            .and_then(|f| self.fill(f, ph_colour.as_ref(), None, said));
        let paint = match paint {
            None | Some(Fill::None) => return None,
            Some(p @ (Fill::Solid(_) | Fill::Gradient(_))) => p,
            Some(_) => {
                said.push("drawing-not-drawn:pattern outline".into());
                return None;
            }
        };
        if let Some(compound) = merged.compound.as_deref().filter(|c| *c != "sng") {
            said.push(format!("drawing-outline-approximate:{compound}"));
        }
        let end = |e: &LineEnd| ArrowEnd {
            kind: e.kind.clone(),
            width: e.width.clone(),
            length: e.length.clone(),
        };
        Some(Outline {
            width: merged.width.unwrap_or(DEFAULT_LINE_EMU),
            fill: Some(paint),
            dash_style: merged.dash.clone().unwrap_or_else(|| "solid".into()),
            custom_dash: merged.custom_dash.clone(),
            line_cap: merged.cap.clone(),
            line_join: merged.join.clone(),
            head_end: merged.head.as_ref().map(end),
            tail_end: merged.tail.as_ref().map(end),
            compound: merged.compound.clone(),
        })
    }

    /// `a:effectLst`, or else the theme's effect style `a:effectRef` names (1-based), as
    /// the drawable's; `None` where nothing resolves.
    pub fn effects(
        &self,
        effects: Option<&source::EffectList>,
        reference: Option<&StyleRef>,
        said: &mut Said,
    ) -> Option<Effects> {
        let from_style = || {
            let r = reference.filter(|r| r.idx > 0)?;
            self.formats?
                .effect_styles
                .get(usize::try_from(r.idx - 1).ok()?)?
                .as_ref()
        };
        let effects = effects.or_else(from_style)?;
        let mut out = Effects::default();
        if let Some(s) = &effects.outer_shadow {
            if let Some(colour) = self.resolve(&s.colour, None, said) {
                out.outer_shadow = Some(OuterShadow {
                    blur_radius: s.blur,
                    distance: s.distance,
                    direction: s.direction / 60000.0,
                    colour,
                    alignment: s.align.clone(),
                    rotate_with_shape: s.rotate_with_shape,
                });
            }
        }
        if let Some(s) = &effects.inner_shadow {
            if let Some(colour) = self.resolve(&s.colour, None, said) {
                out.inner_shadow = Some(InnerShadow {
                    blur_radius: s.blur,
                    distance: s.distance,
                    direction: s.direction / 60000.0,
                    colour,
                });
            }
        }
        if let Some(g) = &effects.glow {
            if let Some(colour) = self.resolve(&g.colour, None, said) {
                out.glow = Some(Glow {
                    radius: g.radius,
                    colour,
                });
            }
        }
        out.soft_edge = effects.soft_edge;
        (out != Effects::default()).then_some(out)
    }
}
