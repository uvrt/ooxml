//! `paint-bench <file.docx> <fonts.json> [--rounds N]`: lay a document out whole once, then
//! time its layout facts (`LayoutFacts::of`, every page) and one page's paint and SVG (the
//! middle page: geometry, header, footer and footnotes, spans, rules, the SVG text), best
//! of N, wall time, as one JSON line. A measurement instrument.

use ooxml_base::{Limits, Locale};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_oracle::export::{Unlimited, KEY};
use ooxml_oracle::fonts_record::fonts_of;
use ooxml_word_layout::facts::LayoutFacts;
use ooxml_word_layout::paint::svg::{page_svg, NoDrawings, NoPictures, SvgContext};
use ooxml_word_layout::paint::{paint_page, DocumentNames, GlyphSize, PaintContext};
use ooxml_word_layout::tables::WordTables;
use ooxml_word_layout::{WordGlyphs, WordLayout};
use ooxml_word_model::Model;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

fn run(args: &[String]) -> Result<Value, String> {
    let path = args.first().ok_or("give a .docx")?;
    let fonts_path = args.get(1).ok_or("give a fonts record (JSON)")?;
    let rounds: usize = args
        .iter()
        .position(|a| a == "--rounds")
        .and_then(|k| args.get(k + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let fonts: Value =
        serde_json::from_slice(&std::fs::read(fonts_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let mut model = Model::new(Arc::new(Unlimited), KEY);
    let document = model.open(&package).map_err(|e| e.to_string())?;
    let glyphs = Arc::new(WordGlyphs::new(fonts_of(&fonts, Some(&bytes))?));
    let budget: Arc<dyn CacheBudget> = Arc::new(Unlimited);
    let mut engine = WordLayout::new(
        Arc::clone(&glyphs) as Arc<dyn ooxml_word_layout::Faces>,
        Arc::new(WordTables),
        Locale::default(),
        &budget,
        KEY,
    );
    let started = Instant::now();
    let laid = engine.lay_out(&mut model, &document);
    let layout_seconds = started.elapsed().as_secs_f64();
    let names = DocumentNames;
    let cx = PaintContext {
        faces: &*glyphs,
        names: &names,
        glyph_size: GlyphSize::Device,
        decimal: '.',
    };
    let svg_cx = SvgContext {
        drawings: &NoDrawings,
        names: &names,
        pictures: &NoPictures,
        glyph_size: GlyphSize::Device,
        version: &laid.document.names,
    };
    let page = laid.page_count() / 2;
    let (mut facts_best, mut page_best, mut bytes_out) = (f64::MAX, f64::MAX, 0);
    for _ in 0..rounds.max(1) {
        let started = Instant::now();
        let facts = LayoutFacts::of(&laid);
        facts_best = facts_best.min(started.elapsed().as_secs_f64());
        drop(facts);
        let started = Instant::now();
        let parts = engine.page_parts(&mut model, &laid, page);
        let svg = paint_page(&laid, &mut model, page, &parts, &cx).map(|p| page_svg(&p, &svg_cx));
        page_best = page_best.min(started.elapsed().as_secs_f64());
        bytes_out = svg.map_or(0, |s| s.len());
    }
    Ok(json!({
        "document": path,
        "pages": laid.page_count(),
        "layout_seconds": layout_seconds,
        "facts_seconds_best": facts_best,
        "page": page,
        "page_svg_seconds_best": page_best,
        "page_svg_bytes": bytes_out,
        "rounds": rounds,
    }))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(report) => println!("{report}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}
