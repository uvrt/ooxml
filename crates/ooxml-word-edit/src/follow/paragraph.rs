//! Owns: a paragraph made to follow its sample paragraph (docx-agent's
//! `follow._follow_paragraph` and `_follow_runs`): its properties but its list membership,
//! section break and change records; its mark's formatting; and its runs' formatting by
//! property -- for each, the value most of the sample's text has given to the copy's runs
//! that have the value most of the copy's text has (a word with its own emphasis keeps it).
//!
//! Must not: change the paragraph's text, or its list membership (it is noted in the log).

use super::take::{key, take, Styles, Taking};
use super::Log;
use crate::names::W;
use crate::review::xml::{find_path, insert_in_order, make, set_attr};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};

/// What a paragraph keeps of its own properties.
const KEEP_PARAGRAPH: [&str; 5] = ["numPr", "sectPr", "pPrChange", "cnfStyle", "rPr"];
/// A run's (and a paragraph mark's) revision marks and change record: never formatting.
const KEEP_RUN: [&str; 5] = ["ins", "del", "moveFrom", "moveTo", "rPrChange"];

/// A run's stated properties by local name: each one's value and element.
type Values = BTreeMap<String, (String, Id)>;

/// `paragraph`'s properties and its runs' formatting from `sample`'s (both in `work`).
///
/// # Errors
/// A style the sink cannot write.
pub(super) fn follow_paragraph(
    work: &mut Work,
    paragraph: Id,
    sample: Id,
    styles: &mut Styles<'_, '_, '_>,
    log: &mut Log,
    orders: &Orders,
) -> Result<()> {
    let sample_properties = work.find(sample, W, "pPr");
    let old_style = find_path(work, paragraph, &["pPr", "pStyle"]);
    let missing = styles.missing.len();
    let taking = Taking {
        local: "pPr",
        keep: &KEEP_PARAGRAPH,
        style: Some(("pStyle", "paragraph")),
    };
    let (_, mut names) = take(work, paragraph, sample_properties, &taking, styles, orders)?;
    if let (true, Some(old_style)) = (styles.missing.len() > missing, old_style) {
        // the sample's style is not here: the copy keeps its own
        let properties = match work.find(paragraph, W, "pPr") {
            Some(found) => found,
            None => {
                let made = make(work, "pPr", &[]);
                work.insert(paragraph, 0, made);
                made
            }
        };
        if work.find(properties, W, "pStyle").is_none() {
            let kept = work.deep_copy(old_style);
            insert_in_order(work, properties, kept, orders);
            names.retain(|n| n != "pStyle");
        }
    }
    let listed = |work: &Work, p: Id| find_path(work, p, &["pPr", "numPr"]).is_some();
    if listed(work, paragraph) != listed(work, sample) {
        log.list.push(paragraph);
    }
    let mark_source = sample_properties.and_then(|p| work.find(p, W, "rPr"));
    let mut owner = work.find(paragraph, W, "pPr");
    let states = |work: &Work, mark: Id| {
        work.children(mark)
            .iter()
            .any(|c| work.is_element(*c) && !work.is_any(*c, W, &KEEP_RUN))
    };
    if owner.is_none() && mark_source.is_some_and(|m| states(work, m)) {
        let made = make(work, "pPr", &[]);
        work.insert(paragraph, 0, made);
        owner = Some(made);
    }
    let mut mark_changed = false;
    if let Some(owner) = owner {
        let taking = Taking {
            local: "rPr",
            keep: &KEEP_RUN,
            style: Some(("rStyle", "character")),
        };
        mark_changed = !take(work, owner, mark_source, &taking, styles, orders)?.1.is_empty();
        if work.children(owner).is_empty() {
            work.remove(owner);
        }
    }
    if !names.is_empty() || mark_changed {
        log.paragraphs += 1;
    }
    log.runs += follow_runs(work, paragraph, sample, styles, orders)?;
    Ok(())
}

/// The paragraph's runs in order, each with how many characters of its text it holds.
fn run_weights(work: &Work, paragraph: Id) -> Vec<(Id, usize)> {
    let walked = crate::text::walk(work, paragraph);
    let mut counts: BTreeMap<Id, usize> = BTreeMap::new();
    for atom in &walked.atoms {
        if let Some(run) = atom.run {
            *counts.entry(run).or_default() += 1;
        }
    }
    walked
        .runs
        .iter()
        .map(|run| (*run, counts.get(run).copied().unwrap_or(0)))
        .collect()
}

