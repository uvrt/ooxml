//! Owns: each range of `check(range=[...])` as docx-agent reports it (`scoped.range_pages`
//! with its extent, `range_keeps`, round 35 R26, round 36 R30; `keep_trials.keep_effects`,
//! round 40): its address, block count and pages; how much of each page it takes
//! (`extent`, `height`); for a range of whole blocks whether it is on one page and kept so,
//! its page breaks and the call that keeps it (`fix`) or that it cannot be kept
//! (`taller_than_page`); and what that keep -- or releasing a keep that starts a page --
//! does, from a trial layout of the document with that one change (`if_kept`,
//! `if_released`), at most [`TRIALS`] a check, none started past [`TRIAL_SECONDS`]
//! (`effect_left_out` says so).
//!
//! A trial is laid out by the document's engine from the version checked
//! ([`PageEngine::trial`]): incrementally, kept nowhere.
//!
//! Must not: find the layout facts (`ooxml_word_pages::layout_facts` does), or change the
//! document (a trial is a package of its own).

use ooxml_base::Cancel;
use ooxml_scene::python_g;
use ooxml_word_edit::{keep_writes, kept_blocks, kept_package, WordDoc};
use ooxml_word_layout::facts::PageFacts;
use ooxml_word_model::docx::Docx;
use ooxml_word_pages::layout_facts::{Break, Facts, KeepEffect, Range, Side};
use ooxml_word_pages::Laid;
use ooxml_word_pages::PageEngine;
use serde_json::{json, Map, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Trial layouts one check makes at most (`keep_trials.TRIALS`).
pub const TRIALS: usize = 4;
/// Seconds of trial layouts one check spends at most: none is started past them
/// (`keep_trials.TRIAL_SECONDS`).
pub const TRIAL_SECONDS: f64 = 6.0;
/// Pages a range's `extent` lists (`EXTENT_PAGES`); the rest are counted.
const EXTENT_PAGES: usize = 12;
/// Page breaks a range's `breaks` lists (`BREAKS_LISTED`); the rest are counted.
const BREAKS_LISTED: usize = 10;

/// Where trials are laid out: the engine that laid the version checked out.
pub struct Trials<'e> {
    /// The engine.
    pub engine: &'e Mutex<PageEngine>,
    /// Trials at most.
    pub trials: usize,
    /// Seconds at most (none started past them).
    pub seconds: f64,
    /// Stops a trial's layout.
    pub cancel: &'e dyn Cancel,
}

/// Each range of `facts`' scope as `check` reports it, with its extent and keeps, each
/// keep tried where `trials` is given (`None`: none tried).
#[must_use]
pub fn ranges_json(facts: &Facts<'_>, doc: &WordDoc, docx: &Docx, trials: Option<&Trials<'_>>) -> Vec<Value> {
    let Some(scope) = facts.scope() else {
        return Vec::new();
    };
    let mut out: Vec<Value> = Vec::new();
    let mut tried = Tried::default();
    for (range, pages) in scope.ranges.iter().zip(scope.pages(facts.version())) {
        let mut entry = Map::new();
        entry.insert("range".into(), Value::from(range.address.clone()));
        entry.insert("blocks".into(), Value::from(range.blocks.len()));
        entry.insert("pages".into(), Value::from(pages.clone()));
        if !pages.is_empty() {
            extent_json(facts, range, &pages, &mut entry);
            keeps_json(facts, doc, docx, range, &mut entry);
            if let Some(trials) = trials {
                keep_effect(facts, doc, docx, range, trials, &mut tried, &mut entry);
            }
        }
        out.push(Value::Object(entry));
    }
    out
}

