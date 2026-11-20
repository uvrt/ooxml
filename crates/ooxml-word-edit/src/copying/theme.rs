//! Owns: theme fonts and colours made concrete for a copy between two documents whose
//! themes differ, as docx-agent's importer makes them (`_theme_faces`, `_theme_colors`,
//! `_concrete`): a theme font attribute written as the face the source's theme names, a
//! colour's theme attributes dropped (its value stays), only where the destination's
//! theme names other faces or colours.
//!
//! Must not: read anything but the two documents' themes.

use super::cascade::{tag_of, with_attribute, without_w, THEME_FONTS};
use crate::names::W;
use crate::Result;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::parts::related_parts;
use ooxml_xml::{Element, Node, QName};
use std::collections::HashMap;
use std::sync::Arc;

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// The theme's relationship type.
pub(crate) const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";

/// The first theme part of the main part, when the package has it (`_theme_root`).
pub(crate) fn theme_part(docx: &Docx) -> Result<Option<String>> {
    let first = related_parts(&docx.package, &docx.main, REL_THEME)?.into_iter().next();
    Ok(first.filter(|p| docx.package.has_part(p)))
}

fn theme_root(docx: &Docx) -> Result<Option<Arc<Element>>> {
    let Some(part) = theme_part(docx)? else { return Ok(None) };
    Ok(docx.package.tree(&part)?.map(|t| Arc::clone(t.root())))
}

/// `minorHAnsi` -> the face the theme names, and so on.
fn faces(docx: &Docx) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    let Some(root) = theme_root(docx)? else { return Ok(out) };
    let scheme = root
        .find(Some(A), "themeElements")
        .and_then(|e| e.find(Some(A), "fontScheme"));
    for which in ["major", "minor"] {
        let Some(font) = scheme.and_then(|s| s.find(Some(A), &format!("{which}Font"))) else {
            continue;
        };
        for (script, key) in [("latin", "HAnsi"), ("ea", "EastAsia"), ("cs", "Bidi")] {
            if let Some(face) = font
                .find(Some(A), script)
                .and_then(|f| f.attribute(None, "typeface"))
                .filter(|f| !f.is_empty())
            {
                out.insert(format!("{which}{key}"), face.to_owned());
            }
        }
    }
    Ok(out)
}

/// The colour scheme's slots and their values.
fn colours(docx: &Docx) -> Result<Option<HashMap<String, String>>> {
    let Some(root) = theme_root(docx)? else {
        return Ok(Some(HashMap::new()));
    };
    let Some(scheme) = root
        .find(Some(A), "themeElements")
        .and_then(|e| e.find(Some(A), "clrScheme"))
    else {
        return Ok(Some(HashMap::new()));
    };
    let mut out = HashMap::new();
    for node in scheme.children().iter() {
        match node {
            Node::Element(slot) => {
                let value = match (slot.find(Some(A), "srgbClr"), slot.find(Some(A), "sysClr")) {
                    (Some(rgb), _) => rgb.attribute(None, "val"),
                    (None, Some(system)) => system.attribute(None, "lastClr"),
                    (None, None) => Some(""),
                };
                // `None` here is a slot whose colour has no value: the dictionaries
                // compare it as Python's None
                out.insert(
                    slot.name().local().to_owned(),
                    value.map_or("\0".to_owned(), str::to_owned),
                );
            }
            // docx-agent reads a comment in the scheme as a slot it cannot name: the
            // importer fails there, so the host copies it
            Node::Comment(_) | Node::Pi(_) => return Ok(None),
            _ => {}
        }
    }
    Ok(Some(out))
}

/// What the importer needs of the two themes.
pub(crate) struct Themes {
    source_faces: HashMap<String, String>,
    faces_differ: bool,
    colours_differ: bool,
}

impl Themes {
    /// The themes of `source` and `destination`; `None` where docx-agent cannot read them.
    ///
    /// # Errors
    /// When a part is not XML.
    pub(crate) fn read(source: &Docx, destination: &Docx) -> Result<Option<Themes>> {
        let source_faces = faces(source)?;
        let faces_differ = source_faces != faces(destination)?;
        let (Some(theirs), Some(ours)) = (colours(source)?, colours(destination)?) else {
            return Ok(None);
        };
        Ok(Some(Themes {
            source_faces,
            faces_differ,
            colours_differ: theirs != ours,
        }))
    }

    /// `node` with theme fonts as the source's faces and theme colours as their values,
    /// where the destination's theme names others (`_concrete`).
    pub(crate) fn concrete(&self, node: &Arc<Element>) -> Arc<Element> {
        let tag = tag_of(node);
        if tag.0.as_deref() != Some(W) {
            return Arc::clone(node);
        }
        match tag.1.as_str() {
            "rFonts" if self.faces_differ => {
                let mut out: Element = (**node).clone();
                for (theme, plain) in THEME_FONTS {
                    let value = out.attribute(Some(W), theme).map(str::to_owned);
                    out = without_w(&out, &[theme]);
                    if let Some(face) = value.and_then(|v| self.source_faces.get(&v)) {
                        let name = Arc::new(QName::new(node.name().prefix(), plain, Some(W)));
                        out = with_attribute(&out, &name, face);
                    }
                }
                Arc::new(out)
            }
            "color" if self.colours_differ => Arc::new(without_w(node, &["themeColor", "themeShade", "themeTint"])),
            _ => Arc::clone(node),
        }
    }
}
