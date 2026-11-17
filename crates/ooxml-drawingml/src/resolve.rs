//! Owns: resolution -- a theme's colours and the colour maps layered over them made a
//! [`ColourContext`], and a stated fill, outline or effect list turned into concrete colours
//! under it ([`Resolved`]). A style reference's colour stands for `phClr` in the theme's
//! style it selects ([`colour`]: PowerPoint's and Word's rule alike, measured by pptx2svg's
//! `make_style_probe.py`): the reference's colour, the entry's own transforms applied to it,
//! its opacity carried where the entry states none.
//!
//! Must not: merge a shape's own line with its style's (PowerPoint and Word differ: the
//! consumer's), load a picture's bytes, or choose what a group fill means when no group
//! states one (the caller passes it).

use crate::read::theme::Theme;
use crate::source::{EffectList, Fill, GradientKind, ImageFill, LineEnd, Outline};
use ooxml_colour::{
    apply_transforms, ColourContext, ColourError, ColourMap, ColourRules, ResolvedColour, SourceColour,
};

/// A line's width where it states none: 0.75 pt, in EMU.
pub const DEFAULT_LINE_WIDTH: f64 = 9525.0;

/// A colour context for `theme` (the Office fallback where there is none) under `map`.
pub fn colour_context(theme: Option<&Theme>, map: ColourMap, rules: ColourRules) -> ColourContext {
    ColourContext::new(theme.map(Theme::colour_map), map, rules)
}

/// A resolved gradient stop.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    /// The position, 0-1.
    pub position: f64,
    /// The colour.
    pub colour: ResolvedColour,
}

/// A resolved gradient: the stated geometry, the stops' colours resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    /// The stops that resolved (at least one).
    pub stops: Vec<Stop>,
    /// Linear or path.
    pub kind: GradientKind,
    /// The angle in degrees.
    pub angle: f64,
    /// A path gradient's centre.
    pub centre: Option<(f64, f64)>,
    /// `a:path@path`.
    pub path: Option<String>,
    /// `a:fillToRect` as 0-1 insets.
    pub focus: Option<[f64; 4]>,
    /// `a:lin@scaled`.
    pub scaled: Option<bool>,
    /// `rotWithShape`.
    pub rotate_with_shape: Option<bool>,
}

/// A fill in concrete colours.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// No fill.
    None,
    /// One colour.
    Solid(ResolvedColour),
    /// A gradient.
    Gradient(Gradient),
    /// A pattern in two colours.
    Pattern {
        /// The preset.
        preset: String,
        /// The foreground.
        foreground: ResolvedColour,
        /// The background.
        background: ResolvedColour,
    },
    /// A picture, still by relationship (its bytes are the consumer's to load).
    Image(ImageFill),
}

/// An outline in concrete colours.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedOutline {
    /// The width, EMU (0.75 pt where unstated).
    pub width: f64,
    /// A solid or gradient fill; `None` for one the outline cannot draw (a picture, a pattern).
    pub fill: Option<Resolved>,
    /// The preset dash (`solid` where unstated).
    pub dash: String,
    /// A custom dash.
    pub custom_dash: Option<Vec<f64>>,
    /// The cap.
    pub cap: Option<String>,
    /// The join.
    pub join: Option<String>,
    /// The head's arrowhead.
    pub head: Option<LineEnd>,
    /// The tail's arrowhead.
    pub tail: Option<LineEnd>,
    /// `cmpd`.
    pub compound: Option<String>,
}

/// A shadow in concrete colours; the direction in degrees.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedShadow {
    /// Blur radius, EMU.
    pub blur: f64,
    /// Distance, EMU.
    pub distance: f64,
    /// Direction, degrees.
    pub direction: f64,
    /// The colour.
    pub colour: ResolvedColour,
    /// An outer shadow's alignment.
    pub align: String,
    /// An outer shadow's `rotWithShape`.
    pub rotate_with_shape: bool,
}

/// Effects in concrete colours (at least one).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedEffects {
    /// The outer shadow.
    pub outer_shadow: Option<ResolvedShadow>,
    /// The inner shadow.
    pub inner_shadow: Option<ResolvedShadow>,
    /// The glow: radius in EMU and colour.
    pub glow: Option<(f64, ResolvedColour)>,
    /// The soft edge's radius, EMU.
    pub soft_edge: Option<f64>,
}

