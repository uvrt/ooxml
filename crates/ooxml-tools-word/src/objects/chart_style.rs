//! Owns: `edit_chart`'s style actions (docx-agent's `_style_chart` through ooxml-edit's
//! `apply_style_action`, `charts/actions.py`): `set_axis_scale` (the axis a scale given
//! without one is for, `scale_axis`; its keys as keywords, `scale_arguments`),
//! `set_gridlines`, `format_series`, `format_point`, `set_series_type`, `set_text_style` and
//! `set_point_labels` (Value From Cells: the labels written into the workbook too,
//! `chart_block`) -- their arguments checked in ooxml-edit's order and the chart part
//! changed (`ooxml_chart::edit`), one [`EditGraphic`](ooxml_word_edit::EditGraphic) command.
//!
//! Not here yet: point labels in a document whose defaults name no language (the session's
//! locale decides it; the handler answers them `unported`).
//!
//! Must not: resolve a series or category by name but through `chart_names`, or write but
//! through the command.

use super::chart_block::{all_sources, Block};
use super::chart_data::Warnings;
use super::chart_keeper::{refused, unported, Keeper};
use super::chart_names::{category_names, choices, position, series_names, Stop};
use super::{ChartAction, ChartAxis, ChartType, EditChart, Gridlines, ScaleArg, ScaleKey, StyleArg};
use ooxml_chart::data::{axis_names, ChartData, Point};
use ooxml_chart::edit::{
    text_size, ChartTree, Colour, DataError, PointLabels, ScaleChange, SeriesFormat, TextHost, TextLook, TextStop,
};
use ooxml_tools::{ArgEnum, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_edit::{EditError, GraphicEdit, GraphicParts, PartWrite, RecordChange};
use ooxml_xml::view::{child, children};
use ooxml_xml::Element;
use std::sync::Arc;

/// Word's look for text properties made new: axis labels' and the legend's, data labels'
/// (`WORD_LOOK.labels`, `.data_labels`, hundredths of a point).
const WORD_TEXT_SIZES: (u32, u32) = (900, 900);
/// `format_series`' and `format_point`'s style keys (`_SERIES_STYLE`), and
/// `set_text_style`'s (`_TEXT_STYLE`).
const SERIES_STYLE: &str = "fill, line, line_width, dash, marker, marker_size, marker_fill";
const TEXT_STYLE: &str = "font, size, bold, color";
/// What a scale may hold (`_expected(chart, "scale")`).
const SCALE_KEYS: &str = "min, max, major_unit, minor_unit, number_format, reverse or auto";

/// A style action, its series and point resolved to positions.
#[derive(Debug, Clone)]
pub(crate) enum StyleAction {
    /// An axis' scale; `values_format` is the values' number format.
    Scale {
        axis: &'static str,
        change: ScaleChange,
        values_format: Option<String>,
    },
    /// An axis' gridlines shown or taken away.
    Gridlines {
        axis: &'static str,
        major: bool,
        minor: bool,
    },
    /// A series' look, or one point's.
    Format {
        series: usize,
        point: Option<usize>,
        look: SeriesFormat,
    },
    /// A series drawn as a line (on the secondary axes) or a column; `values_format` is
    /// the values' number format.
    SeriesType {
        series: usize,
        line: bool,
        secondary: bool,
        values_format: Option<String>,
    },
    /// A series' own point labels (`None`: taken away) at a position (by name), in the
    /// workbook too with `cells`; their text in `lang`.
    PointLabels {
        series: usize,
        labels: Option<Vec<String>>,
        position: Option<&'static str>,
        cells: bool,
        lang: Option<String>,
    },
    /// The text's look, of one part or all; new text properties in `lang`.
    Text {
        part: Option<&'static str>,
        look: TextLook,
        lang: Option<String>,
    },
}

/// A style action on the chart `address` (the target, as ooxml-edit's messages name it).
#[derive(Debug, Clone)]
pub(crate) struct ChartStyleEdit {
    pub(crate) address: String,
    pub(crate) action: StyleAction,
}

impl GraphicEdit for ChartStyleEdit {
    type Out = Warnings;

    fn edit(&self, package: &ooxml_opc::Package, part: &str) -> Result<GraphicParts<Warnings>, EditError> {
        let tree = package.tree(part)?.ok_or_else(|| EditError::NoPart(part.to_owned()))?;
        let mut chart = ChartTree::new(Arc::clone(tree.root()));
        let address = self.address.as_str();
        let refusal = |error: DataError| refused(&error.0);
        let mut keeper = None;
        let mut changed = true;
        match &self.action {
            StyleAction::Scale {
                axis,
                change,
                values_format,
            } => chart
                .set_axis_scale(address, axis, change, values_format.as_deref())
                .map_err(refusal)?,
            StyleAction::Gridlines { axis, major, minor } => chart
                .set_gridlines(address, axis, Some(*major), Some(*minor))
                .map_err(refusal)?,
            StyleAction::Format { series, point, look } => {
                chart.format_series(address, *series, *point, look).map_err(refusal)?;
            }
            StyleAction::SeriesType {
                series,
                line,
                secondary,
                values_format,
            } => {
                let hash = &ooxml_word_model::ids::sha256;
                changed = chart
                    .set_series_type(address, *series, *line, *secondary, values_format.as_deref(), hash)
                    .map_err(refusal)?;
            }
            StyleAction::PointLabels { .. } => keeper = self.point_labels(&mut chart, package, part)?,
            StyleAction::Text { part, look, lang } => {
                let host = TextHost {
                    labels: Some(WORD_TEXT_SIZES.0),
                    data_labels: Some(WORD_TEXT_SIZES.1),
                    lang: lang.as_deref(),
                };
                chart
                    .set_text_style(address, *part, look, &host)
                    .map_err(|stop| match stop {
                        TextStop::Refused(error) => refusal(error),
                        TextStop::NeedsLanguage(_) => unported(
                            "set_text_style: a document whose defaults name no language (the session's locale decides)",
                        ),
                    })?;
            }
        }
        let mut writes = Vec::new();
        if changed {
            writes.push(PartWrite::Tree(
                part.to_owned(),
                tree.with_root(Arc::clone(chart.root())),
            ));
        }
        let mut read = Vec::new();
        if let Some(keeper) = keeper {
            // a workbook kept in step has no reason to change the cache only
            keeper.finish(&mut writes, &mut read)?;
        }
        Ok(GraphicParts {
            read,
            writes,
            record: RecordChange::Kept,
            out: Vec::new(),
        })
    }
}

impl ChartStyleEdit {
    /// `set_point_labels`: checked, the labels written into the workbook too for Value From
    /// Cells (the workbook kept in step returned), then into the chart part.
    fn point_labels<'p>(
        &self,
        chart: &mut ChartTree,
        package: &'p ooxml_opc::Package,
        part: &'p str,
    ) -> Result<Option<Keeper<'p>>, EditError> {
        let StyleAction::PointLabels {
            series,
            labels,
            position,
            cells,
            lang,
        } = &self.action
        else {
            return Ok(None);
        };
        let address = self.address.as_str();
        let code = chart
            .check_point_labels(address, *series, labels.as_deref(), *position)
            .map_err(|error| refused(&error.0))?;
        let mut keeper = None;
        let formula = match labels {
            Some(labels) if *cells => {
                let mut workbook = Keeper::new(package, part);
                let formula = label_cells(chart, &mut workbook, address, *series, labels)?;
                keeper = Some(workbook);
                Some(formula)
            }
            _ => None,
        };
        let look = PointLabels {
            labels: labels.as_deref(),
            position: code,
            cells: formula.as_deref(),
            size: Some(WORD_TEXT_SIZES.1),
            lang: lang.as_deref(),
            part,
        };
        chart.set_point_labels(*series, &look, &ooxml_word_model::ids::sha256);
        Ok(keeper)
    }
}

