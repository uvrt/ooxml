//! Owns: `word_changes`' `accept`, `reject` and `preview` (docx-agent's `_selection`,
//! `_review_changes`, `_preview_changes`, R121's wording): the selection refused as the
//! call that would work, the review run as one command, and the reply -- the records
//! settled, how many of them this session wrote, the changes left by author, the blocks
//! removed; a preview runs both on a private copy of the version it reads.
//!
//! Must not: settle a change but through the edit core's [`Review`].

use super::changes::{invalid, try_call, ChangeAction, Reading, WordChanges};
use super::listing::change_json;
use super::Editing;
use crate::paging::page_list;
use ooxml_store::{Caller, Clock, Store, StoreConfig};
use ooxml_tools::{Done, Refusal, ToolError, ToolResult, WayOut};
use ooxml_word_edit::review::wanted_records;
use ooxml_word_edit::{Mark, RemovedBlocks, Review, Reviewed, Selection, Typed, WordDoc};
use ooxml_word_model::docx::changes::{changes, LogicalChange};
use ooxml_word_model::docx::revisions::records;
use ooxml_word_model::docx::Docx;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

/// How many removed blocks a summary names; the rest are counted.
const NAMED: usize = 6;

/// The selection a call makes, or its refusal.
fn selection(args: &WordChanges) -> Result<Selection, ToolError> {
    let ids = args.ids.clone().filter(|i| !i.is_empty());
    let all = args.all == Some(true);
    let filtered = args.author.is_some() || args.kind.is_some() || args.within.is_some();
    if ids.is_none() && !filtered && !all {
        let mut call = args.clone();
        call.all = Some(true);
        let options = ["ids", "author", "kind", "within", "all"].map(Value::from).to_vec();
        let way =
            WayOut::options(options).map_or_else(|| WayOut::call(try_call(&call)), |w| w.and_call(try_call(&call)));
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "say which changes: ids, author, kind, within or all",
            way,
        ));
    }
    if let Some(ids) = ids {
        if filtered || all {
            let mut call = args.clone();
            call.author = None;
            call.kind = None;
            call.within = None;
            call.all = None;
            return Err(invalid("give ids, or filters, not both", "ids", &call));
        }
        return Ok(Selection::Ids(ids));
    }
    if all && !filtered {
        return Ok(Selection::All);
    }
    Ok(Selection::Matching {
        author: args.author.clone(),
        kind: args.kind.map(|k| k.name().to_owned()),
        within: args.within.clone(),
    })
}

/// A refusal of the review's selection: an id or `within` refused with its way out (a
/// `within` at no field, as today's tool names none).
fn refusal(error: ooxml_word_edit::EditError, args: &WordChanges) -> ToolError {
    let field = if args.ids.is_some() { "ids" } else { "" };
    let mut list = args.clone();
    list.action = ChangeAction::List;
    list.ids = None;
    list.all = None;
    list.author = None;
    list.kind = None;
    list.within = None;
    super::edit_refusal(error, field, try_call(&list))
}

/// `accept` or `reject`.
pub(crate) fn settle(args: &WordChanges, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    let selection = selection(args)?;
    let accept = args.action == ChangeAction::Accept;
    let by_ids = matches!(selection, Selection::Ids(_));
    let review = Review { accept, selection };
    let done = match editing.editor.run(editing.caller, &review) {
        Ok(done) => done,
        // a form not ported (the host's handler answers): said in `unported`
        Err(ooxml_word_edit::EditError::Unsupported(what)) => {
            let mut reply = Done::new(String::new());
            reply.unported.push(what);
            return Ok(ToolResult::done(reply));
        }
        Err(e) => return Err(refusal(e, args)),
    };
    let after = done
        .version
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let opened = docx_of(editing.opened)?;
    let changed = changed_parts(editing.opened, done.version.doc());
    let out = &done.out;
    let session = this_session(&out.settled, &opened, &changed);
    let left = changes(&after);
    let (blocks, order) = merged(&out.blocks);
    let verb = if accept { "Accepted" } else { "Rejected" };
    let mut summary = format!("{verb} {} revision record(s)", out.records);
    if !by_ids {
        summary.push_str(&session_text(session, out.records));
    }
    let named = removed_text(&blocks);
    if !blocks_empty(&blocks) {
        summary.push_str("; ");
        summary.push_str(&named);
    }
    let mut data = Map::new();
    data.insert("records".into(), json!(out.records));
    data.insert("this_session".into(), json!(session));
    data.insert("changes_left".into(), json!(left.len()));
    data.insert("left_by_author".into(), Value::Object(by_author(&left)));
    if !blocks_empty(&blocks) {
        data.insert("removed".into(), blocks_json(&blocks, &order));
    }
    let mut reply = Done::new(summary);
    reply.data = Some(Value::Object(data));
    reply.removed.clone_from(&out.removed);
    reply.renamed.clone_from(&out.renamed);
    let mut result = ToolResult::done(reply);
    result.warnings.clone_from(&out.warnings);
    Ok(result)
}

