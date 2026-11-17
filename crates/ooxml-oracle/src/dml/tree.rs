//! Owns: text bodies and shape trees as JSON, in the form `python/drawingml_diff.py` writes
//! ooxml-common's (`crate::dml::model`'s conventions).
//!
//! Must not: be used by any library; it is a test instrument.

use super::model::{
    blip_effects, class, colour, effects, fill, geometry, geometry_spec, image_tile, num, nums, onum, opt, ostr,
    outline, shape_style, xfrm,
};
use ooxml_drawingml::text::{
    Autofit, BodyProperties, Bullet, ListStyle, Paragraph, ParagraphProperties, RunProperties, Spacing, TextBody,
};
use ooxml_drawingml::tree::{Node, NodeInfo, Placeholder, TableCell, TextRect};
use serde_json::{json, Value as J};

fn run_properties(r: &RunProperties) -> J {
    class(
        "SourceRunProperties",
        vec![
            ("bold", json!(r.bold)),
            ("italic", json!(r.italic)),
            ("underline", json!(r.underline)),
            ("underline_style", ostr(r.underline_style.as_ref())),
            ("strikethrough", json!(r.strike)),
            ("baseline", onum(r.baseline)),
            ("font_size", onum(r.size)),
            ("typeface", ostr(r.latin.as_ref())),
            ("typeface_ea", ostr(r.east_asian.as_ref())),
            ("typeface_cs", ostr(r.complex_script.as_ref())),
            ("color", opt(r.colour.as_ref(), colour)),
            ("highlight", opt(r.highlight.as_ref(), colour)),
            ("outline_width", onum(r.outline_width)),
            ("outline_color", opt(r.outline_colour.as_ref(), colour)),
            ("hyperlink_rel_id", ostr(r.hyperlink.as_ref())),
            ("hyperlink_tooltip", ostr(r.tooltip.as_ref())),
            ("lang", ostr(r.lang.as_ref())),
            ("alt_lang", ostr(r.alt_lang.as_ref())),
        ],
    )
}

fn spacing(s: &Spacing) -> J {
    match s {
        Spacing::Points(v) => class("PointsSpacing", vec![("value", num(*v)), ("type", json!("pts"))]),
        Spacing::Percent(v) => class("PercentSpacing", vec![("value", num(*v)), ("type", json!("pct"))]),
    }
}

fn bullet(b: &Bullet) -> J {
    match b {
        Bullet::None => class("NoBullet", vec![("type", json!("none"))]),
        Bullet::Char(c) => class("CharBullet", vec![("char", json!(c)), ("type", json!("char"))]),
        Bullet::AutoNumber { scheme, start } => class(
            "AutoNumBullet",
            vec![
                ("scheme", json!(scheme)),
                ("start_at", json!(start)),
                ("type", json!("autoNum")),
            ],
        ),
        Bullet::Picture(id) => class(
            "SourceBlipBullet",
            vec![("relationship_id", json!(id)), ("type", json!("blip"))],
        ),
    }
}

fn paragraph_properties(p: &ParagraphProperties) -> J {
    let tabs = |t: &Vec<ooxml_drawingml::text::TabStop>| {
        J::Array(
            t.iter()
                .map(|s| {
                    class(
                        "TabStop",
                        vec![("position", num(s.position)), ("alignment", json!(s.align))],
                    )
                })
                .collect(),
        )
    };
    class(
        "SourceParagraphProperties",
        vec![
            ("align", ostr(p.align.as_ref())),
            ("level", json!(p.level)),
            ("line_spacing", opt(p.line_spacing.as_ref(), spacing)),
            ("space_before", opt(p.space_before.as_ref(), spacing)),
            ("space_after", opt(p.space_after.as_ref(), spacing)),
            ("margin_left", onum(p.margin_left)),
            ("indent", onum(p.indent)),
            ("bullet", opt(p.bullet.as_ref(), bullet)),
            ("bullet_font", ostr(p.bullet_font.as_ref())),
            ("bullet_color", opt(p.bullet_colour.as_ref(), colour)),
            ("bullet_size_pct", onum(p.bullet_size_percent)),
            ("bullet_size_points", onum(p.bullet_size_points)),
            ("tab_stops", opt(p.tabs.as_ref(), tabs)),
            ("default_run_properties", opt(p.default_run.as_ref(), run_properties)),
        ],
    )
}

