//! Owns: the typed SVG writer -- a [`Scene`] (or one node or resource of it) as SVG text,
//! attributes in one fixed order, numbers as [`num`] writes them, ids as
//! `{prefix}{letter}{n}`, images as `data:` URIs -- and its measure: how many characters
//! the same text has, without building it ([`Sink`]).
//!
//! Must not: parse SVG, or decide anything a scene does not say.

use crate::base64;
use crate::geometry::{Matrix, PathData, Segment};
use crate::model::{
    Anchor, Bounds, Cells, Content, Decoration, Def, Fill, Font, Group, GroupTransform, Image, LineCap, LineJoin, Node,
    Origin, Paint, Path, Rect, ResourceId, Rgb, Scene, Stop, StrokeAttr, Text, Viewport,
};
use crate::number::num;

/// Where the writer's text goes: a string, or a count of its characters.
pub trait Sink {
    /// Append `text`.
    fn put(&mut self, text: &str);
    /// Append `bytes` in base64.
    fn put_base64(&mut self, bytes: &[u8]);
}

impl Sink for String {
    fn put(&mut self, text: &str) {
        self.push_str(text);
    }

    fn put_base64(&mut self, bytes: &[u8]) {
        base64::encode_into(bytes, self);
    }
}

/// A [`Sink`] that counts characters (Unicode scalar values) instead of keeping them.
#[derive(Debug, Default, Clone, Copy)]
pub struct CharCount(pub usize);

impl Sink for CharCount {
    fn put(&mut self, text: &str) {
        self.0 += text.chars().count();
    }

    fn put_base64(&mut self, bytes: &[u8]) {
        self.0 += base64::encoded_len(bytes.len());
    }
}

/// The scene as an SVG document, UTF-8.
pub fn write_scene(scene: &Scene) -> Vec<u8> {
    let mut out = String::new();
    let root = &scene.root;
    out.put("<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\"");
    attr(&mut out, "width", &num(root.width));
    attr(&mut out, "height", &num(root.height));
    attr(&mut out, "viewBox", &bounds(&root.view_box));
    if root.stretch {
        out.put(" preserveAspectRatio=\"none\"");
    }
    if let Some(Origin::Metafile) = root.origin {
        out.put(" data-ooxml-metafile=\"gdi\"");
    }
    out.put(">");
    write_content(&mut out, &scene.content);
    out.put("</svg>");
    out.into_bytes()
}

/// The content's resources (in a `defs` element, when it has any) and drawing.
pub fn write_content<S: Sink>(out: &mut S, content: &Content) {
    let prefix = content.id_prefix.as_str();
    if !content.defs.is_empty() {
        out.put("<defs>");
        for def in &content.defs {
            write_def(out, def, prefix);
        }
        out.put("</defs>");
    }
    for node in &content.body {
        write_node(out, node, prefix);
    }
}

/// One drawing element, its ids written with `prefix`.
pub fn write_node<S: Sink>(out: &mut S, node: &Node, prefix: &str) {
    match node {
        Node::Path(path) => write_path(out, path, prefix),
        Node::Cells(cells) => write_cells(out, cells, prefix),
        Node::Rect(rect) => write_rect(out, rect, prefix),
        Node::Text(text) => write_text(out, text, prefix),
        Node::Image(image) => write_image(out, image),
        Node::Group(group) => write_group(out, group, prefix),
        Node::Viewport(viewport) => write_viewport(out, viewport),
    }
}

/// One resource, its ids written with `prefix`.
pub fn write_def<S: Sink>(out: &mut S, def: &Def, prefix: &str) {
    match def {
        Def::ClipPath { id, parent, paths } => {
            out.put("<clipPath");
            id_attr(out, *id, prefix);
            out.put(" clipPathUnits=\"userSpaceOnUse\"");
            if let Some(parent) = parent {
                url_attr(out, "clip-path", *parent, prefix);
            }
            out.put(">");
            paths.iter().for_each(|p| write_path(out, p, prefix));
            out.put("</clipPath>");
        }
        Def::Pattern {
            id,
            width,
            height,
            transform,
            children,
        } => {
            out.put("<pattern");
            id_attr(out, *id, prefix);
            out.put(" patternUnits=\"userSpaceOnUse\"");
            attr(out, "width", &num(*width));
            attr(out, "height", &num(*height));
            if let Some(m) = transform {
                attr(out, "patternTransform", &matrix(m));
            }
            out.put(">");
            children.iter().for_each(|n| write_node(out, n, prefix));
            out.put("</pattern>");
        }
        Def::LinearGradient { .. } => write_gradient(out, def, prefix),
        Def::Mask { id, region, paths } => {
            out.put("<mask");
            id_attr(out, *id, prefix);
            out.put(" maskUnits=\"userSpaceOnUse\"");
            attr(out, "x", &num(region.x));
            attr(out, "y", &num(region.y));
            attr(out, "width", &num(region.width));
            attr(out, "height", &num(region.height));
            out.put(">");
            paths.iter().for_each(|p| write_path(out, p, prefix));
            out.put("</mask>");
        }
    }
}

