//! Owns: a chart part's data changed in its caches, as ooxml-edit's `charts/chart.py` changes
//! them (`_Data.set_point`, `_give_name`): one point of a series' name, categories or values
//! written into its cache (`c:pt idx=` with its `c:v`, made in index order where there is
//! none, taken out for a blank), a series' name given as a literal where it has none, and
//! each source's reference read for its formula (`c:f`), so the caller keeps the embedded
//! workbook in step.
//!
//! The tree is immutable: each change copies the path from the root down to the element it
//! changes ([`ChartTree`] holds the root as changed so far).
//!
//! Must not: write the workbook (the caller does, with `ooxml-sheets`), add or remove points
//! (`c:ptCount` stays), or know which document holds the chart.

mod axes;
mod combine;
mod format;
mod look;
mod looks;
mod make;
mod owned;
mod path;
mod point_labels;
mod points;
mod rich;
mod series;
mod text_style;
mod title;

pub use axes::ScaleChange;
pub use format::{points_to_emu, text_size, Colour, TextLook};
pub use look::{labels_show_values, LabelLook};
pub use looks::SeriesFormat;
pub use point_labels::{PointLabels, LABEL_POSITIONS};
pub use rich::{paragraph_text, rebuild_paragraph, replace_body_text};
pub use series::{Hash, POINT_SOURCES};
pub use text_style::{TextHost, TextStop, TEXT_PARTS};
pub use title::{NeedsLanguage, TitleEdit, TitleLook};

use crate::data::{Point, Source, Unreadable};

use ooxml_base::pyfmt::repr_str;
use ooxml_xml::view::{attr, children, local, py_int, text};
use ooxml_xml::{Atom, Element, Node, QName};
use path::{indexed, update};
use std::sync::Arc;

/// The plot elements a plot area may hold (ooxml-edit's `PLOT_TAGS`).
const PLOTS: [&str; 16] = [
    "areaChart",
    "area3DChart",
    "lineChart",
    "line3DChart",
    "stockChart",
    "radarChart",
    "scatterChart",
    "pieChart",
    "pie3DChart",
    "doughnutChart",
    "barChart",
    "bar3DChart",
    "ofPieChart",
    "surfaceChart",
    "surface3DChart",
    "bubbleChart",
];

/// The references a data source may hold (`_REF_TAGS`).
const REFERENCES: [&str; 6] = ["strRef", "numRef", "multiLvlStrRef", "strLit", "numLit", "v"];

/// `CT_NumData` / `CT_StrData`'s sequence: where a new point goes among a cache's children.
const CACHE_ORDER: [&[&str]; 4] = [&["formatCode"], &["ptCount"], &["pt"], &["extLst"]];

/// Where a series' `c:tx` goes: after `c:idx` and `c:order`, as every series type has them.
const SERIES_HEAD: [&[&str]; 3] = [&["idx"], &["order"], &["tx"]];

/// A series' data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// Its name (`c:tx`).
    Name,
    /// Its categories (`c:cat`, else `c:xVal`).
    Categories,
    /// Its values (`c:val`, else `c:yVal`).
    Values,
    /// A bubble chart's sizes (`c:bubbleSize`).
    Sizes,
    /// Its Value From Cells labels (`c15:datalabelsRange`, in its `c:extLst`).
    Labels,
}

/// A value a point is given.
#[derive(Debug, Clone, PartialEq)]
pub enum PointValue {
    /// A blank: the point goes from the cache.
    Blank,
    /// Text.
    Text(String),
    /// A finite number.
    Number(f64),
}

/// An edit ooxml-edit refuses with `ChartDataError`: its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataError(pub String);

/// A source's reference as an edit reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    /// The reference's local name (`numRef`, `strLit`, `v`...).
    pub kind: String,
    /// Its formula's text, where it has a `c:f` (`None` for a literal; `Some("")` for an
    /// empty one).
    pub formula: Option<String>,
}

impl SourceRef {
    /// A number reference or literal.
    #[must_use]
    pub fn numeric(&self) -> bool {
        matches!(self.kind.as_str(), "numRef" | "numLit")
    }
}

/// A chart part's root, as changed so far.
#[derive(Debug, Clone)]
pub struct ChartTree {
    root: Arc<Element>,
}

impl ChartTree {
    /// The chart part whose root (`c:chartSpace`) is `root`.
    #[must_use]
    pub fn new(root: Arc<Element>) -> ChartTree {
        ChartTree { root }
    }

    /// The root as changed so far.
    #[must_use]
    pub fn root(&self) -> &Arc<Element> {
        &self.root
    }

