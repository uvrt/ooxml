//! Owns: a theme colour's value as docx-agent's `FormatOps._theme_color` reads it for
//! `w:color`'s `w:val`: the main part's theme's colour scheme slot the name is (the text
//! and background names through the settings' `w:clrSchemeMapping`), its `srgbClr` value
//! or `sysClr` last colour, upper-cased.
//!
//! Must not: apply tints or shades (a run's colour is written without them), or change
//! anything.

use crate::{Result, WordDoc};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::parts::{related, related_parts, REL_SETTINGS};
use ooxml_xml::Element;

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";

/// The theme's colour scheme and the settings' mapping, read once for an edit.
#[derive(Debug, Default)]
pub(crate) struct ThemeColours {
    scheme: Option<std::sync::Arc<Element>>,
    mapping: Option<std::sync::Arc<Element>>,
}

fn element_child<'a>(parent: &'a Element, namespace: &str, local: &str) -> Option<&'a std::sync::Arc<Element>> {
    parent.find(Some(namespace), local)
}

impl ThemeColours {
    /// Read `doc`'s theme and colour mapping.
    ///
    /// # Errors
    /// When a part is not XML.
    pub(crate) fn read(doc: &WordDoc, docx: &Docx) -> Result<ThemeColours> {
        let package = doc.package();
        let mut out = ThemeColours::default();
        if let Some(part) = related_parts(package, &docx.main, REL_THEME)?.first() {
            if let Some(tree) = doc.tree(part)? {
                out.scheme = element_child(tree.root(), A, "themeElements")
                    .and_then(|e| element_child(e, A, "clrScheme"))
                    .cloned();
            }
        }
        if let Some(part) = related(package, &docx.main, REL_SETTINGS)? {
            if let Some(tree) = doc.tree(&part)? {
                out.mapping = element_child(tree.root(), W, "clrSchemeMapping").cloned();
            }
        }
        Ok(out)
    }

    /// The value of the theme colour `name` (one of Word's names), if the theme gives one.
    pub(crate) fn value(&self, name: &str) -> Option<String> {
        let scheme = self.scheme.as_ref()?;
        let mut slot = match name {
            "dark1" | "text1" => "dk1",
            "light1" | "background1" => "lt1",
            "dark2" | "text2" => "dk2",
            "light2" | "background2" => "lt2",
            "hyperlink" => "hlink",
            "followedHyperlink" => "folHlink",
            other => other,
        };
        let key = match name {
            "text1" => Some("t1"),
            "background1" => Some("bg1"),
            "text2" => Some("t2"),
            "background2" => Some("bg2"),
            _ => None,
        };
        if let (Some(key), Some(mapping)) = (key, self.mapping.as_ref()) {
            match mapping.attribute(Some(W), key) {
                Some("dark1") => slot = "dk1",
                Some("light1") => slot = "lt1",
                Some("dark2") => slot = "dk2",
                Some("light2") => slot = "lt2",
                _ => {}
            }
        }
        let node = element_child(scheme, A, slot)?;
        for child in node.children() {
            let ooxml_xml::Node::Element(child) = child else {
                continue;
            };
            if child.name().is(Some(A), "srgbClr") {
                return child
                    .attribute(None, "val")
                    .filter(|v| !v.is_empty())
                    .map(str::to_uppercase);
            }
            if child.name().is(Some(A), "sysClr") {
                return child
                    .attribute(None, "lastClr")
                    .filter(|v| !v.is_empty())
                    .map(str::to_uppercase);
            }
        }
        None
    }
}