/// The style action planned (`_style_chart`, then `apply_style_action`, in their order:
/// the series and category, then what the action needs).
pub(super) fn planned(
    args: &EditChart,
    data: &ChartData,
    root: &Element,
    read: &TryCall,
    lang: Option<String>,
) -> Result<StyleAction, Stop> {
    let action = args.action.as_str();
    let plan = Plan { args, data, root, read };
    let hint = if data.category_dates.is_some() {
        "; its categories are dates, named as YYYY-MM-DD"
    } else {
        ""
    };
    let series = match &args.series {
        Some(which) => Some(position(
            &series_names(data),
            Some(which),
            ("series", action, ""),
            read,
        )?),
        None => None,
    };
    let category = match (&args.category, series) {
        (Some(which), Some(_)) => Some(position(
            &category_names(data),
            Some(which),
            ("category", action, hint),
            read,
        )?),
        _ => None,
    };
    match args.action {
        ChartAction::SetAxisScale => plan.scale(),
        ChartAction::SetGridlines => plan.gridlines(),
        ChartAction::FormatSeries | ChartAction::FormatPoint => plan.format(series, category),
        ChartAction::SetSeriesType => {
            let line = match args.chart_type {
                Some(ChartType::Line) => true,
                Some(ChartType::Column) => false,
                _ => return Err(plan.refuse("set_series_type needs chart_type line or column".to_owned())),
            };
            Ok(StyleAction::SeriesType {
                series: series.ok_or_else(|| plan.need_series())?,
                line,
                secondary: args.secondary.unwrap_or(false),
                values_format: data.format.clone(),
            })
        }
        ChartAction::SetPointLabels => {
            let series = series.ok_or_else(|| plan.need_series())?;
            if args.labels.is_some() && lang.is_none() {
                return Err(Stop::Unported(
                    "set_point_labels: a document whose defaults name no language (the session's locale decides)",
                ));
            }
            Ok(StyleAction::PointLabels {
                series,
                labels: args.labels.clone(),
                position: args.position.map(ArgEnum::as_str),
                cells: args.cells.unwrap_or(false),
                lang,
            })
        }
        ChartAction::SetTextStyle => Ok(StyleAction::Text {
            part: args.part.map(ArgEnum::as_str),
            look: text_look(args.style.as_ref()).map_err(|e| plan.refuse(e.0))?,
            lang,
        }),
        _ => Err(Stop::Unported("edit_chart: this style action")),
    }
}

