//! Owns: what ooxml-edit's `Chart` reads of a chart part beyond `chart_model` for
//! `edit_chart`'s `read` (`charts/chart.py`, `charts/styling.py`, `charts/format.py`): how
//! blanks draw, the gap width, whether it reads as a dot chart, the number formats of the
//! values, data labels and value axis, the axes by the names the formatting actions take and
//! each one's scale, and a series' own look (fill, line, marker; points formatted on their
//! own). A series' point labels are [`super::labels`]'.
//!
//! Each reads the part as its Python source does: elements found by their local names, as
//! [`super::chart_data`] finds them, but a fill's kind, which ooxml-edit tells by namespace.
//!
//! Must not: change the part, read the workbook, or default what the part does not say
//! (what ooxml-edit reports as absent stays absent).

use super::source::{decode_number, Point};
use super::Ser;
use crate::model::{AXIS_ELEMENTS, CHART_GROUP_ELEMENTS};
use ooxml_xml::view::{attr, child, children, elements, local, py_int};
use ooxml_xml::Element;

/// DrawingML's main namespace: a fill's kind is told by it.
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// The fills a shape's properties may hold (`a:` names).
const FILLS: [&str; 6] = ["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"];
/// The colour elements `color_text` reads.
const COLOURS: [&str; 4] = ["schemeClr", "srgbClr", "sysClr", "prstClr"];
/// The friendly dashes, by the schema's value (`format.py`'s `DASHES` reversed).
const DASHES: [(&str, &str); 8] = [
    ("solid", "solid"),
    ("sysDot", "dot"),
    ("dash", "dash"),
    ("dashDot", "dash_dot"),
    ("lgDash", "long_dash"),
    ("lgDashDot", "long_dash_dot"),
    ("lgDashDotDot", "long_dash_dot_dot"),
    ("sysDash", "square_dot"),
];
/// EMU in a point.
const EMU_PER_POINT: f64 = 12_700.0;

/// One setting of a look, as ooxml-edit gives it.
#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// `None`: none (no fill, no line), or a value the part leaves out.
    Null,
    /// `True`: a kind of fill that is not a colour.
    Yes,
    /// A colour, a dash or a marker symbol.
    Text(String),
    /// A number as written (a width in points, a marker size).
    Number(Point),
}

/// A look: its settings in ooxml-edit's order (`fill`, `line`, `line_width`, `dash`,
/// `marker`, `marker_size`, `marker_fill`, `marker_line`), each only where the part sets it.
pub type Look = Vec<(&'static str, Said)>;

/// A series' own look, and its points formatted on their own (`c:dPt`, by index).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SeriesLook {
    /// The series' settings.
    pub own: Look,
    /// Each point's, in the part's order (a repeated index: its last, at its first place).
    pub points: Vec<(i64, Look)>,
}

impl SeriesLook {
    /// Whether the series sets anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.own.is_empty() && self.points.is_empty()
    }
}

/// How an axis is scaled (`axis_scale`): bounds and units are absent on a category axis
/// (`None` here), automatic where `Some(None)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    /// `minimum`, `maximum`, `major_unit`, `minor_unit`.
    pub bounds: Option<[Option<Point>; 4]>,
    /// Its own number format; `None` follows the values'.
    pub number_format: Option<String>,
    /// Whether it runs from its maximum.
    pub reverse: bool,
    /// Whether it has major and minor gridlines.
    pub gridlines: (bool, bool),
}

/// The number formats that decide how the chart's numbers look (`number_formats`).
#[derive(Debug, Clone, PartialEq)]
pub struct NumberFormats {
    /// The values' (the first series' value cache's `c:formatCode`).
    pub values: Option<String>,
    /// The first data label's that shows a number.
    pub data_labels: Option<String>,
    /// The value axis's.
    pub value_axis: Option<String>,
}

fn plot_area(root: &Element) -> Option<&Element> {
    child(root, "chart").and_then(|c| child(c, "plotArea"))
}

/// The plots, in the plot area's order.
pub(crate) fn plots(root: &Element) -> Vec<&Element> {
    plot_area(root)
        .into_iter()
        .flat_map(elements)
        .filter(|e| CHART_GROUP_ELEMENTS.contains(&local(e)))
        .collect()
}