    /// The path (child places) to each `c:ser`, plot by plot in the plot area's order.
    fn series_paths(&self) -> Vec<Vec<usize>> {
        let Some((chart_at, chart)) = indexed(&self.root).find(|(_, e)| local(e) == "chart") else {
            return Vec::new();
        };
        let Some((area_at, area)) = indexed(chart).find(|(_, e)| local(e) == "plotArea") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (plot_at, plot) in indexed(area).filter(|(_, e)| PLOTS.contains(&local(e))) {
            for (ser_at, _) in indexed(plot).filter(|(_, e)| local(e) == "ser") {
                out.push(vec![chart_at, area_at, plot_at, ser_at]);
            }
        }
        out
    }

    /// How many series the chart has.
    #[must_use]
    pub fn series_count(&self) -> usize {
        self.series_paths().len()
    }

    fn series(&self, series: usize) -> Option<(Vec<usize>, &Element)> {
        let path = self.series_paths().into_iter().nth(series)?;
        let element = path::at(&self.root, &path)?;
        Some((path, element))
    }

    /// The path to `which`'s reference of `series` (its container's place, then the
    /// reference's), and the reference.
    fn reference(&self, series: usize, which: Which) -> Option<(Vec<usize>, &Element)> {
        let (mut path, ser) = self.series(series)?;
        let (container_at, container) = container(ser, which)?;
        let (reference_at, reference) = indexed(container).find(|(_, e)| REFERENCES.contains(&local(e)))?;
        path.extend([container_at, reference_at]);
        Some((path, reference))
    }

    /// `which`'s reference of `series`; `None` where the source has none.
    #[must_use]
    pub fn source(&self, series: usize, which: Which) -> Option<SourceRef> {
        let (_, ser) = self.series(series)?;
        let mut owned = owned::Owned::open(ser);
        let reference = points::reference(&mut owned, which)?;
        Some(SourceRef {
            formula: reference.formula(),
            kind: reference.kind,
        })
    }

    /// The points of `which` of `series` as ooxml-edit reads them (`_Data.points`); none
    /// for a series or source that is not there.
    ///
    /// # Errors
    /// [`Unreadable`] for a source holding more points than the model reads.
    pub fn points(&self, series: usize, which: Which) -> Result<Vec<Option<Point>>, Unreadable> {
        let Some((_, ser)) = self.series(series) else {
            return Ok(Vec::new());
        };
        Source::of(container(ser, which).map(|(_, c)| c)).points()
    }

    /// Write point `index` of `which` of `series` (`_Data.set_point`): whether the cache
    /// changed.
    ///
    /// # Errors
    /// [`DataError`] for text in numeric data, a multi-level cache, or a source without a
    /// cache to write.
    pub fn set_point(
        &mut self,
        series: usize,
        which: Which,
        index: usize,
        value: &PointValue,
    ) -> Result<bool, DataError> {
        let Some((path, reference)) = self.reference(series, which) else {
            return Err(DataError("this data has no cache to write".to_owned()));
        };
        let kind = local(reference).to_owned();
        let numeric = matches!(kind.as_str(), "numRef" | "numLit");
        if kind == "v" {
            let written = match value {
                PointValue::Blank => String::new(),
                other => encode(other, numeric)?,
            };
            if text(reference) == written {
                return Ok(false);
            }
            let changed = with_text(reference, &written);
            self.root = Arc::new(update(&self.root, &path, |_| changed));
            return Ok(true);
        }
        let (parent_path, parent) = points_parent(reference, &kind)?;
        let mut full = path;
        full.extend(parent_path);
        let wanted = i64::try_from(index).unwrap_or(i64::MAX);
        let current = indexed(parent).find(|(_, pt)| local(pt) == "pt" && point_index(pt) == wanted);
        let changed = match (current, value) {
            (None, PointValue::Blank) => return Ok(false),
            (Some((at, _)), PointValue::Blank) => path::without(parent, at),
            (Some((at, pt)), value) => {
                let written = encode(value, numeric)?;
                let v = indexed(pt).find(|(_, e)| local(e) == "v");
                let pt = match v {
                    Some((_, v)) if text(v) == written => return Ok(false),
                    Some((v_at, v)) => path::replaced(pt, v_at, with_text(v, &written)),
                    None => path::appended(pt, text_element(pt, "v", &written)),
                };
                path::replaced(parent, at, pt)
            }
            (None, value) => {
                let written = encode(value, numeric)?;
                let pt = Element::new(
                    qualified(parent, "pt"),
                    Vec::new(),
                    vec![ooxml_xml::Attribute {
                        name: Arc::new(QName::new(None, "idx", None)),
                        value: Atom::from(index.to_string()),
                    }],
                    vec![Node::Element(Arc::new(text_element(parent, "v", &written)))],
                );
                let later = indexed(parent).find(|(_, p)| local(p) == "pt" && point_index(p) > wanted);
                match later {
                    Some((at, _)) => path::inserted(parent, at, pt),
                    None => ooxml_xml::view::insert_ordered(parent, &CACHE_ORDER, pt),
                }
            }
        };
        self.root = Arc::new(update(&self.root, &full, |_| changed));
        Ok(true)
    }

