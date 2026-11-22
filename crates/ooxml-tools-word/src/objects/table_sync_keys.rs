//! Owns: which table row each source key names in a `sync` (docx-agent's
//! `table_sync._matches`, round 52, R106) -- `match`'s pairs first (a pair written the other
//! way round read as meant, with a note), then the keys equal as they are, then (`loose`)
//! the keys left equal by [`loose_key`] -- and where the rows of new keys go (`_slots`:
//! beside the data rows, in key order where the table is in it, else in the source's); the
//! refusals before any change: a pair naming no key, a key paired twice, a key that loosely
//! matches more than one, rows to add like a row whose one cell spans two columns taken.
//!
//! Must not: change the document.

use super::table_sync::{Grid, Plan, Taken, LISTED};
use super::table_sync_read::refusal;
use crate::sources::{loose_key, natural};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, TryCall};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The keys matched: the table row by source key, the pairs `match` gave that differ, the
/// loose pairs (table key, source key).
#[derive(Debug, Default)]
pub(super) struct Matched {
    pub(super) row_of: HashMap<String, usize>,
    pub(super) paired: Vec<(String, String)>,
    pub(super) loosely: Vec<(String, String)>,
}

/// Which table row each source key names (`_matches`).
pub(super) fn matches(
    plan: &Plan,
    source_keys: &[Option<String>],
    pairs: &[(String, String)],
    loose: bool,
    (address, table): (&str, &str),
    notes: &mut Vec<String>,
    retry: &TryCall,
) -> Result<Matched, ToolError> {
    let wanted: Vec<&String> = source_keys.iter().flatten().collect();
    let (mut out, mut used) = given(plan, &wanted, pairs, (address, table), notes, retry)?;
    for k in &wanted {
        if !out.row_of.contains_key(*k) && !used.contains(*k) {
            if let Some(row) = plan.by_key.get(*k) {
                out.row_of.insert((*k).clone(), *row);
                used.insert((*k).clone());
            }
        }
    }
    if loose {
        loosely(plan, &wanted, &mut out, &used, retry)?;
    }
    Ok(out)
}

/// `match`'s pairs (round 52, R106): the table row by source key, and the table keys they
/// take; a pair naming a key that is not one, or a key paired twice, refused.
fn given(
    plan: &Plan,
    wanted: &[&String],
    pairs: &[(String, String)],
    (address, table): (&str, &str),
    notes: &mut Vec<String>,
    retry: &TryCall,
) -> Result<(Matched, HashSet<String>), ToolError> {
    let in_source: HashSet<&str> = wanted.iter().map(|k| k.as_str()).collect();
    let mut out = Matched::default();
    let mut used: HashSet<String> = HashSet::new();
    for (n, (said_table, said_data)) in pairs.iter().enumerate() {
        let (mut t, mut d) = (super::table_sync::norm(said_table), super::table_sync::norm(said_data));
        if !plan.by_key.contains_key(&t) && plan.by_key.contains_key(&d) && in_source.contains(t.as_str()) {
            notes.push(format!(
                "match pair {} ({}: {}) read the other way round: {} is the table's key, {} the source's",
                n + 1,
                repr_str(said_table),
                repr_str(said_data),
                repr_str(&d),
                repr_str(&t)
            ));
            std::mem::swap(&mut t, &mut d);
        }
        if !plan.by_key.contains_key(&t) || !in_source.contains(d.as_str()) {
            let (bad, keys, place): (&str, Vec<&str>, String) = if plan.by_key.contains_key(&t) {
                (
                    &d,
                    wanted.iter().map(|k| k.as_str()).collect(),
                    format!("a key of {address}"),
                )
            } else {
                (
                    &t,
                    plan.order.iter().map(String::as_str).collect(),
                    format!("a key of {table}"),
                )
            };
            let form = loose_key(bad);
            let near: Vec<&str> = keys.iter().copied().filter(|k| loose_key(k) == form).collect();
            let options: Vec<String> = near
                .iter()
                .copied()
                .chain(keys.iter().copied().filter(|k| !near.contains(k)))
                .take(50)
                .map(str::to_owned)
                .collect();
            let loosely = if near.is_empty() {
                String::new()
            } else {
                let shown: Vec<String> = near.iter().take(10).map(|k| repr_str(k)).collect();
                format!("; loosely equal: {}", shown.join(", "))
            };
            return Err(refusal(
                Refusal::NotFound,
                format!(
                    "match pair {}: {} is not {place} (key column {}); pairs are {{table: table key, data: source \
                     key}}{loosely}",
                    n + 1,
                    repr_str(bad),
                    plan.key_col
                ),
                &format!("match[{n}]"),
                options,
                retry,
            ));
        }
        if used.contains(&t) || out.row_of.contains_key(&d) {
            // docx-agent's message: the table key as it is, the source key in quotes
            let twice = if used.contains(&t) { t.clone() } else { repr_str(&d) };
            return Err(refusal(
                Refusal::InvalidArguments,
                format!("match pair {}: {twice} is paired twice: a key names one row", n + 1),
                &format!("match[{n}]"),
                Vec::new(),
                retry,
            ));
        }
        used.insert(t.clone());
        if let Some(row) = plan.by_key.get(&t) {
            out.row_of.insert(d.clone(), *row);
        }
        if t != d {
            out.paired.push((t, d));
        }
    }
    Ok((out, used))
}

