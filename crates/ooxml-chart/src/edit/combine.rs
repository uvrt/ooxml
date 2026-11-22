//! Owns: a series of a column chart drawn as a line, or back as a column, in the chart part
//! (ooxml-edit's `ChartStyling.set_series_type`, `charts/styling.py`): what may combine
//! checked (no horizontal bars, only column and line plots), the series' look turned from
//! a fill into a line or back (`_to_line`, `_to_column`) and the series moved into the line
//! plot on the primary or secondary axes -- made where there is none, with a secondary value
//! axis at the right and its hidden category axis as PowerPoint writes them
//! (`_line_plot`, `_secondary_axes`) -- in plot order (`_insert_series`); a plot left
//! without series goes, with the axes no other plot uses (`_drop_plot`).
//!
//! Must not: resolve the series by name (the tool's handler does), or hash (the caller gives
//! SHA-256, as ooxml-edit takes `hashlib`'s).

use super::make::Make;
use super::owned::{Order, Owned};
use super::path::indexed;
use super::series::{Hash, SER_ORDER};
use super::{ChartTree, DataError, PLOTS};
use ooxml_xml::view::{attr, child, children, local, py_int};
use ooxml_xml::Element;
use std::collections::BTreeSet;

/// The axis elements (`AXIS_TAGS`).
const AXES: [&str; 4] = ["valAx", "catAx", "dateAx", "serAx"];
/// A plot's head: what a series goes after where the plot has none (`_insert_series`).
const PLOT_HEAD: [&str; 5] = ["barDir", "grouping", "varyColors", "radarStyle", "scatterStyle"];
/// `c:plotArea`'s sequence.
const PLOT_AREA_ORDER: Order = &[&["layout"], &PLOTS, &AXES, &["dTable"], &["spPr"], &["extLst"]];
/// A line series' line width, EMU (`_to_line`).
const LINE_WIDTH: &str = "28575";

/// A plot area's child read for the move: its place, local name and axis ids.
struct Child {
    place: usize,
    name: String,
    ids: Vec<Option<String>>,
}

impl Child {
    fn is_plot(&self) -> bool {
        PLOTS.contains(&self.name.as_str())
    }

    /// `_ax_id`: its first `c:axId`'s value.
    fn ax_id(&self) -> Option<&str> {
        self.ids.first().and_then(Option::as_deref)
    }
}

/// The plot area as the move reads it, before it changes.
struct Area {
    children: Vec<Child>,
}

impl Area {
    fn of(area: &Element) -> Area {
        let children = indexed(area)
            .map(|(place, e)| Child {
                place,
                name: local(e).to_owned(),
                ids: children(e, "axId").map(|n| attr(n, "val").map(str::to_owned)).collect(),
            })
            .collect();
        Area { children }
    }

    fn at(&self, place: usize) -> Option<&Child> {
        self.children.iter().find(|c| c.place == place)
    }

    /// `_primary_axes`: the axes the first plot uses, in the plot area's order.
    fn primary(&self) -> Vec<&Child> {
        let ids: Vec<Option<&str>> = self
            .children
            .iter()
            .find(|c| c.is_plot())
            .map(|p| p.ids.iter().map(Option::as_deref).collect())
            .unwrap_or_default();
        self.children
            .iter()
            .filter(|c| AXES.contains(&c.name.as_str()) && ids.contains(&c.ax_id()))
            .collect()
    }

    /// `_on_secondary`: whether the plot uses an axis the first plot does not.
    fn on_secondary(&self, plot: &Child) -> bool {
        let primary: BTreeSet<Option<&str>> = self.primary().into_iter().map(Child::ax_id).collect();
        let ids: BTreeSet<Option<&str>> = plot.ids.iter().map(Option::as_deref).collect();
        !ids.is_empty() && !ids.is_subset(&primary)
    }
}

/// How the move goes: the series' place, the line plot it goes to (found or made).
struct Move<'a> {
    make: &'a Make,
    plot_at: usize,
    ser_at: usize,
    line: bool,
    secondary: bool,
    /// The secondary axes' ids, category first, where they are made.
    secondary_ids: Option<(String, String)>,
    number_format: Option<&'a str>,
}

