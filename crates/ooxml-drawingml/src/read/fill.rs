//! Owns: reading fills (`a:solidFill`, `a:noFill`, `a:gradFill`, `a:blipFill`, `a:pattFill`,
//! `a:grpFill`), lines (`a:ln` and a cell's borders), shape styles, effect lists and a
//! picture's effects, as ooxml-common's `drawingml/read.py` reads them.
//!
//! Must not: resolve a colour or a style reference (`crate::resolve`).

use super::colour::{colour, colour_node};
use super::{int_of, num, optional_bool, or, or_str, NS_R};
use crate::source::{
    BlipEffects, EffectList, Fill, Glow, GradientFill, GradientKind, GradientStop, ImageFill, LineEnd, Outline,
    PatternFill, Shadow, ShapeStyle, StyleRef, Tile,
};
use ooxml_schema::a;
use ooxml_xml::view::{attr_ns, elements};
use ooxml_xml::Element;

const DASH_STYLES: [&str; 11] = [
    "solid",
    "dash",
    "dot",
    "dashDot",
    "lgDash",
    "lgDashDot",
    "lgDashDotDot",
    "sysDash",
    "sysDot",
    "sysDashDot",
    "sysDashDotDot",
];
const ARROW_TYPES: [&str; 5] = ["triangle", "stealth", "diamond", "oval", "arrow"];
const ARROW_SIZES: [&str; 3] = ["sm", "med", "lg"];
const COMPOUND_LINES: [&str; 5] = ["sng", "dbl", "thickThin", "thinThick", "tri"];
const ALIGNMENTS: [&str; 9] = ["tl", "t", "tr", "l", "ctr", "r", "bl", "b", "br"];
const FONT_COLLECTIONS: [&str; 3] = ["major", "minor", "none"];

/// `value` where it is a rectangle alignment, else `fallback`.
pub(crate) fn alignment(value: Option<&str>, fallback: &str) -> String {
    value.filter(|v| ALIGNMENTS.contains(v)).unwrap_or(fallback).to_owned()
}

/// The fill `parent` holds (`a:spPr`, `a:ln`, `p:bgPr`, `a:tcPr`...), in ooxml-common's order
/// of preference: solid, none, gradient, picture, pattern, group. A solid fill without a
/// colour is no fill; an unreadable gradient, picture or pattern falls through.
pub fn fill(parent: Option<&Element>) -> Option<Fill> {
    let fills = a::Fills::new(parent?);
    if let Some(solid) = fills.solid() {
        return colour(Some(solid.element())).map(Fill::Solid);
    }
    if fills.none().is_some() {
        return Some(Fill::None);
    }
    if let Some(found) = fills.gradient().and_then(gradient) {
        return Some(Fill::Gradient(found));
    }
    if let Some(found) = fills.blip().and_then(image_fill) {
        return Some(Fill::Image(found));
    }
    if let Some(found) = fills.pattern().and_then(pattern) {
        return Some(Fill::Pattern(found));
    }
    fills.group().map(|_| Fill::Group)
}

/// One fill element read on its own (a format scheme's style lists hold bare fills): a
/// solid fill without a colour, an unreadable gradient, picture or pattern, or anything
/// that is not a fill, is none.
pub fn fill_element(node: &Element) -> Option<Fill> {
    if let Some(solid) = a::SolidFill::of(node) {
        return colour(Some(solid.element())).map(Fill::Solid);
    }
    match node.name().local() {
        "noFill" => Some(Fill::None),
        "gradFill" => gradient(a::GradFill::new(node)).map(Fill::Gradient),
        "blipFill" => image_fill(a::BlipFill::new(node)).map(Fill::Image),
        "pattFill" => pattern(a::PattFill::new(node)).map(Fill::Pattern),
        "grpFill" => Some(Fill::Group),
        _ => None,
    }
}

