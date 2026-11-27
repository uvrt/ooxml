//! Owns: what a Word drawing's chart or SmartArt diagram holds, as docx-agent reads it for
//! `word_read` and `word_inspect` (`edit/charts.py`'s `chart_json`, `diagram_json`,
//! `chart_summary` and `diagram_summary`): the chart part a frame's first `c:chart` names,
//! or the data and layout parts its first `dgm:relIds` names, through the frame's part's
//! relationships; read into `ooxml_chart::data` or `ooxml_drawingml::diagram`; and each as
//! docx-agent's JSON and one-line summary.
//!
//! Must not: change a chart or diagram, or read a chart's embedded workbook or a diagram's
//! cached drawing; one whose part is missing or unreadable is none here (docx-agent's
//! `None`), never an error.

use crate::docx::drawings::{C, DGM};
use crate::docx::Docx;
use ooxml_chart::data::{chart_data, point_json, points_json, ChartData};
use ooxml_drawingml::diagram::{diagram_nodes, diagram_summary, DiagramNode};
use ooxml_xml::Element;
use serde_json::{Map, Value};
use std::sync::Arc;

/// Relationships named from a part, as docx-agent reads them (transitional only).
const NS_R: &str = crate::xmlread::Ns::R.uri();

/// What a chart or SmartArt drawing holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Graphic {
    /// A chart's data.
    Chart(Arc<ChartData>),
    /// A diagram's nodes.
    Diagram(Arc<DiagramData>),
}

/// A SmartArt diagram as docx-agent reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramData {
    /// Its layout part's `uniqueId`, where it has one.
    pub layout: Option<String>,
    /// Its nodes, depth first.
    pub nodes: Vec<DiagramNode>,
}

impl Graphic {
    /// What `frame` of the part `part` holds, where it is of `kind` (`chart`, `diagram`).
    #[must_use]
    pub fn of(docx: &Docx, part: &str, frame: &Element, kind: &str) -> Option<Graphic> {
        match kind {
            "chart" => chart_of(docx, part, frame).map(|c| Graphic::Chart(Arc::new(c))),
            "diagram" => diagram_of(docx, part, frame).map(|d| Graphic::Diagram(Arc::new(d))),
            _ => None,
        }
    }

    /// The key docx-agent gives its JSON under, and the word its summary names it by.
    #[must_use]
    pub fn names(&self) -> (&'static str, &'static str) {
        match self {
            Graphic::Chart(_) => ("chart", "chart"),
            Graphic::Diagram(_) => ("diagram", "smartart"),
        }
    }

    /// Its one-line summary.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Graphic::Chart(c) => c.summary(),
            Graphic::Diagram(d) => diagram_summary(&d.nodes),
        }
    }

    /// Its JSON.
    #[must_use]
    pub fn json(&self) -> Value {
        match self {
            Graphic::Chart(c) => chart_json(c),
            Graphic::Diagram(d) => diagram_json(d),
        }
    }
}

/// The part `part`'s relationship `id` names, where it is internal.
fn related(docx: &Docx, part: &str, id: Option<&str>) -> Option<String> {
    let id = id?;
    docx.relationships(part)
        .ok()?
        .get(id)
        .filter(|r| !r.external)?
        .target_part
        .clone()
}

/// The diagram `frame` of the part `part` holds: `None` where it names none, or its data
/// part is missing or unreadable.
#[must_use]
pub fn diagram_of(docx: &Docx, part: &str, frame: &Element) -> Option<DiagramData> {
    let node = crate::parts::descendants(frame, DGM, "relIds").into_iter().next()?;
    let data = related(docx, part, node.attribute(Some(NS_R), "dm"))?;
    let data = docx.package.tree(&data).ok()??;
    let layout = match related(docx, part, node.attribute(Some(NS_R), "lo")) {
        Some(name) => docx.package.tree(&name).ok()?,
        None => None,
    };
    let layout = layout.and_then(|t| t.root().attribute(None, "uniqueId").map(str::to_owned));
    Some(DiagramData {
        layout,
        nodes: diagram_nodes(data.root()).ok()?,
    })
}

/// A diagram's JSON as docx-agent gives it (ooxml-edit's `diagram_model`): `layout` where
/// there is one, then `nodes` (`id`, `lvl`, `t`).
#[must_use]
pub fn diagram_json(data: &DiagramData) -> Value {
    let mut out = Map::new();
    if let Some(layout) = &data.layout {
        out.insert("layout".into(), Value::from(layout.as_str()));
    }
    let nodes = data
        .nodes
        .iter()
        .map(|n| {
            let mut one = Map::new();
            one.insert("id".into(), n.id.as_deref().map_or(Value::Null, Value::from));
            one.insert("lvl".into(), Value::from(n.level));
            one.insert("t".into(), Value::from(n.text.as_str()));
            Value::Object(one)
        })
        .collect();
    out.insert("nodes".into(), Value::Array(nodes));
    Value::Object(out)
}

/// The chart part `frame` (a `wp:inline` or `wp:anchor`, or a group member) of the part
/// `part` names: its first `c:chart`'s internal target (ooxml-edit's `chart_part`).
#[must_use]
pub fn chart_part(docx: &Docx, part: &str, frame: &Element) -> Option<String> {
    let node = crate::parts::descendants(frame, C, "chart").into_iter().next()?;
    related(docx, part, node.attribute(Some(NS_R), "id"))
}

/// The chart `frame` (a `wp:inline` or `wp:anchor`, or a group member) of the part `part`
/// holds: `None` where it names none, or its part is missing or unreadable.
#[must_use]
pub fn chart_of(docx: &Docx, part: &str, frame: &Element) -> Option<ChartData> {
    let target = chart_part(docx, part, frame)?;
    let tree = docx.package.tree(&target).ok()??;
    chart_data(tree.root()).ok()
}

/// A chart's JSON as docx-agent gives it (ooxml-edit's `chart_model`): `types`, `title`,
/// `axis_titles`, `legend`, `format`, `categories`, `series` (`name`, `values`), and
/// `category_dates` and `category_format` where the categories are dates kept as serials.
#[must_use]
pub fn chart_json(data: &ChartData) -> Value {
    let mut out = Map::new();
    out.insert(
        "types".into(),
        data.types.iter().map(|t| Value::from(t.as_str())).collect(),
    );
    out.insert("title".into(), point_json(data.title.as_ref()));
    let axes: Map<String, Value> = data
        .axis_titles
        .iter()
        .map(|(axis, title)| ((*axis).to_owned(), point_json(title.as_ref())))
        .collect();
    out.insert("axis_titles".into(), Value::Object(axes));
    out.insert("legend".into(), Value::Bool(data.legend));
    out.insert("format".into(), data.format.as_deref().map_or(Value::Null, Value::from));
    out.insert("categories".into(), points_json(&data.categories));
    let series = data
        .series
        .iter()
        .map(|s| {
            let mut one = Map::new();
            one.insert("name".into(), point_json(s.name.as_ref()));
            one.insert("values".into(), points_json(&s.values));
            Value::Object(one)
        })
        .collect();
    out.insert("series".into(), Value::Array(series));
    if let Some(dates) = &data.category_dates {
        let dates = dates
            .iter()
            .map(|d| d.as_deref().map_or(Value::Null, Value::from))
            .collect();
        out.insert("category_dates".into(), Value::Array(dates));
        out.insert(
            "category_format".into(),
            data.category_format.as_deref().map_or(Value::Null, Value::from),
        );
    }
    Value::Object(out)
}
