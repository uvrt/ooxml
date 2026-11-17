//! Owns: resolved colours, fills, outlines and effects as JSON, in the form
//! `python/drawingml_diff.py` writes the composition of ooxml-common's `resolve_color` and
//! `apply_transforms` in: a colour `{"hex", "alpha"}`, a value that cannot resolve `null`, a
//! stated hex value that is not one `{"error": "colour"}`.
//!
//! Must not: be used by any library; it is a test instrument.

use super::model::{num, nums, onum, opt, ostr};
use ooxml_colour::{ColourContext, ColourError, ResolvedColour, SourceColour};
use ooxml_drawingml::resolve::{self, Resolved, ResolvedEffects, ResolvedOutline};
use ooxml_drawingml::source::{EffectList, Fill, FormatScheme, GradientKind, LineEnd, Outline, ShapeStyle};
use serde_json::{json, Value as J};

fn error() -> J {
    json!({"error": "colour"})
}

/// A resolved colour.
pub fn colour(c: &ResolvedColour) -> J {
    json!({"hex": c.hex(), "alpha": num(c.alpha)})
}

/// A colour choice resolved (`phClr` as `reference`).
pub fn resolve_colour(context: &ColourContext, c: &SourceColour, reference: Option<ResolvedColour>) -> J {
    match resolve::colour(context, c, reference) {
        Ok(found) => opt(found.as_ref(), colour),
        Err(ColourError::Hex(_)) => error(),
    }
}

fn line_end(e: &LineEnd) -> J {
    json!({"type": e.kind, "width": e.width, "length": e.length})
}

fn fill_json(f: &Resolved) -> J {
    match f {
        Resolved::None => json!({"@": "NoFill"}),
        Resolved::Solid(c) => json!({"@": "SolidFill", "color": colour(c)}),
        Resolved::Pattern {
            preset,
            foreground,
            background,
        } => json!({"@": "PatternFill", "preset": preset, "foreground_color": colour(foreground),
                    "background_color": colour(background)}),
        Resolved::Image(i) => json!({"@": "ImageFill", "blip": i.blip}),
        Resolved::Gradient(g) => {
            let stops: Vec<J> = g
                .stops
                .iter()
                .map(|s| json!({"position": num(s.position), "color": colour(&s.colour)}))
                .collect();
            json!({"@": "GradientFill", "stops": stops, "angle": num(g.angle),
                   "gradient_type": if g.kind == GradientKind::Radial { "radial" } else { "linear" },
                   "center_x": onum(g.centre.map(|c| c.0)), "center_y": onum(g.centre.map(|c| c.1)),
                   "path": ostr(g.path.as_ref()), "focus": opt(g.focus.as_ref(), |f| nums(f)),
                   "scaled": g.scaled, "rotate_with_shape": g.rotate_with_shape})
        }
    }
}

fn outline_json(o: &ResolvedOutline) -> J {
    json!({"@": "Outline", "width": num(o.width), "fill": opt(o.fill.as_ref(), fill_json),
           "dash_style": o.dash, "custom_dash": opt(o.custom_dash.as_ref(), |d| nums(d)),
           "line_cap": ostr(o.cap.as_ref()), "line_join": ostr(o.join.as_ref()),
           "head_end": opt(o.head.as_ref(), line_end), "tail_end": opt(o.tail.as_ref(), line_end),
           "compound": ostr(o.compound.as_ref())})
}

fn effects_json(e: &ResolvedEffects) -> J {
    let shadow = |s: &resolve::ResolvedShadow, outer: bool| {
        let mut v = serde_json::Map::new();
        v.insert("blur_radius".into(), num(s.blur));
        v.insert("distance".into(), num(s.distance));
        v.insert("direction".into(), num(s.direction));
        v.insert("color".into(), colour(&s.colour));
        if outer {
            v.insert("alignment".into(), json!(s.align));
            v.insert("rotate_with_shape".into(), json!(s.rotate_with_shape));
        }
        J::Object(v)
    };
    json!({"@": "EffectList",
           "outer_shadow": opt(e.outer_shadow.as_ref(), |s| shadow(s, true)),
           "inner_shadow": opt(e.inner_shadow.as_ref(), |s| shadow(s, false)),
           "glow": opt(e.glow.as_ref(), |(r, c)| json!({"radius": num(*r), "color": colour(c)})),
           "soft_edge": opt(e.soft_edge.as_ref(), |r| json!({"radius": num(*r)}))})
}

fn or_error<T>(r: Result<Option<T>, ColourError>, f: impl Fn(&T) -> J) -> J {
    match r {
        Ok(found) => opt(found.as_ref(), f),
        Err(_) => error(),
    }
}

/// A stated fill resolved.
pub fn fill(context: &ColourContext, f: &Fill, reference: Option<ResolvedColour>) -> J {
    or_error(resolve::fill(context, f, reference, None), fill_json)
}

/// A stated outline resolved.
pub fn outline(context: &ColourContext, o: &Outline, reference: Option<ResolvedColour>) -> J {
    or_error(resolve::outline(context, o, reference), outline_json)
}

/// A stated effect list resolved.
pub fn effects(context: &ColourContext, e: &EffectList, reference: Option<ResolvedColour>) -> J {
    or_error(resolve::effects(context, e, reference), effects_json)
}

/// A shape style's references resolved against the theme's format scheme, each reference's
/// colour standing for `phClr`.
pub fn style_refs(context: &ColourContext, style: &ShapeStyle, formats: &FormatScheme) -> J {
    let reference = |r: Option<&ooxml_drawingml::source::StyleRef>| match r.and_then(|r| r.colour.as_ref()) {
        Some(c) => context.resolve(c).map_err(|_| ()),
        None => Ok(None),
    };
    let fill_ref = style.fill.as_ref().filter(|r| r.idx != 0).and_then(|r| {
        let f = formats.fill_style(r.idx)?;
        Some(match reference(Some(r)) {
            Ok(ph) => fill(context, f, ph),
            Err(()) => error(),
        })
    });
    let line_ref = style.line.as_ref().filter(|r| r.idx != 0).and_then(|r| {
        let o = formats.line_style(r.idx)?;
        Some(match reference(Some(r)) {
            Ok(ph) => outline(context, o, ph),
            Err(()) => error(),
        })
    });
    let effect_ref = style.effect.as_ref().and_then(|r| {
        let e = formats.effect_style(r.idx)?;
        Some(match reference(Some(r)) {
            Ok(ph) => effects(context, e, ph),
            Err(()) => error(),
        })
    });
    json!({"fill_ref": fill_ref, "line_ref": line_ref, "effect_ref": effect_ref})
}

/// A colour choice under both applications' rules.
pub fn both(contexts: &[(&str, ColourContext)], c: &SourceColour) -> J {
    J::Object(
        contexts
            .iter()
            .map(|(name, context)| ((*name).to_owned(), resolve_colour(context, c, None)))
            .collect(),
    )
}

/// The same for a colour that may be absent.
pub fn maybe(contexts: &[(&str, ColourContext)], c: Option<&SourceColour>) -> J {
    c.map_or(J::Null, |c| both(contexts, c))
}
