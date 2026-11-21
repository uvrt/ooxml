//! `check` for Word documents (lanes P5a, P5b; docx-agent's `tools/shared.py` `check`): a
//! version's facts -- its validation problems against those it opened with (`validate`),
//! its pages since the last check (`reflow`, [`reflow`]) and its fields (`fields`) -- each
//! part's seconds in `timing`, as docx-agent words them.
//!
//! Owns: the tool's definition ([`args`]), the facts read from the document alone, and
//! those read from its pages as the host's engine lays them out ([`crate::WordLayout`]).
//!
//! With the layout, its facts (`layout`, [`layout`]): those read from the pages and from
//! the document alone, on the pages asked for, about the blocks a `range` names, with each
//! range's extent and keeps (`range`), the minimum text size (`min_size`) and the styles
//! allowed (`styles`).
//!
//! Must not: lay out but through the host's engine. Copies, a scale and a deck's arguments
//! are not given here yet: such a call is answered `unported`, and the host answers it; so
//! is a check needing the layout without the host's layout, or of a version its engine
//! does not lay out, a range refused (the host words why), a `min_size` that is not one,
//! and `pages` holding none of the ranges' blocks (the host warns of it).

mod args;
mod facts;
mod laid_facts;
mod laid_tables;
mod layout;
mod placed;
mod ranges;
mod reflow;

pub use args::{Check, CheckCopy, CheckInclude, CheckScale, CopyLists, CopyReference, ScaleShow, ScaleStep, StepText};
pub use facts::{
    direct_formatting_json, empty_run_json, grid_mismatch_json, lost_cell_json, lost_original_json, row_unlike_json,
    skipped_level_json, spacing_problem_json, style_use_json, text_language_json,
};
pub use laid_facts::{
    keep_at_foot_json, kept_together_json, minimum_of, page_facts_json, parted_json, small_text_json,
    smallest_text_json, spacer_json,
};
pub use laid_tables::{column_fit_json, past_margins_json, split_row_json, uncertain_json, unrepeated_json};
pub use ranges::{ranges_json, Trials, TRIALS, TRIAL_SECONDS};
pub use reflow::{LastCheck, Seen};

use crate::describe::cancelled;
use crate::pages::NotLaid;
use crate::version::{Owned, Version};
use ooxml_base::Cancel;
use ooxml_scene::python_g;
use ooxml_tools::{ArgEnum, Done, ToolError};
use ooxml_word_edit::validate::{problems, problems_in, Delta};
use ooxml_word_pages::layout_facts::sizes::Minimum;
use ooxml_word_pages::layout_facts::{Facts, Scope};
use ooxml_word_pages::summarised::Summarised;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
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
/// What `unported` says of a check whose `range` is refused (the host words why).
pub const CHECK_RANGE: &str = "check: a range refused";
/// What `unported` says of a check whose `pages` hold none of its ranges' blocks.
pub const CHECK_PAGES_OUTSIDE: &str = "check: pages outside the ranges";
/// The ranges a check takes at most (docx-agent's `MAX_RANGES`).
pub const MAX_RANGES: usize = 20;
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
    let reflow_wanted = wanted.contains(&CheckInclude::Reflow);
    let layout_wanted = wanted.contains(&CheckInclude::Layout);
    let layout = match (reflow_wanted || layout_wanted, owned.layout) {
        (true, None) => return Ok(unported(CHECK_NO_LAYOUT)),
        (true, Some(layout)) => Some(layout),
        (false, _) => None,
    };
    let minimum = match args.min_size.map(Value::from).as_ref().map(minimum_of).transpose() {
        Ok(minimum) => minimum.flatten(),
        Err(_) => return Ok(unported(CHECK_ARGUMENTS)),
    };
    let mut data = Map::new();
    let mut timing = Map::new();
    let ranges = ranges_of(args.range.as_deref(), version);
    if ranges.len() > MAX_RANGES {
        return Err(ToolError::failure(
            ooxml_tools::Failure::Limit,
            format!(
                "{} ranges: at most {MAX_RANGES} per check; a span p:A..t:B covers many blocks",
                ranges.len()
            ),
        )
        .at("range"));
    }
    let docx = if layout_wanted { Some(version.docx()?) } else { None };
    let scope = match (&docx, ranges.is_empty()) {
        (Some(docx), false) => {
            let started = Instant::now();
            let Ok(scope) = Scope::of(docx, &ranges) else {
                return Ok(unported(CHECK_RANGE));
            };
            timing.insert("resolve".into(), seconds(started));
            Some(scope)
        }
        _ => None,
    };
    if wanted.contains(&CheckInclude::Validate) {
        let started = Instant::now();
        data.insert("validate".into(), validate(owned, scope.as_ref())?);
        timing.insert("validate".into(), seconds(started));
    }
    cancel.check().map_err(cancelled)?;
    if let Some(layout) = layout {
        let checking = Checking {
            args,
            version,
            owned,
            scope: scope.as_ref(),
            docx: docx.as_deref(),
            minimum: minimum.as_ref(),
            cancel,
        };
        if let Some(why) = with_layout(
            &checking,
            layout,
            (reflow_wanted, layout_wanted),
            &mut data,
            &mut timing,
        )? {
            return Ok(unported(why));
        }
    }
    finish(owned, &wanted, scope.as_ref(), data, timing)
}