/// What a style action is planned from.
struct Plan<'a> {
    args: &'a EditChart,
    data: &'a ChartData,
    root: &'a Element,
    read: &'a TryCall,
}

impl Plan<'_> {
    /// A `ChartDataError`, as docx-agent words it: `invalid_arguments`, the message
    /// stripped of quotes at its ends.
    fn refuse(&self, message: String) -> Stop {
        let message = message.trim_matches(['\'', '"']).to_owned();
        Stop::Refused(ToolError::refusal(
            Refusal::InvalidArguments,
            message,
            WayOut::call(self.read.clone()),
        ))
    }

    /// `need("series", ...)`: the action needs a series, one of the chart's.
    fn need_series(&self) -> Stop {
        let names: Vec<String> = self
            .data
            .series
            .iter()
            .map(|s| s.name.as_ref().map_or_else(|| "None".to_owned(), Point::text))
            .collect();
        self.refuse(format!(
            "{} needs series: {}",
            self.args.action.as_str(),
            choices(&names)
        ))
    }

    fn chart_type(&self) -> &str {
        self.data.types.first().map_or("None", String::as_str)
    }

    /// `set_axis_scale`: the scale, the axis it is for, its keywords.
    fn scale(&self) -> Result<StyleAction, Stop> {
        let Some(scale) = &self.args.scale else {
            return Err(self.refuse(format!("set_axis_scale needs scale: {SCALE_KEYS}")));
        };
        let axis = match self.args.axis {
            Some(axis) => axis_name(axis),
            None => {
                let axes = axis_names(self.root);
                scale_axis(&self.args.target, self.root, &axes, self.chart_type(), scale)
                    .map_err(|e| self.refuse(e.0))?
            }
        };
        Ok(StyleAction::Scale {
            axis,
            change: scale_arguments(scale).map_err(|e| self.refuse(e.0))?,
            values_format: self.data.format.clone(),
        })
    }

    /// `set_gridlines`: the axis, then which gridlines.
    fn gridlines(&self) -> Result<StyleAction, Stop> {
        let Some(axis) = self.args.axis else {
            let axes = axis_names(self.root);
            let expected = if axes.is_empty() {
                format!("a {} chart has no axes", self.chart_type())
            } else {
                either(&axes)
            };
            return Err(self.refuse(format!("set_gridlines needs axis: {expected}")));
        };
        let Some(gridlines) = self.args.gridlines else {
            return Err(self.refuse("set_gridlines needs gridlines: none, major, minor or both".to_owned()));
        };
        let (major, minor) = match gridlines {
            Gridlines::None => (false, false),
            Gridlines::Major => (true, false),
            Gridlines::Minor => (false, true),
            Gridlines::Both => (true, true),
        };
        Ok(StyleAction::Gridlines {
            axis: axis_name(axis),
            major,
            minor,
        })
    }

    /// `format_series`, `format_point`: the style, then the series and the point.
    fn format(&self, series: Option<usize>, category: Option<usize>) -> Result<StyleAction, Stop> {
        let action = self.args.action.as_str();
        let look = series_format(self.args.style.as_ref(), action).map_err(|e| self.refuse(e.0))?;
        let need = |name: &str, names: &[String]| self.refuse(format!("{action} needs {name}: {}", choices(names)));
        let Some(series) = series else {
            return Err(self.need_series());
        };
        let point = match (self.args.action, category) {
            (ChartAction::FormatPoint, None) => return Err(need("category", &category_names(self.data))),
            (ChartAction::FormatPoint, at) => at,
            _ => None,
        };
        Ok(StyleAction::Format { series, point, look })
    }
}

