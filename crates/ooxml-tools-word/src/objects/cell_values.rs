//! Owns: `set_cells`' values by reference (docx-agent's `tools/objects.py`): a top-level
//! `from` naming one value read as the `from` of each cell given without its own
//! (`_one_value_cells`, round 45 R66 and round 49 R96: said in a warning, refused without
//! cells with the call that works), each entry's text (`_cell_value`: its `text`, or the one
//! value its `from` names as the host wrote it), and what the reply says of the values
//! written by reference and the keeps of them (`_cell_facts`): each with its cell -- a
//! column's fill with its rows and count (round 48, R89) -- a `from` kept as the source of
//! its cells' whole paragraphs, a reference in a fill's text kept in each cell it went into
//! (`_in_each`), then the references in single cells kept where they went
//! ([`written::keeps_in`], the dispatcher's `keep_written`).
//!
//! The host resolves each value (its dispatcher's `call.fact`) and hands it over as a
//! [`Written`] at `cells[k].from`, or its refusal at that argument
//! ([`crate::Others::refused_value`]); the refusal is the call's where docx-agent reads the
//! value: after the places, in the order of the cells.
//!
//! Must not: resolve a reference (the host's datasets do), or change the document.

use super::edit_table_args::CellArg;
use super::table_view::{table_view, AcceptedView};
use crate::review::Editing;
use crate::written::{self, After, Written};
use ooxml_tools::sources::SourceRefused;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_word_edit::TextKeep;
use serde_json::{Map, Value};
use std::borrow::Cow;

/// The cells `set_cells` writes, as the call gives them or as a top-level `from` naming one
/// value fills them, and the warning that says so.
pub(super) struct CellsGiven<'a> {
    /// The cells entries (`None`: none given).
    pub(super) cells: Option<Cow<'a, [CellArg]>>,
    /// How a top-level `from` naming one value was read, said first in the reply.
    pub(super) read_as: Option<String>,
}

impl<'a> CellsGiven<'a> {
    /// The call's own cells.
    pub(super) fn own(cells: Option<&'a [CellArg]>) -> Self {
        Self {
            cells: cells.map(Cow::Borrowed),
            read_as: None,
        }
    }

    /// A top-level `from` naming one value (`source`) with `cells` (`_one_value_cells`): the
    /// value of each cell that gives no `from` or `text` of its own; without cells it is
    /// refused with the call that works.
    pub(super) fn one_value(source: &str, cells: Option<&'a [CellArg]>, retry: &TryCall) -> Result<Self, ToolError> {
        let example = format!("cells: [{{row: 1, col: 2, from: '{source}'}}]");
        let Some(cells) = cells.filter(|c| !c.is_empty()) else {
            let dataset = source.split('/').next().unwrap_or(source).to_owned();
            return Err(ToolError::refusal(
                Refusal::InvalidArguments,
                format!(
                    "from {source} is one value: set_cells from fills rows (d:<id>, d:<id>/#9:#16); one value goes \
                     in a cell: {example}"
                ),
                WayOut::names([example.clone(), dataset]).unwrap_or_else(|| WayOut::call(retry.clone())),
            )
            .at("from"));
        };
        let given = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.is_empty());
        let mut filled = 0;
        let out: Vec<CellArg> = cells
            .iter()
            .map(|cell| {
                if given(&cell.text) || given(&cell.from) {
                    cell.clone()
                } else {
                    filled += 1;
                    CellArg {
                        from: Some(source.to_owned()),
                        ..cell.clone()
                    }
                }
            })
            .collect();
        let own = out.len() - filled;
        let read_as = if filled == 0 {
            format!(
                "from {source} (one value) filled no cell: each of the {own} cell(s) gives its own from or text, which \
                 it keeps"
            )
        } else if own > 0 {
            format!(
                "from {source} (one value) read as the from of each of the {filled} cell(s) given without their own; \
                 {own} kept their own"
            )
        } else {
            format!("from {source} (one value) read as the from of each of the {filled} cell(s) given")
        };
        Ok(Self {
            cells: Some(Cow::Owned(out)),
            read_as: Some(read_as),
        })
    }
}

