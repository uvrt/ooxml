//! Owns: a paragraph style's own tab stops (docx-agent's `Styles._tab_stops`,
//! `_column_end` and `write_tabs`, round 31): the stops given checked and turned into
//! `(position twips, w:val, w:leader)` -- a stop with no position at the text column's end
//! (the widest section's width between its margins), a clear of a stop the style does not
//! inherit said -- and written as Word writes them.
//!
//! Must not: read what a style inherits but through [`super::read::inherited_tabs`].

use super::sheet::make;
use crate::format::properties_of;
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::docx::Docx;

/// A tab stop's alignment as the tools say it, and its `w:val`.
pub const TAB_ALIGNMENTS: &[(&str, &str)] = &[
    ("left", "left"),
    ("center", "center"),
    ("right", "right"),
    ("decimal", "decimal"),
    ("bar", "bar"),
    ("clear", "clear"),
];
/// A tab stop's leader, and its `w:leader`.
pub const TAB_LEADERS: &[(&str, &str)] = &[
    ("none", "none"),
    ("dot", "dot"),
    ("hyphen", "hyphen"),
    ("underscore", "underscore"),
    ("heavy", "heavy"),
    ("middle_dot", "middleDot"),
];

/// One tab stop as `modify` takes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabStop {
    /// Points; `None`: the text column's end.
    pub position: Option<f64>,
    /// One of [`TAB_ALIGNMENTS`]' names (default left).
    pub alignment: Option<String>,
    /// One of [`TAB_LEADERS`]' names (default none).
    pub leader: Option<String>,
}

/// A stop as written: position (twips), `w:val`, `w:leader`.
pub(crate) type Stop = (i64, String, Option<String>);

fn refused(message: String) -> EditError {
    EditError::Refused(message)
}

fn names(table: &[(&str, &str)]) -> String {
    table.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}