/// An axis' name as the chart part's edit takes it.
fn axis_name(axis: ChartAxis) -> &'static str {
    axis.as_str()
}

/// `_either`: `a`, `a or b`, `a, b or c`.
fn either(names: &[&str]) -> String {
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_owned(),
        None => String::new(),
    }
}

/// The scale keys given (Python's `scale` dict keys), in the tool's order.
fn given_keys(scale: &ScaleArg) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (key, given) in [
        ("min", scale.min.is_some()),
        ("max", scale.max.is_some()),
        ("major_unit", scale.major_unit.is_some()),
        ("minor_unit", scale.minor_unit.is_some()),
        ("number_format", scale.number_format.is_some()),
        ("reverse", scale.reverse.is_some()),
    ] {
        if given {
            out.push(key);
        }
    }
    out
}

/// `scale_axis`: the axis a scale given without `axis` is for -- the value axis, where it
/// is the one axis that takes the scale's bounds or units.
fn scale_axis(
    address: &str,
    root: &Element,
    names: &[&str],
    chart_type: &str,
    scale: &ScaleArg,
) -> Result<&'static str, DataError> {
    const NUMERIC: [&str; 4] = ["min", "max", "major_unit", "minor_unit"];
    let action = "set_axis_scale";
    if names.is_empty() {
        return Err(DataError(format!(
            "{address}: a {chart_type} chart has no axes to scale"
        )));
    }
    let auto = scale.auto.as_deref().unwrap_or_default();
    let numeric =
        given_keys(scale).iter().any(|k| NUMERIC.contains(k)) || auto.iter().any(|k| NUMERIC.contains(&k.as_str()));
    if !numeric {
        return Err(DataError(format!(
            "{action} needs axis: {}; reverse and number_format suit either",
            either(names)
        )));
    }
    let area = child(root, "chart").and_then(|c| child(c, "plotArea"));
    let values = area.map_or(0, |a| children(a, "valAx").count());
    let dates = area.is_some_and(|a| children(a, "dateAx").next().is_some());
    if names == ["category", "value"] && values == 1 && !dates {
        return Ok("value");
    }
    if dates {
        return Err(DataError(format!(
            "{action} needs axis: category (its dates take bounds and units too) or value"
        )));
    }
    if names == ["category", "value"] {
        return Err(DataError(format!(
            "{action} needs axis: {address} has a secondary value axis too, which edit_chart does not scale; give value for the primary one"
        )));
    }
    Err(DataError(format!(
        "{action} needs axis: {}; both take bounds and units",
        either(names)
    )))
}

/// `scale_arguments`: a scale as the chart's keywords, `auto` making bounds automatic again.
fn scale_arguments(scale: &ScaleArg) -> Result<ScaleChange, DataError> {
    let given = given_keys(scale);
    let mut out = ScaleChange {
        minimum: scale.min.map(Some),
        maximum: scale.max.map(Some),
        major_unit: scale.major_unit.map(Some),
        minor_unit: scale.minor_unit.map(Some),
        number_format: scale.number_format.clone().map(Some),
        reverse: scale.reverse,
    };
    for key in scale.auto.as_deref().unwrap_or_default() {
        let name = key.as_str();
        if given.contains(&name) {
            return Err(DataError(format!("scale: {name} is given and listed in auto")));
        }
        match key {
            ScaleKey::Min => out.minimum = Some(None),
            ScaleKey::Max => out.maximum = Some(None),
            ScaleKey::MajorUnit => out.major_unit = Some(None),
            ScaleKey::MinorUnit => out.minor_unit = Some(None),
            ScaleKey::NumberFormat => out.number_format = Some(None),
        }
    }
    if out == ScaleChange::default() {
        return Err(DataError(format!("scale is empty: give {SCALE_KEYS}")));
    }
    Ok(out)
}

