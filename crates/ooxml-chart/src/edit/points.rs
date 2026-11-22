//! Owns: a series' data sources changed in shape, as ooxml-edit's `_Data` changes them
//! (`charts/chart.py`): where each source's reference is (`c:tx`, `c:cat`/`c:xVal`,
//! `c:val`/`c:yVal`, `c:bubbleSize`, and a series' Value From Cells labels,
//! `c15:datalabelsRange`), how many points it holds (`count`), a point put in or taken out
//! with the later points renumbered and `c:ptCount` kept (`insert`, `delete`), per-point
//! formats and labels kept on their points (`_shift_point_formats`), a formula given a new
//! range (`set_range`), and a reference turned into a literal (`_to_literal`).
//!
//! Must not: write the workbook, or decide which sources an edit changes (the caller does,
//! with `ooxml-sheets`).

use super::owned::{Order, Owned};
use super::{encode, DataError, PointValue, Which, CACHE_ORDER};
use ooxml_xml::view::py_int;

/// The namespace of a series' Value From Cells labels (`_LabelRange.C15`).
pub(crate) const C15: &str = "http://schemas.microsoft.com/office/drawing/2012/chart";

/// `c:lvl`'s sequence.
const LEVEL_ORDER: Order = &[&["pt"], &["extLst"]];
/// `c:multiLvlStrCache`'s sequence.
const LEVELS_ORDER: Order = &[&["ptCount"], &["lvl"], &["extLst"]];

/// The references a data source may hold (`_REF_TAGS`).
const REFERENCES: [&str; 6] = ["strRef", "numRef", "multiLvlStrRef", "strLit", "numLit", "v"];

/// The places, from the `c:ser`, down to `which`'s reference; `None` where it has none.
pub(crate) fn reference_path(ser: &Owned, which: Which) -> Option<Vec<usize>> {
    if which == Which::Labels {
        let list = ser.place("extLst")?;
        let extensions = ser.read(list)?;
        for (ext_at, ext) in places(&extensions, "ext") {
            let found = places(ext, "datalabelsRange").find(|(_, e)| e.name().namespace() == Some(C15));
            if let Some((at, _)) = found {
                return Some(vec![list, ext_at, at]);
            }
        }
        return None;
    }
    let names: &[&str] = match which {
        Which::Name => &["tx"],
        Which::Categories => &["cat", "xVal"],
        Which::Values => &["val", "yVal"],
        Which::Sizes => &["bubbleSize"],
        Which::Labels => &[],
    };
    let container = names.iter().find_map(|n| ser.place(n))?;
    let element = ser.read(container)?;
    let (at, _) = element
        .children()
        .iter()
        .enumerate()
        .find(|(_, n)| n.as_element().is_some_and(|e| REFERENCES.contains(&e.name().local())))?;
    Some(vec![container, at])
}

/// The element children of `element` named `name`, with their places.
fn places<'a>(
    element: &'a ooxml_xml::Element,
    name: &'a str,
) -> impl Iterator<Item = (usize, &'a ooxml_xml::Element)> + 'a {
    element
        .children()
        .iter()
        .enumerate()
        .filter_map(move |(i, n)| n.as_element().filter(|e| e.name().local() == name).map(|e| (i, &**e)))
}

/// The element at `path` below `owned`, opened.
pub(crate) fn open_at<'a>(owned: &'a mut Owned, path: &[usize]) -> Option<&'a mut Owned> {
    let mut at = owned;
    for &place in path {
        at = at.at(place)?;
    }
    Some(at)
}

/// A source's reference, opened: its kind (`labelRange` for Value From Cells labels).
pub(crate) struct Reference<'a> {
    pub(crate) kind: String,
    /// A name in the chart namespace, which points and counts are made in.
    c: ooxml_xml::Name,
    pub(crate) element: &'a mut Owned,
}