fn gradient(grad: a::GradFill<'_>) -> Option<GradientFill> {
    let stops: Vec<GradientStop> = grad
        .stops()
        .into_iter()
        .flat_map(a::GradientStops::stops)
        .filter_map(|stop| {
            Some(GradientStop {
                colour: colour(Some(stop.element()))?,
                position: or(stop.pos(), 0.0) / 100_000.0,
            })
        })
        .collect();
    if stops.is_empty() {
        return None;
    }
    let rotate_with_shape = optional_bool(grad.rotate_with_shape());
    if let Some(path) = grad.path() {
        let rect = path.fill_to().map(a::RelativeRect::element);
        let [l, t, r, b] = ["l", "t", "r", "b"].map(|k| or(num(rect, k), 0.0));
        return Some(GradientFill {
            stops,
            kind: GradientKind::Radial,
            angle: 0.0,
            centre: Some((
                (l + (100_000.0 - r)) / 2.0 / 100_000.0,
                (t + (100_000.0 - b)) / 2.0 / 100_000.0,
            )),
            path: Some(or_str(path.path(), "circle")),
            focus: Some([l, t, r, b].map(|v| v / 100_000.0)),
            scaled: None,
            rotate_with_shape,
        });
    }
    let linear = grad.linear();
    Some(GradientFill {
        stops,
        kind: GradientKind::Linear,
        angle: or(linear.and_then(a::LinearShade::angle), 0.0),
        centre: None,
        path: None,
        focus: None,
        scaled: optional_bool(linear.and_then(a::LinearShade::scaled)),
        rotate_with_shape,
    })
}

/// `asvg:svgBlip@r:embed` among a blip's extensions: the picture's vector original.
pub fn svg_blip(blip: Option<a::Blip<'_>>) -> Option<String> {
    blip?
        .extensions()?
        .extensions()
        .filter_map(a::Extension::svg_blip)
        .find_map(|svg| attr_ns(svg.element(), NS_R, "embed"))
        .map(str::to_owned)
}

fn image_fill(blip_fill: a::BlipFill<'_>) -> Option<ImageFill> {
    let blip = blip_fill.blip();
    Some(ImageFill {
        blip: blip.and_then(a::Blip::embed)?.to_owned(),
        svg: svg_blip(blip),
        tile: tile(blip_fill.tile()),
        src_rect: relative_rect(blip_fill.src_rect()),
        stretch: relative_rect(blip_fill.stretch().and_then(a::Stretch::fill_rect)),
    })
}

/// `a:tile`.
pub fn tile(tile: Option<a::Tile<'_>>) -> Option<Tile> {
    let tile = tile?;
    let flip = or_str(tile.flip(), "none");
    Some(Tile {
        tx: or(tile.tx(), 0.0),
        ty: or(tile.ty(), 0.0),
        sx: or(tile.sx(), 100_000.0) / 100_000.0,
        sy: or(tile.sy(), 100_000.0) / 100_000.0,
        flip: if ["x", "y", "xy"].contains(&flip.as_str()) {
            flip
        } else {
            "none".into()
        },
        align: alignment(tile.align(), "tl"),
    })
}

/// `a:srcRect` / `a:fillRect` as 0-1 insets `(l, t, r, b)`.
pub fn relative_rect(rect: Option<a::RelativeRect<'_>>) -> Option<[f64; 4]> {
    let rect = rect?;
    Some([rect.l(), rect.t(), rect.r(), rect.b()].map(|v| or(v, 0.0) / 100_000.0))
}

fn pattern(patt: a::PattFill<'_>) -> Option<PatternFill> {
    Some(PatternFill {
        foreground: colour(patt.foreground().map(a::Any::element))?,
        background: colour(patt.background().map(a::Any::element))?,
        preset: or_str(patt.preset(), "ltDnDiag"),
    })
}

/// `a:ln`, `a:lnL`, `a:lnR`, `a:lnT`, `a:lnB` as stated.
pub fn line(ln: Option<a::Ln<'_>>) -> Option<Outline> {
    let ln = ln?;
    let dash = ln.preset_dash().and_then(a::Val::val);
    let custom_dash: Vec<f64> = ln
        .custom_dash()
        .into_iter()
        .flat_map(a::CustomDash::segments)
        .flat_map(|s| {
            [
                or(s.dash(), 100_000.0) / 100_000.0,
                or(s.space(), 100_000.0) / 100_000.0,
            ]
        })
        .collect();
    let join = if ln.round().is_some() {
        Some("round")
    } else if ln.bevel().is_some() {
        Some("bevel")
    } else {
        ln.miter().map(|_| "miter")
    };
    let cap = ln.cap().filter(|c| !c.is_empty()).and_then(|c| match c {
        "flat" => Some("butt"),
        "sq" => Some("square"),
        "rnd" => Some("round"),
        _ => None,
    });
    Some(Outline {
        width: ln.width(),
        fill: fill(Some(ln.element())),
        dash: dash.filter(|d| DASH_STYLES.contains(d)).map(str::to_owned),
        custom_dash: (!custom_dash.is_empty()).then_some(custom_dash),
        cap: cap.map(str::to_owned),
        join: join.map(str::to_owned),
        head: line_end(ln.head_end()),
        tail: line_end(ln.tail_end()),
        compound: ln.compound().filter(|c| COMPOUND_LINES.contains(c)).map(str::to_owned),
    })
}

