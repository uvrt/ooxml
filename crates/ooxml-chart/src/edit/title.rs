//! Owns: the chart's title and its axes' titles changed as ooxml-edit's `Chart` changes them
//! (`set_title`, `set_axis_title`, `_write_title`): a title made in order (with its
//! `c:overlay`) or taken away, its text written into its rich text with the formatting kept
//! (`rich`), a title drawn from a cell given the text in its cache (the caller writes the
//! cell), a new title's text in the application's title style (`title_text`), and the
//! chart's `c:autoTitleDeleted` kept saying whether a title was taken away.
//!
//! Must not: write the workbook, or choose the title sizes or language (the caller gives
//! the application's look).

use super::axes::axis_order;
use super::make::Make;
use super::owned::{Order, Owned};
use super::points::{insert_point, point_index};
use super::rich::{paragraph_text, replace_body_text};
use super::{encode, ChartTree, DataError, PointValue, SourceRef};
use crate::data::{Point, Source};
use ooxml_schema::orders::local as schema;
use ooxml_xml::view::{attr, child, local};
use ooxml_xml::Element;
use std::sync::Arc;

/// The axis elements (`AXIS_TAGS`).
const AXES: [&str; 4] = ["catAx", "valAx", "dateAx", "serAx"];

/// How new title text looks: the application's sizes (hundredths of a point) and the
/// language.
#[derive(Debug, Clone, Copy)]
pub struct TitleLook<'a> {
    /// A chart title's size (`look.title`).
    pub title: u32,
    /// An axis title's size (`look.axis_title`).
    pub axis_title: u32,
    /// The language of new text; `None` where the caller does not know it.
    pub lang: Option<&'a str>,
}

/// What a title edit did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TitleEdit {
    /// Whether the part changed.
    pub changed: bool,
    /// A title drawn from a cell: its reference, whose cell the caller writes the text into.
    pub written: Option<SourceRef>,
}

/// A title edit that needs the language of new text, which the caller did not give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeedsLanguage;

/// `_title_text`: a title's text (`None`: no title, or one without text of its own).
fn title_text(title: Option<&Element>) -> Option<String> {
    let tx = child(title?, "tx")?;
    if let Some(rich) = child(tx, "rich") {
        let paragraphs: Vec<String> = rich
            .child_elements()
            .filter(|e| local(e) == "p")
            .map(|p| paragraph_text(p))
            .collect();
        return Some(paragraphs.join("\n"));
    }
    let points = Source::of(Some(tx)).points().ok()?;
    match points.into_iter().next()? {
        Some(Point::Text(text)) => Some(text),
        Some(other) => other.number().map(crate::edit::number_text),
        None => None,
    }
}

/// `_ax_pos`.
fn axis_position(axis: &Element) -> Option<&str> {
    child(axis, "axPos").and_then(|p| attr(p, "val"))
}

/// Whether writing `text` into `owner`'s title makes new title text (`_write_title`: no
/// title, no `c:tx`, or a `c:tx` neither rich nor drawn from a cell).
fn makes_text(owner: &Element) -> bool {
    let Some(title) = child(owner, "title") else {
        return true;
    };
    let Some(tx) = child(title, "tx") else { return true };
    child(tx, "rich").is_none() && child(tx, "strRef").is_none()
}

impl ChartTree {
    /// The chart title's text (`title`).
    #[must_use]
    pub fn title(&self) -> Option<String> {
        title_text(child(&self.root, "chart").and_then(|c| child(c, "title")))
    }

    /// The place of the `"category"` or `"value"` axis among the plot area's children
    /// (`_axis_of`), with the places of `c:chart` and `c:plotArea`.
    fn axis_path(&self, axis: &str) -> Option<Vec<usize>> {
        let (chart_at, chart) = super::path::indexed(&self.root).find(|(_, e)| local(e) == "chart")?;
        let (area_at, area) = super::path::indexed(chart).find(|(_, e)| local(e) == "plotArea")?;
        let axes: Vec<(usize, &Element)> = super::path::indexed(area)
            .filter(|(_, e)| AXES.contains(&local(e)))
            .collect();
        let mut categorical: Vec<usize> = axes
            .iter()
            .filter(|(_, e)| matches!(local(e), "catAx" | "dateAx"))
            .map(|(i, _)| *i)
            .collect();
        let mut values: Vec<usize> = axes
            .iter()
            .filter(|(_, e)| local(e) == "valAx")
            .map(|(i, _)| *i)
            .collect();
        if categorical.is_empty() && values.len() >= 2 {
            let horizontal = axes
                .iter()
                .find(|(_, e)| local(e) == "valAx" && matches!(axis_position(e), Some("b" | "t")))
                .map(|(i, _)| *i);
            let first = horizontal.or_else(|| values.first().copied())?;
            categorical = vec![first];
            values.retain(|v| *v != first);
        }
        let chosen = if axis == "category" { categorical } else { values };
        chosen.first().map(|&at| vec![chart_at, area_at, at])
    }

