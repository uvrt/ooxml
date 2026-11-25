//! Owns: an axis' scale and gridlines changed in the chart part (ooxml-edit's
//! `ChartStyling.set_axis_scale`, `set_gridlines`, `charts/styling.py`): the axis a name
//! gives (`_axis_node`: `category`/`value` by role, `x`/`y` by direction), its bounds,
//! units, number format and direction -- the axis it crosses moved to its maximum when it
//! runs backwards -- and Office's gridlines shown or taken away.
//!
//! Must not: decide which axis a scale given without one is for (the tool's handler does,
//! `scale_axis`).

use super::format::gridline;
use super::make::Make;
use super::owned::{Order, Owned};
use super::path::indexed;
use super::{number_text, ChartTree, DataError};
use crate::data::{axis_named, axis_scale, Point};
use ooxml_base::pyfmt::PyNumber;
use ooxml_schema::orders::local as schema;
use ooxml_xml::view::{attr, child, local};
use ooxml_xml::Element;

/// The axis names a caller may give (`AXIS_NAMES`).
const AXIS_NAMES: [&str; 4] = ["category", "value", "x", "y"];
/// The axis elements (`AXIS_TAGS`).
const AXES: [&str; 4] = ["valAx", "catAx", "dateAx", "serAx"];

/// An axis kind's sequence.
pub(crate) fn axis_order(kind: &str) -> Order {
    match kind {
        "catAx" => schema::CAT_AX,
        "dateAx" => schema::DATE_AX,
        "serAx" => schema::SER_AX,
        _ => schema::VAL_AX,
    }
}

/// A change of an axis' scale (`set_axis_scale`'s keywords): `None` stays; a bound or unit
/// `Some(None)` is automatic again, a number format `Some(None)` follows the values'. A
/// number keeps whether it was given as an integer or a float, as messages name it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScaleChange {
    /// The minimum.
    pub minimum: Option<Option<PyNumber>>,
    /// The maximum.
    pub maximum: Option<Option<PyNumber>>,
    /// The major unit.
    pub major_unit: Option<Option<PyNumber>>,
    /// The minor unit.
    pub minor_unit: Option<Option<PyNumber>>,
    /// An Excel code.
    pub number_format: Option<Option<String>>,
    /// Whether it runs from its maximum.
    pub reverse: Option<bool>,
}

impl ScaleChange {
    /// The bounds and units with their names in a message.
    fn numeric(&self) -> [(&'static str, Option<Option<PyNumber>>); 4] {
        [
            ("minimum", self.minimum),
            ("maximum", self.maximum),
            ("major unit", self.major_unit),
            ("minor unit", self.minor_unit),
        ]
    }
}

/// A bound or unit in force after a change: its value and how Python shows it.
type InForce = Option<(f64, String)>;

impl ChartTree {
    /// The plot area's axes, with their places in it.
    fn axes(&self) -> Vec<(usize, &Element)> {
        let area = child(&self.root, "chart").and_then(|c| child(c, "plotArea"));
        area.map(|a| indexed(a).filter(|(_, e)| AXES.contains(&local(e))).collect())
            .unwrap_or_default()
    }

    /// The axis `axis` names (`_axis_node`) and its place in the plot area.
    fn axis_place(&self, address: &str, axis: &str) -> Result<(usize, &Element), DataError> {
        if !AXIS_NAMES.contains(&axis) {
            return Err(DataError(format!("axis '{axis}'; one of {}", AXIS_NAMES.join(", "))));
        }
        let node = axis_named(&self.root, axis);
        let found = node.and_then(|node| self.axes().into_iter().find(|(_, e)| std::ptr::eq(*e, node)));
        found.ok_or_else(|| {
            let have: Vec<&str> = if self.axes().is_empty() {
                Vec::new()
            } else {
                AXIS_NAMES
                    .into_iter()
                    .filter(|n| axis_named(&self.root, n).is_some())
                    .collect()
            };
            let tail = if have.is_empty() {
                " (a pie has none)".to_owned()
            } else {
                format!("; it has {}", have.join(", "))
            };
            DataError(format!("{address}: the chart has no {axis} axis{tail}"))
        })
    }

