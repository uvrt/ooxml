//! `check` for Word documents (lanes P5a, P5b; docx-agent's `tools/shared.py` `check`): a
//! version's facts -- its validation problems against those it opened with (`validate`),
//! its pages since the last check (`reflow`, [`reflow`]) and its fields (`fields`) -- each
//! part's seconds in `timing`, as docx-agent words them.
//!
//! Owns: the tool's definition ([`args`]), the facts read from the document alone, and
//! those read from its pages as the host's engine lays them out ([`crate::WordLayout`]).
//!
//! Must not: lay out but through the host's engine. What needs the layout's facts
//! (`layout`, a `range`'s facts, `min_size`, `styles`), copies, a scale and a deck's
//! arguments are not given here yet: such a call is answered `unported`, and the host
//! answers it; so is a `reflow` without the host's layout, or of a version its engine does
//! not lay out.

mod args;
mod reflow;

pub use args::{Check, CheckCopy, CheckInclude, CheckScale, CopyLists, CopyReference, ScaleShow, ScaleStep, StepText};
pub use reflow::{LastCheck, Seen};

use crate::describe::cancelled;
use crate::version::{Owned, Version};
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, ToolError};
use ooxml_word_edit::validate::{problems, Delta};
use serde_json::{Map, Value};
use std::sync::{Arc, PoisonError};
use std::time::Instant;

/// The problems a check lists at most (the rest are in `new` and the count).
const LISTED: usize = 50;

/// What a field's result is, said with every `fields` fact.
const FIELDS_NOTE: &str = "results are caches: after moving headings or text, word_fields update";

/// What `unported` says of a check that needs the layout.
pub const CHECK_LAYOUT: &str = "check: layout facts";
/// What `unported` says of a check whose arguments this handler does not take yet.
pub const CHECK_ARGUMENTS: &str = "check: arguments";
/// What `unported` says of a check of a document the session does not own.
pub const CHECK_NOT_OWNED: &str = "check: a document the session does not own";
/// What `unported` says of a `reflow` the host hands no layout for.
pub const CHECK_NO_LAYOUT: &str = "check: no layout";
/// What `unported` says of a `reflow` of a version the host's engine does not lay out.
pub const CHECK_NOT_LAID_OUT: &str = "check: the layout falls back";

/// `check` of a Word document at one version.
///
/// # Errors
/// `timeout` when `cancel` asks to stop; `internal` when the version cannot be read.
pub fn check(args: &Check, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    let wanted = match served(args) {
        Ok(wanted) => wanted,
        Err(why) => return Ok(unported(why)),
    };
    let Some(owned) = version.owned else {
        return Ok(unported(CHECK_NOT_OWNED));
    };
    let mut data = Map::new();
    let mut timing = Map::new();
    let layout = match (wanted.contains(&CheckInclude::Reflow), owned.layout) {
        (true, None) => return Ok(unported(CHECK_NO_LAYOUT)),
        (true, Some(layout)) => Some(layout),
        (false, _) => None,
    };
    if wanted.contains(&CheckInclude::Validate) {
        let started = Instant::now();
        data.insert("validate".into(), validate(owned)?);
        timing.insert("validate".into(), seconds(started));
    }
    cancel.check().map_err(cancelled)?;
    if let Some(layout) = layout {
        let started = Instant::now();
        let last = layout.engine().lock().unwrap_or_else(PoisonError::into_inner).last().cloned();
        let Ok(laid) = layout.laid(version.package) else {
            return Ok(unported(CHECK_NOT_LAID_OUT));
        };
        // `layout`: the time laying the version out, where the engine had not laid it out last
        if !last.is_some_and(|last| Arc::ptr_eq(&last, &laid)) {
            timing.insert("layout".into(), seconds(started));
        }
        let started = Instant::now();
        data.insert("reflow".into(), reflow_facts(layout, &laid, version, owned, args.pages.as_deref())?);
        timing.insert("facts".into(), seconds(started));
    }
    cancel.check().map_err(cancelled)?;
    if wanted.contains(&CheckInclude::Fields) {
        let started = Instant::now();
        data.insert("fields".into(), fields(owned)?);
        timing.insert("fields".into(), seconds(started));
    }
    if wanted.contains(&CheckInclude::App) {
        data.insert("app_findings".into(), Value::Null);
    }
    data.insert("timing".into(), Value::Object(timing));
    let named: Vec<&str> = wanted.iter().map(|w| w.as_str()).collect();
    let mut done = Done::new(format!("Checked {}", named.join(", ")));
    done.data = Some(Value::Object(data));
    Ok(done)
}

