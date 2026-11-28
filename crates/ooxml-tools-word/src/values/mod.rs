//! Owns: what a check of numbers and a scale read of a Word version, for the host whose tool
//! layer runs those checks (`ooxml-edit`'s `numbers` and `scales`, through the docx format's
//! `number_texts` and `scale_values` hooks; lane H7e, docx-agent's `tools/textfacts.py` and
//! `tools/scales.py`): the texts a check of numbers reads ([`number_texts`]) and the values a
//! scale's targets hold with the step each is styled as now ([`scale_values`]), each placed on
//! its pages by the session's own layout of the version where one is given; and what both
//! share -- the pages of the version's paragraphs ([`Pages`]) and the cell a paragraph is in
//! ([`cell_of`]).
//!
//! Must not: change the document, read a scale's arguments (the host parses the scale and
//! gives its steps), or choose a value's step from its text (the host's tool layer does).

mod number_texts;
mod scale;
mod scale_values;
mod targets;

pub use number_texts::{number_texts, NumberText, NumberTexts};
pub use scale::{ReadScale, ReadStep};
pub use scale_values::{scale_values, ScaleValue, ScaleValues, ValueHandle};

use crate::pages::{NotLaid, WordLayout};
use ooxml_base::Cancel;
use ooxml_tools::{Failure, ToolError, TryCall, WayOut};
use ooxml_word_model::docx::tables::cells_at;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::docx::{at_path, StoryPart};
use ooxml_word_model::ids::W;
use ooxml_word_model::ids::{BlockKey, StoryIx};
use ooxml_word_pages::LaidVersion;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// The pages of a version's body paragraphs, from its layout (docx-agent's
/// `DocumentLayout.pages_of` of a Rust layout: each page that draws a line of it).
pub(crate) struct Pages {
    version: Arc<LaidVersion>,
    /// Each body paragraph's 1-based pages, by its key.
    body: HashMap<BlockKey, BTreeSet<usize>>,
}

impl Pages {
    /// `reading` (a version's) laid out by `layout`, stopped where `cancel` says so.
    pub(crate) fn of(layout: &dyn WordLayout, reading: &Arc<Docx>, cancel: &dyn Cancel) -> Result<Pages, NotLaid> {
        let version = layout.laid(reading, cancel)?;
        let mut body: HashMap<BlockKey, BTreeSet<usize>> = HashMap::new();
        for (index, page) in version.facts().pages.iter().enumerate() {
            for key in page.lines.iter().filter_map(|l| l.anchor).map(|a| a.block) {
                if key.story() == StoryIx::BODY {
                    body.entry(key).or_default().insert(index + 1);
                }
            }
        }
        Ok(Pages { version, body })
    }

    /// The pages the paragraphs `ids` are drawn on, in order, each once.
    pub(crate) fn of_paragraphs<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
        let mut out: BTreeSet<usize> = BTreeSet::new();
        for id in ids {
            let key = self.version.layout().document.names.key_of_id(StoryIx::BODY, id);
            if let Some(found) = key.and_then(|key| self.body.get(&key)) {
                out.extend(found);
            }
        }
        out.into_iter().collect()
    }
}

/// Why a version was not laid out, in docx-agent's layout's words: `(code, message)`.
pub(crate) fn not_laid(why: NotLaid) -> (&'static str, String) {
    match why {
        NotLaid::Stopped => ("timeout", "laying the document out was stopped".to_owned()),
        reason => (
            "layout_failed",
            format!("the Word layout does not lay this version out yet ({reason})"),
        ),
    }
}

/// The layout a read places its texts with: the version laid out, or why it was not; `None`
/// where the host gives no layout.
pub(crate) fn lay_out(
    layout: Option<&dyn WordLayout>,
    reading: &Arc<Docx>,
    cancel: &dyn Cancel,
) -> Option<Result<Pages, NotLaid>> {
    layout.map(|layout| Pages::of(layout, reading, cancel))
}

/// The pages asked for (`pages`) refused because the version was not laid out (`why`): as
/// not supported yet where it holds what the layout cannot lay out (`check <what> on pages:
/// <what it holds>`, the way out `instead`: the check without `pages`), else as the
/// layout's failure ([`pages_failed`]).
pub(crate) fn pages_refused(what: &str, why: NotLaid, instead: TryCall) -> ToolError {
    if let Some(held) = why.unlaid() {
        let form = format!("check {what} on pages: {held}");
        return ToolError::not_supported(&form, WayOut::call(instead)).at("pages");
    }
    let (code, message) = not_laid(why);
    pages_failed(what, code, &message)
}

/// A check of `what` in the document `doc` (the host's name) without `pages`: `arguments`
/// besides `doc` and `range` (where given), `fill` the arguments the caller gives again.
pub(crate) fn without_pages(
    doc: &str,
    range: Option<&[String]>,
    arguments: &[(&str, Value)],
    fill: &[&str],
) -> TryCall {
    let mut given = serde_json::Map::new();
    given.insert("doc".into(), Value::from(doc));
    for (name, value) in arguments {
        given.insert((*name).to_owned(), value.clone());
    }
    if let Some(range) = range.filter(|r| !r.is_empty()) {
        given.insert("range".into(), Value::from(range.to_vec()));
    }
    TryCall {
        tool: "check".to_owned(),
        arguments: given,
        fill: fill.iter().map(|f| (*f).to_owned()).collect(),
    }
}

/// The pages asked for (`pages`) a layout's failure refuses, in docx-agent's words.
pub(crate) fn pages_failed(what: &str, code: &'static str, message: &str) -> ToolError {
    let message =
        format!("{what} on pages need the layout: {message}; leave pages out to read the whole body, or give range");
    let code = if code == "timeout" {
        Failure::Timeout
    } else {
        Failure::LayoutFailed
    };
    ToolError::failure(code, message).at("pages")
}

/// A table cell a paragraph is in.
pub(crate) struct CellAt {
    /// The `w:tc`'s place in its part.
    pub(crate) path: Box<[u32]>,
    /// Its id, `t:X/c<row>,<grid column>` (docx-agent's `_cell_id`); `None` where its table or
    /// row has none.
    pub(crate) id: Option<String>,
}

/// The innermost cell holding the place `path` of `part`, or `None` outside a table.
pub(crate) fn cell_of(part: &StoryPart, path: &[u32]) -> Option<CellAt> {
    let root = part.root();
    let named = |n: usize, local: &str| {
        path.get(..n)
            .and_then(|p| at_path(root, p))
            .is_some_and(|e| e.name().is(Some(W), local))
    };
    let cell = (1..path.len()).rev().find(|&n| named(n, "tc"))?;
    let cell_path: Box<[u32]> = path.get(..cell)?.into();
    let row = (1..cell).rev().find(|&n| named(n, "tr"));
    let table = (1..cell).rev().find(|&n| named(n, "tbl"));
    let id = row.zip(table).and_then(|(row, table)| {
        let entry = part.ids.table_at(path.get(..table)?)?;
        let row_path = path.get(..row)?;
        let number = entry.rows.iter().position(|(p, _)| **p == *row_path)?;
        let column = cells_at(at_path(root, row_path)?, row_path)
            .into_iter()
            .find(|c| *c.path == *cell_path)?
            .column;
        Some(format!("{}/c{number},{column}", entry.id))
    });
    Some(CellAt { path: cell_path, id })
}