/// Every `c:ser` with its plot, plot by plot.
pub(crate) fn series(root: &Element) -> Vec<(&Element, &Element)> {
    plots(root)
        .into_iter()
        .flat_map(|p| children(p, "ser").map(move |s| (p, s)))
        .collect()
}

fn axes(root: &Element) -> Vec<&Element> {
    plot_area(root)
        .into_iter()
        .flat_map(elements)
        .filter(|e| AXIS_ELEMENTS.contains(&local(e)))
        .collect()
}

/// A flag as ooxml-edit reads one: `val` absent is on.
fn on(element: &Element) -> bool {
    matches!(attr(element, "val").unwrap_or("1"), "1" | "true")
}

/// How blank points draw (`blanks`): `gap`, `zero` or `span`; a part that does not say
/// draws zero.
#[must_use]
pub fn blanks(root: &Element) -> &'static str {
    let said = child(root, "chart")
        .and_then(|c| child(c, "dispBlanksAs"))
        .and_then(|n| attr(n, "val"));
    match said {
        Some("gap") => "gap",
        Some("span") => "span",
        _ => "zero",
    }
}

/// The bar plot's gap width (`gap_width`): 150 where it does not say, `None` without bars.
#[must_use]
pub fn gap_width(root: &Element) -> Option<i64> {
    let plot = plot_area(root).and_then(|p| child(p, "barChart"))?;
    Some(
        child(plot, "gapWidth")
            .and_then(|n| attr(n, "val"))
            .and_then(py_int)
            .unwrap_or(150),
    )
}

/// Whether the chart reads as a dot chart (`is_dot`): one line plot whose every series has
/// no line and a marker.
#[must_use]
pub fn is_dot(root: &Element) -> bool {
    let plots = plots(root);
    if plots.len() != 1 || plots.first().map(|p| local(p)) != Some("lineChart") {
        return false;
    }
    let all = series(root);
    !all.is_empty() && all.iter().all(|(plot, ser)| dot_series(plot, ser))
}

fn dot_series(plot: &Element, ser: &Element) -> bool {
    let lineless = child(ser, "spPr")
        .and_then(|s| child(s, "ln"))
        .and_then(|l| child(l, "noFill"))
        .is_some();
    if !lineless {
        return false;
    }
    if let Some(symbol) = child(ser, "marker").and_then(|m| child(m, "symbol")) {
        return attr(symbol, "val") != Some("none");
    }
    child(plot, "marker").is_some_and(on)
}

/// The number formats (`number_formats`); `values` is the values' own format code.
#[must_use]
pub fn number_formats(root: &Element, values: Option<&str>) -> NumberFormats {
    NumberFormats {
        values: values.map(str::to_owned),
        data_labels: label_format(root, values),
        value_axis: format_of(plot_area(root).and_then(|p| super::axis_of(p, "value")), values),
    }
}

/// A flag of `owner` (`c:delete`, `c:showVal`...): `None` where it is absent.
fn flag(owner: &Element, name: &str) -> Option<bool> {
    child(owner, name).map(on)
}

/// Whether a `c:dLbls` or `c:dLbl` shows a value or a percentage; `None` where it does not say.
fn shows_number(owner: Option<&Element>) -> Option<bool> {
    let owner = owner?;
    if flag(owner, "delete") == Some(true) {
        return Some(false);
    }
    let flags = [flag(owner, "showVal"), flag(owner, "showPercent")];
    if flags.iter().all(Option::is_none) {
        return None;
    }
    Some(flags.contains(&Some(true)))
}

/// The format `owner` gives its numbers: `values` where it links to the source or says none.
fn format_of(owner: Option<&Element>, values: Option<&str>) -> Option<String> {
    let owner = owner?;
    match child(owner, "numFmt") {
        Some(fmt) if !matches!(attr(fmt, "sourceLinked"), Some("1" | "true")) => {
            attr(fmt, "formatCode").map(str::to_owned)
        }
        _ => values.map(str::to_owned),
    }
}

