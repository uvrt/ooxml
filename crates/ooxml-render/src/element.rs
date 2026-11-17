//! Owns: any drawable element as an element of the display list (ooxml-common's
//! `drawingml.elements.render_element`): what it is drawn as, what addresses it (alternative
//! text, `data-pptx-id`, `data-pptx-path`), its hyperlink; and a group's children mapped
//! from its child space -- a uniform scale kept as a `scale()` (text then laid out at its
//! authored size), a non-uniform one folded into each child's own box, as PowerPoint draws
//! a stretched group (no shear).
//!
//! Must not: decide how a shape, picture or table is drawn (`shape` does).

use crate::context::Painter;
use crate::shape::{render_connector, render_image, render_shape, render_table};
use crate::units::emu_to_px;
use ooxml_drawingml::drawable::{Element as Drawable, Group, Meta, Transform};
use ooxml_scene::markup::{Attr, Element, Op, Sep, Tag, Value};

/// How far apart a group's two scale factors may be and still count as one.
const UNIFORM_SCALE_TOLERANCE: f64 = 1e-4;

fn meta(element: &Drawable) -> &Meta {
    match element {
        Drawable::Shape(s) => &s.meta,
        Drawable::Connector(c) => &c.meta,
        Drawable::Image(i) => &i.meta,
        Drawable::Table(t) => &t.meta,
        Drawable::Group(g) => &g.meta,
    }
}

/// One element drawn: its own markup, what addresses it, and its hyperlink.
pub fn render_element(element: &Drawable, painter: &mut Painter<'_>) -> Element {
    let mut drawn = match element {
        Drawable::Shape(s) => render_shape(s, painter),
        Drawable::Connector(c) => render_connector(c, painter),
        Drawable::Image(i) => render_image(i, painter),
        Drawable::Table(t) => render_table(t, painter),
        Drawable::Group(g) => render_group(g, painter),
    };
    let m = meta(element);
    let mut attrs = Vec::new();
    let text = |name: &'static str, value: &str| Attr {
        name,
        value: Value::Text(value.to_owned()),
    };
    if let Some(alt) = m.alt_text.as_deref().filter(|a| !a.is_empty()) {
        attrs.push(text("role", "img"));
        attrs.push(text("aria-label", alt));
    }
    if let Some(id) = m.element_id.as_deref().filter(|i| !i.is_empty()) {
        attrs.push(text("data-pptx-id", id));
    }
    if let (Some(path), false) = (m.element_path.as_deref().filter(|p| !p.is_empty()), painter.edit_marks) {
        attrs.push(text("data-pptx-path", path));
    }
    if !attrs.is_empty() {
        drawn.prepend(attrs);
    }
    match (&m.hyperlink, painter.hyperlinks) {
        (Some(link), true) => Element::open(Tag::A)
            .with("href", Value::Text(link.url.clone()))
            .holding(vec![drawn]),
        _ => drawn,
    }
}

fn is_uniform(sx: f64, sy: f64) -> bool {
    (sx - sy).abs() <= UNIFORM_SCALE_TOLERANCE * sx.abs().max(sy.abs())
}

/// Whether an enclosing group's `x` factor lands on this child's height (measured: past 45
/// degrees, with one flip negating the angle the test is made on).
pub fn swaps_group_factors(t: &Transform) -> bool {
    let angle = if t.flip_h != t.flip_v { -t.rotation } else { t.rotation };
    let turned = angle.rem_euclid(180.0);
    (45.0..135.0).contains(&turned)
}

fn folded(t: &Transform, scale: (f64, f64), child: (f64, f64)) -> Transform {
    let (sx, sy) = scale;
    let (ws, hs) = if swaps_group_factors(t) { (sy, sx) } else { (sx, sy) };
    let width = t.extent_width * ws;
    let height = t.extent_height * hs;
    let cx = (t.offset_x + t.extent_width / 2.0 - child.0) * sx;
    let cy = (t.offset_y + t.extent_height / 2.0 - child.1) * sy;
    Transform {
        offset_x: cx - width / 2.0,
        offset_y: cy - height / 2.0,
        extent_width: width,
        extent_height: height,
        ..*t
    }
}