impl Reference<'_> {
    fn numeric(&self) -> bool {
        matches!(self.kind.as_str(), "numRef" | "numLit")
    }

    /// The cache's local name (`_CACHE_OF`), or `None` for a literal (its own points) and a
    /// `c:v` (none).
    fn cache_name(&self) -> Option<&'static str> {
        match self.kind.as_str() {
            "strRef" => Some("strCache"),
            "numRef" => Some("numCache"),
            "multiLvlStrRef" => Some("multiLvlStrCache"),
            "labelRange" => Some("dlblRangeCache"),
            _ => None,
        }
    }

    fn has_cache(&self) -> bool {
        match self.cache_name() {
            Some(name) => self.element.place(name).is_some(),
            None => matches!(self.kind.as_str(), "strLit" | "numLit"),
        }
    }

    /// The cache, opened (`cache`).
    pub(crate) fn cache(&mut self) -> Option<&mut Owned> {
        match self.cache_name() {
            Some(name) => self.element.child(name),
            None if matches!(self.kind.as_str(), "strLit" | "numLit") => Some(&mut *self.element),
            None => None,
        }
    }

    /// How many points the source holds (`count`): `c:ptCount`'s where it is a number, else
    /// one past the highest point.
    pub(crate) fn count(&mut self) -> usize {
        if self.kind == "v" {
            return 1;
        }
        let multi = self.kind == "multiLvlStrRef";
        let Some(cache) = self.cache() else { return 0 };
        if let Some(place) = cache.place("ptCount") {
            let given = cache
                .read(place)
                .and_then(|e| e.attribute(None, "val").map(str::to_owned));
            if let Some(given) = given.filter(|g| !g.is_empty() && g.chars().all(|c| c.is_ascii_digit())) {
                return given.parse().unwrap_or(usize::MAX);
            }
        }
        let parent = if multi {
            cache.place("lvl").and_then(|at| cache.read(at))
        } else {
            Some(std::sync::Arc::new(cache.clone().close()))
        };
        parent.map_or(0, |p| {
            places(&p, "pt")
                .map(|(_, pt)| point_index(pt.attribute(None, "idx")))
                .max()
                .map_or(0, |highest| usize::try_from(highest + 1).unwrap_or(0))
        })
    }

    /// The element the points are in (`_writable`), opened, and its sequence.
    fn writable(&mut self) -> Result<(&mut Owned, Option<Order>), DataError> {
        let no_cache = || DataError("this data has no cache to write".to_owned());
        if self.kind == "multiLvlStrRef" && self.has_cache() {
            let cache = self.cache().ok_or_else(no_cache)?;
            if cache.places("lvl").len() != 1 {
                return Err(DataError(
                    "multi-level categories are not editable; edit them in the chart's workbook".to_owned(),
                ));
            }
            let level = cache.child("lvl").ok_or_else(no_cache)?;
            return Ok((level, Some(LEVEL_ORDER)));
        }
        let order = self.cache_order();
        let cache = self.cache().ok_or_else(no_cache)?;
        Ok((cache, order))
    }

    fn cache_order(&self) -> Option<Order> {
        match self.kind.as_str() {
            "numRef" | "numLit" => Some(&CACHE_ORDER),
            "strRef" | "strLit" => Some(&CACHE_ORDER),
            "multiLvlStrRef" => Some(LEVELS_ORDER),
            _ => None, // c15:dlblRangeCache: no sequence, appended
        }
    }

    /// `_set_count`: the cache's `c:ptCount` made where there is none.
    fn set_count(&mut self, count: usize) {
        let order = self.cache_order();
        let c = std::sync::Arc::clone(&self.c);
        if let Some(cache) = self.cache() {
            cache.sub(&c, "ptCount", order).set_attr("val", &count.to_string());
        }
    }

    /// A new point at `index` (`insert`): later points move up one.
    pub(crate) fn insert(&mut self, index: usize, value: Option<&PointValue>) -> Result<(), DataError> {
        let numeric = self.numeric();
        let (parent, _) = self.writable()?;
        renumber(parent, |at| (at >= index as i64).then_some(at + 1));
        let encoded = value.map(|v| encode(v, numeric)).transpose()?;
        let count = self.count() + 1;
        self.set_count(count);
        if let Some(text) = encoded {
            let c = std::sync::Arc::clone(&self.c);
            let (parent, order) = self.writable()?;
            insert_point(parent, &c, order, index, &text);
        }
        Ok(())
    }

    /// Point `index` taken out (`delete`): later points move down one.
    pub(crate) fn delete(&mut self, index: usize) -> Result<(), DataError> {
        let (parent, _) = self.writable()?;
        let doomed: Vec<usize> = parent
            .places("pt")
            .into_iter()
            .filter(|&at| {
                parent
                    .read(at)
                    .is_some_and(|pt| point_index(pt.attribute(None, "idx")) == index as i64)
            })
            .collect();
        for at in doomed.into_iter().rev() {
            parent.remove(at);
        }
        renumber(parent, |at| (at > index as i64).then_some(at - 1));
        let count = self.count().saturating_sub(1);
        self.set_count(count);
        Ok(())
    }

    /// Its formula's text (`c:f`, `c15:f` for labels).
    pub(crate) fn formula(&self) -> Option<String> {
        let at = self.element.place("f")?;
        self.element.read(at).map(|f| ooxml_xml::view::text(&f))
    }

    /// Give the formula the text `formula` (`set_range`).
    pub(crate) fn set_formula(&mut self, formula: &str) {
        if let Some(node) = self.element.child("f") {
            if node.text() != formula {
                node.set_text(formula);
            }
        }
    }
}

