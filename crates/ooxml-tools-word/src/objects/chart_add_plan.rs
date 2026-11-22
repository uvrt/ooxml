//! Owns: `edit_chart`'s `add` checked and read before anything is made, as docx-agent's
//! `_add_chart` and ooxml-edit's `add_plan` and `add_style_actions` check it (each refusal
//! in their words and order): `box` refused in a document, what `add` needs, a scatter's or
//! bubble's x values, at most `MAX_ADD_VALUES` values, the legend's position, the look's
//! colours and text style keys; then the chart itself (`chart_data`, `check_options`) and
//! its size. And what it is made of: the new chart ([`NewChart`]), the workbook behind it
//! as `chart_workbook` lays it out ([`workbook`]), the scale, gridlines and look applied
//! after it in the same step ([`Finish`]).
//!
//! Must not: change the document, or plan what depends on the chart made (the scale's
//! axis, the text's look): `chart_add` does, on the new chart.

use super::{ChartAxis, ChartPosition, ChartType, EditChart, Gridlines, ScaleArg, SeriesType, StyleArg, TextPart};
use ooxml_chart::edit::create::{Labels, Legend, NewChart, NewKind, NewSeries, WORD_LOOK};
use ooxml_chart::edit::{Colour, DataError};
use ooxml_sheets::{column_letters, NewBook, NewCell, NewValue};
use ooxml_tools::{ArgEnum, Failure, Refusal, ToolError, TryCall, WayOut};

/// At most this many values (categories times series) in a chart `add` makes.
const MAX_ADD_VALUES: usize = 10_000;
/// The chart types, as `add` names them (`CHART_TYPES`).
const CHART_TYPES: &str =
    "column, stacked_column, bar, stacked_bar, line, pie, scatter, radar, radar_filled, bubble, dot";
/// The text style's keys (`_TEXT_STYLE`).
const TEXT_STYLE: [&str; 4] = ["font", "size", "bold", "color"];
/// Word's new chart (`CHART_WIDTH`, `CHART_HEIGHT`, measured: 5486400 x 3200400 EMU).
const CHART_SIZE: (f64, f64) = (432.0, 252.0);

/// One of the look's calls (`add_style_actions`), in its order.
#[derive(Debug, Clone)]
pub(super) enum Look {
    /// `show_data_labels` on every series.
    DataLabels,
    /// `format_series`: a series' colour as its kind is drawn (`fill`, `line` or
    /// `marker_fill`).
    Series {
        series: usize,
        key: &'static str,
        colour: String,
    },
    /// `format_point`: a pie's slice filled.
    Slice { point: usize, colour: String },
    /// `set_text_style`.
    Text { style: StyleArg, part: Option<TextPart> },
}

/// What `finish_add` applies to the new chart, in its order.
#[derive(Debug, Clone)]
pub(super) struct Finish {
    /// The scale left once a reversed category order is the chart's own (`None`: none).
    pub(super) scale: Option<ScaleArg>,
    /// The axis given.
    pub(super) axis: Option<ChartAxis>,
    /// Minor gridlines asked for (`minor` or `both`), on the value axis (a scatter's or
    /// bubble's `y`).
    pub(super) gridlines: Option<(&'static str, bool, bool)>,
    /// The look.
    pub(super) look: Vec<Look>,
}

/// `add` planned.
#[derive(Debug, Clone)]
pub(super) struct AddPlan {
    /// The chart.
    pub(super) chart: NewChart,
    /// Its size in points.
    pub(super) size: (f64, f64),
    /// What follows it.
    pub(super) finish: Finish,
}

fn invalid(message: impl Into<String>, field: &str, read: &TryCall) -> ToolError {
    ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(read.clone())).at(field)
}

/// A `ChartDataError` as the handler words it: `invalid_arguments`, the message stripped of
/// quotes at its ends.
fn data_error(message: &str, read: &TryCall) -> ToolError {
    let message = message.trim_matches(['\'', '"']).to_owned();
    ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(read.clone()))
}