/// What the check reports, in the order asked (as given: a part named twice is said twice),
/// when this handler answers it; else why not.
fn served(args: &Check) -> Result<Vec<CheckInclude>, &'static str> {
    let ranged = args
        .range
        .as_ref()
        .is_some_and(|r| r.iter().any(|a| !a.trim().is_empty()));
    let copies = args.copies.as_ref().is_some_and(|c| !c.is_empty());
    if args.slides.is_some()
        || args.boxes.is_some()
        || args.spacing.is_some()
        || args.contrast.is_some()
        || copies
        || args.scale.is_some()
    {
        return Err(CHECK_ARGUMENTS);
    }
    if args.min_size.is_some() || args.styles.is_some() || ranged {
        return Err(CHECK_LAYOUT);
    }
    let wanted = args
        .include
        .clone()
        .unwrap_or_else(|| vec![CheckInclude::Validate, CheckInclude::Reflow, CheckInclude::Fields]);
    for part in &wanted {
        match part {
            CheckInclude::Validate | CheckInclude::Fields | CheckInclude::App | CheckInclude::Reflow => {}
            CheckInclude::Layout => return Err(CHECK_LAYOUT),
            _ => return Err(CHECK_ARGUMENTS),
        }
    }
    Ok(wanted)
}

/// `reflow`: the version's pages, coverage and first warnings, and the pages changed since
/// the document's last check (which this one now is).
fn reflow_facts(
    layout: &dyn crate::WordLayout,
    laid: &ooxml_word_pages::LaidVersion,
    version: &Version<'_>,
    owned: Owned<'_>,
    pages: Option<&[i64]>,
) -> Result<Value, ToolError> {
    let found = reflow::laid_facts(layout, laid).map_err(|e| ToolError::internal(e, "check:reflow"))?;
    let mut facts = found.facts;
    let last = owned.last_check.get();
    let aliases = owned.doc.ids().aliases();
    if let Some(since) = reflow::since(last.as_ref(), &found.summary, version.number, aliases, pages) {
        facts.insert("since_last_check".into(), since);
    }
    owned.last_check.set(Some(Seen {
        version: Some(version.number),
        summary: found.summary,
    }));
    if let Some(warnings) = found.warnings {
        facts.insert("layout_warnings".into(), warnings);
    }
    Ok(Value::Object(facts))
}

fn unported(why: &str) -> Done<Value> {
    let mut done = Done::new("");
    done.unported.push(why.to_owned());
    done
}

fn seconds(started: Instant) -> Value {
    Value::from(started.elapsed().as_secs_f64())
}

fn internal(error: &ooxml_word_edit::EditError) -> ToolError {
    ToolError::internal(error.to_string(), "check:read")
}

/// `validate`: the problems now against those the document opened with (the save gate's
/// comparison), and the first of them.
fn validate(owned: Owned<'_>) -> Result<Value, ToolError> {
    let now = problems(owned.doc).map_err(|e| internal(&e))?;
    let delta = Delta::of(owned.baseline, &now);
    let mut out = Map::new();
    out.insert("new".into(), Value::from(delta.new));
    out.insert("fixed".into(), Value::from(delta.fixed));
    out.insert("baseline".into(), Value::from(delta.baseline));
    let listed: Vec<String> = now.iter().take(LISTED).map(ToString::to_string).collect();
    out.insert("problems".into(), Value::from(listed));
    Ok(Value::Object(out))
}

/// `fields`: the count of each kind, the tables of contents that list nothing and the
/// references to missing bookmarks (each only where there is one), and what a result is.
fn fields(owned: Owned<'_>) -> Result<Value, ToolError> {
    let facts = ooxml_word_edit::field_facts(owned.doc).map_err(|e| internal(&e))?;
    let counts: Map<String, Value> = facts
        .counts
        .into_iter()
        .map(|(kind, count)| (kind, Value::from(count)))
        .collect();
    let mut out = Map::new();
    out.insert("counts".into(), Value::Object(counts));
    if !facts.empty_toc.is_empty() {
        out.insert("empty_toc".into(), Value::from(facts.empty_toc));
    }
    if !facts.missing_bookmarks.is_empty() {
        out.insert("missing_bookmarks".into(), Value::from(facts.missing_bookmarks));
    }
    out.insert("note".into(), Value::from(FIELDS_NOTE));
    Ok(Value::Object(out))
}