/// The format of the first data label that shows a number: a point's own `c:dLbl` over its
/// series' `c:dLbls` over its plot's.
fn label_format(root: &Element, values: Option<&str>) -> Option<String> {
    for (plot, ser) in series(root) {
        let own = child(ser, "dLbls");
        let shared = child(plot, "dLbls");
        let mut points: Vec<(&str, &Element)> = Vec::new();
        for label in own.into_iter().flat_map(|o| children(o, "dLbl")) {
            if let Some(index) = child(label, "idx") {
                let key = attr(index, "val").unwrap_or("");
                match points.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = label,
                    None => points.push((key, label)),
                }
            }
        }
        // (a source too long to read makes the chart unreadable before its formats are asked)
        let count = Ser::of(ser).values.count().unwrap_or(0).max(1);
        for index in 0..count {
            let key = index.to_string();
            let mine = points.iter().find(|(k, _)| *k == key).map(|(_, l)| *l);
            for owner in [mine, own, shared] {
                match shows_number(owner) {
                    None => continue,
                    Some(true) => return format_of(owner, values),
                    Some(false) => break,
                }
            }
        }
    }
    None
}

/// The axis `c:axPos`.
fn position(axis: &Element) -> Option<&str> {
    child(axis, "axPos").and_then(|p| attr(p, "val"))
}

/// The axes the first plot uses.
fn primary_axes(root: &Element) -> Vec<&Element> {
    let ids: Vec<Option<&str>> = plots(root)
        .first()
        .map(|p| children(p, "axId").map(|n| attr(n, "val")).collect())
        .unwrap_or_default();
    axes(root)
        .into_iter()
        .filter(|a| ids.contains(&child(a, "axId").and_then(|n| attr(n, "val"))))
        .collect()
}

/// A scatter or bubble chart: no category or date axis, two value axes or more.
fn xy(root: &Element) -> bool {
    let Some(area) = plot_area(root) else { return false };
    children(area, "catAx").next().is_none()
        && children(area, "dateAx").next().is_none()
        && children(area, "valAx").count() >= 2
}

/// The primary axis `name` names: `category`/`value` by role, `x`/`y` by direction.
fn axis_named<'a>(root: &'a Element, name: &str) -> Option<&'a Element> {
    match name {
        "category" | "value" => plot_area(root).and_then(|p| super::axis_of(p, name)),
        _ => {
            let wanted: &[&str] = if name == "x" { &["b", "t"] } else { &["l", "r"] };
            primary_axes(root)
                .into_iter()
                .find(|a| position(a).is_some_and(|p| wanted.contains(&p)))
        }
    }
}

/// The names the chart's axes go by (`axis_names`): `category` and `value`, `x` and `y` for
/// a scatter or bubble chart, none for a pie.
#[must_use]
pub fn axis_names(root: &Element) -> Vec<&'static str> {
    let names: [&'static str; 2] = if xy(root) { ["x", "y"] } else { ["category", "value"] };
    names.into_iter().filter(|n| axis_named(root, n).is_some()).collect()
}

/// How the axis `name` (one of [`axis_names`]) is scaled (`axis_scale`).
#[must_use]
pub fn axis_scale(root: &Element, name: &str) -> Option<Scale> {
    let axis = axis_named(root, name)?;
    let scaling = child(axis, "scaling");
    let number = |parent: Option<&Element>, name: &str| {
        parent
            .and_then(|p| child(p, name))
            .and_then(|n| decode_number(attr(n, "val").unwrap_or("")))
    };
    let bounds = (local(axis) != "catAx").then(|| {
        [
            number(scaling, "min"),
            number(scaling, "max"),
            number(Some(axis), "majorUnit"),
            number(Some(axis), "minorUnit"),
        ]
    });
    let reverse = scaling
        .and_then(|s| child(s, "orientation"))
        .is_some_and(|o| attr(o, "val") == Some("maxMin"));
    Some(Scale {
        bounds,
        number_format: format_of(Some(axis), None),
        reverse,
        gridlines: (
            child(axis, "majorGridlines").is_some(),
            child(axis, "minorGridlines").is_some(),
        ),
    })
}

/// What fills `owner` (a `c:spPr` or `a:ln`, `fill_text`): `None` where it says nothing
/// (automatic).
fn fill(owner: Option<&Element>) -> Option<Said> {
    for node in elements(owner?) {
        let name = local(node);
        if !node.name().is(Some(A), name) || !FILLS.contains(&name) {
            continue;
        }
        return Some(match name {
            "noFill" => Said::Null,
            "solidFill" => colour(node).map_or(Said::Null, Said::Text),
            _ => Said::Yes,
        });
    }
    None
}