/// `which`'s reference of the `c:ser` `ser`, opened.
pub(crate) fn reference(ser: &mut Owned, which: Which) -> Option<Reference<'_>> {
    let path = reference_path(ser, which)?;
    let c = std::sync::Arc::clone(ser.name());
    let element = open_at(ser, &path)?;
    let kind = if which == Which::Labels {
        "labelRange".to_owned()
    } else {
        element.local().to_owned()
    };
    Some(Reference { kind, c, element })
}

/// Each point's `idx` changed by `change` where it gives a new one.
fn renumber(parent: &mut Owned, change: impl Fn(i64) -> Option<i64>) {
    for at in parent.places("pt") {
        let Some(pt) = parent.at(at) else { continue };
        if let Some(new) = change(point_index(pt.attr("idx"))) {
            pt.set_attr("idx", &new.to_string());
        }
    }
}

/// `_insert_pt`: `c:pt idx=` with its `c:v`, before the first point after it, else where the
/// cache's sequence puts it.
pub(crate) fn insert_point(parent: &mut Owned, c: &ooxml_xml::Name, order: Option<Order>, index: usize, text: &str) {
    let name = std::sync::Arc::clone(c);
    let pt = Owned::like(&name, "pt")
        .with_attr("idx", &index.to_string())
        .with(Owned::like(&name, "v").with_text(text));
    let later = parent.places("pt").into_iter().find(|&at| {
        parent
            .read(at)
            .is_some_and(|p| point_index(p.attribute(None, "idx")) > index as i64)
    });
    match later {
        Some(at) => parent.insert(at, pt),
        None => {
            parent.insert_ordered(order, pt);
        }
    }
}

/// A point's `idx` as `_idx` reads it (`-1` where it is not a number).
pub(crate) fn point_index(given: Option<&str>) -> i64 {
    given.map_or(Some(-1), py_int).unwrap_or(-1)
}

/// `_shift_point_formats`: the series' `c:dPt` and its labels' `c:dLbl` kept on their
/// points as point `index` is put in (`delta` 1) or taken out (-1, its own removed).
pub(crate) fn shift_point_formats(ser: &mut Owned, index: usize, delta: i64) {
    let index = index as i64;
    let mut holders: Vec<Vec<usize>> = ser.places("dPt").into_iter().map(|at| vec![at]).collect();
    if let Some(labels) = ser.place("dLbls") {
        if let Some(element) = ser.read(labels) {
            holders.extend(places(&element, "dLbl").map(|(at, _)| vec![labels, at]));
        }
    }
    // taken out last to first, so the places found stay true
    for path in holders.into_iter().rev() {
        let Some(holder) = open_at(ser, &path) else { continue };
        let Some(node) = holder.child("idx") else { continue };
        let Some(position) = node.attr("val").and_then(py_int) else {
            continue;
        };
        if delta < 0 && position == index {
            let (last, parent_path) = path.split_last().unwrap_or((&0, &[]));
            if let Some(parent) = open_at(ser, parent_path) {
                parent.remove(*last);
            }
        } else if if delta > 0 { position >= index } else { position > index } {
            node.set_attr("val", &(position + delta).to_string());
        }
    }
}

/// `_to_literal`: a formula-backed source (`c:strRef`, `c:numRef`) turned into a literal
/// holding its cache's children.
pub(crate) fn to_literal(ser: &mut Owned, which: Which) {
    let Some(path) = reference_path(ser, which) else { return };
    let (last, parent_path) = path.split_last().unwrap_or((&0, &[]));
    let Some(parent) = open_at(ser, parent_path) else {
        return;
    };
    let Some(reference) = parent.read(*last) else { return };
    let (literal, cache) = match reference.name().local() {
        "strRef" => ("strLit", "strCache"),
        "numRef" => ("numLit", "numCache"),
        _ => return,
    };
    let mut made = Owned::like(reference.name(), literal);
    if let Some((_, cache)) = places(&reference, cache).next() {
        made.take_elements(&mut Owned::open(cache));
    }
    parent.remove(*last);
    parent.insert(*last, made);
}
