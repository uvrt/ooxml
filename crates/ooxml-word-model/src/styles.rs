//! Owns: the styles part as declared ([`StyleSheet`]): `w:docDefaults` and each style's own
//! properties (not what it inherits), table styles' regions included, read as docx2svg's
//! `parse_styles` reads them; and a style's `w:basedOn` chain ([`StyleSheet::chain`]).
//!
//! Must not: merge a chain into resolved properties (`crate::resolve` does).

use crate::props::{paragraph_properties, run_properties, table_properties};
use crate::theme::ThemeColours;
use crate::value::{text, Props, Text};
use crate::w;
use crate::xmlread::is_on;
use ooxml_xml::Element;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// A style kind (`w:type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StyleKind {
    /// `paragraph` (also what an unstated type means).
    Paragraph,
    /// `character`.
    Character,
    /// `table`.
    Table,
    /// `numbering`.
    Numbering,
    /// Anything else, as written.
    Other,
}

impl StyleKind {
    fn of(value: Option<&str>) -> (StyleKind, Text) {
        let written = value.filter(|v| !v.is_empty()).unwrap_or("paragraph");
        let kind = match written {
            "paragraph" => StyleKind::Paragraph,
            "character" => StyleKind::Character,
            "table" => StyleKind::Table,
            "numbering" => StyleKind::Numbering,
            _ => StyleKind::Other,
        };
        (kind, text(written))
    }
}

/// A table style's region (`w:tblStylePr`): what it declares for paragraphs, runs, the
/// table, rows and cells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Region {
    /// Its `w:pPr`.
    pub paragraph: Props,
    /// Its `w:rPr`.
    pub run: Props,
    /// Its `w:tblPr`.
    pub table: Props,
    /// Its `w:trPr`.
    pub row: Props,
    /// Its `w:tcPr`.
    pub cell: Props,
}

/// One `w:style`, unresolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    /// `w:styleId`.
    pub style_id: Text,
    /// `w:type`.
    pub kind: StyleKind,
    /// `w:type` as written (`paragraph` when unstated).
    pub kind_name: Text,
    /// `w:name`.
    pub name: Option<Text>,
    /// `w:basedOn`.
    pub based_on: Option<Text>,
    /// `w:next`.
    pub next_style: Option<Text>,
    /// `w:link`.
    pub link: Option<Text>,
    /// `w:default="1"`.
    pub default: bool,
    /// Its `w:pPr`.
    pub paragraph: Props,
    /// Its `w:rPr`.
    pub run: Props,
    /// A table style's regions by `w:type`, in document order (the last of a type wins).
    pub conditional: BTreeMap<Text, Region>,
    /// A table style's own `w:tblPr`, `w:trPr` and `w:tcPr`.
    pub table: Props,
    /// See `table`.
    pub table_row: Props,
    /// See `table`.
    pub table_cell: Props,
}

/// `word/styles.xml`: `w:docDefaults` and the styles by id (the first of an id).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleSheet {
    /// `w:rPrDefault/w:rPr`.
    pub run_defaults: Props,
    /// `w:pPrDefault/w:pPr`.
    pub paragraph_defaults: Props,
    /// The styles by id.
    pub styles: BTreeMap<Text, Arc<Style>>,
    /// The styles' ids in document order (first occurrences).
    pub order: Vec<Text>,
}

fn value(v: Option<w::Val<'_>>) -> Option<Text> {
    v.and_then(w::Val::val).map(text)
}

fn region(node: w::TblStylePr<'_>, colours: &ThemeColours) -> Region {
    Region {
        paragraph: paragraph_properties(node.paragraph().map(w::PPr::element), colours),
        run: run_properties(node.run().map(w::RPr::element), colours),
        table: table_properties(node.table().map(w::TblPr::element), colours),
        row: table_properties(node.row().map(w::TrPr::element), colours),
        cell: table_properties(node.cell().map(w::TcPr::element), colours),
    }
}

fn style(element: w::Style<'_>, style_id: &str, colours: &ThemeColours) -> Style {
    let (kind, kind_name) = StyleKind::of(element.kind());
    let mut conditional = BTreeMap::new();
    for node in element.conditional() {
        if let Some(kind) = node.kind().filter(|k| !k.is_empty()) {
            conditional.insert(text(kind), region(node, colours));
        }
    }
    Style {
        style_id: text(style_id),
        kind,
        kind_name,
        name: value(element.name()),
        based_on: value(element.based_on()),
        next_style: value(element.next()),
        link: value(element.link()),
        default: element.default().is_some_and(is_on),
        paragraph: paragraph_properties(element.paragraph().map(w::PPr::element), colours),
        run: run_properties(element.run().map(w::RPr::element), colours),
        conditional,
        table: table_properties(element.table().map(w::TblPr::element), colours),
        table_row: table_properties(element.row().map(w::TrPr::element), colours),
        table_cell: table_properties(element.cell().map(w::TcPr::element), colours),
    }
}

impl StyleSheet {
    /// The styles part's sheet.
    pub fn read(root: &Element, colours: &ThemeColours) -> StyleSheet {
        let styles = w::Styles::new(root);
        let defaults = styles.defaults();
        let mut sheet = StyleSheet {
            run_defaults: run_properties(
                defaults
                    .and_then(w::DocDefaults::run)
                    .and_then(w::RPrDefault::properties)
                    .map(w::RPr::element),
                colours,
            ),
            paragraph_defaults: paragraph_properties(
                defaults
                    .and_then(w::DocDefaults::paragraph)
                    .and_then(w::PPrDefault::properties)
                    .map(w::PPr::element),
                colours,
            ),
            ..StyleSheet::default()
        };
        for element in styles.styles() {
            let Some(id) = element.style_id() else { continue };
            if sheet.styles.contains_key(id) {
                continue;
            }
            sheet.order.push(text(id));
            sheet.styles.insert(text(id), Arc::new(style(element, id, colours)));
        }
        sheet
    }

    /// The style `id`, if there is one.
    pub fn get(&self, id: &str) -> Option<&Arc<Style>> {
        self.styles.get(id)
    }

    /// The default style of `kind` (`w:default="1"`): the first in document order.
    pub fn default_of(&self, kind: StyleKind) -> Option<&Arc<Style>> {
        self.order
            .iter()
            .filter_map(|id| self.styles.get(id))
            .find(|s| s.kind == kind && s.default)
    }

    /// The `w:basedOn` chain of `id`, root first, stopping at a cycle or at a style of
    /// another kind; empty for an id naming no style of `kind`. For `Character` the default
    /// character style is left out: Word applies nothing it declares.
    pub fn chain(&self, id: Option<&str>, kind: StyleKind) -> Vec<Arc<Style>> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut current = id.and_then(|i| self.styles.get(i)).filter(|s| s.kind == kind);
        while let Some(style) = current {
            if style.kind != kind || !seen.insert(style.style_id.clone()) {
                break;
            }
            chain.push(Arc::clone(style));
            current = style
                .based_on
                .as_deref()
                .filter(|b| !b.is_empty())
                .and_then(|b| self.styles.get(b));
        }
        chain.reverse();
        if kind == StyleKind::Character {
            chain.retain(|s| !s.default);
        }
        chain
    }
}
