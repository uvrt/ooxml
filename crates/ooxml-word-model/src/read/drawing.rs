//! Owns: a `w:drawing` as layout reads it -- docx2svg's `parse.drawing` and the drawing
//! parts of its document reader: an inline drawing's mark (`drawing:cx:cy:l:t:r:b`, its
//! extent and effect extent, EMU), what it shows (a picture by relationship, or a shape,
//! group, chart or SmartArt), a floating drawing's anchor as written (position, extent,
//! wrap, stacking), and a text box's blocks. Word's `mc:AlternateContent` choice is the
//! DrawingML one.
//!
//! Must not: read how a shape is painted -- geometry, fill, outline, effects, a picture's
//! crop -- which its painter reads from the element kept as the graphic's `source`.

use super::fields::Fields;
use super::Reader;
use crate::model::{Anchor, AnchorPosition, BodyValue, Graphic, InlineObject};
use crate::value::{text, Text};
use crate::w;
use crate::xmlread::{attr, attr_int, child, children, elements, local, text as element_text};
use ooxml_xml::Element;
use std::collections::BTreeMap;
use std::sync::Arc;

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const DRAWN_EFFECTS: &[&str] = &["outerShdw", "innerShdw", "glow", "softEdge"];

/// The `w:drawing` a run's object is: itself, or an `mc:AlternateContent`'s first choice
/// holding one.
pub(crate) fn drawing_element(node: &Element) -> Option<&Element> {
    match local(node) {
        "drawing" => Some(node),
        "AlternateContent" => children(node, "Choice").find_map(|c| child(c, "drawing")),
        _ => None,
    }
}

/// The mark an inline object leaves in its run: `drawing:cx:cy:l:t:r:b` for a drawing with
/// one `wp:inline`, `drawing` for anything else.
pub(crate) fn object_mark(node: &Element) -> String {
    let Some(drawing) = drawing_element(node) else {
        return "drawing".into();
    };
    let inline: Vec<&Element> = children(drawing, "inline").collect();
    let [only] = inline.as_slice() else {
        return "drawing".into();
    };
    let Some(extent) = child(only, "extent") else {
        return "drawing".into();
    };
    let effect = child(only, "effectExtent");
    let mut values = vec![attr_int(extent, "cx").unwrap_or(0), attr_int(extent, "cy").unwrap_or(0)];
    values.extend(
        ["l", "t", "r", "b"]
            .iter()
            .map(|s| effect.and_then(|e| attr_int(e, s)).unwrap_or(0)),
    );
    let joined: Vec<String> = values.iter().map(ToString::to_string).collect();
    format!("drawing:{}", joined.join(":"))
}

fn int_or(element: Option<&Element>, name: &str, default: i64) -> i64 {
    element.and_then(|e| attr_int(e, name)).unwrap_or(default)
}

fn flag(element: &Element, name: &str, default: bool) -> bool {
    attr(element, name).map_or(default, |v| !matches!(v, "0" | "false" | "off"))
}

fn position(element: Option<w::Position<'_>>) -> AnchorPosition {
    let Some(p) = element else {
        return AnchorPosition::default();
    };
    let offset = p.offset().map(|o| element_text(o.element())).and_then(|t| {
        let s = t.trim();
        let digits = s.strip_prefix('-').unwrap_or(s);
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| s.parse::<i64>().ok())
            .flatten()
    });
    let align = p
        .align()
        .map(|a| element_text(a.element()))
        .filter(|t| !t.is_empty())
        .map(|t| text(t.trim()));
    AnchorPosition {
        relative: p.relative_from().map(text),
        offset,
        align,
    }
}

fn polygon(wrap: Option<&Element>) -> Option<Arc<[(i64, i64)]>> {
    let polygon = child(wrap?, "wrapPolygon")?;
    let points: Vec<(i64, i64)> = elements(polygon)
        .filter(|p| matches!(local(p), "start" | "lineTo"))
        .map(|p| (attr_int(p, "x").unwrap_or(0), attr_int(p, "y").unwrap_or(0)))
        .collect();
    (!points.is_empty()).then(|| Arc::from(points))
}

fn unsupported(sp_pr: Option<&Element>) -> Vec<Text> {
    let Some(sp_pr) = sp_pr else { return Vec::new() };
    let mut out: Vec<Text> = child(sp_pr, "effectLst")
        .into_iter()
        .flat_map(elements)
        .filter(|n| !DRAWN_EFFECTS.contains(&local(n)))
        .map(|n| text(&format!("effect {}", local(n))))
        .collect();
    out.extend(
        ["effectDag", "scene3d", "sp3d"]
            .iter()
            .filter(|n| child(sp_pr, n).is_some())
            .map(|n| text(n)),
    );
    out
}

