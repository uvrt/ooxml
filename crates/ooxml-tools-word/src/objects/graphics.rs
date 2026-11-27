//! Owns: Word's hosts of the chart tools (`ooxml_tools_chart`'s [`GraphicHost`] and
//! [`GraphicWriter`]): the chart a target names on a version -- the first drawing or group
//! member with that id (docx-agent's `Document.chart`), its part through its first
//! `c:chart`'s relationship -- refused as `not_found` or `refused` with the call that lists
//! the drawings; and a chart edit planned in its own transaction and applied as one
//! [`EditGraphic`] command, the language new text is written in the document's
//! (`chart_language`), the styles part read for it.
//!
//! Must not: plan a chart edit or word its reply (the chart tools do), or change the
//! document but through the command.

use super::edit_chart::{chart_language, styles_part};
use crate::planned::run_planned;
use crate::review::edit_refusal;
use crate::structure::common::{call, policy};
use crate::{Editing, Version};
use ooxml_base::pyfmt::repr_str;
use ooxml_base::Cancel;
use ooxml_chart::edit::GraphicEdit;
use ooxml_store::Txn;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_tools_chart::{Found, Graphic, GraphicHost, GraphicWriter, NoChart, Warnings, Written};
use ooxml_word_edit::{EditGraphic, Key, WordDoc};
use ooxml_word_model::docx::drawings::graphics;
use ooxml_word_model::docx::graphics::chart_part;
use ooxml_word_model::docx::Docx;
use serde_json::json;
use std::sync::Arc;

/// The charts of the version a read pins, for the call on the document `doc`.
pub(super) struct VersionGraphics<'a, 'v> {
    /// The version.
    pub(super) version: &'a Version<'v>,
    /// The call's cancel port: checked before the chart is found, and again once it is.
    pub(super) cancel: &'a dyn Cancel,
    /// The document as the call names it (in the refusals' way out).
    pub(super) doc: &'a str,
}

impl GraphicHost for VersionGraphics<'_, '_> {
    fn find(&self, address: &str) -> Result<Found, ToolError> {
        crate::review::stopped(self.cancel)?;
        let docx = self.version.docx()?;
        let found = word_graphic(&docx, self.doc, address)?;
        if found.is_ok() {
            crate::review::stopped(self.cancel)?;
        }
        Ok(found)
    }
}

/// The charts the call's editor changes, for the call on the document `doc`.
pub(super) struct WordGraphics<'a, 'e> {
    /// The call's editor.
    pub(super) editing: &'a Editing<'e>,
    /// The document as the call names it (in the refusals' way out).
    pub(super) doc: &'a str,
}

impl GraphicWriter for WordGraphics<'_, '_> {
    fn write<E: GraphicEdit<Out = Warnings>>(
        &self,
        address: &str,
        read: &TryCall,
        plan: &dyn Fn(&Found, &str) -> Result<E, ToolError>,
    ) -> Result<Written, ToolError> {
        let planned = |tx: &mut Txn<'_, WordDoc>| self.planned(tx, address, plan);
        let (edited, version) = match run_planned(self.editing, planned)? {
            Ok(done) => (done.out.0, done.version),
            Err(error) => return Err(edit_refusal(error, "", read.clone())),
        };
        let after = version
            .doc()
            .docx()
            .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
        Ok(Written {
            warnings: edited.warnings.into_iter().chain(edited.out).collect(),
            after: word_graphic(&after, self.doc, address)?,
        })
    }
}

impl WordGraphics<'_, '_> {
    /// The command a chart edit is on the transaction's version: the chart `address` names
    /// found there and `plan` made of it in the language new text is written in; the styles
    /// part read for that language (the chart's part the command reads itself).
    pub(super) fn planned<E: GraphicEdit>(
        &self,
        tx: &mut Txn<'_, WordDoc>,
        address: &str,
        plan: &dyn Fn(&Found, &str) -> Result<E, ToolError>,
    ) -> Result<(EditGraphic<E>, ()), ToolError> {
        let docx = tx
            .doc()
            .docx()
            .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
        let found = word_graphic(&docx, self.doc, address)?;
        let edit = plan(&found, &chart_language(&docx, self.editing))?;
        let command = EditGraphic {
            drawing: address.to_owned(),
            edit,
            policy: policy(self.editing),
        };
        if let Some(part) = styles_part(&docx) {
            tx.read(Key::part(part.as_str()));
        }
        Ok((command, ()))
    }
}

/// The chart `address` names in `docx` (the first drawing or group member with that id):
/// its part and the part's root; why it cannot be read where the part is missing or not
/// XML.
///
/// # Errors
/// `not_found` for an address no drawing has, `refused` for a drawing that is not a chart.
fn word_graphic(docx: &Docx, doc: &str, address: &str) -> Result<Found, ToolError> {
    let drawings = graphics(docx);
    let charts = || drawings.iter().filter(|d| d.kind == "chart").map(|d| d.id.clone());
    let list = call("word_drawings", &[("doc", json!(doc)), ("action", json!("list"))]);
    let way_out = || WayOut::names(charts()).map_or_else(|| WayOut::call(list.clone()), |w| w.and_call(list.clone()));
    let Some(drawing) = drawings.iter().find(|d| d.id == address) else {
        let message = format!("no drawing {}", repr_str(address));
        return Err(ToolError::refusal(Refusal::NotFound, message, way_out()));
    };
    if drawing.kind != "chart" {
        let message = format!("{address} is a {}, not a chart", drawing.kind);
        return Err(ToolError::refusal(Refusal::Refused, message, way_out()));
    }
    let Some(part) = chart_part(docx, &drawing.part, &drawing.element) else {
        return Ok(Err(NoChart::NoPart));
    };
    let Ok(Some(tree)) = docx.package.tree(&part) else {
        return Ok(Err(NoChart::NotXml));
    };
    Ok(Ok(Graphic {
        holder: drawing.part.clone(),
        chart: part,
        frame: address.to_owned(),
        package: docx.package.clone(),
        root: Arc::clone(tree.root()),
    }))
}
