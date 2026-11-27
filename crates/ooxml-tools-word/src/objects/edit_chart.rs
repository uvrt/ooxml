//! Owns: `edit_chart` as Word serves it (docx-agent's `tools/shared.py`, `edit_chart`):
//! the chart tools' handlers (`ooxml_tools_chart`) over Word's hosts (`graphics`), and the
//! language new chart text is written in: the document's default language, else the call's
//! locale's (the session's, en-US where the host sets none: [`chart_language`]).
//!
//! Must not: change the document but through a command.

use super::graphics::{VersionGraphics, WordGraphics};
use crate::{Editing, Version};
use ooxml_base::Cancel;
use ooxml_tools::{Done, ToolError, ToolResult};
use ooxml_tools_chart::EditChart;
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use serde_json::Value;

/// `edit_chart`'s `read`: what the chart holds and how it looks, changing nothing.
///
/// # Errors
/// `not_found` for a target no drawing has, `refused` for a drawing that is not a chart,
/// `timeout` when the call is cancelled.
fn edit_chart_read(args: &EditChart, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    let host = VersionGraphics {
        version,
        cancel,
        doc: &args.doc,
    };
    ooxml_tools_chart::edit_chart_read(args, &host)
}

/// `edit_chart`'s actions that change the chart.
#[must_use]
fn edit_chart(args: &EditChart, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let writer = WordGraphics {
        editing,
        doc: &args.doc,
    };
    ooxml_tools_chart::edit_chart(args, &writer)
}

/// The language new chart text is written in (docx-agent's `default_language`): the
/// document's default run language, else the language of the locale the call edits under
/// (the call's, else the session's), else en-US -- as Word takes its editing language.
pub(super) fn chart_language(docx: &Docx, editing: &Editing<'_>) -> String {
    default_language(docx).unwrap_or_else(|| {
        editing
            .locale
            .map_or_else(|| ooxml_numfmt::EN_US.language.to_string(), |l| l.language.to_string())
    })
}

/// The document's default run language (`w:docDefaults/w:rPrDefault/w:rPr/w:lang`);
/// `None` where it names none.
fn default_language(docx: &Docx) -> Option<String> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let part = styles_part(docx)?;
    let tree = docx.package.tree(&part).ok()??;
    let mut node: &Element = tree.root();
    for name in ["docDefaults", "rPrDefault", "rPr", "lang"] {
        node = node.find(Some(W), name)?;
    }
    node.attribute(Some(W), "val")
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// The document's styles part, where it has one: what [`default_language`] reads.
pub(super) fn styles_part(docx: &Docx) -> Option<String> {
    const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
    ooxml_word_model::parts::related(&docx.package, &docx.main, STYLES).ok()?
}

/// Registers `edit_chart` (the group's [`super::register`]): the host asks the session first
/// (its data actions that change points in place; the rest are refused as not supported yet).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.reading_edit::<EditChart>(edit_chart_read, edit_chart);
}