/// `extent`, `height` and `extent_more` (`range_extent`).
fn extent_json(facts: &Facts<'_>, range: &Range, pages: &[usize], entry: &mut Map<String, Value>) {
    let extents = facts.extent(&range.paragraphs, Some(pages));
    let listed: Vec<Value> = extents
        .iter()
        .take(EXTENT_PAGES)
        .map(|e| {
            let mut one = Map::new();
            one.insert("page".into(), Value::from(e.page));
            one.insert("top".into(), Value::from(e.top));
            one.insert("bottom".into(), Value::from(e.bottom));
            one.insert("body".into(), Value::from(e.body));
            if e.area.1 - e.area.0 < e.body - 0.01 {
                one.insert("area".into(), json!([e.area.0, e.area.1]));
            }
            if e.columns > 1 {
                one.insert("columns".into(), Value::from(e.columns));
            }
            Value::Object(one)
        })
        .collect();
    let height: f64 = extents.iter().map(|e| e.bottom - e.top).sum();
    entry.insert("extent".into(), Value::from(listed));
    entry.insert("height".into(), Value::from(round(height, 2)));
    if extents.len() > EXTENT_PAGES {
        entry.insert("extent_more".into(), Value::from(extents.len() - EXTENT_PAGES));
    }
}

/// `one_page`, `kept`, `breaks`, `breaks_more`, `fix`, `taller_than_page` (`range_keeps`):
/// nothing for a range that is not whole blocks or not drawn whole.
fn keeps_json(facts: &Facts<'_>, doc: &WordDoc, docx: &Docx, range: &Range, entry: &mut Map<String, Value>) {
    let Ok(blocks) = kept_blocks(doc, docx, &ooxml_word_model::address::Address::parse(&range.address)) else {
        return;
    };
    let Some(laid) = facts.range_layout(range, &blocks) else {
        return;
    };
    let Ok(writes) = keep_writes(
        doc,
        docx,
        &ooxml_word_model::address::Address::parse(&range.address),
        true,
    ) else {
        return;
    };
    entry.insert("one_page".into(), Value::from(laid.one_page));
    entry.insert("kept".into(), Value::from(!writes));
    if laid.one_page {
        return;
    }
    let breaks: Vec<Value> = laid.breaks.iter().take(BREAKS_LISTED).map(break_json).collect();
    entry.insert("breaks".into(), Value::from(breaks));
    if laid.breaks.len() > BREAKS_LISTED {
        entry.insert("breaks_more".into(), Value::from(laid.breaks.len() - BREAKS_LISTED));
    }
    let all_page_breaks = laid
        .breaks
        .iter()
        .all(|b| matches!(b, Break::Between { page_break: true, .. }));
    if laid.height > laid.room + 0.5 {
        entry.insert("taller_than_page".into(), Value::from(true));
    } else if writes && !all_page_breaks {
        entry.insert("fix".into(), keep_call(&range.address, true));
    }
}

fn break_json(found: &Break) -> Value {
    match found {
        Break::Between {
            after,
            before,
            page,
            page_break,
        } => {
            let mut out = Map::new();
            out.insert("after".into(), Value::from(after.clone()));
            out.insert("before".into(), Value::from(before.clone()));
            out.insert("page".into(), Value::from(*page));
            if *page_break {
                out.insert("page_break".into(), Value::from(true));
            }
            Value::Object(out)
        }
        Break::Inside { block, pages } => json!({"inside": block, "pages": pages}),
    }
}

/// The `word_format` call keeping (`on`) or releasing `address`'s blocks.
fn keep_call(address: &str, on: bool) -> Value {
    json!({"tool": "word_format", "calls": [{"targets": [address], "keep_together": on}]})
}

/// The trials one check made so far, and when the first began.
#[derive(Default)]
struct Tried {
    count: usize,
    began: Option<Instant>,
}

