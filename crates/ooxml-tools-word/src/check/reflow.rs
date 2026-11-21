//! Owns: `check`'s `reflow` (docx-agent's `_layout_facts` and `check_worker.facts_of`): a
//! version's page count, whether it was laid out whole, its coverage
//! (`layout.coverage_facts`) and its first warnings (`layout_warnings`), read from the
//! document's page engine; and `since_last_check`, the pages that changed since the
//! document's last check with `reflow` ([`LastCheck`], kept per document by the session),
//! ids followed through the version's aliases. A check keeps the version it saw, not its
//! summary: the next compares the two page by page ([`Summarised::compare`]), painting
//! only the pages an edit reached; a first check paints none.
//!
//! Must not: lay out but through the host's engine ([`WordLayout`]).

use crate::pages::{NotLaid, WordLayout};
use ooxml_word_pages::coverage::Coverage;
use ooxml_word_pages::summarised::Summarised;
use ooxml_word_pages::warnings::LayoutWarning;
use ooxml_word_pages::LaidVersion;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// The approximations, story stops and placeholders a coverage lists at most.
const LISTED: usize = 5;
/// The warnings a reflow lists at most.
const WARNINGS: usize = 10;
/// How long a listed message or warning may be, characters.
const SHORT: usize = 160;
/// The moved blocks a `since_last_check` lists at most.
const MOVED: usize = 20;

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
/// checks still run reads and sets it.  Beside it, what the last check that laid the
/// document out saw, with `reflow` or not (`layout`, `range`, `min_size`, `styles`): the
/// version a changing call is measured against before its change.  A check of the layout
/// alone reports no reflow, so it is not what `since_last_check` measures from (docx-agent's
/// rule), but its pages are the document's as they were.
#[derive(Debug, Default)]
pub struct LastCheck {
    reflow: Mutex<Option<Seen>>,
    laid: Mutex<Option<Seen>>,
}

impl LastCheck {
    /// What the last check with `reflow` saw, if any.
    #[must_use]
    pub fn get(&self) -> Option<Seen> {
        self.reflow.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep `seen` as what the last check with `reflow` saw (`None`: forget it); it is the
    /// last check that laid the document out too.
    pub fn set(&self, seen: Option<Seen>) {
        self.set_laid(seen.clone());
        *self.reflow.lock().unwrap_or_else(PoisonError::into_inner) = seen;
    }

    /// What the last check that laid the document out saw, with `reflow` or not, if any.
    #[must_use]
    pub fn laid(&self) -> Option<Seen> {
        self.laid.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep `seen` as what the last check that laid the document out saw (`None`: forget it).
    pub fn set_laid(&self, seen: Option<Seen>) {
        *self.laid.lock().unwrap_or_else(PoisonError::into_inner) = seen;
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
                let (reason, message) = a.listed();
                let mut entry = Map::new();
                entry.insert("page".into(), a.page.map_or(Value::Null, Value::from));
                entry.insert("reason".into(), Value::from(reason));
                entry.insert("path".into(), a.path.clone().map_or(Value::Null, Value::from));
                entry.insert("message".into(), Value::from(short(message, SHORT)));
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
    Ok(Some(Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_word_pages::coverage::glyph_approximation;
    use ooxml_word_pages::warnings::Approximation;

    fn coverage(approximations: Vec<Approximation>) -> Coverage {
        Coverage {
            complete: true,
            pages: 1,
            blocks: 1,
            substituted: Vec::new(),
            missing: Vec::new(),
            approximations,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn a_glyph_a_face_lacks_is_listed_as_docx_agent_lists_it() {
        // A face with no glyph for the character (no MS Gothic on Linux: simulated).
        let facts = coverage_facts(&coverage(vec![glyph_approximation("MS Gothic", '\u{2612}', ".notdef")]));
        assert_eq!(facts["status"], "approximate");
        assert_eq!(
            facts["approximations"][0],
            serde_json::json!({
                "page": null,
                "reason": "'MS Gothic' has no glyph for '\u{2612}': measured with its missing glyph",
                "path": null,
                "message": "",
            })
        );
        let other = coverage_facts(&coverage(vec![glyph_approximation("Calibri", 'x', "Carlito")]));
        assert_eq!(
            other["approximations"][0]["reason"],
            "'Calibri' has no glyph for 'x': measured with 'Carlito'"
        );
    }

    #[test]
    fn a_layout_approximation_is_listed_by_the_reason_its_code_gives() {
        let approximation = |code: &str| Approximation {
            code: code.to_owned(),
            message: "a frame (w:framePr) is  laid out\nin the flow".to_owned(),
            page: Some(2),
            path: Some("w:body/w:p[3]".to_owned()),
        };
        let facts = coverage_facts(&coverage(vec![
            approximation("layout-approximate:frame"),
            approximation("odd"),
        ]));
        assert_eq!(
            facts["approximations"][0],
            serde_json::json!({
                "page": 2,
                "reason": "frame",
                "path": "w:body/w:p[3]",
                "message": "a frame (w:framePr) is laid out in the flow",
            })
        );
        assert_eq!(facts["approximations"][1]["reason"], "odd");
    }
}
