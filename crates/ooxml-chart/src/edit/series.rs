//! Owns: a chart's series and categories changed in number in the chart part, as
//! ooxml-edit's `Chart` changes them (`charts/chart.py`): a category put in or taken out of
//! every source of every series (`add_category`, `remove_category`), a series made from its
//! template (`_new_series`: its look dropped for the application's automatic colours, a dot
//! or line-less scatter series' markers kept, its own `c16:uniqueId`, `_give_unique_id`;
//! its plot order, `_renumber_order`) or taken out with its legend entries
//! (`remove_series`), and a source's formula given a new range or turned into a literal.
//!
//! Must not: write the workbook or check that it can follow (the caller does, with
//! `ooxml-sheets`), or hash (the caller gives SHA-256, as ooxml-edit takes `hashlib`'s).

use super::make::Make;
use super::owned::Owned;
use super::points::{insert_point, reference, shift_point_formats, to_literal};
use super::{encode, ChartTree, DataError, PointValue, Which};
use ooxml_schema::orders::local as schema;
use ooxml_xml::view::{attr, child, local, py_int};
use std::sync::Arc;

/// The sources with one point per category (`point_sources`), in their order.
pub const POINT_SOURCES: [Which; 4] = [Which::Categories, Which::Values, Which::Sizes, Which::Labels];

/// The `c:ext` a series' `c16:uniqueId` is written in (`SERIES_ID_EXT`), and its namespace.
const SERIES_ID_EXT: &str = "{C3380CC4-5D6E-409C-BE32-E72D297353CC}";
const C16: &str = "http://schemas.microsoft.com/office/drawing/2014/chart";
/// A new scatter series' line width, EMU (`SCATTER_LINE_WIDTH`), and a line series' (`LINE_WIDTH`).
const SCATTER_LINE_WIDTH: u32 = 19050;
const LINE_WIDTH: u32 = 28575;
/// Office's automatic series colours (`ACCENTS`, `VARIATIONS`).
const ACCENTS: [&str; 6] = ["accent1", "accent2", "accent3", "accent4", "accent5", "accent6"];
const VARIATIONS: [&[(&str, u32)]; 9] = [
    &[],
    &[("lumMod", 60000)],
    &[("lumMod", 80000), ("lumOff", 20000)],
    &[("lumMod", 80000)],
    &[("lumMod", 60000), ("lumOff", 40000)],
    &[("lumMod", 50000)],
    &[("lumMod", 70000), ("lumOff", 30000)],
    &[("lumMod", 70000)],
    &[("lumMod", 50000), ("lumOff", 50000)],
];

/// SHA-256 of some bytes (`hashlib.sha256(...).digest()`).
pub type Hash<'h> = &'h dyn Fn(&[u8]) -> [u8; 32];

impl ChartTree {
    /// Change the `c:ser` of `series` with `change`; `None` where there is no such series.
    pub(crate) fn change_series<R>(&mut self, series: usize, change: impl FnOnce(&mut Owned) -> R) -> Option<R> {
        let (path, ser) = self.series(series)?;
        let mut owned = Owned::open(ser);
        let out = change(&mut owned);
        let changed = owned.close();
        self.root = Arc::new(super::path::update(&self.root, &path, |_| changed));
        Some(out)
    }

    /// Change the root with `change`.
    pub(crate) fn change_root<R>(&mut self, change: impl FnOnce(&mut Owned) -> R) -> R {
        let mut owned = Owned::open(&self.root);
        let out = change(&mut owned);
        self.root = Arc::new(owned.close());
        out
    }

    /// The most points any source of any series holds (`point_count`).
    #[must_use]
    pub fn point_count(&self) -> usize {
        (0..self.series_count())
            .map(|series| self.series_point_count(series))
            .max()
            .unwrap_or(0)
    }

