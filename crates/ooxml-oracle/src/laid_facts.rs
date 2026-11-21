//! `word-reads`' layout fact cases: `check`'s facts read from the pages
//! (`ooxml_word_pages::layout_facts`, worded and capped by `ooxml_tools_word`), of a page
//! engine laying the document out with the case's fonts (docx-agent's), in the form
//! `python/word_laid_facts_cases.py` writes docx-agent's (`layout_facts`' keys): `{"facts":
//! {...}}`, or `{"error"}`.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::Unlimited;
use ooxml_opc::Package;
use ooxml_word_pages::layout_facts::{Facts, PageTexts};
use ooxml_word_pages::line_text::LineText;
use ooxml_word_pages::{LaidVersion, PageEngine};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex, PoisonError};

/// The pages' text, painted by the engine that laid the version out.
struct Painted<'e> {
    engine: &'e Mutex<PageEngine>,
    version: Arc<LaidVersion>,
}

impl PageTexts for Painted<'_> {
    fn line_texts(&self, page: usize) -> Option<Vec<LineText>> {
        let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
        engine.line_texts(&self.version, page)
    }
}

/// The layout facts of `package` laid out with `fonts`, `min_size` the minimum text size.
#[must_use]
pub fn reply(package: &Package, fonts: &Value, min_size: Option<&Value>) -> Value {
    match facts(package, fonts, min_size) {
        Ok(found) => json!({"facts": found}),
        Err(error) => json!({"error": error}),
    }
}

fn facts(package: &Package, fonts: &Value, min_size: Option<&Value>) -> Result<Value, String> {
    let minimum = ooxml_tools_word::minimum_of(min_size.unwrap_or(&Value::Null))?;
    let engine = Mutex::new(crate::structure_tools::page_engine(fonts));
    let version = {
        let mut held = engine.lock().unwrap_or_else(PoisonError::into_inner);
        match held.lay_out(package).map_err(|e| e.to_string())? {
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
    let facts = Facts::new(&version, &docx, None, &painted);
    let mut out = Map::new();
    let mut settings = Map::new();
    ooxml_tools_word::page_facts_json(&facts, minimum.as_ref(), &mut out, &mut settings);
    out.insert("settings".into(), Value::Object(settings));
    Ok(Value::Object(out))
}
