//! Owns: `app.xml`'s statistics brought up to date as docx-agent's `refreshed_app` does on
//! a save: each statistic the part has (`Pages`, `Words`, `Characters`, `Lines`,
//! `Paragraphs`, `CharactersWithSpaces`, direct children of the root) given its value where
//! its text differs; one the part lacks is not added, nor one whose value is not known
//! (pages and lines without a layout).
//!
//! Must not: count anything (the caller gives the values) or add the part.

use super::written;
use crate::doc::ModelError;
use crate::new_part::edit_part;
use crate::Result;
use ooxml_opc::Package;
use ooxml_word_model::parts::related;
use std::sync::Arc;

const APP_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
const REL_APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";

/// The statistics a save writes into `app.xml`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppStatistics {
    /// Words.
    pub words: u64,
    /// Characters, white space left out.
    pub characters: u64,
    /// Characters with spaces.
    pub characters_with_spaces: u64,
    /// Paragraphs with text.
    pub paragraphs: u64,
    /// Pages, where the document has been laid out.
    pub pages: Option<u64>,
    /// The body's lines, where it has been laid out.
    pub lines: Option<u64>,
}

impl AppStatistics {
    /// Each statistic known, by its element's name, in docx-agent's order.
    fn known(&self) -> Vec<(&'static str, u64)> {
        let mut out = vec![
            ("Words", self.words),
            ("Characters", self.characters),
            ("CharactersWithSpaces", self.characters_with_spaces),
            ("Paragraphs", self.paragraphs),
        ];
        out.extend(self.pages.map(|v| ("Pages", v)));
        out.extend(self.lines.map(|v| ("Lines", v)));
        out
    }
}

/// `app.xml` (its part name) with `stats` written in, as ooxml-edit writes it; `None`
/// without the part, when it is not XML, or when every statistic it has is already right.
///
/// # Errors
/// When the package's relationships are not XML.
pub(super) fn refreshed(package: &Package, stats: &AppStatistics) -> Result<Option<(String, Arc<[u8]>)>> {
    let Some(part) = related(package, "", REL_APP).map_err(ModelError::from)? else {
        return Ok(None);
    };
    let Ok(Some(tree)) = package.tree(&part) else {
        return Ok(None);
    };
    let root = tree.root();
    let due: Vec<(&str, String)> = stats
        .known()
        .into_iter()
        .filter_map(|(local, value)| {
            let node = root.child_elements().find(|e| e.name().is(Some(APP_NS), local))?;
            let value = value.to_string();
            (ooxml_xml::view::text(node) != value).then_some((local, value))
        })
        .collect();
    if due.is_empty() {
        return Ok(None);
    }
    let edited = edit_part(package, &part, |work| {
        let root = work.root();
        for (local, value) in &due {
            if let Some(node) = work.find(root, APP_NS, local) {
                work.set_text(node, value);
            }
        }
    })?;
    Ok(Some((part.clone(), written(&edited, &part)?)))
}