/// A cells entry's text, and the value by reference it is, where its `from` names one.
pub(super) struct CellValue<'e> {
    pub(super) text: String,
    pub(super) fact: Option<&'e Written>,
}

/// Each entry's text, in order (`_cell_value`): its `text`, or the value its `from` names as
/// the host wrote it -- a strict decoder's `""` no value beside the other; neither, or both,
/// refused; a value the host refused, its refusal.
pub(super) fn cell_values<'e>(
    cells: &[CellArg],
    editing: &Editing<'e>,
    retry: &TryCall,
) -> Result<Vec<CellValue<'e>>, ToolError> {
    cells
        .iter()
        .enumerate()
        .map(|(k, cell)| cell_value(cell, k, editing, retry))
        .collect()
}

fn cell_value<'e>(
    cell: &CellArg,
    k: usize,
    editing: &Editing<'e>,
    retry: &TryCall,
) -> Result<CellValue<'e>, ToolError> {
    let names = |options: [&str; 2]| WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()));
    let source = cell.from.as_deref().filter(|f| !f.is_empty());
    let text = cell.text.as_deref().filter(|t| source.is_none() || !t.is_empty());
    let Some(source) = source else {
        return text
            .map(|text| CellValue {
                text: text.to_owned(),
                fact: None,
            })
            .ok_or_else(|| {
                ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!("cells[{k}] gives no text: give text, or from (one value, d:<id>/#12/<column>)"),
                    names(["text", "from"]),
                )
                .at(format!("cells[{k}].text"))
            });
    };
    let field = format!("cells[{k}].from");
    if text.is_some() {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!("cells[{k}] gives text and from: give one"),
            names(["text", "from"]),
        )
        .at(field));
    }
    // a cell given without its own value reads the call's one-value `from` (docx-agent's
    // `_cell_froms`): the host resolved it once, at `from`
    let at = |w: &&Written| w.field == field;
    let inherited = |w: &&Written| w.field == "from";
    if let Some(fact) = editing
        .written
        .iter()
        .find(at)
        .or_else(|| editing.written.iter().find(inherited))
    {
        return Ok(CellValue {
            text: fact.text.clone(),
            fact: Some(fact),
        });
    }
    let refused = editing.others.refused_value(&field).cloned().or_else(|| {
        editing
            .others
            .refused_value("from")
            .filter(|r| r.given == source)
            .map(|r| SourceRefused {
                field: Some(field.clone()),
                ..r.clone()
            })
    });
    Err(match refused {
        Some(refused) => crate::sources::value_refusal(&refused, retry),
        None => crate::sources::no_rows(source, &field, retry),
    })
}

/// The entry `cells[k].text` a value written by reference went into, where it is one.
fn text_entry(fact: &Written) -> Option<usize> {
    fact.field.strip_prefix("cells[")?.strip_suffix("].text")?.parse().ok()
}

/// Whether the call wrote any value by reference (a `from`, or a reference in a text).
pub(super) fn any_by_reference(values: &[CellValue<'_>], editing: &Editing<'_>) -> bool {
    values.iter().any(|v| v.fact.is_some()) || editing.written.iter().any(|w| text_entry(w).is_some())
}

/// What `set_cells` wrote, as its reply's facts are made of it.
pub(super) struct Wrote<'a, 'e> {
    /// The entries, as written (a top-level value filled in).
    pub(super) cells: &'a [CellArg],
    /// Each entry's value.
    pub(super) values: &'a [CellValue<'e>],
    /// The entry each write came from.
    pub(super) owners: &'a [usize],
    /// The table as the call named it.
    pub(super) table: &'a str,
    /// Its positions as accepted, before the call.
    pub(super) accepted: &'a AcceptedView,
}

