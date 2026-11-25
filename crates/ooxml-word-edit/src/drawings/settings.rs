//! Owns: a floating drawing's settings written into its frame as docx-agent's
//! `_apply_drawing` writes them -- wrapping (with Word's polygon and `behindDoc`), the
//! offsets or alignments across and down and what they measure from, the z-order, the
//! size (the shape's own transform with it; a floating one's `wp14:sizeRelH`/`sizeRelV`) --
//! and the VML fallback brought in line (`_sync_fallback_style`, `_sync_fallback_wrap`).
//!
//! Must not: read the document (the heights it orders against are given), or write
//! outside the frame and its fallback.

use super::{points_text, python_int, A, FIRST_HEIGHT, HEIGHT_STEP, POLYGON, V, W10, WPS};
use crate::marks::insert_in_order;
use crate::names::{MC, W, WP, WP14};
use crate::units::Emu;
use crate::validate::Orders;
use crate::work::{Id, Work};
use ooxml_xml::QName;
use std::sync::Arc;

/// DrawingML pictures.
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
/// Word 2010's groups.
const WPG: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup";

/// A z-order change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrder {
    /// A step above the highest.
    Front,
    /// 1025 below the lowest (measured).
    Back,
    /// Just above the next higher.
    Forward,
    /// Just below the next lower.
    Backward,
}

/// The settings a call gives a drawing (docx-agent's `set_drawing` keywords the tool
/// passes); `None` leaves one as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawingValues {
    /// `square`, `tight`, `through`, `top_and_bottom`, `front`, `none` or `behind`.
    pub wrap: Option<String>,
    /// Across.
    pub x: Option<Emu>,
    /// Down.
    pub y: Option<Emu>,
    /// Across, an alignment instead of `x`.
    pub x_align: Option<String>,
    /// What `x` and `x_align` measure from.
    pub horizontal_from: Option<String>,
    /// The z-order.
    pub z_order: Option<ZOrder>,
    /// One alone keeps the aspect ratio.
    pub width: Option<Emu>,
    /// The height.
    pub height: Option<Emu>,
}

impl DrawingValues {
    /// Whether any setting is given.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == DrawingValues::default()
    }

    /// Whether a setting other than the size is given (an inline drawing takes only that).
    #[must_use]
    pub fn places(&self) -> bool {
        DrawingValues {
            width: None,
            height: None,
            ..self.clone()
        } != DrawingValues::default()
    }
}

/// The wrap element Word writes for each kind.
pub(crate) fn wrap_tag(kind: &str) -> &'static str {
    match kind {
        "square" => "wrapSquare",
        "tight" => "wrapTight",
        "through" => "wrapThrough",
        "top_and_bottom" => "wrapTopAndBottom",
        _ => "wrapNone",
    }
}

/// A frame's wrapping kind and side as docx-agent's `read_wrap` reads them.
fn read_wrap(work: &Work, frame: Id) -> (String, Option<&'static str>) {
    let wrap = work.children(frame).iter().copied().find(|c| {
        work.name(*c)
            .is_some_and(|n| n.namespace() == Some(WP) && n.local().starts_with("wrap"))
    });
    let kind = match wrap.and_then(|w| work.name(w)).map(|n| n.local().to_owned()).as_deref() {
        Some("wrapSquare") => "square",
        Some("wrapTight") => "tight",
        Some("wrapThrough") => "through",
        Some("wrapTopAndBottom") => "top_and_bottom",
        _ => "front",
    };
    let kind = if kind == "front" && matches!(work.get(frame, None, "behindDoc"), Some("1" | "true")) {
        "behind"
    } else {
        kind
    };
    let side = wrap.and_then(|w| work.get(w, None, "wrapText")).and_then(|s| match s {
        "bothSides" => Some("both"),
        "left" => Some("left"),
        "right" => Some("right"),
        "largest" => Some("largest"),
        _ => None,
    });
    (kind.to_owned(), side)
}

fn side_value(side: &str) -> &'static str {
    match side {
        "left" => "left",
        "right" => "right",
        "largest" => "largest",
        _ => "bothSides",
    }
}

fn plain(name: &str) -> Arc<QName> {
    Arc::new(QName::new(None, name, None))
}