/// `doc` read.
fn docx_of(doc: &WordDoc) -> Result<Arc<Docx>, ToolError> {
    doc.docx().map_err(|e| ToolError::internal(e.to_string(), "word:model"))
}

/// The parts of `now` whose content differs from the document as opened.
fn changed_parts(opened: &WordDoc, now: &WordDoc) -> BTreeSet<String> {
    now.package()
        .part_names()
        .into_iter()
        .filter(|name| match (opened.package().part(name), now.package().part(name)) {
            (Some(a), Some(b)) => !a.same(b),
            _ => true,
        })
        .collect()
}

/// A record's author and date.
type Stamp = (Option<String>, Option<String>);

/// How many of the records `settled` this session wrote (`_this_session`): in a part
/// changed since the document was opened, dated, with an author and date no record of
/// that part had then.
fn this_session(settled: &BTreeMap<Mark, usize>, opened: &Docx, changed: &BTreeSet<String>) -> usize {
    let mut stamps: BTreeMap<String, BTreeSet<Stamp>> = BTreeMap::new();
    for r in records(opened) {
        let get = |l: &str| {
            r.element
                .attribute(Some(ooxml_word_model::ids::W), l)
                .map(str::to_owned)
        };
        stamps
            .entry(r.part.clone())
            .or_default()
            .insert((get("author"), get("date")));
    }
    settled
        .iter()
        .filter(|((part, _, _, author, date), _)| {
            changed.contains(part)
                && date.is_some()
                && !stamps
                    .get(part)
                    .is_some_and(|s| s.contains(&(author.clone(), date.clone())))
        })
        .map(|(_, n)| *n)
        .sum()
}

/// ` (N written this session, since the document was opened)`, or nothing.
fn session_text(count: usize, records: usize) -> String {
    if count == 0 {
        return String::new();
    }
    let n = if count == records {
        "all".to_owned()
    } else {
        count.to_string()
    };
    format!(" ({n} written this session, since the document was opened)")
}

/// The changes left, counted by author.
fn by_author(left: &[LogicalChange]) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for c in left {
        let key = c.author.clone().unwrap_or_else(|| "None".to_owned());
        let n = out.get(&key).and_then(Value::as_u64).unwrap_or(0);
        out.insert(key, json!(n + 1));
    }
    out
}

/// The removed blocks' kinds, in the order docx-agent's merged dict has them: as each
/// first appears, step by step.
type Order = Vec<&'static str>;

/// The kinds `typed` holds, in its own order.
fn kinds(typed: &Typed) -> Order {
    [
        ("tables", typed.tables.is_empty()),
        ("rows", typed.rows.is_empty()),
        ("paragraphs", typed.paragraphs.is_empty()),
        ("merged", typed.merged.is_empty()),
        ("controls", typed.controls.is_empty()),
    ]
    .into_iter()
    .filter(|(_, empty)| !empty)
    .map(|(k, _)| k)
    .collect()
}