    /// A new point at `index` in every source of every series (`add_category`): the
    /// categories get `label`, the values their series' value of `values`, other sources a
    /// blank; per-point formats move along.
    ///
    /// # Errors
    /// [`DataError`] for text in numeric categories, multi-level categories, or a source
    /// without a cache.
    pub fn insert_category(
        &mut self,
        index: usize,
        label: Option<&PointValue>,
        values: &[Option<f64>],
    ) -> Result<(), DataError> {
        for series in 0..self.series_count() {
            let value = values.get(series).copied().flatten().map(PointValue::Number);
            let done = self.change_series(series, |ser| {
                for which in POINT_SOURCES {
                    let given = match which {
                        Which::Categories => label,
                        Which::Values => value.as_ref(),
                        _ => None,
                    };
                    if let Some(mut source) = reference(ser, which) {
                        source.insert(index, given)?;
                    }
                }
                shift_point_formats(ser, index, 1);
                Ok(())
            });
            done.unwrap_or(Ok(()))?;
        }
        Ok(())
    }

    /// Point `index` taken out of every source of every series (`remove_category`).
    ///
    /// # Errors
    /// [`DataError`] for multi-level categories or a source without a cache.
    pub fn delete_category(&mut self, index: usize) -> Result<(), DataError> {
        for series in 0..self.series_count() {
            let done = self.change_series(series, |ser| {
                for which in POINT_SOURCES {
                    if let Some(mut source) = reference(ser, which) {
                        source.delete(index)?;
                    }
                }
                shift_point_formats(ser, index, -1);
                Ok(())
            });
            done.unwrap_or(Ok(()))?;
        }
        Ok(())
    }

    /// Give `which`'s formula of `series` the text `formula` (`set_range`).
    pub fn set_formula(&mut self, series: usize, which: Which, formula: &str) {
        self.change_series(series, |ser| {
            if let Some(mut source) = reference(ser, which) {
                source.set_formula(formula);
            }
        });
    }

    /// `which`'s formula-backed source of `series` turned into a literal (`_to_literal`).
    pub fn to_literal(&mut self, series: usize, which: Which) {
        self.change_series(series, |ser| to_literal(ser, which));
    }

    /// `_name_to_literal`: `series`' name as the literal `name` (none: no `c:tx`).
    pub fn name_to_literal(&mut self, series: usize, name: Option<&str>) {
        self.change_series(series, |ser| {
            let tx = ser.place("tx");
            let value = tx.and_then(|at| ser.read(at)).and_then(|tx| {
                tx.child_elements()
                    .find(|e| ["strRef", "numRef", "multiLvlStrRef", "strLit", "numLit", "v"].contains(&local(e)))
                    .map(|e| local(e) == "v")
            });
            match (name, tx) {
                (Some(name), Some(at)) if value == Some(true) => {
                    if let Some(v) = ser.at(at).and_then(|tx| tx.child("v")) {
                        v.set_text(name);
                    }
                }
                (None, Some(at)) => {
                    ser.remove(at);
                }
                (None, None) => {}
                (Some(name), _) => give_name(ser, name),
            }
        });
    }

    /// The `c:idx` of `series` as ooxml-edit reads it (`idx`: 0 where it is not a number).
    #[must_use]
    pub fn series_idx(&self, series: usize) -> i64 {
        self.series(series).map_or(0, |(_, ser)| number_of(ser, "idx"))
    }

    /// Take out `series` and the legend's entries for it (`remove_series`).
    pub fn delete_series(&mut self, series: usize) {
        let idx = self.series_idx(series).to_string();
        let Some((path, _)) = self.series(series) else { return };
        let (last, parent_path) = path.split_last().unwrap_or((&0, &[]));
        let (last, parent_path) = (*last, parent_path.to_vec());
        self.change_root(|root| {
            if let Some(legend) = root.child("chart").and_then(|c| c.child("legend")) {
                let doomed: Vec<usize> = legend
                    .places("legendEntry")
                    .into_iter()
                    .filter(|&at| {
                        legend
                            .read(at)
                            .and_then(|e| child(&e, "idx").and_then(|i| attr(i, "val")).map(|v| v == idx))
                            .unwrap_or(false)
                    })
                    .collect();
                for at in doomed.into_iter().rev() {
                    legend.remove(at);
                }
            }
            if let Some(plot) = super::points::open_at(root, &parent_path) {
                plot.remove(last);
            }
        });
    }

