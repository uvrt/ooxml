//! Owns: the reflow a changing call's checks report ([`reflow_after_edit`], docx-agent's
//! `_base.checks`): the version before the call's first edit -- where the document's last
//! check saw it or the engine laid it out last -- against the version the call made;
//! "stale: call check" past [`REFLOW_PAGES`] pages before; a stop laying out said as
//! docx-agent's deadline says it.
//!
//! A call never waits for the document's engine: where another call holds it, the two
//! versions are laid out on a spare engine of its kind ([`EngineSeed::spare`]) and compared
//! there -- the layout is per-version data, so no lock is shared across a relayout.
//!
//! Must not: lay out but through the host's engine or a spare made from its seed, or
//! change what the last check saw.

use super::reflow::{reflow_json, LastCheck};
use crate::pages::{LayoutLimit, WordLayout};
use ooxml_base::Cancel;
use ooxml_word_model::docx::Docx;
use ooxml_word_pages::summarised::Summarised;
use ooxml_word_pages::{EngineSeed, LaidVersion, PageEngine, PagesError};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, TryLockError};

/// The most pages a changing call's checks compare (docx-agent's `REFLOW_PAGES`): beyond,
/// the reflow says [`STALE`].
pub const REFLOW_PAGES: usize = 20;
/// The reflow of a changing call whose version before had more than [`REFLOW_PAGES`].
pub const STALE: &str = "stale: call check";

/// The version a changing call's first edit was applied to: its number and package.
#[derive(Clone, Copy)]
pub struct EditedFrom<'a> {
    /// Its number.
    pub version: u64,
    /// Its reading (the document's one chain of readings).
    pub reading: &'a Arc<Docx>,
}

/// The reflow a changing call's checks report (docx-agent's `_base.checks`), measured from
/// `before` -- where the document's last check that laid it out saw it (`last`, with
/// `reflow` or not) or `layout`'s engine laid it out last, else `None`: no reflow is measured, as docx-agent measures none without a
/// layout before -- to `after` (the version the call's last edit made), ids followed through
/// `aliases`. More than [`REFLOW_PAGES`] before: [`STALE`]. The versions are laid out under
/// the call's `limit` on laying out (else until `cancel`): a stop is said as docx-agent's
/// deadline says it. What the last check saw is left as it was.
///
/// The engine is never waited for: where another call holds it, both versions are laid out
/// on a spare engine of its kind and compared there (what it laid out last is read from
/// its seed); else it is held for the whole measure (the version before kept comparable,
/// the version after laid out, the two compared page by page).
#[must_use]
pub fn reflow_after_edit(
    layout: &dyn WordLayout,
    last: &LastCheck,
    before: EditedFrom<'_>,
    after: &Arc<Docx>,
    aliases: &BTreeMap<String, String>,
    (limit, cancel): (Option<LayoutLimit<'_>>, &dyn Cancel),
) -> Option<Value> {
    let seen = last.laid().filter(|seen| seen.version == before.version);
    let stop = Stop { limit, cancel };
    let aliases: HashMap<String, String> = aliases.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut engine = match layout.engine().try_lock() {
        Ok(engine) => engine,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => {
            let count = match &seen {
                Some(seen) => seen.pages.count(),
                None => layout.seed().laid_last(&before.reading.package)?.page_count(),
            };
            return on_spare(layout.seed(), count, (before.reading, after), &aliases, stop);
        }
    };
    let was = match seen.map(|s| s.pages) {
        Some(Summarised::Laid {
            version,
            engine: theirs,
        }) if Arc::ptr_eq(&theirs, layout.engine()) => Was::Here(version),
        Some(other) => Was::Elsewhere(other),
        None => Was::Here(engine.laid_last(&before.reading.package)?),
    };
    if was.count() > REFLOW_PAGES {
        return Some(Value::from(STALE));
    }
    if let Was::Here(version) = &was {
        engine.watch(version);
    }
    let now = match stop.lay_out(&mut engine, after) {
        Ok(now) => now,
        Err(said) => return said,
    };
    let found = match was {
        Was::Here(version) => engine.compare(&now, &version, &aliases),
        Was::Elsewhere(seen) => {
            drop(engine);
            Summarised::watched(now, Arc::clone(layout.engine())).compare(&seen, &aliases)
        }
    };
    found.map(reflow_json)
}

/// [`reflow_after_edit`] where the document's engine is held by another call: the version
/// before (of `count` pages, laid out there or seen by the last check) and the version after
/// laid out on a spare engine made from `seed`, and compared page by page there.
fn on_spare(
    seed: &EngineSeed,
    count: usize,
    (before, after): (&Arc<Docx>, &Arc<Docx>),
    aliases: &HashMap<String, String>,
    stop: Stop<'_>,
) -> Option<Value> {
    if count > REFLOW_PAGES {
        return Some(Value::from(STALE));
    }
    let mut spare = seed.spare();
    let was = match stop.lay_out(&mut spare, before) {
        Ok(was) => was,
        Err(said) => return said,
    };
    spare.watch(&was);
    let now = match stop.lay_out(&mut spare, after) {
        Ok(now) => now,
        Err(said) => return said,
    };
    spare.compare(&now, &was, aliases).map(reflow_json)
}

/// When a call's laying out stops: its limit on laying out, else its cancel.
#[derive(Clone, Copy)]
struct Stop<'a> {
    limit: Option<LayoutLimit<'a>>,
    cancel: &'a dyn Cancel,
}

impl Stop<'_> {
    /// `version` laid out by `engine`; else what the reflow says (`Some`: a stop, as
    /// docx-agent's deadline says it) or `None` (the engine does not lay it out).
    fn lay_out(self, engine: &mut PageEngine, version: &Arc<Docx>) -> Result<Arc<LaidVersion>, Option<Value>> {
        match engine.lay_out_until(version, self.limit.map_or(self.cancel, |l| l.cancel)) {
            Ok(Ok(laid)) => Ok(laid),
            Err(PagesError::Stopped) => {
                let within = self.limit.map_or_else(String::new, |l| {
                    format!(" within {} s", ooxml_scene::python_g(l.timeout.as_secs_f64()))
                });
                Err(Some(Value::from(format!(
                    "timeout: laying the document out did not finish{within}"
                ))))
            }
            Ok(Err(_)) | Err(_) => Err(None),
        }
    }
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