/// The steps' removed blocks merged (`_merged_blocks`): a paragraph merged into one a
/// later step merged in turn named by the last, rows of a table removed left out; and the
/// order its kinds first appear in.
fn merged(steps: &[RemovedBlocks]) -> (Typed, Order) {
    let mut out = Typed::default();
    let mut order: Order = Vec::new();
    for s in steps {
        for kind in kinds(&s.typed) {
            if !order.contains(&kind) {
                order.push(kind);
            }
        }
        out.tables.extend(s.typed.tables.iter().cloned());
        out.rows.extend(s.typed.rows.iter().cloned());
        out.paragraphs.extend(s.typed.paragraphs.iter().cloned());
        for (gone, into) in &s.typed.merged {
            match out.merged.iter_mut().find(|(g, _)| g == gone) {
                Some(slot) => slot.1.clone_from(into),
                None => out.merged.push((gone.clone(), into.clone())),
            }
        }
        out.controls.extend(s.typed.controls.iter().cloned());
    }
    let joins: BTreeMap<String, String> = out.merged.iter().cloned().collect();
    for (gone, into) in &mut out.merged {
        let mut seen: BTreeSet<String> = BTreeSet::from([gone.clone()]);
        while let Some(next) = joins.get(into.as_str()) {
            if seen.contains(into.as_str()) {
                break;
            }
            seen.insert(into.clone());
            into.clone_from(next);
        }
    }
    let tables: BTreeSet<&str> = out.tables.iter().map(|t| t.id.as_str()).collect();
    let rows = out
        .rows
        .iter()
        .filter(|r| r.table.as_deref().is_none_or(|t| !tables.contains(t)))
        .cloned()
        .collect();
    out.rows = rows;
    if out.rows.is_empty() {
        order.retain(|k| *k != "rows");
    }
    (out, order)
}

fn blocks_empty(b: &Typed) -> bool {
    b.tables.is_empty() && b.rows.is_empty() && b.paragraphs.is_empty() && b.merged.is_empty() && b.controls.is_empty()
}

/// The removed blocks as docx-agent's `data.removed`, its kinds in `order`.
fn blocks_json(b: &Typed, order: &Order) -> Value {
    let mut out = Map::new();
    if !b.tables.is_empty() {
        let tables: Vec<Value> = b
            .tables
            .iter()
            .map(|t| json!({"id": t.id, "rows": t.rows, "paragraphs": t.paragraphs}))
            .collect();
        out.insert("tables".into(), Value::Array(tables));
    }
    if !b.rows.is_empty() {
        let rows: Vec<Value> = b
            .rows
            .iter()
            .map(|r| {
                let mut row = Map::new();
                row.insert("id".into(), json!(r.id));
                row.insert("paragraphs".into(), json!(r.paragraphs));
                if let Some(t) = &r.table {
                    row.insert("table".into(), json!(t));
                }
                Value::Object(row)
            })
            .collect();
        out.insert("rows".into(), Value::Array(rows));
    }
    if !b.paragraphs.is_empty() {
        out.insert("paragraphs".into(), json!(b.paragraphs));
    }
    if !b.merged.is_empty() {
        let merged: Map<String, Value> = b.merged.iter().map(|(g, i)| (g.clone(), json!(i))).collect();
        out.insert("merged".into(), Value::Object(merged));
    }
    if !b.controls.is_empty() {
        out.insert("controls".into(), json!(b.controls));
    }
    let ordered: Map<String, Value> = order
        .iter()
        .filter_map(|k| out.get(*k).map(|v| ((*k).to_owned(), v.clone())))
        .collect();
    Value::Object(ordered)
}