fn write_gradient<S: Sink>(out: &mut S, def: &Def, prefix: &str) {
    let Def::LinearGradient {
        id,
        x1,
        y1,
        x2,
        y2,
        transform,
        stops,
    } = def
    else {
        return;
    };
    out.put("<linearGradient");
    id_attr(out, *id, prefix);
    out.put(" gradientUnits=\"userSpaceOnUse\"");
    attr(out, "x1", &num(*x1));
    attr(out, "y1", &num(*y1));
    attr(out, "x2", &num(*x2));
    attr(out, "y2", &num(*y2));
    if let Some(m) = transform {
        attr(out, "gradientTransform", &matrix(m));
    }
    out.put(">");
    for Stop { offset, color, opacity } in stops {
        out.put("<stop");
        attr(out, "offset", &num(*offset));
        attr(out, "stop-color", &rgb(*color));
        if let Some(o) = opacity {
            attr(out, "stop-opacity", &num(*o));
        }
        out.put("/>");
    }
    out.put("</linearGradient>");
}

fn write_path<S: Sink>(out: &mut S, path: &Path, prefix: &str) {
    out.put("<path d=\"");
    path_data(out, &path.data);
    out.put("\"");
    if let Some(m) = &path.transform {
        attr(out, "transform", &matrix(m));
    }
    if let Some(fill) = &path.fill {
        fill_attrs(out, fill, prefix);
    }
    if path.clip_even_odd {
        out.put(" clip-rule=\"evenodd\"");
    }
    for a in &path.stroke {
        stroke_attr(out, a, prefix);
    }
    if let Some(clip) = path.clip {
        url_attr(out, "clip-path", clip, prefix);
    }
    out.put("/>");
}

fn write_cells<S: Sink>(out: &mut S, cells: &Cells, prefix: &str) {
    out.put("<path d=\"");
    for &(x, y, w, h) in &cells.cells {
        out.put(&format!("M{} {}h{}v{}h-{}z", num(x), num(y), num(w), num(h), num(w)));
    }
    out.put("\"");
    fill_attrs(out, &cells.fill, prefix);
    out.put("/>");
}

fn write_rect<S: Sink>(out: &mut S, rect: &Rect, prefix: &str) {
    out.put("<rect");
    attr(out, "x", &num(rect.x));
    attr(out, "y", &num(rect.y));
    attr(out, "width", &num(rect.width));
    attr(out, "height", &num(rect.height));
    fill_attrs(out, &rect.fill, prefix);
    out.put("/>");
}

fn write_text<S: Sink>(out: &mut S, text: &Text, prefix: &str) {
    out.put("<text");
    attr(out, "x", &list(&text.x));
    attr(out, "y", &list(&text.y));
    if let Some(m) = &text.transform {
        attr(out, "transform", &matrix(m));
    }
    fill_attrs(out, &text.fill, prefix);
    font_attrs(out, &text.font);
    match text.anchor {
        Some(Anchor::Middle) => out.put(" text-anchor=\"middle\""),
        Some(Anchor::End) => out.put(" text-anchor=\"end\""),
        None => {}
    }
    out.put(" xml:space=\"preserve\">");
    out.put(&escape(&text.content, false));
    out.put("</text>");
}

fn font_attrs<S: Sink>(out: &mut S, font: &Font) {
    attr(out, "font-size", &num(font.size));
    if let Some(family) = &font.family {
        attr(out, "font-family", &escape(family, true));
    }
    if font.bold {
        out.put(" font-weight=\"bold\"");
    }
    if font.italic {
        out.put(" font-style=\"italic\"");
    }
    if !font.decorations.is_empty() {
        let words: Vec<&str> = font
            .decorations
            .iter()
            .map(|d| match d {
                Decoration::Underline => "underline",
                Decoration::LineThrough => "line-through",
            })
            .collect();
        attr(out, "text-decoration", &words.join(" "));
    }
}

fn write_image<S: Sink>(out: &mut S, image: &Image) {
    out.put("<image");
    if let Some(x) = image.x {
        attr(out, "x", &num(x));
    }
    if let Some(y) = image.y {
        attr(out, "y", &num(y));
    }
    attr(out, "width", &num(image.width));
    attr(out, "height", &num(image.height));
    if image.stretch {
        out.put(" preserveAspectRatio=\"none\"");
    }
    if image.pixelated {
        out.put(" image-rendering=\"optimizeSpeed\" style=\"image-rendering:pixelated\"");
    }
    out.put(" href=\"data:");
    out.put(&image.data.mime);
    out.put(";base64,");
    out.put_base64(&image.data.bytes);
    out.put("\"/>");
}