fn line_end(end: Option<a::LineEnd<'_>>) -> Option<LineEnd> {
    let end = end?;
    let kind = end.kind().filter(|k| ARROW_TYPES.contains(k))?;
    let size = |v: Option<&str>| {
        let v = or_str(v, "med");
        if ARROW_SIZES.contains(&v.as_str()) {
            v
        } else {
            "med".into()
        }
    };
    Some(LineEnd {
        kind: kind.to_owned(),
        width: size(end.width()),
        length: size(end.length()),
    })
}

fn style_ref(reference: Option<a::StyleRef<'_>>) -> Option<StyleRef> {
    let reference = reference?;
    Some(StyleRef {
        idx: int_of(or(reference.number(), 0.0)),
        colour: colour(Some(reference.element())),
        collection: reference
            .idx()
            .filter(|c| FONT_COLLECTIONS.contains(c))
            .map(str::to_owned),
    })
}

/// `p:style`, `None` where it references nothing.
pub fn shape_style(style: Option<a::ShapeStyle<'_>>) -> Option<ShapeStyle> {
    let style = style?;
    let parsed = ShapeStyle {
        fill: style_ref(style.fill_ref()),
        line: style_ref(style.line_ref()),
        effect: style_ref(style.effect_ref()),
        font: style_ref(style.font_ref()),
    };
    (parsed.fill.is_some() || parsed.line.is_some() || parsed.effect.is_some() || parsed.font.is_some())
        .then_some(parsed)
}

fn shadow(node: Option<a::Shadow<'_>>) -> Option<Shadow> {
    let node = node?;
    Some(Shadow {
        colour: colour(Some(node.element()))?,
        blur: or(node.blur(), 0.0),
        distance: or(node.distance(), 0.0),
        direction: or(node.direction(), 0.0),
        align: alignment(node.align(), "b"),
        rotate_with_shape: node.rotate_with_shape() != Some("0"),
    })
}

/// `a:effectLst`, `None` where it holds no effect read here.
pub fn effects(list: Option<a::EffectList<'_>>) -> Option<EffectList> {
    let list = list?;
    let glow = list.glow().and_then(|g| {
        Some(Glow {
            colour: colour(Some(g.element()))?,
            radius: or(g.radius(), 0.0),
        })
    });
    let parsed = EffectList {
        outer_shadow: shadow(list.outer_shadow()),
        inner_shadow: shadow(list.inner_shadow()),
        glow,
        soft_edge: list.soft_edge().map(|s| or(s.radius(), 0.0)),
    };
    let any = parsed.outer_shadow.is_some() || parsed.inner_shadow.is_some();
    (any || parsed.glow.is_some() || parsed.soft_edge.is_some()).then_some(parsed)
}

/// A picture's effects from its `a:blip`, `None` where it states none.
pub fn blip_effects(blip: Option<a::Blip<'_>>) -> Option<BlipEffects> {
    let blip = blip?;
    let duotone = blip.duotone().and_then(|d| {
        let mut colours = elements(d.element()).filter_map(colour_node);
        Some((colours.next()?, colours.next()?))
    });
    let colour_change = blip.colour_change().and_then(|c| {
        Some((
            colour(c.from().map(a::Any::element))?,
            colour(c.to().map(a::Any::element))?,
        ))
    });
    let alpha = blip
        .alpha_mod_fix()
        .map(|f| or(f.amount(), 100_000.0) / 100_000.0)
        .filter(|v| *v < 1.0);
    let parsed = BlipEffects {
        grayscale: blip.grayscale().is_some(),
        bi_level: blip.bi_level().map(|b| or(b.threshold(), 50_000.0) / 100_000.0),
        blur: blip.blur().map(|b| (or(b.radius(), 0.0), b.grow() != Some("0"))),
        luminance: blip
            .luminance()
            .map(|l| (or(l.bright(), 0.0) / 100_000.0, or(l.contrast(), 0.0) / 100_000.0)),
        duotone,
        colour_change,
        alpha,
    };
    let any = parsed.grayscale || parsed.bi_level.is_some() || parsed.blur.is_some() || parsed.luminance.is_some();
    (any || parsed.duotone.is_some() || parsed.colour_change.is_some() || parsed.alpha.is_some()).then_some(parsed)
}
