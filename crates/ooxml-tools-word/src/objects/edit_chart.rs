//! Owns: `edit_chart`'s handlers (docx-agent's `tools/shared.py`, `edit_chart`): `read` on
//! the version the call pins -- the chart the target names (the first drawing or group
//! member with that id, docx-agent's `Document.chart`) and its reply (`chart_read`) --
//! every other action through the editor.
//!
//! Not here yet: the data actions and the style actions answer `unported` (lane P4c's
//! later parts), as does a read ooxml-edit words as its own exception (a chart part that is
//! missing or unreadable, a workbook `ooxml-sheets` cannot read), so a host that owns the
//! document answers those its own way meanwhile.
//!
//! Must not: change the document but through a command.

use super::chart_read::chart_read_json;
use super::EditChart;
use crate::structure::common::call;
use crate::{Editing, Version};
use ooxml_base::pyfmt::repr_str;
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolError, ToolResult, WayOut};
use ooxml_word_model::docx::drawings::graphics;
use ooxml_word_model::docx::graphics::chart_part;
use serde_json::{json, Value};

/// `edit_chart`'s `read`: what the chart holds and how it looks, changing nothing.
///
/// # Errors
/// `not_found` for a target no drawing has, `refused` for a drawing that is not a chart,
/// `timeout` when the call is cancelled.
pub fn edit_chart_read(args: &EditChart, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    crate::review::stopped(cancel)?;
    let docx = version.docx()?;
    let drawings = graphics(&docx);
    let charts = || drawings.iter().filter(|d| d.kind == "chart").map(|d| d.id.clone());
    let list = call("word_drawings", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    let way_out = || WayOut::names(charts()).map_or_else(|| WayOut::call(list.clone()), |w| w.and_call(list.clone()));
    let Some(drawing) = drawings.iter().find(|d| d.id == args.target) else {
        let message = format!("no drawing {}", repr_str(&args.target));
        return Err(ToolError::refusal(Refusal::NotFound, message, way_out()));
    };
    if drawing.kind != "chart" {
        let message = format!("{} is a {}, not a chart", args.target, drawing.kind);
        return Err(ToolError::refusal(Refusal::Refused, message, way_out()));
    }
    let summary = format!("Read {}", args.target);
    let Some(part) = chart_part(&docx, &drawing.part, &drawing.element) else {
        return Ok(unported(summary, "edit_chart read: a chart whose part is missing"));
    };
    let Ok(Some(tree)) = docx.package.tree(&part) else {
        return Ok(unported(
            summary,
            "edit_chart read: a chart part that is missing or not XML",
        ));
    };
    crate::review::stopped(cancel)?;
    match chart_read_json(&args.target, &docx.package, &part, tree.root()) {
        Ok(data) => {
            let mut done = Done::new(summary);
            done.data = Some(data);
            Ok(done)
        }
        Err(why) => Ok(unported(summary, why.0)),
    }
}

/// `edit_chart`'s actions that change the chart.
pub fn edit_chart(args: &EditChart, _: &Editing<'_>) -> ToolResult<Value> {
    let action = args.action.as_str();
    ToolResult::done(unported(
        format!("{action} on {}", args.target),
        &format!("edit_chart {action}"),
    ))
}

fn unported(summary: String, what: &str) -> Done<Value> {
    let mut done = Done::new(summary);
    done.unported.push(what.to_owned());
    done
}
