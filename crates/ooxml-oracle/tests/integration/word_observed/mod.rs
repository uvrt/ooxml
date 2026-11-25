//! Word's layout measurements held against the Rust layout: what Word 16.106 drew for the
//! probe documents docx2svg's tests scored its layout against (`word/observations/layout`,
//! registered in `word/index.py`'s `LAYOUT`), now scored against the page engine's paint --
//! what `LaidPages` draws -- with the deleted tests' tolerances (6e850e18), a family per
//! module.
//!
//! Each family lays out the observed documents (frozen in `corpus/`, or where the gates do
//! not read them in `tests/data/word-probes/`) with the open faces the repository ships,
//! Calibri laid out with Carlito (its widths), so a run needs no Word and no font of the
//! machine's. `wrap_side` names each case the layout disagrees with Word on as a known
//! difference with its reason, strictly (a known difference that agrees fails too). The
//! families ported as Phase 3's safety net hold each case to a floor instead ([`Floors`]):
//! today's agreement, measured, so a change that loses a line Word agrees with fails while
//! today's gaps stay counted where they are.
//!
//! Must not: loosen a tolerance, or score anything but the layout against Word.

// Test code: a recording that is not as read, or a document the layout refuses, fails the test.
#![allow(clippy::panic, clippy::indexing_slicing)]

mod glyphs;
mod lines;
mod notes;
mod pagination;
mod tables;
mod wrap_side;
mod wrap_table;

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

