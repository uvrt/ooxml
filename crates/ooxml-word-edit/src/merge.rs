//! Owns: runs an edit split merged back where their properties are the same, as
//! docx-agent's `inline.merge_runs` merges them: twins are adjacent runs
//! of text and the like only (`_MERGEABLE`), with the same attributes and the same
//! `w:rPr`; the right one's content joins the left one, and adjacent `w:t`s with the same
//! attributes join into one.
//!
//! Must not: merge a run nobody touched with another untouched one.

use crate::names::W;
use crate::text::write::set_spaced_text;
use crate::work::{Id, Work};

/// What a mergeable run may hold.
const MERGEABLE: [&str; 9] = [
    "rPr",
    "t",
    "tab",
    "br",
    "cr",
    "noBreakHyphen",
    "softHyphen",
    "sym",
    "ptab",
];

/// Merge adjacent twins among `runs`.
pub(crate) fn merge_runs(work: &mut Work, runs: &[Id]) {
    let mut seen: Vec<Id> = Vec::new();
    for &run in runs {
        if seen.contains(&run) || work.parent(run).is_none() {
            continue;
        }
        seen.push(run);
        let mut current = run;
        while let Some(previous) = work
            .previous(current)
            .filter(|p| runs.contains(p) && twins(work, *p, current))
        {
            join(work, previous, current);
            current = previous;
        }
        while let Some(following) = work
            .next(current)
            .filter(|f| runs.contains(f) && twins(work, current, *f))
        {
            seen.push(following);
            join(work, current, following);
        }
    }
}

fn mergeable(work: &Work, run: Id) -> bool {
    work.is(run, W, "r")
        && work
            .children(run)
            .iter()
            .all(|c| !work.is_element(*c) || work.is_any(*c, W, &MERGEABLE))
}

fn twins(work: &Work, left: Id, right: Id) -> bool {
    let attributes = |run: Id| {
        let mut out: Vec<(Option<String>, String, String)> = work
            .attributes(run)
            .iter()
            .map(|a| {
                (
                    a.name.namespace().map(str::to_owned),
                    a.name.local().to_owned(),
                    a.value.to_string(),
                )
            })
            .collect();
        out.sort();
        out
    };
    let same_properties = match (work.find(left, W, "rPr"), work.find(right, W, "rPr")) {
        (None, None) => true,
        (Some(a), Some(b)) => work.same_xml(a, b),
        _ => false,
    };
    work.next(left) == Some(right)
        && work.tail(left).trim().is_empty()
        && mergeable(work, left)
        && mergeable(work, right)
        && attributes(left) == attributes(right)
        // the key a tracking edit gave each run is docx-agent's temporary attribute
        && work.key(left) == work.key(right)
        && same_properties
}

fn join(work: &mut Work, left: Id, right: Id) {
    for child in work.children(right).to_vec() {
        if work.is(child, W, "rPr") {
            continue;
        }
        work.append(left, child);
    }
    work.remove(right);
    join_texts(work, left);
}

fn join_texts(work: &mut Work, run: Id) {
    let mut previous: Option<Id> = None;
    for child in work.children(run).to_vec() {
        if let Some(before) = previous.filter(|p| work.is(child, W, "t") && work.is(*p, W, "t")) {
            if same_attributes(work, before, child) {
                let text = format!("{}{}", work.text(before), work.text(child));
                set_spaced_text(work, before, &text);
                work.remove(child);
                continue;
            }
        }
        previous = Some(child);
    }
}

fn same_attributes(work: &Work, a: Id, b: Id) -> bool {
    let key = |id: Id| {
        let mut out: Vec<(Option<String>, String, String)> = work
            .attributes(id)
            .iter()
            .filter(|t| !t.name.is(Some(crate::names::XML), "space"))
            .map(|t| {
                (
                    t.name.namespace().map(str::to_owned),
                    t.name.local().to_owned(),
                    t.value.to_string(),
                )
            })
            .collect();
        out.sort();
        out
    };
    key(a) == key(b)
}
