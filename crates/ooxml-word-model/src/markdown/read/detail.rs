//! Owns: what `word_inspect` reads of a paragraph's inline content besides its Markdown
//! (docx-agent's `_Inline` record details): the hyperlinks' numbering in a paragraph, and
//! a drawing's record (picture or drawing, with a group's members and the charts and
//! diagrams they hold).
//!
//! Must not: write Markdown items.

use super::{index, DrawingRecord, MemberRecord};
use crate::docx::drawings::{drawing_kind, group_members, member_kind};
use crate::docx::graphics::Graphic;
use crate::ids::{W, WP};
use ooxml_xml::{Element, Node};

const WPG: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup";
const WPS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";

/// The places of every `w:hyperlink` below `paragraph` (at `base`), in document order.
#[must_use]
pub(super) fn every_hyperlink(paragraph: &Element, base: &[u32]) -> Vec<Box<[u32]>> {
    let mut out = Vec::new();
    let mut path = base.to_vec();
    collect(paragraph, &mut path, &mut out);
    out
}

fn collect(node: &Element, path: &mut Vec<u32>, out: &mut Vec<Box<[u32]>>) {
    for (k, child) in node.children().iter().enumerate() {
        let Node::Element(e) = child else { continue };
        path.push(index(k));
        if e.name().is(Some(W), "hyperlink") {
            out.push(path.clone().into_boxed_slice());
        }
        collect(e, path, out);
        path.pop();
    }
}

/// A frame's record: a picture where `picture` names it, else a drawing, with what
/// `graphic_of` reads of it (or of a group's member) given its kind.
#[must_use]
pub(super) fn drawing_record(
    frame: &Element,
    picture: Option<String>,
    drawing: Option<String>,
    (name, alt): (Option<String>, String),
    graphic_of: &dyn Fn(&Element, &str) -> Option<Graphic>,
) -> DrawingRecord {
    let floating = frame.name().is(Some(WP), "anchor");
    let kind = drawing_kind(frame);
    if picture.is_some() {
        let size = frame.find(Some(WP), "extent").and_then(|e| {
            let cx: i64 = e.attribute(None, "cx")?.trim().parse().ok()?;
            let cy: i64 = e.attribute(None, "cy")?.trim().parse().ok()?;
            Some((cx as f64 / 12700.0, cy as f64 / 12700.0))
        });
        return DrawingRecord {
            id: picture,
            kind,
            name,
            alt,
            floating,
            size,
            picture: true,
            graphic: None,
            members: None,
        };
    }
    let members = (kind == "group").then(|| {
        let prefix = drawing.clone().unwrap_or_else(|| "None".to_owned());
        group_members(frame, &prefix, false)
            .into_iter()
            .map(|(id, element)| {
                let name = [WPG, WPS]
                    .iter()
                    .find_map(|ns| element.find(Some(ns), "cNvPr"))
                    .and_then(|p| p.attribute(None, "name"))
                    .map(str::to_owned);
                let kind = member_kind(&element);
                let graphic = graphic_of(&element, kind);
                MemberRecord {
                    id,
                    kind,
                    name,
                    graphic,
                }
            })
            .collect()
    });
    DrawingRecord {
        id: drawing,
        kind,
        name,
        alt,
        floating,
        size: None,
        picture: false,
        graphic: graphic_of(frame, kind),
        members,
    }
}