    /// A new series after the last one, in its plot (`add_series` without an index): a copy
    /// of the last series (`_new_series`) with `name` and `values`, its own unique id
    /// (`_give_unique_id`, from `part`'s name and the ids the chart holds) and plot order.
    /// Its data keeps the template's formulas; the caller points them at the workbook or
    /// turns them into literals.
    ///
    /// # Errors
    /// [`DataError`] for a template without a value cache.
    pub fn append_series(
        &mut self,
        name: Option<&str>,
        values: &[Option<f64>],
        part: &str,
        hash: Hash<'_>,
    ) -> Result<(), DataError> {
        let count = self.series_count();
        let Some((path, template)) = self.series(count.wrapping_sub(1)) else {
            return Err(DataError(
                "a chart with no series has no template to add one from".to_owned(),
            ));
        };
        let make = Make::of(&self.root);
        let plot_local = path::plot_local(&self.root, &path);
        let idxs: Vec<i64> = (0..count).map(|s| self.series_idx(s)).collect();
        let orders: Vec<i64> = (0..count)
            .map(|s| self.series(s).map_or(0, |(_, e)| number_of(e, "order")))
            .collect();
        let idx = idxs.iter().copied().max().unwrap_or(-1) + 1;
        let plot_marker = path::plot_marker(&self.root, &path);
        let mut element = new_series(template, &make, idx, plot_local.as_deref(), plot_marker, name, values)?;
        let taken = unique_ids(&self.root);
        give_unique_id(&mut element, &make, part, idx, &taken, hash);
        // `_renumber_order` at the end: the order after every other
        let wanted = orders.iter().copied().max().unwrap_or(-1) + 1;
        if let Some(node) = element.child("order") {
            node.set_attr("val", &wanted.to_string());
        }
        let (last, parent_path) = path.split_last().unwrap_or((&0, &[]));
        let (last, parent_path) = (*last, parent_path.to_vec());
        self.change_root(|root| {
            if let Some(plot) = super::points::open_at(root, &parent_path) {
                let tail = plot.tail(last);
                let at = plot.after(last);
                plot.insert(at, element);
                if !tail.is_empty() {
                    plot.insert_text(at + 1, &tail);
                }
            }
        });
        Ok(())
    }
}

/// The paths' helpers: what the plot a series is in says.
mod path {
    use ooxml_xml::view::{attr, child, local};
    use ooxml_xml::Element;

    fn plot<'a>(root: &'a Element, path: &[usize]) -> Option<&'a Element> {
        super::super::path::at(root, path.split_last()?.1)
    }

    pub(super) fn plot_local(root: &Element, path: &[usize]) -> Option<String> {
        plot(root, path).map(|p| local(p).to_owned())
    }

    /// Whether the plot's `c:marker` is on (`val` 1 or true; on where it says none).
    pub(super) fn plot_marker(root: &Element, path: &[usize]) -> bool {
        plot(root, path)
            .and_then(|p| child(p, "marker"))
            .is_some_and(|m| matches!(attr(m, "val").unwrap_or("1"), "1" | "true"))
    }
}

/// A `c:ser`'s `c:idx` or `c:order` as ooxml-edit reads it (0 where it is not a number).
fn number_of(ser: &ooxml_xml::Element, name: &str) -> i64 {
    child(ser, name)
        .and_then(|n| attr(n, "val"))
        .and_then(py_int)
        .unwrap_or(0)
}

/// Every `c16:uniqueId` value the chart holds.
fn unique_ids(root: &ooxml_xml::Element) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(element) = stack.pop() {
        if local(element) == "uniqueId" && element.name().namespace() == Some(C16) {
            if let Some(value) = attr(element, "val") {
                out.push(value.to_owned());
            }
        }
        stack.extend(element.child_elements().map(|e| &**e));
    }
    out
}

/// Whether the series draws no line (`c:spPr/a:ln/a:noFill`).
fn lineless(ser: &ooxml_xml::Element) -> bool {
    child(ser, "spPr")
        .and_then(|s| child(s, "ln"))
        .and_then(|l| child(l, "noFill"))
        .is_some()
}