/// Twips as points the way Python's `f"{twips / 20:g}"` writes them (a twentieth has at
/// most two decimals).
pub(crate) fn points(twips: i64) -> String {
    let text = format!("{:.2}", twips as f64 / 20.0);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// A section's width between its margins and gutter, twips (`Section.text_width`);
/// `None` where it states no page size.
fn text_width(properties: &ooxml_xml::Element) -> Option<i64> {
    let int = |node: Option<&ooxml_xml::Element>, local: &str| {
        node.and_then(|n| n.attribute(Some(W), local))
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(0)
    };
    let size = properties.find(Some(W), "pgSz")?;
    let margins = properties.find(Some(W), "pgMar").map(|m| &**m);
    Some(int(Some(size), "w") - int(margins, "left") - int(margins, "right") - int(margins, "gutter"))
}

/// The text column's end, twips, and a note where the sections' widths differ.
pub(crate) fn column_end(docx: &Docx) -> (i64, Option<String>) {
    let widths: Vec<i64> = ooxml_word_model::docx::sections::sections(docx)
        .iter()
        .filter_map(|s| text_width(&s.properties))
        .filter(|w| *w != 0)
        .collect();
    let Some(widest) = widths.iter().copied().max() else {
        return (9026, None);
    };
    let mut distinct = widths.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() == 1 {
        return (widest, None);
    }
    let listed: Vec<String> = distinct.iter().map(|w| points(*w)).collect();
    let note = format!(
        "the sections' text columns differ ({} pt): the stop is at the widest's end, {} pt; Word draws a right tab past a narrower column's margin at that margin",
        listed.join(", "),
        points(widest)
    );
    (widest, Some(note))
}

/// `tabs` checked as written stops, sorted, and the warnings (`_tab_stops`); `inherited`
/// the positions the style inherits, `end` the text column's end when a stop needs it.
///
/// # Errors
/// [`EditError::Refused`] for a stop given wrong.
pub(crate) fn stops(
    tabs: &[TabStop],
    inherited: &[i64],
    end: &mut dyn FnMut() -> (i64, Option<String>),
) -> Result<(Vec<Stop>, Vec<String>)> {
    let mut out: Vec<Stop> = Vec::new();
    let mut warnings = Vec::new();
    let mut column: Option<i64> = None;
    for stop in tabs {
        let alignment = stop.alignment.as_deref().unwrap_or("left");
        let Some((_, val)) = TAB_ALIGNMENTS.iter().find(|(n, _)| *n == alignment) else {
            return Err(refused(format!(
                "a tab stop's alignment is one of {}, not {}",
                names(TAB_ALIGNMENTS),
                repr_str(alignment)
            )));
        };
        let leader = match stop.leader.as_deref() {
            None => None,
            Some(given) => Some(TAB_LEADERS.iter().find(|(n, _)| *n == given).ok_or_else(|| {
                refused(format!(
                    "a tab stop's leader is one of {}, not {}",
                    names(TAB_LEADERS),
                    repr_str(given)
                ))
            })?),
        };
        if alignment == "clear" && leader.is_some_and(|(n, _)| *n != "none") {
            return Err(refused("a cleared tab stop has no leader".to_owned()));
        }
        let twips = match stop.position {
            None => {
                if alignment == "clear" {
                    return Err(refused(
                        "a tab stop to clear needs its position (the inherited stop's)".to_owned(),
                    ));
                }
                match column {
                    Some(at) => at,
                    None => {
                        let (at, note) = end();
                        warnings.extend(note);
                        column = Some(at);
                        at
                    }
                }
            }
            Some(position) if !(-1584.0..=1584.0).contains(&position) => {
                return Err(refused(format!(
                    "a tab stop's position is in points, -1584 to 1584, not {}",
                    position
                )));
            }
            Some(position) => (position * 20.0).round_ties_even() as i64,
        };
        if out.iter().any(|(t, _, _)| *t == twips) {
            return Err(refused(format!(
                "two tab stops at {} pt: give each position once",
                points(twips)
            )));
        }
        if alignment == "clear" && !inherited.contains(&twips) {
            let mut held: Vec<i64> = inherited.to_vec();
            held.sort_unstable();
            let held: Vec<String> = held.iter().map(|t| points(*t)).collect();
            let held = if held.is_empty() {
                "none".to_owned()
            } else {
                held.join(", ")
            };
            warnings.push(format!(
                "the style inherits no tab stop at {} pt to clear (it inherits: {held})",
                points(twips)
            ));
        }
        let leader = leader.filter(|(n, _)| *n != "none").map(|(_, l)| (*l).to_owned());
        out.push((twips, (*val).to_owned(), leader));
    }
    out.sort();
    Ok((out, warnings))
}

/// A style's own `w:tabs` replaced by `stops` (`write_tabs`); none removes it.
pub(crate) fn write(work: &mut Work, node: Id, stops: &[Stop], orders: &Orders) {
    let properties = work.find(node, W, "pPr");
    if let Some(old) = properties.and_then(|p| work.find(p, W, "tabs")) {
        work.remove(old);
    }
    if stops.is_empty() {
        return;
    }
    let Some(properties) = properties_of(work, node, "pPr", true, orders) else {
        return;
    };
    let tabs = make(work, "tabs", &[]);
    crate::marks::insert_in_order(work, properties, tabs, orders);
    for (position, value, leader) in stops {
        let tab = make(work, "tab", &[("val", value)]);
        if let Some(leader) = leader {
            crate::review::xml::set_attr(work, tab, "leader", leader);
        }
        crate::review::xml::set_attr(work, tab, "pos", &position.to_string());
        work.append(tabs, tab);
    }
}

/// The positions (twips) of the tab stops style `id` inherits through `basedOn` (not its
/// own), each base's stops over its base's and its clears taking one away
/// (`Styles.inherited_tabs`).
pub(crate) fn inherited(work: &Work, root: Id, id: &str) -> Vec<i64> {
    use super::sheet::{by_id, child_val};
    let mut chain = Vec::new();
    let mut seen = vec![id.to_owned()];
    let mut based = by_id(work, root, id)
        .and_then(|n| child_val(work, n, "basedOn"))
        .map(str::to_owned);
    while let Some(next) = based.take().filter(|b| !seen.contains(b)) {
        let Some(current) = by_id(work, root, &next) else { break };
        seen.push(next);
        chain.push(current);
        based = child_val(work, current, "basedOn").map(str::to_owned);
    }
    let mut out: Vec<i64> = Vec::new();
    for style in chain.iter().rev() {
        let tabs = work.find(*style, W, "pPr").and_then(|p| work.find(p, W, "tabs"));
        for tab in tabs.map(|t| work.children(t).to_vec()).unwrap_or_default() {
            if !work.is(tab, W, "tab") {
                continue;
            }
            let Some(position) = work
                .get(tab, Some(W), "pos")
                .and_then(ooxml_word_model::xmlread::py_int)
            else {
                continue;
            };
            out.retain(|p| *p != position);
            if work.get(tab, Some(W), "val") != Some("clear") {
                out.push(position);
            }
        }
    }
    out
}