    /// The axis at `place` in the plot area, opened in `root`, with what `change` does.
    fn change_axis(root: &mut Owned, place: usize, change: impl FnOnce(&mut Owned)) {
        let area = root.child("chart").and_then(|c| c.child("plotArea"));
        if let Some(axis) = area.and_then(|a| a.at(place)) {
            change(axis);
        }
    }

    /// Fix the scale of the axis `axis` names (`set_axis_scale`); `values_format` is the
    /// values' number format (`number_format`), which a format following them is given.
    ///
    /// # Errors
    /// [`DataError`] (`FormatError`) for an axis the chart does not have, bounds or units
    /// on a category axis, a unit not above 0, a minimum not below the maximum, a minor
    /// unit above the major one or a number format that is no Excel code.
    pub fn set_axis_scale(
        &mut self,
        address: &str,
        axis: &str,
        change: &ScaleChange,
        values_format: Option<&str>,
    ) -> Result<(), DataError> {
        let (place, node) = self.axis_place(address, axis)?;
        let kind = local(node).to_owned();
        self.check_scale(address, axis, &kind, change)?;
        let crossing = change.reverse.and_then(|_| self.crossing(node));
        let make = Make::of(&self.root);
        let order = axis_order(&kind);
        self.change_root(|root| {
            Self::change_axis(root, place, |node| {
                write_scale(node, &make, order, change, values_format)
            });
            if let (Some(reverse), Some((other, other_kind))) = (change.reverse, crossing) {
                Self::change_axis(root, other, |axis| {
                    for name in ["crosses", "crossesAt"] {
                        if let Some(at) = axis.place(name) {
                            axis.remove(at);
                        }
                    }
                    let crosses = make
                        .c("crosses")
                        .with_attr("val", if reverse { "max" } else { "autoZero" });
                    axis.insert_ordered(Some(axis_order(&other_kind)), crosses);
                });
            }
        });
        Ok(())
    }

    /// `set_axis_scale`'s checks, in its order.
    fn check_scale(&self, address: &str, axis: &str, kind: &str, change: &ScaleChange) -> Result<(), DataError> {
        let numeric = change.numeric();
        if kind == "catAx" && numeric.iter().any(|(_, b)| matches!(b, Some(Some(_)))) {
            return Err(DataError(format!(
                "{address}: the {axis} axis shows categories, which have no numeric bounds or units; give them for the value axis"
            )));
        }
        for (name, given) in numeric.iter().skip(2) {
            if matches!(given, Some(Some(value)) if value.value() <= 0.0) {
                return Err(DataError(format!("{name} must be greater than 0")));
            }
        }
        let current: [Option<Point>; 4] = if kind == "catAx" {
            Default::default()
        } else {
            axis_scale(&self.root, axis).and_then(|s| s.bounds).unwrap_or_default()
        };
        let [low, high, major, minor]: [InForce; 4] = std::array::from_fn(|k| {
            let given = numeric.get(k).and_then(|(_, g)| *g);
            match given {
                Some(given) => given.map(|v| (v.value(), v.to_string())),
                None => current
                    .get(k)
                    .and_then(Option::as_ref)
                    .and_then(|p| p.number().map(|v| (v, p.text()))),
            }
        });
        if let (Some(low), Some(high)) = (low, high) {
            if low.0 >= high.0 {
                return Err(DataError(format!(
                    "{address}: the minimum ({}) must be below the maximum ({})",
                    low.1, high.1
                )));
            }
        }
        if let (Some(major), Some(minor)) = (major, minor) {
            if minor.0 > major.0 {
                return Err(DataError(format!(
                    "{address}: the minor unit ({}) is larger than the major unit ({})",
                    minor.1, major.1
                )));
            }
        }
        if let Some(Some(format)) = &change.number_format {
            if ooxml_xml::view::py_flat(format).is_empty() || format.chars().count() > 255 {
                return Err(DataError(format!(
                    "{address}: a number format is an Excel format code, such as 0.0"
                )));
            }
        }
        Ok(())
    }

