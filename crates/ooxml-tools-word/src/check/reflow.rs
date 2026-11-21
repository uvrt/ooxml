//! Owns: `check`'s `reflow` (docx-agent's `_layout_facts` and `check_worker.facts_of`): a
//! version's page count, whether it was laid out whole, its coverage
//! (`layout.coverage_facts`) and its first warnings (`layout_warnings`), read from the
//! document's page engine; and `since_last_check`, the pages that changed since the
//! document's last check with `reflow` ([`LastCheck`], kept per document by the session),
//! ids followed through the version's aliases. A check keeps the version it saw, not its
//! summary: the next compares the two page by page ([`Summarised::compare`]), painting
//! only the pages an edit reached; a first check paints none. Also owns the reflow a
//! changing call's checks report ([`reflow_after_edit`], docx-agent's `_base.checks`): the
//! version before the call's first edit, where a check saw it or the engine laid it out
//! last, against the version the call made.
//!
//! Must not: lay out but through the host's engine ([`WordLayout`]).

use crate::pages::{LayoutLimit, NotLaid, WordLayout};
use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_word_pages::coverage::Coverage;
use ooxml_word_pages::summarised::Summarised;
use ooxml_word_pages::summary::Reflow;
use ooxml_word_pages::warnings::LayoutWarning;
use ooxml_word_pages::{LaidVersion, PagesError};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

/// The approximations, story stops and placeholders a coverage lists at most.
const LISTED: usize = 5;
/// The warnings a reflow lists at most.
const WARNINGS: usize = 10;
/// How long a listed message or warning may be, characters.
const SHORT: usize = 160;
/// The moved blocks a `since_last_check` lists at most.
const MOVED: usize = 20;
/// The most pages a changing call's checks compare (docx-agent's `REFLOW_PAGES`): beyond,
/// the reflow says [`STALE`].
pub const REFLOW_PAGES: usize = 20;
/// The reflow of a changing call whose version before had more than [`REFLOW_PAGES`].
pub const STALE: &str = "stale: call check";

/// What a document's last check with `reflow` saw: the version and its pages (the
/// version laid out, or a summary the host's check made).
#[derive(Debug, Clone)]
pub struct Seen {
    /// The version's number, where the session's check saw it (`None`: the host's check saw
    /// a version it names otherwise).
    pub version: Option<u64>,
    /// Its pages, summarised when compared.
    pub pages: Summarised,
}

/// What a document's last check with `reflow` saw, which the next measures
/// `since_last_check` from; the session keeps one per document, and a host whose own
/// checks still run reads and sets it.
#[derive(Debug, Default)]
pub struct LastCheck(Mutex<Option<Seen>>);

impl LastCheck {
    /// What the last check saw, if any.
    #[must_use]
    pub fn get(&self) -> Option<Seen> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep `seen` (`None`: forget it).
    pub fn set(&self, seen: Option<Seen>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = seen;
    }
}

