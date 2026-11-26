//! Owns: what `describe` and `save_document` read of a version's layout by the document's
//! engine (the host's, else the session's own, [`Owned::layout`]): the page count
//! (docx-agent's `_pages`), the coverage (`_coverage`, as `check`'s `reflow` gives it) and
//! `app.xml`'s pages and lines (`layout_statistics`); each read failing as a short text
//! (`"<code>: <message>"`, as docx-agent says a layout's failure in these places).
//!
//! Must not: lay out but through the version's engine, within the call's layout limit.

use crate::coverage::coverage_facts;
use crate::version::{LayoutCounts, Owned};
use crate::WordLayout;
use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_word_pages::LaidVersion;
use serde_json::Value;
use std::sync::{Arc, PoisonError};

/// A version laid out by its engine, read for its pages.
pub(crate) struct LaidRead<'a> {
    layout: &'a dyn WordLayout,
    laid: Arc<LaidVersion>,
}

/// `package` laid out by `owned`'s engine within the call's layout limit (else until
/// `cancel`): `None` where the call has no layout; `Err` the failure as docx-agent says it.
pub(crate) fn laid<'a>(
    owned: &Owned<'a>,
    package: &Package,
    cancel: &dyn Cancel,
) -> Option<Result<LaidRead<'a>, String>> {
    let layout = owned.layout?;
    let stop = owned.layout_limit.map_or(cancel, |limit| limit.cancel);
    Some(match layout.laid(package, stop) {
        Ok(laid) => Ok(LaidRead { layout, laid }),
        Err(why) => {
            let (code, message) = crate::values::not_laid(why);
            Err(format!("{code}: {message}"))
        }
    })
}

impl LaidRead<'_> {
    /// The pages laid out.
    pub(crate) fn pages(&self) -> usize {
        self.laid.page_count()
    }

    /// The coverage facts (`layout.coverage_facts()`), or why not.
    pub(crate) fn coverage(&self) -> Result<Value, String> {
        let coverage = self
            .layout
            .engine()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .coverage(&self.laid)
            .ok_or_else(not_ours)?;
        Ok(coverage_facts(&coverage, |key, path| {
            ooxml_word_pages::coverage::id_of(&self.laid, key, path)
        }))
    }

    /// `app.xml`'s pages and lines: the pages laid out, the body's lines on them.
    pub(crate) fn counts(&self) -> Option<LayoutCounts> {
        let lines = self
            .layout
            .engine()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .body_lines(&self.laid)?;
        Some(LayoutCounts {
            pages: u64::try_from(self.pages()).ok()?,
            lines: u64::try_from(lines).ok()?,
        })
    }
}

/// A version an engine of another generation laid out, as said where it is read.
fn not_ours() -> String {
    "layout_failed: the version laid out is not the engine's".to_owned()
}

/// A failure said as docx-agent's `{"error": ...}` of a coverage.
pub(crate) fn error_json(said: String) -> Value {
    serde_json::json!({ "error": said })
}