/// The colour a fill or colour element holds, as a specification (`color_text`):
/// `accent1 lumMod=75% lumOff=25%`, `1F4E79`.
fn colour(holder: &Element) -> Option<String> {
    let element = if COLOURS.contains(&local(holder)) {
        holder
    } else {
        elements(holder).find(|c| COLOURS.contains(&local(c)))?
    };
    let value = if local(element) == "sysClr" {
        attr(element, "lastClr")
    } else {
        attr(element, "val")
    };
    let mut parts = vec![value.unwrap_or("").to_owned()];
    for modifier in elements(element) {
        let name = local(modifier);
        parts.push(match attr(modifier, "val") {
            None => name.to_owned(),
            Some(raw) => match py_int(raw) {
                #[allow(clippy::cast_precision_loss)]
                Some(n) => format!("{name}={}%", crate::pyfmt::general(n as f64 / 1000.0, 6)),
                None => format!("{name}={raw}"),
            },
        });
    }
    Some(parts.join(" "))
}

/// A `c:spPr`'s line (`line_text`): its colour, width in points and dash, where given.
fn line(shape: Option<&Element>, out: &mut Look) {
    let Some(ln) = shape.and_then(|s| child(s, "ln")) else {
        return;
    };
    if let Some(said) = fill(Some(ln)) {
        out.push(("line", said));
    }
    if let Some(width) = attr(ln, "w").filter(|w| !w.is_empty()).and_then(py_int) {
        #[allow(clippy::cast_precision_loss)]
        let points = crate::pyfmt::round(width as f64 / EMU_PER_POINT, 2);
        out.push(("line_width", Said::Number(Point::Float(points))));
    }
    if let Some(dash) = child(ln, "prstDash") {
        let said = attr(dash, "val").map_or(Said::Null, |v| {
            let friendly = DASHES.iter().find(|(k, _)| *k == v).map_or(v, |(_, f)| *f);
            Said::Text(friendly.to_owned())
        });
        out.push(("dash", said));
    }
}

/// `owner`'s marker (`marker_text`).
fn marker(owner: &Element, out: &mut Look) {
    let Some(marker) = child(owner, "marker") else { return };
    if let Some(symbol) = child(marker, "symbol") {
        let said = attr(symbol, "val").map_or(Said::Null, |v| Said::Text(v.to_owned()));
        out.push(("marker", said));
    }
    let size = child(marker, "size").and_then(|s| attr(s, "val"));
    if let Some(size) = size.filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())) {
        if let Ok(n) = size.parse::<i64>() {
            out.push(("marker_size", Said::Number(Point::Int(n))));
        }
    }
    let shape = child(marker, "spPr");
    if let Some(said) = fill(shape) {
        out.push(("marker_fill", said));
    }
    let mut lined = Look::new();
    line(shape, &mut lined);
    if let Some((_, said)) = lined.into_iter().find(|(k, _)| *k == "line") {
        out.push(("marker_line", said));
    }
}

/// `owner`'s own look (`_look`): a series' or a point's fill, line and marker.
fn look(owner: &Element) -> Look {
    let mut out = Look::new();
    let shape = child(owner, "spPr");
    if let Some(said) = fill(shape).filter(|s| *s != Said::Yes) {
        out.push(("fill", said));
    }
    line(shape, &mut out);
    marker(owner, &mut out);
    out
}

/// A series' own look (`series_format`), its points formatted on their own by index.
#[must_use]
pub fn series_look(ser: &Element) -> SeriesLook {
    let mut points: Vec<(i64, Look)> = Vec::new();
    for point in children(ser, "dPt") {
        let index = child(point, "idx").and_then(|i| attr(i, "val"));
        let Some(index) = index.filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())) else {
            continue;
        };
        let Ok(index) = index.parse::<i64>() else { continue };
        let said = look(point);
        match points.iter_mut().find(|(k, _)| *k == index) {
            Some(slot) => slot.1 = said,
            None => points.push((index, said)),
        }
    }
    SeriesLook { own: look(ser), points }
}
