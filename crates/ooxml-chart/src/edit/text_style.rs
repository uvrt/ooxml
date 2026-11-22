//! Owns: the chart's text styled in the chart part (ooxml-edit's
//! `ChartStyling.set_text_style`, `_text_holders`, `charts/styling.py`): the title's, the
//! axis titles', the axis labels', the data labels' and the legend's character properties
//! -- each holder's `c:txPr` made in Office's chart text style where it has none -- and,
//! for all the text, the chart's own default (`c:chartSpace/c:txPr`).
//!
//! Must not: decide the sizes or language new text properties get (the caller gives the
//! application's look and the document's language).

use super::format::{defaults_of, rich_properties, style_characters, TextLook};
use super::look::text_properties;
use super::make::Make;
use super::owned::{Order, Owned};
use super::title::NeedsLanguage;
use super::{ChartTree, DataError};
use ooxml_xml::view::{attr, child, local};
use ooxml_xml::Element;

/// The parts of a chart's text (`TEXT_PARTS`).
pub const TEXT_PARTS: [&str; 5] = ["title", "axis_titles", "axis_labels", "data_labels", "legend"];
/// The axis elements.
const AXES: [&str; 4] = ["valAx", "catAx", "dateAx", "serAx"];
/// What a chart space's `c:txPr` goes before, in this order of preference.
const SPACE_AFTER: [&str; 4] = ["externalData", "printSettings", "userShapes", "extLst"];
/// `c:dLbls`' and `c:legend`'s sequences, as far as `c:txPr`.
pub(crate) const DLBLS_ORDER: Order = &[
    &["dLbl"],
    &["delete"],
    &["numFmt"],
    &["spPr"],
    &["txPr"],
    &["dLblPos"],
    &["showLegendKey"],
    &["showVal"],
    &["showCatName"],
    &["showSerName"],
    &["showPercent"],
    &["showBubbleSize"],
    &["separator"],
    &["showLeaderLines"],
    &["leaderLines"],
    &["extLst"],
];
const LEGEND_ORDER: Order = &[
    &["legendPos"],
    &["legendEntry"],
    &["layout"],
    &["overlay"],
    &["spPr"],
    &["txPr"],
    &["extLst"],
];

/// The application's sizes for text properties made new (`ChartLook.labels`,
/// `.data_labels`, hundredths of a point; none: the theme's) and the language their text
/// is in.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextHost<'a> {
    /// Axis labels' and the legend's.
    pub labels: Option<u32>,
    /// Data labels'.
    pub data_labels: Option<u32>,
    /// The language (`None`: the session's, which the caller holds).
    pub lang: Option<&'a str>,
}

/// Why `set_text_style` stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextStop {
    /// Refused (`FormatError`).
    Refused(DataError),
    /// New text properties are needed and the host gives no language.
    NeedsLanguage(NeedsLanguage),
}

impl From<DataError> for TextStop {
    fn from(error: DataError) -> TextStop {
        TextStop::Refused(error)
    }
}

impl ChartTree {
    /// Style the text of `part` (one of [`TEXT_PARTS`]; `None`: all of it and the chart's
    /// default) with `look` (`set_text_style`, past its checks of the look).
    ///
    /// # Errors
    /// [`TextStop`]: a chart that shows no such text, a font that is no typeface name, or
    /// text properties to make without a language.
    pub fn set_text_style(
        &mut self,
        address: &str,
        part: Option<&str>,
        look: &TextLook,
        host: &TextHost<'_>,
    ) -> Result<(), TextStop> {
        if host.lang.is_none() && self.makes_text_properties(part) {
            return Err(TextStop::NeedsLanguage(NeedsLanguage));
        }
        let styler = Styler {
            make: Make::of(&self.root),
            look,
            host,
        };
        let wants = |name: &str| part.is_none_or(|p| p == name);
        let mut root = Owned::open(&self.root);
        let mut count = 0usize;
        if let Some(chart) = root.child("chart") {
            count += styler.chart(chart, &wants)?;
        }
        if wants("data_labels") {
            count += each_labels(&mut root, &mut |labels| styler.labels(labels))?;
        }
        if wants("legend") {
            if let Some(legend) = root.child("chart").and_then(|c| c.child("legend")) {
                styler.txpr(legend, LEGEND_ORDER, host.labels)?;
                count += 1;
            }
        }
        if part.is_none() {
            styler.space(&mut root)?;
            count += 1;
        }
        if count == 0 {
            let what = part.unwrap_or("text").replace('_', " ");
            return Err(TextStop::Refused(DataError(format!(
                "{address}: the chart shows no {what}"
            ))));
        }
        self.root = std::sync::Arc::new(root.close());
        Ok(())
    }