/// `if_kept` or `if_released` on a range whose keep is tried (`keep_effects`), or
/// `effect_left_out` where it is not laid out.
fn keep_effect(
    facts: &Facts<'_>,
    doc: &WordDoc,
    docx: &Docx,
    range: &Range,
    trials: &Trials<'_>,
    tried: &mut Tried,
    entry: &mut Map<String, Value>,
) {
    let Some(on) = wanted(facts, range, entry) else {
        return;
    };
    let past = tried
        .began
        .is_some_and(|began| began.elapsed() > Duration::from_secs_f64(trials.seconds));
    if tried.count >= trials.trials || (tried.count > 0 && past) {
        let doing = if on { "keeping" } else { "releasing" };
        let said = format!(
            "not laid out: a check tries at most {} keeps (within {} s); check this range alone (range: \
             [{}]) for what {doing} it does",
            trials.trials,
            python_g(trials.seconds),
            python_repr(&range.address)
        );
        entry.insert("effect_left_out".into(), Value::from(said));
        return;
    }
    tried.began.get_or_insert_with(Instant::now);
    tried.count += 1;
    match trial(facts, doc, docx, range, trials, on) {
        Ok(Some(effect)) => {
            let key = if on { "if_kept" } else { "if_released" };
            entry.insert(key.into(), effect_json(&effect, &range.address, on));
        }
        Ok(None) => {}
        Err(why) => {
            let said = format!("not laid out: the trial layout failed ({why}); make the call and check again");
            entry.insert("effect_left_out".into(), Value::from(said));
        }
    }
}

/// `Some(true)` -- try the keep -- for a range with a keep `fix`; `Some(false)` -- try
/// releasing it -- for one kept on one page that starts a page only because of its keep
/// (`_wanted`).
fn wanted(facts: &Facts<'_>, range: &Range, entry: &Map<String, Value>) -> Option<bool> {
    if entry.get("fix").is_some_and(|f| !f.is_null()) {
        return Some(true);
    }
    let flag = |key: &str| entry.get(key).and_then(Value::as_bool).unwrap_or(false);
    if !(flag("kept") && flag("one_page")) {
        return None;
    }
    let start = entry.get("pages")?.as_array()?.first()?.as_u64()?;
    let start = usize::try_from(start).ok()?;
    facts.starts_page_by_keep(range, start).then_some(false)
}

/// One trial: the document with that one change laid out from the version checked.
fn trial(
    facts: &Facts<'_>,
    doc: &WordDoc,
    docx: &Docx,
    range: &Range,
    trials: &Trials<'_>,
    on: bool,
) -> Result<Option<KeepEffect>, &'static str> {
    let Ok(blocks) = kept_blocks(doc, docx, &ooxml_word_model::address::Address::parse(&range.address)) else {
        return Ok(None);
    };
    let Some(placed) = facts.trial_blocks(&blocks) else {
        return Ok(None);
    };
    let Ok(Some(package)) = kept_package(
        doc,
        docx,
        &ooxml_word_model::address::Address::parse(&range.address),
        on,
    ) else {
        return Ok(None);
    };
    let version = facts.version();
    let mut engine = ooxml_word_pages::lock_engine(trials.engine);
    let then = match engine.trial(&package, version, trials.cancel) {
        Ok(Some(then)) => then,
        Ok(None) => return Err("not laid out by this engine"),
        Err(ooxml_word_pages::PagesError::Stopped) => return Err("stopped"),
        Err(_) => return Err("the document does not open"),
    };
    // The blank is measured with what the facts leave out (text boxes, footnotes), for the
    // two pages measured only.
    let mut foot = |side: Side, page: &PageFacts| {
        let laid = match side {
            Side::Now => Laid::Version(version),
            Side::Then => Laid::Trial(&then),
        };
        engine.beyond_facts(laid, page.page)
    };
    Ok(KeepEffect::of(
        (
            (version.facts(), &version.layout().document.names),
            (then.facts(), then.names()),
        ),
        &placed,
        on,
        &mut foot,
    ))
}

