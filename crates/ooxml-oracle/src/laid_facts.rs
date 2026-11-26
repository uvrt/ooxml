//! `word-reads`' layout fact cases: `check`'s facts read from the pages
//! (`ooxml_word_pages::layout_facts`, worded and capped by `ooxml_tools_word`), of a page
//! engine laying the document out with the case's fonts (docx-agent's) -- with each
//! range's report (`range`: extent, keeps, each keep tried) where scoped -- in the form
//! `python/word_laid_facts_cases.py` writes docx-agent's (`layout_facts`' keys): `{"facts":
//! {...}}`, or `{"error"}`.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::Unlimited;
use ooxml_opc::Package;
use ooxml_word_pages::layout_facts::{Facts, PageReads, Scope, ScopeError};
use ooxml_word_pages::page_entry::PageEntry;
use ooxml_word_pages::{LaidVersion, PageEngine};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex, PoisonError};

/// Never cancelled.
struct Never;

impl ooxml_base::Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// The pages' text, painted by the engine that laid the version out.
struct Painted<'e> {
    engine: &'e Mutex<PageEngine>,
    version: Arc<LaidVersion>,
}

impl PageReads for Painted<'_> {
    fn page_read(&self, page: usize) -> Option<Arc<PageEntry>> {
        let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
        engine.page_read(&self.version, page)
    }

    fn laid_entry(&self, page: usize) -> Option<Arc<ooxml_word_pages::laid_entry::LaidEntry>> {
        self.engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .laid_entry(&self.version, page)
    }

    fn resolved_table(
        &self,
        table: &ooxml_word_model::model::Table,
    ) -> Option<Arc<ooxml_word_model::resolve::ResolvedTable>> {
        self.engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resolved_table(&self.version, table)
    }
}

/// The layout facts of `package` laid out with `fonts`, `min_size` the minimum text size,
/// scoped to `range` (addresses) where given, on its pages.
#[must_use]
pub fn reply(package: &Package, fonts: &Value, min_size: Option<&Value>, range: Option<&Value>) -> Value {
    match facts(package, fonts, min_size, range) {
        Ok(found) => json!({"facts": found}),
        Err(error) => json!({"error": error}),
    }
}

fn facts(package: &Package, fonts: &Value, min_size: Option<&Value>, range: Option<&Value>) -> Result<Value, String> {
    let minimum = ooxml_tools_word::minimum_of(min_size.unwrap_or(&Value::Null))?;
    let engine = Mutex::new(crate::structure_tools::page_engine(fonts));
    let version = {
        let mut held = engine.lock().unwrap_or_else(PoisonError::into_inner);
        match held.lay_out(package, None).map_err(|e| e.to_string())? {
            Ok(version) => version,
            Err(fallback) => return Err(format!("falls back: {} at {}", fallback.reason, fallback.at)),
        }
    };
    let doc = ooxml_word_edit::WordDoc::open(package.clone(), Arc::new(Unlimited)).map_err(|e| e.to_string())?;
    let docx = doc.docx().map_err(|e| e.to_string())?;
    let painted = Painted {
        engine: &engine,
        version: Arc::clone(&version),
    };
    let coverage = {
        let mut held = engine.lock().unwrap_or_else(PoisonError::into_inner);
        held.coverage(&version).unwrap_or_default()
    };
    let scope = match range.and_then(Value::as_array) {
        Some(addresses) => {
            let addresses: Vec<String> = addresses.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            Some(Scope::of(&docx, &addresses).map_err(|e| refused(&e))?)
        }
        None => None,
    };
    let pages: Option<Vec<usize>> = scope.as_ref().map(|s| {
        let all: std::collections::BTreeSet<usize> = s.pages(&version).into_iter().flatten().collect();
        all.into_iter().collect()
    });
    let mut facts = Facts::new(&version, &docx, pages.as_deref(), &painted);
    if let Some(scope) = scope {
        facts = facts.scoped(scope);
    }
    let mut out = Map::new();
    let mut settings = Map::new();
    ooxml_tools_word::page_facts_json(&facts, minimum.as_ref(), &coverage, &mut out, &mut settings);
    out.insert("settings".into(), Value::Object(settings));
    if facts.scope().is_some() {
        let trials = ooxml_tools_word::Trials {
            engine: &engine,
            trials: ooxml_tools_word::TRIALS,
            seconds: ooxml_tools_word::TRIAL_SECONDS,
            cancel: &Never,
        };
        let ranges = ooxml_tools_word::ranges_json(&facts, &doc, &docx, Some(&trials));
        out.insert("range".into(), Value::from(ranges));
    }
    Ok(Value::Object(out))
}

/// A refused range, said briefly (the differential compares facts, not refusals).
fn refused(error: &ScopeError) -> String {
    format!("range refused: {error:?}")
}