/// A new `wp:<local>` element (`wp14:` with `wp14`), detached, its attributes unqualified.
pub(crate) fn make_wp(work: &mut Work, frame: Id, local: &str, attributes: &[(&str, &str)]) -> Id {
    let element = work.new_element(work.element_name(frame, WP, local, "wp"));
    for (name, value) in attributes {
        work.set(element, plain(name), value);
    }
    element
}

fn make_wp14(work: &mut Work, frame: Id, local: &str, attributes: &[(&str, &str)]) -> Id {
    let element = work.new_element(work.element_name(frame, WP14, local, "wp14"));
    for (name, value) in attributes {
        work.set(element, plain(name), value);
    }
    element
}

/// Write `values` into `frame` (docx-agent's `_apply_drawing`); `heights` every floating
/// drawing's in the document.
pub(crate) fn apply(work: &mut Work, frame: Id, values: &DrawingValues, heights: &[i64], orders: &Orders) {
    if let Some(kind) = &values.wrap {
        set_wrap(work, frame, kind, orders);
    }
    for (axis, tag, offset, align, origin) in [
        (
            "x",
            "positionH",
            values.x,
            values.x_align.as_deref(),
            values.horizontal_from.as_deref(),
        ),
        ("y", "positionV", values.y, None, None),
    ] {
        if offset.is_some() || align.is_some() || origin.is_some() {
            set_position(work, frame, (axis, tag), (offset, align, origin), orders);
        }
    }
    if let Some(order) = values.z_order {
        set_order(work, frame, order, heights);
    }
    if values.width.is_some() || values.height.is_some() {
        resize(work, frame, values, orders);
    }
    if work.is(frame, WP, "anchor") {
        sync_fallback_style(work, frame);
    }
}

/// The wrapping `kind` (its side kept), Word's polygon for tight and through.
fn set_wrap(work: &mut Work, frame: Id, kind: &str, orders: &Orders) {
    let (_, side) = read_wrap(work, frame);
    let side = side.unwrap_or("both");
    let old: Vec<Id> = work
        .children(frame)
        .iter()
        .copied()
        .filter(|c| {
            work.name(*c)
                .is_some_and(|n| n.namespace() == Some(WP) && n.local().starts_with("wrap"))
        })
        .collect();
    for child in old {
        work.remove(child);
    }
    let tag = wrap_tag(kind);
    let node = make_wp(work, frame, tag, &[]);
    if matches!(tag, "wrapSquare" | "wrapTight" | "wrapThrough") {
        work.set(node, plain("wrapText"), side_value(side));
    }
    if matches!(tag, "wrapTight" | "wrapThrough") {
        let polygon = make_wp(work, frame, "wrapPolygon", &[("edited", "0")]);
        for (k, (x, y)) in POLYGON.iter().enumerate() {
            let local = if k == 0 { "start" } else { "lineTo" };
            let point = make_wp(work, frame, local, &[("x", &x.to_string()), ("y", &y.to_string())]);
            work.append(polygon, point);
        }
        work.append(node, polygon);
    }
    insert_in_order(work, frame, node, orders);
    work.set(frame, plain("behindDoc"), if kind == "behind" { "1" } else { "0" });
    sync_fallback_wrap(work, frame, kind, side);
}

/// One axis's place: an offset or an alignment, against `origin` when given.
fn set_position(
    work: &mut Work,
    frame: Id,
    (axis, tag): (&str, &str),
    (offset, align, origin): (Option<Emu>, Option<&str>, Option<&str>),
    orders: &Orders,
) {
    let node = match work.find(frame, WP, tag) {
        Some(node) => node,
        None => {
            let from = if axis == "x" { "column" } else { "paragraph" };
            let node = make_wp(work, frame, tag, &[("relativeFrom", from)]);
            insert_in_order(work, frame, node, orders);
            node
        }
    };
    if let Some(origin) = origin {
        work.set(node, plain("relativeFrom"), origin);
    }
    if offset.is_none() && align.is_none() {
        return;
    }
    for child in work.children(node).to_vec() {
        work.remove(child);
    }
    let child = match align {
        Some(align) => {
            let child = make_wp(work, frame, "align", &[]);
            work.set_text(child, align);
            child
        }
        None => {
            let child = make_wp(work, frame, "posOffset", &[]);
            work.set_text(child, &offset.unwrap_or_default().0.to_string());
            child
        }
    };
    work.append(node, child);
}