fn plural(count: u64, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// `removed table t:A (3 rows, 6 paragraphs), paragraph p:B`, or `removed no block`.
fn removed_text(b: &Typed) -> String {
    let mut named: Vec<String> = b
        .tables
        .iter()
        .map(|t| {
            format!(
                "table {} ({}, {})",
                t.id,
                plural(t.rows, "row"),
                plural(t.paragraphs, "paragraph")
            )
        })
        .collect();
    named.extend(b.rows.iter().map(|r| match &r.table {
        Some(t) => format!("row {} of {t}", r.id),
        None => format!("row {}", r.id),
    }));
    named.extend(b.paragraphs.iter().map(|p| format!("paragraph {p}")));
    named.extend(
        b.merged
            .iter()
            .map(|(p, into)| format!("paragraph {p} (merged into {into})")),
    );
    named.extend(b.controls.iter().map(|c| format!("content control {c}")));
    if named.is_empty() {
        return "removed no block".to_owned();
    }
    let more = named.len().saturating_sub(NAMED);
    let shown: Vec<&String> = named.iter().take(NAMED).collect();
    let mut text = format!(
        "removed {}",
        shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
    );
    if more > 0 {
        text.push_str(&format!(" and {more} more (data.removed)"));
    }
    text
}

/// A clock for a preview's private store, whose timings nobody reads.
pub(crate) struct Still;

impl Clock for Still {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// `preview`: what accepting and what rejecting would settle and remove, each on a
/// private store made from the version `reading` reads; nothing changes.
pub(crate) fn preview(args: &WordChanges, reading: &Reading<'_>) -> Result<Done<Value>, ToolError> {
    let selection = selection(args)?;
    let docx = docx_of(reading.doc)?;
    let chosen = chosen(&docx, &selection).map_err(|e| refusal(e, args))?;
    let opened = docx_of(reading.opened)?;
    let changed = changed_parts(reading.opened, reading.doc);
    let mut data = Map::new();
    let mut parts = Vec::new();
    for accept in [true, false] {
        let store = Store::new(reading.doc.clone(), StoreConfig::default(), Arc::new(Still));
        let review = Review {
            accept,
            selection: selection.clone(),
        };
        let done = store.run(&Caller::anonymous(), &review).map_err(|e| refusal(e, args))?;
        let left = done
            .version
            .doc()
            .docx()
            .map(|d| changes(&d).len())
            .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
        let reviewed: &Reviewed = &done.out;
        let (blocks, order) = merged(&reviewed.blocks);
        let session = this_session(&reviewed.settled, &opened, &changed);
        let mut outcome = Map::new();
        outcome.insert("records".into(), json!(reviewed.records));
        outcome.insert("this_session".into(), json!(session));
        outcome.insert("changes_left".into(), json!(left));
        if !blocks_empty(&blocks) {
            outcome.insert("removed".into(), blocks_json(&blocks, &order));
        }
        let action = if accept { "accept" } else { "reject" };
        data.insert(action.into(), Value::Object(outcome));
        let removed = removed_text(&blocks)
            .replacen("removed no block", "remove no block", 1)
            .replacen("removed ", "remove ", 1);
        parts.push(format!(
            "{action} would settle {} record(s){} and {removed}",
            reviewed.records,
            session_text(session, reviewed.records)
        ));
    }
    let items: Vec<Value> = chosen.iter().map(change_json).collect();
    let mut first = args.clone();
    first.cursor = None;
    let (shown, total, _) = page_list(&items, None, reading.limits.list_items, try_call(&first))?;
    let count = shown.len();
    data.insert("changes".into(), Value::Array(shown));
    if total > count {
        data.insert(
            "changes_not_shown".into(),
            json!({"count": total - count,
                   "call": {"tool": "word_changes", "arguments": {"doc": reading.name, "action": "list"}}}),
        );
    }
    let mut reply = Done::new(format!(
        "Preview, nothing changed: {} change(s); {}",
        chosen.len(),
        parts.join("; ")
    ));
    reply.data = Some(Value::Object(data));
    Ok(reply)
}

/// The changes a selection settles, as they stand.
fn chosen(docx: &Docx, selection: &Selection) -> Result<Vec<LogicalChange>, ooxml_word_edit::EditError> {
    let all = changes(docx);
    Ok(match selection {
        Selection::All => all,
        Selection::Ids(ids) => {
            let wanted: BTreeSet<String> = wanted_records(docx, ids)?.into_iter().collect();
            all.into_iter()
                .filter(|c| c.records.iter().any(|r| wanted.contains(r)))
                .collect()
        }
        Selection::Matching { author, kind, within } => {
            let place = within
                .as_deref()
                .map(|w| ooxml_word_edit::review::place_of(docx, w))
                .transpose()?;
            all.into_iter()
                .filter(|c| ooxml_word_edit::review::matches(c, author.as_deref(), kind.as_deref(), place.as_ref()))
                .collect()
        }
    })
}