    /// The `"category"` or `"value"` axis' title (`axis_title`): `None` where the chart has
    /// no such axis.
    #[must_use]
    pub fn axis_title(&self, axis: &str) -> Option<Option<String>> {
        let path = self.axis_path(axis)?;
        let element = super::path::at(&self.root, &path)?;
        Some(title_text(child(element, "title")))
    }

    /// `set_title`: the chart's title given `text` (`None`: taken away, and no automatic one
    /// shown).
    ///
    /// # Errors
    /// [`NeedsLanguage`] where new title text is made and `look` gives no language.
    pub fn set_title(
        &mut self,
        text: Option<&str>,
        look: &TitleLook<'_>,
    ) -> Result<Result<TitleEdit, DataError>, NeedsLanguage> {
        let chart = child(&self.root, "chart");
        let has_title = chart.and_then(|c| child(c, "title")).is_some();
        if text.map(str::to_owned) == self.title() && (text.is_some() || !has_title) {
            return Ok(Ok(TitleEdit::default()));
        }
        if text.is_some() && look.lang.is_none() && chart.is_some_and(makes_text) {
            return Err(NeedsLanguage);
        }
        let make = Make::of(&self.root);
        Ok(self.change_root(|root| {
            let Some(chart) = root.child("chart") else {
                return Ok(TitleEdit::default());
            };
            let written = write_title(chart, &make, schema::CHART, text, false, look.title, look.lang)?;
            let c = Arc::clone(chart.name());
            let wanted = if text.is_none() { "1" } else { "0" };
            chart
                .sub(&c, "autoTitleDeleted", Some(schema::CHART))
                .set_attr("val", wanted);
            Ok(TitleEdit { changed: true, written })
        }))
    }

    /// `set_axis_title`: the `"category"` or `"value"` axis' title given `text` (`None`:
    /// taken away); `None` where the chart has no such axis.
    ///
    /// # Errors
    /// [`NeedsLanguage`] where new title text is made and `look` gives no language.
    pub fn set_axis_title(
        &mut self,
        axis: &str,
        text: Option<&str>,
        look: &TitleLook<'_>,
    ) -> Result<Option<Result<TitleEdit, DataError>>, NeedsLanguage> {
        let Some(current) = self.axis_title(axis) else {
            return Ok(None);
        };
        if current.as_deref() == text {
            return Ok(Some(Ok(TitleEdit::default())));
        }
        let Some(path) = self.axis_path(axis) else {
            return Ok(None);
        };
        let element = super::path::at(&self.root, &path);
        if text.is_some() && look.lang.is_none() && element.is_some_and(makes_text) {
            return Err(NeedsLanguage);
        }
        let vertical = element.and_then(axis_position).is_some_and(|p| matches!(p, "l" | "r"));
        let make = Make::of(&self.root);
        Ok(Some(self.change_root(|root| {
            let Some(owner) = super::points::open_at(root, &path) else {
                return Ok(TitleEdit::default());
            };
            let order = axis_order(owner.local());
            let written = write_title(owner, &make, order, text, vertical, look.axis_title, look.lang)?;
            Ok(TitleEdit { changed: true, written })
        })))
    }
}

