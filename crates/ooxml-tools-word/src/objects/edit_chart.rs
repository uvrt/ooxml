//! Owns: `edit_chart`'s handlers (docx-agent's `tools/shared.py`, `edit_chart`): `read` on
//! the version the call pins -- the chart the target names (the first drawing or group
//! member with that id, docx-agent's `Document.chart`) and its reply (`chart_read`) --
//! the data actions (`apply_data_action`: points changed in place, categories and series
//! added and removed, the titles, the legend, value labels and the gap width; their series
//! and category named as `chart_names` resolves them, the edit `chart_data`'s, one
//! [`EditGraphic`] command, the chart's record of where its values came from kept true by
//! `chart_record`) through the editor -- and the style actions (`chart_style`).
//!
//! And `add` (`chart_add`). New chart text is in the document's default language, else the
//! call's locale's (the session's, en-US where the host sets none: [`chart_language`]).
//! Not here yet: values given `from` a table (its provenance records), and a read
//! or edit ooxml-edit words as its own exception (a chart part that is missing or
//! unreadable, a workbook `ooxml-sheets` cannot read or write), so a host that owns the
//! document answers those its own way meanwhile.
//!
//! Must not: change the document but through a command.

use super::chart_data::{ChartDataEdit, DataAction};
use super::chart_names::{category_names, position, series_names, Stop};
use super::chart_read::{chart_json, chart_read_json, style_json};
use super::chart_style;
use super::chart_style::{ChartStyleEdit, StyleAction};
use super::{ChartAction, ChartAxis, ChartPosition, EditChart};
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
        Err(why) => return Err(not_supported(args, &why)),
    };
    crate::review::stopped(cancel)?;
    match chart_read_json(&args.target, &docx.package, &part, &root) {
        Ok(data) => {
            let mut done = Done::new(summary);
            done.data = Some(data);
            Ok(done)
        }
        Err(why) => Err(not_supported(args, why.0)),
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
    if args.action == ChartAction::Read {
        return ToolResult::failed(not_supported(args, &format!("edit_chart {action}")));
    }
    if args.from.is_some() || args.columns.is_some() || args.date_format.is_some() || args.blanks.is_some() {
        // values from a table, with their provenance records: the host's (KeepText later)
        return ToolResult::failed(not_supported(args, &format!("edit_chart {action} from")));
    }
    let ran = if args.action == ChartAction::Add {
        let lang = editing
            .editor
            .store()
            .snapshot()
            .doc()
            .docx()
            .map(|docx| chart_language(&docx, editing));
        match lang {
            Ok(lang) => super::chart_add::add(args, editing, lang),
            Err(e) => Err(ToolError::internal(e.to_string(), "word:model")),
        }
    } else {
        chart_action(args, editing, summary)
    };
    match ran {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

/// The style actions (ooxml-edit's `STYLE_ACTIONS`): `chart_style` plans them.
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
/// Word's look for titles: a chart's and an axis' (`WORD_LOOK.title`, `.axis_title`).
const WORD_TITLE_SIZES: (u32, u32) = (1400, 1000);

/// A data or style action, planned.
enum Planned {
    Data(DataAction),
    Style(StyleAction),
}

/// A data or style action: the chart found and read on the head, its series and category
/// resolved, then one command.
fn chart_action(args: &EditChart, editing: &Editing<'_>, summary: String) -> Result<ToolResult<Value>, ToolError> {
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
        Err(why) => return Err(not_supported(args, &why)),
    };
    let style = STYLE_ACTIONS.contains(&args.action);
    let action = if style {
        chart_style::planned(args, &data.0, &data.1, &read, chart_language(&docx, editing)).map(Planned::Style)
    } else {
        planned(args, &data.0, &read, &chart_language(&docx, editing)).map(Planned::Data)
    };
    let action = match action {
        Ok(action) => action,
        Err(Stop::Refused(error)) => return Err(error),
        Err(Stop::NotSupported(why)) => return Err(not_supported(args, why)),
    };
    let (drawing, address) = (args.target.clone(), args.target.clone());
    let ran = match action {
        Planned::Data(action) => editing.editor.run(
            editing.caller,
            &EditGraphic {
                drawing,
                edit: ChartDataEdit { address, action },
                policy: policy(editing),
            },
        ),
        Planned::Style(action) => editing.editor.run(
            editing.caller,
            &EditGraphic {
                drawing,
                edit: ChartStyleEdit { address, action },
                policy: policy(editing),
            },
        ),
    };
    let done = match ran {
        Ok(done) => done,
        Err(error) => return Ok(refused(error, read)),
    };
    let after = done
        .version
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let data = match read_data(chart_of(args, &after, args.action.as_str())?) {
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

/// The action with its series and category resolved (`apply_data_action`, in its order:
/// the series, the category, then what the action needs).
fn planned(args: &EditChart, data: &ChartData, read: &TryCall, lang: &str) -> Result<DataAction, Stop> {
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

/// `what` refused as not supported yet, reading the chart as the way out.
fn not_supported(args: &EditChart, what: &str) -> ToolError {
    let read = json!({"doc": args.doc, "target": args.target, "action": "read"});
    crate::refusal::not_supported(what, "edit_chart", read, &[])
}

/// Registers `edit_chart` (the group's [`super::register`]): the host asks the session first
/// (its data actions that change points in place; the rest are refused as not supported yet).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.reading_edit::<EditChart>(edit_chart_read, edit_chart, crate::Hosting::SERVED);
}
