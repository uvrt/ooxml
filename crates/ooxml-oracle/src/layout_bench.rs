//! `layout-bench <file.docx> <fonts.json> [--rounds N] [--check]`: lay a document out
//! whole, then a version with one body paragraph's text changed (`bench::edited`) laid
//! out again from it, and report wall and CPU-free timings (best of N), the pages, the
//! passes and what the relayout laid out again, as one JSON line. `--check` also lays the
//! edited version out whole in a fresh engine and model and compares the pages.
//!
//! Tables: the Word layout's tables are lane L3b's; for a timing of the whole body, each
//! table here is a stand-in of fixed-height rows ([`FixedRows`]) -- a measurement aid, not
//! a layout, never compared with docx2svg.
//!
//! Must not: be used by any library; it is a measurement instrument.

use crate::bench::edited;
use crate::export::{Unlimited, KEY};
use crate::text::fonts_of;
use ooxml_base::{Limits, Locale};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_layout::px::Px;
use ooxml_word_layout::sections::SectionGeometry;
use ooxml_word_layout::table::{PlacedRow, TableFlow, TableLayout, TablePage, TableRequest, Unsupported};
use ooxml_word_layout::{WordGlyphs, WordLayout};
use ooxml_word_model::model::{Paragraph, Table};
use ooxml_word_model::value::Text;
use ooxml_word_model::{Model, WordDocument};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

/// A stand-in table: its rows, each 40 px, never split, moved whole to the next page.
#[derive(Debug)]
pub struct FixedRows {
    rows: usize,
}

const ROW: i64 = 40;

impl TableFlow for FixedRows {
    fn rows(&self) -> usize {
        self.rows
    }
    fn keeps_with_next(&self, _row: usize) -> bool {
        false
    }
    fn header_rows(&self) -> usize {
        0
    }
    fn first_style(&self) -> Option<Text> {
        None
    }
    fn top_border(&self) -> Px {
        Px::ZERO
    }
    fn bottom_border(&self) -> Px {
        Px::ZERO
    }
    fn place(&self, request: &TableRequest) -> Result<TablePage, Unsupported> {
        let mut y = request.y;
        let mut rows = Vec::new();
        for row in request.row..self.rows {
            let h = Px::int(ROW);
            if y + h > request.bottom && !(rows.is_empty() && request.first_on_page) {
                return Ok(TablePage {
                    end: y,
                    next: Some((row, Vec::new())),
                    rows,
                });
            }
            rows.push(PlacedRow {
                row,
                header: false,
                top: y,
                height: h,
                part: Vec::new(),
            });
            y += h;
        }
        Ok(TablePage { end: y, next: None, rows })
    }
}

/// Lays every table out as [`FixedRows`].
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedRowTables;

impl TableLayout for FixedRowTables {
    fn flow_table(
        &self,
        _document: &WordDocument,
        table: &Table,
        _section: &SectionGeometry,
        _previous: Option<&Paragraph>,
        _labels: &[Option<String>],
    ) -> Result<Arc<dyn TableFlow>, Unsupported> {
        Ok(Arc::new(FixedRows { rows: table.rows.len() }))
    }
}

fn engine(fonts: &Value, bytes: &[u8]) -> Result<WordLayout, String> {
    let budget: Arc<dyn CacheBudget> = Arc::new(Unlimited);
    Ok(WordLayout::new(
        Arc::new(WordGlyphs::new(fonts_of(fonts, Some(bytes))?)),
        Arc::new(FixedRowTables),
        Locale::default(),
        &budget,
        KEY,
    ))
}

/// Run the benchmark.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("give a .docx")?;
    let fonts_path = args.get(1).ok_or("give a fonts record (JSON)")?;
    let rounds: usize = args
        .iter()
        .position(|a| a == "--rounds")
        .and_then(|k| args.get(k + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let check = args.iter().any(|a| a == "--check");
    let fonts: Value = serde_json::from_slice(&std::fs::read(fonts_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let main = "word/document.xml";
    let xml = package.read(main).map_err(|e| e.to_string())?.ok_or("no main part")?;
    let next = package.with_bytes(main, Arc::from(edited(&xml).ok_or("nothing to edit")?));
    let (mut full_best, mut relayout_best, mut open_best) = (f64::MAX, f64::MAX, f64::MAX);
    let mut report = json!({});
    for _ in 0..rounds.max(1) {
        let mut model = Model::new(Arc::new(Unlimited), KEY);
        let started = Instant::now();
        let document = model.open(&package).map_err(|e| e.to_string())?;
        open_best = open_best.min(started.elapsed().as_secs_f64());
        let mut layout = engine(&fonts, &bytes)?;
        let started = Instant::now();
        let whole = layout.lay_out(&mut model, &document);
        full_best = full_best.min(started.elapsed().as_secs_f64());
        let edited_version = model.open(&next).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let again = layout.relayout(&mut model, &edited_version, &whole);
        relayout_best = relayout_best.min(started.elapsed().as_secs_f64());
        let equal = if check {
            let mut fresh_model = Model::new(Arc::new(Unlimited), KEY);
            let fresh = fresh_model.open(&next).map_err(|e| e.to_string())?;
            let reference = engine(&fonts, &bytes)?.lay_out(&mut fresh_model, &fresh);
            Some(reference.pagination == again.pagination && reference.fields == again.fields)
        } else {
            None
        };
        report = json!({
            "pages": whole.pagination.page_count(), "laid_pages": whole.pagination.pages.len(),
            "items": whole.flow.items.len(), "passes": whole.passes, "settled": whole.settled,
            "fields": whole.fields.len(), "stopped": whole.pagination.stopped.as_ref().map(|s| s.reason.clone()),
            "relayout_passes": again.passes,
            "relaid": again.relaid.iter().map(|s| json!({"change": s.change.map(|c| [c.a, c.b_old, c.b_new]), "restart": s.restart, "laid": s.laid, "joined": s.joined})).collect::<Vec<_>>(),
            "relayout_equals_full": equal,
        });
    }
    let line = json!({
        "file": path, "rounds": rounds, "open_s": open_best, "full_layout_s": full_best,
        "relayout_s": relayout_best, "tables": "fixed-row stand-in", "layout": report,
    });
    println!("{line}");
    Ok(())
}
