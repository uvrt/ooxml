//! Owns: content controls as docx-agent names them (`edit/annotations.py`): `cc:<w:id>`,
//! `cc:<id>#<k>` for the k-th repeat of an id, `cc@<story>/<n>` for one without an id, in
//! story and document order; and what kind each is.
//!
//! Must not: change a control or read its content's formatting.

use crate::docx::{child_val, Docx};
use crate::ids::W;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// Word 2010's namespace (`w14:checkbox`).
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
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
