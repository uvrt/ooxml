//! Owns: a shape's effects (soft edge, glow, outer and inner shadow) and a picture's colour
//! effects (`a:clrChange`, grey scale, luminance, bi-level, duotone, blur, alpha) as SVG
//! filters (ooxml-common's `drawingml.effect`), with the filter region an element's drawn
//! bounds need -- at the painter's dpi.
//!
//! Must not: draw the element the filter goes on.

use crate::context::Painter;
use crate::units::emu_to_px_at;
use ooxml_drawingml::drawable::{BlipEffects, Effects};
use ooxml_scene::drawing_num as num;
use ooxml_scene::markup::{Attr, Element, Tag, Value};

/// How far a blur reaches, in standard deviations.
const BLUR_REACH: f64 = 3.0;
/// An inner shadow's reach, in standard deviations.
const INNER_REACH: f64 = 4.0;
/// User units added on every side of a filter region.
const REGION_PAD: f64 = 2.0;
/// How sharply `a:clrChange`'s mask falls away from the keyed colour.
const CLR_CHANGE_SHARPNESS: i32 = 64;
const LUMA: &str = "0.2126 0.7152 0.0722 0 0  0.2126 0.7152 0.0722 0 0  0.2126 0.7152 0.0722 0 0  0 0 0 1 0";

fn round2(value: f64) -> f64 {
    format!("{value:.2}").parse().unwrap_or(0.0)
}

