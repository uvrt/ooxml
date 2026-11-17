//! Owns: ooxml-common's scene elements as its golden harness freezes them
//! (`tests/goldens.py`, `to_data`: each dataclass a dict led by its class name, fields at
//! their defaults left out, a tuple `{"()": [...]}`, a dict `{"{}": [[k, v], ...]}`) read
//! into `ooxml-drawingml`'s drawable elements.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_drawingml::drawable::{
    ArrowEnd, BlipEffects, BodyProperties, Bullet, CellBorders, Colour, CustomPath, Effects, Element, Fill, Geometry,
    Glow, GradientFill, GradientStop, Group, Hyperlink, Image, ImageTile, InnerShadow, Insets, Meta, OuterShadow,
    Outline, Paragraph, ParagraphProperties, RunProperties, Shape, Table, TableCell, TableRow, TextBody, TextOutline,
    TextRun, Transform,
};
use ooxml_drawingml::text::{Spacing, TabStop};
use serde_json::Value as J;

fn f(v: &J, key: &str, default: f64) -> f64 {
    v.get(key).and_then(J::as_f64).unwrap_or(default)
}

fn of(v: &J, key: &str) -> Option<f64> {
    v.get(key).and_then(J::as_f64)
}

fn b(v: &J, key: &str) -> bool {
    v.get(key).and_then(J::as_bool).unwrap_or(false)
}

fn s(v: &J, key: &str, default: &str) -> String {
    v.get(key).and_then(J::as_str).unwrap_or(default).to_owned()
}

fn os(v: &J, key: &str) -> Option<String> {
    v.get(key).and_then(J::as_str).map(str::to_owned)
}

fn obj<'a>(v: &'a J, key: &str) -> Option<&'a J> {
    v.get(key).filter(|x| !x.is_null())
}

fn list<'a>(v: &'a J, key: &str) -> &'a [J] {
    let value = v.get(key);
    let inner = value.and_then(|x| x.get("()")).or(value);
    inner.and_then(J::as_array).map_or(&[], Vec::as_slice)
}

fn class(v: &J) -> &str {
    v.get("@").and_then(J::as_str).unwrap_or("")
}

fn colour(v: &J) -> Colour {
    Colour {
        hex: s(v, "hex", "#000000"),
        alpha: f(v, "alpha", 1.0),
    }
}

fn ocolour(v: &J, key: &str) -> Option<Colour> {
    obj(v, key).map(colour)
}

fn transform(v: Option<&J>) -> Transform {
    let Some(v) = v else { return Transform::default() };
    Transform {
        offset_x: f(v, "offset_x", 0.0),
        offset_y: f(v, "offset_y", 0.0),
        extent_width: f(v, "extent_width", 0.0),
        extent_height: f(v, "extent_height", 0.0),
        rotation: f(v, "rotation", 0.0),
        flip_h: b(v, "flip_h"),
        flip_v: b(v, "flip_v"),
    }
}