/// What a check with the layout reads, besides the layout.
struct Checking<'c, 'v> {
    args: &'c Check,
    version: &'c Version<'v>,
    owned: Owned<'v>,
    scope: Option<&'c Scope>,
    docx: Option<&'c ooxml_word_model::docx::Docx>,
    minimum: Option<&'c Minimum>,
    cancel: &'c dyn Cancel,
}

/// `reflow`, `layout` and `range` of the version laid out by `layout` into `data`
/// (`wanted`: reflow, layout), their seconds into `timing`; else why the host answers.
fn with_layout(
    c: &Checking<'_, '_>,
    layout: &dyn crate::WordLayout,
    (reflow_wanted, layout_wanted): (bool, bool),
    data: &mut Map<String, Value>,
    timing: &mut Map<String, Value>,
) -> Result<Option<&'static str>, ToolError> {
    let started = Instant::now();
    let last = layout
        .engine()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last()
        .cloned();
    let stop = c.owned.layout_limit.map_or(c.cancel, |limit| limit.cancel);
    let laid = layout.laid(c.version.package, stop);
    // `layout`: the time laying the version out, where the engine had not laid it out last
    if !matches!((&last, &laid), (Some(last), Ok(laid)) if Arc::ptr_eq(last, laid)) {
        timing.insert("layout".into(), seconds(started));
    }
    let started = Instant::now();
    let laid = match laid {
        Ok(laid) => laid,
        Err(NotLaid::Other(_)) => return Ok(Some(CHECK_NOT_LAID_OUT)),
        Err(NotLaid::Stopped) => {
            // The call's own stop ends it; the layout's limit passed is said, and the
            // check goes on (docx-agent's `reflow.error`), with the facts read from the
            // document alone.
            c.cancel.check().map_err(cancelled)?;
            not_finished(c, (reflow_wanted, layout_wanted), data)?;
            timing.insert("facts".into(), seconds(started));
            return Ok(None);
        }
    };
    // The ranges' pages, among those asked for: the pages the facts are read on.
    let on: Option<BTreeSet<usize>> = c.scope.map(|s| s.pages(&laid).into_iter().flatten().collect());
    let Some(pages) = pages_read(c.args.pages.as_deref(), on.as_ref()) else {
        return Ok(Some(CHECK_PAGES_OUTSIDE));
    };
    let mut reflow = None;
    if reflow_wanted {
        let Ok(mut facts) = reflow_facts(layout, &laid, c.version, c.owned, c.args.pages.as_deref()) else {
            return Ok(Some(CHECK_NOT_LAID_OUT));
        };
        if let (Some(pages), Some(Value::Array(changed))) = (
            pages.as_ref().filter(|_| on.is_some()),
            facts.pointer_mut("/since_last_check/changed"),
        ) {
            changed.retain(|n| n.as_u64().is_some_and(|n| pages.iter().any(|p| *p as u64 == n)));
        }
        reflow = Some(facts);
    }
    if let (true, Some(docx)) = (layout_wanted, c.docx) {
        if !reflow_wanted {
            // Laid out, no reflow: kept as the version's pages a changing call measures from.
            c.owned.last_check.set_laid(Some(Seen {
                version: Some(c.version.number),
                pages: Summarised::watched(Arc::clone(&laid), Arc::clone(layout.engine())),
            }));
        }
        let (found, range) = layout_facts(c, layout, &laid, docx, pages.as_deref(), stop)?;
        if let Some(range) = range {
            data.insert("range".into(), range);
        }
        if let Some(reflow) = reflow.take() {
            data.insert("reflow".into(), reflow);
        }
        data.insert("layout".into(), found);
    }
    if let Some(reflow) = reflow {
        data.insert("reflow".into(), reflow);
    }
    timing.insert("facts".into(), seconds(started));
    Ok(None)
}