/// The z-order: against every other drawing's height (`heights`, the document's).
fn set_order(work: &mut Work, frame: Id, order: ZOrder, heights: &[i64]) {
    let own = work.get(frame, None, "relativeHeight").and_then(python_int);
    let mut others: Vec<i64> = heights.iter().copied().filter(|h| *h != own.unwrap_or(0)).collect();
    others.sort_unstable();
    let mine = own.unwrap_or(FIRST_HEIGHT);
    let value = match order {
        ZOrder::Front => others.iter().copied().chain([mine - HEIGHT_STEP]).max().unwrap_or(mine) + HEIGHT_STEP,
        ZOrder::Back => others.iter().copied().chain([mine]).min().unwrap_or(mine) - HEIGHT_STEP - 1,
        ZOrder::Forward => others.iter().copied().find(|h| *h > mine).map_or(mine, |h| h + 1),
        ZOrder::Backward => others.iter().copied().rev().find(|h| *h < mine).map_or(mine, |h| h - 1),
    };
    work.set(frame, plain("relativeHeight"), &value.max(0).to_string());
}

/// The new size: one dimension alone keeps the aspect ratio.
fn resize(work: &mut Work, frame: Id, values: &DrawingValues, orders: &Orders) {
    let Some(extent) = work.find(frame, WP, "extent") else {
        return;
    };
    let read = |k: &str| work.get(extent, None, k).and_then(python_int).unwrap_or(0);
    let (cx, cy) = (read("cx"), read("cy"));
    let mut width = values.width.map(|w| w.0);
    let mut height = values.height.map(|h| h.0);
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let scaled = |a: i64, b: i64, c: i64| ((a as f64) * (b as f64) / (c as f64)).round_ties_even() as i64;
    if width.is_none() {
        let h = height.unwrap_or(cy);
        width = Some(if cy != 0 { scaled(cx, h, cy) } else { cx });
    }
    let w = width.unwrap_or(cx);
    if height.is_none() {
        height = Some(if cx != 0 { scaled(cy, w, cx) } else { cy });
    }
    let h = height.unwrap_or(cy);
    for node in [Some(extent), xfrm_ext(work, frame)].into_iter().flatten() {
        work.set(node, plain("cx"), &w.to_string());
        work.set(node, plain("cy"), &h.to_string());
    }
    if work.is(frame, WP, "anchor") {
        for (tag, child) in [("sizeRelH", "pctWidth"), ("sizeRelV", "pctHeight")] {
            if work.find(frame, WP14, tag).is_some() {
                continue;
            }
            let node = make_wp14(work, frame, tag, &[("relativeFrom", "margin")]);
            let value = make_wp14(work, frame, child, &[]);
            work.set_text(value, "0");
            work.append(node, value);
            insert_in_order(work, frame, node, orders);
        }
    }
}

/// The `a:ext` of the shape's own transform (a picture's, a shape's, a group's).
fn xfrm_ext(work: &Work, frame: Id) -> Option<Id> {
    let data = work
        .find(frame, A, "graphic")
        .and_then(|g| work.find(g, A, "graphicData"))?;
    let shape = work.children(data).iter().copied().find(|c| work.is_element(*c))?;
    let in_properties = |properties: Id| {
        let xfrm = work.find(properties, A, "xfrm")?;
        work.find(xfrm, A, "ext")
    };
    // a picture's spPr anywhere below it, then a shape's, then a group's
    let picture = work
        .descendants(shape)
        .into_iter()
        .filter(|n| work.is(*n, PIC, "spPr"))
        .find_map(in_properties);
    picture
        .or_else(|| work.find(shape, WPS, "spPr").and_then(in_properties))
        .or_else(|| work.find(shape, WPG, "grpSpPr").and_then(in_properties))
}

/// The `mc:AlternateContent` whose choice holds the frame, if any.
pub(crate) fn alternate_content(work: &Work, frame: Id) -> Option<Id> {
    let mut node = work.parent(frame);
    while let Some(n) = node {
        if work.is(n, W, "r") {
            return None;
        }
        if work.is(n, MC, "AlternateContent") {
            return Some(n);
        }
        node = work.parent(n);
    }
    None
}