impl ChartTree {
    /// Draw the series at `series` as a line (`line`; on the secondary axes with
    /// `secondary`) or as a column again (`set_series_type`); `number_format` is the values'
    /// (`number_format`), which a secondary value axis made new is given. Whether the part
    /// changed (a series drawn so already stays).
    ///
    /// # Errors
    /// [`DataError`] (`ChartDataError`) for horizontal bars, a chart that is not a column
    /// chart (with lines), columns on the secondary axis, or a column wanted where the
    /// chart has no column plot.
    pub fn set_series_type(
        &mut self,
        address: &str,
        series: usize,
        line: bool,
        secondary: bool,
        number_format: Option<&str>,
        hash: Hash<'_>,
    ) -> Result<bool, DataError> {
        let area_element = child(&self.root, "chart").and_then(|c| child(c, "plotArea"));
        let area = area_element.map_or(Area { children: Vec::new() }, Area::of);
        let types: Vec<String> = area
            .children
            .iter()
            .filter(|c| c.is_plot())
            .map(|c| c.name.trim_end_matches("Chart").to_owned())
            .collect();
        let bars = area_element.and_then(|a| child(a, "barChart"));
        if bars.and_then(|b| child(b, "barDir")).and_then(|d| attr(d, "val")) == Some("bar") {
            return Err(DataError(format!(
                "{address}: PowerPoint cannot combine horizontal bars with a line; make it a column chart"
            )));
        }
        let combines = types.iter().all(|t| t == "bar" || t == "line");
        if !combines || (!types.iter().any(|t| t == "bar") && line) {
            let shown = if types.is_empty() {
                "empty".to_owned()
            } else {
                types.join("/")
            };
            return Err(DataError(format!(
                "{address}: only a column chart takes a line series (this one is {shown})"
            )));
        }
        let Some((path, _)) = self.series(series) else {
            return Err(DataError(format!("{address}: no series {series}")));
        };
        let (&[_, _, plot_at, ser_at], Some(source)) = (path.as_slice(), path.get(2).and_then(|&at| area.at(at)))
        else {
            return Err(DataError(format!("{address}: no series {series}")));
        };
        let current_line = source.name == "lineChart";
        if current_line == line && area.on_secondary(source) == secondary {
            return Ok(false);
        }
        if !line && secondary {
            return Err(DataError(format!("{address}: columns stay on the primary axis")));
        }
        if !line && bars.is_none() {
            return Err(DataError(format!(
                "{address}: the chart has no column plot to move the series into"
            )));
        }
        let make = Make::of(&self.root);
        let mut taken: Vec<&str> = Vec::new();
        collect_ids(&self.root, &mut taken);
        let secondary_ids =
            (line && secondary && !has_line_plot(&area, true)).then(|| secondary_axis_ids(&taken, hash));
        let plan = Move {
            make: &make,
            plot_at,
            ser_at,
            line,
            secondary,
            secondary_ids,
            number_format,
        };
        let primary_value = area
            .primary()
            .into_iter()
            .find(|a| a.name == "valAx")
            .and_then(|a| area_element.and_then(|e| super::path::at(e, &[a.place])))
            .and_then(|axis| child(axis, "txPr"))
            .map(Owned::open);
        self.change_root(|root| {
            let Some(area_owned) = root.child("chart").and_then(|c| c.child("plotArea")) else {
                return;
            };
            plan.run(area_owned, &area, primary_value);
        });
        Ok(true)
    }
}

