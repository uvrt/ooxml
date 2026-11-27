//! `layout-bench <file.docx> <fonts.json> [--rounds N] [--check]`: lay a document out
//! whole, then a version with one body paragraph's text changed (`bench::edited`) laid
//! out again from it, and report wall and CPU-free timings (best of N), the pages, the
//! passes and what the relayout laid out again, and the time one page's facts take on
//! their own and every page's, as one JSON line. `--check` also lays the edited version
//! out whole in a fresh engine and model and compares the pages.
//!
//! Must not: be used by any library; it is a measurement instrument.

use crate::bench::edited;
use crate::export::{Unlimited, KEY};
use crate::fonts_record::fonts_of;
use ooxml_base::{Limits, Locale};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_layout::tables::WordTables;
use ooxml_word_layout::{WordGlyphs, WordLayout};
use ooxml_word_model::Model;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

fn engine(fonts: &Value, bytes: &[u8]) -> Result<WordLayout, String> {
    let budget: Arc<dyn CacheBudget> = Arc::new(Unlimited);
    Ok(WordLayout::new(
        Arc::new(WordGlyphs::new(fonts_of(fonts, Some(bytes))?)),
        Arc::new(WordTables),
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
    let fonts: Value =
        serde_json::from_slice(&std::fs::read(fonts_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let main = "word/document.xml";
    let xml = package.read(main).map_err(|e| e.to_string())?.ok_or("no main part")?;
    let package = ooxml_word_model::WordPackage::open(package).0;
    let next = ooxml_word_model::WordPackage::open_after(
        package.with_bytes(main, Arc::from(edited(&xml).ok_or("nothing to edit")?)),
        Some(&package),
    )
    .0;
    let (mut full_best, mut relayout_best, mut open_best) = (f64::MAX, f64::MAX, f64::MAX);
    let (mut page_facts_best, mut facts_best) = (f64::MAX, f64::MAX);
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
        // One page's facts on their own (the middle page, made once), then every page's.
        let started = Instant::now();
        let _ = again.page_facts(again.page_count() / 2);
        page_facts_best = page_facts_best.min(started.elapsed().as_secs_f64());
        let started = Instant::now();
        let _ = ooxml_word_layout::facts::LayoutFacts::of(&again);
        facts_best = facts_best.min(started.elapsed().as_secs_f64());
        let equal = if check {
            let mut fresh_model = Model::new(Arc::new(Unlimited), KEY);
            let fresh = fresh_model.open(&next).map_err(|e| e.to_string())?;
            let reference = engine(&fonts, &bytes)?.lay_out(&mut fresh_model, &fresh);
            Some(reference.pagination == again.pagination && reference.fields == again.fields)
        } else {
            None
        };
        // Which items of the first pass differ between the versions, where they line up.
        let differing: Vec<usize> = match (whole.pass(0), again.pass(0)) {
            (Some((a, _)), Some((b, _))) if a.items.len() == b.items.len() => (0..a.items.len())
                .filter(|k| match (a.items.get(*k), b.items.get(*k)) {
                    (Some(x), Some(y)) => !ooxml_word_layout::incremental::same_item(x, y),
                    _ => true,
                })
                .collect(),
            _ => Vec::new(),
        };
        report = json!({
            "differing_items": differing.len(), "differing_first": differing.iter().take(8).collect::<Vec<_>>(),
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
        "relayout_s": relayout_best, "page_facts_s": page_facts_best, "facts_s": facts_best, "layout": report,
    });
    println!("{line}");
    Ok(())
}