/// A probe document: frozen in `corpus/<maker>/` where the gates read it too, else in
/// `tests/data/word-probes/<maker>/` (built once by the probe's maker, docx2svg's
/// `tools/make_*_probe.py` at 6e850e18, byte for byte).
pub fn probe(maker: &str, name: &str) -> Vec<u8> {
    let corpus = format!("{CRATE}/corpus/{maker}/{name}.docx");
    if std::path::Path::new(&corpus).exists() {
        return std::fs::read(&corpus).unwrap_or_else(|e| panic!("{corpus}: {e}"));
    }
    let path = format!("{CRATE}/tests/data/word-probes/{maker}/{name}.docx");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A document of the crate's `fixtures/<folder>/`.
pub fn corpus_fixture(folder: &str, name: &str) -> Vec<u8> {
    let path = format!("{CRATE}/fixtures/{folder}/{name}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The case family of each of Word's pages of `document`, as the probe's reader named them
/// (`tests/data/word-probes/families.json`: per family of recordings, per document, runs of
/// `[family, pages]`).
pub fn page_families(recording: &str, document: &str) -> Vec<String> {
    let path = format!("{CRATE}/tests/data/word-probes/families.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let all: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
    let runs = all[recording][document]
        .as_array()
        .unwrap_or_else(|| panic!("{path}: no pages for {recording} {document}"));
    let mut out = Vec::new();
    for run in runs {
        let family = run[0].as_str().unwrap_or_default();
        let pages = run[1].as_u64().unwrap_or_default();
        out.extend((0..pages).map(|_| family.to_owned()));
    }
    out
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
    pub fn lines<'a>(&mut self, lines: &BTreeMap<glyphs::LineKey, Vec<String>>, family_of: impl Fn(usize) -> &'a str) {
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

/// A family's floors: per document, per case, `(case, agreeing, scored)` as the layout scores
/// today. A case's lines are Word's, so their count is fixed; a glyph check's count is the
/// layout's glyphs where it counts what the layout draws, so only its disagreeing are held.
pub type Floors = &'static [(&'static str, &'static [(&'static str, usize, usize)])];

/// The cases scored against the floors: `Err` lists every case below its floor (fewer
/// agreeing, or more disagreeing for a glyph check), a case of Word's whose count changed,
/// and a case scored with no floor or a floor with nothing scored.
pub fn check_floors(document: &str, scored: &Scored, floors: Floors) -> Result<(), Vec<String>> {
    let rows: BTreeMap<&str, (usize, usize)> = floors
        .iter()
        .find(|(name, _)| *name == document)
        .map(|(_, rows)| {
            rows.iter()
                .map(|(case, agree, total)| (*case, (*agree, *total)))
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for (case, (bad, total)) in &scored.cases {
        let agree = total - bad;
        let Some(&(floor, floor_total)) = rows.get(case.as_str()) else {
            out.push(format!("{document} {case}: {agree} of {total} agree, no floor"));
            continue;
        };
        let words =
            case.starts_with("lines") || matches!(case.as_str(), "glyphs not drawn" | "warnings" | "pages" | "cases");
        let below = if words {
            *total != floor_total || agree < floor
        } else {
            *bad > floor_total - floor
        };
        if below {
            out.push(format!(
                "{document} {case}: {agree} of {total} agree with Word, floor {floor} of {floor_total}"
            ));
        }
    }
    for case in rows.keys() {
        if !scored.cases.contains_key(*case) {
            out.push(format!("{document} {case}: a floor, nothing scored"));
        }
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}

impl Scored {
    /// The document's row of floors as the layout scores it now, for a family's table.
    pub fn floor_row(&self, document: &str) -> String {
        let cases: Vec<String> = self
            .cases
            .iter()
            .map(|(case, (bad, total))| format!("(\"{case}\", {}, {total})", total - bad))
            .collect();
        format!("    (\"{document}\", &[{}]),\n", cases.join(", "))
    }
}

/// One document of a recording of text objects (`objects`, as `read_render.word_objects`
/// keeps them) scored as `wrap_side`'s are: Word's lines within half a pixel, per case
/// family when the recording's reader counted them so (`families`), else as one, `all`;
/// the glyph row's checks; the warning codes.
pub fn score_objects(recording: &Value, maker: &str, document: &str, families: Option<&str>) -> Scored {
    let recorded = &recording["documents"][document];
    let laid = lay_out(document, probe(maker, document));
    let model = glyphs::laid_glyphs(&laid.pages);
    let word = glyphs::word_glyphs(&recorded["objects"]);
    let lines = glyphs::line_score(&model, &word);
    let pages = families.map(|f| page_families(f, document)).unwrap_or_default();
    let mut scored = Scored::default();
    scored.lines(&lines, |page| match families {
        Some(_) => pages.get(page).map_or("past the last case", String::as_str),
        None => "all",
    });
    scored.glyphs(&glyphs::compare(&model, &word));
    scored.warnings(&laid.warnings, &recorded["warnings"]);
    if let Some(word) = recorded["pages"].get(1).and_then(Value::as_u64) {
        let same = laid.pages.len() as u64 == word;
        let shown = (!same).then(|| format!("layout {}, Word {word}", laid.pages.len()));
        scored.add("pages".into(), usize::from(!same), 1, shown);
    }
    scored
}

/// The documents of a recording, in its order.
pub fn documents_of(recording: &Value) -> Vec<String> {
    recording["documents"]
        .as_object()
        .map(|documents| documents.keys().cloned().collect())
        .unwrap_or_default()
}

/// Every document of a family scored and held to its floors; a failure lists every case
/// below its floor and, to update a floor that has risen, every document's row as scored.
pub fn check_family(documents: &[&str], floors: Floors, score: impl Fn(&str) -> Scored) {
    let mut problems = Vec::new();
    let mut rows = String::new();
    let mut samples = String::new();
    for document in documents {
        let scored = score(document);
        rows.push_str(&scored.floor_row(document));
        if let Err(found) = check_floors(document, &scored, floors) {
            problems.extend(found);
            samples.push_str(&scored.sample(5));
        }
    }
    for (document, _) in floors {
        if !documents.contains(document) {
            problems.push(format!("{document}: a floor, not scored"));
        }
    }
    assert!(
        problems.is_empty(),
        "{}\nas scored now:\n{rows}first problems:\n{samples}",
        problems.join("\n")
    );
}