    /// Give `series`, which has no name of its own, the literal name `name` (`_give_name`:
    /// `c:tx/c:v`, the `c:tx` made after `c:idx` and `c:order` where there is none).
    pub fn give_name(&mut self, series: usize, name: &str) {
        let Some((path, ser)) = self.series(series) else { return };
        let value = Node::Element(Arc::new(text_element(ser, "v", name)));
        let changed = match indexed(ser).find(|(_, e)| local(e) == "tx") {
            Some((at, tx)) => path::replaced(ser, at, tx.with_children(ooxml_xml::Children::from_vec(vec![value]))),
            None => {
                let tx = Element::new(qualified(ser, "tx"), Vec::new(), Vec::new(), vec![value]);
                ooxml_xml::view::insert_ordered(ser, &SERIES_HEAD, tx)
            }
        };
        self.root = Arc::new(update(&self.root, &path, |_| changed));
    }
}

/// `which`'s container in the series `ser`, with its place.
fn container(ser: &Element, which: Which) -> Option<(usize, &Element)> {
    let first = |names: &[&str]| names.iter().find_map(|n| indexed(ser).find(|(_, e)| local(e) == *n));
    match which {
        Which::Name => first(&["tx"]),
        Which::Categories => first(&["cat", "xVal"]),
        Which::Values => first(&["val", "yVal"]),
        Which::Sizes => first(&["bubbleSize"]),
        Which::Labels => None,
    }
}

/// Where a reference's points are, below it (`_writable`): its cache, or a literal itself.
fn points_parent<'a>(reference: &'a Element, kind: &str) -> Result<(Vec<usize>, &'a Element), DataError> {
    let no_cache = || DataError("this data has no cache to write".to_owned());
    match kind {
        "strLit" | "numLit" => Ok((Vec::new(), reference)),
        "strRef" | "numRef" => {
            let name = if kind == "strRef" { "strCache" } else { "numCache" };
            let (at, cache) = indexed(reference)
                .find(|(_, e)| local(e) == name)
                .ok_or_else(no_cache)?;
            Ok((vec![at], cache))
        }
        "multiLvlStrRef" => {
            let Some((at, cache)) = indexed(reference).find(|(_, e)| local(e) == "multiLvlStrCache") else {
                return Err(no_cache());
            };
            if children(cache, "lvl").count() != 1 {
                return Err(DataError(
                    "multi-level categories are not editable; edit them in the chart's workbook".to_owned(),
                ));
            }
            let (level_at, level) = indexed(cache).find(|(_, e)| local(e) == "lvl").ok_or_else(no_cache)?;
            Ok((vec![at, level_at], level))
        }
        _ => Err(no_cache()),
    }
}

/// A point's `idx` as Python's `int()` reads it (`_idx`: `-1` where it is not one).
fn point_index(point: &Element) -> i64 {
    attr(point, "idx").map_or(Some(-1), py_int).unwrap_or(-1)
}

/// A value as its cache writes it (`_encode`).
fn encode(value: &PointValue, numeric: bool) -> Result<String, DataError> {
    match value {
        PointValue::Text(text) if numeric => Err(DataError(format!(
            "{} is not a number, and this data is numeric",
            repr_str(text)
        ))),
        PointValue::Text(text) => Ok(text.clone()),
        PointValue::Number(number) => Ok(number_text(*number)),
        PointValue::Blank => Ok(String::new()),
    }
}

/// A number as ooxml-edit's `number_text` writes it: whole numbers below 10^15 as integers,
/// others as Python's `repr` of the float.
#[must_use]
pub fn number_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        // whole and below 10^15: exact as an integer
        #[allow(clippy::cast_possible_truncation)]
        let whole = number as i64;
        return whole.to_string();
    }
    ooxml_base::pyfmt::repr_float(number)
}

/// `element` with its text (lxml's `.text`: the text before its first child) set to `value`.
fn with_text(element: &Element, value: &str) -> Element {
    let mut nodes: Vec<Node> = element
        .children()
        .iter()
        .skip_while(|n| matches!(n, Node::Text(_)))
        .cloned()
        .collect();
    nodes.insert(0, Node::Text(Atom::from(value)));
    element.with_children(ooxml_xml::Children::from_vec(nodes))
}

/// A new element `local_name` in the namespace and prefix `like` is named with, its text
/// set to `value` (an empty text written `<c:v></c:v>`, as lxml writes one set to `""`).
fn text_element(like: &Element, local_name: &str, value: &str) -> Element {
    let text = vec![Node::Text(Atom::from(value))];
    Element::new(qualified(like, local_name), Vec::new(), Vec::new(), text)
}

fn qualified(like: &Element, local_name: &str) -> ooxml_xml::Name {
    Arc::new(QName::new(like.name().prefix(), local_name, like.name().namespace()))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod style_tests;

#[cfg(test)]
mod combine_tests;
