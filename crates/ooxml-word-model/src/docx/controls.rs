//! Owns: content controls as docx-agent names them (`edit/annotations.py`): `cc:<w:id>`,
//! `cc:<id>#<k>` for the k-th repeat of an id, `cc@<story>/<n>` for one without an id, in
//! story and document order; what kind each is, where it stands (`control_level`), and
//! what it holds (`ContentControl.text`, `Document.control_value`): its items, its
//! check box's state, its text.
//!
//! Must not: change a control or read its content's formatting.

use crate::docx::text::{offset_before, paragraph_text};

use crate::docx::{at_path, child_val, Docx, StoryPart};
use crate::ids::W;
use crate::view::View;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// Word 2010's namespace (`w14:checkbox`).
pub const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
/// Word 2012's namespace (`w15:repeatingSection`).
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";

/// A content control.
#[derive(Debug, Clone)]
pub struct Control {
    /// Its part.
    pub part: String,
    /// Its id.
    pub id: String,
    /// The `w:sdt`.
    pub element: Arc<Element>,
    /// Its place in the part.
    pub path: Box<[u32]>,
}

impl Control {
    /// Its kind: `rich-text`, `text`, `checkbox`, `drop-down`, `date`...
    #[must_use]
    pub fn kind(&self) -> &'static str {
        control_kind(&self.element)
    }

    /// Its `w:tag`.
    #[must_use]
    pub fn tag(&self) -> Option<&str> {
        self.element.find(Some(W), "sdtPr").and_then(|p| child_val(p, "tag"))
    }

    /// Its title (`w:alias`).
    #[must_use]
    pub fn alias(&self) -> Option<&str> {
        self.element.find(Some(W), "sdtPr").and_then(|p| child_val(p, "alias"))
    }

    /// Where it stands in `part` (its own): `block`, `inline`, `row` or `cell`.
    #[must_use]
    pub fn level(&self, part: &StoryPart) -> &'static str {
        control_level(part.root(), &self.path, &self.element)
    }

    /// The text it holds now (docx-agent's `ContentControl.text`): in the line, the
    /// characters of its paragraph's text inside its content; otherwise its paragraphs'
    /// texts, a line each.
    #[must_use]
    pub fn text(&self, part: &StoryPart) -> String {
        if self.level(part) != "inline" {
            let texts: Vec<String> = part
                .ids
                .paragraphs_within(&self.path)
                .iter()
                .map(|p| paragraph_text(&p.element, View::Final))
                .collect();
            return texts.join("\n");
        }
        let root = part.root();
        let above = (1..self.path.len()).rev().find_map(|n| {
            let (at, below) = (self.path.get(..n)?, self.path.get(n..)?);
            at_path(root, at)
                .filter(|e| e.name().is(Some(W), "p"))
                .map(|p| (p, below))
        });
        let (Some((paragraph, below)), Some(content)) = (above, content_index(&self.element)) else {
            return String::new();
        };
        let mut start: Vec<u32> = below.to_vec();
        start.push(content);
        let mut end = start.clone();
        if let Some(last) = end.last_mut() {
            *last += 1;
        }
        let (from, to) = (
            offset_before(paragraph, &start, View::Final),
            offset_before(paragraph, &end, View::Final),
        );
        paragraph_text(paragraph, View::Final)
            .chars()
            .skip(from)
            .take(to.saturating_sub(from))
            .collect()
    }

    /// A list control's items: (display text, value) (docx-agent's `items`).
    #[must_use]
    pub fn items(&self) -> Vec<(String, String)> {
        items(&self.element)
    }

    /// Its check box's state (docx-agent's `checked`): `None` for a control without one.
    #[must_use]
    pub fn checked(&self) -> Option<bool> {
        checked(&self.element)
    }
}

/// The place of a `w:sdt`'s `w:sdtContent` among its children.
fn content_index(sdt: &Element) -> Option<u32> {
    sdt.children()
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), "sdtContent")))
        .and_then(|k| u32::try_from(k).ok())
}

/// A `w:sdt`'s property `local` (in `w:sdtPr`) of `namespace`.
#[must_use]
pub fn property<'e>(sdt: &'e Element, namespace: &str, local: &str) -> Option<&'e Arc<Element>> {
    sdt.find(Some(W), "sdtPr")?.find(Some(namespace), local)
}