fn geometry(v: &J) -> Geometry {
    if class(v) == "CustomGeometry" {
        return Geometry::Custom(
            list(v, "paths")
                .iter()
                .map(|p| CustomPath {
                    width: f(p, "width", 0.0),
                    height: f(p, "height", 0.0),
                    commands: s(p, "commands", ""),
                })
                .collect(),
        );
    }
    let adjust = v
        .get("adjust_values")
        .and_then(|a| a.get("{}"))
        .and_then(J::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| Some((pair.get(0)?.as_str()?.to_owned(), pair.get(1)?.as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    Geometry::Preset {
        name: s(v, "preset", "rect"),
        adjust,
    }
}

fn four(v: Option<&J>) -> Option<[f64; 4]> {
    let items = v?.get("()").or(v)?.as_array()?;
    let n = |i: usize| items.get(i).and_then(J::as_f64).unwrap_or(0.0);
    Some([n(0), n(1), n(2), n(3)])
}

fn tile(v: &J) -> ImageTile {
    ImageTile {
        tx: f(v, "tx", 0.0),
        ty: f(v, "ty", 0.0),
        sx: f(v, "sx", 1.0),
        sy: f(v, "sy", 1.0),
        flip: s(v, "flip", "none"),
        align: s(v, "align", "tl"),
    }
}

fn fill(v: &J) -> Option<Fill> {
    Some(match class(v) {
        "SolidFill" => Fill::Solid(colour(obj(v, "color")?)),
        "NoFill" => Fill::None,
        "GradientFill" => Fill::Gradient(GradientFill {
            stops: list(v, "stops")
                .iter()
                .map(|st| GradientStop {
                    position: f(st, "position", 0.0),
                    colour: obj(st, "color")
                        .map(colour)
                        .unwrap_or_else(|| Colour::opaque("#000000")),
                })
                .collect(),
            angle: f(v, "angle", 0.0),
            gradient_type: s(v, "gradient_type", "linear"),
            center_x: of(v, "center_x"),
            center_y: of(v, "center_y"),
            path: os(v, "path"),
            focus: four(obj(v, "focus")),
            scaled: v.get("scaled").and_then(J::as_bool),
            rotate_with_shape: v.get("rotate_with_shape").and_then(J::as_bool),
        }),
        "ImageFill" => Fill::Image {
            data: s(v, "image_data", ""),
            mime: s(v, "mime_type", ""),
            tile: obj(v, "tile").map(tile),
        },
        "PatternFill" => Fill::Pattern {
            preset: s(v, "preset", ""),
            foreground: colour(obj(v, "foreground_color")?),
            background: colour(obj(v, "background_color")?),
        },
        _ => return None,
    })
}

fn ofill(v: &J, key: &str) -> Option<Fill> {
    obj(v, key).and_then(fill)
}

fn arrow(v: &J) -> ArrowEnd {
    ArrowEnd {
        kind: s(v, "type", "none"),
        width: s(v, "width", "med"),
        length: s(v, "length", "med"),
    }
}

fn outline(v: &J) -> Outline {
    Outline {
        width: f(v, "width", 9525.0),
        fill: ofill(v, "fill"),
        dash_style: s(v, "dash_style", "solid"),
        custom_dash: v
            .get("custom_dash")
            .and_then(J::as_array)
            .map(|d| d.iter().filter_map(J::as_f64).collect()),
        line_cap: os(v, "line_cap"),
        line_join: os(v, "line_join"),
        head_end: obj(v, "head_end").map(arrow),
        tail_end: obj(v, "tail_end").map(arrow),
        compound: os(v, "compound"),
    }
}

fn ooutline(v: &J, key: &str) -> Option<Outline> {
    obj(v, key).map(outline)
}

fn effects(v: &J) -> Effects {
    Effects {
        outer_shadow: obj(v, "outer_shadow").map(|o| OuterShadow {
            blur_radius: f(o, "blur_radius", 0.0),
            distance: f(o, "distance", 0.0),
            direction: f(o, "direction", 0.0),
            colour: obj(o, "color").map(colour).unwrap_or_else(|| Colour::opaque("#000000")),
            alignment: s(o, "alignment", "b"),
            rotate_with_shape: o.get("rotate_with_shape").and_then(J::as_bool).unwrap_or(true),
        }),
        inner_shadow: obj(v, "inner_shadow").map(|o| InnerShadow {
            blur_radius: f(o, "blur_radius", 0.0),
            distance: f(o, "distance", 0.0),
            direction: f(o, "direction", 0.0),
            colour: obj(o, "color").map(colour).unwrap_or_else(|| Colour::opaque("#000000")),
        }),
        glow: obj(v, "glow").map(|g| Glow {
            radius: f(g, "radius", 0.0),
            colour: obj(g, "color").map(colour).unwrap_or_else(|| Colour::opaque("#000000")),
        }),
        soft_edge: obj(v, "soft_edge").map(|e| f(e, "radius", 0.0)),
    }
}

fn blip(v: &J) -> BlipEffects {
    let pair = |key: &str, a: &str, bk: &str| {
        obj(v, key).and_then(|x| Some((obj(x, a).map(colour)?, obj(x, bk).map(colour)?)))
    };
    BlipEffects {
        grayscale: b(v, "grayscale"),
        bi_level: obj(v, "bi_level").map(|x| f(x, "threshold", 0.0)),
        blur: obj(v, "blur").map(|x| (f(x, "radius", 0.0), x.get("grow").and_then(J::as_bool).unwrap_or(true))),
        lum: obj(v, "lum").map(|x| (f(x, "brightness", 0.0), f(x, "contrast", 0.0))),
        duotone: pair("duotone", "color1", "color2"),
        clr_change: pair("clr_change", "clr_from", "clr_to"),
        alpha: of(v, "alpha"),
    }
}

fn insets(v: &J) -> Insets {
    Insets {
        left: f(v, "left", 0.0),
        top: f(v, "top", 0.0),
        right: f(v, "right", 0.0),
        bottom: f(v, "bottom", 0.0),
    }
}

fn hyperlink(v: &J) -> Hyperlink {
    Hyperlink {
        url: s(v, "url", ""),
        tooltip: os(v, "tooltip"),
    }
}

fn run_properties(v: &J) -> RunProperties {
    RunProperties {
        font_size: of(v, "font_size"),
        font_family: os(v, "font_family"),
        font_family_ea: os(v, "font_family_ea"),
        font_family_cs: os(v, "font_family_cs"),
        bold: b(v, "bold"),
        italic: b(v, "italic"),
        underline: b(v, "underline"),
        underline_style: os(v, "underline_style"),
        strikethrough: b(v, "strikethrough"),
        colour: ocolour(v, "color"),
        baseline: f(v, "baseline", 0.0),
        hyperlink: obj(v, "hyperlink").map(hyperlink),
        outline: obj(v, "outline").map(|o| TextOutline {
            width: f(o, "width", 0.0),
            colour: obj(o, "color").map(colour).unwrap_or_else(|| Colour::opaque("#000000")),
        }),
        highlight: ocolour(v, "highlight"),
    }
}

fn spacing(v: &J) -> Spacing {
    if class(v) == "PointsSpacing" {
        Spacing::Points(f(v, "value", 0.0))
    } else {
        Spacing::Percent(f(v, "value", 0.0))
    }
}

fn bullet(v: &J) -> Bullet {
    match class(v) {
        "CharBullet" => Bullet::Char(s(v, "char", "")),
        "AutoNumBullet" => Bullet::AutoNumber {
            scheme: s(v, "scheme", "arabicPeriod"),
            start: v.get("start_at").and_then(J::as_i64).unwrap_or(1),
        },
        "BlipBullet" => Bullet::Picture {
            data: s(v, "image_data", ""),
            mime: s(v, "mime_type", ""),
        },
        _ => Bullet::None,
    }
}

fn paragraph_properties(v: Option<&J>) -> ParagraphProperties {
    let Some(v) = v else {
        return ParagraphProperties::default();
    };
    ParagraphProperties {
        alignment: os(v, "alignment"),
        line_spacing: obj(v, "line_spacing").map(spacing),
        space_before: obj(v, "space_before").map_or(Spacing::Percent(0.0), spacing),
        space_after: obj(v, "space_after").map_or(Spacing::Percent(0.0), spacing),
        level: v.get("level").and_then(J::as_i64).unwrap_or(0),
        bullet: obj(v, "bullet").map(bullet),
        bullet_font: os(v, "bullet_font"),
        bullet_colour: ocolour(v, "bullet_color"),
        bullet_size_percent: of(v, "bullet_size_pct"),
        bullet_size_points: of(v, "bullet_size_points"),
        margin_left: of(v, "margin_left"),
        indent: of(v, "indent"),
        tab_stops: list(v, "tab_stops")
            .iter()
            .map(|t| TabStop {
                position: f(t, "position", 0.0),
                align: s(t, "alignment", "l"),
            })
            .collect(),
    }
}

fn body_properties(v: Option<&J>) -> BodyProperties {
    let d = BodyProperties::default();
    let Some(v) = v else { return d };
    BodyProperties {
        anchor: s(v, "anchor", &d.anchor),
        margin_left: f(v, "margin_left", d.margin_left),
        margin_right: f(v, "margin_right", d.margin_right),
        margin_top: f(v, "margin_top", d.margin_top),
        margin_bottom: f(v, "margin_bottom", d.margin_bottom),
        wrap: s(v, "wrap", &d.wrap),
        auto_fit: s(v, "auto_fit", &d.auto_fit),
        font_scale: f(v, "font_scale", d.font_scale),
        line_reduction: f(v, "ln_spc_reduction", d.line_reduction),
        columns: v.get("num_col").and_then(J::as_i64).unwrap_or(d.columns),
        vert: s(v, "vert", &d.vert),
        rotation: f(v, "rotation", d.rotation),
        default_tab_size: f(v, "default_tab_size", d.default_tab_size),
    }
}

fn text_body(v: &J) -> TextBody {
    TextBody {
        paragraphs: list(v, "paragraphs")
            .iter()
            .map(|p| Paragraph {
                runs: list(p, "runs")
                    .iter()
                    .map(|r| TextRun {
                        text: s(r, "text", ""),
                        properties: obj(r, "properties").map(run_properties).unwrap_or_default(),
                    })
                    .collect(),
                properties: paragraph_properties(obj(p, "properties")),
                end_properties: obj(p, "end_para_run_properties").map(run_properties),
            })
            .collect(),
        properties: body_properties(obj(v, "body_properties")),
    }
}

fn meta(v: &J) -> Meta {
    Meta {
        alt_text: os(v, "alt_text"),
        hyperlink: obj(v, "hyperlink").map(hyperlink),
        element_id: os(v, "element_id"),
        element_path: os(v, "element_path"),
    }
}

fn table(v: &J) -> Table {
    let data = obj(v, "table").cloned().unwrap_or(J::Null);
    let borders = |c: &J| {
        obj(c, "borders").map(|x| CellBorders {
            top: ooutline(x, "top"),
            bottom: ooutline(x, "bottom"),
            left: ooutline(x, "left"),
            right: ooutline(x, "right"),
        })
    };
    Table {
        transform: transform(obj(v, "transform")),
        rows: list(&data, "rows")
            .iter()
            .map(|r| TableRow {
                height: f(r, "height", 0.0),
                cells: list(r, "cells")
                    .iter()
                    .map(|c| TableCell {
                        text: obj(c, "text_body").map(text_body),
                        fill: ofill(c, "fill"),
                        borders: borders(c),
                        grid_span: c.get("grid_span").and_then(J::as_i64).unwrap_or(1),
                        row_span: c.get("row_span").and_then(J::as_i64).unwrap_or(1),
                        h_merge: b(c, "h_merge"),
                        v_merge: b(c, "v_merge"),
                    })
                    .collect(),
            })
            .collect(),
        columns: list(&data, "columns").iter().map(|c| f(c, "width", 0.0)).collect(),
        background: ofill(&data, "background"),
        effects: obj(&data, "effects").map(effects),
        meta: meta(v),
    }
}

/// One frozen scene element as a drawable element (`None` for a kind not read here).
pub fn element(v: &J) -> Option<Element> {
    Some(match class(v) {
        "ShapeElement" => Element::Shape(Box::new(Shape {
            transform: transform(obj(v, "transform")),
            geometry: obj(v, "geometry").map(geometry).unwrap_or_default(),
            fill: ofill(v, "fill"),
            outline: ooutline(v, "outline"),
            text: obj(v, "text_body").map(text_body),
            text_transform: obj(v, "text_transform").map(|t| transform(Some(t))),
            text_rect: None,
            effects: obj(v, "effects").map(effects),
            meta: meta(v),
        })),
        "ConnectorElement" => Element::Connector(Box::new(ooxml_drawingml::drawable::Connector {
            transform: transform(obj(v, "transform")),
            geometry: obj(v, "geometry").map(geometry).unwrap_or_default(),
            outline: ooutline(v, "outline"),
            effects: obj(v, "effects").map(effects),
            meta: meta(v),
        })),
        "ImageElement" => Element::Image(Box::new(Image {
            transform: transform(obj(v, "transform")),
            data: s(v, "image_data", ""),
            mime: s(v, "mime_type", ""),
            effects: obj(v, "effects").map(effects),
            blip_effects: obj(v, "blip_effects").map(blip),
            src_rect: obj(v, "src_rect").map(insets),
            stretch: obj(v, "stretch").map(insets),
            tile: obj(v, "tile").map(tile),
            geometry: obj(v, "geometry").map(geometry),
            outline: ooutline(v, "outline"),
            meta: meta(v),
        })),
        "GroupElement" | "ChartElement" => Element::Group(Box::new(Group {
            transform: transform(obj(v, "transform")),
            child_transform: transform(obj(v, "child_transform")),
            children: list(v, "children").iter().filter_map(element).collect(),
            meta: meta(v),
        })),
        "TableElement" => Element::Table(Box::new(table(v))),
        _ => return None,
    })
}