/// `a:xfrm`'s placement into `g`.
fn place(g: &mut Graphic, xfrm: Option<&Element>) {
    let Some(x) = xfrm.map(w::Xfrm::new) else { return };
    let point = |p: Option<w::Point<'_>>| {
        (
            p.and_then(w::Point::x).unwrap_or(0),
            p.and_then(w::Point::y).unwrap_or(0),
        )
    };
    let size = |e: Option<w::Extent<'_>>| {
        (
            e.and_then(w::Extent::cx).unwrap_or(0),
            e.and_then(w::Extent::cy).unwrap_or(0),
        )
    };
    g.offset = point(x.offset());
    g.extent = size(x.extent());
    g.rotation = x.rot().unwrap_or(0);
    g.flip_h = flag(x.element(), "flipH", false);
    g.flip_v = flag(x.element(), "flipV", false);
    if let Some(ch_ext) = x.child_extent() {
        g.child_offset = point(x.child_offset());
        g.child_extent = size(Some(ch_ext));
    }
}

fn body_settings(body: Option<&Element>) -> Vec<(Text, BodyValue)> {
    let Some(body) = body else { return Vec::new() };
    let mut out: BTreeMap<&str, BodyValue> = BTreeMap::new();
    for key in ["lIns", "tIns", "rIns", "bIns"] {
        if attr(body, key).is_some() {
            out.insert(key, BodyValue::Int(attr_int(body, key)));
        }
    }
    for key in ["anchor", "vert", "wrap", "rot", "anchorCtr"] {
        if let Some(v) = attr(body, key) {
            out.insert(key, BodyValue::Str(text(v)));
        }
    }
    if let Some(fit) = elements(body)
        .map(|c| local(c))
        .find(|l| matches!(*l, "noAutofit" | "spAutoFit" | "normAutofit"))
    {
        out.insert("fit", BodyValue::Str(text(fit)));
    }
    out.into_iter().map(|(k, v)| (text(k), v)).collect()
}