fn paragraph(p: &Paragraph) -> J {
    let runs = p
        .runs
        .iter()
        .map(|r| {
            class(
                "SourceTextRun",
                vec![
                    ("text", json!(r.text)),
                    ("properties", opt(r.properties.as_ref(), run_properties)),
                    ("field_type", ostr(r.field_type.as_ref())),
                ],
            )
        })
        .collect();
    class(
        "SourceParagraph",
        vec![
            ("runs", J::Array(runs)),
            ("properties", opt(p.properties.as_ref(), paragraph_properties)),
            ("end_para_run_properties", opt(p.end.as_ref(), run_properties)),
        ],
    )
}

fn list_style(s: &ListStyle) -> J {
    class(
        "SourceTextStyle",
        vec![
            ("default_paragraph", opt(s.default.as_ref(), paragraph_properties)),
            (
                "levels",
                J::Array(s.levels.iter().map(|l| opt(l.as_ref(), paragraph_properties)).collect()),
            ),
        ],
    )
}

fn body_properties(b: &BodyProperties) -> J {
    let autofit = b.autofit.map(|a| match a {
        Autofit::None => "noAutofit",
        Autofit::Normal => "normAutofit",
        Autofit::Shape => "spAutofit",
    });
    class(
        "SourceTextBodyProperties",
        vec![
            ("margin_left", onum(b.inset_left)),
            ("margin_right", onum(b.inset_right)),
            ("margin_top", onum(b.inset_top)),
            ("margin_bottom", onum(b.inset_bottom)),
            ("anchor", ostr(b.anchor.as_ref())),
            ("wrap", ostr(b.wrap.as_ref())),
            ("auto_fit", json!(autofit)),
            ("font_scale", onum(b.font_scale)),
            ("ln_spc_reduction", onum(b.line_reduction)),
            ("num_col", json!(b.columns)),
            ("vert", ostr(b.vert.as_ref())),
            ("rotation", onum(b.rotation)),
            ("default_tab_size", onum(b.tab_size)),
        ],
    )
}

/// A text body.
pub fn text_body(t: &TextBody) -> J {
    class(
        "SourceTextBody",
        vec![
            ("paragraphs", J::Array(t.paragraphs.iter().map(paragraph).collect())),
            ("properties", opt(t.properties.as_ref(), body_properties)),
            ("list_style", opt(t.list_style.as_ref(), list_style)),
        ],
    )
}

fn placeholder(p: &Placeholder) -> J {
    class(
        "SourcePlaceholder",
        vec![("type", ostr(p.kind.as_ref())), ("idx", json!(p.idx))],
    )
}

fn text_rect(t: &TextRect) -> J {
    json!([opt(t.geometry.as_ref(), geometry_spec), t.rect])
}

fn info(i: &NodeInfo) -> Vec<(&'static str, J)> {
    vec![
        ("name", ostr(i.name.as_ref())),
        ("shape_id", ostr(i.id.as_ref())),
        ("alt_text", ostr(i.alt_text.as_ref())),
        ("hidden", json!(i.hidden)),
    ]
}

fn cell(c: &TableCell) -> J {
    class(
        "SourceTableCell",
        vec![
            ("text_body", opt(c.text.as_ref(), text_body)),
            ("fill", opt(c.fill.as_ref(), fill)),
            ("border_top", opt(c.border_top.as_ref(), outline)),
            ("border_bottom", opt(c.border_bottom.as_ref(), outline)),
            ("border_left", opt(c.border_left.as_ref(), outline)),
            ("border_right", opt(c.border_right.as_ref(), outline)),
            ("grid_span", json!(c.grid_span)),
            ("row_span", json!(c.row_span)),
            ("h_merge", json!(c.h_merge)),
            ("v_merge", json!(c.v_merge)),
            ("margin_left", onum(c.margin_left)),
            ("margin_right", onum(c.margin_right)),
            ("margin_top", onum(c.margin_top)),
            ("margin_bottom", onum(c.margin_bottom)),
            ("anchor", ostr(c.anchor.as_ref())),
        ],
    )
}

