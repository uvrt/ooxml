//! Owns: each series' references as a reader of the chart's workbook follows them
//! (ooxml-edit's `Chart.workbook_values`): for its name, categories and values, whether the
//! source has a reference at all, and that reference's formula (`c:f`) as written.
//!
//! Must not: parse a formula or read the workbook (the consumer does, with its own reader),
//! or read a source's cache.

use super::look::series;
use super::Ser;
use ooxml_xml::view::{child, text};
use ooxml_xml::Element;

/// One data source's reference, as a workbook reader sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// No reference: nothing to read.
    Absent,
    /// A reference without a formula (a literal, or a cache alone).
    Literal,
    /// A reference with a formula: its text as written (`""` where the `c:f` is empty).
    Formula(String),
}

impl Reference {
    fn of(source: &super::source::Source<'_>) -> Reference {
        match source.reference() {
            None => Reference::Absent,
            Some(reference) => child(reference, "f").map_or(Reference::Literal, |f| Reference::Formula(text(f))),
        }
    }

    /// The formula's text, where there is a formula and it says something.
    #[must_use]
    pub fn formula(&self) -> Option<&str> {
        match self {
            Reference::Formula(written) if !written.is_empty() => Some(written),
            _ => None,
        }
    }
}

/// A series' references: its name's (`c:tx`), its categories' (`c:cat` or `c:xVal`) and
/// its values' (`c:val` or `c:yVal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesReferences {
    /// The name's.
    pub name: Reference,
    /// The categories'.
    pub categories: Reference,
    /// The values'.
    pub values: Reference,
}

/// Every series' references, plot by plot in the plot area's order (the order
/// [`super::chart_data`] lists the series in).
#[must_use]
pub fn series_references(root: &Element) -> Vec<SeriesReferences> {
    series(root)
        .into_iter()
        .map(|(_, ser)| {
            let ser = Ser::of(ser);
            SeriesReferences {
                name: Reference::of(&ser.name),
                categories: Reference::of(&ser.categories),
                values: Reference::of(&ser.values),
            }
        })
        .collect()
}

/// Every `c:ser`, plot by plot in the plot area's order: what [`super::series_look`] and
/// [`super::point_labels`] read.
#[must_use]
pub fn series_elements(root: &Element) -> Vec<&Element> {
    series(root).into_iter().map(|(_, ser)| ser).collect()
}