/// The VML shape of the frame's fallback (`v:shape`, `v:rect`, `v:group`...).
fn fallback_shape(work: &Work, frame: Id) -> Option<Id> {
    let holder = alternate_content(work, frame)?;
    let fallback = work.find(holder, MC, "Fallback")?;
    let pict = work.find(fallback, W, "pict")?;
    work.children(pict).iter().copied().find(|c| {
        work.name(*c)
            .is_some_and(|n| n.namespace() == Some(V) && n.local() != "shapetype")
    })
}

fn style_of(work: &Work, shape: Id) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for item in work.get(shape, None, "style").unwrap_or("").split(';') {
        let (key, value) = item.split_once(':').unwrap_or((item, ""));
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        put(&mut out, key, value.trim().to_owned());
    }
    out
}

fn put(style: &mut Vec<(String, String)>, key: &str, value: String) {
    match style.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value,
        None => style.push((key.to_owned(), value)),
    }
}

/// Bring a fallback shape's size, place and order in line with the drawing.
fn sync_fallback_style(work: &mut Work, frame: Id) {
    let Some(shape) = fallback_shape(work, frame) else {
        return;
    };
    let mut style = style_of(work, shape);
    let points = |text: Option<&str>| {
        text.and_then(python_int).map_or(0.0, |v| {
            #[allow(clippy::cast_precision_loss)]
            let v = v as f64;
            v / super::EMU
        })
    };
    if let Some(extent) = work.find(frame, WP, "extent") {
        put(&mut style, "width", points_text(points(work.get(extent, None, "cx"))));
        put(&mut style, "height", points_text(points(work.get(extent, None, "cy"))));
    }
    let z = work
        .get(frame, None, "relativeHeight")
        .filter(|z| !z.is_empty())
        .map(str::to_owned)
        .or_else(|| style.iter().find(|(k, _)| k == "z-index").map(|(_, v)| v.clone()))
        .unwrap_or_else(|| "0".to_owned());
    put(&mut style, "z-index", z);
    for (axis, tag, margin) in [
        ("horizontal", "positionH", "margin-left"),
        ("vertical", "positionV", "margin-top"),
    ] {
        let Some(node) = work.find(frame, WP, tag) else {
            continue;
        };
        let origin = work.get(node, None, "relativeFrom").unwrap_or("").to_owned();
        let relative = match origin.as_str() {
            "column" | "paragraph" => "text".to_owned(),
            "character" => "char".to_owned(),
            "line" => "line".to_owned(),
            o if o.ends_with("Margin") => o.replace("Margin", "-margin-area"),
            o => o.to_owned(),
        };
        put(&mut style, &format!("mso-position-{axis}-relative"), relative);
        if let Some(offset) = work.find(node, WP, "posOffset") {
            let value = points(Some(work.text(offset)));
            put(&mut style, margin, points_text(value));
            put(&mut style, &format!("mso-position-{axis}"), "absolute".to_owned());
        } else if let Some(align) = work.find(node, WP, "align") {
            put(&mut style, &format!("mso-position-{axis}"), work.text(align).to_owned());
        }
    }
    for (key, attribute) in [
        ("mso-wrap-distance-left", "distL"),
        ("mso-wrap-distance-right", "distR"),
        ("mso-wrap-distance-top", "distT"),
        ("mso-wrap-distance-bottom", "distB"),
    ] {
        if let Some(value) = work.get(frame, None, attribute) {
            let value = points(Some(value));
            put(&mut style, key, points_text(value));
        }
    }
    let text: Vec<String> = style.iter().map(|(k, v)| format!("{k}:{v}")).collect();
    work.set(shape, plain("style"), &text.join(";"));
}

fn sync_fallback_wrap(work: &mut Work, frame: Id, kind: &str, side: &str) {
    let Some(shape) = fallback_shape(work, frame) else {
        return;
    };
    for node in work.children(shape).to_vec() {
        if work.is(node, W10, "wrap") {
            work.remove(node);
        }
    }
    let vml = match kind {
        "square" => "square",
        "tight" => "tight",
        "through" => "through",
        "top_and_bottom" => "topAndBottom",
        _ => return,
    };
    let node = work.new_element(work.element_name(shape, W10, "wrap", "w10"));
    work.set(node, plain("type"), vml);
    if side != "both" {
        work.set(node, plain("side"), side);
    }
    work.append(shape, node);
}
