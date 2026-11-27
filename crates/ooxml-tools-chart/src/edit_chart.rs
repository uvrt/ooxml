//! Owns: `edit_chart`'s handlers over a format's host (docx-agent's `tools/shared.py`,
//! `edit_chart`): `read` on the version the call pins -- the chart the target names
//! ([`GraphicHost::find`]) and its reply (`read`) -- and the actions that change the chart
//! ([`changed`]: the style actions, `style`'s, through [`GraphicWriter::write`]), with their
//! reply: the chart as the version made holds it.
//!
//! Must not: find a frame or apply an edit (the host does).

use crate::host::{Found, GraphicHost, GraphicWriter};
use crate::names::Stop;
use crate::read::{chart_json, chart_read_json, style_json};
use crate::style::ChartStyleEdit;
use crate::{style, ChartAction, EditChart, Warnings};
use ooxml_chart::data::{chart_data, ChartData};
use ooxml_chart::edit::GraphicEdit;
use ooxml_tools::{ArgEnum, Done, ToolError, ToolResult, TryCall, WayOut};
use ooxml_xml::Element;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// `edit_chart`'s `read`: what the chart holds and how it looks, changing nothing.
///
/// # Errors
/// The host's refusal of the target (`not_found`, `refused`, `timeout`); a chart the model
/// cannot read refused as not supported yet.
pub fn edit_chart_read(args: &EditChart, host: &impl GraphicHost) -> Result<Done<Value>, ToolError> {
    let summary = format!("Read {}", args.target);
    let graphic = match host.find(&args.target)? {
        Ok(found) => found,
        Err(why) => return Err(not_supported(args, &why.said("read"))),
    };
    match chart_read_json(&args.target, &graphic.package, &graphic.chart, &graphic.root) {
        Ok(data) => {
            let mut done = Done::new(summary);
            done.data = Some(data);
            Ok(done)
        }
        Err(why) => Err(not_supported(args, why.0)),
    }
}

/// The style actions (ooxml-edit's `STYLE_ACTIONS`): `style` plans them.
pub const STYLE_ACTIONS: [ChartAction; 7] = [
    ChartAction::SetAxisScale,
    ChartAction::SetGridlines,
    ChartAction::FormatSeries,
    ChartAction::FormatPoint,
    ChartAction::SetSeriesType,
    ChartAction::SetTextStyle,
    ChartAction::SetPointLabels,
];

/// The call that reads the chart: every refusal's way out.
#[must_use]
pub fn read_call(args: &EditChart) -> TryCall {
    let arguments: Map<String, Value> = [
        ("doc".to_owned(), json!(args.doc)),
        ("target".to_owned(), json!(args.target)),
        ("action".to_owned(), json!("read")),
    ]
    .into_iter()
    .collect();
    TryCall {
        tool: "edit_chart".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// A style action (one of [`STYLE_ACTIONS`]): planned by `style` where the edit is applied.
///
/// # Errors
/// As [`changed`].
pub fn style_action(
    args: &EditChart,
    writer: &impl GraphicWriter,
    summary: String,
) -> Result<ToolResult<Value>, ToolError> {
    let read = read_call(args);
    let edit = |data: &ChartData, root: &Element, lang: &str| {
        style::planned(args, data, root, &read, lang.to_owned()).map(|action| ChartStyleEdit {
            address: args.target.clone(),
            action,
        })
    };
    changed(args, writer, summary, true, &edit)
}

/// A data or style action (`style`: one of [`STYLE_ACTIONS`]): the chart found and read
/// where the edit is applied, `edit` working it out from its data, its root and the
/// language new text is written in, as one edit; the reply the chart as the version made
/// holds it (with its look, for a style action).
///
/// # Errors
/// The writer's refusal of the target, the plan's or the edit's.
pub fn changed<E: GraphicEdit<Out = Warnings>>(
    args: &EditChart,
    writer: &impl GraphicWriter,
    summary: String,
    style: bool,
    edit: &dyn Fn(&ChartData, &Element, &str) -> Result<E, Stop>,
) -> Result<ToolResult<Value>, ToolError> {
    let read = read_call(args);
    // the series and categories are named as indices in the edit's transaction, on the
    // version it is applied to: a series added or removed meanwhile is a conflict, and the
    // names are resolved again
    let written = writer.write(&args.target, &read, &planner(args, edit))?;
    let data = match chart_data_of(&written.after, args.action.as_str()) {
        Ok((data, root)) => {
            let mut out = chart_json(&args.target, &root, &data);
            // (the look's point labels unreadable: no data, as for a chart the model cannot read)
            if style && style_json(&root, &data, &mut out).is_err() {
                Value::Null
            } else {
                Value::Object(out)
            }
        }
        Err(_) => Value::Null,
    };
    let mut out = Done::new(summary);
    out.changed.push(args.target.clone());
    out.data = Some(data);
    let mut result = ToolResult::done(out);
    result.warnings = written.warnings;
    Ok(result)
}

/// The plan [`GraphicWriter::write`] makes on the version the edit is applied to: the
/// chart found there read, `edit` working the edit out from its data and root in the
/// language given; a chart the model cannot read, and a form `edit` does not support yet,
/// refused as not supported yet.
pub fn planner<'a, E>(
    args: &'a EditChart,
    edit: &'a dyn Fn(&ChartData, &Element, &str) -> Result<E, Stop>,
) -> impl Fn(&Found, &str) -> Result<E, ToolError> + 'a {
    move |found: &Found, lang: &str| {
        let (data, root) = match chart_data_of(found, args.action.as_str()) {
            Ok(data) => data,
            Err(why) => return Err(not_supported(args, &why)),
        };
        match edit(&data, &root, lang) {
            Ok(edit) => Ok(edit),
            Err(Stop::Refused(error)) => Err(error),
            Err(Stop::NotSupported(why)) => Err(not_supported(args, why)),
        }
    }
}

/// The chart found with its data read; why the call is not supported yet (for `action`)
/// where the model cannot read it.
fn chart_data_of(found: &Found, action: &str) -> Result<(ChartData, Arc<Element>), String> {
    match found {
        Ok(graphic) => chart_data(&graphic.root)
            .map(|data| (data, Arc::clone(&graphic.root)))
            .map_err(|_| "edit_chart: a chart whose data the model cannot read".to_owned()),
        Err(why) => Err(why.said(action)),
    }
}

/// `what` refused as not supported yet, reading the chart as the way out.
#[must_use]
pub fn not_supported(args: &EditChart, what: &str) -> ToolError {
    ToolError::not_supported(what, WayOut::call(read_call(args)))
}
