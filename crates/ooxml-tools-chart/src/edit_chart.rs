//! Owns: `edit_chart`'s handlers over a format's host (docx-agent's `tools/shared.py`,
//! `edit_chart`): `read` on the version the call pins -- the chart the target names
//! ([`GraphicHost::find`]) and its reply (`read`) -- and the actions that change the chart
//! through the host's writer: `add` (`add`), the style actions (`style`'s) and the data
//! actions (`apply_data_action`: points changed in place, categories and series added and
//! removed, the titles, the legend, value labels and the gap width; their series and
//! category named as `names` resolves them, the edit `data`'s, the chart's record of where
//! its values came from kept true by `record`), each planned where the edit is applied
//! ([`GraphicWriter::write`]), with their reply: the chart as the version made holds it.
//!
//! Must not: find a frame or apply an edit (the host does).

use crate::data::{ChartDataEdit, DataAction};
use crate::host::{Found, GraphicHost, GraphicWriter};
use crate::names::{category_names, position, series_names, Stop};
use crate::read::{chart_json, chart_read_json, style_json};
use crate::style::ChartStyleEdit;
use crate::{style, ChartAction, ChartAxis, ChartPosition, EditChart, Warnings};
use ooxml_chart::data::{chart_data, ChartData};
use ooxml_chart::edit::GraphicEdit;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolError, ToolResult, TryCall, WayOut};
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