    /// The axis `axis` crosses (`_crossing`: its `c:crossAx`): its place and kind.
    fn crossing(&self, axis: &Element) -> Option<(usize, String)> {
        let id = attr(child(axis, "crossAx")?, "val");
        self.axes()
            .into_iter()
            .find(|(_, a)| child(a, "axId").and_then(|n| attr(n, "val")) == id)
            .map(|(at, a)| (at, local(a).to_owned()))
    }

    /// Show (`Some(true)`) or take away an axis' major and minor gridlines
    /// (`set_gridlines`), Office's where shown.
    ///
    /// # Errors
    /// [`DataError`] for an axis the chart does not have.
    pub fn set_gridlines(
        &mut self,
        address: &str,
        axis: &str,
        major: Option<bool>,
        minor: Option<bool>,
    ) -> Result<(), DataError> {
        let (place, node) = self.axis_place(address, axis)?;
        let order = axis_order(local(node));
        let make = Make::of(&self.root);
        self.change_root(|root| {
            Self::change_axis(root, place, |node| {
                for (wanted, name, is_major) in [(major, "majorGridlines", true), (minor, "minorGridlines", false)] {
                    let Some(wanted) = wanted else { continue };
                    match (wanted, node.place(name)) {
                        (true, None) => {
                            node.insert_ordered(Some(order), gridline(&make, is_major));
                        }
                        (false, Some(at)) => {
                            node.remove(at);
                        }
                        _ => {}
                    }
                }
            });
        });
        Ok(())
    }
}

/// The scale written into the axis `node` (of the sequence `order`).
fn write_scale(node: &mut Owned, make: &Make, order: Order, change: &ScaleChange, values_format: Option<&str>) {
    let c = std::sync::Arc::clone(node.name());
    let scaling = node.sub(&c, "scaling", Some(order));
    if scaling.place("orientation").is_none() {
        scaling.insert_ordered(Some(schema::SCALING), make.c("orientation").with_attr("val", "minMax"));
    }
    for (given, name) in [(change.maximum, "max"), (change.minimum, "min")] {
        if let Some(given) = given {
            set_value(scaling, make, name, given, schema::SCALING);
        }
    }
    if let Some(reverse) = change.reverse {
        if let Some(orientation) = scaling.child("orientation") {
            orientation.set_attr("val", if reverse { "maxMin" } else { "minMax" });
        }
    }
    for (given, name) in [(change.major_unit, "majorUnit"), (change.minor_unit, "minorUnit")] {
        if let Some(given) = given {
            set_value(node, make, name, given, order);
        }
    }
    if let Some(format) = &change.number_format {
        let at = match node.place("numFmt") {
            Some(at) => at,
            None => node.insert_ordered(Some(order), make.c("numFmt")),
        };
        if let Some(number_format) = node.at(at) {
            let (code, linked) = match format {
                None => (values_format.unwrap_or("General"), "1"),
                Some(code) => (code.as_str(), "0"),
            };
            number_format.set_attr("formatCode", code);
            number_format.set_attr("sourceLinked", linked);
        }
    }
}

/// `_set_value`: `parent`'s `name` given `val` (made where `order` puts it), or taken out
/// for `None`.
fn set_value(parent: &mut Owned, make: &Make, name: &str, value: Option<PyNumber>, order: Order) {
    match value {
        None => {
            if let Some(at) = parent.place(name) {
                parent.remove(at);
            }
        }
        Some(value) => {
            let at = match parent.place(name) {
                Some(at) => at,
                None => parent.insert_ordered(Some(order), make.c(name)),
            };
            if let Some(node) = parent.at(at) {
                node.set_attr("val", &number_text(value.value()));
            }
        }
    }
}