/// The reply's `facts` (`None` without a value by reference) and the keeps of the values,
/// on the version the writes `at` (document row, grid column, in order) made (`_cell_facts`).
pub(super) fn cell_facts(
    wrote: &Wrote<'_, '_>,
    editing: &Editing<'_>,
    at: &[(usize, usize)],
    after: &After,
    retry: &TryCall,
) -> (Option<Vec<Value>>, Vec<TextKeep>) {
    let in_text = |k: usize| editing.written.iter().filter(move |w| text_entry(w) == Some(k));
    if !any_by_reference(wrote.values, editing) {
        return (None, Vec::new());
    }
    let now = after.doc().ids().follow(wrote.table);
    let Ok(view) = table_view(after.doc(), after.docx(), &now, retry) else {
        return (Some(Vec::new()), Vec::new());
    };
    let accepted = &wrote.accepted.table;
    let mut out = Vec::new();
    let mut keeps = Vec::new();
    for (k, value) in wrote.values.iter().enumerate() {
        if value.fact.is_none() && in_text(k).next().is_none() {
            continue;
        }
        let placed: Vec<(usize, usize)> = wrote
            .owners
            .iter()
            .zip(at)
            .filter(|(owner, _)| **owner == k)
            .map(|(_, place)| *place)
            .collect();
        let origins: Vec<(usize, usize)> = placed.iter().filter_map(|(r, c)| view.origin(*r, *c)).collect();
        let cell = wrote.cells.get(k);
        let many = cell.and_then(|c| c.rows.as_deref()).is_some_and(|r| !r.is_empty());
        let mut said = Map::new();
        if many {
            let position = |row: usize| {
                if accepted.rows_active() {
                    accepted.from_row(row)
                } else {
                    row
                }
            };
            let rows = match (origins.first(), origins.last()) {
                (Some(first), Some(last)) => format!("{}..{}", position(first.0), position(last.0)),
                _ => String::new(),
            };
            let col = placed.first().map_or(0, |p| accepted.from_grid(p.1));
            said.insert("col".into(), Value::from(col));
            said.insert("rows".into(), Value::from(rows));
            said.insert("cells".into(), Value::from(origins.len()));
        } else if let Some((r, c)) = origins.first() {
            said.insert("cell".into(), Value::from(format!("{now}/c{r},{c}")));
        }
        let paragraphs: Vec<Vec<String>> = origins
            .iter()
            .map(|(r, c)| written::paragraphs_of(after, &format!("{now}/c{r},{c}")))
            .collect();
        if let Some(fact) = value.fact {
            for ids in &paragraphs {
                keeps.extend(written::whole_keeps(after, fact, ids));
            }
            out.push(said_fact(fact, &said));
        }
        for fact in in_text(k) {
            if many {
                let text = cell.and_then(|c| c.text.as_deref());
                keeps.extend(in_each(fact, text, &paragraphs));
            }
            out.push(said_fact(fact, &said));
        }
    }
    (Some(out), keeps)
}

/// What the reply says of `fact`, where it went (`said`) in place of its blocks, before its
/// sources (`_said_fact`).
fn said_fact(fact: &Written, said: &Map<String, Value>) -> Value {
    let Value::Object(brief) = written::brief(fact, None) else {
        return Value::Null;
    };
    let mut out = Map::new();
    for key in ["from", "text", "chars"] {
        if let Some(value) = brief.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    out.extend(said.clone());
    for key in ["sources", "stale"] {
        if let Some(value) = brief.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    Value::Object(out)
}

/// A reference written in a column fill's `text`, kept on each cell it went into: the
/// paragraph of its line, at its place in the line (`_in_each`).
fn in_each(fact: &Written, text: Option<&str>, paragraphs: &[Vec<String>]) -> Vec<TextKeep> {
    let Some(text) = text else { return Vec::new() };
    let chars: Vec<char> = text.chars().collect();
    let own: Vec<char> = fact.text.chars().collect();
    if own.is_empty() || chars.get(fact.start..fact.end) != Some(&own[..]) {
        return Vec::new();
    }
    let before = chars.get(..fact.start).unwrap_or_default();
    let line = before.iter().filter(|c| **c == '\n').count();
    let at = fact.start - before.iter().rposition(|c| *c == '\n').map_or(0, |n| n + 1);
    let mut record = fact.record.clone();
    record.insert("p".into(), Value::from(0));
    record.insert("at".into(), Value::from(at));
    paragraphs
        .iter()
        .filter_map(|ids| ids.get(line))
        .map(|paragraph| TextKeep {
            paragraph: paragraph.clone(),
            facts: vec![Value::Object(record.clone())],
            labels: Vec::new(),
        })
        .collect()
}