/// The pages a check reads its facts on: those `asked` for, and of them the ranges' pages
/// (`on`) where scoped (`Some(None)`: every page); `None` where none of `asked` holds any of
/// the ranges' blocks.
fn pages_read(asked: Option<&[i64]>, on: Option<&BTreeSet<usize>>) -> Option<Option<Vec<usize>>> {
    let asked: Option<Vec<usize>> = asked.map(|pages| pages.iter().filter_map(|p| usize::try_from(*p).ok()).collect());
    match (on, asked) {
        (Some(on), None) => Some(Some(on.iter().copied().collect())),
        (Some(on), Some(asked)) => {
            let kept: Vec<usize> = on.iter().copied().filter(|p| asked.contains(p)).collect();
            (!kept.is_empty()).then_some(Some(kept))
        }
        (None, asked) => Some(asked),
    }
}

/// `layout` and `range` of version `laid` (read as `docx`) on `pages`, its keeps tried by
/// `layout`'s engine, stopped by `stop`.
fn layout_facts(
    c: &Checking<'_, '_>,
    layout: &dyn crate::WordLayout,
    laid: &Arc<ooxml_word_pages::LaidVersion>,
    docx: &ooxml_word_model::docx::Docx,
    pages: Option<&[usize]>,
    stop: &dyn Cancel,
) -> Result<(Value, Option<Value>), ToolError> {
    let painted = layout::Painted {
        engine: layout.engine(),
        version: Arc::clone(laid),
    };
    let approximations = layout
        .engine()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .coverage(laid)
        .map(|found| found.approximations)
        .unwrap_or_default();
    let mut facts = Facts::new(laid, docx, pages, &painted);
    if let Some(scope) = c.scope.cloned() {
        facts = facts.scoped(scope);
    }
    let asked = layout::Asked {
        minimum: c.minimum,
        styles: c.args.styles.as_deref(),
    };
    layout::laid(&facts, c.owned, &asked, &approximations, (layout.engine(), stop))
}

/// What a check says where laying the version out did not finish within its limit: each
/// range without pages, the facts read from the document alone, and the reflow's error.
fn not_finished(
    c: &Checking<'_, '_>,
    (reflow_wanted, layout_wanted): (bool, bool),
    data: &mut Map<String, Value>,
) -> Result<(), ToolError> {
    let reflow = timed_out(c.owned.layout_limit.map(|limit| limit.timeout));
    if let Some(scope) = c.scope {
        data.insert("range".into(), layout::unlaid_ranges(scope));
    }
    if layout_wanted {
        let error = reflow.get("error").cloned().unwrap_or(Value::Null);
        data.insert("layout".into(), layout::unlaid(c.owned, c.scope, error)?);
    }
    if reflow_wanted {
        data.insert("reflow".into(), reflow);
    }
    Ok(())
}

/// The check's result: `fields` where asked, `app_findings`, `timing`, and the summary
/// naming what it checked (and in how many ranges).
fn finish(
    owned: Owned<'_>,
    wanted: &[CheckInclude],
    scope: Option<&Scope>,
    mut data: Map<String, Value>,
    mut timing: Map<String, Value>,
) -> Result<Done<Value>, ToolError> {
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
    let mut summary = format!("Checked {}", named.join(", "));
    if let Some(scope) = scope {
        summary.push_str(&format!(" in {} range(s)", scope.ranges.len()));
    }
    let mut done = Done::new(summary);
    done.data = Some(Value::Object(data));
    Ok(done)
}