/// `add`'s arguments checked and read, in docx-agent's order; `lang` is the new text's.
///
/// # Errors
/// The first refusal.
pub(super) fn add_plan(args: &EditChart, lang: String, read: &TryCall) -> Result<AddPlan, ToolError> {
    if args.frame.is_some() {
        return Err(invalid("box is for decks; in a document give width", "box", read));
    }
    let Some(kind) = args.chart_type else {
        return Err(invalid(
            format!("add needs chart_type: one of {CHART_TYPES}"),
            "chart_type",
            read,
        ));
    };
    let Some(data) = &args.data else {
        let message = "add needs data: each series as {name, values}, one value per category";
        return Err(invalid(message, "data", read));
    };
    let labels = labels(args, kind, read)?;
    let count = match &labels {
        Labels::Text(t) => t.len(),
        Labels::Numbers(n) => n.len(),
    };
    if count.saturating_mul(data.len().max(1)) > MAX_ADD_VALUES {
        return Err(ToolError::failure(Failure::Limit, "at most 10,000 chart values").at("data"));
    }
    let legend = legend(args.position, read)?;
    let look = look(args, kind).map_err(|e| data_error(&e.0, read))?;
    let given = args.scale.as_ref();
    let reversed = given.and_then(|s| s.reverse).unwrap_or(false) && args.axis == Some(ChartAxis::Category);
    let scale = given
        .map(|s| ScaleArg {
            reverse: if reversed { None } else { s.reverse },
            ..s.clone()
        })
        .filter(|s| *s != empty_scale());
    let chart = NewChart {
        kind: new_kind(kind),
        labels,
        series: data
            .iter()
            .map(|s| NewSeries {
                name: Some(s.name.clone()),
                values: s.values.clone(),
                line: s.kind.map(|k| k == SeriesType::Line),
                size: s.size.as_ref().map(|v| v.iter().copied().map(Some).collect()),
            })
            .collect(),
        title: args.text.clone(),
        legend,
        number_format: args.number_format.clone(),
        look: WORD_LOOK,
        lang,
        reversed,
        gridlines: args.gridlines != Some(Gridlines::None),
    };
    chart.check().map_err(|e| data_error(&e.0, read))?;
    let size = args.width.map_or(CHART_SIZE, |w| (w, w * CHART_SIZE.1 / CHART_SIZE.0));
    let minor = match args.gridlines {
        Some(Gridlines::Minor) => Some((false, true)),
        Some(Gridlines::Both) => Some((true, true)),
        _ => None,
    };
    let axis = if matches!(kind, ChartType::Scatter | ChartType::Bubble) {
        "y"
    } else {
        "value"
    };
    Ok(AddPlan {
        chart,
        size,
        finish: Finish {
            scale,
            axis: args.axis,
            gridlines: minor.map(|(major, minor)| (axis, major, minor)),
            look,
        },
    })
}

/// The categories, or a scatter's or bubble's x values (`add_plan`'s checks of them).
fn labels(args: &EditChart, kind: ChartType, read: &TryCall) -> Result<Labels, ToolError> {
    let name = kind.as_str();
    Ok(if matches!(kind, ChartType::Scatter | ChartType::Bubble) {
        let (Some(values), None) = (&args.values, &args.categories) else {
            let message = format!("a {name} chart takes its x values in values, not categories");
            return Err(invalid(message, "values", read));
        };
        let Some(xs) = values.iter().copied().collect::<Option<Vec<f64>>>() else {
            let message =
                format!("a {name} chart's x values are numbers, none blank: a blank point goes in a series' values");
            return Err(invalid(message, "values", read));
        };
        Labels::Numbers(xs)
    } else {
        let Some(categories) = &args.categories else {
            return Err(invalid(
                "add needs categories: the category labels, in order",
                "categories",
                read,
            ));
        };
        if args.values.is_some() {
            let message = "values are a scatter or bubble chart's x values; give each series' values in data";
            return Err(invalid(message, "values", read));
        }
        Labels::Text(categories.clone())
    })
}

