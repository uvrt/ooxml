//! Owns: a series' own point labels as ooxml-edit's `point_labels` reads them
//! (`charts/styling.py`): one per point, `None` where a point has none; a label's text (its
//! runs up to a field, a trailing `", "` dropped), a Value From Cells field's text from the
//! series' label range cache (`c15:datalabelsRange`).
//!
//! Must not: change the part or read the workbook.

use super::source::{Source, Unreadable};
use ooxml_xml::view::{attr, child, children, local, py_int, text};
use ooxml_xml::{Element, Node};

/// The `c:ext` a series' Value From Cells range is in.
const LABEL_RANGE_EXT: &str = "{02D57815-91ED-43cb-92C2-25804820EDAC}";

/// Digits only, as Python's `str.isdigit` reads ASCII.
fn digits(text: &str) -> Option<usize> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The series' Value From Cells range (`_LabelRange`): the first extension holding one.
fn label_range(ser: &Element) -> Option<&Element> {
    let list = child(ser, "extLst")?;
    children(list, "ext").find_map(|ext| child(ext, "datalabelsRange"))
}

/// The label range's point count, as a source's is counted.
fn range_count(range: &Element) -> usize {
    let Some(cache) = child(range, "dlblRangeCache") else {
        return 0;
    };
    if let Some(count) = child(cache, "ptCount").and_then(|n| attr(n, "val")).and_then(digits) {
        return count;
    }
    let highest = children(cache, "pt")
        .filter_map(|p| attr(p, "idx").and_then(py_int))
        .max()
        .unwrap_or(-1);
    usize::try_from(highest.saturating_add(1)).unwrap_or(0)
}

/// The label range cache's texts (`_label_range_points`): empty where there is none.
fn cached(ser: &Element) -> Vec<Option<String>> {
    let Some(list) = child(ser, "extLst") else {
        return Vec::new();
    };
    let Some(ext) = children(list, "ext").find(|e| attr(e, "uri") == Some(LABEL_RANGE_EXT)) else {
        return Vec::new();
    };
    let Some(cache) = child(ext, "datalabelsRange").and_then(|r| child(r, "dlblRangeCache")) else {
        return Vec::new();
    };
    let size = child(cache, "ptCount")
        .and_then(|n| attr(n, "val"))
        .and_then(digits)
        .unwrap_or(0)
        .min(crate::model::MAX_CACHE_POINTS);
    let mut out = vec![None; size];
    for point in children(cache, "pt") {
        let Some(index) = attr(point, "idx").and_then(digits) else {
            continue;
        };
        if let Some(slot) = out.get_mut(index) {
            *slot = Some(child(point, "v").map(text).unwrap_or_default());
        }
    }
    out
}

/// Every `a:r` and `a:fld` below `node`, in document order.
fn runs_and_fields<'a>(node: &'a Element, out: &mut Vec<&'a Element>) {
    for item in node.children() {
        if let Node::Element(e) = item {
            if matches!(local(e), "r" | "fld") {
                out.push(e);
            }
            runs_and_fields(e, out);
        }
    }
}

/// A series' own point labels (`point_labels`): one per point, `None` where it has none.
///
/// # Errors
/// [`Unreadable`] where a source holds more points than the model reads.
pub fn point_labels(ser: &Element) -> Result<Vec<Option<String>>, Unreadable> {
    let either = |a: &str, b: &str| child(ser, a).or_else(|| child(ser, b));
    let mut count = 0;
    for source in [either("cat", "xVal"), either("val", "yVal"), child(ser, "bubbleSize")] {
        let source = Source::of(source);
        if source.is_some() {
            count = count.max(source.count()?);
        }
    }
    if let Some(range) = label_range(ser) {
        count = count.max(range_count(range));
    }
    if count > crate::model::MAX_CACHE_POINTS {
        return Err(Unreadable);
    }
    let mut out = vec![None; count];
    let cells = cached(ser);
    for label in child(ser, "dLbls").into_iter().flat_map(|h| children(h, "dLbl")) {
        let index = child(label, "idx").and_then(|i| attr(i, "val")).and_then(digits);
        let rich = child(label, "tx").and_then(|t| child(t, "rich"));
        let (Some(index), Some(rich)) = (index, rich) else {
            continue;
        };
        let Some(slot) = out.get_mut(index) else { continue };
        let mut found = Vec::new();
        runs_and_fields(rich, &mut found);
        let mut said = String::new();
        for node in found {
            if local(node) == "fld" {
                if attr(node, "type") == Some("CELLRANGE") {
                    said.push_str(cells.get(index).and_then(Option::as_deref).unwrap_or(""));
                }
                break; // a value field and what follows it are not the label
            }
            said.push_str(&child(node, "t").map(text).unwrap_or_default());
        }
        let said = said.strip_suffix(", ").map_or(said.clone(), str::to_owned);
        *slot = Some(said);
    }
    Ok(out)
}