/// `range` as a list of addresses (docx-agent's `ranges_of`): each stripped, blanks and
/// repeats left out, each as it names a block now.
fn ranges_of(range: Option<&[String]>, version: &Version<'_>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for address in range.unwrap_or_default() {
        let address = address.trim();
        if !address.is_empty() && !out.iter().any(|a| a == address) {
            out.push(address.to_owned());
        }
    }
    out.iter().map(|a| version.follow(a)).collect()
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
    let mut wanted = args
        .include
        .clone()
        .unwrap_or_else(|| vec![CheckInclude::Validate, CheckInclude::Reflow, CheckInclude::Fields]);
    for part in &wanted {
        match part {
            CheckInclude::Validate
            | CheckInclude::Fields
            | CheckInclude::App
            | CheckInclude::Reflow
            | CheckInclude::Layout => {}
            _ => return Err(CHECK_ARGUMENTS),
        }
    }
    // What `min_size`, `styles` and a range measure: the layout's facts (round 16, F8).
    if (args.min_size.is_some() || args.styles.is_some() || ranged) && !wanted.contains(&CheckInclude::Layout) {
        wanted.push(CheckInclude::Layout);
    }
    Ok(wanted)
}

/// `reflow`: the version's pages, coverage and first warnings, and the pages changed since
/// the document's last check (which this one now is).
fn reflow_facts(
    layout: &dyn crate::WordLayout,
    laid: &Arc<ooxml_word_pages::LaidVersion>,
    version: &Version<'_>,
    owned: Owned<'_>,
    pages: Option<&[i64]>,
) -> Result<Value, NotLaid> {
    let found = reflow::laid_facts(layout, laid)?;
    let mut facts = found.facts;
    let last = owned.last_check.get();
    let aliases = owned.doc.ids().aliases();
    let now = Summarised::watched(Arc::clone(laid), Arc::clone(layout.engine()));
    if let Some(since) = reflow::since(last.as_ref(), &now, version.number, aliases, pages).map_err(NotLaid::Other)? {
        facts.insert("since_last_check".into(), since);
    }
    owned.last_check.set(Some(Seen {
        version: Some(version.number),
        pages: now,
    }));
    if let Some(warnings) = found.warnings {
        facts.insert("layout_warnings".into(), warnings);
    }
    Ok(Value::Object(facts))
}

/// `reflow` where laying the version out passed the call's limit on it, as docx-agent says
/// it (`f"timeout: laying the document out did not finish within {timeout:g} s"`).
fn timed_out(timeout: Option<std::time::Duration>) -> Value {
    let within = timeout.map_or_else(String::new, |t| format!(" within {} s", python_g(t.as_secs_f64())));
    let mut out = Map::new();
    out.insert(
        "error".into(),
        Value::from(format!("timeout: laying the document out did not finish{within}")),
    );
    Value::Object(out)
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
/// comparison), and the first of them; in `scope`'s blocks only where scoped (with how
/// many are elsewhere, and only its new ones).
fn validate(owned: Owned<'_>, scope: Option<&Scope>) -> Result<Value, ToolError> {
    let now = problems(owned.doc).map_err(|e| internal(&e))?;
    let delta = Delta::of(owned.baseline, &now);
    let mut out = Map::new();
    let Some(scope) = scope else {
        out.insert("new".into(), Value::from(delta.new));
        out.insert("fixed".into(), Value::from(delta.fixed));
        out.insert("baseline".into(), Value::from(delta.baseline));
        let listed: Vec<String> = now.iter().take(LISTED).map(ToString::to_string).collect();
        out.insert("problems".into(), Value::from(listed));
        return Ok(Value::Object(out));
    };
    let docx = owned.doc.docx().map_err(|e| internal(&e))?;
    let roots: Vec<Arc<ooxml_xml::Element>> = docx
        .body()
        .map(|body| {
            scope
                .roots
                .iter()
                .filter_map(|place| ooxml_word_model::docx::at_path(body.root(), place).cloned())
                .collect()
        })
        .unwrap_or_default();
    let scoped = problems_in(owned.doc, &now, &docx.main, &roots);
    let inside: Vec<String> = scoped.inside.iter().map(ToString::to_string).collect();
    let new: Vec<String> = delta.new.into_iter().filter(|p| inside.contains(p)).collect();
    out.insert("new".into(), Value::from(new));
    out.insert("fixed".into(), Value::from(delta.fixed));
    out.insert("baseline".into(), Value::from(delta.baseline));
    out.insert(
        "problems".into(),
        Value::from(inside.into_iter().take(LISTED).collect::<Vec<_>>()),
    );
    out.insert("elsewhere".into(), Value::from(scoped.elsewhere));
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