impl Move<'_> {
    fn run(&self, owned: &mut Owned, area: &Area, primary_text: Option<Owned>) {
        let Some(mut ser) = owned.at(self.plot_at).and_then(|p| p.remove(self.ser_at)) else {
            return;
        };
        if self.line {
            to_line(&mut ser, self.make);
        } else {
            to_column(&mut ser, self.make);
        }
        let mut source_at = self.plot_at;
        let target_at = if self.line {
            self.line_plot(owned, area, primary_text, &mut source_at)
        } else {
            area.children.iter().find(|c| c.name == "barChart").map(|c| c.place)
        };
        let Some(target) = target_at.and_then(|at| owned.at(at)) else {
            return;
        };
        insert_series(target, ser);
        let emptied = owned.at(source_at).is_some_and(|p| p.place("ser").is_none());
        if emptied {
            drop_plot(owned, source_at);
        }
    }

    /// `_line_plot`: the line plot on the primary or secondary axes, made where there is
    /// none; its place (`source_at` moved along where a plot goes in before it).
    fn line_plot(
        &self,
        owned: &mut Owned,
        area: &Area,
        primary_text: Option<Owned>,
        source_at: &mut usize,
    ) -> Option<usize> {
        let found = area
            .children
            .iter()
            .find(|c| c.name == "lineChart" && area.on_secondary(c) == self.secondary);
        if let Some(found) = found {
            return Some(found.place);
        }
        let primary = area.primary();
        let ids: Vec<Option<String>> = match &self.secondary_ids {
            Some((category, value)) => {
                for at in self.secondary_axes(owned, &primary, primary_text, (category, value)) {
                    if at <= *source_at {
                        *source_at += 1;
                    }
                }
                vec![Some(category.clone()), Some(value.clone())]
            }
            None => match area.children.iter().find(|c| c.name == "barChart") {
                Some(bars) => bars.ids.clone(),
                None => primary.iter().map(|a| a.ax_id().map(str::to_owned)).collect(),
            },
        };
        let make = self.make;
        let mut plot = make
            .c("lineChart")
            .with(make.c("grouping").with_attr("val", "standard"))
            .with(make.c("varyColors").with_attr("val", "0"));
        let mut labels = make.c("dLbls");
        for flag in [
            "showLegendKey",
            "showVal",
            "showCatName",
            "showSerName",
            "showPercent",
            "showBubbleSize",
        ] {
            labels.push(make.c(flag).with_attr("val", "0"));
        }
        plot.push(labels);
        plot.push(make.c("marker").with_attr("val", "1"));
        plot.push(make.c("smooth").with_attr("val", "0"));
        for id in ids {
            let mut axis = make.c("axId");
            if let Some(id) = id {
                axis.set_attr("val", &id);
            }
            plot.push(axis);
        }
        // the anchor: the last bar plot for the primary axes, else the last plot
        let is_plot = |name: &str| name.ends_with("Chart");
        let elements = owned.elements();
        let anchor = if self.secondary {
            None
        } else {
            elements.iter().rev().find(|(_, n)| n == "barChart").map(|(at, _)| *at)
        };
        let anchor = anchor.or_else(|| elements.iter().rev().find(|(_, n)| is_plot(n)).map(|(at, _)| *at))?;
        let at = owned.after(anchor);
        owned.insert(at, plot);
        if at <= *source_at {
            *source_at += 1;
        }
        Some(at)
    }

    /// `_secondary_axes`: a secondary value axis at the right and its hidden category axis
    /// after the primary axes (`primary` read before any change: axes come after plots, so
    /// their places hold while only axes go in).
    fn secondary_axes(
        &self,
        owned: &mut Owned,
        primary: &[&Child],
        text: Option<Owned>,
        (cat_id, val_id): (&str, &str),
    ) -> Vec<usize> {
        let make = self.make;
        let scaling = || make.c("scaling").with(make.c("orientation").with_attr("val", "minMax"));
        let shape = make
            .c("spPr")
            .with(make.top(make.a("noFill")))
            .with(make.top(make.a("ln").with(make.a("noFill"))))
            .with(make.top(make.a("effectLst")));
        let mut value = make
            .c("valAx")
            .with(make.c("axId").with_attr("val", val_id))
            .with(scaling())
            .with(make.c("delete").with_attr("val", "0"))
            .with(make.c("axPos").with_attr("val", "r"))
            .with(
                make.c("numFmt")
                    .with_attr(
                        "formatCode",
                        self.number_format.filter(|f| !f.is_empty()).unwrap_or("General"),
                    )
                    .with_attr("sourceLinked", "1"),
            )
            .with(make.c("majorTickMark").with_attr("val", "none"))
            .with(make.c("minorTickMark").with_attr("val", "none"))
            .with(make.c("tickLblPos").with_attr("val", "nextTo"))
            .with(shape);
        if let Some(text) = text {
            value.push(text);
        }
        let value = value
            .with(make.c("crossAx").with_attr("val", cat_id))
            .with(make.c("crosses").with_attr("val", "max"))
            .with(make.c("crossBetween").with_attr("val", "between"));
        let mut category = make
            .c("catAx")
            .with(make.c("axId").with_attr("val", cat_id))
            .with(scaling())
            .with(make.c("delete").with_attr("val", "1"))
            .with(make.c("axPos").with_attr("val", "b"))
            .with(
                make.c("numFmt")
                    .with_attr("formatCode", "General")
                    .with_attr("sourceLinked", "1"),
            )
            .with(make.c("majorTickMark").with_attr("val", "none"))
            .with(make.c("minorTickMark").with_attr("val", "none"))
            .with(make.c("tickLblPos").with_attr("val", "nextTo"))
            .with(make.c("crossAx").with_attr("val", val_id));
        for (flag, val) in [
            ("auto", "1"),
            ("lblAlgn", "ctr"),
            ("lblOffset", "100"),
            ("noMultiLvlLbl", "0"),
        ] {
            category.push(make.c(flag).with_attr("val", val));
        }
        match primary.last() {
            Some(last) => {
                let at = owned.after(last.place);
                owned.insert(at, value);
                owned.insert(at + 1, category);
                vec![at, at + 1]
            }
            None => {
                let first = owned.insert_ordered(Some(PLOT_AREA_ORDER), value);
                vec![first, owned.insert_ordered(Some(PLOT_AREA_ORDER), category)]
            }
        }
    }
}