/// Per stated property of `run` (its `w:rPr`, or a paragraph mark's), its value and
/// element; with `styles`, a character style the sample's as this document's (a copy).
fn run_values(work: &mut Work, run: Id, mut styles: Option<&mut Styles<'_, '_, '_>>) -> Result<Values> {
    let mut out = Values::new();
    let Some(properties) = work.find(run, W, "rPr") else {
        return Ok(out);
    };
    for child in work.children(properties).to_vec() {
        if !work.is_element(child) || work.is_any(child, W, &KEEP_RUN) {
            continue;
        }
        let Some(name) = work.name(child).map(|n| n.local().to_owned()) else {
            continue;
        };
        let mut node = child;
        if let (true, Some(styles)) = (name == "rStyle", styles.as_deref_mut()) {
            let wanted = work.get(child, Some(W), "val").map(str::to_owned);
            let Some(found) = styles.id(wanted.as_deref(), "character")? else {
                continue;
            };
            node = work.deep_copy(child);
            set_attr(work, node, "val", &found);
        }
        out.insert(name, (key(work, node), node));
    }
    Ok(out)
}

/// The value and element of `name` most of the text has (`None`: not stated); a tie goes
/// to not stated, then to the earlier run's.
fn majority<'v>(values: impl Iterator<Item = (&'v Values, usize)>, name: &str) -> Option<(String, Id)> {
    // each value met: its count, where it was first met and its first element
    let mut counts: Vec<(Option<&String>, usize, usize, Option<Id>)> = Vec::new();
    for (k, (found, weight)) in values.enumerate() {
        let entry = found.get(name);
        let value = entry.map(|e| &e.0);
        match counts.iter_mut().find(|c| c.0 == value) {
            Some(c) => c.1 += weight,
            None => counts.push((value, weight, k, entry.map(|e| e.1))),
        }
    }
    let best = counts
        .iter()
        .max_by_key(|(value, count, first, _)| (*count, value.is_none(), std::cmp::Reverse(*first)))?;
    Some((best.0?.clone(), best.3?))
}

/// The copy's runs' formatting toward the sample's, property by property; how many runs
/// changed.
fn follow_runs(
    work: &mut Work,
    paragraph: Id,
    sample: Id,
    styles: &mut Styles<'_, '_, '_>,
    orders: &Orders,
) -> Result<usize> {
    let mut sample_runs: Vec<(Values, usize)> = Vec::new();
    for (run, weight) in run_weights(work, sample) {
        sample_runs.push((run_values(work, run, Some(&mut *styles))?, weight));
    }
    let mut runs: Vec<(Id, Values, usize)> = Vec::new();
    for (run, weight) in run_weights(work, paragraph) {
        runs.push((run, run_values(work, run, None)?, weight));
    }
    if sample_runs.iter().all(|(_, w)| *w == 0) {
        return Ok(0);
    }
    if runs.iter().all(|(_, _, w)| *w == 0) {
        // no text yet: the paragraph mark's formatting is what text typed there takes
        let holder = match work.find(paragraph, W, "pPr") {
            Some(found) => found,
            None => {
                let made = make(work, "pPr", &[]);
                work.insert(paragraph, 0, made);
                made
            }
        };
        runs.push((holder, run_values(work, holder, None)?, 1));
    }
    let names: BTreeSet<String> = sample_runs
        .iter()
        .flat_map(|(found, _)| found.keys())
        .chain(runs.iter().flat_map(|(_, found, _)| found.keys()))
        .cloned()
        .collect();
    let mut changed: BTreeSet<Id> = BTreeSet::new();
    for name in &names {
        let wanted = majority(sample_runs.iter().map(|(f, w)| (f, *w)), name);
        let mine = majority(runs.iter().map(|(_, f, w)| (f, *w)), name);
        let mine_value = mine.map(|m| m.0);
        if wanted.as_ref().map(|w| &w.0) == mine_value.as_ref() {
            continue;
        }
        for (run, found, _) in &runs {
            if found.get(name).map(|f| &f.0) != mine_value.as_ref() {
                continue; // its own emphasis
            }
            restate(work, *run, name, wanted.as_ref().map(|w| w.1), orders);
            changed.insert(*run);
        }
    }
    for (holder, _, _) in &runs {
        if work.is(*holder, W, "pPr") && work.children(*holder).is_empty() && work.parent(*holder).is_some() {
            work.remove(*holder);
        }
    }
    Ok(changed.len())
}

/// `run`'s (or a paragraph mark's) `name` made `wanted` (`None`: not stated).
fn restate(work: &mut Work, run: Id, name: &str, wanted: Option<Id>, orders: &Orders) {
    let mut properties = work.find(run, W, "rPr");
    if let Some(found) = properties {
        for node in work.children(found).to_vec() {
            if work.is(node, W, name) {
                work.remove(node);
            }
        }
    }
    if let Some(wanted) = wanted {
        let holder = match properties {
            Some(found) => found,
            None => {
                let made = make(work, "rPr", &[]);
                insert_in_order(work, run, made, orders);
                made
            }
        };
        properties = Some(holder);
        let copy = work.deep_copy(wanted);
        insert_in_order(work, holder, copy, orders);
    }
    if let Some(found) = properties.filter(|p| work.children(*p).is_empty()) {
        work.remove(found);
    }
    if work.is(run, W, "pPr") && work.children(run).is_empty() {
        work.remove(run);
    }
}
