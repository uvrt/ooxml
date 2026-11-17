//! Owns: a series' data caches -- numbers with blanks kept as `None`, category strings from
//! any of the four spellings (`c:strRef`, `c:numRef`, `c:multiLvlStrRef`, the literals), a
//! cache's format code (with Office's `formatcode2` alternate), the label range of a
//! "Value From Cells" label, and a title's cached string -- as `chart/read.py` reads them.
//! `c:ptCount` is clamped: it is attacker-controlled.
//!
//! Must not: read the formula (`c:f`) or the workbook behind it.

use super::int_of;
use crate::model::MAX_CACHE_POINTS;
use ooxml_xml::view::{attr_num, child, children, text};
use ooxml_xml::Element;

/// A dense list of numbers, `None` for blanks.
type Numbers = Vec<Option<f64>>;

/// Each point's own format code.
type Codes = Vec<Option<String>>;

/// Python's `str.strip()`: Unicode white space and the four information separators.
pub(crate) fn py_strip(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// `int_attr(node, name)`.
fn int_attr(node: &Element, name: &str) -> Option<i64> {
    attr_num(node, name).map(int_of)
}

/// The populated cache under a data source: `c:{kind}Ref/c:{kind}Cache`, `c:{kind}Lit`, or
/// `c:{kind}Cache` itself -- the first holding points, else the first there.
fn cache<'a>(reference: Option<&'a Element>, kind: &str) -> Option<&'a Element> {
    let reference = reference?;
    let candidates = [
        child(reference, &format!("{kind}Ref")).and_then(|r| child(r, &format!("{kind}Cache"))),
        child(reference, &format!("{kind}Lit")),
        child(reference, &format!("{kind}Cache")),
    ];
    candidates
        .iter()
        .flatten()
        .find(|c| child(c, "pt").is_some())
        .or_else(|| candidates.iter().flatten().next())
        .copied()
}

/// How long a series is: the larger of the highest `c:pt` index and `c:ptCount`, clamped.
fn point_count<'a>(cache: &'a Element, points: Option<Vec<&'a Element>>) -> usize {
    let points = points.unwrap_or_else(|| children(cache, "pt").collect());
    let limit = i64::try_from(MAX_CACHE_POINTS).unwrap_or(i64::MAX);
    let present = points
        .iter()
        .filter_map(|p| int_attr(p, "idx"))
        .filter(|i| (0..limit).contains(i))
        .max()
        .map_or(0, |i| i + 1);
    let declared = child(cache, "ptCount").and_then(|c| int_attr(c, "val"));
    let count = match declared {
        Some(d) if (0..=limit).contains(&d) => d.max(present).min(limit),
        _ => present,
    };
    usize::try_from(count).unwrap_or(0)
}

/// The index of a point within `0..count`.
fn slot(point: &Element, count: usize) -> Option<usize> {
    let index = usize::try_from(int_attr(point, "idx")?).ok()?;
    (index < count).then_some(index)
}

/// A dense value list with `None` for blanks, and the cache's format code.
pub(super) fn numeric_points(reference: Option<&Element>) -> (Vec<Option<f64>>, Option<String>) {
    let Some(cache) = cache(reference, "num") else {
        return (Vec::new(), None);
    };
    let count = point_count(cache, None);
    let mut values = vec![None; count];
    for point in children(cache, "pt") {
        let Some(index) = slot(point, count) else { continue };
        let raw = child(point, "v").map(text).unwrap_or_default();
        let value = ooxml_xml::view::py_float(py_strip(&raw)).filter(|v| v.is_finite());
        if let (Some(value), Some(cell)) = (value, values.get_mut(index)) {
            *cell = Some(value);
        }
    }
    (values, cache_format_code(cache))
}

/// `.text` stripped, `None` where empty.
fn stripped_text(node: &Element) -> Option<String> {
    Some(py_strip(&text(node)).to_owned()).filter(|t| !t.is_empty())
}

/// A number cache's format code: `c:formatCode`; or, inside `mc:AlternateContent`, a
/// Choice's `formatcode2` before the Fallback's `c:formatCode`.
fn cache_format_code(cache: &Element) -> Option<String> {
    if let Some(node) = child(cache, "formatCode") {
        return stripped_text(node);
    }
    for alternate in children(cache, "AlternateContent") {
        for choice in children(alternate, "Choice") {
            if let Some(found) = child(choice, "formatcode2").and_then(stripped_text) {
                return Some(found);
            }
        }
        let fallback = child(alternate, "Fallback").and_then(|f| child(f, "formatCode"));
        if let Some(found) = fallback.and_then(stripped_text) {
            return Some(found);
        }
    }
    None
}

