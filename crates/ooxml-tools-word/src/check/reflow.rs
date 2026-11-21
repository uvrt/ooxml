//! Owns: `check`'s `reflow` (docx-agent's `_layout_facts` and `check_worker.facts_of`): a
//! version's page count, whether it was laid out whole, its coverage
//! (`layout.coverage_facts`) and its first warnings (`layout_warnings`), read from the
//! document's page engine; and `since_last_check`, the pages that changed since the
//! document's last check with `reflow` ([`LastCheck`], kept per document by the session),
//! ids followed through the version's aliases.
//!
//! Must not: lay out but through the host's engine ([`WordLayout`]).

use crate::pages::WordLayout;
use ooxml_word_pages::coverage::Coverage;
use ooxml_word_pages::summary::Summary;
use ooxml_word_pages::warnings::LayoutWarning;
use ooxml_word_pages::LaidVersion;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// The approximations, story stops and placeholders a coverage lists at most.
const LISTED: usize = 5;
/// The warnings a reflow lists at most.
const WARNINGS: usize = 10;
/// How long a listed message or warning may be, characters.
const SHORT: usize = 160;
/// The moved blocks a `since_last_check` lists at most.
const MOVED: usize = 20;

/// What a document's last check with `reflow` saw: the version and its layout summary.
#[derive(Debug, Clone)]
pub struct Seen {
    /// The version's number, where the session's check saw it (`None`: the host's check saw
    /// a version it names otherwise).
    pub version: Option<u64>,
    /// Its layout summary.
    pub summary: Arc<Summary>,
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
    pub(super) summary: Arc<Summary>,
}

/// `version`'s reflow facts, and its summary, read from `layout`'s engine.
///
/// # Errors
/// Where the version is not the engine's (a version laid out by another generation).
pub(super) fn laid_facts(layout: &dyn WordLayout, version: &LaidVersion) -> Result<Laid, String> {
    let mut engine = layout.engine().lock().unwrap_or_else(PoisonError::into_inner);
    let not_ours = || "the version laid out is not the engine's".to_owned();
    let coverage = engine.coverage(version).ok_or_else(not_ours)?;
    let summary = engine.summary(version).ok_or_else(not_ours)?;
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
    Ok(Laid {
        facts,
        warnings,
        summary,
    })
}

/// `since_last_check` of `now` (version `version`) against what `last` saw, ids followed
/// through `aliases`; `None` where no check saw a version before, or the last saw this very
/// one. `pages` (non-empty) keeps the changed pages among them.
pub(super) fn since(
    last: Option<&Seen>,
    now: &Arc<Summary>,
    version: u64,
    aliases: &std::collections::BTreeMap<String, String>,
    pages: Option<&[i64]>,
) -> Option<Value> {
    let before = last?;
    if before.version == Some(version) || Arc::ptr_eq(&before.summary, now) {
        return None;
    }
    let aliases: HashMap<String, String> = aliases.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let found = now.compare(&before.summary, &aliases);
    let changed: Vec<usize> = match pages.filter(|p| !p.is_empty()) {
        Some(pages) => found
            .changed
            .into_iter()
            .filter(|c| pages.iter().any(|p| usize::try_from(*p).ok() == Some(*c)))
            .collect(),
        None => found.changed,
    };
    let mut out = Map::new();
    out.insert("changed".into(), Value::from(changed));
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
    Some(Value::Object(out))
}