/// Whether the plot area has a line plot on the secondary (or primary) axes.
fn has_line_plot(area: &Area, secondary: bool) -> bool {
    area.children
        .iter()
        .any(|c| c.name == "lineChart" && area.on_secondary(c) == secondary)
}

/// Every `c:axId` value below `element` (`root.iter(c:axId)`).
fn collect_ids<'a>(element: &'a Element, out: &mut Vec<&'a str>) {
    for e in element.child_elements() {
        if local(e) == "axId" {
            if let Some(value) = attr(e, "val") {
                out.push(value);
            }
        }
        collect_ids(e, out);
    }
}

/// The secondary axes' ids, category first: from SHA-256 of the ids the chart has.
fn secondary_axis_ids(taken: &[&str], hash: Hash<'_>) -> (String, String) {
    let sorted: BTreeSet<&str> = taken.iter().copied().collect();
    let seed: Vec<&str> = sorted.into_iter().collect();
    let digest = hash(seed.join("\n").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let id = |range: std::ops::Range<usize>| {
        let number = u64::from_str_radix(&hex[range], 16).unwrap_or(0) + 1;
        number.to_string()
    };
    (id(0..7), id(7..14))
}

/// The places of `owner`'s children named any of `names`, last first (for taking out).
fn places_of(owner: &Owned, names: &[&str]) -> Vec<usize> {
    let mut places: Vec<usize> = owner
        .elements()
        .into_iter()
        .filter(|(_, n)| names.contains(&n.as_str()))
        .map(|(at, _)| at)
        .collect();
    places.reverse();
    places
}

/// Take out `owner`'s children named any of `names`.
fn remove_all(owner: &mut Owned, names: &[&str]) {
    for at in places_of(owner, names) {
        owner.remove(at);
    }
}

/// The series' `c:spPr` taken out, with a copy of the fill it holds at `path` (`a:solidFill`
/// below it, by the local names of its steps).
fn take_shape_colour(ser: &mut Owned, path: &[&str]) -> Option<Owned> {
    let at = ser.place("spPr")?;
    let shape = ser.read(at)?;
    let mut node: &Element = &shape;
    let mut found = None;
    for (step, name) in path.iter().enumerate() {
        match child(node, name) {
            Some(next) if step + 1 == path.len() => found = Some(Owned::open(next)),
            Some(next) => node = next,
            None => break,
        }
    }
    ser.remove(at);
    found
}

/// `_to_line`: a bar series' elements as a line series': its colour becomes the line's.
fn to_line(ser: &mut Owned, make: &Make) {
    let colour = take_shape_colour(ser, &["solidFill"]);
    remove_all(ser, &["invertIfNegative", "pictureOptions", "shape", "explosion"]);
    for at in ser.places("dPt") {
        if let Some(point) = ser.at(at) {
            if let Some(first) = point.place("invertIfNegative") {
                point.remove(first);
            }
        }
    }
    let line = make
        .a("ln")
        .with_attr("w", LINE_WIDTH)
        .with_attr("cap", "rnd")
        .with(colour.unwrap_or_else(|| make.a("noFill")))
        .with(make.a("round"));
    let shape = make.c("spPr").with(make.top(line)).with(make.top(make.a("effectLst")));
    ser.insert_ordered(Some(SER_ORDER), shape);
    if ser.place("marker").is_none() {
        let marker = make.c("marker").with(make.c("symbol").with_attr("val", "none"));
        ser.insert_ordered(Some(SER_ORDER), marker);
    }
    if ser.place("smooth").is_none() {
        ser.insert_ordered(Some(SER_ORDER), make.c("smooth").with_attr("val", "0"));
    }
}

/// `_to_column`: a line series' elements as a column series': its line colour becomes the
/// fill.
fn to_column(ser: &mut Owned, make: &Make) {
    let colour = take_shape_colour(ser, &["ln", "solidFill"]);
    remove_all(ser, &["marker", "smooth"]);
    for at in ser.places("dPt") {
        if let Some(point) = ser.at(at) {
            remove_all(point, &["marker"]);
        }
    }
    let fill = colour.unwrap_or_else(|| make.a("noFill"));
    let shape = make
        .c("spPr")
        .with(make.top(fill))
        .with(make.top(make.a("ln").with(make.a("noFill"))))
        .with(make.top(make.a("effectLst")));
    ser.insert_ordered(Some(SER_ORDER), shape);
    ser.insert_ordered(Some(SER_ORDER), make.c("invertIfNegative").with_attr("val", "0"));
}

/// A series' `c:order` as `_insert_series` reads it (0 where it is not a number).
fn order_of(ser: &Element) -> i64 {
    child(ser, "order")
        .and_then(|n| attr(n, "val"))
        .and_then(py_int)
        .unwrap_or(0)
}

/// `_insert_series`: `ser` into `plot` in plot order (`c:order`), after the plot's other
/// series.
fn insert_series(plot: &mut Owned, ser: Owned) {
    let own = order_of(&ser.clone().close());
    let others = plot.places("ser");
    let later = others
        .iter()
        .copied()
        .find(|&at| plot.read(at).is_some_and(|s| order_of(&s) > own));
    if let Some(at) = later {
        plot.insert(at, ser);
    } else if let Some(&last) = others.last() {
        let at = plot.after(last);
        plot.insert(at, ser);
    } else {
        match plot
            .elements()
            .into_iter()
            .find(|(_, n)| !PLOT_HEAD.contains(&n.as_str()))
        {
            Some((at, _)) => plot.insert(at, ser),
            None => plot.push(ser),
        }
    }
}

/// `_drop_plot`: the emptied plot at `place` taken out, with the axes no other plot uses.
fn drop_plot(area: &mut Owned, place: usize) {
    let ids_of = |e: &Element| -> Vec<Option<String>> {
        children(e, "axId").map(|n| attr(n, "val").map(str::to_owned)).collect()
    };
    let Some(plot) = area.read(place) else { return };
    let ids = ids_of(&plot);
    area.remove(place);
    let mut used = Vec::new();
    for (at, name) in area.elements() {
        if name.ends_with("Chart") {
            if let Some(other) = area.read(at) {
                used.extend(ids_of(&other));
            }
        }
    }
    let doomed: Vec<usize> = area
        .elements()
        .into_iter()
        .filter_map(|(at, _)| {
            // `_ax_id`: none where the child has no c:axId, as where it has no value
            let first = area.read(at).and_then(|e| ids_of(&e).into_iter().next().flatten());
            (ids.contains(&first) && !used.contains(&first)).then_some(at)
        })
        .rev()
        .collect();
    for at in doomed {
        area.remove(at);
    }
}