/// `edit_chart`'s actions that change the chart, through the host's writer: `add`
/// (`add`), the style actions (`style`) and the data actions (`data`), each as one edit;
/// `read`, and values given `from` a table (their provenance records: the host's, KeepText
/// later), refused as not supported here.
#[must_use]
pub fn edit_chart(args: &EditChart, writer: &impl GraphicWriter) -> ToolResult<Value> {
    let action = args.action.as_str();
    let summary = format!("{action} on {}", args.target);
    if args.action == ChartAction::Read {
        return ToolResult::failed(not_supported(args, &format!("edit_chart {action}")));
    }
    if args.from.is_some() || args.columns.is_some() || args.date_format.is_some() || args.blanks.is_some() {
        // values from a table, with their provenance records: the host's (KeepText later)
        return ToolResult::failed(not_supported(args, &format!("edit_chart {action} from")));
    }
    let ran = if args.action == ChartAction::Add {
        crate::add::add(args, writer)
    } else if STYLE_ACTIONS.contains(&args.action) {
        style_action(args, writer, summary)
    } else {
        let read = read_call(args);
        let edit = |data: &ChartData, _: &Element, lang: &str| {
            planned(args, data, &read, lang).map(|action| ChartDataEdit {
                address: args.target.clone(),
                action,
            })
        };
        changed(args, writer, summary, false, &edit)
    };
    match ran {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

/// The style actions (ooxml-edit's `STYLE_ACTIONS`): `style` plans them.
pub(crate) const STYLE_ACTIONS: [ChartAction; 7] = [
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
pub(crate) fn read_call(args: &EditChart) -> TryCall {
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
pub(crate) fn style_action(
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
pub(crate) fn changed<E: GraphicEdit<Out = Warnings>>(
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
pub(crate) fn not_supported(args: &EditChart, what: &str) -> ToolError {
    ToolError::not_supported(what, WayOut::call(read_call(args)))
}

/// Word's look for value labels: their size (`WORD_LOOK.data_labels`, hundredths of a point).
const WORD_LABEL_SIZE: u32 = 900;
/// Word's look for titles: a chart's and an axis' (`WORD_LOOK.title`, `.axis_title`).
const WORD_TITLE_SIZES: (u32, u32) = (1400, 1000);

/// The action with its series and category resolved (`apply_data_action`, in its order:
/// the series, the category, then what the action needs).
pub fn planned(args: &EditChart, data: &ChartData, read: &TryCall, lang: &str) -> Result<DataAction, Stop> {
    let action = args.action.as_str();
    let named = |names: Vec<String>, which: Option<&String>, what: &str, hint: &str| {
        position(&names, which, (what, action, hint), read)
    };
    let hint = if data.category_dates.is_some() {
        "; its categories are dates, named as YYYY-MM-DD"
    } else {
        ""
    };
    let series = || named(series_names(data), args.series.as_ref(), "series", "");
    let category = || named(category_names(data), args.category.as_ref(), "category", hint);
    let want = |name: &str, given: bool, expected: &str| {
        if given {
            return Ok(());
        }
        let message = format!("{action} needs {name}: {expected}");
        let refusal = ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(read.clone()));
        Err(Stop::Refused(refusal.at(name)))
    };
    let text = || args.text.clone().unwrap_or_default();
    Ok(match args.action {
        ChartAction::SetValues => {
            let series = series()?;
            let expected = format!("one number per category ({})", data.categories.len());
            want("values", args.values.is_some(), &expected)?;
            let values = args.values.clone().unwrap_or_default();
            DataAction::SetValues { series, values }
        }
        ChartAction::SetValue => {
            let (series, category) = (series()?, category()?);
            want("value", args.value.is_some(), "the number")?;
            let value = args.value.unwrap_or_default();
            DataAction::SetValue {
                series,
                category,
                value,
            }
        }
        ChartAction::RenameSeries => {
            let series = series()?;
            want("text", args.text.is_some(), "the new name")?;
            DataAction::RenameSeries { series, text: text() }
        }
        ChartAction::RenameCategory => {
            let category = category()?;
            want("text", args.text.is_some(), "the new label")?;
            DataAction::RenameCategory { category, text: text() }
        }
        _ => shape_action(args, data, read, lang, &series, &category, &want)?,
    })
}

/// The actions that change the data's shape or how it shows, planned.
#[allow(clippy::too_many_arguments)]
fn shape_action(
    args: &EditChart,
    data: &ChartData,
    read: &TryCall,
    lang: &str,
    series: &dyn Fn() -> Result<usize, Stop>,
    category: &dyn Fn() -> Result<usize, Stop>,
    want: &dyn Fn(&str, bool, &str) -> Result<(), Stop>,
) -> Result<DataAction, Stop> {
    let text = || args.text.clone().unwrap_or_default();
    Ok(match args.action {
        ChartAction::AddCategory => {
            want("text", args.text.is_some(), "the new category's label")?;
            DataAction::AddCategory {
                text: text(),
                values: args.values.clone(),
            }
        }
        ChartAction::RemoveCategory => DataAction::RemoveCategory { category: category()? },
        ChartAction::AddSeries => {
            want("text", args.text.is_some(), "the new series' name")?;
            DataAction::AddSeries {
                text: text(),
                values: args.values.clone(),
            }
        }
        ChartAction::RemoveSeries => DataAction::RemoveSeries { series: series()? },
        ChartAction::SetLegend => DataAction::SetLegend {
            position: legend_position(args.position, read)?,
        },
        ChartAction::ShowDataLabels | ChartAction::HideDataLabels => {
            let show = args.action == ChartAction::ShowDataLabels;
            let series = args.series.as_ref().map(|_| series()).transpose()?;
            DataAction::DataLabels {
                show,
                series,
                number_format: args.number_format.clone(),
                size: Some(WORD_LABEL_SIZE),
                lang: lang.to_owned(),
            }
        }
        ChartAction::SetTitle => DataAction::Title {
            axis: None,
            text: args.text.clone(),
            sizes: WORD_TITLE_SIZES,
            lang: lang.to_owned(),
        },
        ChartAction::SetAxisTitle => DataAction::Title {
            axis: Some(axis_of(args, data, read, want)?),
            text: args.text.clone(),
            sizes: WORD_TITLE_SIZES,
            lang: lang.to_owned(),
        },
        _ => {
            want(
                "value",
                args.value.is_some(),
                "the gap as a whole % of a bar's width, 0-500",
            )?;
            let given = args.value.unwrap_or_default();
            if given.fract() != 0.0 || !given.is_finite() {
                let message = "a gap width is a whole percentage, 0-500";
                let refusal = ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(read.clone()));
                return Err(Stop::Refused(refusal.at("value")));
            }
            #[allow(clippy::cast_possible_truncation)] // whole, and checked 0-500 after
            DataAction::GapWidth { percent: given as i64 }
        }
    })
}

/// `set_axis_title`'s axis: given (`need`: the chart's axes said), `category` or `value`
/// (`_axis_of`), and one the chart has (`axis_title`).
fn axis_of(
    args: &EditChart,
    data: &ChartData,
    read: &TryCall,
    want: &dyn Fn(&str, bool, &str) -> Result<(), Stop>,
) -> Result<&'static str, Stop> {
    let axes: Vec<&str> = data.axis_titles.iter().map(|(name, _)| *name).collect();
    let expected = if axes.is_empty() {
        "the chart has no axes".to_owned()
    } else {
        axes.join(" or ")
    };
    want("axis", args.axis.is_some(), &expected)?;
    let axis = match args.axis {
        Some(ChartAxis::Category) => "category",
        Some(ChartAxis::Value) => "value",
        _ => {
            let message = "axis is 'category' or 'value'";
            let refusal = ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(read.clone()));
            return Err(Stop::Refused(refusal));
        }
    };
    if !axes.contains(&axis) {
        let message = format!("{}: the chart has no {axis} axis", args.target);
        let refusal = ToolError::refusal(Refusal::NotFound, message, WayOut::call(read.clone()));
        return Err(Stop::Refused(refusal));
    }
    Ok(axis)
}

/// `legend_position`: the legend's position as the chart names it (`None`: no legend; the
/// right where none is given); a label position refused.
fn legend_position(given: Option<ChartPosition>, read: &TryCall) -> Result<Option<&'static str>, Stop> {
    Ok(match given.unwrap_or(ChartPosition::Right) {
        ChartPosition::Right => Some("r"),
        ChartPosition::Left => Some("l"),
        ChartPosition::Top => Some("t"),
        ChartPosition::Bottom => Some("b"),
        ChartPosition::None => None,
        other => {
            let message = format!(
                "{} is a label position; a legend is right, left, top, bottom or none",
                other.as_str()
            );
            let refusal = ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(read.clone()));
            return Err(Stop::Refused(refusal.at("position")));
        }
    })
}