/// `_new_series`.
fn new_series(
    template: &ooxml_xml::Element,
    make: &Make,
    idx: i64,
    plot: Option<&str>,
    plot_marker: bool,
    name: Option<&str>,
    values: &[Option<f64>],
) -> Result<Owned, DataError> {
    let scatter_lineless = plot == Some("scatterChart") && lineless(template);
    let dot = !scatter_lineless
        && lineless(template)
        && match child(template, "marker").and_then(|m| child(m, "symbol")) {
            Some(symbol) => attr(symbol, "val") != Some("none"),
            None => plot_marker,
        };
    let mut element = Owned::open(template);
    for tag in ["spPr", "dPt", "trendline", "errBars", "extLst", "explosion"] {
        for at in element.places(tag).into_iter().rev() {
            element.remove(at);
        }
    }
    if let Some(marker) = element.child("marker") {
        for at in marker.places("spPr").into_iter().rev() {
            marker.remove(at);
        }
    }
    if let Some(labels) = element.child("dLbls") {
        for at in labels.places("dLbl").into_iter().rev() {
            labels.remove(at);
        }
    }
    let c = Arc::clone(element.name());
    let idx_text = idx.to_string();
    element.sub(&c, "idx", Some(schema::SER)).set_attr("val", &idx_text);
    element.sub(&c, "order", Some(schema::SER)).set_attr("val", &idx_text);
    let index = usize::try_from(idx).unwrap_or(0);
    if dot {
        if let Some(at) = element.place("marker") {
            element.remove(at);
        }
        element.insert_ordered(Some(schema::SER), no_line_shape(make, LINE_WIDTH));
        let marker = make
            .c("marker")
            .with(make.c("symbol").with_attr("val", "circle"))
            .with(make.c("size").with_attr("val", "5"))
            .with(marker_shape(make, index));
        element.insert_ordered(Some(schema::SER), marker);
    } else if scatter_lineless {
        element.insert_ordered(Some(schema::SER), no_line_shape(make, SCATTER_LINE_WIDTH));
        if let Some(marker) = element.child("marker") {
            let symbol = marker.place("symbol").and_then(|at| marker.read(at));
            if symbol.is_none_or(|s| attr(&s, "val") != Some("none")) {
                marker.insert_ordered(Some(schema::MARKER), marker_shape(make, index));
            }
        }
    }
    new_name(&mut element, name);
    new_data(&mut element, values)?;
    Ok(element)
}

/// `_new_series`' name: a name reference's cache given the new name, or a literal name.
fn new_name(element: &mut Owned, name: Option<&str>) {
    let c = Arc::clone(element.name());
    // the name
    match reference(element, Which::Name) {
        Some(source) if source.kind == "strRef" => {
            if let Some(cache) = source.element.child("strCache") {
                for at in cache.places("pt").into_iter().rev() {
                    cache.remove(at);
                }
                cache.sub(&c, "ptCount", Some(schema::CACHE)).set_attr("val", "1");
                if let Some(name) = name {
                    insert_point(cache, &c, Some(schema::CACHE), 0, name);
                }
            }
        }
        _ => {
            if let Some(at) = element.place("tx") {
                element.remove(at);
            }
            if let Some(name) = name {
                give_name(element, name);
            }
        }
    }
}

/// `_new_series`' data: the values' cache given the new values, the sizes emptied and made
/// a literal.
fn new_data(element: &mut Owned, values: &[Option<f64>]) -> Result<(), DataError> {
    let c = Arc::clone(element.name());
    // the values
    let numeric = {
        let source = reference(element, Which::Values);
        let numeric = source
            .as_ref()
            .is_some_and(|s| matches!(s.kind.as_str(), "numRef" | "numLit"));
        let has_cache = source.is_some_and(|mut s| s.cache().is_some());
        if !has_cache {
            return Err(DataError("the template series has no value cache to copy".to_owned()));
        }
        numeric
    };
    if let Some(mut source) = reference(element, Which::Values) {
        if let Some(cache) = source.cache() {
            for at in cache.places("pt").into_iter().rev() {
                cache.remove(at);
            }
            cache
                .sub(&c, "ptCount", Some(schema::CACHE))
                .set_attr("val", &values.len().to_string());
            for (at, value) in values.iter().enumerate() {
                if let Some(value) = value {
                    let text = encode(&PointValue::Number(*value), numeric)?;
                    insert_point(cache, &c, Some(schema::CACHE), at, &text);
                }
            }
        }
    }
    // the sizes: not in the workbook, so a literal
    if let Some(mut source) = reference(element, Which::Sizes) {
        if let Some(cache) = source.cache() {
            for at in cache.places("pt").into_iter().rev() {
                cache.remove(at);
            }
            cache
                .sub(&c, "ptCount", Some(schema::CACHE))
                .set_attr("val", &values.len().to_string());
        }
        to_literal(element, Which::Sizes);
    }
    Ok(())
}