/// The style keys given, sorted (Python's `sorted(style)`), with whether each is a
/// series' (else the text's).
fn style_keys(style: &StyleArg) -> Vec<(&'static str, bool)> {
    let mut out = Vec::new();
    for (key, given, series) in [
        ("bold", style.bold.is_some(), false),
        ("color", style.color.is_some(), false),
        ("dash", style.dash.is_some(), true),
        ("fill", style.fill.is_some(), true),
        ("font", style.font.is_some(), false),
        ("line", style.line.is_some(), true),
        ("line_width", style.line_width.is_some(), true),
        ("marker", style.marker.is_some(), true),
        ("marker_fill", style.marker_fill.is_some(), true),
        ("marker_size", style.marker_size.is_some(), true),
        ("size", style.size.is_some(), false),
    ] {
        if given {
            out.push((key, series));
        }
    }
    out
}

/// `"none"` is no colour (`_colour`).
fn colour_given(value: Option<&String>) -> Option<Option<String>> {
    value.map(|v| (v.trim().to_lowercase() != "none").then(|| v.clone()))
}

/// `format_series`' and `format_point`'s style, checked as `apply_style_action` checks it.
fn series_format(style: Option<&StyleArg>, action: &str) -> Result<SeriesFormat, DataError> {
    let keys = style.map(style_keys).unwrap_or_default();
    if let Some((key, _)) = keys.iter().find(|(_, series)| !series) {
        return Err(DataError(format!(
            "style: '{key}' is not for {action}; give {SERIES_STYLE}"
        )));
    }
    let Some(style) = style.filter(|_| !keys.is_empty()) else {
        return Err(DataError(format!("{action} needs style: {SERIES_STYLE}")));
    };
    Ok(SeriesFormat {
        fill: colour_given(style.fill.as_ref()),
        line: colour_given(style.line.as_ref()),
        line_width: style.line_width,
        dash: style.dash.map(|d| dash_value(d.as_str()).to_owned()),
        marker: style.marker.map(|m| m.as_str().to_owned()),
        marker_size: style.marker_size,
        marker_fill: colour_given(style.marker_fill.as_ref()),
    })
}

/// A dash by its name (`DASHES`) as the schema names it.
fn dash_value(dash: &str) -> &str {
    match dash {
        "dot" => "sysDot",
        "dash_dot" => "dashDot",
        "long_dash" => "lgDash",
        other => other,
    }
}

/// `set_text_style`'s style, checked as `apply_style_action` and `set_text_style` check it
/// before anything changes (the size, then the colour).
fn text_look(style: Option<&StyleArg>) -> Result<TextLook, DataError> {
    let keys = style.map(style_keys).unwrap_or_default();
    if let Some((key, _)) = keys.iter().find(|(_, series)| *series) {
        return Err(DataError(format!(
            "style: '{key}' is not for set_text_style; give {TEXT_STYLE}"
        )));
    }
    let Some(style) = style.filter(|_| !keys.is_empty()) else {
        return Err(DataError(format!("set_text_style needs style: {TEXT_STYLE}")));
    };
    Ok(TextLook {
        font: style.font.clone(),
        size: style.size.map(text_size).transpose()?,
        bold: style.bold,
        colour: style.color.as_deref().map(Colour::parse).transpose()?,
    })
}

/// `_label_cells`: the series' labels written into the workbook line after the chart's data
/// (its old range unlinked first); the formula of their range.
fn label_cells(
    chart: &mut ChartTree,
    keeper: &mut Keeper<'_>,
    address: &str,
    series: usize,
    labels: &[String],
) -> Result<String, EditError> {
    chart.drop_label_range(series);
    let sources = all_sources(chart)?;
    let Some(block) = Block::find(address, &sources, keeper, chart.point_count())? else {
        let why = keeper.cache_only().unwrap_or("it has none");
        return Err(refused(&format!(
            "{address}: Value From Cells needs the chart's embedded workbook, and {why}; label the points without cells=True"
        )));
    };
    block.label_cells(address, keeper, &sources, series, labels)
}
