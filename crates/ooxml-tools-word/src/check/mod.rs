//! `check` for Word documents (lane P5a; docx-agent's `tools/shared.py` `check`): a
//! version's facts -- its validation problems against those it opened with (`validate`) and
//! its fields (`fields`) -- each part's seconds in `timing`, as docx-agent words them.
//!
//! Owns: the tool's definition ([`args`]) and the facts read from the document alone.
//!
//! Must not: lay out. What needs the layout (`reflow`, `layout`, a `range`'s or `pages`'
//! facts, `min_size`, `styles`), copies, a scale and a deck's arguments are not given here
//! yet: such a call is answered `unported`, and the host answers it.

mod args;
mod facts;

pub use args::{Check, CheckCopy, CheckInclude, CheckScale, CopyLists, CopyReference, ScaleShow, ScaleStep, StepText};
pub use facts::{empty_run_json, grid_mismatch_json, lost_cell_json, skipped_level_json};

use crate::describe::cancelled;
use crate::version::{Owned, Version};
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, ToolError};
use ooxml_word_edit::validate::{problems, Delta};
use serde_json::{Map, Value};
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
    if wanted.contains(&CheckInclude::Validate) {
        let started = Instant::now();
        data.insert("validate".into(), validate(owned)?);
        timing.insert("validate".into(), seconds(started));
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
    if args.pages.is_some() || args.min_size.is_some() || args.styles.is_some() || ranged {
        return Err(CHECK_LAYOUT);
    }
    let Some(wanted) = &args.include else {
        return Err(CHECK_LAYOUT); // the default reports reflow
    };
    for part in wanted {
        match part {
            CheckInclude::Validate | CheckInclude::Fields | CheckInclude::App => {}
            CheckInclude::Reflow | CheckInclude::Layout => return Err(CHECK_LAYOUT),
            _ => return Err(CHECK_ARGUMENTS),
        }
    }
    Ok(wanted.clone())
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

/// Registers `check` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.read::<Check>(check);
}