/// `_write_title`: `owner`'s `c:title` set, made or taken away; a title drawn from a cell
/// returns its reference (its cache written here, the cell the caller's).
fn write_title(
    owner: &mut Owned,
    make: &Make,
    order: Order,
    text: Option<&str>,
    vertical: bool,
    size: u32,
    lang: Option<&str>,
) -> Result<Option<SourceRef>, DataError> {
    let Some(text) = text else {
        if let Some(at) = owner.place("title") {
            owner.remove(at);
        }
        return Ok(None);
    };
    if owner.place("title").is_none() {
        let mut title = make.c("title");
        title.insert_ordered(Some(schema::TITLE), make.c("overlay").with_attr("val", "0"));
        owner.insert_ordered(Some(order), title);
    }
    let Some(title) = owner.child("title") else {
        return Ok(None);
    };
    if let Some(at) = title.place("tx") {
        let tx = title.read(at);
        let rich = tx.as_deref().and_then(|t| child(t, "rich")).is_some();
        if !rich {
            let reference = tx.as_deref().and_then(|t| child(t, "strRef")).map(|r| SourceRef {
                kind: "strRef".to_owned(),
                formula: child(r, "f").map(ooxml_xml::view::text),
            });
            if let Some(reference) = reference {
                if let Some(tx) = title.at(at) {
                    set_cached_text(tx, text)?;
                }
                return Ok(Some(reference));
            }
            title.remove(at);
        }
    }
    if title.place("tx").is_none() {
        title.insert_ordered(Some(schema::TITLE), new_title_text(make, size, vertical, lang));
    }
    if let Some(tx) = title.child("tx") {
        if let Some(at) = tx.place("rich") {
            if let Some(rich) = tx.read(at) {
                let written = replace_body_text(&rich, text);
                tx.remove(at);
                tx.insert(at, Owned::open(&written));
            }
        }
    }
    Ok(None)
}

/// `_Data.set_point(0, text)` on a `c:tx` drawn from a cell: its `c:strRef`'s cache given
/// the text.
fn set_cached_text(tx: &mut Owned, text: &str) -> Result<(), DataError> {
    let no_cache = || DataError("this data has no cache to write".to_owned());
    let reference = tx.child("strRef").ok_or_else(no_cache)?;
    let c = Arc::clone(reference.name());
    let cache = reference.child("strCache").ok_or_else(no_cache)?;
    let written = encode(&PointValue::Text(text.to_owned()), false)?;
    let first = cache.places("pt").into_iter().find(|&at| {
        cache
            .read(at)
            .is_some_and(|pt| point_index(pt.attribute(None, "idx")) == 0)
    });
    match first {
        Some(at) => {
            if let Some(pt) = cache.at(at) {
                let v = pt.sub(&c, "v", None);
                if v.text() != written {
                    v.set_text(&written);
                }
            }
        }
        None => insert_point(cache, &c, Some(schema::CACHE), 0, &written),
    }
    Ok(())
}

/// `title_text`: the empty `c:tx` of a title in Office's default title style, `size`
/// hundredths of a point, rotated when `vertical`, `lang` on the paragraph's end.
fn new_title_text(make: &Make, size: u32, vertical: bool, lang: Option<&str>) -> Owned {
    let rotation = if vertical { "-5400000" } else { "0" };
    let body = make
        .a("bodyPr")
        .with_attr("rot", rotation)
        .with_attr("spcFirstLastPara", "1")
        .with_attr("vertOverflow", "ellipsis")
        .with_attr("vert", "horz")
        .with_attr("wrap", "square")
        .with_attr("anchor", "ctr")
        .with_attr("anchorCtr", "1");
    let shade = 65_000;
    let colour = make
        .a("schemeClr")
        .with_attr("val", "tx1")
        .with(make.a("lumMod").with_attr("val", &shade.to_string()))
        .with(make.a("lumOff").with_attr("val", &(100_000 - shade).to_string()));
    let mut defaults = make.a("defRPr");
    for (name, value) in [
        ("sz", size.to_string().as_str()),
        ("b", "0"),
        ("i", "0"),
        ("u", "none"),
        ("strike", "noStrike"),
        ("kern", "1200"),
        ("spc", "0"),
        ("baseline", "0"),
    ] {
        defaults.set_attr(name, value);
    }
    let defaults = defaults
        .with(make.a("solidFill").with(colour))
        .with(make.a("latin").with_attr("typeface", "+mn-lt"))
        .with(make.a("ea").with_attr("typeface", "+mn-ea"))
        .with(make.a("cs").with_attr("typeface", "+mn-cs"));
    let mut end = make.a("endParaRPr");
    if let Some(lang) = lang.filter(|l| !l.is_empty()) {
        end.set_attr("lang", lang);
    }
    let paragraph = make.a("p").with(make.a("pPr").with(defaults)).with(end);
    let rich = make.c("rich").with(body).with(make.a("lstStyle")).with(paragraph);
    make.top(make.c("tx").with(rich))
}