/// The legend's position (`None`: none; Office's where none is given); a label position
/// refused.
fn legend(position: Option<ChartPosition>, read: &TryCall) -> Result<Option<Legend>, ToolError> {
    Ok(match position {
        None => Some(Legend::Default),
        Some(ChartPosition::None) => None,
        Some(ChartPosition::Right) => Some(Legend::Right),
        Some(ChartPosition::Left) => Some(Legend::Left),
        Some(ChartPosition::Top) => Some(Legend::Top),
        Some(ChartPosition::Bottom) => Some(Legend::Bottom),
        Some(other) => {
            let message = format!(
                "{} is a label position; a legend is right, left, top, bottom or none",
                other.as_str()
            );
            return Err(invalid(message, "position", read));
        }
    })
}

/// A scale that gives nothing (Python's empty `scale` dict).
fn empty_scale() -> ScaleArg {
    ScaleArg {
        min: None,
        max: None,
        major_unit: None,
        minor_unit: None,
        number_format: None,
        reverse: None,
        auto: None,
    }
}

fn new_kind(kind: ChartType) -> NewKind {
    match kind {
        ChartType::Column => NewKind::Column,
        ChartType::StackedColumn => NewKind::StackedColumn,
        ChartType::Bar => NewKind::Bar,
        ChartType::StackedBar => NewKind::StackedBar,
        ChartType::Line => NewKind::Line,
        ChartType::Pie => NewKind::Pie,
        ChartType::Scatter => NewKind::Scatter,
        ChartType::Radar => NewKind::Radar,
        ChartType::RadarFilled => NewKind::RadarFilled,
        ChartType::Bubble => NewKind::Bubble,
        ChartType::Dot => NewKind::Dot,
    }
}

/// The text style's keys given, in sorted order.
fn text_keys(style: &StyleArg) -> Vec<&'static str> {
    let mut keys = Vec::new();
    for (key, given) in [
        ("bold", style.bold.is_some()),
        ("color", style.color.is_some()),
        ("dash", style.dash.is_some()),
        ("fill", style.fill.is_some()),
        ("font", style.font.is_some()),
        ("line", style.line.is_some()),
        ("line_width", style.line_width.is_some()),
        ("marker", style.marker.is_some()),
        ("marker_fill", style.marker_fill.is_some()),
        ("marker_size", style.marker_size.is_some()),
        ("size", style.size.is_some()),
    ] {
        if given {
            keys.push(key);
        }
    }
    keys
}

/// `add_style_actions`: the look's calls, each list, colour and style key checked first.
fn look(args: &EditChart, kind: ChartType) -> Result<Vec<Look>, DataError> {
    let refuse = |text: String| Err(DataError(text));
    let entries = args.data.as_deref().unwrap_or_default();
    let mut out = Vec::new();
    if args.data_labels == Some(true) {
        out.push(Look::DataLabels);
    }
    if let Some(colours) = &args.colors {
        let pie = kind == ChartType::Pie;
        if pie {
            let points = entries.first().map_or(0, |e| e.values.len());
            if colours.len() > points {
                return refuse(format!(
                    "colors: {} colours for a pie of {points} slices; give at most one per slice",
                    colours.len()
                ));
            }
        } else if colours.len() > entries.len() {
            return refuse(format!(
                "colors: {} colours for {} series; give at most one per series, in order",
                colours.len(),
                entries.len()
            ));
        }
        for (index, colour) in colours.iter().enumerate() {
            if colour.trim().is_empty() {
                return refuse(format!("colors[{index}] is a colour, as shapes take them"));
            }
            if colour.trim().to_lowercase() != "none" {
                if let Err(error) = Colour::parse(colour) {
                    return refuse(format!("colors[{index}]: {}", error.0));
                }
            }
            if pie {
                out.push(Look::Slice {
                    point: index,
                    colour: colour.clone(),
                });
                continue;
            }
            let line = entries.get(index).and_then(|e| e.kind) == Some(SeriesType::Line);
            let key = match kind {
                _ if line => "line",
                ChartType::Line | ChartType::Radar => "line",
                ChartType::Scatter | ChartType::Dot => "marker_fill",
                _ => "fill",
            };
            out.push(Look::Series {
                series: index,
                key,
                colour: colour.clone(),
            });
        }
    }
    match &args.style {
        Some(style) => {
            let keys = text_keys(style);
            if let Some(unknown) = keys.iter().find(|k| !TEXT_STYLE.contains(*k)) {
                return refuse(format!(
                    "style: '{unknown}' is not for add, which takes the text's {}; colour series with colors",
                    TEXT_STYLE.join(", ")
                ));
            }
            if keys.is_empty() {
                return refuse(format!("style is empty: give {}", TEXT_STYLE.join(", ")));
            }
            out.push(Look::Text {
                style: style.clone(),
                part: args.part,
            });
        }
        None if args.part.is_some() => return refuse("part is the text style's: give style too".to_owned()),
        None => {}
    }
    Ok(out)
}

