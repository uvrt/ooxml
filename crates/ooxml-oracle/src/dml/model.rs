//! Owns: the DrawingML source model as JSON in the form `python/drawingml_diff.py` writes
//! ooxml-common's dataclasses in -- each value a map led by its Python class name (`"@"`)
//! with every field by its Python name, tuples and lists as arrays, a dict as
//! `{"{}": [[key, value]...]}`, and a number that is not finite as `{"float": "nan"}`.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_colour::{SourceColour, Transform};
use ooxml_drawingml::source::{
    BlipEffects, EffectList, Fill, FormatScheme, Geometry, GeometrySpec, GradientKind, LineEnd, Outline, Shadow,
    ShapeStyle, SpecPath, StyleRef, Tile, Xfrm,
};
use serde_json::{json, Map, Value as J};

/// A number as JSON (a non-finite one as Python's `repr`).
pub fn num(v: f64) -> J {
    if v.is_finite() {
        json!(v)
    } else if v.is_nan() {
        json!({"float": "nan"})
    } else if v > 0.0 {
        json!({"float": "inf"})
    } else {
        json!({"float": "-inf"})
    }
}

/// An optional number.
pub fn onum(v: Option<f64>) -> J {
    v.map_or(J::Null, num)
}

/// A dataclass: its class name and fields.
pub fn class(name: &str, fields: Vec<(&str, J)>) -> J {
    let mut out = Map::new();
    out.insert("@".into(), json!(name));
    for (k, v) in fields {
        out.insert(k.into(), v);
    }
    J::Object(out)
}

/// An optional value through `f`.
pub fn opt<T>(v: Option<&T>, f: impl Fn(&T) -> J) -> J {
    v.map_or(J::Null, f)
}

/// An optional string.
pub fn ostr(v: Option<&String>) -> J {
    v.map_or(J::Null, |s| json!(s))
}

/// Floats as an array.
pub fn nums(values: &[f64]) -> J {
    J::Array(values.iter().copied().map(num).collect())
}

fn transform(t: &Transform) -> J {
    class(
        "ColorTransform",
        vec![("kind", json!(t.kind.name())), ("value", num(t.value))],
    )
}

/// A colour choice.
pub fn colour(c: &SourceColour) -> J {
    let transforms = J::Array(c.transforms().iter().map(transform).collect());
    match c {
        SourceColour::Srgb { hex, linear, .. } => class(
            "SrgbColor",
            vec![
                ("hex", json!(hex)),
                ("transforms", transforms),
                ("kind", json!("srgb")),
                ("linear", opt(linear.as_ref(), |l| nums(l))),
            ],
        ),
        SourceColour::Scheme { slot, .. } => class(
            "SchemeColor",
            vec![
                ("scheme", json!(slot)),
                ("transforms", transforms),
                ("kind", json!("scheme")),
            ],
        ),
        SourceColour::System { value, last, .. } => class(
            "SystemColor",
            vec![
                ("value", json!(value)),
                ("last_color", ostr(last.as_ref())),
                ("transforms", transforms),
                ("kind", json!("system")),
            ],
        ),
    }
}

fn tile(t: &Tile) -> J {
    class(
        "SourceImageFillTile",
        vec![
            ("tx", num(t.tx)),
            ("ty", num(t.ty)),
            ("sx", num(t.sx)),
            ("sy", num(t.sy)),
            ("flip", json!(t.flip)),
            ("align", json!(t.align)),
        ],
    )
}

/// A picture's tile, for a picture node.
pub fn image_tile(t: Option<&Tile>) -> J {
    opt(t, tile)
}