fn shape(s: &ooxml_drawingml::tree::Shape) -> J {
    let mut f = info(&s.info);
    f.extend([
        ("placeholder", opt(s.placeholder.as_ref(), placeholder)),
        ("transform", opt(s.xfrm.as_ref(), xfrm)),
        ("geometry", opt(s.geometry.as_ref(), geometry)),
        ("fill", opt(s.fill.as_ref(), fill)),
        ("outline", opt(s.outline.as_ref(), outline)),
        ("effects", opt(s.effects.as_ref(), effects)),
        ("style", opt(s.style.as_ref(), shape_style)),
        ("text_body", opt(s.text.as_ref(), text_body)),
        ("text_transform", opt(s.text_xfrm.as_ref(), xfrm)),
        ("text_rect", opt(s.text_rect.as_ref(), text_rect)),
        ("hyperlink_rel_id", ostr(s.hyperlink.as_ref())),
        ("kind", json!("shape")),
    ]);
    class("SourceShape", f)
}

fn connector(c: &ooxml_drawingml::tree::Connector) -> J {
    let mut f = info(&c.info);
    f.extend([
        ("transform", opt(c.xfrm.as_ref(), xfrm)),
        ("geometry", opt(c.geometry.as_ref(), geometry)),
        ("outline", opt(c.outline.as_ref(), outline)),
        ("effects", opt(c.effects.as_ref(), effects)),
        ("style", opt(c.style.as_ref(), shape_style)),
        ("kind", json!("connector")),
    ]);
    class("SourceConnector", f)
}

fn picture(p: &ooxml_drawingml::tree::Picture) -> J {
    let mut f = info(&p.info);
    f.extend([
        ("blip_relationship_id", ostr(p.blip.as_ref())),
        ("svg_relationship_id", ostr(p.svg.as_ref())),
        ("placeholder", opt(p.placeholder.as_ref(), placeholder)),
        ("transform", opt(p.xfrm.as_ref(), xfrm)),
        ("geometry", opt(p.geometry.as_ref(), geometry)),
        ("outline", opt(p.outline.as_ref(), outline)),
        ("effects", opt(p.effects.as_ref(), effects)),
        ("blip_effects", opt(p.blip_effects.as_ref(), blip_effects)),
        ("src_rect", opt(p.src_rect.as_ref(), |r| nums(r))),
        ("stretch", opt(p.stretch.as_ref(), |r| nums(r))),
        ("tile", image_tile(p.tile.as_ref())),
        ("hyperlink_rel_id", ostr(p.hyperlink.as_ref())),
        ("kind", json!("image")),
    ]);
    class("SourceImage", f)
}

fn table(t: &ooxml_drawingml::tree::Table) -> J {
    let mut f = info(&t.info);
    let rows = t
        .rows
        .iter()
        .map(|r| {
            class(
                "SourceTableRow",
                vec![
                    ("height", num(r.height)),
                    ("cells", J::Array(r.cells.iter().map(cell).collect())),
                ],
            )
        })
        .collect();
    let g = t.regions;
    f.extend([
        ("transform", opt(t.xfrm.as_ref(), xfrm)),
        ("columns", nums(&t.columns)),
        ("rows", J::Array(rows)),
        ("first_row", json!(g.first_row)),
        ("last_row", json!(g.last_row)),
        ("first_col", json!(g.first_col)),
        ("last_col", json!(g.last_col)),
        ("band_row", json!(g.band_row)),
        ("band_col", json!(g.band_col)),
        ("style_id", ostr(t.style_id.as_ref())),
        ("kind", json!("table")),
    ]);
    class("SourceTable", f)
}

/// One member of a shape tree.
pub fn node(n: &Node) -> J {
    match n {
        Node::Shape(s) => shape(s),
        Node::Connector(c) => connector(c),
        Node::Picture(p) => picture(p),
        Node::Table(t) => table(t),
        Node::Group(g) => {
            let mut f = info(&g.info);
            f.extend([
                ("transform", opt(g.xfrm.as_ref(), xfrm)),
                ("child_transform", opt(g.child_xfrm.as_ref(), xfrm)),
                ("fill", opt(g.fill.as_ref(), fill)),
                ("effects", opt(g.effects.as_ref(), effects)),
                ("children", J::Array(g.children.iter().map(node).collect())),
                ("kind", json!("group")),
            ]);
            class("SourceGroup", f)
        }
        Node::Frame(u) => {
            let mut f = info(&u.info);
            f.extend([
                ("what", json!(u.what)),
                ("transform", opt(u.xfrm.as_ref(), xfrm)),
                ("fallback_rel_id", ostr(u.fallback.as_ref())),
                ("fallback_part", J::Null),
                ("kind", json!("unsupported")),
            ]);
            class("SourceUnsupported", f)
        }
    }
}