/// The workbook behind `chart`, as `chart_workbook` lays it out: `Sheet1` with the
/// categories in column A from row 2, each series' name in row 1 and its values below (a
/// bubble chart's sizes in the next column, headed "Size"), the number format on the value
/// cells.
///
/// # Errors
/// A column past the sheet's last (more series than the checks allow).
pub(super) fn workbook(chart: &NewChart) -> Result<NewBook, ooxml_sheets::AddressError> {
    let custom = chart.number_format.as_deref().filter(|f| *f != "General");
    let value_style = usize::from(custom.is_some());
    let columns = chart.data_columns();
    let mut rows = Vec::new();
    let mut header = Vec::new();
    for (series, (column, sizes)) in chart.series.iter().zip(&columns) {
        if let Some(name) = &series.name {
            header.push(cell(*column, NewValue::Text(name.clone()), 0));
        }
        if let Some(sizes) = sizes {
            header.push(cell(*sizes, NewValue::Text("Size".to_owned()), 0));
        }
    }
    let last = columns.iter().map(|(c, s)| s.unwrap_or(*c)).max().unwrap_or(1);
    let first_row = if header.is_empty() { 2 } else { 1 };
    if !header.is_empty() {
        rows.push((1, header));
    }
    let labels: Vec<NewValue> = match &chart.labels {
        Labels::Text(t) => t.iter().cloned().map(NewValue::Text).collect(),
        Labels::Numbers(n) => n.iter().copied().map(NewValue::Number).collect(),
    };
    let mut row = 1u32;
    for (index, label) in labels.into_iter().enumerate() {
        row += 1;
        let mut cells = vec![cell(1, label, 0)];
        for (series, (column, sizes)) in chart.series.iter().zip(&columns) {
            if let Some(Some(value)) = series.values.get(index) {
                cells.push(cell(*column, NewValue::Number(*value), value_style));
            }
            let size = series.size.as_ref().and_then(|s| s.get(index).copied().flatten());
            if let (Some(sizes), Some(size)) = (sizes, size) {
                cells.push(cell(*sizes, NewValue::Number(size), 0));
            }
        }
        rows.push((row, cells));
    }
    Ok(NewBook {
        sheet: "Sheet1".to_owned(),
        rows,
        dimension: format!("A{first_row}:{}{row}", column_letters(last)?),
        formats: custom.map(str::to_owned).into_iter().collect(),
    })
}

fn cell(column: u32, value: NewValue, style: usize) -> NewCell {
    NewCell { column, value, style }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbook_laid_out_as_chart_workbook() {
        let chart = NewChart {
            kind: NewKind::Column,
            labels: Labels::Text(vec!["North".to_owned(), "South".to_owned()]),
            series: vec![NewSeries {
                name: Some("2024".to_owned()),
                values: vec![Some(1.0), None],
                line: None,
                size: None,
            }],
            title: None,
            legend: Some(Legend::Default),
            number_format: Some("0.0".to_owned()),
            look: WORD_LOOK,
            lang: "en-US".to_owned(),
            reversed: false,
            gridlines: true,
        };
        let book = workbook(&chart).ok();
        let book = book.as_ref();
        assert_eq!(book.map(|b| b.dimension.as_str()), Some("A1:B3"));
        assert_eq!(book.map(|b| b.rows.len()), Some(3));
        assert_eq!(book.map(|b| b.formats.clone()), Some(vec!["0.0".to_owned()]));
        assert_eq!(book.and_then(|b| b.rows.get(2)).map(|(_, cells)| cells.len()), Some(1));
    }
}