/// A fill.
pub fn fill(f: &Fill) -> J {
    match f {
        Fill::Solid(c) => class("SourceSolidFill", vec![("color", colour(c)), ("kind", json!("solid"))]),
        Fill::None => class("SourceNoFill", vec![("kind", json!("none"))]),
        Fill::Group => class("SourceGroupFill", vec![("kind", json!("group"))]),
        Fill::Gradient(g) => {
            let stops = g
                .stops
                .iter()
                .map(|s| {
                    class(
                        "SourceGradientStop",
                        vec![("position", num(s.position)), ("color", colour(&s.colour))],
                    )
                })
                .collect();
            let kind = if g.kind == GradientKind::Radial {
                "radial"
            } else {
                "linear"
            };
            class(
                "SourceGradientFill",
                vec![
                    ("stops", J::Array(stops)),
                    ("gradient_type", json!(kind)),
                    ("angle", num(g.angle)),
                    ("center_x", onum(g.centre.map(|c| c.0))),
                    ("center_y", onum(g.centre.map(|c| c.1))),
                    ("kind", json!("gradient")),
                    ("path", ostr(g.path.as_ref())),
                    ("focus", opt(g.focus.as_ref(), |f| nums(f))),
                    ("scaled", json!(g.scaled)),
                    ("rotate_with_shape", json!(g.rotate_with_shape)),
                ],
            )
        }
        Fill::Image(i) => class(
            "SourceImageFill",
            vec![
                ("blip_relationship_id", json!(i.blip)),
                ("svg_relationship_id", ostr(i.svg.as_ref())),
                ("tile", opt(i.tile.as_ref(), tile)),
                ("src_rect", opt(i.src_rect.as_ref(), |r| nums(r))),
                ("stretch", opt(i.stretch.as_ref(), |r| nums(r))),
                ("kind", json!("image")),
            ],
        ),
        Fill::Pattern(p) => class(
            "SourcePatternFill",
            vec![
                ("preset", json!(p.preset)),
                ("foreground_color", colour(&p.foreground)),
                ("background_color", colour(&p.background)),
                ("kind", json!("pattern")),
            ],
        ),
    }
}

fn line_end(e: &LineEnd) -> J {
    class(
        "ArrowEndpoint",
        vec![
            ("type", json!(e.kind)),
            ("width", json!(e.width)),
            ("length", json!(e.length)),
        ],
    )
}

/// An outline.
pub fn outline(o: &Outline) -> J {
    class(
        "SourceOutline",
        vec![
            ("width", onum(o.width)),
            ("fill", opt(o.fill.as_ref(), fill)),
            ("dash_style", ostr(o.dash.as_ref())),
            ("custom_dash", opt(o.custom_dash.as_ref(), |d| nums(d))),
            ("line_cap", ostr(o.cap.as_ref())),
            ("line_join", ostr(o.join.as_ref())),
            ("head_end", opt(o.head.as_ref(), line_end)),
            ("tail_end", opt(o.tail.as_ref(), line_end)),
            ("compound", ostr(o.compound.as_ref())),
        ],
    )
}

fn style_ref(r: &StyleRef) -> J {
    class(
        "SourceStyleReference",
        vec![
            ("idx", json!(r.idx)),
            ("color", opt(r.colour.as_ref(), colour)),
            ("collection", ostr(r.collection.as_ref())),
        ],
    )
}

/// A shape style.
pub fn shape_style(s: &ShapeStyle) -> J {
    class(
        "SourceShapeStyle",
        vec![
            ("fill_ref", opt(s.fill.as_ref(), style_ref)),
            ("line_ref", opt(s.line.as_ref(), style_ref)),
            ("effect_ref", opt(s.effect.as_ref(), style_ref)),
            ("font_ref", opt(s.font.as_ref(), style_ref)),
        ],
    )
}

fn shadow(s: &Shadow, outer: bool) -> J {
    let mut fields = vec![
        ("blur_radius", num(s.blur)),
        ("distance", num(s.distance)),
        ("direction", num(s.direction)),
        ("color", colour(&s.colour)),
    ];
    if outer {
        fields.push(("alignment", json!(s.align)));
        fields.push(("rotate_with_shape", json!(s.rotate_with_shape)));
    }
    class(
        if outer {
            "SourceOuterShadow"
        } else {
            "SourceInnerShadow"
        },
        fields,
    )
}

/// An effect list.
pub fn effects(e: &EffectList) -> J {
    class(
        "SourceEffectList",
        vec![
            ("outer_shadow", opt(e.outer_shadow.as_ref(), |s| shadow(s, true))),
            ("inner_shadow", opt(e.inner_shadow.as_ref(), |s| shadow(s, false))),
            (
                "glow",
                opt(e.glow.as_ref(), |g| {
                    class(
                        "SourceGlow",
                        vec![("radius", num(g.radius)), ("color", colour(&g.colour))],
                    )
                }),
            ),
            (
                "soft_edge",
                opt(e.soft_edge.as_ref(), |r| {
                    class("SourceSoftEdge", vec![("radius", num(*r))])
                }),
            ),
        ],
    )
}

