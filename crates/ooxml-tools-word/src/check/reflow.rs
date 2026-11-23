//! Owns: `check`'s `reflow` (docx-agent's `_layout_facts` and `check_worker.facts_of`): a
//! version's page count, whether it was laid out whole, its coverage
//! (`layout.coverage_facts`) and its first warnings (`layout_warnings`), read from the
//! document's page engine; and `since_last_check`, the pages that changed since the
//! document's last check with `reflow` ([`LastCheck`], kept per document by the session),
//! ids followed through the version's aliases. A check keeps the version it saw, not its
//! summary: the next compares the two page by page ([`Summarised::compare`]), painting
//! only the pages an edit reached; a first check paints none. A reflow is written as
//! docx-agent's `reflow_json` writes it (also for an edit's checks: [`super::edit_reflow`]).
//!
//! Must not: lay out but through the host's engine ([`WordLayout`]).

use crate::coverage::{coverage_facts, short, SHORT};
use crate::pages::{NotLaid, WordLayout};
use ooxml_word_pages::summarised::Summarised;
use ooxml_word_pages::summary::Reflow;
use ooxml_word_pages::warnings::LayoutWarning;
use ooxml_word_pages::LaidVersion;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// The warnings a reflow lists at most.
const WARNINGS: usize = 10;
/// The moved blocks a `since_last_check` lists at most.
const MOVED: usize = 20;

/// What a document's last check saw: the version and its pages laid out.
#[derive(Debug, Clone)]
pub struct Seen {
    /// The version's number.
    pub version: u64,
    /// Its pages, summarised when compared.
    pub pages: Summarised,
}

/// What a document's last check with `reflow` saw, which the next measures
/// `since_last_check` from; the session keeps one per document.  Beside it, what the last
/// check that laid the document out saw, with `reflow` or not (`layout`, `range`,
/// `min_size`, `styles`): the version a changing call is measured against before its
/// change.  A check of the layout alone reports no reflow, so it is not what
/// `since_last_check` measures from (docx-agent's rule), but its pages are the document's as
/// they were.  The two are kept under one lock, so concurrent checks never leave them
/// disagreeing.
#[derive(Debug, Default)]
pub struct LastCheck {
    kept: Mutex<Kept>,
    copies: super::copies::CopyCache,
}

/// The two slots of a [`LastCheck`], changed together.
#[derive(Debug, Default)]
struct Kept {
    /// What the last check with `reflow` saw.
    reflow: Option<Seen>,
    /// What the last check that laid the document out saw.
    laid: Option<Seen>,
}

impl LastCheck {
    /// The copies' comparisons the document's checks keep.
    pub(crate) fn copies(&self) -> &super::copies::CopyCache {
        &self.copies
    }

    /// The two slots, locked. A panic while they were held leaves each slot whole (each is
    /// written by one assignment), so a poisoned lock is taken as it is.
    fn kept(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the last check with `reflow` saw, if any.
    #[must_use]
    pub fn get(&self) -> Option<Seen> {
        self.kept().reflow.clone()
    }

    /// Keep `seen` as what the last check with `reflow` saw -- the last check that laid the
    /// document out too -- and what was kept before it, in one step.
    pub fn replace(&self, seen: Seen) -> Option<Seen> {
        let mut kept = self.kept();
        kept.laid = Some(seen.clone());
        kept.reflow.replace(seen)
    }

    /// What the last check that laid the document out saw, with `reflow` or not, if any.
    #[must_use]
    pub fn laid(&self) -> Option<Seen> {
        self.kept().laid.clone()
    }

    /// Keep `seen` as what the last check that laid the document out saw.
    pub fn set_laid(&self, seen: Seen) {
        self.kept().laid = Some(seen);
    }
}

/// A warning as docx2svg's `Warning.__str__` writes it.
fn warning_text(warning: &LayoutWarning) -> String {
    match warning.page.filter(|p| *p != 0) {
        Some(page) => format!("[{}] (page {page}) {}", warning.code, warning.message),
        None => format!("[{}] {}", warning.code, warning.message),
    }
}

/// The facts of a version laid out: `pages`, `complete`, `coverage`, and its first
/// warnings (`None`: it says none).
pub(super) struct Laid {
    pub(super) facts: Map<String, Value>,
    pub(super) warnings: Option<Value>,
}

/// `version`'s reflow facts, read from `layout`'s engine (no page painted for them but
/// where its coverage needs it). Only laying the version out is held to the call's limit on
/// laying out (docx-agent's deadline holds its layout, not the facts read from it).
///
/// # Errors
/// Where the version is not the engine's (a version laid out by another generation).
pub(super) fn laid_facts(layout: &dyn WordLayout, version: &LaidVersion) -> Result<Laid, NotLaid> {
    let mut engine = ooxml_word_pages::lock_engine(layout.engine());
    let not_ours = || NotLaid::NotTheEngines;
    let coverage = engine.coverage(version).ok_or_else(not_ours)?;
    drop(engine);
    let mut facts = Map::new();
    facts.insert("pages".into(), Value::from(version.page_count()));
    facts.insert("complete".into(), Value::from(true));
    facts.insert(
        "coverage".into(),
        coverage_facts(&coverage, |path| version.id_of(path).map(str::to_owned)),
    );
    let warnings = (!coverage.warnings.is_empty()).then(|| {
        let listed: Vec<String> = coverage
            .warnings
            .iter()
            .take(WARNINGS)
            .map(|w| short(&warning_text(w), SHORT))
            .collect();
        Value::from(listed)
    });
    Ok(Laid { facts, warnings })
}

/// `since_last_check` of `now` (version `version`) against what `last` saw, ids followed
/// through `aliases`; `None` where no check saw a version before, or the last saw this very
/// one. `pages` (non-empty) keeps the changed pages among them.
///
/// # Errors
/// Where either version can no longer be digested (its engine no longer paints it).
pub(super) fn since(
    last: Option<&Seen>,
    now: &Summarised,
    version: u64,
    aliases: &std::collections::BTreeMap<String, String>,
    pages: Option<&[i64]>,
) -> Result<Option<Value>, String> {
    let Some(before) = last else {
        return Ok(None);
    };
    if before.version == version || before.pages.is(now) {
        return Ok(None);
    }
    let aliases: HashMap<String, String> = aliases.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let found = now
        .compare(&before.pages, &aliases)
        .ok_or_else(|| "the version last checked is no longer laid out by the engine".to_owned())?;
    let changed: Vec<usize> = match pages.filter(|p| !p.is_empty()) {
        Some(pages) => found
            .changed
            .iter()
            .copied()
            .filter(|c| pages.iter().any(|p| usize::try_from(*p).ok() == Some(*c)))
            .collect(),
        None => found.changed.clone(),
    };
    Ok(Some(reflow_json(Reflow { changed, ..found })))
}

/// A reflow as docx-agent's `reflow_json` writes it: the pages changed, the page counts,
/// and where present the first blocks that moved and the pages not compared.
pub(super) fn reflow_json(found: Reflow) -> Value {
    let mut out = Map::new();
    out.insert("changed".into(), Value::from(found.changed));
    out.insert(
        "pages".into(),
        Value::from(vec![found.page_count.0, found.page_count.1]),
    );
    if !found.moved.is_empty() {
        let moved: Map<String, Value> = found
            .moved
            .into_iter()
            .take(MOVED)
            .map(|(id, (was, now))| (id, Value::from(vec![was, now])))
            .collect();
        out.insert("moved".into(), Value::Object(moved));
    }
    if !found.unknown.is_empty() {
        out.insert("unknown".into(), Value::from(found.unknown));
    }
    Value::Object(out)
}