/// `text` with its white space runs made one space, cut to `limit` characters with an
/// ellipsis (docx-agent's `short`).
fn short(text: &str, limit: usize) -> String {
    let joined = text
        .split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut out: String = joined.chars().take(limit.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// A warning as docx2svg's `Warning.__str__` writes it.
fn warning_text(warning: &LayoutWarning) -> String {
    match warning.page.filter(|p| *p != 0) {
        Some(page) => format!("[{}] (page {page}) {}", warning.code, warning.message),
        None => format!("[{}] {}", warning.code, warning.message),
    }
}

/// `layout.coverage_facts()` of a version laid out in Rust.
fn coverage_facts(coverage: &Coverage) -> Value {
    let mut out = Map::new();
    let status = if !coverage.complete {
        "partial"
    } else if coverage.approximations.is_empty() {
        "complete"
    } else {
        "approximate"
    };
    out.insert("complete".into(), Value::from(coverage.complete));
    out.insert("status".into(), Value::from(status));
    out.insert("pages".into(), Value::from(coverage.pages));
    out.insert(
        "blocks_laid_out".into(),
        Value::from(vec![coverage.blocks, coverage.blocks]),
    );
    if !coverage.substituted.is_empty() {
        let named: Vec<String> = coverage
            .substituted
            .iter()
            .map(|s| {
                let approximate = if s.metric_compatible { "" } else { " (approximate)" };
                format!("{} -> {}{approximate}", s.family, s.substitute)
            })
            .collect();
        out.insert("substituted_fonts".into(), Value::from(named));
    }
    if !coverage.missing.is_empty() {
        out.insert("missing_fonts".into(), Value::from(coverage.missing.clone()));
    }
    if !coverage.approximations.is_empty() {
        let listed: Vec<Value> = coverage
            .approximations
            .iter()
            .take(LISTED)
            .map(|a| {
                let reason = a.code.split_once(':').map_or("", |(_, r)| r);
                let mut entry = Map::new();
                entry.insert("page".into(), a.page.map_or(Value::Null, Value::from));
                entry.insert("reason".into(), Value::from(reason));
                entry.insert("path".into(), a.path.clone().map_or(Value::Null, Value::from));
                entry.insert("message".into(), Value::from(short(&a.message, SHORT)));
                Value::Object(entry)
            })
            .collect();
        out.insert("approximations".into(), Value::from(listed));
        if coverage.approximations.len() > LISTED {
            out.insert(
                "approximations_total".into(),
                Value::from(coverage.approximations.len()),
            );
        }
    }
    Value::Object(out)
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
    let mut engine = layout.engine().lock().unwrap_or_else(PoisonError::into_inner);
    let not_ours = || NotLaid::Other("the version laid out is not the engine's".to_owned());
    let coverage = engine.coverage(version).ok_or_else(not_ours)?;
    drop(engine);
    let mut facts = Map::new();
    facts.insert("pages".into(), Value::from(version.page_count()));
    facts.insert("complete".into(), Value::from(true));
    facts.insert("coverage".into(), coverage_facts(&coverage));
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
    if before.version == Some(version) || before.pages.is(now) {
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
fn reflow_json(found: Reflow) -> Value {
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

/// The version a changing call's first edit was applied to: its number and package.
#[derive(Clone, Copy)]
pub struct EditedFrom<'a> {
    /// Its number.
    pub version: u64,
    /// Its package.
    pub package: &'a Package,
}

/// The reflow a changing call's checks report (docx-agent's `_base.checks`), measured from
/// `before` -- where the document's last check saw it (`last`) or `layout`'s engine laid it
/// out last, else `None`: no reflow is measured -- to `after` (the version the call's last
/// edit made), ids followed through `aliases`. More than [`REFLOW_PAGES`] before: [`STALE`].
/// `after` is laid out under the call's `limit` on laying out (else until `cancel`): a stop
/// is said as docx-agent's deadline says it. What the last check saw is left as it was.
///
/// The engine is never waited for: where another call lays a version out on it, no reflow is
/// measured (writers are not serialized behind another's relayout); else it is held for the
/// whole measure (the version before kept comparable, the version after laid out, the two
/// compared page by page).
#[must_use]
pub fn reflow_after_edit(
    layout: &dyn WordLayout,
    last: &LastCheck,
    before: EditedFrom<'_>,
    after: &Package,
    aliases: &std::collections::BTreeMap<String, String>,
    (limit, cancel): (Option<LayoutLimit<'_>>, &dyn Cancel),
) -> Option<Value> {
    let seen = last.get().filter(|seen| seen.version == Some(before.version));
    let mut engine = match layout.engine().try_lock() {
        Ok(engine) => engine,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return None,
    };
    let was = match seen.map(|s| s.pages) {
        Some(Summarised::Laid {
            version,
            engine: theirs,
        }) if Arc::ptr_eq(&theirs, layout.engine()) => Was::Here(version),
        Some(other) => Was::Elsewhere(other),
        None => Was::Here(engine.laid_last(before.package)?),
    };
    if was.count() > REFLOW_PAGES {
        return Some(Value::from(STALE));
    }
    if let Was::Here(version) = &was {
        engine.watch(version);
    }
    let now = match engine.lay_out_until(after, limit.map_or(cancel, |l| l.cancel)) {
        Ok(Ok(now)) => now,
        Err(PagesError::Stopped) => {
            let within = limit.map_or_else(String::new, |l| {
                format!(" within {} s", ooxml_scene::python_g(l.timeout.as_secs_f64()))
            });
            return Some(Value::from(format!(
                "timeout: laying the document out did not finish{within}"
            )));
        }
        Ok(Err(_)) | Err(_) => return None,
    };
    let aliases: HashMap<String, String> = aliases.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let found = match was {
        Was::Here(version) => engine.compare(&now, &version, &aliases),
        Was::Elsewhere(seen) => {
            drop(engine);
            Summarised::watched(now, Arc::clone(layout.engine())).compare(&seen, &aliases)
        }
    };
    found.map(reflow_json)
}

/// The version before a call's edit, as [`reflow_after_edit`] compares with it: laid out by
/// the call's engine, or what a check saw elsewhere (a summary, another engine's version).
enum Was {
    Here(Arc<LaidVersion>),
    Elsewhere(Summarised),
}

impl Was {
    fn count(&self) -> usize {
        match self {
            Was::Here(version) => version.page_count(),
            Was::Elsewhere(seen) => seen.count(),
        }
    }
}