/// A picture's effects.
pub fn blip_effects(b: &BlipEffects) -> J {
    let pair = |p: &(SourceColour, SourceColour)| json!([colour(&p.0), colour(&p.1)]);
    class(
        "SourceBlipEffects",
        vec![
            ("grayscale", json!(b.grayscale)),
            ("bi_level", onum(b.bi_level)),
            ("blur", opt(b.blur.as_ref(), |(r, g)| json!([num(*r), g]))),
            ("lum", opt(b.luminance.as_ref(), |(x, y)| json!([num(*x), num(*y)]))),
            ("duotone", opt(b.duotone.as_ref(), pair)),
            ("clr_change", opt(b.colour_change.as_ref(), pair)),
            ("alpha", onum(b.alpha)),
        ],
    )
}

/// A transform.
pub fn xfrm(x: &Xfrm) -> J {
    class(
        "SourceTransform",
        vec![
            ("offset_x", num(x.x)),
            ("offset_y", num(x.y)),
            ("width", num(x.width)),
            ("height", num(x.height)),
            ("rotation", num(x.rotation)),
            ("flip_horizontal", json!(x.flip_h)),
            ("flip_vertical", json!(x.flip_v)),
        ],
    )
}

fn pairs<T>(items: &[(String, T)], f: impl Fn(&T) -> J) -> J {
    J::Array(items.iter().map(|(k, v)| json!([k, f(v)])).collect())
}

/// A geometry.
pub fn geometry(g: &Geometry) -> J {
    match g {
        Geometry::Preset { name, adjust } => class(
            "SourcePresetGeometry",
            vec![
                ("preset", json!(name)),
                ("adjust_values", json!({"{}": pairs(adjust, |v| num(*v))})),
                ("kind", json!("preset")),
            ],
        ),
        Geometry::Custom(paths) => {
            let paths = paths
                .iter()
                .map(|p| {
                    class(
                        "CustomGeometryPath",
                        vec![
                            ("width", num(p.width)),
                            ("height", num(p.height)),
                            ("commands", json!(p.commands)),
                        ],
                    )
                })
                .collect();
            class(
                "SourceCustomGeometry",
                vec![("paths", J::Array(paths)), ("kind", json!("custom"))],
            )
        }
    }
}

/// A path as written: `[fill, stroke, space, [[letter, operands...]...]]`.
pub fn spec_path(p: &SpecPath) -> J {
    let commands: Vec<J> = p
        .commands
        .iter()
        .map(|c| {
            let mut items = vec![json!(c.letter.to_string())];
            items.extend(c.operands.iter().map(|o| json!(o)));
            J::Array(items)
        })
        .collect();
    json!([
        p.fill,
        p.stroke,
        opt(p.space.as_ref(), |(w, h)| json!([num(*w), num(*h)])),
        commands
    ])
}

fn string_pairs(items: &[(String, String)]) -> J {
    J::Array(items.iter().map(|(k, v)| json!([k, v])).collect())
}

/// A geometry as written, as `parse_geometry_spec`'s tuple.
pub fn geometry_spec(g: &GeometrySpec) -> J {
    match g {
        GeometrySpec::Preset { name, adjust } => json!(["preset", name, {"{}": string_pairs(adjust)}]),
        GeometrySpec::Custom {
            adjustments,
            guides,
            paths,
        } => json!([
            "custom",
            [
                string_pairs(adjustments),
                string_pairs(guides),
                paths.iter().map(spec_path).collect::<Vec<_>>()
            ]
        ]),
    }
}

/// A format scheme.
pub fn format_scheme(f: &FormatScheme) -> J {
    class(
        "SourceFormatScheme",
        vec![
            ("fill_styles", J::Array(f.fill_styles.iter().map(fill).collect())),
            ("line_styles", J::Array(f.line_styles.iter().map(outline).collect())),
            (
                "effect_styles",
                J::Array(f.effect_styles.iter().map(|e| opt(e.as_ref(), effects)).collect()),
            ),
            (
                "bg_fill_styles",
                J::Array(f.background_fill_styles.iter().map(fill).collect()),
            ),
        ],
    )
}
