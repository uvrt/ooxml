//! Owns: `edit_chart`'s handlers (docx-agent's `tools/shared.py`, `edit_chart`): `read` on
//! the version the call pins -- the chart the target names (the first drawing or group
//! member with that id, docx-agent's `Document.chart`) and its reply (`chart_read`) --
//! the data actions that change points in place (`set_values`, `set_value`,
//! `rename_series`, `rename_category`: their series and category named as `chart_names`
//! resolves them, the edit `chart_data`'s, one [`EditGraphic`] command) through the editor.
//!
//! Not here yet: `add`, the structural data actions (categories and series added or
//! removed), the title, legend, label and gap actions, and the style actions answer
//! `unported` (lane P4c's later parts); so do values given `from` a table (its provenance
//! records), and a read or edit ooxml-edit words as its own exception (a chart part that
//! is missing or unreadable, a workbook `ooxml-sheets` cannot read or write), so a host that
//! owns the document answers those its own way meanwhile.
//!
//! Must not: change the document but through a command.

use super::chart_data::{ChartDataEdit, DataAction};
use super::chart_names::{category_names, resolve, series_names, Named};
use super::chart_read::{chart_json, chart_read_json};
use super::{ChartAction, ChartPosition, EditChart};
use crate::structure::common::{call, policy, refused};
use crate::{Editing, Version};
use ooxml_base::pyfmt::repr_str;
use ooxml_base::Cancel;
use ooxml_chart::data::{chart_data, ChartData};
use ooxml_tools::TryCall;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolError, ToolResult, WayOut};
use ooxml_word_edit::EditGraphic;
use ooxml_word_model::docx::drawings::graphics;
use ooxml_word_model::docx::graphics::chart_part;
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use serde_json::{json, Value};
use std::sync::Arc;

/// `edit_chart`'s `read`: what the chart holds and how it looks, changing nothing.
///
/// # Errors
/// `not_found` for a target no drawing has, `refused` for a drawing that is not a chart,
/// `timeout` when the call is cancelled.
pub fn edit_chart_read(args: &EditChart, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    crate::review::stopped(cancel)?;
    let docx = version.docx()?;
    let summary = format!("Read {}", args.target);
    let (part, root) = match chart_of(args, &docx, "read")? {
        Ok(found) => found,
        Err(why) => return Ok(unported(summary, &why)),
    };
    crate::review::stopped(cancel)?;
    match chart_read_json(&args.target, &docx.package, &part, &root) {
        Ok(data) => {
            let mut done = Done::new(summary);
            done.data = Some(data);
            Ok(done)
        }
        Err(why) => Ok(unported(summary, why.0)),
    }
}