impl Reader<'_> {
    fn shape(&self, element: &Element, path: &str, fields: &mut Fields) -> Graphic {
        let sp_pr = child(element, "spPr");
        let body = child(element, "bodyPr");
        let content = child(element, "txbx").and_then(|t| child(t, "txbxContent"));
        let blocks = content.map(|c| self.text_box_story(c, &format!("{path}/wps:txbx/w:txbxContent"), fields));
        let mut unsupported = unsupported(sp_pr);
        if child(element, "linkedTxbx").is_some() {
            unsupported.push(text("linked text box"));
        }
        let warp = body.and_then(|b| child(b, "prstTxWarp")).and_then(|w| attr(w, "prst"));
        if let Some(warp) = warp.filter(|w| *w != "textNoShape") {
            unsupported.push(text(&format!("text warp {warp}")));
        }
        let mut g = Graphic {
            kind: text("shape"),
            text: Arc::from(blocks.unwrap_or_default()),
            body: Arc::from(body_settings(body)),
            unsupported: Arc::from(unsupported),
            path: text(path),
            ..Graphic::default()
        };
        place(&mut g, sp_pr.and_then(|s| child(s, "xfrm")));
        g
    }

    fn group(&self, element: &Element, path: &str, fields: &mut Fields) -> Graphic {
        let group_pr = child(element, "grpSpPr");
        let mut members = Vec::new();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for node in elements(element) {
            let l = local(node);
            let prefix = match l {
                "wsp" => "wps",
                "pic" => "pic",
                "grpSp" | "graphicFrame" => "wpg",
                _ => continue,
            };
            let count = counts.entry(l).or_insert(0);
            *count += 1;
            let member_path = format!("{path}/{prefix}:{l}[{count}]");
            let member = self.graphic(node, &member_path, fields).unwrap_or_else(|| Graphic {
                kind: text("other"),
                uri: Some(text(l)),
                path: text(path),
                ..Graphic::default()
            });
            members.push(Arc::new(member));
        }
        let mut g = Graphic {
            kind: text("group"),
            children: Arc::from(members),
            unsupported: Arc::from(unsupported(group_pr)),
            path: text(path),
            ..Graphic::default()
        };
        place(&mut g, group_pr.and_then(|s| child(s, "xfrm")));
        g
    }

    fn graphic(&self, element: &Element, path: &str, fields: &mut Fields) -> Option<Graphic> {
        let refs = w::RelationshipRefs::new(element);
        let mut g = match local(element) {
            "chart" if refs.id().is_some_and(|i| !i.is_empty()) => Graphic {
                kind: text("chart"),
                relationship: refs.id().map(text),
                path: text(path),
                ..Graphic::default()
            },
            "relIds" if refs.data_model().is_some_and(|i| !i.is_empty()) => Graphic {
                kind: text("diagram"),
                relationship: refs.data_model().map(text),
                path: text(path),
                ..Graphic::default()
            },
            "pic" => {
                let sp_pr = child(element, "spPr");
                let blip = child(element, "blipFill").and_then(|b| child(b, "blip"));
                let mut g = Graphic {
                    kind: text("picture"),
                    relationship: blip.and_then(|b| b.attribute(Some(R), "embed")).map(text),
                    unsupported: Arc::from(unsupported(sp_pr)),
                    path: text(path),
                    ..Graphic::default()
                };
                place(&mut g, sp_pr.and_then(|s| child(s, "xfrm")));
                g
            }
            "wsp" => self.shape(element, path, fields),
            "graphicFrame" => {
                let data = child(element, "graphic").and_then(|g| child(g, "graphicData"));
                let content = data.and_then(|d| elements(d).next())?;
                let mut inner = self.graphic(content, path, fields)?;
                if let Some(x) = child(element, "xfrm") {
                    place(&mut inner, Some(x));
                }
                inner
            }
            "wgp" | "grpSp" => self.group(element, path, fields),
            _ => return None,
        };
        if matches!(local(element), "pic" | "wsp" | "wgp" | "grpSp") {
            g.source = Some(element.clone());
        }
        g.path = text(path);
        Some(g)
    }

    fn graphic_data(&self, data: &Element, path: &str, fields: &mut Fields) -> Graphic {
        let other = || Graphic {
            kind: text("other"),
            uri: attr(data, "uri").map(text),
            path: text(path),
            ..Graphic::default()
        };
        match elements(data).next() {
            Some(content) => self.graphic(content, path, fields).unwrap_or_else(other),
            None => other(),
        }
    }

    /// A floating drawing's anchor, or `None` for an object that is not one.
    pub(crate) fn anchor_of(&self, node: &Element, path: &str, fields: &mut Fields) -> Option<Anchor> {
        let anchor = child(drawing_element(node)?, "anchor")?;
        let view = w::Anchor::new(anchor);
        let extent = child(anchor, "extent");
        let effect = child(anchor, "effectExtent");
        let wrap = elements(anchor).find(|c| local(c).starts_with("wrap")).map(|e| &**e);
        let simple = flag(anchor, "simplePos", false).then(|| {
            let point = child(anchor, "simplePos");
            (int_or(point, "x", 0), int_or(point, "y", 0))
        });
        let data = child(anchor, "graphic").and_then(|g| child(g, "graphicData"));
        let graphic = data.map(|d| Arc::new(self.graphic_data(d, path, fields)));
        Some(Anchor {
            extent: (int_or(extent, "cx", 0), int_or(extent, "cy", 0)),
            effect: ["l", "t", "r", "b"].map(|s| int_or(effect, s, 0)),
            h: position(view.position_h()),
            v: position(view.position_v()),
            simple,
            wrap: wrap.map(|w| text(local(w))),
            wrap_text: wrap.and_then(|w| attr(w, "wrapText")).map(text),
            distance: ["distT", "distB", "distL", "distR"].map(|s| int_or(Some(anchor), s, 0)),
            behind: flag(anchor, "behindDoc", false),
            relative_height: int_or(Some(anchor), "relativeHeight", 0),
            allow_overlap: flag(anchor, "allowOverlap", true),
            layout_in_cell: flag(anchor, "layoutInCell", true),
            locked: flag(anchor, "locked", false),
            graphic,
            identifier: child(anchor, "docPr").and_then(|d| attr(d, "id")).map(text),
            polygon: polygon(wrap),
            vml: false,
        })
    }

    /// What an inline drawing shows: its picture's relationship, else its shape, group,
    /// chart or SmartArt.
    pub(crate) fn inline_object(&self, node: &Element, path: &str, fields: &mut Fields) -> InlineObject {
        let Some(drawing) = drawing_element(node) else {
            return InlineObject::None;
        };
        let Some(inline) = child(drawing, "inline") else {
            return InlineObject::None;
        };
        if let Some(picture) = picture(drawing) {
            return InlineObject::Picture(picture);
        }
        let Some(data) = child(inline, "graphic").and_then(|g| child(g, "graphicData")) else {
            return InlineObject::None;
        };
        let g = self.graphic_data(data, path, fields);
        if matches!(&*g.kind, "shape" | "group" | "chart" | "diagram") {
            InlineObject::Graphic(Arc::new(g))
        } else {
            InlineObject::None
        }
    }
}

/// The relationship id of the picture an inline drawing shows: a `pic:pic`'s blip, else
/// the first blip anywhere in it.
fn picture(drawing: &Element) -> Option<Text> {
    let mut first_blip: Option<Text> = None;
    let mut stack: Vec<&Element> = vec![drawing];
    while let Some(node) = stack.pop() {
        let l = local(node);
        if l == "pic" {
            let blip = child(node, "blipFill").and_then(|b| child(b, "blip"));
            if let Some(embed) = blip
                .and_then(|b| b.attribute(Some(R), "embed"))
                .filter(|e| !e.is_empty())
            {
                return Some(text(embed));
            }
        }
        if l == "blip" && first_blip.is_none() {
            first_blip = node.attribute(Some(R), "embed").filter(|e| !e.is_empty()).map(text);
        }
        let mut kids: Vec<&Element> = node.child_elements().map(|e| &**e).collect();
        kids.reverse();
        stack.extend(kids);
    }
    first_blip
}