/// `colour` under `context`, where `phClr` is `reference` (a style reference's own colour;
/// without one `phClr` names no colour).
pub fn colour(
    context: &ColourContext,
    colour: &SourceColour,
    reference: Option<ResolvedColour>,
) -> Result<Option<ResolvedColour>, ColourError> {
    let SourceColour::Scheme { slot, transforms } = colour else {
        return context.resolve(colour);
    };
    if slot != "phClr" {
        return context.resolve(colour);
    }
    let Some(reference) = reference else {
        return Ok(None);
    };
    let mut resolved = apply_transforms(&reference.hex(), transforms, context.rules())?;
    let states_alpha = transforms.iter().any(|t| t.kind == ooxml_colour::TransformKind::Alpha);
    if reference.alpha < 1.0 && !states_alpha {
        resolved.alpha = reference.alpha;
    }
    Ok(Some(resolved))
}

/// A stated fill in concrete colours; `group` is what `a:grpFill` means here. `None` where
/// nothing can be drawn (a solid fill whose colour names nothing, a gradient none of whose
/// stops resolve, a pattern missing a colour, a group fill with no group's fill).
pub fn fill(
    context: &ColourContext,
    stated: &Fill,
    reference: Option<ResolvedColour>,
    group: Option<&Resolved>,
) -> Result<Option<Resolved>, ColourError> {
    let resolve = |c: &SourceColour| colour(context, c, reference);
    Ok(match stated {
        Fill::None => Some(Resolved::None),
        Fill::Solid(c) => resolve(c)?.map(Resolved::Solid),
        Fill::Gradient(g) => {
            let mut stops = Vec::new();
            for stop in &g.stops {
                if let Some(c) = resolve(&stop.colour)? {
                    stops.push(Stop {
                        position: stop.position,
                        colour: c,
                    });
                }
            }
            (!stops.is_empty()).then(|| {
                Resolved::Gradient(Gradient {
                    stops,
                    kind: g.kind,
                    angle: g.angle / 60000.0,
                    centre: g.centre,
                    path: g.path.clone(),
                    focus: g.focus,
                    scaled: g.scaled,
                    rotate_with_shape: g.rotate_with_shape,
                })
            })
        }
        Fill::Pattern(p) => match (resolve(&p.foreground)?, resolve(&p.background)?) {
            (Some(foreground), Some(background)) => Some(Resolved::Pattern {
                preset: p.preset.clone(),
                foreground,
                background,
            }),
            _ => None,
        },
        Fill::Image(image) => Some(Resolved::Image(image.clone())),
        Fill::Group => group.cloned(),
    })
}

/// A stated outline in concrete colours, `None` for a line with no fill (`a:noFill`).
pub fn outline(
    context: &ColourContext,
    stated: &Outline,
    reference: Option<ResolvedColour>,
) -> Result<Option<ResolvedOutline>, ColourError> {
    let paint = match &stated.fill {
        Some(f) => fill(context, f, reference, None)?,
        None => None,
    };
    if paint == Some(Resolved::None) {
        return Ok(None);
    }
    Ok(Some(ResolvedOutline {
        width: stated.width.unwrap_or(DEFAULT_LINE_WIDTH),
        fill: paint.filter(|p| matches!(p, Resolved::Solid(_) | Resolved::Gradient(_))),
        dash: stated.dash.clone().unwrap_or_else(|| "solid".into()),
        custom_dash: stated.custom_dash.clone().filter(|d| !d.is_empty()),
        cap: stated.cap.clone(),
        join: stated.join.clone(),
        head: stated.head.clone(),
        tail: stated.tail.clone(),
        compound: stated.compound.clone(),
    }))
}

/// A stated effect list in concrete colours, `None` where no effect resolves.
pub fn effects(
    context: &ColourContext,
    stated: &EffectList,
    reference: Option<ResolvedColour>,
) -> Result<Option<ResolvedEffects>, ColourError> {
    let shadow = |s: &crate::source::Shadow| -> Result<Option<ResolvedShadow>, ColourError> {
        Ok(colour(context, &s.colour, reference)?.map(|c| ResolvedShadow {
            blur: s.blur,
            distance: s.distance,
            direction: s.direction / 60000.0,
            colour: c,
            align: s.align.clone(),
            rotate_with_shape: s.rotate_with_shape,
        }))
    };
    let mut out = ResolvedEffects::default();
    if let Some(s) = &stated.outer_shadow {
        out.outer_shadow = shadow(s)?;
    }
    if let Some(s) = &stated.inner_shadow {
        out.inner_shadow = shadow(s)?;
    }
    if let Some(g) = &stated.glow {
        out.glow = colour(context, &g.colour, reference)?.map(|c| (g.radius, c));
    }
    out.soft_edge = stated.soft_edge;
    Ok((out != ResolvedEffects::default()).then_some(out))
}