/// The keys left on both sides matched by [`loose_key`] (`loose`); a key that would loosely
/// match more than one on the other side refused, naming them.
fn loosely(
    plan: &Plan,
    wanted: &[&String],
    out: &mut Matched,
    used: &HashSet<String>,
    retry: &TryCall,
) -> Result<(), ToolError> {
    let mut tables: Vec<(Vec<crate::sources::Run>, Vec<String>)> = Vec::new();
    for k in &plan.order {
        let form = loose_key(k);
        if !used.contains(k) && !form.is_empty() {
            match tables.iter_mut().find(|(f, _)| *f == form) {
                Some((_, keys)) => keys.push(k.clone()),
                None => tables.push((form, vec![k.clone()])),
            }
        }
    }
    let mut datas: HashMap<Vec<crate::sources::Run>, Vec<String>> = HashMap::new();
    for k in wanted {
        let form = loose_key(k);
        if !out.row_of.contains_key(*k) && !form.is_empty() {
            datas.entry(form).or_default().push((*k).clone());
        }
    }
    let mut clashes = Vec::new();
    for (form, ours) in &tables {
        let Some(theirs) = datas.get(form) else { continue };
        if ours.len() > 1 || theirs.len() > 1 {
            let side = |keys: &[String]| keys.iter().map(|k| repr_str(k)).collect::<Vec<_>>().join(", ");
            clashes.push(format!("{} (table) / {} (source)", side(ours), side(theirs)));
            continue;
        }
        let (Some(ours), Some(theirs)) = (ours.first(), theirs.first()) else {
            continue;
        };
        if let Some(row) = plan.by_key.get(ours) {
            out.row_of.insert(theirs.clone(), *row);
        }
        out.loosely.push((ours.clone(), theirs.clone()));
    }
    if !clashes.is_empty() {
        let more = if clashes.len() > 10 {
            format!(" and {} more", clashes.len() - 10)
        } else {
            String::new()
        };
        return Err(refusal(
            Refusal::Ambiguous,
            format!(
                "{} key(s) loosely match more than one key on the other side: {}{more}: pair them with match ({{table \
                 key: source key}}) or make the keys unique; nothing was changed",
                clashes.len(),
                clashes.iter().take(10).cloned().collect::<Vec<_>>().join("; ")
            ),
            "loose",
            vec!["match".to_owned()],
            retry,
        ));
    }
    Ok(())
}

/// Without `loose`: which table keys gone and source keys added would match loosely
/// (`_note_loose`) -- the way out.
pub(super) fn note_loose(gone: &[String], added: &[String], remove: bool, notes: &mut Vec<String>) {
    let mut forms: HashMap<Vec<crate::sources::Run>, Vec<&String>> = HashMap::new();
    for k in gone {
        forms.entry(loose_key(k)).or_default().push(k);
    }
    let pairs: Vec<(&String, &String)> = added
        .iter()
        .filter_map(|k| match forms.get(&loose_key(k)).map(Vec::as_slice) {
            Some(&[found]) => Some((found, k)),
            _ => None,
        })
        .collect();
    if pairs.is_empty() {
        return;
    }
    let mut shown = pairs
        .iter()
        .take(LISTED)
        .map(|(a, b)| format!("{} / {}", repr_str(a), repr_str(b)))
        .collect::<Vec<_>>()
        .join(", ");
    if pairs.len() > LISTED {
        shown.push_str(&format!(" and {} more", pairs.len() - LISTED));
    }
    notes.push(format!(
        "{} table key(s) match a source key loosely (case, spaces, separators, leading zeros) but were compared \
         exactly (the source's added, the table's {}): {shown} (table / source); loose: 'table' matches them, \
         keeping the table's spelling",
        pairs.len(),
        if remove { "removed" } else { "missing" }
    ));
}

