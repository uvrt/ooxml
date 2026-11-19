//! Owns: the ranges `word_format`'s `find` items name (docx-agent's `found_ranges`, round
//! 40): each item's text in its `within` (the body by default), once -- or its
//! `occurrence`, or each match with `expect` `all`; every one found before anything
//! changes. A text found nowhere is refused with the places it occurs written otherwise
//! (case, spaces) or what `within` holds; one found more than once with each match.
//!
//! Must not: change the document.

use super::format_args::{FindExpect, FindItem};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_word_edit::{occurrences, range_text, EditError, Occurrence, WordDoc};

/// The ranges, in the items' order.
///
/// # Errors
/// docx-agent's refusals.
pub(crate) fn found_ranges(doc: &WordDoc, items: &[FindItem], again: &TryCall) -> Result<Vec<String>, ToolError> {
    let mut out = Vec::new();
    for (k, item) in items.iter().enumerate() {
        let place = format!("find[{k}]");
        let within = item.within.as_deref().filter(|w| !w.is_empty());
        let expect = item.expect.unwrap_or(FindExpect::One);
        if item.occurrence.is_some() && expect == FindExpect::All {
            let message = format!("{place}: occurrence picks one match and expect all takes each: give one of them");
            let way = WayOut::names(["occurrence", "expect"]).unwrap_or_else(|| WayOut::call(again.clone()));
            return Err(ToolError::refusal(Refusal::InvalidArguments, message, way).at(format!("{place}.occurrence")));
        }
        let matches = found(doc, &item.text, within, true, again)?;
        let shown = within.map(|w| format!(" in {w}")).unwrap_or_default();
        if matches.is_empty() {
            return Err(not_there(doc, &item.text, within, &place, again)?);
        }
        let text = repr_str(&item.text);
        if let Some(occurrence) = item.occurrence {
            let picked = usize::try_from(occurrence)
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|n| matches.get(n));
            let Some(picked) = picked else {
                let message = format!(
                    "{place}: {text} occurs {} time(s){shown}; no occurrence {occurrence}",
                    matches.len()
                );
                return Err(refusal(Refusal::NotFound, message, listed(&matches), &place, again));
            };
            out.push(picked.id.clone());
        } else if expect == FindExpect::One && matches.len() > 1 {
            let message = format!(
                "{place}: {} places match {text}{shown}: give occurrence (1-{}, in this order), a narrower within, or expect all",
                matches.len(),
                matches.len()
            );
            return Err(refusal(Refusal::Ambiguous, message, listed(&matches), &place, again));
        } else {
            out.extend(matches.into_iter().map(|m| m.id));
        }
    }
    Ok(out)
}

fn refusal(code: Refusal, message: String, options: Vec<String>, place: &str, again: &TryCall) -> ToolError {
    let way = WayOut::names(options).unwrap_or_else(|| WayOut::call(again.clone()));
    ToolError::refusal(code, message, way).at(format!("{place}.occurrence"))
}

fn found(
    doc: &WordDoc,
    text: &str,
    within: Option<&str>,
    exact: bool,
    again: &TryCall,
) -> Result<Vec<Occurrence>, ToolError> {
    occurrences(doc, text, within, exact, !exact).map_err(|e| {
        let message = match e {
            EditError::Address(a) => a.message,
            other => other.to_string(),
        };
        let message = format!("{}: {message}", within.unwrap_or("None"));
        ToolError::refusal(Refusal::NotFound, message, WayOut::call(again.clone())).at("find")
    })
}

/// Each match as `occurrence n: <range> <context>`: the call can name it either way.
fn listed(matches: &[Occurrence]) -> Vec<String> {
    matches
        .iter()
        .take(50)
        .enumerate()
        .map(|(n, m)| format!("occurrence {}: {} {}", n + 1, m.id, repr_str(&m.context)))
        .collect()
}

/// The refusal of a text found nowhere (docx-agent's `_not_there`).
fn not_there(
    doc: &WordDoc,
    text: &str,
    within: Option<&str>,
    place: &str,
    again: &TryCall,
) -> Result<ToolError, ToolError> {
    let near = found(doc, text, within, false, again)?;
    let shown = within.map(|w| format!(" in {w}")).unwrap_or_default();
    let mut message = format!("{place}: {} occurs nowhere{shown}", repr_str(text));
    let mut options = Vec::new();
    if !near.is_empty() {
        message.push_str(&format!(
            "; written otherwise at {} place(s): give the text as written there",
            near.len()
        ));
        options = near
            .iter()
            .take(50)
            .map(|m| format!("{} {}", m.id, repr_str(&m.text)))
            .collect();
    } else if let Some(held) = within.and_then(|w| range_text(doc, w).ok()) {
        message.push_str(&format!("; it holds {}", repr_str(&short(&held, 160))));
    }
    let way = WayOut::names(options).unwrap_or_else(|| WayOut::call(again.clone()));
    Ok(ToolError::refusal(Refusal::NotFound, message, way).at(format!("{place}.text")))
}

/// docx-agent's `short`: white space as one space, at most `limit` characters.
fn short(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let mut out: String = text.chars().take(limit - 1).collect();
    out.push('…');
    out
}