/// `_give_name`: a literal name (`c:tx/c:v`), the `c:tx` made in order where there is none.
fn give_name(ser: &mut Owned, name: &str) {
    let c = Arc::clone(ser.name());
    let tx = ser.sub(&c, "tx", Some(schema::SER));
    for at in tx.elements().into_iter().map(|(at, _)| at).rev() {
        tx.remove(at);
    }
    tx.insert_ordered(Some(schema::TX), Owned::like(&c, "v").with_text(name));
}

/// `_give_unique_id`: the series' `c16:uniqueId`, `{00000000-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`
/// from SHA-256 of the part's name, the series' `c:idx` and the ids the chart holds (sorted),
/// moved on by a step where it is taken; in a new `c:extLst`.
fn give_unique_id(element: &mut Owned, make: &Make, part: &str, idx: i64, taken: &[String], hash: Hash<'_>) {
    let mut seeds: Vec<&str> = taken.iter().map(String::as_str).filter(|v| !v.is_empty()).collect();
    seeds.sort_unstable();
    let idx_text = idx.to_string();
    let mut lines = vec![part, idx_text.as_str()];
    lines.extend(seeds);
    let seed = lines.join("\n");
    let mut step = 0u64;
    let value = loop {
        let digest = hash(format!("{seed}\n{step}").as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02X}")).collect();
        let value = format!(
            "{{00000000-{}-{}-{}-{}}}",
            &hex[0..4],
            &hex[4..8],
            &hex[8..12],
            &hex[12..24]
        );
        if !taken.contains(&value) {
            break value;
        }
        step += 1;
    };
    let mut ext = make.c("ext");
    ext.declare("c16", C16);
    ext.set_attr("uri", SERIES_ID_EXT);
    let ext = ext.with(Owned::named(Some("c16"), "uniqueId", Some(C16)).with_attr("val", &value));
    element.insert_ordered(Some(schema::SER), make.c("extLst").with(ext));
}

/// `no_line_shape`: a series' `c:spPr` with its line off.
fn no_line_shape(make: &Make, width: u32) -> Owned {
    let line = make
        .a("ln")
        .with_attr("w", &width.to_string())
        .with_attr("cap", "rnd")
        .with(make.a("noFill"))
        .with(make.a("round"));
    make.top(make.c("spPr").with(line).with(make.a("effectLst")))
}

/// `marker_shape`: a marker's `c:spPr` in series `index`'s colour.
fn marker_shape(make: &Make, index: usize) -> Owned {
    let fill = || make.a("solidFill").with(series_colour(make, index));
    let line = make.a("ln").with_attr("w", "9525").with(fill());
    make.top(make.c("spPr").with(fill()).with(line).with(make.a("effectLst")))
}

/// `series_color`: Office's automatic colour for series `index`.
pub(crate) fn series_colour(make: &Make, index: usize) -> Owned {
    let accent = ACCENTS.get(index % ACCENTS.len()).copied().unwrap_or("accent1");
    let variation = VARIATIONS
        .get((index / ACCENTS.len()) % VARIATIONS.len())
        .copied()
        .unwrap_or(&[]);
    let mut colour = make.a("schemeClr").with_attr("val", accent);
    for (key, value) in variation {
        colour.push(make.a(key).with_attr("val", &value.to_string()));
    }
    colour
}