fn fold(child: &Drawable, scale: (f64, f64), offset: (f64, f64)) -> Option<Drawable> {
    let mut out = child.clone();
    match &mut out {
        Drawable::Shape(s) => {
            s.transform = folded(&s.transform, scale, offset);
            s.text_transform = s.text_transform.map(|t| folded(&t, scale, offset));
        }
        Drawable::Connector(c) => c.transform = folded(&c.transform, scale, offset),
        Drawable::Image(i) => i.transform = folded(&i.transform, scale, offset),
        Drawable::Group(g) => g.transform = folded(&g.transform, scale, offset),
        Drawable::Table(_) => return None,
    }
    Some(out)
}

fn scaled(children: &[Drawable], scale: (f64, f64), painter: &mut Painter<'_>) -> Vec<Element> {
    let outer = painter.group_scale;
    painter.group_scale = (outer.0 * scale.0, outer.1 * scale.1);
    let out = children.iter().map(|c| render_element(c, painter)).collect();
    painter.group_scale = outer;
    out
}

/// A group: its children's space mapped onto its box.
pub fn render_group(group: &Group, painter: &mut Painter<'_>) -> Element {
    let t = &group.transform;
    if t.rotation != 0.0 && !painter.turned {
        painter.turned = true;
        let out = render_group(group, painter);
        painter.turned = false;
        return out;
    }
    let (x, y, w, h) = (
        emu_to_px(t.offset_x),
        emu_to_px(t.offset_y),
        emu_to_px(t.extent_width),
        emu_to_px(t.extent_height),
    );
    let c = &group.child_transform;
    let (cw, ch, cx, cy) = (
        emu_to_px(c.extent_width),
        emu_to_px(c.extent_height),
        emu_to_px(c.offset_x),
        emu_to_px(c.offset_y),
    );
    let sx = if cw != 0.0 { w / cw } else { 1.0 };
    let sy = if ch != 0.0 { h / ch } else { 1.0 };
    let mut ops = vec![Op::Translate(x, y, Sep::CommaSpace)];
    if t.rotation != 0.0 {
        ops.push(Op::RotateAbout(t.rotation, w / 2.0, h / 2.0, Sep::CommaSpace));
    }
    if t.flip_h || t.flip_v {
        ops.push(Op::Translate(
            if t.flip_h { w } else { 0.0 },
            if t.flip_v { h } else { 0.0 },
            Sep::CommaSpace,
        ));
        ops.push(Op::Scale(
            if t.flip_h { -1.0 } else { 1.0 },
            if t.flip_v { -1.0 } else { 1.0 },
            Sep::CommaSpace,
        ));
    }
    let wrapper = vec![
        Op::Scale(sx, sy, Sep::CommaSpace),
        Op::Translate(-cx, -cy, Sep::CommaSpace),
    ];
    let out = Element::open(Tag::G);
    if is_uniform(sx, sy) {
        ops.extend(wrapper);
        let children = scaled(&group.children, (sx, sy), painter);
        return out.with("transform", Value::Transform(ops)).holding(children);
    }
    let mut rendered = Vec::new();
    for child in &group.children {
        match fold(child, (sx, sy), (c.offset_x, c.offset_y)) {
            Some(folded) => rendered.push(render_element(&folded, painter)),
            None => {
                let inner = scaled(std::slice::from_ref(child), (sx, sy), painter);
                if !inner.is_empty() {
                    rendered.push(
                        Element::open(Tag::G)
                            .with("transform", Value::Transform(wrapper.clone()))
                            .holding(inner),
                    );
                }
            }
        }
    }
    out.with("transform", Value::Transform(ops)).holding(rendered)
}