/// `edit_chart`'s actions that change the chart.
#[must_use]
pub fn edit_chart(args: &EditChart, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let action = args.action.as_str();
    let summary = format!("{action} on {}", args.target);
    let ported = !matches!(
        args.action,
        ChartAction::Read | ChartAction::Add | ChartAction::SetTitle | ChartAction::SetAxisTitle
    ) && !STYLE_ACTIONS.contains(&args.action);
    if !ported {
        return ToolResult::done(unported(summary, &format!("edit_chart {action}")));
    }
    if args.from.is_some() || args.columns.is_some() || args.date_format.is_some() || args.blanks.is_some() {
        // values from a table, with their provenance records: the host's (KeepText later)
        return ToolResult::done(unported(summary, &format!("edit_chart {action} from")));
    }
    match data_action(args, editing, summary) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

/// The style actions (ooxml-edit's `STYLE_ACTIONS`), not ported yet.
const STYLE_ACTIONS: [ChartAction; 7] = [
    ChartAction::SetAxisScale,
    ChartAction::SetGridlines,
    ChartAction::FormatSeries,
    ChartAction::FormatPoint,
    ChartAction::SetSeriesType,
    ChartAction::SetTextStyle,
    ChartAction::SetPointLabels,
];

/// Word's look for value labels: their size (`WORD_LOOK.data_labels`, hundredths of a point).
const WORD_LABEL_SIZE: u32 = 900;

/// A data action: the chart found and read on the head, its series and category resolved,
/// then one command.
fn data_action(args: &EditChart, editing: &Editing<'_>, summary: String) -> Result<ToolResult<Value>, ToolError> {
    let read = call(
        "edit_chart",
        &[
            ("doc", json!(args.doc)),
            ("target", json!(args.target)),
            ("action", json!("read")),
        ],
    );
    let head = editing.editor.store().snapshot();
    let docx = head
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let read_data = |found: Result<(String, Arc<Element>), String>| {
        found.and_then(|(_, root)| {
            chart_data(&root)
                .map(|data| (data, root))
                .map_err(|_| "edit_chart: a chart whose data the model cannot read".to_owned())
        })
    };
    let data = match read_data(chart_of(args, &docx, args.action.as_str())?) {
        Ok(data) => data,
        Err(why) => return Ok(ToolResult::done(unported(summary, &why))),
    };
    let action = match planned(args, &data.0, &read, &docx) {
        Ok(action) => action,
        Err(Stop::Refused(error)) => return Err(error),
        Err(Stop::Unported(why)) => return Ok(ToolResult::done(unported(summary, why))),
    };
    let command = EditGraphic {
        drawing: args.target.clone(),
        edit: ChartDataEdit {
            address: args.target.clone(),
            action,
        },
        policy: policy(editing),
    };
    let done = match editing.editor.run(editing.caller, &command) {
        Ok(done) => done,
        Err(error) => return Ok(refused(error, read)),
    };
    let after = done
        .version
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let data = match read_data(chart_of(args, &after, args.action.as_str())?) {
        Ok((data, root)) => Value::Object(chart_json(&args.target, &root, &data)),
        Err(_) => Value::Null,
    };
    let mut out = Done::new(summary);
    out.changed.push(args.target.clone());
    out.data = Some(data);
    let mut result = ToolResult::done(out);
    result.warnings = done.out.warnings.into_iter().chain(done.out.out).collect();
    Ok(result)
}

/// The chart the target names (the first drawing or group member with that id): its part
/// and the part's root; the reason the call (`what`, its action) is left to the host where
/// the part is missing or not XML.
///
/// # Errors
/// `not_found` for a target no drawing has, `refused` for a drawing that is not a chart.
fn chart_of(args: &EditChart, docx: &Docx, what: &str) -> Result<Result<(String, Arc<Element>), String>, ToolError> {
    let drawings = graphics(docx);
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
    let Some(part) = chart_part(docx, &drawing.part, &drawing.element) else {
        return Ok(Err(format!("edit_chart {what}: a chart whose part is missing")));
    };
    let Ok(Some(tree)) = docx.package.tree(&part) else {
        return Ok(Err(format!(
            "edit_chart {what}: a chart part that is missing or not XML"
        )));
    };
    Ok(Ok((part, Arc::clone(tree.root()))))
}

/// Why a data action is not planned: refused, or left to the host.
enum Stop {
    Refused(ToolError),
    Unported(&'static str),
}

/// The action with its series and category resolved (`apply_data_action`, in its order:
/// the series, the category, then what the action needs).
fn planned(args: &EditChart, data: &ChartData, read: &TryCall, docx: &Docx) -> Result<DataAction, Stop> {
    let action = args.action.as_str();
    let named = |names: Vec<String>, which: Option<&String>, what: &str, hint: &str| match resolve(
        &names,
        which.map(String::as_str),
        (what, action, hint),
        read,
    ) {
        Named::At(at) => Ok(at),
        Named::Refused(error) => Err(Stop::Refused(error)),
        Named::Unported(why) => Err(Stop::Unported(why)),
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
        _ => shape_action(args, read, docx, &series, &category, &want)?,
    })
}

/// The actions that change the data's shape or how it shows, planned.
fn shape_action(
    args: &EditChart,
    read: &TryCall,
    docx: &Docx,
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
            let lang = default_language(docx);
            if show && lang.is_none() {
                return Err(Stop::Unported(
                    "edit_chart show_data_labels: a document whose defaults name no language (the session's locale decides)",
                ));
            }
            DataAction::DataLabels {
                show,
                series,
                number_format: args.number_format.clone(),
                size: Some(WORD_LABEL_SIZE),
                lang,
            }
        }
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

/// `default_language`'s first answer: the document's default run language
/// (`w:docDefaults/w:rPrDefault/w:rPr/w:lang`); `None` where it names none, and the
/// session's locale (which the host holds) decides.
fn default_language(docx: &Docx) -> Option<String> {
    const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let part = ooxml_word_model::parts::related(&docx.package, &docx.main, STYLES).ok()??;
    let tree = docx.package.tree(&part).ok()??;
    let mut node: &Element = tree.root();
    for name in ["docDefaults", "rPrDefault", "rPr", "lang"] {
        node = node.find(Some(W), name)?;
    }
    node.attribute(Some(W), "val")
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

fn unported(summary: String, what: &str) -> Done<Value> {
    let mut done = Done::new(summary);
    done.unported.push(what.to_owned());
    done
}

/// Registers `edit_chart` (the group's [`super::register`]): the host asks the session first
/// (its data actions that change points in place; the rest answer `unported`).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.reading_edit::<EditChart>(edit_chart_read, edit_chart, crate::Hosting::SERVED);
}
