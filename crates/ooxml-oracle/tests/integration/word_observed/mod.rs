//! Word's layout measurements held against the Rust layout: what Word 16.106 drew for the
//! probe documents docx2svg's tests scored its layout against (`word/observations/layout`,
//! registered in `word/index.py`'s `LAYOUT`), now scored against the page engine's paint --
//! what `LaidPages` draws -- with the deleted tests' tolerances (6e850e18), a family per
//! module.
//!
//! Each family lays out the observed documents (frozen in `corpus/`) with the open faces
//! the repository ships, Calibri laid out with Carlito (its widths), so a run needs no Word
//! and no font of the machine's. Where the layout disagrees with Word beyond the tolerance,
//! the family names the case as a known difference with its reason; the check is strict: a
//! known difference that agrees fails too, so the list is kept to what is true.
//!
//! Must not: loosen a tolerance, or score anything but the layout against Word.

// Test code: a recording that is not as read, or a document the layout refuses, fails the test.
#![allow(clippy::panic, clippy::indexing_slicing)]

mod glyphs;
mod wrap_side;

use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_oracle::edits::Unlimited;
use ooxml_word_layout::paint::PagePaint;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The crate's directory.
const CRATE: &str = env!("CARGO_MANIFEST_DIR");

/// A recording of `word/observations/layout`.
pub fn observations(file: &str) -> Value {
    let path = format!("{CRATE}/word/observations/layout/{file}");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The bytes of a frozen probe document, `corpus/<maker>/<name>.docx`.
pub fn corpus(maker: &str, name: &str) -> Vec<u8> {
    let path = format!("{CRATE}/corpus/{maker}/{name}.docx");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A page engine over the open faces the repository ships, Word's substitutes for the faces
/// they stand in for.
fn engine() -> PageEngine {
    let folder = Folder {
        location: Location::Folder,
        path: format!("{CRATE}/../../python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"),
    };
    let setup = Arc::new(FontSetup::new(
        Arc::new(DiskFonts),
        &[folder],
        WordFontOptions::default(),
    ));
    let budgets = EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Unlimited),
    };
    PageEngine::new(setup, EngineOptions::default(), budgets)
}

/// A document laid out and every page painted.
pub struct Laid {
    /// Each page's paint.
    pub pages: Vec<Arc<PagePaint>>,
    /// The layout's warnings' codes, sorted, once each.
    pub warnings: Vec<String>,
}

/// Lay `data` out and paint every page.
pub fn lay_out(name: &str, data: Vec<u8>) -> Laid {
    let mut engine = engine();
    let version = match engine.lay_out_bytes(Arc::from(data)) {
        Ok(Ok(version)) => version,
        Ok(Err(fallback)) => panic!("{name}: the layout does not lay it out: {fallback}"),
        Err(error) => panic!("{name}: {error}"),
    };
    let pages = (0..version.page_count())
        .map(|page| {
            engine
                .paint(&version, page)
                .unwrap_or_else(|| panic!("{name}: page {page} not painted"))
        })
        .collect();
    let mut warnings: Vec<String> = version.said().warnings.iter().map(|w| w.code.clone()).collect();
    warnings.sort();
    warnings.dedup();
    Laid { pages, warnings }
}

/// One document's cases scored: per case, `(disagreeing, scored)` -- `lines <family>`: Word's
/// lines of a case family; `glyphs extra`, `glyphs not drawn`, `glyphs face`, `glyphs size`:
/// the glyph row's checks; `warnings`: the layout's warning codes against the recorded ones
/// -- and the first problems, to show where a case disagrees.
#[derive(Debug, Default)]
pub struct Scored {
    /// Per case, `(disagreeing, scored)`.
    pub cases: BTreeMap<String, (usize, usize)>,
    /// What disagrees, per case.
    pub problems: BTreeMap<String, Vec<String>>,
}

impl Scored {
    fn add(&mut self, case: String, bad: usize, total: usize, problems: impl IntoIterator<Item = String>) {
        let row = self.cases.entry(case.clone()).or_default();
        row.0 += bad;
        row.1 += total;
        self.problems.entry(case).or_default().extend(problems);
    }

    /// Word's lines (`glyphs::line_score`), a line's case family that of its page.
    pub fn lines(&mut self, lines: &BTreeMap<glyphs::LineKey, Vec<String>>, family_of: impl Fn(usize) -> &'static str) {
        for ((page, y), problems) in lines {
            let shown = problems
                .first()
                .map(|p| format!("p{} y={y}: {p} ({} glyphs)", page + 1, problems.len()));
            self.add(
                format!("lines {}", family_of(*page)),
                usize::from(!problems.is_empty()),
                1,
                shown,
            );
        }
    }

    /// The glyph row's checks that a recording holds whole: every glyph Word drew drawn,
    /// none it did not, each matched one in Word's face and size.
    pub fn glyphs(&mut self, score: &glyphs::Score) {
        let faces: Vec<String> = score.problems.iter().filter(|p| p.contains("face")).cloned().collect();
        self.add("glyphs extra".into(), score.extra, score.model, None);
        self.add("glyphs not drawn".into(), score.not_drawn, score.word, None);
        self.add(
            "glyphs face".into(),
            score.matched - score.face_agree,
            score.matched,
            faces,
        );
        self.add(
            "glyphs size".into(),
            score.matched - score.size_agree,
            score.matched,
            None,
        );
    }

    /// The layout's warning codes against those recorded for docx2svg's.
    pub fn warnings(&mut self, laid: &[String], recorded: &Value) {
        let recorded: Vec<String> = recorded
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let same = laid == recorded.as_slice();
        let shown = (!same).then(|| format!("layout {laid:?}, recorded {recorded:?}"));
        self.add("warnings".into(), usize::from(!same), 1, shown);
    }

    /// The first `limit` problems of every case, for a failure's message.
    pub fn sample(&self, limit: usize) -> String {
        let mut out = String::new();
        for (case, problems) in &self.problems {
            for p in problems.iter().take(limit) {
                out.push_str(&format!("  {case}: {p}\n"));
            }
        }
        out
    }
}

/// A family's known differences: per document, `(case, disagreeing, why)`.
pub type Known = &'static [(&'static str, &'static [(&'static str, usize, &'static str)])];

/// The cases scored against the known differences: `Err` lists every case that is not as
/// known -- one that disagrees beyond them, and one known that agrees (the check is strict).
pub fn check_known(document: &str, scored: &Scored, known: Known) -> Result<(), Vec<String>> {
    let expected: BTreeMap<&str, usize> = known
        .iter()
        .find(|(name, _)| *name == document)
        .map(|(_, rows)| rows.iter().map(|(case, bad, _)| (*case, *bad)).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (case, (bad, total)) in &scored.cases {
        let known = expected.get(case.as_str()).copied().unwrap_or(0);
        if *bad != known {
            out.push(format!(
                "{document} {case}: {bad} of {total} disagree with Word, {known} known"
            ));
        }
    }
    for (case, bad) in &expected {
        if !scored.cases.contains_key(*case) {
            out.push(format!("{document} {case}: {bad} known, nothing scored"));
        }
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}
