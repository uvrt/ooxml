//! Owns: `edit_chart` as Word serves it (docx-agent's `tools/shared.py`, `edit_chart`):
//! its handlers over Word's hosts of the chart tools (`graphics`) -- `read`, and the
//! style actions, are the chart tools' (`ooxml_tools_chart`) -- the data actions
//! (`apply_data_action`: points changed in place, categories and series added and removed,
//! the titles, the legend, value labels and the gap width; their series and category
//! named as `ooxml_tools_chart::names` resolves them, the edit `chart_data`'s, the chart's
//! record of where its values came from kept true by `chart_record`) through Word's
//! writer.
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
use super::graphics::{VersionGraphics, WordGraphics};
use crate::{Editing, Version};
use ooxml_base::Cancel;
use ooxml_chart::data::ChartData;
use ooxml_tools::TryCall;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolError, ToolResult, WayOut};
use ooxml_tools_chart::names::{category_names, position, series_names, Stop};
use ooxml_tools_chart::{
    changed, not_supported, read_call, style_action, ChartAction, ChartAxis, ChartPosition, EditChart, STYLE_ACTIONS,
};
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
    let action = args.action.as_str();
    let summary = format!("{action} on {}", args.target);
    if args.action == ChartAction::Read {
        return ToolResult::failed(not_supported(args, &format!("edit_chart {action}")));
    }
    if args.from.is_some() || args.columns.is_some() || args.date_format.is_some() || args.blanks.is_some() {
        // values from a table, with their provenance records: the host's (KeepText later)
        return ToolResult::failed(not_supported(args, &format!("edit_chart {action} from")));
    }
    let writer = WordGraphics {
        editing,
        doc: &args.doc,
    };
    let ran = if args.action == ChartAction::Add {
        let lang = editing
            .editor
            .current()
            .doc()
            .docx()
            .map(|docx| chart_language(&docx, editing));
        match lang {
            Ok(lang) => super::chart_add::add(args, editing, lang),
            Err(e) => Err(ToolError::internal(e.to_string(), "word:model")),
        }
    } else if STYLE_ACTIONS.contains(&args.action) {
        style_action(args, &writer, summary)
    } else {
        let read = read_call(args);
        let edit = |data: &ChartData, _: &Element, lang: &str| {
            planned(args, data, &read, lang).map(|action| ChartDataEdit {
                address: args.target.clone(),
                action,
            })
        };
        changed(args, &writer, summary, false, &edit)
    };
    match ran {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

/// Word's look for value labels: their size (`WORD_LOOK.data_labels`, hundredths of a point).
const WORD_LABEL_SIZE: u32 = 900;
/// Word's look for titles: a chart's and an axis' (`WORD_LOOK.title`, `.axis_title`).
const WORD_TITLE_SIZES: (u32, u32) = (1400, 1000);

/// The action with its series and category resolved (`apply_data_action`, in its order:
/// the series, the category, then what the action needs).
pub(super) fn planned(args: &EditChart, data: &ChartData, read: &TryCall, lang: &str) -> Result<DataAction, Stop> {
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