    /// Whether styling `part` makes a `c:txPr` (whose text then needs a language).
    fn makes_text_properties(&self, part: Option<&str>) -> bool {
        let wants = |name: &str| part.is_none_or(|p| p == name);
        let lacks = |e: &Element| child(e, "txPr").is_none();
        let chart = child(&self.root, "chart");
        let area = chart.and_then(|c| child(c, "plotArea"));
        let axes = area
            .into_iter()
            .flat_map(|a| a.children().iter())
            .filter_map(|n| n.as_element());
        if wants("axis_labels")
            && axes
                .filter(|a| AXES.contains(&local(a)))
                .any(|a| !labels_hidden(a) && lacks(a))
        {
            return true;
        }
        if wants("data_labels") && any_labels(&self.root, &|l| !deleted_element(l) && lacks(l)) {
            return true;
        }
        if wants("legend") && chart.and_then(|c| child(c, "legend")).is_some_and(lacks) {
            return true;
        }
        part.is_none() && lacks(&self.root)
    }
}

/// What styles the holders: the look, and what new text properties are made with.
struct Styler<'a> {
    make: Make,
    look: &'a TextLook,
    host: &'a TextHost<'a>,
}

impl Styler<'_> {
    fn style(&self, properties: &mut Owned) -> Result<(), DataError> {
        style_characters(properties, &self.make, self.look)
    }

    /// `owner`'s `c:txPr` (made in Office's style at `size` where it has none, `txpr`):
    /// its defaults styled.
    fn txpr(&self, owner: &mut Owned, order: Order, size: Option<u32>) -> Result<(), DataError> {
        let make = &self.make;
        let at = match owner.place("txPr") {
            Some(at) => at,
            None => owner.insert_ordered(
                Some(order),
                make.top(text_properties(make, size, 65000, self.host.lang)),
            ),
        };
        let text = owner.at(at).unwrap_or_else(|| unreachable!("a c:txPr"));
        self.style(defaults_of(text, make))
    }

    /// The title's, the axis titles' and the axis labels' holders in `c:chart`.
    fn chart(&self, chart: &mut Owned, wants: &dyn Fn(&str) -> bool) -> Result<usize, DataError> {
        let mut count = 0;
        let mut style = |p: &mut Owned| self.style(p);
        if wants("title") {
            if let Some(title) = chart.child("title") {
                count += title_holders(title, &self.make, &mut style)?;
            }
        }
        let Some(area) = chart.child("plotArea") else {
            return Ok(count);
        };
        for (at, name) in area.elements() {
            if !AXES.contains(&name.as_str()) {
                continue;
            }
            let Some(axis) = area.at(at) else { continue };
            if wants("axis_titles") {
                if let Some(title) = axis.child("title") {
                    count += title_holders(title, &self.make, &mut style)?;
                }
            }
            if wants("axis_labels") && !deleted_axis(axis) {
                self.txpr(axis, super::axes::axis_order(&name), self.host.labels)?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// One `c:dLbls`' holders: its `c:txPr`'s, then each label's of its own.
    fn labels(&self, labels: &mut Owned) -> Result<usize, DataError> {
        self.txpr(labels, DLBLS_ORDER, self.host.data_labels)?;
        let mut found = 1;
        let mut style = |p: &mut Owned| self.style(p);
        for at in labels.places("dLbl") {
            let Some(label) = labels.at(at) else { continue };
            if deleted(label) {
                continue;
            }
            if let Some(rich) = label.child("tx").and_then(|t| t.child("rich")) {
                found += rich_properties(rich, &self.make, &mut style)?;
            } else if let Some(text) = label.child("txPr") {
                style(defaults_of(text, &self.make))?;
                found += 1;
            }
        }
        Ok(found)
    }

    /// The chart's default text (`c:chartSpace/c:txPr`, made before what follows it).
    fn space(&self, root: &mut Owned) -> Result<(), DataError> {
        let make = &self.make;
        let at = match root.place("txPr") {
            Some(at) => at,
            None => {
                let text = make
                    .c("txPr")
                    .with(make.a("bodyPr"))
                    .with(make.a("lstStyle"))
                    .with(make.a("p"));
                match SPACE_AFTER.iter().find_map(|name| root.place(name)) {
                    Some(at) => {
                        root.insert(at, make.top(text));
                        at
                    }
                    None => {
                        root.push(make.top(text));
                        root.elements().last().map_or(0, |(at, _)| *at)
                    }
                }
            }
        };
        let text = root.at(at).unwrap_or_else(|| unreachable!("a c:txPr"));
        self.style(defaults_of(text, make))
    }
}

/// A title's holders (`title_props`): its rich text's, then its `c:txPr`'s defaults.
fn title_holders(
    title: &mut Owned,
    make: &Make,
    style: &mut dyn FnMut(&mut Owned) -> Result<(), DataError>,
) -> Result<usize, DataError> {
    let mut count = 0;
    if let Some(rich) = title.child("tx").and_then(|t| t.child("rich")) {
        count += rich_properties(rich, make, style)?;
    }
    if let Some(text) = title.child("txPr") {
        style(defaults_of(text, make))?;
        count += 1;
    }
    Ok(count)
}

/// Each `c:dLbls` below `owner` that is not deleted, in document order, given to `each`;
/// the holders it counted.
fn each_labels(
    owner: &mut Owned,
    each: &mut dyn FnMut(&mut Owned) -> Result<usize, DataError>,
) -> Result<usize, DataError> {
    let mut count = 0;
    for (at, name) in owner.elements() {
        let Some(read) = owner.read(at) else { continue };
        if name != "dLbls" && !any_labels(&read, &|_| true) {
            continue;
        }
        let Some(node) = owner.at(at) else { continue };
        if name == "dLbls" && !deleted(node) {
            count += each(node)?;
        }
        count += each_labels(node, each)?;
    }
    Ok(count)
}

/// Whether some `c:dLbls` below `element` passes `test`.
fn any_labels(element: &Element, test: &dyn Fn(&Element) -> bool) -> bool {
    element
        .children()
        .iter()
        .filter_map(|n| n.as_element())
        .any(|e| (local(e) == "dLbls" && test(e)) || any_labels(e, test))
}

/// `_deleted`: a `c:delete` that is on (on where its value is left out).
fn deleted_element(holder: &Element) -> bool {
    child(holder, "delete").is_some_and(|d| matches!(attr(d, "val").unwrap_or("1"), "1" | "true"))
}

/// An axis whose labels are hidden: `c:delete` on, only where it says so.
fn labels_hidden(axis: &Element) -> bool {
    child(axis, "delete").is_some_and(|d| matches!(attr(d, "val"), Some("1" | "true")))
}

fn deleted(holder: &Owned) -> bool {
    holder
        .place("delete")
        .and_then(|at| holder.read(at))
        .is_some_and(|d| matches!(attr(&d, "val").unwrap_or("1"), "1" | "true"))
}

/// An axis whose labels are hidden (`c:delete` on; on only where it says so).
fn deleted_axis(axis: &Owned) -> bool {
    axis.place("delete")
        .and_then(|at| axis.read(at))
        .is_some_and(|d| matches!(attr(&d, "val"), Some("1" | "true")))
}