/// A trial's effect as docx-agent says it, with what it says in words (`said`) and, for a
/// release, the call that releases it.
fn effect_json(effect: &KeepEffect, address: &str, on: bool) -> Value {
    let mut out = Map::new();
    out.insert("starts".into(), Value::from(effect.starts));
    out.insert("ends".into(), Value::from(effect.ends));
    out.insert("one_page".into(), Value::from(effect.starts == effect.ends));
    if let Some(blank) = &effect.blank {
        out.insert(
            "blank".into(),
            json!({"page": blank.page, "pt": blank.pt, "share": blank.share, "now_pt": blank.now_pt,
                   "now_share": blank.now_share}),
        );
    }
    out.insert(
        "document_pages".into(),
        json!([effect.document_pages.0, effect.document_pages.1]),
    );
    if let Some((now, then)) = effect.after {
        out.insert("after".into(), json!([now, then]));
    }
    if !on {
        out.insert("fix".into(), keep_call(address, false));
    }
    out.insert("said".into(), Value::from(said(effect, on)));
    Value::Object(out)
}

/// `_said`.
fn said(effect: &KeepEffect, on: bool) -> String {
    let (start, end) = (effect.starts, effect.ends);
    let mut out = String::from(if on {
        "keeping it whole (this range alone): "
    } else {
        "releasing its keep (this range alone): "
    });
    out.push_str(&format!("starts on page {start}"));
    if start == end {
        out.push_str(", on one page");
    } else {
        out.push_str(&format!(", over pages {start}-{end}"));
    }
    if let Some(blank) = &effect.blank {
        out.push_str(&format!(
            "; page {} {} {}% blank ({} pt), now {}%",
            blank.page,
            if on { "left" } else { "then" },
            percent(blank.share),
            python_g(blank.pt),
            percent(blank.now_share)
        ));
    }
    let (before, after) = effect.document_pages;
    if before == after {
        out.push_str(&format!("; the document stays at {}", pages(before)));
    } else {
        out.push_str(&format!("; the document goes from {before} to {}", pages(after)));
    }
    if let Some((now, then)) = effect.after {
        if now == then {
            out.push_str(&format!("; what follows stays on page {now}"));
        } else {
            out.push_str(&format!("; what follows moves from page {now} to {then}"));
        }
    }
    out
}

fn pages(count: usize) -> String {
    format!("{count} page{}", if count == 1 { "" } else { "s" })
}

/// Python's `round(share * 100)`: to the nearest whole number, ties to even.
fn percent(share: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let whole = (share * 100.0).round_ties_even() as i64;
    whole
}

/// Python's `round(value, places)`.
fn round(value: f64, places: usize) -> f64 {
    ooxml_word_pages::layout_facts::round(value, places)
}

/// Python's `repr` of a string, as an f-string's `!r` writes it.
fn python_repr(text: &str) -> String {
    if text.contains('\'') && !text.contains('"') {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_word_pages::layout_facts::Blank;

    #[test]
    fn a_trial_is_said_as_docx_agent_says_it() {
        let effect = KeepEffect {
            starts: 2,
            ends: 2,
            blank: Some(Blank {
                page: 1,
                pt: 165.0,
                share: 0.24,
                now_pt: 13.2,
                now_share: 0.02,
            }),
            document_pages: (2, 3),
            after: Some((2, 3)),
        };
        assert_eq!(
            said(&effect, true),
            "keeping it whole (this range alone): starts on page 2, on one page; page 1 left 24% blank \
             (165 pt), now 2%; the document goes from 2 to 3 pages; what follows moves from page 2 to 3"
        );
        let released = KeepEffect {
            starts: 1,
            ends: 2,
            blank: None,
            document_pages: (1, 1),
            after: Some((2, 2)),
        };
        assert_eq!(
            said(&released, false),
            "releasing its keep (this range alone): starts on page 1, over pages 1-2; the document stays at \
             1 page; what follows stays on page 2"
        );
    }

    #[test]
    fn an_address_is_quoted_as_python_quotes_it() {
        assert_eq!(python_repr("p:1A..t:2B"), "'p:1A..t:2B'");
        assert_eq!(python_repr("it's"), "\"it's\"");
        assert_eq!(python_repr("a'b\"c"), "'a\\'b\"c'");
    }
}