fn el(tag: Tag, attrs: &[(&'static str, &str)]) -> Element {
    let mut e = Element::empty(tag);
    for (name, value) in attrs {
        e.set(name, Value::Word((*value).to_owned()));
    }
    e
}

fn merge(result: &str, first: &str, second: &str) -> Element {
    Element::open(Tag::FeMerge).word("result", result).holding(vec![
        el(Tag::FeMergeNode, &[("in", first)]),
        el(Tag::FeMergeNode, &[("in", second)]),
    ])
}

fn shadow_offset(blur_radius: f64, distance: f64, direction: f64, dpi: f64) -> (f64, f64, f64) {
    let std_dev = emu_to_px_at(blur_radius, dpi) / 2.0;
    let distance = emu_to_px_at(distance, dpi);
    let radians = direction.to_radians();
    (
        std_dev,
        round2(distance * radians.cos()),
        round2(distance * radians.sin()),
    )
}

fn glow_and_soft_edge(effects: &Effects, primitives: &mut Vec<Element>, dpi: f64) -> &'static str {
    let mut last = "SourceGraphic";
    if let Some(radius) = effects.soft_edge {
        let r = num(emu_to_px_at(radius, dpi));
        primitives.push(el(
            Tag::FeGaussianBlur,
            &[("in", "SourceAlpha"), ("stdDeviation", &r), ("result", "softEdgeMask")],
        ));
        primitives.push(el(
            Tag::FeComposite,
            &[
                ("in", "SourceGraphic"),
                ("in2", "softEdgeMask"),
                ("operator", "in"),
                ("result", "softEdgeResult"),
            ],
        ));
        last = "softEdgeResult";
    }
    if let Some(glow) = &effects.glow {
        let r = num(emu_to_px_at(glow.radius, dpi));
        if last != "SourceGraphic" {
            primitives.push(el(
                Tag::FeColorMatrix,
                &[
                    ("in", last),
                    ("type", "matrix"),
                    ("values", "0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0"),
                    ("result", "glowAlpha"),
                ],
            ));
        }
        let blur_in = if last == "SourceGraphic" {
            "SourceAlpha"
        } else {
            "glowAlpha"
        };
        primitives.push(el(
            Tag::FeGaussianBlur,
            &[("in", blur_in), ("stdDeviation", &r), ("result", "glowBlur")],
        ));
        primitives.push(el(
            Tag::FeFlood,
            &[
                ("flood-color", &glow.colour.hex),
                ("flood-opacity", &num(glow.colour.alpha)),
                ("result", "glowColor"),
            ],
        ));
        primitives.push(el(
            Tag::FeComposite,
            &[
                ("in", "glowColor"),
                ("in2", "glowBlur"),
                ("operator", "in"),
                ("result", "glowFinal"),
            ],
        ));
        primitives.push(merge("glowMerge", "glowFinal", last));
        last = "glowMerge";
    }
    last
}

fn shadows(effects: &Effects, mut last: &'static str, primitives: &mut Vec<Element>, dpi: f64) {
    if let Some(s) = &effects.outer_shadow {
        let (std_dev, dx, dy) = shadow_offset(s.blur_radius, s.distance, s.direction, dpi);
        primitives.push(el(
            Tag::FeGaussianBlur,
            &[
                ("in", "SourceAlpha"),
                ("stdDeviation", &num(std_dev)),
                ("result", "shadowBlur"),
            ],
        ));
        primitives.push(el(
            Tag::FeOffset,
            &[
                ("in", "shadowBlur"),
                ("dx", &num(dx)),
                ("dy", &num(dy)),
                ("result", "shadowOffset"),
            ],
        ));
        primitives.push(el(
            Tag::FeFlood,
            &[
                ("flood-color", &s.colour.hex),
                ("flood-opacity", &num(s.colour.alpha)),
                ("result", "shadowColor"),
            ],
        ));
        primitives.push(el(
            Tag::FeComposite,
            &[
                ("in", "shadowColor"),
                ("in2", "shadowOffset"),
                ("operator", "in"),
                ("result", "shadowFinal"),
            ],
        ));
        primitives.push(merge("outerShadowMerge", "shadowFinal", last));
        last = "outerShadowMerge";
    }
    inner_shadow(effects, last, primitives, dpi);
}

fn inner_shadow(effects: &Effects, last: &'static str, primitives: &mut Vec<Element>, dpi: f64) {
    if let Some(s) = &effects.inner_shadow {
        let (std_dev, dx, dy) = shadow_offset(s.blur_radius, s.distance, s.direction, dpi);
        primitives.push(
            Element::open(Tag::FeComponentTransfer)
                .word("in", "SourceAlpha")
                .word("result", "innerShdwInverse")
                .holding(vec![el(Tag::FeFuncA, &[("type", "table"), ("tableValues", "1 0")])]),
        );
        primitives.push(el(
            Tag::FeGaussianBlur,
            &[
                ("in", "innerShdwInverse"),
                ("stdDeviation", &num(std_dev)),
                ("result", "innerShdwBlur"),
            ],
        ));
        primitives.push(el(
            Tag::FeOffset,
            &[
                ("in", "innerShdwBlur"),
                ("dx", &num(dx)),
                ("dy", &num(dy)),
                ("result", "innerShdwOffset"),
            ],
        ));
        primitives.push(el(
            Tag::FeFlood,
            &[
                ("flood-color", &s.colour.hex),
                ("flood-opacity", &num(s.colour.alpha)),
                ("result", "innerShdwFill"),
            ],
        ));
        primitives.push(el(
            Tag::FeComposite,
            &[
                ("in", "innerShdwFill"),
                ("in2", "innerShdwOffset"),
                ("operator", "in"),
                ("result", "innerShdwColored"),
            ],
        ));
        primitives.push(el(
            Tag::FeComposite,
            &[
                ("in", "innerShdwColored"),
                ("in2", "SourceAlpha"),
                ("operator", "in"),
                ("result", "innerShdwClipped"),
            ],
        ));
        primitives.push(el(
            Tag::FeComposite,
            &[("in", "innerShdwClipped"), ("in2", last), ("operator", "over")],
        ));
    }
}

/// `filter="url(#...)"` for an element's effects, the filter declared; `bounds`
/// (`x0, y0, x1, y1`) is what the element draws within, where known.
pub fn render_effects(
    effects: Option<&Effects>,
    painter: &mut Painter<'_>,
    bounds: Option<(f64, f64, f64, f64)>,
) -> Option<Attr> {
    let effects = effects?;
    let mut primitives = Vec::new();
    let last = glow_and_soft_edge(effects, &mut primitives, painter.dpi);
    shadows(effects, last, &mut primitives, painter.dpi);
    if primitives.is_empty() {
        return None;
    }
    let id = painter.new_id("effect");
    let mut filter = Element::open(Tag::Filter).word("id", &id);
    match bounds {
        None => {
            filter = filter
                .word("x", "-50%")
                .word("y", "-50%")
                .word("width", "200%")
                .word("height", "200%");
        }
        Some(b) => {
            let (x, y, w, h) = effect_region(effects, b, painter.dpi);
            filter = filter
                .word("filterUnits", "userSpaceOnUse")
                .num("x", x)
                .num("y", y)
                .num("width", w)
                .num("height", h);
        }
    }
    painter.add_def(filter.word("color-interpolation-filters", "sRGB").holding(primitives));
    Some(Attr {
        name: "filter",
        value: Value::Url(id),
    })
}

/// The filter region of `effects` over an element drawing within `bounds`, at `dpi`.
pub fn effect_region(effects: &Effects, bounds: (f64, f64, f64, f64), dpi: f64) -> (f64, f64, f64, f64) {
    let mut side = 0.0_f64;
    for radius in [effects.soft_edge, effects.glow.as_ref().map(|g| g.radius)]
        .into_iter()
        .flatten()
    {
        side = side.max(BLUR_REACH * emu_to_px_at(radius, dpi));
    }
    let outer = effects
        .outer_shadow
        .as_ref()
        .map(|s| (s.blur_radius, s.distance, s.direction, false));
    let inner = effects
        .inner_shadow
        .as_ref()
        .map(|s| (s.blur_radius, s.distance, s.direction, true));
    for (blur, distance, direction, is_inner) in [outer, inner].into_iter().flatten() {
        let (std_dev, dx, dy) = shadow_offset(blur, distance, direction, dpi);
        let mut reach = BLUR_REACH * std_dev;
        if is_inner {
            reach *= INNER_REACH / BLUR_REACH;
        }
        reach += dx.hypot(dy);
        side = side.max(reach);
    }
    let (x0, y0, x1, y1) = bounds;
    (
        x0 - side - REGION_PAD,
        y0 - side - REGION_PAD,
        (x1 - x0) + side + side + 2.0 * REGION_PAD,
        (y1 - y0) + side + side + 2.0 * REGION_PAD,
    )
}

fn unit(hex: &str, index: usize) -> f64 {
    let value = hex.trim_start_matches('#');
    let channel = value
        .get(index * 2..index * 2 + 2)
        .and_then(|t| u8::from_str_radix(t, 16).ok())
        .unwrap_or(0);
    f64::from(channel) / 255.0
}

fn channel(hex: &str, index: usize) -> String {
    num(unit(hex, index))
}

fn transfer(funcs: [(Tag, Vec<(&'static str, String)>); 3]) -> Element {
    let children = funcs
        .into_iter()
        .map(|(tag, attrs)| {
            let mut e = Element::empty(tag);
            for (name, value) in attrs {
                e.set(name, Value::Word(value));
            }
            e
        })
        .collect();
    Element::open(Tag::FeComponentTransfer).holding(children)
}

fn clr_change(source: &str, target: &str, out: &mut Vec<Element>) {
    let (r0, g0, b0) = (unit(source, 0), unit(source, 1), unit(source, 2));
    let k = -CLR_CHANGE_SHARPNESS;
    out.push(el(
        Tag::FeColorMatrix,
        &[
            ("type", "matrix"),
            ("in", "SourceGraphic"),
            ("result", "clrChangeAbove"),
            (
                "values",
                &format!(
                    "1 0 0 0 {}  0 1 0 0 {}  0 0 1 0 {}  0 0 0 0 1",
                    num(-r0),
                    num(-g0),
                    num(-b0)
                ),
            ),
        ],
    ));
    out.push(el(
        Tag::FeColorMatrix,
        &[
            ("type", "matrix"),
            ("in", "SourceGraphic"),
            ("result", "clrChangeBelow"),
            (
                "values",
                &format!(
                    "-1 0 0 0 {}  0 -1 0 0 {}  0 0 -1 0 {}  0 0 0 0 1",
                    num(r0),
                    num(g0),
                    num(b0)
                ),
            ),
        ],
    ));
    out.push(el(
        Tag::FeComposite,
        &[
            ("in", "clrChangeAbove"),
            ("in2", "clrChangeBelow"),
            ("operator", "arithmetic"),
            ("k1", "0"),
            ("k2", "1"),
            ("k3", "1"),
            ("k4", "0"),
            ("result", "clrChangeDistance"),
        ],
    ));
    let values = format!(
        "0 0 0 0 {}  0 0 0 0 {}  0 0 0 0 {}  {k} {k} {k} 0 1",
        channel(target, 0),
        channel(target, 1),
        channel(target, 2)
    );
    out.push(el(
        Tag::FeColorMatrix,
        &[
            ("type", "matrix"),
            ("in", "clrChangeDistance"),
            ("result", "clrChangeMask"),
            ("values", &values),
        ],
    ));
    out.push(el(
        Tag::FeComposite,
        &[
            ("in", "clrChangeMask"),
            ("in2", "SourceGraphic"),
            ("operator", "in"),
            ("result", "clrChangeKeyed"),
        ],
    ));
    out.push(el(
        Tag::FeComposite,
        &[("in", "clrChangeKeyed"), ("in2", "SourceGraphic"), ("operator", "over")],
    ));
}

fn luma() -> Element {
    el(Tag::FeColorMatrix, &[("type", "matrix"), ("values", LUMA)])
}

fn tone_effects(effects: &BlipEffects, out: &mut Vec<Element>) {
    if let Some((brightness, contrast)) = effects.lum {
        let linear = || {
            vec![
                ("type", "linear".to_owned()),
                ("slope", num(1.0 + contrast)),
                ("intercept", num(brightness)),
            ]
        };
        out.push(transfer([
            (Tag::FeFuncR, linear()),
            (Tag::FeFuncG, linear()),
            (Tag::FeFuncB, linear()),
        ]));
    }
    if let Some(threshold) = effects.bi_level {
        let buckets = 20_i64;
        let black = ((threshold * 20.0).round_ties_even() as i64).clamp(1, buckets - 1);
        let mut table = vec!["0"; usize::try_from(black).unwrap_or(1)];
        table.extend(vec!["1"; usize::try_from(buckets - black).unwrap_or(1)]);
        let table = table.join(" ");
        let discrete = || vec![("type", "discrete".to_owned()), ("tableValues", table.clone())];
        out.push(luma());
        out.push(transfer([
            (Tag::FeFuncR, discrete()),
            (Tag::FeFuncG, discrete()),
            (Tag::FeFuncB, discrete()),
        ]));
    }
    if let Some((first, second)) = &effects.duotone {
        let ramp = |i: usize| {
            vec![
                ("type", "table".to_owned()),
                (
                    "tableValues",
                    format!("{} {}", channel(&first.hex, i), channel(&second.hex, i)),
                ),
            ]
        };
        out.push(luma());
        out.push(transfer([
            (Tag::FeFuncR, ramp(0)),
            (Tag::FeFuncG, ramp(1)),
            (Tag::FeFuncB, ramp(2)),
        ]));
    }
}

/// `filter="url(#...)"` for a picture's colour effects, the filter declared.
pub fn render_blip_effects(effects: Option<&BlipEffects>, painter: &mut Painter<'_>) -> Option<Attr> {
    let effects = effects?;
    let mut primitives = Vec::new();
    if let Some((from, to)) = &effects.clr_change {
        clr_change(&from.hex, &to.hex, &mut primitives);
    }
    if effects.grayscale {
        primitives.push(luma());
    }
    tone_effects(effects, &mut primitives);
    if let Some((radius, _)) = effects.blur {
        primitives.push(el(
            Tag::FeGaussianBlur,
            &[("stdDeviation", &num(emu_to_px_at(radius, painter.dpi) / 2.0))],
        ));
    }
    if let Some(alpha) = effects.alpha.filter(|a| *a < 1.0) {
        primitives.push(Element::open(Tag::FeComponentTransfer).holding(vec![el(
            Tag::FeFuncA,
            &[("type", "linear"), ("slope", &num(alpha)), ("intercept", "0")],
        )]));
    }
    if primitives.is_empty() {
        return None;
    }
    let id = painter.new_id("blip");
    painter.add_def(
        Element::open(Tag::Filter)
            .word("id", &id)
            .word("x", "0%")
            .word("y", "0%")
            .word("width", "100%")
            .word("height", "100%")
            .word("color-interpolation-filters", "sRGB")
            .holding(primitives),
    );
    Some(Attr {
        name: "filter",
        value: Value::Url(id),
    })
}