/// A multi-level cache's first level (`c:multiLvlStrRef/c:multiLvlStrCache` or
/// `c:multiLvlStrLit`), with its container.
fn multi_level(reference: &Element) -> Option<&Element> {
    child(reference, "multiLvlStrRef")
        .and_then(|r| child(r, "multiLvlStrCache"))
        .or_else(|| child(reference, "multiLvlStrLit"))
}

/// Category labels from whichever cache spelling the file uses; only a multi-level axis'
/// innermost level.
pub(super) fn string_points(reference: Option<&Element>) -> (Vec<String>, Option<String>) {
    let Some(reference) = reference else {
        return (Vec::new(), None);
    };
    let strings = cache(Some(reference), "str");
    if strings.is_none_or(|c| child(c, "pt").is_none()) {
        if let Some(multi) = multi_level(reference) {
            if let Some(level) = children(multi, "lvl").next() {
                let count = point_count(multi, Some(children(level, "pt").collect()));
                return (level_strings(level, count), None);
            }
        }
    }
    let numbers = || {
        let (values, code) = numeric_points(Some(reference));
        (values.into_iter().map(default_number_text).collect(), code)
    };
    match strings {
        None => numbers(),
        Some(c) if child(c, "pt").is_none() && cache(Some(reference), "num").is_some() => numbers(),
        Some(c) => (level_strings(c, point_count(c, None)), None),
    }
}

/// The numbers behind `c:cat` when they are numbers, and each point's own format code
/// (where any point has one); `(None, None)` for text categories.
pub(super) fn numeric_categories(reference: Option<&Element>) -> (Option<Numbers>, Option<Codes>) {
    let Some(reference) = reference else {
        return (None, None);
    };
    let Some(numbers) = cache(Some(reference), "num") else {
        return (None, None);
    };
    if cache(Some(reference), "str").is_some_and(|c| child(c, "pt").is_some()) {
        return (None, None);
    }
    if multi_level(reference).is_some_and(|m| child(m, "lvl").is_some()) {
        return (None, None);
    }
    let (values, _) = numeric_points(Some(reference));
    if values.is_empty() {
        return (None, None);
    }
    let mut codes: Vec<Option<String>> = vec![None; values.len()];
    for point in children(numbers, "pt") {
        let code = py_strip(point.attribute(None, "formatCode").unwrap_or("")).to_owned();
        if code.is_empty() {
            continue;
        }
        if let Some(cell) = slot(point, codes.len()).and_then(|i| codes.get_mut(i)) {
            *cell = Some(code);
        }
    }
    let any = codes.iter().any(Option::is_some);
    (Some(values), any.then_some(codes))
}

fn level_strings(container: &Element, count: usize) -> Vec<String> {
    let mut values = vec![String::new(); count];
    for point in children(container, "pt") {
        let Some(index) = slot(point, count) else { continue };
        if let Some(cell) = values.get_mut(index) {
            *cell = child(point, "v").map(Element::text_content).unwrap_or_default();
        }
    }
    values
}

/// A category number as text: a whole number without a point, else Python's
/// `repr(round(value, 10))`.
fn default_number_text(value: Option<f64>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if value == value.trunc() {
        let whole = format!("{value:.0}");
        return if whole == "-0" { "0".into() } else { whole };
    }
    crate::pyfmt::repr(crate::pyfmt::round(value, 10))
}

/// `c15:datalabelsRange`'s `c15:dlblRangeCache`: one string per point index.
pub(super) fn label_range(node: Option<&Element>) -> Option<Vec<Option<String>>> {
    let cache = child(node?, "dlblRangeCache")?;
    let points: Vec<&Element> = children(cache, "pt").collect();
    let count = point_count(cache, Some(points.clone())).min(MAX_CACHE_POINTS);
    let mut out: Vec<Option<String>> = vec![None; count];
    for point in points {
        let (Some(index), Some(value)) = (slot(point, count), child(point, "v")) else {
            continue;
        };
        if let Some(cell) = out.get_mut(index) {
            *cell = Some(value.text_content());
        }
    }
    Some(out)
}

/// A title's or name's cached string: the first `c:pt/c:v` of its string cache, else its
/// own `c:v`.
pub(super) fn cached_string(node: &Element) -> Option<String> {
    if let Some(cache) = cache(Some(node), "str") {
        if let Some(value) = children(cache, "pt").find_map(|p| child(p, "v")) {
            return Some(value.text_content());
        }
    }
    child(node, "v").map(Element::text_content)
}