/// A list control's items: (display text, value), the display text the value where none
/// is given (docx-agent's `items`).
#[must_use]
pub fn items(sdt: &Element) -> Vec<(String, String)> {
    for local in ["dropDownList", "comboBox"] {
        if let Some(list) = property(sdt, W, local) {
            return list
                .child_elements()
                .filter(|i| i.name().is(Some(W), "listItem"))
                .map(|i| {
                    let value = i.attribute(Some(W), "value").unwrap_or("");
                    let display = i
                        .attribute(Some(W), "displayText")
                        .filter(|d| !d.is_empty())
                        .unwrap_or(value);
                    (display.to_owned(), value.to_owned())
                })
                .collect();
        }
    }
    Vec::new()
}

/// A check box's state (`w14:checked`, `1` or `true`); `None` for a control without one.
#[must_use]
pub fn checked(sdt: &Element) -> Option<bool> {
    let control = property(sdt, W14, "checkbox")?;
    let state = control.find(Some(W14), "checked");
    Some(state.is_some_and(|s| matches!(s.attribute(Some(W14), "val").unwrap_or("0"), "1" | "true")))
}

/// How a control is locked (`w:lock`): `sdtLocked`, `contentLocked`, `sdtContentLocked`.
#[must_use]
pub fn lock(sdt: &Element) -> Option<&str> {
    property(sdt, W, "lock")?.attribute(Some(W), "val")
}

/// Where the control at `path` of the part whose root is `root` stands (docx-agent's
/// `control_level`): `block`, `inline`, `row` or `cell`.
#[must_use]
pub fn control_level(root: &Arc<Element>, path: &[u32], sdt: &Element) -> &'static str {
    let mut above = path.split_last().map_or(&[][..], |(_, p)| p);
    let element_at = |p: &[u32]| if p.is_empty() { Some(root) } else { at_path(root, p) };
    let mut parent = element_at(above);
    while parent.is_some_and(|p| is_w(p, &["customXml", "sdtContent", "smartTag"])) {
        let Some((_, up)) = above.split_last() else {
            parent = None;
            break;
        };
        above = up;
        parent = element_at(above);
    }
    match parent {
        Some(p)
            if is_w(
                p,
                &["p", "hyperlink", "ins", "del", "moveTo", "moveFrom", "fldSimple", "sdt"],
            ) =>
        {
            let content = sdt.find(Some(W), "sdtContent");
            let holds = content.is_some_and(|c| c.find(Some(W), "p").is_some() || c.find(Some(W), "tbl").is_some());
            if holds {
                "block"
            } else {
                "inline"
            }
        }
        Some(p) if is_w(p, &["tbl"]) => "row",
        Some(p) if is_w(p, &["tr"]) => "cell",
        _ => "block",
    }
}

fn is_w(element: &Element, locals: &[&str]) -> bool {
    element.name().namespace() == Some(W) && locals.contains(&element.name().local())
}

/// Every content control, in story and document order.
#[must_use]
pub fn controls(docx: &Docx) -> Vec<Control> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for part in &docx.parts {
        for (number, f) in part.facts().controls().into_iter().enumerate() {
            let raw = f.element.find(Some(W), "sdtPr").and_then(|p| child_val(p, "id"));
            let id = match raw {
                None => format!("cc@{}/{number}", part.story),
                Some(raw) => {
                    let count = seen.entry(raw.to_owned()).or_insert(0);
                    let id = if *count > 0 {
                        format!("cc:{raw}#{count}")
                    } else {
                        format!("cc:{raw}")
                    };
                    *count += 1;
                    id
                }
            };
            out.push(Control {
                part: part.part.clone(),
                id,
                element: f.element,
                path: f.path,
            });
        }
    }
    out
}

/// What kind of content control a `w:sdt` is, by the first kind its properties name.
#[must_use]
pub fn control_kind(sdt: &Element) -> &'static str {
    const KINDS: [(&str, &str, &str); 15] = [
        (W14, "checkbox", "checkbox"),
        (W, "dropDownList", "drop-down"),
        (W, "comboBox", "combo-box"),
        (W, "date", "date"),
        (W, "picture", "picture"),
        (W, "docPartObj", "building-block"),
        (W, "docPartList", "building-block"),
        (W, "group", "group"),
        (W, "citation", "citation"),
        (W, "bibliography", "bibliography"),
        (W, "equation", "equation"),
        (W15, "repeatingSection", "repeating-section"),
        (W15, "repeatingSectionItem", "repeating-section-item"),
        (W, "text", "text"),
        (W, "richText", "rich-text"),
    ];
    if let Some(properties) = sdt.find(Some(W), "sdtPr") {
        for (namespace, local, kind) in KINDS {
            if properties.find(Some(namespace), local).is_some() {
                return kind;
            }
        }
    }
    "rich-text"
}