fn write_group<S: Sink>(out: &mut S, group: &Group, prefix: &str) {
    out.put("<g");
    match &group.transform {
        Some(GroupTransform::Matrix(m)) => attr(out, "transform", &matrix(m)),
        Some(GroupTransform::TranslateRotate { x, y, angle }) => {
            let mut value = format!("translate({} {})", num(*x), num(*y));
            if let Some(a) = angle {
                value.push_str(&format!(" rotate({})", num(*a)));
            }
            attr(out, "transform", &value);
        }
        None => {}
    }
    if let Some(o) = group.opacity {
        attr(out, "opacity", &num(o));
    }
    if let Some(clip) = group.clip {
        url_attr(out, "clip-path", clip, prefix);
    }
    if let Some(mask) = group.mask {
        url_attr(out, "mask", mask, prefix);
    }
    out.put(">");
    group.children.iter().for_each(|n| write_node(out, n, prefix));
    out.put("</g>");
}

fn write_viewport<S: Sink>(out: &mut S, viewport: &Viewport) {
    out.put("<svg");
    if let Some(x) = viewport.x {
        attr(out, "x", &num(x));
    }
    if let Some(y) = viewport.y {
        attr(out, "y", &num(y));
    }
    attr(out, "width", &num(viewport.width));
    attr(out, "height", &num(viewport.height));
    attr(out, "viewBox", &bounds(&viewport.view_box));
    out.put(" preserveAspectRatio=\"none\"");
    if viewport.clipped {
        out.put(" overflow=\"hidden\"");
    }
    out.put(">");
    write_content(out, &viewport.content);
    out.put("</svg>");
}

fn fill_attrs<S: Sink>(out: &mut S, fill: &Fill, prefix: &str) {
    paint_attr(out, "fill", fill.paint, prefix);
    if let Some(o) = fill.opacity {
        attr(out, "fill-opacity", &num(o));
    }
    if fill.even_odd {
        out.put(" fill-rule=\"evenodd\"");
    }
}

fn stroke_attr<S: Sink>(out: &mut S, a: &StrokeAttr, prefix: &str) {
    match a {
        StrokeAttr::Paint(p) => paint_attr(out, "stroke", *p, prefix),
        StrokeAttr::Opacity(v) => attr(out, "stroke-opacity", &num(*v)),
        StrokeAttr::Width(v) => attr(out, "stroke-width", &num(*v)),
        StrokeAttr::Cap(cap) => attr(
            out,
            "stroke-linecap",
            match cap {
                LineCap::Butt => "butt",
                LineCap::Round => "round",
                LineCap::Square => "square",
            },
        ),
        StrokeAttr::Join(join) => attr(
            out,
            "stroke-linejoin",
            match join {
                LineJoin::Miter => "miter",
                LineJoin::MiterClip => "miter-clip",
                LineJoin::Round => "round",
                LineJoin::Bevel => "bevel",
            },
        ),
        StrokeAttr::MiterLimit(v) => attr(out, "stroke-miterlimit", &num(*v)),
        StrokeAttr::Dashes(v) => attr(out, "stroke-dasharray", &list(v)),
        StrokeAttr::DashOffset(v) => attr(out, "stroke-dashoffset", &num(*v)),
        StrokeAttr::InertDashes(v) => attr(out, "data-d", &list(v)),
        StrokeAttr::InertDashOffset(v) => attr(out, "data-o", &num(*v)),
    }
}

fn paint_attr<S: Sink>(out: &mut S, name: &str, paint: Paint, prefix: &str) {
    match paint {
        Paint::None => attr(out, name, "none"),
        Paint::Color(c) => attr(out, name, &rgb(c)),
        Paint::Server(id) => url_attr(out, name, id, prefix),
    }
}

/// `d`'s value.
pub fn path_data<S: Sink>(out: &mut S, data: &PathData) {
    for fig in &data.figures {
        out.put(&format!("M{} {}", num(fig.start.0), num(fig.start.1)));
        for seg in &fig.segments {
            match *seg {
                Segment::Line(x, y) => out.put(&format!("L{} {}", num(x), num(y))),
                Segment::Cubic(a, b, c, d, e, f) => out.put(&format!("C{}", list(&[a, b, c, d, e, f]))),
            }
        }
        if fig.closed {
            out.put("Z");
        }
    }
}

fn attr<S: Sink>(out: &mut S, name: &str, value: &str) {
    out.put(" ");
    out.put(name);
    out.put("=\"");
    out.put(value);
    out.put("\"");
}

fn id_attr<S: Sink>(out: &mut S, id: ResourceId, prefix: &str) {
    attr(out, "id", &id_text(id, prefix));
}

fn url_attr<S: Sink>(out: &mut S, name: &str, id: ResourceId, prefix: &str) {
    attr(out, name, &format!("url(#{})", id_text(id, prefix)));
}

/// An id as written: `{prefix}{letter}{n}`.
pub fn id_text(id: ResourceId, prefix: &str) -> String {
    format!("{prefix}{}{}", id.kind.letter(), id.n)
}

fn matrix(m: &Matrix) -> String {
    format!("matrix({})", list(&m.values()))
}

fn bounds(b: &Bounds) -> String {
    list(&[b.x, b.y, b.width, b.height])
}

fn list(values: &[f64]) -> String {
    values.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

/// A colour as `#RRGGBB`, upper case.
pub fn rgb(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// `&`, `<` and `>` as entities, and `"` too in an attribute value.
pub fn escape(text: &str, quote: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if quote => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}