/// Refuse where one cell of `row` spans two or more columns taken (`_merged_refusal`):
/// which, and the `from_columns` that takes only the first of them.
pub(super) fn merged_refusal(
    what: &str,
    row: usize,
    grid: &Grid,
    plan: &Plan,
    taken: &Taken<'_>,
    retry: &TryCall,
) -> Result<(), ToolError> {
    let mut by_cell: Vec<((usize, usize), Vec<usize>)> = Vec::new();
    for (col, _) in &plan.pairs {
        let Some(cell) = grid.get(row).and_then(|l| l.get(*col)).and_then(Option::as_ref) else {
            continue;
        };
        let origin = (cell.row, cell.column);
        match by_cell.iter_mut().find(|(o, _)| *o == origin) {
            Some((_, cols)) => cols.push(*col),
            None => by_cell.push((origin, vec![*col])),
        }
    }
    let picks: BTreeMap<usize, usize> = plan.pairs.iter().copied().collect();
    for (origin, cols) in by_cell {
        if cols.len() < 2 {
            continue;
        }
        let names: Vec<String> = cols
            .iter()
            .filter_map(|c| picks.get(c))
            .map(|p| taken.label(*p))
            .collect();
        let widest = plan.pairs.iter().map(|p| p.0).max().unwrap_or(0);
        let mut example = vec![String::new(); widest + 1];
        for (c, pick) in &plan.pairs {
            if !cols.iter().skip(1).any(|x| x == c) {
                if let Some(slot) = example.get_mut(*c) {
                    *slot = taken.label(*pick);
                }
            }
        }
        let example_said = format!(
            "[{}]",
            example.iter().map(|e| repr_str(e)).collect::<Vec<_>>().join(", ")
        );
        return Err(refusal(
            Refusal::Refused,
            format!(
                "{what}: its cell ({}, {}) is merged across columns {}, each taken ({}), one value each: take only one \
                 of them -- '' for the others in from_columns, e.g. {example_said} -- or split the cell first (nothing \
                 was changed)",
                origin.0,
                origin.1,
                cols.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "),
                names.join(", ")
            ),
            "from_columns",
            names,
            retry,
        )
        .with_detail("from_columns", json!(example)));
    }
    Ok(())
}

/// Whether the table's keys are in key order (two at least: `_sorted`).
pub(super) fn sorted(keys: &[String]) -> bool {
    if keys.len() < 2 {
        return false;
    }
    let order: Vec<_> = keys.iter().map(|k| natural(k)).collect();
    order.iter().zip(order.iter().skip(1)).all(|(a, b)| a <= b)
}

/// Where rows added go: beside table row `row` (below it unless not `below`), the source
/// rows in that order.
#[derive(Debug, Clone)]
pub(super) struct Slot {
    pub(super) row: usize,
    pub(super) below: bool,
    pub(super) rows: Vec<usize>,
}

/// The slots of the rows added (`_slots`), by row.
pub(super) fn slots(
    plan: &Plan,
    source_keys: &[Option<String>],
    added: &[usize],
    in_key_order: bool,
    row_of: &HashMap<String, usize>,
) -> Vec<Slot> {
    if added.is_empty() {
        return Vec::new();
    }
    let key_of = |i: usize| source_keys.get(i).cloned().flatten().unwrap_or_default();
    let data_rows: Vec<usize> = plan.order.iter().filter_map(|k| plan.by_key.get(k).copied()).collect();
    let mut found: Vec<((usize, bool), Vec<usize>)> = Vec::new();
    let mut put = |place: (usize, bool), i: usize| match found.iter_mut().find(|(p, _)| *p == place) {
        Some((_, rows)) => rows.push(i),
        None => found.push((place, vec![i])),
    };
    match (data_rows.first().copied(), data_rows.last().copied()) {
        (Some(_), Some(last_row)) if in_key_order => {
            let mut order: Vec<usize> = added.to_vec();
            order.sort_by_key(|i| natural(&key_of(*i)));
            let keys: Vec<_> = plan.order.iter().map(|k| natural(k)).collect();
            for i in order {
                let mine = natural(&key_of(i));
                let after = keys.iter().zip(&data_rows).find(|(k, _)| **k > mine).map(|(_, r)| *r);
                put(after.map_or((last_row, true), |r| (r, false)), i);
            }
        }
        (Some(first_row), Some(_)) => {
            let mut last: Option<usize> = None;
            for (i, found) in source_keys.iter().enumerate() {
                let Some(found) = found else { continue };
                if let Some(row) = row_of.get(found) {
                    last = Some(*row);
                } else if added.contains(&i) {
                    put(last.map_or((first_row, false), |r| (r, true)), i);
                }
            }
        }
        _ => {
            // no data row: after the body's last row (the header's, or a row with no key)
            for i in added {
                put((plan.end.saturating_sub(1), true), *i);
            }
        }
    }
    // sorted by row, a row's slots as found (a stable sort)
    let mut out: Vec<Slot> = found
        .into_iter()
        .map(|((row, below), rows)| Slot { row, below, rows })
        .collect();
    out.sort_by_key(|s| s.row);
    out
}
